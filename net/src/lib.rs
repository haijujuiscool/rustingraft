//! Rusting Raft sessions (spec 3.2). A host (solo, or online through the relay) runs the
//! authoritative `World`; relay clients keep a mirror fed by slow sections, fast snapshots
//! and events, and send requests. Messages are JSON inside relay frames (`proto`).

mod proto;

use proto::{audience, ToClient, ToHost};
use raft_relay::{ClientIncoming, ClientLink, HostIncoming, HostLink, RelayClient, BROADCAST};
use raft_sim::world::MAX_DT;
use raft_sim::{DynamicSnapshot, Event, Ground, PlayerId, Request, SectionId, World};
use serde_json::Value;
use std::collections::BTreeMap;
use std::hash::{DefaultHasher, Hasher};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread;

/// Bump on any wire change (messages, or the sim types they carry).
pub const PROTOCOL_VERSION: u32 = 1;

/// Slow sections and own player entries: checked at 4 Hz, sent when changed.
const SECTION_PERIOD: f64 = 0.25;
const SNAPSHOT_PERIOD: f64 = 1.0 / 15.0;
/// Client movement (PlayerTransform, BoatTransform).
const POSE_PERIOD: f64 = 1.0 / 20.0;
/// Messages per second taken from one client; the excess is ignored.
const MAX_MSGS_PER_SEC: u32 = 200;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Status {
    Connecting,
    Playing,
    Closed(String),
}

pub struct Session(Role);

enum Role {
    Host(Host),
    Client(Client),
}

impl Session {
    /// Offline host: no network. Joins `name` as the local player.
    pub fn solo(world: World, name: &str) -> Result<Session, String> {
        let (world, me) = adopt(world, name);
        let me = me.map_err(String::from)?;
        Ok(Session(Role::Host(Host::new(
            world,
            name,
            Some(me),
            HostNet::Offline,
        ))))
    }

    /// Online host via relay. Connects in a background thread; `status()` is Connecting until
    /// the relay returns the room code, then Playing. The local player plays immediately. A
    /// refused `name` gives `Closed(reason)` with `me()` None.
    pub fn host(relay_addr: &str, world: World, name: &str) -> Session {
        let (world, me) = adopt(world, name);
        let net = match me {
            Ok(_) => {
                let (tx, rx) = mpsc::channel();
                let addr = relay_addr.to_string();
                thread::spawn(move || {
                    let _ = tx.send(RelayClient::host(addr).map_err(|e| e.to_string()));
                });
                HostNet::Connecting(rx)
            }
            Err(e) => HostNet::Closed(e.into()),
        };
        Session(Role::Host(Host::new(world, name, me.ok(), net)))
    }

    /// Online client via relay + room code. `world()`/`me()` are None until Welcome arrives.
    pub fn join(relay_addr: &str, code: &str, name: &str) -> Session {
        let (tx, rx) = mpsc::channel();
        let code = code.trim().to_ascii_uppercase();
        let (addr, room, name) = (relay_addr.to_string(), code.clone(), name.to_string());
        thread::spawn(move || {
            let hello = encode(&ToHost::Hello {
                version: PROTOCOL_VERSION,
                name,
            });
            let link = RelayClient::join(addr, &room)
                .map_err(|e| e.to_string())
                .and_then(|(_, link)| link.send(&hello).map(|()| link).map_err(|e| e.to_string()));
            let _ = tx.send(link);
        });
        Session(Role::Client(Client {
            code,
            net: ClientNet::Connecting(rx),
            world: None,
            me: None,
            time: 0.0,
            pose_at: 0.0,
            pose: None,
            boat: None,
        }))
    }

    /// Call once per fixed tick (dt = 1/60). Host: network in, local requests, `World::step`,
    /// network out. Client: network in, throttled movement out. Returns the events the local
    /// player should react to (host: every event of its world; client: events received).
    pub fn update(&mut self, dt: f32, ground: &dyn Ground) -> Vec<Event> {
        match &mut self.0 {
            Role::Host(h) => h.update(dt, ground),
            Role::Client(c) => c.update(dt),
        }
    }

    /// Host: queued and applied at the start of the next `update` (`World::apply` needs the
    /// ground); its events come back from that `update`. Client: sent to the host, movement
    /// throttled to 20 Hz keeping the latest, and mirrored locally right away.
    pub fn request(&mut self, req: Request) {
        match &mut self.0 {
            Role::Host(h) => h.queued.push(req),
            Role::Client(c) => c.request(req),
        }
    }

    pub fn world(&self) -> Option<&World> {
        match &self.0 {
            Role::Host(h) => Some(&h.world),
            Role::Client(c) => c.world.as_ref(),
        }
    }

    pub fn me(&self) -> Option<PlayerId> {
        match &self.0 {
            Role::Host(h) => h.me,
            Role::Client(c) => c.me,
        }
    }

    pub fn is_host(&self) -> bool {
        matches!(self.0, Role::Host(_))
    }

    /// Host: the relay room code once known. Client: the code it joined with.
    pub fn room_code(&self) -> Option<&str> {
        match &self.0 {
            Role::Host(h) => match &h.net {
                HostNet::Online(code, _) => Some(code),
                _ => None,
            },
            Role::Client(c) => Some(&c.code),
        }
    }

    pub fn status(&self) -> Status {
        match &self.0 {
            Role::Host(h) => match &h.net {
                HostNet::Connecting(_) => Status::Connecting,
                HostNet::Offline | HostNet::Online(..) => Status::Playing,
                HostNet::Closed(reason) => Status::Closed(reason.clone()),
            },
            Role::Client(c) => match &c.net {
                ClientNet::Open(_) if c.world.is_some() => Status::Playing,
                ClientNet::Connecting(_) | ClientNet::Open(_) => Status::Connecting,
                ClientNet::Closed(reason) => Status::Closed(reason.clone()),
            },
        }
    }

    /// Host only: replace the world (load save / game over reload); re-joins everyone
    /// connected (by name) and resyncs every client with a fresh Welcome. No-op on clients.
    pub fn replace_world(&mut self, world: World) {
        if let Role::Host(h) = &mut self.0 {
            h.replace_world(world);
        }
    }

    /// Host only: the world JSON for saving.
    pub fn save_json(&self) -> Option<String> {
        match &self.0 {
            Role::Host(h) => Some(h.world.to_json()),
            Role::Client(_) => None,
        }
    }

    /// Other players in the session (host: welcomed clients; client: others connected).
    pub fn peer_count(&self) -> usize {
        match &self.0 {
            Role::Host(h) => h.peers.values().filter(|p| p.player.is_some()).count(),
            Role::Client(c) => match (&c.world, c.me) {
                (Some(w), Some(me)) => w.connected().filter(|(id, _)| *id != me).count(),
                _ => 0,
            },
        }
    }
}

// ---------------------------------------------------------------- host

enum HostNet {
    Offline,
    Connecting(Receiver<Result<(String, HostLink), String>>),
    Online(String, HostLink),
    Closed(String),
}

#[derive(Default)]
struct Peer {
    /// Set once the Hello was accepted.
    player: Option<PlayerId>,
    /// Hash of the last own entry sent.
    entry: Option<u64>,
    /// Messages in the current one-second window.
    msgs: u32,
}

struct Host {
    world: World,
    name: String,
    me: Option<PlayerId>,
    net: HostNet,
    /// Relay client id -> peer.
    peers: BTreeMap<u8, Peer>,
    queued: Vec<Request>,
    /// Hash of the last sent value per `SectionId::ALL` entry.
    sections: [Option<u64>; SectionId::ALL.len()],
    time: f64,
    sections_at: f64,
    snapshot_at: f64,
    window_at: f64,
}

/// Disconnects everyone in `world` (fresh or loaded) and joins the local player.
fn adopt(mut world: World, name: &str) -> (World, Result<PlayerId, &'static str>) {
    let ids: Vec<PlayerId> = world.players.keys().copied().collect();
    for id in ids {
        world.leave(id);
    }
    let me = world.join(name);
    (world, me)
}

impl Host {
    fn new(world: World, name: &str, me: Option<PlayerId>, net: HostNet) -> Host {
        Host {
            world,
            name: name.to_string(),
            me,
            net,
            peers: BTreeMap::new(),
            queued: Vec::new(),
            sections: Default::default(),
            time: 0.0,
            sections_at: 0.0,
            snapshot_at: 0.0,
            window_at: 0.0,
        }
    }

    fn update(&mut self, dt: f32, ground: &dyn Ground) -> Vec<Event> {
        self.time += seconds(dt);
        if let HostNet::Connecting(rx) = &self.net {
            match rx.try_recv() {
                Ok(Ok((code, link))) => self.net = HostNet::Online(code, link),
                Ok(Err(e)) => self.net = HostNet::Closed(e),
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => {
                    self.net = HostNet::Closed("Relay connection failed".into())
                }
            }
        }
        let mut events = Vec::new();
        let queued = std::mem::take(&mut self.queued);
        if let Some(me) = self.me {
            for req in queued {
                events.extend(self.world.apply(me, req, ground));
            }
        }
        self.receive(ground, &mut events);
        events.extend(self.world.step(dt, ground));
        self.send_updates(&events);
        events
    }

    fn receive(&mut self, ground: &dyn Ground, events: &mut Vec<Event>) {
        if self.time - self.window_at >= 1.0 {
            self.window_at = self.time;
            for p in self.peers.values_mut() {
                p.msgs = 0;
            }
        }
        let HostNet::Online(_, link) = &self.net else {
            return;
        };
        let incoming: Vec<HostIncoming> = std::iter::from_fn(|| link.try_recv()).collect();
        for msg in incoming {
            match msg {
                HostIncoming::Joined(id) => {
                    self.peers.insert(id, Peer::default());
                }
                HostIncoming::Left(id) => {
                    if let Some(pid) = self.peers.remove(&id).and_then(|p| p.player) {
                        self.world.leave(pid);
                    }
                }
                HostIncoming::Data { from, bytes } => self.on_data(from, &bytes, ground, events),
                HostIncoming::Closed(reason) => {
                    for pid in self.peers.values().filter_map(|p| p.player) {
                        self.world.leave(pid);
                    }
                    self.peers.clear();
                    self.net = HostNet::Closed(reason);
                }
            }
        }
    }

    fn on_data(&mut self, from: u8, bytes: &[u8], ground: &dyn Ground, events: &mut Vec<Event>) {
        let Some(peer) = self.peers.get_mut(&from) else {
            return;
        };
        peer.msgs += 1;
        if peer.msgs > MAX_MSGS_PER_SEC {
            return;
        }
        // Malformed or out-of-order messages are ignored.
        match (serde_json::from_slice::<ToHost>(bytes), peer.player) {
            (Ok(ToHost::Hello { version, name }), None) => {
                if version == PROTOCOL_VERSION {
                    self.admit(from, &name);
                } else {
                    let reason = "Version mismatch".into();
                    self.send(from, &ToClient::Reject { reason });
                }
            }
            (Ok(ToHost::Req(req) | ToHost::Pose(req)), Some(pid)) => {
                events.extend(self.world.apply(pid, req, ground));
            }
            _ => {}
        }
    }

    /// Joins `name` for relay client `id` and answers with the full world, or a Reject.
    fn admit(&mut self, id: u8, name: &str) {
        let Some(peer) = self.peers.get_mut(&id) else {
            return;
        };
        let reply = match self.world.join(name) {
            Ok(pid) => {
                peer.player = Some(pid);
                peer.entry = None;
                ToClient::Welcome {
                    player_id: pid,
                    world: Box::new(self.world.clone()),
                }
            }
            Err(e) => {
                peer.player = None;
                ToClient::Reject { reason: e.into() }
            }
        };
        self.send(id, &reply);
    }

    fn replace_world(&mut self, world: World) {
        let names: Vec<(u8, String)> = self
            .peers
            .iter()
            .filter_map(|(&id, p)| Some((id, self.world.player(p.player?)?.name.clone())))
            .collect();
        let (world, me) = adopt(world, &self.name);
        self.world = world;
        self.me = me.ok();
        self.sections = Default::default();
        for p in self.peers.values_mut() {
            p.player = None;
        }
        for (id, name) in names {
            self.admit(id, &name);
        }
    }

    fn send_updates(&mut self, events: &[Event]) {
        let welcomed: Vec<(u8, PlayerId)> = self
            .peers
            .iter()
            .filter_map(|(&id, p)| Some((id, p.player?)))
            .collect();
        if welcomed.is_empty() {
            return;
        }
        // Per client, so each keeps the host's event order.
        for &(id, pid) in &welcomed {
            let mine: Vec<Event> = events
                .iter()
                .filter(|e| audience(e).includes(pid))
                .cloned()
                .collect();
            if !mine.is_empty() {
                self.send(id, &ToClient::Events(mine));
            }
        }
        if due(self.time, &mut self.sections_at, SECTION_PERIOD) {
            for (id, value) in changed_sections(&self.world, &mut self.sections) {
                self.send(BROADCAST, &ToClient::Section { id, value });
            }
            for &(id, pid) in &welcomed {
                let (Some(value), Some(peer)) =
                    (self.world.player_entry(pid), self.peers.get_mut(&id))
                else {
                    continue;
                };
                if changed(&mut peer.entry, &value) {
                    self.send(id, &ToClient::Me(value));
                }
            }
        }
        if due(self.time, &mut self.snapshot_at, SNAPSHOT_PERIOD) {
            self.send(BROADCAST, &ToClient::Snapshot(self.world.snapshot()));
        }
    }

    fn send(&self, target: u8, msg: &ToClient) {
        if let HostNet::Online(_, link) = &self.net {
            // ponytail: one relay frame per message (MAX_DATA, 64 KiB); a full 8-player world
            // is ~7 KB. Split messages into parts if worlds ever grow near the limit.
            let _ = link.send_to(target, &encode(msg));
        }
    }
}

/// Stores the hash of `value` in `last`; true when it differs from the previous one.
fn changed(last: &mut Option<u64>, value: &Value) -> bool {
    let mut h = DefaultHasher::new();
    h.write(value.to_string().as_bytes());
    let hash = Some(h.finish());
    std::mem::replace(last, hash) != hash
}

fn changed_sections(
    world: &World,
    last: &mut [Option<u64>; SectionId::ALL.len()],
) -> Vec<(SectionId, Value)> {
    SectionId::ALL
        .iter()
        .zip(last)
        .filter_map(|(&id, last)| {
            let value = world.section(id);
            changed(last, &value).then_some((id, value))
        })
        .collect()
}

/// Session time for one tick; like `World::step`, a non-finite dt counts as nothing.
fn seconds(dt: f32) -> f64 {
    if dt.is_finite() {
        f64::from(dt.clamp(0.0, MAX_DT))
    } else {
        0.0
    }
}

/// True once `period` seconds passed since `*at`, restarting the timer.
fn due(now: f64, at: &mut f64, period: f64) -> bool {
    // The epsilon keeps sums of 1/60 from landing just short of 1/15 or 1/20.
    if now - *at + 1e-6 < period {
        return false;
    }
    *at = now;
    true
}

fn encode<T: serde::Serialize>(msg: &T) -> Vec<u8> {
    serde_json::to_vec(msg).expect("messages always serialize")
}

// ---------------------------------------------------------------- client

enum ClientNet {
    Connecting(Receiver<Result<ClientLink, String>>),
    Open(ClientLink),
    Closed(String),
}

struct Client {
    code: String,
    net: ClientNet,
    world: Option<World>,
    me: Option<PlayerId>,
    time: f64,
    pose_at: f64,
    /// Latest unsent movement.
    pose: Option<Request>,
    boat: Option<Request>,
}

impl Client {
    fn update(&mut self, dt: f32) -> Vec<Event> {
        self.time += seconds(dt);
        if let ClientNet::Connecting(rx) = &self.net {
            match rx.try_recv() {
                Ok(Ok(link)) => self.net = ClientNet::Open(link),
                Ok(Err(e)) => self.net = ClientNet::Closed(e),
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => {
                    self.net = ClientNet::Closed("Relay connection failed".into())
                }
            }
        }
        let ClientNet::Open(link) = &self.net else {
            return Vec::new();
        };
        let incoming: Vec<ClientIncoming> = std::iter::from_fn(|| link.try_recv()).collect();
        let mut events = Vec::new();
        for msg in incoming {
            match msg {
                ClientIncoming::Data(bytes) => {
                    if let Ok(msg) = serde_json::from_slice(&bytes) {
                        self.on_message(msg, &mut events);
                    }
                }
                ClientIncoming::Closed(_) => self.net = ClientNet::Closed("Host left".into()),
            }
            if matches!(self.net, ClientNet::Closed(_)) {
                return events;
            }
        }
        if due(self.time, &mut self.pose_at, POSE_PERIOD) {
            self.flush();
        }
        events
    }

    fn mirror(&mut self) -> Option<(&mut World, PlayerId)> {
        Some((self.world.as_mut()?, self.me?))
    }

    fn on_message(&mut self, msg: ToClient, events: &mut Vec<Event>) {
        match msg {
            ToClient::Welcome { player_id, world } => {
                self.world = Some(*world);
                self.me = Some(player_id);
            }
            ToClient::Reject { reason } => self.net = ClientNet::Closed(reason),
            ToClient::Section { id, value } => {
                if let Some(w) = &mut self.world {
                    let _ = w.apply_section(id, value);
                }
            }
            ToClient::Me(value) => self.apply_me(value),
            ToClient::Snapshot(snap) => self.apply_snapshot(&snap),
            ToClient::Events(ev) => {
                let me = self.me;
                let teleport = ev.iter().rev().find_map(|e| match e {
                    Event::PlayerTeleport { player, pos } if Some(*player) == me => Some(*pos),
                    _ => None,
                });
                if let (Some(pos), Some((w, me))) = (teleport, self.mirror()) {
                    if let Some(p) = w.player_mut(me) {
                        p.pos = pos;
                    }
                }
                events.extend(ev);
            }
        }
    }

    /// The own entry replaces the mirror's, except the pose: movement is client-owned
    /// (unless seated: the host moves seated players with their boat).
    fn apply_me(&mut self, value: Value) {
        let Some((w, me)) = self.mirror() else {
            return;
        };
        let old = w.player(me).cloned();
        if w.apply_player_entry(me, value).is_err() {
            return;
        }
        if let (Some(old), Some(p)) = (old, w.player_mut(me)) {
            if p.seat.is_none() {
                p.pos = old.pos;
                p.yaw = old.yaw;
                p.in_water = old.in_water;
                p.sprinting = old.sprinting;
            }
        }
    }

    /// The snapshot skips the own pose; the boat this client drives keeps its local pose too.
    fn apply_snapshot(&mut self, snap: &DynamicSnapshot) {
        let Some((w, me)) = self.mirror() else {
            return;
        };
        let driven =
            driven_boat(w, me).and_then(|id| Some((id, w.dynamic.boats.get(&id)?.clone())));
        w.apply_snapshot(snap, Some(me));
        if let Some((id, old)) = driven {
            if let Some(b) = w.dynamic.boats.get_mut(&id) {
                b.pos = old.pos;
                b.yaw = old.yaw;
            }
        }
    }

    fn request(&mut self, req: Request) {
        let Some((w, me)) = self.mirror() else {
            return;
        };
        match req {
            Request::PlayerTransform {
                pos,
                yaw,
                in_water,
                sprinting,
            } => {
                if let Some(p) = w.player_mut(me) {
                    p.pos = pos;
                    p.yaw = yaw;
                    p.in_water = in_water;
                    p.sprinting = sprinting;
                }
                self.pose = Some(req);
            }
            Request::BoatTransform { pos, yaw, .. } => {
                if let Some(b) = driven_boat(w, me).and_then(|id| w.dynamic.boats.get_mut(&id)) {
                    b.pos = pos;
                    b.yaw = yaw;
                }
                self.boat = Some(req);
            }
            _ => {
                // Movement first, so the host checks ranges against where we are now.
                self.flush();
                self.send(&ToHost::Req(req));
            }
        }
    }

    fn flush(&mut self) {
        for req in [self.pose.take(), self.boat.take()].into_iter().flatten() {
            self.send(&ToHost::Pose(req));
        }
    }

    fn send(&self, msg: &ToHost) {
        if let ClientNet::Open(link) = &self.net {
            let _ = link.send(&encode(msg));
        }
    }
}

fn driven_boat(w: &World, me: PlayerId) -> Option<raft_sim::EntityId> {
    let seat = w.player(me)?.seat?;
    (seat.index == 0).then_some(seat.boat)
}

#[cfg(test)]
mod tests {
    use super::*;
    use raft_sim::V3;

    #[test]
    fn sections_are_resent_only_when_changed() {
        let mut w = World::new(1);
        w.join("Ana").unwrap();
        let mut last = Default::default();
        assert_eq!(changed_sections(&w, &mut last).len(), SectionId::ALL.len());
        assert!(changed_sections(&w, &mut last).is_empty());
        w.economy.save_count += 1;
        let ids: Vec<SectionId> = changed_sections(&w, &mut last)
            .into_iter()
            .map(|s| s.0)
            .collect();
        assert_eq!(ids, [SectionId::Economy]);
        w.leave(0);
        let ids: Vec<SectionId> = changed_sections(&w, &mut last)
            .into_iter()
            .map(|s| s.0)
            .collect();
        assert_eq!(ids, [SectionId::Roster]);

        let mut entry = None;
        let value = w.player_entry(0).unwrap();
        assert!(changed(&mut entry, &value));
        assert!(!changed(&mut entry, &value));
    }

    #[test]
    fn rates_hold_at_60_hz() {
        let (mut snap, mut pose, mut slow, mut now) = (0.0, 0.0, 0.0, 0.0);
        let mut counts = [0; 3];
        for _ in 0..600 {
            now += f64::from(1.0f32 / 60.0);
            counts[0] += due(now, &mut snap, SNAPSHOT_PERIOD) as u32;
            counts[1] += due(now, &mut pose, POSE_PERIOD) as u32;
            counts[2] += due(now, &mut slow, SECTION_PERIOD) as u32;
        }
        assert_eq!(counts, [150, 200, 40]);
    }

    fn mirror_client() -> (Client, World, PlayerId) {
        let mut host = World::new(3);
        let me = host.join("Ana").unwrap();
        let client = Client {
            code: "ABCDEF".into(),
            net: ClientNet::Closed("test".into()),
            world: Some(host.clone()),
            me: Some(me),
            time: 0.0,
            pose_at: 0.0,
            pose: None,
            boat: None,
        };
        (client, host, me)
    }

    #[test]
    fn client_keeps_its_own_pose() {
        let (mut c, mut host, me) = mirror_client();
        let here = V3::new(12.0, 0.0, -7.0);
        c.request(Request::PlayerTransform {
            pos: here,
            yaw: 0.5,
            in_water: true,
            sprinting: false,
        });
        assert!(c.pose.is_some(), "movement waits for the 20 Hz flush");

        let p = host.player_mut(me).unwrap();
        p.pos = V3::new(1.0, 0.45, 1.0);
        p.coins = 40;
        c.apply_me(host.player_entry(me).unwrap());
        c.apply_snapshot(&host.snapshot());
        let p = c.world.as_ref().unwrap().player(me).unwrap();
        assert_eq!((p.pos, p.yaw, p.in_water, p.coins), (here, 0.5, true, 40));

        let mut events = Vec::new();
        let to = V3::new(2.0, 0.45, 2.0);
        c.on_message(
            ToClient::Events(vec![Event::PlayerTeleport {
                player: me,
                pos: to,
            }]),
            &mut events,
        );
        assert_eq!(c.world.as_ref().unwrap().player(me).unwrap().pos, to);
        assert_eq!(events.len(), 1);
    }

    #[test]
    fn driver_keeps_its_boat_pose() {
        let (mut c, mut host, me) = mirror_client();
        let boat = host.spawn_boat(raft_sim::content::BoatKind::Skiff, V3::new(0.0, 0.0, 10.0));
        c.apply_snapshot(&host.snapshot());
        let seat = raft_sim::players::Seat { boat, index: 0 };
        c.world.as_mut().unwrap().player_mut(me).unwrap().seat = Some(seat);
        let ahead = V3::new(0.0, 0.0, 20.0);
        c.request(Request::BoatTransform {
            pos: ahead,
            yaw: 1.0,
            speed: 3.0,
        });
        c.apply_snapshot(&host.snapshot());
        assert_eq!(c.world.as_ref().unwrap().dynamic.boats[&boat].pos, ahead);
    }
}
