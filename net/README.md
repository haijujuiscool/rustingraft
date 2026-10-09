# raft_net

Sessions for Rusting Raft (spec 3.2). The host (solo, or online through `raft_relay`) runs
the authoritative `raft_sim::World`. Clients keep a mirror of it and send requests. No
engine dependency.

## API

```rust
pub const PROTOCOL_VERSION: u32; // 3
pub fn new_token() -> String;      // 128 random bits as 32 hex digits
pub enum Status { Connecting, Playing, Closed(String) }

impl Session {
    pub fn solo(world: World, name: &str) -> Result<Session, String>;
    pub fn host(relay_addr: &str, world: World, name: &str) -> Session;
    pub fn join(relay_addr: &str, code: &str, name: &str, secret: &str) -> Session;
    pub fn update(&mut self, dt: f32, ground: &dyn Ground) -> Vec<Event>; // once per fixed tick
    pub fn request(&mut self, req: Request);
    pub fn world(&self) -> Option<&World>;
    pub fn me(&self) -> Option<PlayerId>;
    pub fn is_host(&self) -> bool;
    pub fn room_code(&self) -> Option<&str>;
    pub fn status(&self) -> Status;
    pub fn replace_world(&mut self, world: World); // host only
    pub fn save_json(&self) -> Option<String>;     // host only
    pub fn restore_owners(&mut self, save_json: &str); // host only
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
- A client passes the same `secret` every time. Make it once per install with
  `new_token()` and keep it in the settings. A secret that is not 32 hex digits gives
  `Closed("bad token")` at once.
- Link loss on a client gives `Closed("Host left")`. The host stays `Playing` after its
  clients leave.

## Identity and limits

- Each world has a random 128-bit world key. The client never sends its secret. It
  answers the host's `Challenge` with a token derived from the secret and the world key:
  two SipHash-2-4 runs keyed by the secret, one per 64-bit half, each over a domain tag
  and the world key. The same install shows each world a different token, so a token
  seen by one host, by the relay or on the network does not work on another world.
- The host stores an owner for each entry a client creates: a keyed SipHash-2-4 of the
  token under a random key. The token itself is never saved.
- A name that already has an entry is given only to the token that created it. Otherwise
  the client gets `Reject("name taken")`. Clients cannot take entries without an owner
  (the host's own entries, or a save whose owners were not restored).
- `save_json` adds `net_world_key` and a `net_owners` object to the world JSON, and
  `World::load` ignores them. After `solo`, `host` or `replace_world` with a loaded save,
  call `restore_owners(&save_json)`. Without it, nobody can take back an entry from that
  save. `replace_world` keeps the current key.
- `restore_owners` takes the save's key. A save without one gets a fresh key, so its old
  owners stay locked. When the key changes, the host challenges every connected client
  again, and each plays on after its new Hello. If a connected client plays a name that
  the save gives to another token, the host disconnects that client.
- Clients can create new entries only while the world holds fewer than 16. After that, a
  new name gets `Reject("world full")`. Returning names still join.
- Each request acts as the sender's own player, which the host finds by relay client id.
  No payload names a player to act as.
- The host drops a client message longer than 1024 bytes without reading it.

Known gaps:

- A host that once joined world W as a client knows W's key. In its own room it can send
  that key as its `Challenge` and collect, from each player who joins, that player's
  token for W, then play their entries on W. Players expose their W token only to the
  rooms they join.
- Traffic is not encrypted. Anyone who sees a player's traffic with world W (including
  the relay operator) can replay that player's W token on W itself.

## Protocol

Each relay frame holds one JSON message.

Client to host (`ToHost`):

- `Hello { version, name, token }`: the answer to each `Challenge`, with the token for
  its world key. A wrong version gets `Reject("Version mismatch")`. A Hello with no
  `Challenge` to answer, or a token that is not 32 hex digits, gets `Reject("bad token")`.
  Hellos answer Challenges in order, and the host ignores answers to an older one. A
  refused name gets `Reject` with `"name taken"`, `"world full"` or the reason from
  `World::join`.
- `Req(Request)`: any request except movement. Sent at once.
- `Pose(Request)`: the latest `PlayerTransform` or `BoatTransform`.

Host to client (`ToClient`):

- `Challenge { world_key }`: first message to each client, and again when
  `restore_owners` changes the key. Its shape must stay the same across versions, so that
  every client can answer with its version.
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

Apart from `Challenge` and `Reject`, the host sends nothing until one client has a Welcome.

## Tests

```
cargo test -p raft_net
```

`tests/sessions.rs` runs a local relay with a host and two clients.
