# raft_net

Sessions for Rusting Raft (spec 3.2). The host (solo, or online through `raft_relay`) runs
the authoritative `raft_sim::World`. Clients keep a mirror of it and send requests. No
engine dependency.

## API

```rust
pub const PROTOCOL_VERSION: u32;
pub enum Status { Connecting, Playing, Closed(String) }

impl Session {
    pub fn solo(world: World, name: &str) -> Result<Session, String>;
    pub fn host(relay_addr: &str, world: World, name: &str) -> Session;
    pub fn join(relay_addr: &str, code: &str, name: &str) -> Session;
    pub fn update(&mut self, dt: f32, ground: &dyn Ground) -> Vec<Event>; // once per fixed tick
    pub fn request(&mut self, req: Request);
    pub fn world(&self) -> Option<&World>;
    pub fn me(&self) -> Option<PlayerId>;
    pub fn is_host(&self) -> bool;
    pub fn room_code(&self) -> Option<&str>;
    pub fn status(&self) -> Status;
    pub fn replace_world(&mut self, world: World); // host only
    pub fn save_json(&self) -> Option<String>;     // host only
    pub fn peer_count(&self) -> usize;
}
```

- `solo`, `host` and `replace_world` disconnect every player in the given world, then join
  the local player.
- On the host, `request` is queued. The next `update` applies it, and that `update` returns
  its events. The host `update` returns every event of its world. Filter by `me()`.
- On a client, `world()` and `me()` are `None` until the Welcome arrives. Movement requests
  apply to the mirror at once.
- Clients keep their own pose (and the pose of the boat they drive) instead of the host
  snapshot. A `PlayerTeleport` event for the client moves it.
- Link loss on a client gives `Closed("Host left")`. The host stays `Playing` after its
  clients leave.

## Protocol

Each relay frame holds one JSON message.

Client to host (`ToHost`):

- `Hello { version, name }`: first message. A wrong version gets `Reject("Version mismatch")`.
  A refused name gets `Reject` with the reason from `World::join`.
- `Req(Request)`: any request except movement. Sent at once.
- `Pose(Request)`: the latest `PlayerTransform` or `BoatTransform`.

Host to client (`ToClient`):

- `Welcome { player_id, world }`: the full world. Sent again after `replace_world`.
- `Reject { reason }`
- `Section { id, value }`: one slow section (`World::section`), broadcast.
- `Me(value)`: the receiver's own full player entry.
- `Snapshot(DynamicSnapshot)`: broadcast.
- `Events(Vec<Event>)`: per client, filtered by `audience()` in `src/proto.rs`. Personal
  events go only to their player. `SaveRequested` stays on the host. Everything else goes
  to all.

The host ignores malformed messages, and requests sent before an accepted Hello.

## Rates (simulation time)

| Message | Rate |
| --- | --- |
| Sections, own entry | checked at 4 Hz, sent only when changed |
| Snapshot | 15 Hz |
| Client movement | 20 Hz, latest pose only |
| Events, other requests | at once |
| Client message cap | 200 per second, the excess is ignored |

The host sends nothing until one client has a Welcome.

## Tests

```
cargo test -p raft_net
```

`tests/sessions.rs` runs a local relay with a host and two clients.
