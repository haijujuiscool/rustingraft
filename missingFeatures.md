# Missing in RustingEngine

Things Rusting Raft needed that RustingEngine does not have yet (checked against
RustingEngine 2.3.0, 61ab8e9). Each entry says what the game wrote instead, so the
workaround can go once the engine has the feature.

## Features

### Faster raycasts (broadphase)

`CpuPhysics::raycast_where` (`src/runtime/cpu_physics.rs:694`) tests every collider for
every ray. The world has about 1,500 objects, and the game samples island heights with
about 15,000 downward rays: done in one go it froze the game for over 5 seconds, long
enough to time out an online connect.

- Instead: `src/ground.rs` samples a few rows per main-menu tick and finishes the rest
  before a session starts.
- Wanted: a BVH or grid over static colliders, so a ray only tests the colliders near it.

### Raycast that keeps only some objects

`GameScene::raycast_skipping` skips classes, but there is no "only hit class X" and no
filter by name. The game's moving objects (`debris.*`, `enemy.*`, `avatar.*`, pooled and
spawned copies) are not in classes.

- Instead: `src/ground.rs` and `src/player/mod.rs` (`fn ray`) re-cast past unwanted hits
  up to 4 or 8 times, checking `hit.name` each time.
- Wanted: `raycast_in_class(origin, dir, max, class)` and/or a `raycast_where` on
  `GameScene` that takes a closure over the hit name or entity.

### Per-vertex colour in `MeshVertex`

`MeshVertex` (`src/assets/mod.rs:907`) has position, normal, uv and tangent, no colour.

- Instead: the ocean shallows (`src/view/ocean.rs`) put a 32x1 gradient texture made with
  `scene.create_texture` on the mesh and pick the colour through the UVs.
- Wanted: an optional `color: [f32; 4]` per vertex, multiplied into the base colour.

### Third-person camera collision that includes the shoulder offset

The controller's camera sweep (`src/runtime/player.rs`, `CAMERA_RADIUS`) starts at the
orbit centre and ignores the camera offset, so an over-the-shoulder camera clips into
walls beside the player.

- Instead: `third_person` in `src/player/mod.rs` casts from the centre to the shoulder,
  then from the shoulder backwards, and sets the offset and distance every frame.
- Wanted: the controller sweeps centre -> offset -> camera itself.

### Hardened relay

The engine relay (`run_relay`, `src/net.rs`) is fine on a LAN but not for a relay open to
the internet on a home server: one lock over every room, no per-IP connection or rate
limits, no bandwidth cap, no limit on wrong-token guesses (its own `ponytail:` note says
so), plaintext passwords.

- Instead: Rusting Raft ships its own std-only relay (`relay/`, `docs/relay.md`) with
  per-IP limits, join and room rate limits, a handshake deadline, per-connection
  bandwidth caps and send queues, and a systemd sandbox unit
  (`deploy/rusting-relay.service`). The game client talks to it with `net/` (`raft_net`).
- Wanted: those limits in `run_relay`, so games can use `NetSession::host_room` /
  `join_room` directly.

### Scenario step for the mouse wheel

Scenario steps (`StepAction`, `src/scenario.rs:1038`) can move the pointer, click a
label and press actions, but cannot turn the mouse wheel. The game's long windows
(Workbench, Logbook) sit in an egui `ScrollArea` (`src/ui/windows.rs:75`), and `click`
only finds text that is visible, so rows below the fold (Wood Farm, Radio Station) cannot
be clicked by label.

- Instead: `tests/flow_05_farm.json` and `tests/flow_06_radio.json` drag the scroll bar:
  a `pointer` step on the bar, `press` `use` (bound to `MouseLeft`), more `pointer`
  steps, then `release`. This depends on the bar's pixel position.
- Wanted: a `{"scroll": [dx, dy]}` step, or a `click` that scrolls the target into view
  first.

## Bugs seen

- `run_relay` returns on the first failed `accept` (`let stream = stream?;`,
  `src/net.rs:359`). A transient error such as running out of file descriptors stops
  the whole relay; `continue` (maybe after a short sleep) keeps it serving.
- `NetSession::host_on` runs each handshake on the accept thread (noted in its
  `ponytail:` comment), so one silent client delays every join by up to 5 seconds.
- Flat shading (`src/rendering/scene_renderer.rs:8771`) calls `dFdx`/`dFdy` inside an
  `if` on a varying. The flag is per material, so it works in practice, but derivatives
  in non-uniform control flow are undefined in GLSL; computing `face` before the `if`
  is safe on every driver.
