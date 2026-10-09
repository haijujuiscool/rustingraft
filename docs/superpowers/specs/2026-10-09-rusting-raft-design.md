# Rusting Raft: design

Date: 2026-10-09. Status: approved by the user in chat. Engine: RustingEngine 2.0.3 at
`C:\Users\juliu\RustingEngine` (path dependency). Engine capability map:
the "engine facts" section at the end of this file.

## 1. The game in one paragraph

You wake on a 4x4 raft anchored in the open ocean with a campfire, a workbench and a
logbook. A small island with a fisherman is a few seconds' swim away. You fish for rare
fish and sell them for coins, harpoon floating debris into your sac, feed it to a shredder
that turns it into wood, metal and oil for the shared stockpile, and burn wood and oil in the
campfire to level it up. Each campfire level unlocks recipes: bigger rafts, boats, weapons,
automatic farms, and at level 4 a radio that finds stranded survivors on far islands. Rescued
survivors run your farms. Days last 5 minutes and nights 1 minute; at night monster fish hunt
anyone outside the campfire's light, and if the fire goes out, crawlers climb onto the raft.
Far islands hold quests, chests and bosses of rising difficulty, ending with the Leviathan at
the Lighthouse Atoll. Up to 8 players play together over a relay server.

Target length: 1-2 hours to the ending. 4 big islands, 10 small islets.

## 2. Decisions made with the user

| Topic | Decision |
| --- | --- |
| Networking | Host-authoritative co-op through a hardened, std-only Rust relay server the user runs on a Linux home server. Room codes. Up to 8 players. |
| Camera | First and third person, `V` toggles. |
| Saving | Host world saves to disk, but every save costs coins (expensive, see 9). No free autosave. |
| Art | Downloaded CC0 low-poly assets (Kenney, Quaternius, etc.), under 10 GB total (target < 500 MB). Semi-low-poly look, rough shadows, faceted ocean. |
| Death | Downed where you fall; others get a ping and unlimited time to revive. If every player is downed (or a solo player is downed): game over, reload the last save. |
| Length | 1-2 hours; 3-4 big islands plus many small ones. |
| Platforms | Windows and Linux clients. Linux relay. |
| Engine edits | Allowed (the user is the engine author). Chosen: hard-shadow filter option, flat-shading material flag, window title from `project.json`. Swimming stays game-side. |
| Raft | Anchored. An ocean current carries debris past it. Travel is by boat. |
| Water | Custom faceted low-poly wave mesh. |
| Dependencies | `rusting_engine` + `serde` (derive). Everything else std. `serde_json` is re-exported by the engine. |
| Islands | Far away (user asked to push them out). Only Hook Isle is close. |
| Project | `C:\Users\juliu\RustingRaft`, title "Rusting Raft". |

## 3. Architecture

Cargo workspace rooted at `C:\Users\juliu\RustingRaft`:

```
RustingRaft/
  Cargo.toml          game package `rustingraft` + [workspace] members = ["sim", "relay"]
  project.json        rusting project manifest
  scenes/main.rscene  the one scene (exports ship only the main scene)
  assets/             CC0 models, textures, sounds (+ assets/CREDITS.md)
  src/                game client (depends on engine, raft_sim, raft_relay)
    main.rs           rusting_game!(update); thread_local Game; phase machine
    app.rs            Game struct: phase, session (local host / remote client), view, ui state
    net/              host session, client session, transports (local + relay TCP)
    view/             engine glue: pooled visuals, ocean mesh, day/night, effects, audio
    player/           local movement, swimming, camera toggle, harpoon, melee, fishing, interact
    ui/               egui: menus, HUD, hotbar, windows
    save.rs           save/load through scene.save_data / load_data
  sim/                crate `raft_sim`: pure rules, no engine dependency
  relay/              crate `raft_relay`: lib (frame codec, relay client) + bin (server)
  tests/              rusting scenarios (offline, single player)
  docs/               this spec, the plan, relay deployment guide
```

### 3.1 `raft_sim` (pure, deterministic, unit-tested)

- No engine types. Deps: `serde` only. Own math type `V3 { x, y, z }` (f32).
- Own deterministic RNG (`SplitMix64`/`Pcg32`), seeded from the world seed; state is part of
  the world so saves and reloads continue the same sequence.
- `World` is composed of **replication sections**, each `Serialize + Deserialize + Clone`:
  - `clock`: time of day, day number, phase (day/dusk/night/dawn).
  - `raft`: tile grid, placed structures (campfire, workbench, logbook, shredder, farms,
    radio, railings), each with an id, grid cell, rotation and state.
  - `economy`: stockpile (wood, metal, biomass, oil), campfire level/progress/fuel, shredder
    queue, farm cycles, survivors on the raft, crafting unlocks, save counter.
  - `islands`: per-island state: chests opened, shrines, quest flags, survivor present/rescued.
  - `quests`: quest log progress.
  - `trader`: present/absent, stock for this visit, dock position.
  - `players`: per player: name, id, transform, health, hunger, downed, coins, sac
    (slots + capacity), hotbar tiers (harpoon, melee, rod), food/heal stacks, boat seat.
  - `dynamic`: debris, enemies, carcasses, boats, harpoon pulls: fast-changing transforms.
- API:
  - `World::new(seed) -> World`
  - `World::step(&mut self, dt: f32, ground: &dyn Ground) -> Vec<Event>` (host only).
  - `World::apply(&mut self, player: PlayerId, req: Request, ground: &dyn Ground) -> Vec<Event>`
    (host only). Every player action is a `Request`. Invalid requests are rejected with
    `Event::Denied { player, reason }` and change nothing.
  - `World::join(name) -> PlayerId`, `World::leave(id)`.
  - `trait Ground { fn height_at(&self, x: f32, z: f32) -> Option<f32>; }` the host
    implements it with an engine raycast; tests use a flat mock.
- Content tables live in `sim/src/content.rs` (items, recipes, fish, islands, quests, shop
  stock, enemy stats). Balance changes touch only that file.

### 3.2 Replication

- The host runs `World::step` at the fixed 60 Hz tick. Clients never run game rules.
- Host -> clients:
  - Slow sections (`clock`, `raft`, `economy`, `islands`, `quests`, `trader`, and each
    client's own `players` entry in full) are sent as JSON **only when their serialized
    hash changes**, at most 4 Hz per section.
  - Fast section: `dynamic` transforms + other players' transforms/health at 15 Hz in a
    compact form (ids + quantized positions/yaw).
  - `Event`s that need feedback (sounds, popups, pings, denials) are sent as they happen.
- Client -> host: own player transform at 20 Hz (client-authoritative movement, co-op trust),
  driven boat transform while driving, and `Request`s.
- Clients keep a mirror `World` updated from sections; the view renders host and clients
  from the same `World` type, so there is one view code path. Remote players and dynamic
  objects interpolate with a ~100 ms buffer.
- Solo play is a host with no network.
- Messages are `serde_json` inside relay frames. Version handshake: `Hello { version, name }`
  -> `Welcome { player_id, full_world }` or `Reject { reason }`.

### 3.3 Relay server (`raft_relay`)

- std only, no dependencies, Linux first (builds anywhere). Thread per connection, bounded.
- Framing: `u32` big-endian length + payload, max 65,536 bytes. Larger = disconnect.
- Handshake (first frame, ASCII):
  - `RRLY1 HOST` -> `OK <room code>`
  - `RRLY1 JOIN <code>` -> `OK <client id 1..7>` or `ERR <reason>` (`full`, `no_room`,
    `rate_limited`, `bad_request`)
- Data frames:
  - client -> relay: payload, forwarded to the host prefixed with 1 byte source id.
  - host -> relay: 1 byte target (0 = all clients, 1..7 = one client) + payload.
  - relay -> host control: `0xFF` + ASCII `JOINED <id>` / `LEFT <id>`.
  - Keepalive: empty frame every 5 s both ways; idle timeout 30 s.
- Room code: 6 characters from a 32-symbol unambiguous alphabet (`23456789ABCDEFGHJKMNPQRSTUVWXYZ`
  minus one), random from `/dev/urandom` (fallback: hashed time + counter on non-Linux).
  Room closes when the host disconnects; all its clients are disconnected.
- Hardening:
  - Limits (CLI flags with these defaults): 64 rooms, 7 clients per room, 512 total
    connections, 8 connections per IP, handshake timeout 5 s, idle timeout 30 s.
  - Token bucket per connection: 256 KiB/s sustained, 512 KiB burst; over the limit = drop.
  - Join attempts: 10 per minute per IP; then `ERR rate_limited` for 60 s (stops code brute force).
  - Never parses or logs payloads; logs only connect/room events with counts to stderr.
  - No files written, no shell, no `unsafe`, no panics on bad input (all reads checked).
  - `deploy/rusting-relay.service` systemd unit: `DynamicUser=yes`, `NoNewPrivileges=yes`,
    `ProtectSystem=strict`, `ProtectHome=yes`, `PrivateTmp=yes`, `PrivateDevices=yes`,
    `RestrictAddressFamilies=AF_INET AF_INET6`, `MemoryMax=256M`, `TasksMax=600`,
    `CapabilityBoundingSet=` (empty), `SystemCallFilter=@system-service`, restart on failure.
  - `docs/relay.md`: build, install, port forward of one TCP port (default 27777), firewall,
    what it can and cannot defend against (volumetric DDoS on a home line cannot be stopped
    in software).
- `raft_relay` lib also contains the client side (`RelayClient::host(addr)`,
  `RelayClient::join(addr, code)`) with background reader/writer threads and `mpsc`
  channels, used by the game.
- Tests: frame codec, limits, rate limiter, code generator, and a loopback integration test
  (server + 1 host + 2 clients on 127.0.0.1).

### 3.4 Game client (`rustingraft`)

- State: `thread_local! static GAME: RefCell<Option<Game>>` (f15_dogfight pattern).
- Phases: `MainMenu` -> (`Solo` | `HostLobby` | `JoinLobby`) -> `Playing` -> `GameOver`.
  Main menu: New world, Load world (costly saves only), Host online, Join online
  (relay address + room code + player name), Settings (volume, mouse sensitivity, render
  scale, relay address default).
- `view` reads the `World` mirror each frame: spawns/pools engine objects from hidden
  templates (`spawn_copy`) and moves them; builds the ocean mesh; sets lights for day/night.
- `player` owns local input -> movement, swim, camera mode, harpoon, melee, fishing,
  interaction; it emits `Request`s to the session.
- `ui` draws HUD and windows from the `World` mirror and emits `Request`s.
- Networking is off in `rusting test` scenarios (solo session), so tests stay deterministic.

## 4. World layout

- +Y up, water at y = 0, raft deck top at y = 0.45. Raft tiles 2 m x 2 m. Starting raft
  4x4 tiles centered on the origin. Max raft 16x16 tiles.
- Ocean current: +X at 1.2 m/s. Debris spawns upstream (x = -70, z in [-40, 40]) and
  despawns at x = +80. Spawn every 3.5 s on average, max 45 active.
- Islands (center, rough radius). Hook Isle is close on purpose, the rest are far:

| Island | Center (x, z) | Distance | Radius | Tier | Contents |
| --- | --- | --- | --- | --- | --- |
| Hook Isle | (0, -38) | 38 m | 11 m | 0 | Fisherman Barnaby, dock, palm trees |
| Palm Reef | (-420, -560) | 700 m | 40 m | 1 | 3 tide shrines, chest (Harpoon T3), survivor, reef crabs |
| Rusted Wreck | (1150, -600) | 1300 m | 55 m | 2 | Wrecked freighter, generator quest, chest (Coral Cutlass + Sac 24), survivor, crawlers |
| Smoking Rig | (300, 1950) | 1975 m | 45 m | 3 | Oil platform, Rig Brute miniboss, chest (Harpoon T4), oil cache, survivor |
| Lighthouse Atoll | (-2100, 1700) | 2700 m | 60 m | 4 | Lighthouse, Beacon socket, Leviathan boss (night) |
| 10 islets | 300-2400 m, scattered | | 8-15 m | 1-3 | Palm or rock, chest (coins/bait/bandages/debris), crabs or crawlers; 3 hold survivors |

- Islands are authored once in `scenes/main.rscene` (generated by a patch script from
  templates/models) with Fixed colliders.
- The ocean mesh follows the camera, snapped to its cell size so the facets do not swim.
- Fog hides the far plane; each big island has a tall silhouette (palm cluster, wreck mast,
  rig derrick, lighthouse with a light at night) so players can navigate by sight. A compass
  strip on the HUD shows discovered islands, pings and radio signals.

## 5. Day/night

- One cycle = 360 s: day 300 s (dawn blend first 15 s, dusk blend last 20 s), night 60 s.
- Day number increments at dawn. Sun and moon arc across the sky; sky/fog/ambient/sun
  colors blend from the `daylight`, `golden_hour` and `night` presets. Stars at night.
- Night threats:
  - **Gloomfin** hunt every player who is in the water or farther than the campfire light
    radius from the campfire (on the raft, islands or boats). Up to 1 + day/3 per player,
    max 3 per player, max 12 total. At dawn they dive and despawn.
  - **Crawlers** climb onto the raft edges while the campfire is out at night: one every
    8 s, max 2 + day/2. They stay until killed.

## 6. Player

- Health 100. Hunger 100, drains 100 per 420 s (x1.5 while sprinting or swimming fast).
  At hunger 0, health drains 1 per second.
- Walk 4.5 m/s, sprint 7, swim 4.0, fast swim 5.5. Jump on land. In water: Space rises,
  Ctrl dives (shallow, 3 m max), the player floats at the surface by default.
- Downed at 0 health: lies where it fell, other players see a ping on the compass and a
  marker in the world. Hold E for 3 s within 2 m to revive (back at 30 health). If no
  player is up (or solo), the game-over screen offers "Load last save" or "Main menu".
- `V` toggles first/third person. `E` interacts. `Tab` opens the sac. `Esc` menu.
  `B` opens the build/craft list when near the workbench. Mouse left = use, right = aim.

### 6.1 Hotbar (fixed, keys 1-6)

| Key | Slot | Tiers |
| --- | --- | --- |
| 1 | Harpoon | T1 start: range 14 m, dmg 15, cooldown 1.2 s. T2 (Barnaby, 120 coins): 20 m, 25. T3 (Palm Reef chest): 28 m, 40. T4 (Smoking Rig chest): 36 m, 60, stuns 1.5 s. |
| 2 | Sac | Opens the inventory grid. Capacity 8 -> 12 (Barnaby, 80 coins) -> 16 (trader, 150) -> 24 (Rusted Wreck chest or trader 400) -> 32 (trader, 900). |
| 3 | Melee | Driftwood Club (start, 12 dmg) -> Scrap Machete (craft L2, 22) -> Coral Cutlass (Rusted Wreck chest, 35) -> Abyss Blade (craft L5, 55). 0.6 s swing. |
| 4 | Food | Stack of one food item, eaten with left click. |
| 5 | Heal | Stack of one heal item (Bandage +30 hp, Biomass Salve +60 hp). |
| 6 | Fishing rod | Twig Rod (start) -> Reed Rod (Barnaby, 60 coins) -> Carbon Rod (craft L3 or trader 350). |

Harpoon: fire at a surface (raft, island, rock, wreck, boat) to grapple: the player is
pulled toward the hit point at 16 m/s and released near it. Fire at debris/loot to drag it
to you (it goes into the sac on arrival if there is room). Fire at an enemy to damage it and
pull it toward you.

### 6.2 Sac and items

- Sac slots hold stacks: debris (stack 10), fish (stack 5), food/heal (stack 10), bait (20),
  misc (1), quest items (1).
- Debris items: Driftwood Plank, Palm Bundle, Scrap Chunk, Floating Crate, Oil Barrel,
  Carcass (Crawler / Gloomfin / Reef Crab / boss).
- Coins are a per-player wallet, not a sac item.

## 7. Fishing

- Cast with left click (rod slot) at water within 12 m; bobber floats. Bite after 4-10 s.
  Then a timing bar: a marker sweeps, click while it is in the green zone. Hit = catch,
  miss = the fish escapes. Rarer fish have smaller zones and faster markers.
- The host rolls the species on a successful catch.

| Fish | Base weight | Sell (coins) | Cooked hunger |
| --- | --- | --- | --- |
| Sardine | 60 | 4 | 15 |
| Mackerel | 28 | 10 | 25 |
| Goldfin | 9 | 35 | 40 |
| Moonray | 2.6 | 110 | 60 |
| Abyssal Koi | 0.4 | 400 | 100 |

- Reed Rod x1.6 on Goldfin and rarer, Carbon Rod x2.5. Bait x1.5 (one bait per cast if
  any). Night x2 on Moonray and Koi.
- Raw fish can be eaten for half the hunger. Cooking at the campfire is instant.

## 8. Shredder, stockpile, campfire, farms

- Shredder (on the raft from the start): interact deposits all debris from your sac; it
  processes one item per 2 s into the shared stockpile:
  Plank -> 3 wood, Palm Bundle -> 2 wood, Scrap Chunk -> 2 metal, Oil Barrel -> 2 oil,
  Floating Crate -> 2-4 wood + 1-2 metal (+20% bait x2, +10% coins 10-30 to the depositor),
  Crawler carcass -> 2 biomass, Gloomfin -> 3, Reef Crab -> 1, Rig Brute -> 15.
- Stockpile (shared by all players, shown at the workbench): wood, metal, biomass, oil.
- Campfire: interact to feed 5 wood or 1 oil from the stockpile (hold to repeat).
  - Upgrade points: wood 1, oil 5. Level 1 -> 5 at 25 / 50 / 75 / 100 points.
  - Fuel: wood +20 s, oil +100 s, cap 600 s, starts at 240 s, burns 1 s/s.
  - Light radius by level: 6, 8, 10, 12, 15 m. Out of fuel = no light, no safety.
- Farms (campfire L3), placed on the raft:
  - Wood farm: 90 s cycle -> 4 Palm Bundles. Metal farm: 120 s -> 3 Scrap Chunks.
    Oil farm: 150 s -> 2 Oil Barrels.
  - Without survivors: interact to start a cycle; when done, interact to take the output
    into your sac.
  - With at least one rescued survivor: cycles restart by themselves and output goes
    straight on: wood/metal farm output into the shredder queue, oil farm output into the
    campfire. Rate x(1 + 0.25 x (survivors - 1)).

## 9. Saving (costs coins)

- The Captain's Logbook stands on the raft from the start. Interacting with it offers
  "Save world" for `150 + 75 x campfire level` coins (225 at L1, 525 at L5), paid by the
  player who saves. Only the host's machine writes the file; any player can pay.
- No autosave. Quitting asks for confirmation and warns that unsaved progress is lost.
- Game over loads the last save. With no save, game over returns to the main menu and the
  world is lost.
- Save content: the whole `World` (all sections, RNG state) as JSON, plus player entries
  keyed by name so returning players get their inventory back.

## 10. Crafting (at the workbench, from the stockpile)

| Level | Recipe | Cost |
| --- | --- | --- |
| L1 | Raft Floor (placed next to an existing tile) | 4 wood |
| L1 | Railing (tile edge) | 2 wood |
| L2 | Dinghy | 30 wood, 8 metal |
| L2 | Scrap Machete | 6 metal, 4 wood |
| L2 | Bandage x2 | 2 biomass, 1 wood |
| L2 | Bait x3 | 2 biomass |
| L3 | Wood Farm | 20 wood, 10 metal |
| L3 | Metal Farm | 10 wood, 25 metal |
| L3 | Oil Farm | 15 wood, 20 metal, 10 biomass |
| L3 | Carbon Rod | 12 metal, 6 biomass |
| L3 | Biomass Salve | 4 biomass |
| L4 | Radio Station | 20 metal, 10 oil, 15 biomass |
| L4 | Skiff | 40 wood, 30 metal |
| L5 | Motor Launch | 20 wood, 40 metal, 20 oil |
| L5 | Abyss Blade | 30 metal, 20 biomass |
| L5 | Beacon Core | 50 metal, 30 oil, 30 biomass |

Placeables (floors, railings, farms, radio) enter a ghost-preview placement mode: a green or
red ghost on the raft grid, left click places, R rotates, right click cancels. Boats spawn
at the raft's dock edge.

## 11. Boats

| Boat | Seats | Speed | Notes |
| --- | --- | --- | --- |
| Dinghy | 2 | 9 m/s | reaches Palm Reef and Rusted Wreck |
| Skiff | 4 | 15 m/s | needed for Smoking Rig in reasonable time |
| Motor Launch | 8 | 24 m/s | burns 1 oil per 60 s of driving from the stockpile; needed for Lighthouse Atoll |

- Seated model: E on a boat takes the driver seat (if free) or a passenger seat. The driver
  steers with WASD; passengers are locked to their seats; E again leaves the seat.
- The driver's client owns the boat transform while driving and streams it to the host.
- Boats float with the waves (pitch/roll from `wave()` at the hull ends) and stop at island
  shores (simple sphere-vs-island-radius push-out).

## 12. NPCs, trader, survivors

- **Fisherman Barnaby** (Hook Isle): buys any fish at the table price; sells Reed Rod 60,
  Harpoon T2 120, Sac 12 80, Bait x5 20. Gives quests 1-3.
- **Wandering trader** (boat): arrives at the dawn of every second day (days 2, 4, 6...),
  docks at the raft, leaves at dusk. Each visit offers 5 items drawn (by the world RNG) from:
  Sac 16 (150), Sac 24 (400), Sac 32 (900), Bait x5 (25), Bandage x3 (40),
  Flare (60; a 30 s light that keeps Gloomfin away), Barbed Harpoon Tip (+10 harpoon
  damage, 250), Lucky Lure (+50% to rare fish, 300), Carbon Rod (350).
  Sac upgrades the player already has are not offered to that player.
- **Survivors** (6): one each on Palm Reef, Rusted Wreck, Smoking Rig and three islets.
  They only appear once a Radio Station exists. The radio window lists signals; the compass
  shows them. Talking to a survivor rescues them: they appear on the raft and start working
  the farms (8).

## 13. Quests

| # | Giver / place | Goal | Reward |
| --- | --- | --- | --- |
| 1 | Barnaby | Sell 3 fish | 30 coins |
| 2 | Barnaby | Campfire level 2 | 40 coins + Bait x5 |
| 3 | Barnaby | Bring a Goldfin | Sac 12 (or 80 coins if owned) |
| 4 | Palm Reef | Activate 3 tide shrines (on high rocks, grapple up) | chest: Harpoon T3 |
| 5 | Rusted Wreck | Deliver 20 metal to the generator | chest: Coral Cutlass + Sac 24 |
| 6 | Smoking Rig | Defeat the Rig Brute | chest: Harpoon T4 + 20 oil |
| 7 | Lighthouse Atoll | Install the Beacon Core at night and defeat the Leviathan | ending, 2000 coins |

After the ending the world continues.

## 14. Enemies

| Enemy | HP | Damage | Speed | Where | Carcass |
| --- | --- | --- | --- | --- | --- |
| Gloomfin | 40 | 15 bite / 1.5 s | 7 m/s | night water and dark areas | Gloomfin (3 biomass) |
| Crawler | 30 | 10 / 1.2 s | 3.5 m/s | raft when fire is out at night; Rusted Wreck, islets | Crawler (2) |
| Reef Crab | 20 | 6 / 1 s | 3 m/s | Palm Reef, islets | Reef Crab (1) |
| Rig Brute | 300 | 25 / 2 s | 3 m/s | Smoking Rig (miniboss) | Rig Brute (15) |
| Leviathan | 1200 | 35 / 2.5 s | 9 m/s | Lighthouse Atoll at night after the beacon is lit | none |

- HP x tier multiplier 1.0 / 1.3 / 1.7 / 2.2 by island tier.
- Dead enemies drop a floating carcass (debris item). Island enemies respawn after 2 days.

## 15. Look and feel

- Semi-low-poly: CC0 models (Kenney Pirate/Survival/Nature/Watercraft kits, Quaternius
  packs), flat-shaded via the new engine material flag, hard shadows via the new engine
  shadow filter option, faceted ocean, warm key / cool fill, fog with distance.
- Campfire: fire + embers + smoke particles, flickering point light whose range is the
  light radius.
- Sounds: CC0 sfx (Kenney audio packs) plus a generated ocean ambience loop.
- Remote players: a CC0 character with a name tag.
- HUD: health and hunger bars bottom left, hotbar bottom center, compass top center,
  clock/day top right, interaction prompt center, toasts for events.

## 16. Engine changes (RustingEngine, committed to its repo with tests)

1. Shadow filter option: `RenderSettings` gets a `shadow_filter` (`Soft` = current 3x3 PCF,
   `Hard` = one tap), also settable from the scene and from game code.
2. Flat shading: `MaterialAsset` / inline material `flat_shading: bool`; the shader uses the
   screen-space derivative face normal when set.
3. Window title: `project.json` `window_title` (falls back to the current title).

Each change: schema/docs updated, a test, `cargo test` passes, existing samples still build.

## 17. Testing

- `cargo test -p raft_sim`: rules (campfire levels and fuel, shredder outputs, hunger,
  fishing odds, crafting costs/unlocks, placement, quests, trader schedule, survivors and
  farms, night spawns, downed/revive/game over, save cost, save round trip).
- `cargo test -p raft_relay`: codec, limits, rate limiting, code alphabet, loopback relay.
- Game crate unit tests: replication section diffing and compact transform codec.
- `rusting test` scenarios (solo): boot to playing, fishing catch adds a fish, shredder
  turns debris into stockpile wood, campfire levels at 25 wood, night with no fire spawns
  crawlers, downed solo -> game over, saving deducts coins. Captures: raft at day, raft at
  night with fire, Hook Isle, a far island, HUD and sac window.
- Manual: two local instances through a local relay (`RUSTING_USER_DATA` per instance).

## 18. Engine facts (from the capability map)

- CLI: `C:\Users\juliu\RustingEngine\target\release\rusting.exe` (not on PATH).
- `rusting_game!(update)`; `update(&mut GameScene, &FrameTime)` runs once per rendered frame
  after 0..8 fixed steps at 1/60 s. State in `thread_local` (see
  `samples/f15_dogfight/src/main.rs:83-87, 201-233`).
- `spawn_copy(template, name, pos)` deep-copies colliders and children (children named
  `"<name>/<child>"`); `spawn_cube`/`spawn_sphere` have no collider. Duplicate names panic.
- Custom meshes: `world().resource_mut::<AssetServer>().meshes.insert(MeshAsset{..})`.
- Water: `src/runtime/water.rs`, `WaterBody::wave(x, z, t)` is public and deterministic.
- Player controller ignores water; swimming is done in game code by changing
  `PlayerController` fields through `world().get_mut::<PlayerController>(e)`.
- Riding moving platforms is translation-only; boats use the seated model.
- One directional shadow map, reach 30-80 m depending on quality; `ShadowQuality::Low` is crisp.
- egui 0.31 via `scene.ui()`; `TextEdit` works for IP/room code entry.
- `save_data`/`load_data` write under `%APPDATA%\<exe>\` (override `RUSTING_USER_DATA`).
- Export cooks only the main scene: all islands live in `scenes/main.rscene`.
- `rusting test` runs headless; GPU-simulated bodies freeze headless, so everything is CPU.
