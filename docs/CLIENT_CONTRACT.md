# Rusting Raft client contract

The game client (`src/`) turns the `raft_net::Session` into a playable game on
RustingEngine 2.1.0. Rules live in `raft_sim` (`sim/`), networking in `raft_net`
(`net/`); the client never changes game state itself, it only sends `Request`s.

## Ownership

| Path | Owner | Job |
| --- | --- | --- |
| `src/main.rs` | skeleton (frozen) | `rusting_game!(update, tick: tick)`, the thread-local `Game` |
| `src/app.rs` | skeleton (shared) | phases, session start/stop, tick and frame flow, saving, test counters |
| `src/shared.rs` | everyone, append-only | types passed between the modules |
| `src/ground.rs`, `src/save.rs` | skeleton | `raft_sim::Ground` from the scene, user data files |
| `src/view/` | view agent | scene objects that mirror the `World` |
| `src/ui/` | ui agent | egui menus, HUD, windows |
| `src/player/` | player agent | local input, controller, camera, interaction |
| `tests/` | everyone | scenarios, one file per feature (`tests/<module>_<feature>.json`) |

Each module agent edits only its own folder and may split it into submodules
(`src/view/ocean.rs`, ...). In `app.rs`, only add the match arm a new `UiAction` needs,
or pass a new value into your module's entry point; keep such edits to a few lines.
Never edit `sim/`, `net/`, `relay/`, `scenes/`, `assets/`, `Cargo.toml` or the engine.

Boundaries: `player` never draws egui; `ui` never moves scene objects; `view` never
sends requests. Modules never call each other; they talk through `shared.rs` types.

## Per-tick and per-frame flow

```
tick (FixedUpdate, 60 Hz, before the frame's update)
  autostart check -> Session::update(1/60, &SceneGround) -> events appended to Game.events
  host: Event::SaveRequested -> save_json -> "world.json"; Event::GameOver -> GameOver
  status: Connecting -> Playing, Closed(reason) -> MainMenu with the reason
  mirror test counters

frame (once per rendered frame)
  frame = Frame { world, me, events (all ticks since last frame), dt, is_host, room_code, peers }
  Playing:  Local::update(scene, &frame, ui.modal(), &mut requests)   (player)
  always:   Ui::draw(scene, &UiState) -> Vec<UiAction>                  (ui)
  if world: View::sync(scene, &frame, &local.hud)                        (view)
  Game.events cleared; requests -> Session::request; UiActions -> Game::act
```

`Game` is created lazily by whichever hook runs first (the tick can run before the
first frame). `Frame.events` holds each event exactly once.

## Entry points

```rust
// player: `Local { pub hud: LocalHud, .. }`, the hud read by ui and view
impl Local {
    pub fn update(&mut self, scene: &mut GameScene, frame: &Frame, ui_modal: bool, out: &mut Vec<Request>);
    pub fn begin_placement(&mut self, recipe: RecipeId); // from UiAction::BeginPlacement
}
// ui
impl Ui {
    pub fn draw(&mut self, scene: &mut GameScene, state: &UiState) -> Vec<UiAction>;
    pub fn modal(&self) -> bool;                         // a window owns the pointer
}
// view
impl View {
    pub fn sync(&mut self, scene: &mut GameScene, frame: &Frame, hud: &LocalHud);
    pub fn clear(&mut self, scene: &mut GameScene);      // leaving a session
}
```

A new session (or a reload) resets `Local` to its `Default` and calls `View::clear`;
`Ui` lives for the whole run.

## Shared types (`src/shared.rs`)

- `Frame<'a>`: `world: &World`, `me: PlayerId`, `events: &[Event]`, `dt: f32`,
  `is_host: bool`, `room_code: Option<&str>` (host online only), `peers: usize`.
- `AppPhase`: `MainMenu`, `Connecting`, `Playing`, `GameOver`.
- `UiState<'a>`: `phase`, `frame: Option<&Frame>`, `hud: &LocalHud`, `settings: &Settings`,
  `has_save: bool`, `notice: Option<&str>` (why the last session ended).
- `UiAction`: `Request(Request)`, `StartSolo`, `LoadWorld`, `Host { relay, name, load }`,
  `Join { relay, code, name }`, `BeginPlacement(RecipeId)`, `SetSettings(Settings)`,
  `LoadLastSave`, `ToMenu`, `Quit`.
- `LocalHud` (written by player only): `prompt`, `camera_first_person`, `aiming`,
  `selected_slot`, `swimming`, `fishing: FishingUi { reeling, marker, zone }`,
  `ghost: Option<Ghost { recipe, cell, rot, side, valid }>`.
- `Settings` (`settings.json`): `relay` ("127.0.0.1:27777"), `name` ("Sailor"), `volume`,
  `mouse_sensitivity` (multiplier on the scene controller's look sensitivity),
  `render_scale`, `token` (this install's identity for `Session::join`, made by
  `save::load_settings`; keep it when editing settings, never show it). `Game` applies
  them on start and on `SetSettings`.

Extending: append fields and variants, never rename or remove one. New `LocalHud`
fields must have a sensible `Default`. A value only one module needs stays private
to that module.

## Sending requests

- player: push onto `out` in `Local::update`, e.g. `Request::PickUpDebris(id)`. The
  skeleton already sends `PlayerTransform { pos (feet), yaw, in_water, sprinting }` at
  20 Hz and the E interact for floating debris, campfire, shredder and logbook. F
  (`stash`) sends `PickUpDebris` for the world item on the aim ray, else `DropLast`
  (newest sac kind out; it rests on the deck or ground, `Debris::resting`, at most
  `debris::MAX_RESTING`, or floats). Resting items are F's only, never E's.
- ui: return `UiAction::Request(req)` from `Ui::draw` (craft, eat, equip, buy, ...).
- Results come back as events: show `Event::Denied` / `Event::Toast` (ui does) instead
  of predicting the outcome. Clients mirror their own requests locally right away.

## Test hooks

- `{"tick": 1, "set": {"counter": "autostart", "value": 1}}` starts a new solo world
  (seed: the scenario's `seed`). Networking is never used in scenarios.
- Counters mirrored every tick: `phase` (0 menu, 1 connecting, 2 playing, 3 game over),
  `day`, `time` (seconds into the cycle), `coins`, `hp`, `hunger`, `wood`, `metal`,
  `biomass`, `oil`, `campfire_level`, `fuel` (seconds), `raft_tiles`, `debris`,
  `world_items_resting` (dropped items lying on the deck or ground), `enemies`. Add more in `Game::mirror_counters` with the module prefix
  (`view_avatars`, `player_swimming`). Counters are `i32`.
- Saves: each scenario starts with an empty user data folder; seed one with the
  scenario's top-level `files`, check one with `expect_file` (`world.json`).
- `tests/boot.json`: autostart, 16 raft tiles, player on the deck, clock running,
  `shots/boot.png`.

## Naming

- Templates (scene, see `docs/scene-names.md`): `tpl.<snake_case enum variant>`, e.g.
  `tpl.raft_floor`, `tpl.railing`, `tpl.wood_farm`, `tpl.enemy.gloomfin`,
  `tpl.boat.dinghy`, `tpl.avatar`. `view::snake("WoodFarm") == "wood_farm"`.
- A raft template's origin is the deck top at its footprint centre, authored for
  `rot` 0 (railings run along X). Copies keep the template's rotation and scale.
- Spawned copies: `<group>.<key>`: `raft.tile.<i>.<j>`, `raft.structure.<id>`,
  `raft.railing.<i>.<j>.<Side>`; new groups follow suit (`debris.<id>`, `enemy.<id>`,
  `boat.<id>`, `avatar.<player id>`). The view owns every name it spawns.
- Input actions (scene): `use`, `aim`, `camera`, `sac`, `build`, `rotate`, `pause`,
  `interact`, `stash`, `slot1`..`slot6`; movement is the built-in `player.*` set.
- Labels that scenarios click (`New world`, `Load world`, `Host online`, `Join online`,
  `Load last save`, `Main menu`, `Quit`) stay unique and unchanged.

## Commands

```powershell
$rusting = 'C:\Users\juliu\RustingEngine\target\release\rusting.exe'
& $rusting check C:\Users\juliu\RustingRaft            # cargo build + project checks
& $rusting test C:\Users\juliu\RustingRaft             # every scenario in tests/
& $rusting test C:\Users\juliu\RustingRaft tests\boot.json
& $rusting run C:\Users\juliu\RustingRaft --ticks 600 --json
cargo test --bin rustingraft --manifest-path C:\Users\juliu\RustingRaft\Cargo.toml  # client unit tests
# sim unit tests (keep a separate target dir)
$env:CARGO_TARGET_DIR = 'C:\Users\juliu\RustingRaft\target\sim'
cargo test -p raft_sim --manifest-path C:\Users\juliu\RustingRaft\Cargo.toml
```

Captures land in `tests/shots/` (git-ignored); open them to look at the result.

## Engine pitfalls (2.1.0)

- `rusting_engine::prelude::*` exports bevy's `World` and a `serde` re-export. Import
  `raft_sim::World` by name, and import single items instead of the prelude glob in a
  file that derives serde traits, so `serde` names one crate.
- `spawn_copy` / `spawn_copy_at_root` panic when the name exists: despawn first or keep
  names unique. Copies keep the template's hidden flag: call `set_visible(name, true)`.
  `spawn_copy_at_root` takes the whole transform, so pass the template's scale.
- `object(name)` panics on a missing object; use `try_object`. Templates may be missing
  while the scene grows: skip, do not panic.
- `set_counter` creates an object named after the counter: never give a scene object a
  counter's name.
- The player controller is a kinematic capsule: its position is the body centre,
  `player::FEET_OFFSET` (0.9 m) above the feet the sim uses. It ignores water: swimming
  is game code (gravity, speed through `set_player`). Teleport with `set_position` and
  zero `vertical_speed`.
- `raycast` sees only colliders with a physics body, also hits sensors, and a ray that
  starts inside a collider passes it. It hits nothing before physics has stepped once,
  and child colliders sit at a stale pose until a step after the first frame (which can
  run several steps). Island ground colliders are in class `ground` (on the collider
  object itself); `SceneGround` samples them once, on the first tick after a step that
  followed the first frame. The player's capsule is in class `player`, which the
  player's own rays skip.
- `pressed` in the frame hook sees this frame's presses; player code runs in the frame.
- `set_paused` stops fixed ticks and with them `Session::update`: never pause while
  online.
- egui is 0.31 (`rusting_engine::egui`); `scene.ui()` returns the `Context`. Scenario
  `click` finds a button by its label text.
- Only the host writes `world.json`; there is no autosave. A world loaded from the save
  (`LoadWorld`, `Host { load: true }`, `LoadLastSave`) also gets
  `Session::restore_owners(raw save text)`, so returning clients get their entries back.
- `Status::Closed` reasons the menu explains: `name taken`, `world full`, `bad token`,
  `Version mismatch` (`ui::notice_text`); any other reason shows as is.
