# raft_sim contract

Pure rules crate for Rusting Raft (spec: `docs/superpowers/specs/2026-10-09-rusting-raft-design.md`).
Deps: `serde` (derive) and `serde_json` only. No engine types.

Build and test (always with the shared target dir):

```
$env:CARGO_TARGET_DIR='C:\Users\juliu\RustingRaft\target\sim'
cargo test -p raft_sim --manifest-path C:\Users\juliu\RustingRaft\Cargo.toml
cargo clippy -p raft_sim --manifest-path C:\Users\juliu\RustingRaft\Cargo.toml --all-targets
```

## Modules and ownership

| Module | Owner | Contents |
| --- | --- | --- |
| `math.rs` | core (done) | `V3`, `facing(yaw)`, `yaw_toward`, `wrap_angle`, `turn_toward` |
| `rng.rs` | core (done) | `Rng` (SplitMix64; state is saved with the world) |
| `content.rs` | core (done), balance edits by anyone | every table and tuning number |
| `inventory.rs` | core (done) | `Stack`, `Sac` (all-or-nothing add/remove), `exchange` |
| `msg.rs` | shared, append-only | `Request`, `Event`, `SoundKind`, `Target`, `Hand`, `deny/toast/sound` |
| `world.rs` | coordinator | `World`, `Dynamic`, `Ground`, mocks, join/leave, step/apply dispatch, helpers |
| `replicate.rs` | core (done) | sections, roster, `DynamicSnapshot`, JSON |
| `clock.rs` | core (done) | `Clock`, `Phase`, `phase_entered` |
| `raft.rs` | core (done) | grid geometry, `Structure`, `FarmState`, start layout |
| `save.rs` | core (done) | `SaveWorld` |
| `players.rs` | players agent | `Player`, `Gear`, `Seat`; hunger, downed/revive, hands, flares, cooking |
| `economy.rs` | economy agent | `Economy`, `Campfire`, `Shredder`; shredder, campfire, crafting, placing, farms |
| `debris.rs` | debris agent | `Debris`, `DebrisSpawn`; spawning, drift, pickup, harpoon drag |
| `enemies.rs` | enemies agent | `Enemy`, `EnemyHome`, `EnemyState`, `EnemySpawn`; spawns, AI, combat |
| `fishing.rs` | fishing agent | `Fishing`; cast, bite, reel |
| `islands.rs` | islands agent | `Islands`, `IslandState`, `Quests`, `QuestStatus`; discovery, chests, shrines, generator, beacon, quests |
| `survivors.rs` | survivors agent | `SurvivorState`; radio signals, rescue |
| `trader.rs` | trader agent | `Trader`; Barnaby buy/sell, wandering trader |
| `boats.rs` | boats agent | `Boat`, `World::boat_seats`; board, leave, driving, fuel |

Each stub module's doc comment lists the spec rules it must implement. `TODO(<system>)`
marks the work. `todo!()` is not allowed.

## Where state lives

`World` (world.rs) holds `seed, rng, next_id, game_over, clock, raft, economy, islands,
quests, trader, players: BTreeMap<PlayerId, Player>, dynamic`.
`Dynamic` holds `debris, enemies, boats` (all `BTreeMap<EntityId, _>`), `debris_spawn`,
`enemy_spawn`.

Add fields freely to the structs your module owns (keep `Serialize, Deserialize, Clone,
Debug, PartialEq`). Do not add fields to `World` or `Dynamic`; put system bookkeeping in
your own struct (`DebrisSpawn`, `EnemySpawn`, `Economy`, `Islands`, ...). Ask the
coordinator if you truly need a new top-level field.

Notable placements:
- Boat seats: only `Player::seat` (seat 0 drives). `World::boat_seats(id)` derives the list.
- Revive: the reviver holds `reviving: Option<PlayerId>` and `revive_progress`.
- Fishing state: `Player::fishing: Option<Fishing>`.
- Flares are player-held: `Player::flare_until` (no flare entities). `World::in_light`.
- Survivors on the raft: `Economy::survivors: Vec<IslandId>`; per-island `IslandState::survivor`.
- Quest progress: `Quests::entries[QuestId as usize] { status, progress }`.
- Farm state: `raft::Structure::state = StructureState::Farm(FarmState { running, progress, ready })`.
- Save counter: `Economy::save_count`. Game over flag: `World::game_over`.

## Flow

`World::step(dt, ground) -> Vec<Event>` (host, 60 Hz). Non-finite or `<= 0` dt does
nothing; dt is clamped to `MAX_DT` (0.25 s); nothing runs while `game_over`. Order:

`clock, boats, players, economy, debris, fishing, enemies, islands, survivors, trader`

Systems run after the clock, so `clock::phase_entered(ev, Phase::Dawn)` and
`ev.contains(&Event::DayStarted(d))` see this tick's phase edges.

`World::apply(pid, req, ground) -> Vec<Event>` (host). Before routing:
- unknown or disconnected player -> `Denied`;
- `game_over` -> `Denied` (PlayerTransform ignored silently);
- downed -> `Denied "You are downed"` (PlayerTransform reaches players.rs, which ignores it).

Then each request group goes to `<module>::handle(w, pid, req, ground, ev)`.

Rules for every handler:
- Validate everything first, then mutate. A refused request emits exactly one
  `Event::Denied { player, reason }` (short, player-facing) and changes nothing.
- Request data comes from the network: never index with it (`slot`, `offer`, island ids,
  entity ids) without `.get()`. `content::island(id)` panics on bad ids, so check with
  `ISLANDS.get(id as usize)` first. Reject non-finite floats.
- Interaction range: `dist_xz <= INTERACT_RANGE` (3 m) unless the rule says otherwise.
  Use `World::near_structure(pid, kind)` and `World::near_spot(pid, island, spot, range)`.
- Randomness only through `w.rng` (determinism and saves). Maps are `BTreeMap`/`BTreeSet`,
  never `HashMap`.
- Time: `World::now()` (f64 seconds since creation) for timestamps and cooldowns.
- Never write NaN/inf into the world (serde_json turns them into `null` and the save fails
  to load).

## Adding a request or event

1. `msg.rs`: add the `Request` variant inside your group, above `// append <system>`.
   Events go above `// append events`. Never reorder or remove variants.
2. `world.rs`: add the variant to your group's arm in `World::apply` (the match is
   exhaustive, so the build fails until you do).
3. Handle it in your module's `handle`.

## Helpers

- `World::give(pid, goods, ev)` / `give_all(pid, &[goods], ev)`: all or nothing; returns
  false (nothing changed) if items do not fit. Sac upgrades in the list apply first.
  Coins, stockpile, gear tiers (never lowered), barbed tip, lucky lure. Emits `Gained`.
- `World::spawn_debris(kind, pos)`, `spawn_enemy(kind, pos, home, ev)` (emits
  `EnemySpawned`), `spawn_boat(kind, pos)`, `alloc_id()`.
- `World::player(_mut)`, `connected()`, `player_pos`, `structure_pos`, `campfire_pos`,
  `in_campfire_light`, `in_flare_light`, `in_light`.
- `msg::{deny, toast, sound}`.
- `Raft`: `has_tile`, `covers_xz`, `on_deck`, `bounds`, `center`, `can_add_tile`,
  `can_place`, `occupant`, `find`, `count`, `is_outer_edge`, `can_add_railing`,
  `outer_edges`, `edge_pos`, `dock_pos` (boats, +Z side), `trader_dock_pos` (+X side);
  `raft::{cell_center, cell_at, footprint, neighbor}`.
- `Campfire::{lit, light_radius}`; `content` lookups (`RecipeId::def`, `Fish::def`,
  `EnemyKind::def`, `BoatKind::def`, `island`, `island_near`, `harpoon/melee/rod(tier)`,
  `roll_fish`, `shred`, `campfire_level`, `farm`, `farm_rate`, `save_cost`).

## Conventions

- Units: metres, seconds, radians. Y up. Sea level y = 0. Deck top `DECK_Y` (0.45).
  `Player::pos` is the feet position. Yaw 0 faces -Z (north, toward Hook Isle);
  `facing(yaw) = (-sin, 0, -cos)`.
- Raft cell (i, j) covers x in [2i, 2i+2), z in [2j, 2j+2). Start tiles -2..2 on both
  axes, so the raft is centred on the origin. The raft does not move; debris drifts past
  on `CURRENT` (+X).
- Gear upgrades are `Goods`, not sac items.
- Players: `World::join(name) -> Result<PlayerId, &str>` (trims; refuses empty, >20
  chars, control chars, a name already connected). Entries persist after `leave`, so a
  returning name gets the same id and its things. `World::load(json)` disconnects
  everyone and clears `game_over`.

## Replication (replicate.rs)

- Slow sections: `SectionId::{Clock, Raft, Economy, Islands, Quests, Trader, Roster}`.
  Host: `w.section(id)` (send when its JSON changes, max 4 Hz). Client:
  `w.apply_section(id, value)`. `Roster` creates/updates name + connected for every
  player on the mirror.
- Own player entry: `w.player_entry(pid)` -> `w.apply_player_entry(pid, value)`.
- Fast: `w.snapshot()` (15 Hz; cm positions, quantized yaw) ->
  `w.apply_snapshot(&snap, Some(local_pid))` (skips the local pose, removes dynamic
  objects that are gone).
- Full world: `w.to_json()` / `World::from_json` (Welcome and saves). `from_json` and
  `apply_section` return `Err` when the island or quest tables do not match the content.

## Tests

- Unit tests in each module (`#[cfg(test)] mod tests`); integration in `sim/tests/`.
- Grounds: `FlatGround::WATER` (no terrain), `FlatGround::LAND` (y = 0 everywhere),
  `IslandGround` (islands as discs at y = 1).
- Set the time of day with `w.clock.skip_to(t)`, never by writing `clock.time`.
- Typical setup: `let mut w = World::new(1); let a = w.join("Ana").unwrap();` then move
  the player with `w.player_mut(a).unwrap().pos = ...` and call `w.apply(a, req, &g)`.
  Note the start raft is 8 x 8 m: spawn points are within 3 m of several structures.

## Fishing, trader, islands, boats, save (added by the fishing/islands/boats agent)

- Fishing: `CastLine { at }` (water within CAST_RANGE, not over raft or ground; uses 1
  Bait if the sac has one) -> `Event::FishBite { player, fish }` after 4-10 s -> the
  client runs the minigame with `fish.def().zone` / `marker_speed` and sends
  `ReelIn { success }` within HOOK_SECONDS (else the fish escapes). A full sac denies a
  successful ReelIn and keeps the fish hooked. Walking more than CAST_RANGE + LINE_SLACK
  from the bobber snaps the line. `Player::fishing` is the replicated state.
- Trader: `trader::can_buy(&Player, Goods) -> Result<(), &str>` tells the shop UI which
  offers are greyed out (owned gear, smaller or equal sac). Offers never sell out.
  `Trader` arrives on even DayStarted at `raft.trader_dock_pos()`, leaves at dusk.
- Quests: statuses derive from the world in `islands::refresh` (run in `step` and after
  each island/trader request). Island quests 4-6 go straight to Done and pay through
  their chest (`OpenChest`); quests 1, 2, 7 go Ready and pay on `ClaimQuest`; quest 3
  stays Active and its claim takes the Goldfin. Quest 7 is claimed anywhere and pays
  every connected player.
- Bosses: `islands::step` reads `Event::EnemyKilled` from the step's own events. Kills
  made inside `World::apply` (harpoon, melee) never reach it, so enemies.rs MUST call
  `crate::islands::on_enemy_killed(w, kind)` whenever an enemy dies (repeats are
  harmless). It marks Rig Brute / Leviathan defeated. The Leviathan surfaces every night
  while the beacon is lit and it is undefeated (one at a time, `EnemyHome::Island(4)`).
- Generator: `DeliverGeneratorMetal` takes Scrap Chunks from the sac (METAL_PER_SCRAP
  each), not stockpile metal.
- Boats: `boats::seat_pos(&Boat, index) -> V3` is where a seated player sits (driver row
  at the stern). `Boat::fuel_timer` = seconds of paid driving left. A Motor Launch with
  no paid time and no stockpile oil denies `BoatTransform` with speed != 0 ("Out of
  oil"); the driver gets one toast when it stops. `LeaveBoat` always emits
  `PlayerTeleport`. A `BoatTransform` more than MAX_MOVE (50 m) from the hull is denied
  ("Moved too far"), like `PlayerTransform`.
- Saving: `Request::SaveWorld` is handled in save.rs. A `from_json` copy and a
  load-plus-rejoin continue identically (`save::tests`); serde_json needs its
  `float_roundtrip` feature for that.
- `tests/progression.rs`: a bot finishes the whole game (fire L5, every recipe and
  boat, six survivors, quests 1-7) through requests, with teleports for travel.
