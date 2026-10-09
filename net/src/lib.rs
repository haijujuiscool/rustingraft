//! Rusting Raft sessions (spec 3.2). A host (solo, or online through the relay) runs the
//! authoritative `World`; relay clients keep a mirror fed by slow sections, fast snapshots
//! and events, and send requests. Messages are JSON inside relay frames (`proto`).

mod proto;

use proto::{audience, ToClient, ToHost};
use raft_relay::{ClientIncoming, ClientLink, HostIncoming, HostLink, RelayClient, BROADCAST};
use raft_sim::world::MAX_DT;
use raft_sim::{DynamicSnapshot, Event, Ground, PlayerId, Request, SectionId, World};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::hash::{BuildHasher, DefaultHasher, Hasher, RandomState};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread;

/// Bump on any wire change (messages, or the sim types they carry).
pub const PROTOCOL_VERSION: u32 = 3;

/// Slow sections and own player entries: checked at 4 Hz, sent when changed.
const SECTION_PERIOD: f64 = 0.25;
const SNAPSHOT_PERIOD: f64 = 1.0 / 15.0;
/// Client movement (PlayerTransform, BoatTransform).
const POSE_PERIOD: f64 = 1.0 / 20.0;
/// Messages per second taken from one client; the excess is ignored.
const MAX_MSGS_PER_SEC: u32 = 200;
/// Longest client message read; real ones are under 300 bytes.
const MAX_IN_BYTES: usize = 1024;
/// Player entries a world may hold before clients' new names are refused. Keeps the Welcome
/// inside one relay frame and the save small.
const MAX_ENTRIES: usize = 16;

/// 128 random bits as 32 hex digits: the install secret for `Session::join` (make one per
/// install and keep it), or a world key.
pub fn new_token() -> String {
    format!("{:016x}{:016x}", random(), random())
}

/// Random bits from std's OS-seeded hasher keys.
fn random() -> u64 {
    RandomState::new().build_hasher().finish()
}

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
    /// `secret` (see `new_token`, the same one every time) proves this install owns `name`
    /// when it comes back. It never leaves the client: each world gets a token derived from
    /// it and that world's key. A secret that is not 32 hex digits gives `Closed("bad token")`.
    pub fn join(relay_addr: &str, code: &str, name: &str, secret: &str) -> Session {
        let code = code.trim().to_ascii_uppercase();
        let secret = parse_secret(secret);
        let net = if secret.is_some() {
            let (tx, rx) = mpsc::channel();
            let (addr, room) = (relay_addr.to_string(), code.clone());
            thread::spawn(move || {
                let link = RelayClient::join(addr, &room).map(|(_, link)| link);
                let _ = tx.send(link.map_err(|e| e.to_string()));
            });
            ClientNet::Connecting(rx)
        } else {
            ClientNet::Closed("bad token".into())
        };
        Session(Role::Client(Client {
            code,
            name: name.to_string(),
            secret: secret.unwrap_or_default(),
            net,
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
    /// connected (by name and token) and resyncs every client with a fresh Welcome. The world
    /// key stays; call `restore_owners` with the save as well. No-op on clients.
    pub fn replace_world(&mut self, world: World) {
        if let Role::Host(h) = &mut self.0 {
            h.replace_world(world);
        }
    }

    /// Host only: the world JSON for saving, plus the world key and who owns each client's
    /// entry (token hashes). `World::load` reads it as before; pass it to `restore_owners` too.
    pub fn save_json(&self) -> Option<String> {
        match &self.0 {
            Role::Host(h) => Some(h.save_json()),
            Role::Client(_) => None,
        }
    }

    /// Host only: takes the world key and entry owners from a `save_json` string. Call it after
    /// `solo`, `host` or `replace_world` with a world loaded from that save; without it no
    /// client can take back its entry. A save without a key gets a fresh one, so its owners
    /// stay locked. When the key changes, connected clients are challenged again; clients
    /// playing a name the save gives to another token are disconnected.
    pub fn restore_owners(&mut self, save_json: &str) {
        if let Role::Host(h) = &mut self.0 {
            h.restore_owners(save_json);
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
    /// Token of the accepted Hello (this world's token for the client's secret).
    token: String,
    /// Challenges sent that no Hello answered yet. Hellos answer them in order.
    challenges: u32,
    /// Hash of the last own entry sent.
    entry: Option<u64>,
    /// Messages in the current one-second window.
    msgs: u32,
}

/// Who may play an entry: a keyed hash of the client token, so saves never hold tokens.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
struct Owner {
    key: [u64; 2],
    hash: u64,
}

impl Owner {
    fn new(token: &str) -> Owner {
        let key = [random(), random()];
        Owner {
            key,
            hash: sip(key, &[token.as_bytes()]),
        }
    }

    fn matches(&self, token: &str) -> bool {
        sip(self.key, &[token.as_bytes()]) == self.hash
    }
}

/// SipHash-2-4 of the concatenated `parts`. std's SipHasher is that for good; DefaultHasher
/// may change between Rust releases, which would lock everyone out of their saved entries.
#[allow(deprecated)]
fn sip(key: [u64; 2], parts: &[&[u8]]) -> u64 {
    let mut h = std::hash::SipHasher::new_with_keys(key[0], key[1]);
    for part in parts {
        h.write(part);
    }
    h.finish()
}

/// The token `secret` shows the world with `world_key`: 128 bits from two SipHash runs keyed
/// by the secret. Tokens for different worlds say nothing about each other or the secret.
fn derive(secret: [u64; 2], world_key: &str) -> String {
    let half = |tag: &[u8]| sip(secret, &[tag, world_key.as_bytes()]);
    format!(
        "{:016x}{:016x}",
        half(b"raft_net token 0"),
        half(b"raft_net token 1")
    )
}

fn parse_secret(secret: &str) -> Option<[u64; 2]> {
    if !valid_token(secret) {
        return None;
    }
    let half = |s| u64::from_str_radix(s, 16).ok();
    Some([half(&secret[..16])?, half(&secret[16..])?])
}

/// 32 hex digits, as `new_token` makes them.
fn valid_token(token: &str) -> bool {
    token.len() == 32 && token.bytes().all(|b| b.is_ascii_hexdigit())
}

fn has_entry(world: &World, name: &str) -> bool {
    world.players.values().any(|p| p.name == name)
}

struct Host {
    world: World,
    name: String,
    me: Option<PlayerId>,
    net: HostNet,
    /// Relay client id -> peer.
    peers: BTreeMap<u8, Peer>,
    /// Entry name -> the client that created it. Entries without one (the host's own, or a
    /// save whose owners were not restored) cannot be taken by clients.
    owners: BTreeMap<String, Owner>,
    /// Sent in every Challenge. Client tokens derive from it, so each world sees its own
    /// token per install. Saved with the owners.
    world_key: String,
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
            owners: BTreeMap::new(),
            world_key: new_token(),
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
                HostIncoming::Joined(id) => self.joined(id),
                HostIncoming::Left(id) => self.left(id),
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

    fn joined(&mut self, id: u8) {
        // A reused id whose Left has not arrived: free the old player.
        self.left(id);
        self.peers.insert(id, Peer::default());
        self.challenge(id);
    }

    fn left(&mut self, id: u8) {
        if let Some(pid) = self.peers.remove(&id).and_then(|p| p.player) {
            self.world.leave(pid);
        }
    }

    /// Asks client `id` for its token under the current world key; it plays no entry until
    /// its Hello answers.
    fn challenge(&mut self, id: u8) {
        let Some(peer) = self.peers.get_mut(&id) else {
            return;
        };
        if let Some(pid) = peer.player.take() {
            self.world.leave(pid);
        }
        peer.challenges += 1;
        let world_key = self.world_key.clone();
        self.send(id, &ToClient::Challenge { world_key });
    }

    fn on_data(&mut self, from: u8, bytes: &[u8], ground: &dyn Ground, events: &mut Vec<Event>) {
        let Some(peer) = self.peers.get_mut(&from) else {
            return;
        };
        peer.msgs += 1;
        if peer.msgs > MAX_MSGS_PER_SEC || bytes.len() > MAX_IN_BYTES {
            return;
        }
        // Malformed or out-of-order messages are ignored. Requests act as the sender's own
        // player, found by relay client id; payloads carry no player to act as.
        match (serde_json::from_slice::<ToHost>(bytes), peer.player) {
            (
                Ok(ToHost::Hello {
                    version,
                    name,
                    token,
                }),
                None,
            ) => {
                let open = peer.challenges;
                peer.challenges = open.saturating_sub(1);
                let token = token.to_ascii_lowercase();
                match open {
                    _ if version != PROTOCOL_VERSION => self.reject(from, "Version mismatch"),
                    1 if valid_token(&token) => {
                        let _ = self.admit(from, &name, &token);
                    }
                    // No Challenge to answer, or not a token.
                    0 | 1 => self.reject(from, "bad token"),
                    // Answers an older Challenge (world key); the latest one is on its way.
                    _ => {}
                }
            }
            (Ok(ToHost::Req(req) | ToHost::Pose(req)), Some(pid)) => {
                events.extend(self.world.apply(pid, req, ground));
            }
            _ => {}
        }
    }

    fn reject(&self, id: u8, reason: &str) {
        let reason = reason.into();
        self.send(id, &ToClient::Reject { reason });
    }

    /// Joins `name` for relay client `id` if `token` may play it, and answers with the full
    /// world or a Reject.
    fn admit(&mut self, id: u8, name: &str, token: &str) -> Result<PlayerId, &'static str> {
        if !self.peers.contains_key(&id) {
            return Err("unknown client");
        }
        let name = name.trim();
        let known = has_entry(&self.world, name);
        let joined = match self.owners.get(name) {
            Some(owner) if !owner.matches(token) => Err("name taken"),
            None if known => Err("name taken"),
            _ if !known && self.world.players.len() >= MAX_ENTRIES => Err("world full"),
            _ => self.world.join(name),
        };
        if joined.is_ok() {
            let owner = Owner::new(token);
            self.owners.entry(name.to_string()).or_insert(owner);
        }
        if let Some(peer) = self.peers.get_mut(&id) {
            peer.player = joined.ok();
            peer.token = token.to_string();
            peer.entry = None;
        }
        match joined {
            Ok(pid) => self.send(
                id,
                &ToClient::Welcome {
                    player_id: pid,
                    world: Box::new(self.world.clone()),
                },
            ),
            Err(e) => self.reject(id, e),
        }
        joined
    }

    fn replace_world(&mut self, world: World) {
        let back: Vec<(u8, String, String)> = self
            .peers
            .iter()
            .filter_map(|(&id, p)| {
                let name = self.world.player(p.player?)?.name.clone();
                Some((id, name, p.token.clone()))
            })
            .collect();
        let (world, me) = adopt(world, &self.name);
        self.world = world;
        self.me = me.ok();
        self.owners.retain(|name, _| has_entry(&self.world, name));
        self.sections = Default::default();
        for p in self.peers.values_mut() {
            p.player = None;
        }
        for (id, name, token) in back {
            let _ = self.admit(id, &name, &token);
        }
    }

    fn save_json(&self) -> String {
        #[derive(Serialize)]
        struct Save<'a> {
            #[serde(flatten)]
            world: &'a World,
            net_world_key: &'a str,
            net_owners: BTreeMap<&'a String, &'a Owner>,
        }
        let net_owners = self
            .owners
            .iter()
            .filter(|(name, _)| has_entry(&self.world, name))
            .collect();
        let save = Save {
            world: &self.world,
            net_world_key: &self.world_key,
            net_owners,
        };
        serde_json::to_string(&save).expect("world types always serialize")
    }

    fn restore_owners(&mut self, save_json: &str) {
        #[derive(Deserialize)]
        struct Saved {
            #[serde(default)]
            net_world_key: String,
            #[serde(default)]
            net_owners: BTreeMap<String, Owner>,
        }
        let Ok(saved) = serde_json::from_str::<Saved>(save_json) else {
            return;
        };
        self.owners.extend(saved.net_owners);
        // Without a key the save's owners match no token: its entries stay locked.
        let key = match saved.net_world_key.to_ascii_lowercase() {
            key if valid_token(&key) => key,
            _ => new_token(),
        };
        if key != self.world_key {
            self.world_key = key;
            let ids: Vec<u8> = self.peers.keys().copied().collect();
            for id in ids {
                self.challenge(id);
            }
            return;
        }
        let lost: Vec<(u8, PlayerId)> = self
            .peers
            .iter()
            .filter_map(|(&id, p)| {
                let pid = p.player?;
                let name = &self.world.player(pid)?.name;
                let owned = self.owners.get(name).is_some_and(|o| o.matches(&p.token));
                (!owned).then_some((id, pid))
            })
            .collect();
        for (id, pid) in lost {
            self.world.leave(pid);
            if let Some(p) = self.peers.get_mut(&id) {
                p.player = None;
            }
            self.reject(id, "name taken");
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
    name: String,
    secret: [u64; 2],
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
            // Also after the host loads a save; the Welcome that follows resyncs.
            ToClient::Challenge { world_key } => self.send(&ToHost::Hello {
                version: PROTOCOL_VERSION,
                name: self.name.clone(),
                token: derive(self.secret, &world_key),
            }),
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
    use raft_sim::{FlatGround, V3};

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
            name: "Ana".into(),
            secret: [1, 2],
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

    fn offline_host() -> Session {
        let (world, me) = adopt(World::new(1), "Hana");
        Session(Role::Host(Host::new(
            world,
            "Hana",
            me.ok(),
            HostNet::Offline,
        )))
    }

    fn host_of(s: &mut Session) -> &mut Host {
        match &mut s.0 {
            Role::Host(h) => h,
            Role::Client(_) => unreachable!(),
        }
    }

    /// Relay client `id` connects and answers the Challenge; the player it got, if any.
    fn hello(h: &mut Host, id: u8, name: &str, token: &str) -> Option<PlayerId> {
        h.joined(id);
        answer(h, id, name, token)
    }

    /// Relay client `id` sends a Hello; its player afterwards.
    fn answer(h: &mut Host, id: u8, name: &str, token: &str) -> Option<PlayerId> {
        let msg = encode(&ToHost::Hello {
            version: PROTOCOL_VERSION,
            name: name.into(),
            token: token.into(),
        });
        h.on_data(id, &msg, &FlatGround::WATER, &mut Vec::new());
        h.peers[&id].player
    }

    #[test]
    fn returning_names_need_their_token() {
        let (ta, tb) = (new_token(), new_token());
        assert!(ta != tb && valid_token(&ta));
        let mut s = offline_host();
        let h = host_of(&mut s);
        let ana = hello(h, 1, "Ana", &ta).unwrap();
        h.world.player_mut(ana).unwrap().coins = 50;
        h.left(1);
        assert_eq!(hello(h, 2, "Ana", &tb), None, "impostor");
        assert_eq!(h.admit(2, " Ana ", &tb), Err("name taken"));
        assert_eq!(
            h.admit(2, "Hana", &tb),
            Err("name taken"),
            "the host's entry"
        );
        assert_eq!(hello(h, 3, "Cy", "1234"), None, "bad token");
        assert!(!has_entry(&h.world, "Cy"));
        assert_eq!(hello(h, 2, "Ana", &ta.to_uppercase()), Some(ana));

        // Requests act as the sender's player, whatever they carry.
        let bo = hello(h, 4, "Bo", &tb).unwrap();
        let to = V3::new(5.0, 0.0, 30.0);
        let pose = encode(&ToHost::Pose(Request::PlayerTransform {
            pos: to,
            yaw: 0.0,
            in_water: true,
            sprinting: false,
        }));
        h.on_data(4, &pose, &FlatGround::WATER, &mut Vec::new());
        assert!(h.world.player(bo).unwrap().pos.dist(to) < 0.01);
        assert!(h.world.player(ana).unwrap().pos.dist(to) > 1.0);

        // Owners go into the save as keyed hashes and come back with `restore_owners`.
        let json = s.save_json().unwrap();
        assert!(!json.contains(&ta) && !json.contains(&tb));
        let mut s = Session::solo(World::load(&json).unwrap(), "Hana").unwrap();
        s.restore_owners(&json);
        let h = host_of(&mut s);
        assert_eq!(hello(h, 1, "Ana", &tb), None);
        assert_eq!(hello(h, 2, "Ana", &ta), Some(ana));
        assert_eq!(h.world.player(ana).unwrap().coins, 50);

        // An impostor who made "Ana" in a fresh world loses it once that save is restored.
        let mut s = offline_host();
        hello(host_of(&mut s), 1, "Ana", &tb).unwrap();
        s.replace_world(World::load(&json).unwrap());
        assert!(host_of(&mut s).peers[&1].player.is_some());
        s.restore_owners(&json);
        let h = host_of(&mut s);
        assert_eq!(h.peers[&1].player, None);
        assert!(!h.world.player(ana).unwrap().connected);
        assert_eq!(hello(h, 2, "Ana", &ta), Some(ana));
    }

    #[test]
    fn clients_cannot_grow_the_world() {
        let t = new_token();
        let mut s = offline_host();
        let h = host_of(&mut s);
        let padded = format!("{}Eve", " ".repeat(MAX_IN_BYTES));
        assert_eq!(hello(h, 1, &padded, &t), None, "oversized message");
        assert!(hello(h, 1, "Eve", &t).is_some());

        // One client cycling through names.
        for i in 0.. {
            h.left(1);
            if hello(h, 1, &format!("P{i}"), &t).is_none() {
                break;
            }
        }
        assert_eq!(h.world.players.len(), MAX_ENTRIES);
        assert_eq!(h.admit(1, "Newcomer", &t), Err("world full"));
        assert!(
            hello(h, 2, "Eve", &t).is_some(),
            "returning players still fit"
        );
        let welcome = encode(&ToClient::Welcome {
            player_id: 0,
            world: Box::new(h.world.clone()),
        });
        assert!(
            welcome.len() < raft_relay::MAX_DATA / 2,
            "{}",
            welcome.len()
        );
    }

    #[test]
    fn tokens_are_per_world() {
        let secret = parse_secret(&new_token().to_uppercase()).unwrap();
        assert_eq!(parse_secret("12g4"), None);
        let (mut a, mut b) = (offline_host(), offline_host());
        let (a, b) = (host_of(&mut a), host_of(&mut b));
        let (ta, tb) = (derive(secret, &a.world_key), derive(secret, &b.world_key));
        assert!(ta != tb && valid_token(&ta) && valid_token(&tb));
        assert_eq!(derive(secret, &b.world_key), tb);
        let ana = hello(b, 1, "Ana", &tb).unwrap();
        b.left(1);
        // Ana's token for A, replayed on B by host A or anyone on A's path.
        assert_eq!(hello(b, 2, "Ana", &ta), None);
        assert_eq!(b.admit(2, "Ana", &ta), Err("name taken"));
        assert_eq!(hello(b, 2, "Ana", &tb), Some(ana));

        // One Hello per Challenge: none before it, no second try after a refusal.
        b.peers.insert(3, Peer::default());
        assert_eq!(answer(b, 3, "Cy", &new_token()), None);
        assert_eq!(hello(b, 4, "Hana", &new_token()), None);
        assert_eq!(answer(b, 4, "Cy", &new_token()), None);
        assert!(!has_entry(&b.world, "Cy"));
    }

    #[test]
    fn loading_another_worlds_save_asks_again() {
        let (ana_secret, bo_secret) = (parse_secret(&new_token()), parse_secret(&new_token()));
        let (ana_secret, bo_secret) = (ana_secret.unwrap(), bo_secret.unwrap());
        let mut first = offline_host();
        let k1 = host_of(&mut first).world_key.clone();
        let ana = hello(host_of(&mut first), 1, "Ana", &derive(ana_secret, &k1)).unwrap();
        let save = first.save_json().unwrap();

        let mut s = offline_host();
        let h = host_of(&mut s);
        let k2 = h.world_key.clone();
        hello(h, 1, "Ana", &derive(ana_secret, &k2)).unwrap();
        h.joined(2); // Bo's answer for k2 is still on its way.
        s.replace_world(World::load(&save).unwrap());
        s.restore_owners(&save);
        let h = host_of(&mut s);
        assert_eq!(h.world_key, k1);
        assert_eq!(h.peers[&1].player, None, "challenged again");
        assert_eq!(answer(h, 2, "Bo", &derive(bo_secret, &k2)), None, "old");
        assert!(answer(h, 2, "Bo", &derive(bo_secret, &k1)).is_some());
        assert_eq!(answer(h, 1, "Ana", &derive(ana_secret, &k1)), Some(ana));

        // A save without a key: its owners stay locked.
        let old = save.replace("net_world_key", "old_key");
        s.restore_owners(&old);
        let h = host_of(&mut s);
        let k3 = h.world_key.clone();
        assert!(k3 != k1 && valid_token(&k3));
        assert_eq!(answer(h, 1, "Ana", &derive(ana_secret, &k3)), None);
    }
}
