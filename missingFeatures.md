# Missing in RustingEngine

Things Rusting Raft needed that RustingEngine does not have yet (checked against
RustingEngine 2.4.1, 10e9702). Each entry says what the game wrote instead, so the
workaround can go once the engine has the feature.

## Features

### Raycasts before the first physics step

`GameScene::raycast` reads `PhysicsWorld` bodies, which only `step_cpu_physics` fills
(`set_bodies`, `src/runtime/cpu_physics.rs:1948`). On the first fixed tick
(`fixed_tick` 0) the game's tick sees no collider at all. Child colliders (the island
ground) also take the `GlobalTransform` propagated after the previous frame
(`src/runtime/cpu_physics.rs:2196`), and the first windowed frame runs several steps
(up to 8) before any propagation, so their bodies sit at unpropagated poses until a step
in a later frame: a grid sampled at `fixed_tick` 1 found 484 of 8660 heights.
Scenarios do not show it, since their tick 0 is a zero-delta frame that propagates first.

- Instead: `tick` in `src/app.rs` builds `SceneGround` once `fixed_tick` is past the
  value it had in the first frame, and the session waits for it.
- Wanted: bodies built and posed from propagated `GlobalTransform`s when the scene loads,
  so rays hit from tick 0.

### Reading the pulled third-person shoulder

The controller's camera sweep (`player_look`, `src/runtime/player.rs:367`) pulls the
over-the-shoulder offset in from walls beside the player, then the camera in from walls
behind, but keeps the pulled share of the offset (`shoulder`) to itself. The game's aim
starts at the shoulder, so it needs that point.

- Instead: `shoulder` in `src/player/mod.rs` repeats the centre-to-shoulder ray (0.3 m off
  walls, not the engine's 0.2 m sphere) every frame, sets the pulled offset and aims from
  it. The camera distance is left to the engine.
- Wanted: the pulled offset or pivot readable, such as a `PlayerController` field that
  `player_look` writes.

### Hardened relay

2.4.0's `run_relay_with` (`src/net.rs:863`, `RelayLimits`) caps connections overall and
per address (IPv6 per /64), new connections per address per minute and the read rate per
connection, skips failed accepts, and gives each handshake its own thread and a deadline.
Still missing next to the game's relay: forwarding writes straight to each socket while
holding the one lock over every room (its `ponytail:` note, `src/net.rs:829`; `forward`,
`src/net.rs:1048`), so a peer that stops reading stalls forwarding, room opens and joins
in every room on the relay for up to 5 s (the write timeout) instead of filling its own
send queue; no cap on open rooms or on clients in a room; no separate limit on room opens
and join attempts; no idle timeout after the handshake; a sender over the rate is slowed,
not dropped.

- Instead: Rusting Raft ships its own std-only relay (`relay/`, `docs/relay.md`) with
  per-IP limits, join and room rate limits, a handshake deadline, per-connection
  bandwidth caps and send queues, and a systemd sandbox unit
  (`deploy/rusting-relay.service`). The game client talks to it with `net/` (`raft_net`),
  whose protocol differs from `NetSession`'s.
- Wanted: those limits in `run_relay`, so games can use `NetSession::host_room` /
  `join_room` directly.

### Scenario step for the mouse wheel

Scenario steps (`StepAction`, `src/scenario.rs:1046`) can move the pointer, click a
label and press actions, but cannot turn the mouse wheel. The game's long windows
(Workbench, Logbook) sit in an egui `ScrollArea` (`src/ui/windows.rs:75`), and `click`
only finds text that is visible, so rows below the fold (Wood Farm, Radio Station) cannot
be clicked by label.

- Instead: `tests/flow_05_farm.json` and `tests/flow_06_radio.json` drag the scroll bar:
  a `pointer` step on the bar, `press` `use` (bound to `MouseLeft`), more `pointer`
  steps, then `release`. This depends on the bar's pixel position.
- Wanted: a `{"scroll": [dx, dy]}` step, or a `click` that scrolls the target into view
  first.

### Cursor grab that follows mouse look and focus

The controller's look code (`player_look`, `src/runtime/player.rs:326`) frees the cursor
when mouse look goes off or Esc goes down, but grabs it again only on the next left click.
So after a window closes (`GameScene::set_mouse_look(.., true)`,
`src/project_runner.rs:872`) the camera does not turn until the player clicks. Losing focus
also keeps the grab: `RuntimeInput::record_focus` (`crates/rusting-core/src/input.rs:235`)
releases keys only, and `GameScene` has no call to grab or free the cursor.

- Instead: `cursor` in `src/app.rs` writes `RuntimeInput::set_cursor_captured` through
  `scene.world().resource_mut::<RuntimeInput>()` every frame: grabbed while mouse look is
  on, so a window closing or an Esc that opens nothing (placing a part) keeps the camera
  turning. Focus loss or the Windows key frees it until the next click, and that click
  does not fire the held tool. The engine's look code runs after the game's update, so
  an Esc the game used still frees the cursor for that one frame.
- Wanted: `GameScene::set_cursor_captured(bool)`, a grab when `set_mouse_look` turns
  look on while the window has focus, a release on focus loss, and a way for the game to
  keep an Esc it used (cancel placing a part) from freeing the cursor.

### Scenario step for a raw key

Scenario `press` and `tap` (`StepAction`, `src/scenario.rs:1046`) take a named action
only. A key that no action uses, such as the Windows key (`SuperLeft`), cannot be pressed
in a scenario.

- Instead: the unit test `cursor_follows_look_and_stays_free_after_focus_loss_until_a_click` in
  `src/app.rs` covers the Windows key; `tests/ui_cursor.json` covers focus loss.
- Wanted: a `{"key": "SuperLeft"}` step that presses one key code.

### Quaternion-safe animation blending

Clip sampling (`AnimationTrack::sample`, `lerp` in `src/runtime/animation.rs:265`) and
the crossfade (`mix`, `src/runtime/animation.rs:591`) lerp Orientation keys component by
component, with no sign check (the result is only normalised when written). Two keys for
the same rotation with opposite signs (q and -q) pass near zero in between, and the bone
collapses or spins. A
crossfade also keeps a track that only the old clip keys at full weight, so that bone
stays where the old clip left it.

- Instead: the hand-keyed "swim" and "tread" clips (`swim_clips` in
  `tools/scene/build_scene.py`) keep w <= 0 like the imported clips, add a second key at
  the same time wherever the sign flips, and key Position and Orientation on every bone.
  The crawl arms are keyed as a swing from rest with no twist (`stroke`), so a full
  stroke ends on the same key instead of the negated one. Every clip on a swimming human
  also gets rest keys for the bones it leaves alone, so the swim roll does not stay on
  after the fade into a clip that never keys the root.
- Wanted: nlerp or slerp with a shortest-path sign flip for Orientation in `sample` and
  `mix`, and a crossfade that eases tracks only one side has back to rest.

## Bugs seen

- The frame cap (`FramePacer::request_next_frame`, `src/rendering/frame_pacer.rs:77`)
  sets `next_frame = now + interval` from the moment the loop wakes, so each late wake
  from `ControlFlow::WaitUntil` adds to the period and the rate settles below the cap.
  `next_frame += interval` (back to `now` only when far behind) holds the cap. The
  game's Max FPS setting (`GameScene::set_max_fps`) inherits this.
- `FrameTime::real_delta` is documented as real time since the last frame, but
  `RuntimeApp::update` (`src/runtime/mod.rs:686`) clamps it to one fixed step while the
  app is overloaded, so a frame counter built on it never reads below 60. The game's FPS
  counter (`src/ui/mod.rs`) reads `Instant` instead.
