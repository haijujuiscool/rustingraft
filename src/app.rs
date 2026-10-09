//! Game phases, the session and the per-tick / per-frame flow.

use crate::ground::SceneGround;
use crate::player::{self, Local};
use crate::save;
use crate::shared::{AppPhase, Frame, Settings, UiAction, UiState};
use crate::ui::Ui;
use crate::view::View;
use raft_net::{Session, Status};
use raft_sim::{Event, Request, World};
use rusting_engine::prelude::GameScene;

pub const TICK_DT: f32 = 1.0 / 60.0;

pub struct Game {
    pub phase: AppPhase,
    session: Option<Session>,
    view: View,
    ui: Ui,
    local: Local,
    settings: Settings,
    /// Events from the ticks since the last frame.
    events: Vec<Event>,
    /// Sampled on the first tick of the first session.
    ground: Option<SceneGround>,
    has_save: bool,
    notice: Option<String>,
    /// The scene controller's own look sensitivity, scaled by the setting.
    base_look: f32,
}

impl Game {
    pub fn new(scene: &mut GameScene) -> Game {
        let settings = save::load_settings(scene);
        let base_look = scene
            .player(player::PLAYER)
            .map_or(0.002, |pc| pc.look_sensitivity);
        let game = Game {
            phase: AppPhase::MainMenu,
            session: None,
            view: View::default(),
            ui: Ui::default(),
            local: Local::default(),
            has_save: save::has_world(scene),
            settings,
            events: Vec::new(),
            ground: None,
            notice: None,
            base_look,
        };
        game.apply_settings(scene);
        game
    }

    /// Fixed tick (60 Hz).
    pub fn tick(&mut self, scene: &mut GameScene) {
        // Scenarios start a solo world with `{"set": {"counter": "autostart", "value": 1}}`.
        if self.phase == AppPhase::MainMenu && scene.counter_or("autostart", 0) == 1 {
            scene.set_counter("autostart", 2);
            self.act(scene, UiAction::StartSolo);
        }
        scene.set_counter("phase", self.phase as i32);
        let Some(session) = self.session.as_mut() else {
            return;
        };
        let ground = self.ground.get_or_insert_with(|| SceneGround::build(scene));
        let events = session.update(TICK_DT, ground);
        for event in &events {
            match event {
                Event::SaveRequested { .. } if session.is_host() => {
                    let text = match session
                        .save_json()
                        .map(|json| save::save_world(scene, &json))
                    {
                        Some(Ok(())) => {
                            self.has_save = true;
                            "World saved".to_string()
                        }
                        Some(Err(err)) => format!("Save failed: {err}"),
                        None => continue,
                    };
                    self.events.push(Event::Toast { player: None, text });
                }
                Event::GameOver => self.phase = AppPhase::GameOver,
                _ => {}
            }
        }
        self.events.extend(events);
        match session.status() {
            Status::Connecting => {}
            Status::Playing if self.phase == AppPhase::Connecting => self.phase = AppPhase::Playing,
            Status::Playing => {}
            Status::Closed(reason) => {
                self.to_menu(scene, Some(reason));
                return;
            }
        }
        self.mirror_counters(scene);
    }

    /// Rendered frame: player -> ui -> view, then the collected requests and actions.
    pub fn frame(&mut self, scene: &mut GameScene, dt: f32) {
        let mut requests: Vec<Request> = Vec::new();
        let actions = {
            let session = self.session.as_ref();
            let frame = session.and_then(|s| {
                Some(Frame {
                    world: s.world()?,
                    me: s.me()?,
                    events: &self.events,
                    dt,
                    is_host: s.is_host(),
                    room_code: s.room_code().filter(|_| s.is_host()),
                    peers: s.peer_count(),
                })
            });
            let modal = self.ui.modal();
            if let (Some(frame), AppPhase::Playing) = (&frame, self.phase) {
                self.local.update(scene, frame, modal, &mut requests);
            } else {
                scene.set_mouse_look(player::PLAYER, false);
            }
            let actions = self.ui.draw(
                scene,
                &UiState {
                    phase: self.phase,
                    frame: frame.as_ref(),
                    hud: &self.local.hud,
                    settings: &self.settings,
                    has_save: self.has_save,
                    notice: self.notice.as_deref(),
                },
            );
            if let Some(frame) = &frame {
                self.view.sync(scene, frame, &self.local.hud);
            }
            actions
        };
        self.events.clear();
        if let Some(session) = self.session.as_mut() {
            for req in requests {
                session.request(req);
            }
        }
        self.follow_host();
        for action in actions {
            self.act(scene, action);
        }
    }

    pub fn act(&mut self, scene: &mut GameScene, action: UiAction) {
        match action {
            UiAction::Request(req) => {
                if let Some(session) = self.session.as_mut() {
                    session.request(req);
                }
            }
            UiAction::StartSolo => self.start_solo(scene, false),
            UiAction::LoadWorld => self.start_solo(scene, true),
            UiAction::Host { relay, name, load } => {
                self.remember(scene, &relay, &name);
                match pick_world(scene, load) {
                    Ok((world, save)) => {
                        self.start(scene, Session::host(&relay, world, &name), save)
                    }
                    Err(err) => self.notice = Some(err),
                }
            }
            UiAction::Join { relay, code, name } => {
                self.remember(scene, &relay, &name);
                let session = Session::join(&relay, &code, &name, &self.settings.token);
                self.start(scene, session, None);
            }
            UiAction::BeginPlacement(recipe) => self.local.begin_placement(recipe),
            UiAction::SetSettings(settings) => {
                self.settings = settings;
                save::save_settings(scene, &self.settings);
                self.apply_settings(scene);
            }
            UiAction::LoadLastSave => match (save::load_world(scene), self.session.as_mut()) {
                (Ok((world, text)), Some(session)) => {
                    session.replace_world(world);
                    session.restore_owners(&text);
                    self.local = Local::default();
                    self.phase = AppPhase::Playing;
                }
                (Err(err), _) => self.events.push(Event::Toast {
                    player: None,
                    text: err,
                }),
                (Ok(_), None) => {}
            },
            UiAction::ToMenu => self.to_menu(scene, None),
            UiAction::Quit => scene.quit(),
        }
    }

    fn start_solo(&mut self, scene: &mut GameScene, load: bool) {
        let started = pick_world(scene, load)
            .and_then(|(world, save)| Ok((Session::solo(world, &self.settings.name)?, save)));
        match started {
            Ok((session, save)) => self.start(scene, session, save),
            Err(err) => self.notice = Some(err),
        }
    }

    /// `save`: the raw save text when the world came from one.
    fn start(&mut self, scene: &mut GameScene, mut session: Session, save: Option<String>) {
        if let Some(text) = &save {
            session.restore_owners(text);
        }
        self.view.clear(scene);
        self.local = Local::default();
        self.events.clear();
        self.notice = None;
        self.session = Some(session);
        self.phase = AppPhase::Connecting;
    }

    fn to_menu(&mut self, scene: &mut GameScene, notice: Option<String>) {
        self.view.clear(scene);
        self.session = None;
        self.events.clear();
        self.notice = notice;
        self.phase = AppPhase::MainMenu;
    }

    /// A client in GameOver plays on once the host's reload brings its player back up.
    // ponytail: guesses the reload from our own player, a host->client message if it misfires.
    fn follow_host(&mut self) {
        if self.phase != AppPhase::GameOver {
            return;
        }
        let Some(session) = self.session.as_ref().filter(|s| !s.is_host()) else {
            return;
        };
        let up = session
            .world()
            .zip(session.me())
            .and_then(|(w, me)| w.player(me))
            .is_some_and(|p| p.connected && !p.downed);
        if up {
            self.local = Local::default();
            self.phase = AppPhase::Playing;
        }
    }

    /// Keeps the menu's relay and name for next time.
    fn remember(&mut self, scene: &mut GameScene, relay: &str, name: &str) {
        if self.settings.relay != relay || self.settings.name != name {
            self.settings.relay = relay.to_string();
            self.settings.name = name.to_string();
            save::save_settings(scene, &self.settings);
        }
    }

    fn apply_settings(&self, scene: &mut GameScene) {
        let s = &self.settings;
        scene.set_master_volume(s.volume);
        scene.set_render_scale(s.render_scale);
        let look = self.base_look * s.mouse_sensitivity;
        scene.set_player(player::PLAYER, |pc| pc.look_sensitivity = look);
    }

    /// Test hooks: world values as counters, every tick.
    fn mirror_counters(&self, scene: &mut GameScene) {
        let Some(session) = &self.session else {
            return;
        };
        let Some(w) = session.world() else {
            return;
        };
        let stock = &w.economy.stock;
        let fire = &w.economy.campfire;
        let counters = [
            ("day", w.clock.day as i32),
            ("time", w.clock.time as i32),
            ("wood", stock.wood as i32),
            ("metal", stock.metal as i32),
            ("biomass", stock.biomass as i32),
            ("oil", stock.oil as i32),
            ("campfire_level", fire.level as i32),
            ("fuel", fire.fuel as i32),
            ("raft_tiles", w.raft.tiles.len() as i32),
            ("debris", w.dynamic.debris.len() as i32),
            ("enemies", w.dynamic.enemies.len() as i32),
        ];
        for (name, value) in counters {
            scene.set_counter(name, value);
        }
        if let Some(me) = session.me().and_then(|id| w.player(id)) {
            scene.set_counter("hp", me.health.round() as i32);
            scene.set_counter("hunger", me.hunger.round() as i32);
            scene.set_counter("coins", me.coins as i32);
        }
    }
}

/// A new world, or the saved one with its raw text for `Session::restore_owners`.
fn pick_world(scene: &GameScene, load: bool) -> Result<(World, Option<String>), String> {
    if load {
        save::load_world(scene).map(|(world, text)| (world, Some(text)))
    } else {
        Ok((World::new(new_seed(scene)), None))
    }
}

/// The scenario seed, or a fresh one outside tests.
fn new_seed(scene: &GameScene) -> u64 {
    match scene.seed() {
        0 => std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(1, |d| d.as_nanos() as u64),
        seed => seed,
    }
}
