"""Write assets/CATALOG.md (game role -> asset paths with measured sizes, origin and clip notes)
and assets/CREDITS.md (from the .rmeta sidecars written by `rusting asset import`).

Usage: python -I tools/assets/catalog.py
Fails if a listed path does not exist, so the catalog cannot drift from assets/.
"""
import collections
import json
import pathlib
import sys

sys.dont_write_bytecode = True
sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
import bounds  # noqa: E402

ROOT = pathlib.Path(__file__).resolve().parents[2]
ASSETS = ROOT / "assets"

R, D, I, C, CR, B, IT = (f"models/{c}/" for c in ("raft", "debris", "island", "character", "creature", "boat", "item"))

MODELS = {
    "Raft": [
        ("Raft Floor tile (2 x 2 m, deck top y = 0.45)",
         [R + "kenney-pirate/platform.glb", R + "kenney-pirate/platform-planks.glb", R + "kenney-survival/floor.glb",
          R + "kenney-survival/floor-old.glb", R + "kenney-survival/floor-hole.glb"],
         "platform: scale 0.8 -> 2.0 x 0.18 x 2.0. Survival floor: scale 4 (0.2 thick). floor-old = damaged variant."),
        ("Railing (tile edge)",
         [R + "kenney-survival/fence.glb", R + "kenney-survival/fence-fortified.glb", R + "kenney-survival/fence-doorway.glb",
          R + "kenney-pirate/structure-fence.glb", R + "kenney-pirate/structure-fence-sides.glb"],
         "Survival fence: origin = tile center, fence on the -Z edge; rotate 90/180/270 deg per edge. Scale (4, 2, 4) -> 2 m wide, 1 m tall."),
        ("Campfire (levels 1-5)",
         [R + "kenney-survival/campfire-pit.glb", R + "kenney-survival/campfire-stand.glb",
          R + "kenney-survival/campfire-fishing-stand.glb", I + "kenney-nature/campfire_stones.glb"],
         "Scale 4 -> ~1.1 m pit. Add stand/fishing-stand at higher levels. Fire = particles + point light."),
        ("Workbench",
         [R + "kenney-survival/workbench.glb", R + "kenney-survival/workbench-anvil.glb", R + "kenney-survival/workbench-grind.glb"],
         "Scale 4 -> 1.3 x 1.15 x 1.2 m."),
        ("Captain's Logbook",
         [R + "book_open.glb", R + "book.glb", R + "kenney-survival/signpost.glb"],
         "book_open on a stand (signpost or box-large scaled). Book origins are at the center, not the bottom. Scale ~0.5."),
        ("Shredder",
         [R + "kenney-factory/hopper-square.glb", R + "kenney-factory/machine.glb", R + "kenney-factory/hopper-round.glb",
          R + "kenney-factory/cog-a.glb", R + "kenney-factory/cog-b.glb", R + "kenney-factory/conveyor.glb"],
         "Stack hopper-square on machine (machine top at y = 1.3). Spin a cog for feedback. Scale ~1."),
        ("Wood farm",
         [R + "kenney-nature/pot_large.glb", R + "kenney-nature/crops_bambooStageA.glb", R + "kenney-nature/crops_bambooStageB.glb",
          R + "kenney-nature/crops_leafsStageA.glb", R + "kenney-nature/crops_leafsStageB.glb",
          R + "kenney-nature/crops_dirtSingle.glb", R + "kenney-nature/log_stack.glb"],
         "Planter + growth stages (A = growing, B = ready). Scale 3."),
        ("Metal farm",
         [R + "kenney-factory/machine-fortified.glb", R + "kenney-factory/piston-round.glb", R + "kenney-factory/screen-small.glb",
          R + "kenney-factory/machine-bed.glb"],
         "piston-round has clips toggle-on/toggle-off/toggle (pumping)."),
        ("Oil farm",
         [R + "kenney-industrial/detail-tank.glb", R + "kenney-factory/pipe-large-valve.glb", R + "kenney-factory/pipe-large.glb"],
         "detail-tank scale 2.5 -> 2.1 x 1.0 x 1.3 m."),
        ("Radio Station (L4)",
         [R + "radio_q.glb", R + "antenna_b.glb", R + "antenna_a.glb", R + "radio_k.glb",
          R + "kenney-survival/structure-metal.glb", R + "kenney-factory/lever-single.glb"],
         "radio_q on a table, antenna_b on top. radio_k is tiny (0.3 m) with its origin at a corner. lever-single has toggle clips."),
        ("Raft decor / optional structures",
         [R + "kenney-pirate/mast.glb", R + "kenney-pirate/mast-ropes.glb", R + "kenney-pirate/flag-pirate.glb", R + "kenney-pirate/flag.glb",
          R + "kenney-pirate/structure-platform.glb", R + "kenney-pirate/structure-platform-small.glb",
          R + "kenney-survival/structure.glb", R + "kenney-survival/structure-floor.glb", R + "kenney-survival/structure-roof.glb",
          R + "kenney-survival/structure-canvas.glb", R + "kenney-survival/structure-metal-floor.glb",
          R + "kenney-survival/structure-metal-wall.glb", R + "kenney-survival/structure-metal-roof.glb",
          R + "kenney-survival/structure-metal-doorway.glb", R + "kenney-survival/metal-panel.glb",
          R + "kenney-survival/metal-panel-screws.glb", R + "kenney-survival/bedroll.glb", R + "kenney-survival/tent.glb",
          R + "kenney-survival/bucket.glb", R + "kenney-survival/box-large.glb", R + "kenney-survival/box-large-open.glb",
          R + "kenney-industrial/solar-panel-flat.glb", R + "kenney-industrial/windmill-low.glb",
          R + "kenney-factory/cog-a.glb"],
         "Mast doubles as a tall silhouette. Survival structure-* pieces are 0.5 m modules (scale 4 for 2 m tiles)."),
    ],
    "Debris (floating, harpoonable)": [
        ("Driftwood Plank",
         [D + "kenney-survival/resource-planks.glb", D + "kenney-nature/log.glb", D + "kenney-survival/tree-log-small.glb",
          D + "kenney-survival/resource-wood.glb"],
         "Scale 2.5 -> 0.9 x 0.24 x 1.6 m."),
        ("Palm Bundle",
         [D + "kenney-nature/grass_leafsLarge.glb", D + "kenney-nature/grass_leafs.glb"],
         "Scale 2.5. Origin 0.05 above the bottom (nature kit)."),
        ("Scrap Chunk",
         [D + "kenney-survival/metal-panel-screws-half.glb", D + "kenney-survival/metal-panel-narrow.glb",
          D + "kenney-factory/cog-e.glb", D + "kenney-factory/box-small.glb", D + "debris_pile.glb"],
         "Lay panels flat (rotate -90 deg about X), scale 2. debris_pile is a 2 m flat scrap heap (island dressing)."),
        ("Floating Crate",
         [D + "kenney-pirate/crate.glb", D + "kenney-pirate/crate-bottles.glb", D + "kenney-survival/box.glb", D + "kenney-survival/box-open.glb"],
         "Pirate crate scale 0.8 -> 0.86 x 0.62 x 1.05 m."),
        ("Oil Barrel",
         [D + "kenney-pirate/barrel.glb", D + "kenney-survival/barrel.glb", D + "kenney-survival/barrel-open.glb"],
         "Pirate barrel scale 0.7 -> ~0.9 m. Tint dark/red for oil."),
        ("Carcass (Crawler / Gloomfin / Reef Crab / boss)",
         [D + "kenney-food/fish-bones.glb", D + "kenney-food/meat-ribs.glb"],
         "Or reuse the enemy model on its Death clip's last frame, tinted grey and floating. fish-bones scale 2-3."),
        ("Chest (closed / open)",
         [D + "kenney-pirate/chest.glb", D + "chest_closed.glb", D + "chest_open.glb", D + "chest_ocean.glb", D + "kenney-survival/chest.glb"],
         "Pirate chest has clips open/close/open-close (scale 0.8). chest_ocean origin is mid-height."),
        ("Large floating dressing",
         [D + "kenney-watercraft/buoy.glb", D + "kenney-watercraft/buoy-flag.glb", D + "kenney-watercraft/cargo-container-a.glb",
          D + "kenney-watercraft/cargo-pile-a.glb"],
         "Buoys have origin at the waterline (bottom at y = -0.3)."),
    ],
    "Islands": [
        ("Palm trees",
         [I + "kenney-pirate/palm-straight.glb", I + "kenney-pirate/palm-bend.glb", I + "kenney-pirate/palm-detailed-straight.glb",
          I + "kenney-pirate/palm-detailed-bend.glb", I + "kenney-nature/tree_palm.glb", I + "kenney-nature/tree_palmBend.glb",
          I + "kenney-nature/tree_palmTall.glb", I + "kenney-nature/tree_palmDetailedTall.glb", I + "kenney-nature/tree_palmShort.glb",
          I + "kenney-nature/tree_palmDetailedShort.glb"],
         "Pirate palms scale 1.5-2 -> 6-8 m. Nature palms scale 5."),
        ("Rocks",
         [I + "kenney-pirate/rocks-a.glb", I + "kenney-pirate/rocks-b.glb", I + "kenney-pirate/rocks-c.glb",
          I + "kenney-pirate/rocks-sand-a.glb", I + "kenney-pirate/rocks-sand-b.glb", I + "kenney-pirate/rocks-sand-c.glb",
          I + "kenney-nature/rock_largeA.glb", I + "kenney-nature/rock_largeB.glb", I + "kenney-nature/rock_largeC.glb",
          I + "kenney-nature/rock_tallA.glb", I + "kenney-nature/rock_tallB.glb", I + "kenney-nature/rock_smallA.glb",
          I + "kenney-nature/rock_smallB.glb", I + "kenney-nature/stone_largeA.glb", I + "kenney-nature/stone_tallA.glb",
          I + "kenney-castle/rocks-large.glb", I + "kenney-castle/rocks-small.glb"],
         "High rocks for the Palm Reef shrines: rock_tallA/B scaled 6-10."),
        ("Cliffs (modular 1 m blocks)",
         [I + "kenney-nature/cliff_block_rock.glb", I + "kenney-nature/cliff_large_rock.glb", I + "kenney-nature/cliff_corner_rock.glb",
          I + "kenney-nature/cliff_half_rock.glb", I + "kenney-nature/cliff_top_rock.glb", I + "kenney-nature/cliff_steps_rock.glb",
          I + "kenney-nature/cliff_waterfall_rock.glb"],
         "Wall pieces sit on the +Z half of the 1 m cell. Scale 4-8."),
        ("Sand / island ground",
         [I + "kenney-pirate/patch-sand.glb", I + "kenney-pirate/patch-sand-foliage.glb", I + "kenney-pirate/patch-grass.glb",
          I + "kenney-pirate/patch-grass-foliage.glb", I + "kenney-nature/platform_beach.glb", I + "kenney-nature/platform_grass.glb",
          I + "kenney-nature/platform_stone.glb"],
         "No island-sized ground mesh. Fallback: flattened Sphere/Cylinder primitive (radius = island radius, sand color) with patches on top."),
        ("Grass / bushes",
         [I + "kenney-pirate/grass.glb", I + "kenney-pirate/grass-plant.glb", I + "kenney-nature/plant_bush.glb",
          I + "kenney-nature/plant_bushLarge.glb", I + "kenney-nature/plant_bushDetailed.glb"],
         ""),
        ("Dock / pier (Hook Isle, trader)",
         [I + "kenney-pirate/structure-platform-dock.glb", I + "kenney-pirate/structure-platform-dock-small.glb",
          I + "dock_long.glb", I + "dock.glb", I + "dock_broken.glb", I + "kenney-nature/bridge_wood.glb"],
         "Pirate dock: 2.5 m module with posts, chain along X/Z. dock_long is a whole 22 m pier. dock is a tiny segment (scale 3)."),
        ("Huts / Barnaby's shack",
         [I + "kenney-pirate/structure.glb", I + "kenney-pirate/structure-roof.glb", I + "kenney-pirate/tower-watch.glb",
          I + "kenney-nature/sign.glb", I + "kenney-nature/fence_planks.glb", I + "kenney-pirate/hole.glb"],
         "structure + structure-roof = 2.5 m hut."),
        ("Rusted Wreck freighter",
         [I + "kenney-watercraft/ship-cargo-a.glb", I + "kenney-pirate/ship-wreck.glb", B + "kenney-watercraft/ship-small.glb"],
         "ship-cargo-a scale 4-5 -> 42-53 m, tilt ~12 deg and sink 2 m. ship-wreck has a broken mast (silhouette)."),
        ("Rusted Wreck generator",
         [R + "kenney-factory/machine-fortified.glb", R + "kenney-factory/screen-small.glb"],
         "Shares the raft metal-farm model; tint rust."),
        ("Smoking Rig (oil platform)",
         [I + "kenney-factory/floor-large.glb", I + "kenney-factory/structure-tall.glb", I + "kenney-factory/structure-high.glb",
          I + "kenney-factory/structure-medium.glb", I + "kenney-factory/catwalk-straight.glb", I + "kenney-factory/catwalk-corner.glb",
          I + "kenney-factory/catwalk-stairs.glb", I + "kenney-factory/crane.glb", I + "kenney-factory/crane-lift.glb",
          I + "kenney-factory/pipe-large-long.glb", I + "kenney-factory/pipe-large-bend.glb", I + "kenney-factory/top-large.glb",
          I + "kenney-factory/warning-orange.glb", I + "kenney-industrial/detail-tank-large.glb",
          I + "kenney-industrial/chimney-large.glb", I + "kenney-industrial/chimney-medium.glb",
          I + "kenney-industrial/water-tower.glb", I + "kenney-industrial/shipping-container-a.glb"],
         "No single rig model: build it. Legs = Cylinder primitives, deck = floor-large tiles (2 m, zero-thickness: add a Box under it), "
         "derrick = crane or water-tower scaled 4, smoke from chimney-large. Oil cache = debris barrels."),
        ("Lighthouse (Lighthouse Atoll)",
         [I + "lighthouse.glb", I + "kenney-castle/tower-hexagon-base.glb", I + "kenney-castle/tower-hexagon-mid.glb",
          I + "kenney-castle/tower-hexagon-top.glb", I + "kenney-castle/tower-hexagon-roof.glb",
          I + "kenney-pirate/tower-complete-large.glb", I + "kenney-castle/stairs-stone.glb"],
         "lighthouse.glb origin is off-center (x -3.9..0.36): offset it. Alt: stack castle hexagon pieces. Light at night = point/spot light. "
         "Beacon socket/Beacon Core: no model, use an emissive Sphere on a statue_column."),
        ("Tide shrines (Palm Reef x3) / totems",
         [I + "shrine_a.glb", I + "shrine_b.glb", I + "kenney-nature/statue_obelisk.glb", I + "kenney-nature/statue_column.glb",
          I + "kenney-nature/statue_columnDamaged.glb", I + "kenney-nature/statue_head.glb", I + "kenney-nature/statue_ring.glb",
          I + "kenney-nature/statue_block.glb"],
         "Activated state: emissive tint + light. Nature statues scale 3."),
        ("Watchtowers / silhouettes",
         [I + "kenney-pirate/tower-complete-large.glb", I + "kenney-pirate/tower-complete-small.glb"],
         ""),
    ],
    "Characters": [
        ("Players (up to 8)",
         [C + "adventurer.glb", C + "casual.glb", C + "beach_character.glb", C + "worker.glb", C + "farmer.glb",
          C + "kenney-mini/character-male-e.glb", C + "kenney-mini/character-female-a.glb"],
         "In use: Kenney mini male-e for every player (0.7 m, fit to height, slimmed in bind space by SLIM; "
         "build_scene.py adds hand-keyed swim / tread clips). Quaternius humans are 1.86 m, face +Z, no swim clip."),
        ("Fisherman Barnaby",
         [C + "pirate_captain.glb"],
         "Wave/Yes/No clips for dialogue."),
        ("Wandering trader",
         [C + "farmer.glb", C + "worker.glb"],
         "Rides the trader boat."),
        ("Survivors (6)",
         [C + "kenney-mini/character-male-b.glb", C + "kenney-mini/character-male-c.glb", C + "kenney-mini/character-male-d.glb",
          C + "kenney-mini/character-male-e.glb", C + "kenney-mini/character-male-f.glb", C + "kenney-mini/character-female-b.glb",
          C + "kenney-mini/character-female-c.glb", C + "kenney-mini/character-female-d.glb", C + "kenney-mini/character-female-e.glb",
          C + "kenney-mini/character-female-f.glb"],
         "In use: male-b, female-e, male-f, female-f, male-d, female-b (SURVIVORS). Clips sit / interact-right "
         "for farm work, emote-yes on rescue."),
    ],
    "Creatures": [
        ("Sardine", [CR + "fish_sardine.glb"], "Target ~0.25 m long. Faces +Z."),
        ("Mackerel", [CR + "fish_mackerel.glb", CR + "fish_tuna.glb"], "Static mesh, long axis X, head at -X: rotate. Target ~0.4 m (scale 0.25)."),
        ("Goldfin", [CR + "fish_tang.glb", CR + "fish_butterfly.glb"], "Tint gold. Target ~0.4 m."),
        ("Moonray", [CR + "fish_manta.glb"], "Tint pale blue, emissive at night. Target ~1.5 m wide."),
        ("Abyssal Koi", [CR + "fish_koi.glb"], "Tint dark + emissive. Target ~0.8 m."),
        ("Gloomfin (night hunter)", [CR + "anglerfish.glb", CR + "goblin_shark.glb", CR + "shark_c.glb"],
         "Attack, Swimming_Fast clips. Target ~2 m."),
        ("Crawler", [CR + "fish_generic_b.glb", CR + "sharky.glb"],
         "Fish-man humanoid (Walk, Punch, Death clips). Target ~1.6 m."),
        ("Reef Crab", [CR + "crab.glb"], "Target ~0.8 m wide (scale ~0.3)."),
        ("Rig Brute (miniboss)", [CR + "giant.glb", CR + "mech.glb"], "giant ~3.9 m tall (bind pose)."),
        ("Leviathan (boss)", [CR + "shark_a.glb", CR + "whale.glb", CR + "tentacle_a.glb", CR + "shark_b.glb"],
         "shark_a ~16 m with one Swim clip; add tentacle_a instances (Tentacle_Attack/Poke) rising around the atoll. Tint dark."),
        ("Ambient sea life (optional)", [CR + "fish_generic_c.glb", CR + "squidle.glb", CR + "glub.glb", CR + "snake.glb"], ""),
    ],
    "Boats": [
        ("Dinghy (2 seats)", [B + "kenney-watercraft/boat-row-small.glb", B + "lifeboat.glb", B + "kenney-nature/canoe.glb"],
         "Bow at +Z. Width includes the oars. Scale 1.2."),
        ("Skiff (4 seats)", [B + "kenney-watercraft/boat-fishing-small.glb", B + "kenney-watercraft/boat-row-large.glb",
                             B + "kenney-watercraft/boat-sail-a.glb"], "Bow at +Z. Scale 1.3."),
        ("Motor Launch (8 seats)", [B + "kenney-watercraft/boat-speed-a.glb", B + "kenney-watercraft/boat-speed-e.glb",
                                    B + "kenney-watercraft/boat-tug-a.glb"], "Bow at +Z. Scale 1.6 -> 5.4 m."),
        ("Trader boat", [B + "kenney-watercraft/boat-house-a.glb", B + "boat_q.glb"], "Bow at +Z. Scale 1.5."),
        ("Paddle / large ship dressing", [B + "kenney-nature/canoe_paddle.glb", B + "kenney-watercraft/ship-small.glb"], ""),
    ],
    "Items (held / dropped)": [
        ("Harpoon T1-T4", [IT + "spear_a.glb", IT + "spear_b.glb"],
         "spear_a lies along Z, tip at +Z, origin near the grip. Tint per tier (wood/iron/coral/abyss). Barbed Harpoon Tip: no model, use a Cone."),
        ("Driftwood Club", [IT + "kenney-pirate/tool-paddle.glb", D + "kenney-nature/log.glb"],
         "Paddle along Y, origin at the bottom. Scale 0.45 -> 1.05 m. Fallback: brown Capsule 0.08 x 0.9 m."),
        ("Scrap Machete", [IT + "machete_ct.glb", IT + "machete_pr.glb"],
         "machete_ct: blade along -Z from the handle origin. machete_pr: along Y, ~0.5 m."),
        ("Coral Cutlass", [IT + "cutlass.glb", IT + "sword_c.glb"], "Tint coral pink."),
        ("Abyss Blade", [IT + "sword_b.glb", IT + "sword_a.glb", IT + "sword_d.glb"],
         "No dedicated model: tint dark purple + emissive edge. sword_b scale 0.4."),
        ("Twig / Reed / Carbon Rod", [IT + "fishing_rod_c.glb", IT + "fishing_rod_a.glb", IT + "fishing_rod_b.glb",
                                      IT + "fishing_rod_d.glb", IT + "fishing_rod_e.glb"],
         "Order = Twig, Reed, Carbon (pick by look after a capture). All along Y, ~6-8 m: scale 0.25. Bobber: no model, red/white Sphere 0.08 m."),
        ("Bandage", [IT + "first_aid_a.glb", IT + "first_aid_b.glb"],
         "First-aid box, not a roll. Scale 0.2. Fallback: white Cylinder 0.06 x 0.1 m."),
        ("Biomass Salve", [IT + "potion_a.glb", IT + "potion_b.glb"], "Tint green. Scale 0.25."),
        ("Bait", [IT + "kenney-food/can-open.glb", IT + "kenney-food/mussel-open.glb", IT + "kenney-food/bag.glb"], ""),
        ("Flare", [IT + "flare_gun.glb"], "Long axis X. Scale 0.12. Flare light = emissive Sphere + point light."),
        ("Lucky Lure", [IT + "lure.glb"], "Long axis X. Scale 0.1."),
        ("Fish (held / cooked)", [IT + "kenney-food/fish.glb", IT + "kenney-survival/fish.glb", IT + "kenney-survival/fish-large.glb",
                                  IT + "kenney-food/skewer.glb", IT + "bucket_of_fish.glb"],
         "kenney-food/fish: head at +Z. Survival fish: long X, head at -X. Cooked = skewer or tint brown."),
        ("Food", [IT + "kenney-food/coconut.glb", IT + "kenney-food/coconut-half.glb", IT + "kenney-food/banana.glb",
                  IT + "kenney-food/pineapple.glb", IT + "kenney-food/meat-cooked.glb", IT + "kenney-food/meat-raw.glb",
                  IT + "kenney-food/can.glb", IT + "kenney-food/bowl-soup.glb", IT + "kenney-food/pot-stew.glb",
                  IT + "kenney-food/frying-pan.glb", IT + "kenney-survival/bottle.glb", IT + "kenney-pirate/bottle.glb"], ""),
        ("Oil (resource)", [IT + "kenney-food/bottle-oil.glb"], ""),
        ("Sac (bag)", [IT + "kenney-food/bag.glb"], "UI only in the spec; model for drops if needed."),
        ("Tools", [IT + "kenney-survival/tool-hammer.glb", IT + "kenney-survival/tool-hammer-upgraded.glb",
                   IT + "kenney-survival/tool-axe.glb", IT + "kenney-survival/tool-pickaxe.glb", IT + "kenney-pirate/tool-shovel.glb",
                   IT + "kenney-pirate/cannon-ball.glb"], "Build hammer for the build mode."),
        ("Coins", [], "No model. Fallback: gold Cylinder 0.15 x 0.02 m (UI-only wallet in the spec)."),
    ],
}

SOUNDS = [
    ("Harpoon throw", ["harpoon_throw_1.ogg", "harpoon_throw_2.ogg", "whoosh.wav"]),
    ("Harpoon hit (flesh / wood)", ["harpoon_hit_1.ogg", "harpoon_hit_2.ogg", "harpoon_hit_wood.ogg"]),
    ("Melee swing", ["melee_swing_1.ogg", "melee_swing_2.ogg", "whoosh.wav"]),
    ("Melee hit", ["melee_hit_1.ogg", "melee_hit_2.ogg", "melee_hit_3.ogg"]),
    ("Splash (player / debris / cast)", ["splash_1.ogg", "splash_2.ogg", "splash_3.ogg", "splash_4.ogg", "splash_big.wav", "beach_waves.ogg"]),
    ("Fishing cast / bite / reel / catch", ["bait_plop.ogg", "fishing_bite.wav", "fishing_bite_2.ogg", "fishing_reel.ogg", "fishing_catch.wav"]),
    ("Pickup / drop", ["pickup_1.ogg", "pickup_2.ogg", "item_drop.ogg", "debris_bump.ogg"]),
    ("Shredder (play in sequence, one per item)", ["shredder_1.ogg", "shredder_2.ogg", "shredder_3.ogg"]),
    ("Campfire loop (positional)", ["fire_loop.ogg", "fire_crackle.ogg"]),
    ("UI", ["ui_click.ogg", "ui_confirm.ogg", "ui_back.ogg", "ui_error.ogg", "ui_open.ogg", "ui_close.ogg", "ui_select.ogg"]),
    ("Coin (sell / buy / save)", ["coin_1.ogg", "coin_2.ogg"]),
    ("Craft / build", ["craft_1.ogg", "craft_2.ogg", "craft_3.ogg", "build_place_1.ogg", "build_place_2.ogg", "build_metal.ogg"]),
    ("Enemy hurt / growl / death", ["enemy_hurt_1.ogg", "enemy_hurt_2.ogg", "enemy_hurt_3.ogg", "enemy_hurt_4.ogg",
                                    "enemy_growl_1.ogg", "enemy_growl_2.ogg", "enemy_death_1.ogg", "enemy_death_2.ogg"]),
    ("Player hurt (no voice)", ["player_hurt_1.ogg", "player_hurt_2.ogg"]),
    ("Night sting / Leviathan", ["night_sting.ogg", "leviathan_rumble.ogg"]),
    ("Quest / trader arrival", ["quest_complete.ogg", "bell.ogg"]),
    ("Logbook / chest / equip", ["logbook_open.ogg", "logbook_flip.ogg", "logbook_close.ogg", "chest_open.ogg", "creak.ogg", "equip.ogg"]),
    ("Footsteps (raft wood / island sand)", ["footstep_wood_1.ogg", "footstep_wood_2.ogg", "footstep_wood_3.ogg",
                                             "footstep_sand_1.ogg", "footstep_sand_2.ogg", "footstep_sand_3.ogg"]),
    ("Ambience loops", ["ocean_loop.wav", "wind_loop.wav", "water_lap_loop.ogg", "underwater_loop.ogg"]),
    ("Boat engine loop / radio", ["boat_engine_loop.ogg", "radio_static.ogg"]),
]

CLIP_SET_NAMES = {"adventurer.glb": "QH", "pirate_captain.glb": "QP", "character-female-a.glb": "KM", "anglerfish.glb": "QF",
                  "crab.glb": "QC", "fish_generic_b.glb": "QG", "fish_sardine.glb": "QS", "giant.glb": "QE", "glub.glb": "QB",
                  "mech.glb": "QR", "shark_b.glb": "QK", "snake.glb": "QN", "tentacle_a.glb": "QT", "chest.glb": "KC",
                  "lever-single.glb": "KL"}


def fmt(v):
    return f"{v:.2f}".rstrip("0").rstrip(".")


def size_text(info):
    return " x ".join(fmt(v) for v in info["size_xyz"])


def origin_note(info):
    size, lo, hi = info["size_xyz"], info["min"], info["max"]
    notes = []
    if abs(lo[1]) > max(0.06, 0.05 * size[1]):
        notes.append(f"origin y {fmt(-lo[1])} above bottom")
    if any(abs(lo[i] + hi[i]) / 2 > 0.15 * size[i] + 0.02 for i in (0, 2)):
        notes.append(f"origin off-center (x {fmt(lo[0])}..{fmt(hi[0])}, z {fmt(lo[2])}..{fmt(hi[2])})")
    return notes


def main():
    infos, clip_sets = {}, {}
    for rows in MODELS.values():
        for _, paths, _ in rows:
            for p in paths:
                f = ASSETS / p
                assert f.is_file(), f"missing {p}"
                infos[p] = bounds.info(str(f))
    for p in sorted(str(f.relative_to(ASSETS)).replace("\\", "/") for f in (ASSETS / "models").rglob("*.glb")):
        info = infos.get(p) or bounds.info(str(ASSETS / p))
        if info["animations"]:
            clip_sets.setdefault(tuple(info["animations"]), []).append(p)
    set_of = {}
    for clips, paths in clip_sets.items():
        name = next(CLIP_SET_NAMES[pathlib.PurePath(p).name] for p in paths if pathlib.PurePath(p).name in CLIP_SET_NAMES)
        for p in paths:
            set_of[p] = name
    for s in SOUNDS:
        for f in s[1]:
            assert (ASSETS / "sfx" / f).is_file(), f"missing sfx/{f}"

    out = ["# Asset catalog", "",
           "Generated by `python -I tools/assets/catalog.py` (do not edit by hand). Paths are relative to `assets/`.",
           "Sizes are X x Y x Z in meters from `tools/assets/bounds.py` (glTF accessor bounds with node transforms).", "",
           "Conventions:", "",
           "- +Y up. Kenney and Quaternius models face +Z (Kenney boats: bow at +Z; Quaternius fish: head at +Z, "
           "checked by end taper). If game code treats -Z as forward, rotate the model by pi about Y.",
           "- Kenney kits are toy-scale (survival floor tile = 0.5 m, raft tile = 2 m): the Notes give suggested scale factors. "
           "Kenney origins are at the bottom center unless a note says otherwise (nature kit: bottom at y = -0.05).",
           "- Kenney GLBs use `Textures/colormap.png` next to them, one per kit folder. Do not move a GLB without its Textures folder.",
           "- Skinned models (marked with a clip set) report bind-pose bounds; the armature transform is not applied, so "
           "Quaternius fish and monsters can render at a very different size. Check with `rusting capture` and scale to the target size.",
           "- Rows with no path or a 'Fallback' note have no good asset: use the suggested engine primitive.", ""]
    for section, rows in MODELS.items():
        out += [f"## {section}", "", "| Role | Assets (size m) | Notes |", "| --- | --- | --- |"]
        for role, paths, notes in rows:
            cells = "<br>".join(f"`{p}` ({size_text(infos[p])}{', clips ' + set_of[p] if p in set_of else ''})" for p in paths) or "(none)"
            extra = origin_note(infos[paths[0]]) if paths else []
            note = " ".join(x for x in [notes, "; ".join(extra) + "." if extra else ""] if x)
            out.append(f"| {role} | {cells} | {note} |")
        out.append("")
    out += ["## Sounds", "", "All in `assets/sfx/`; reference them as `sfx/<file>`. Loops: ocean_loop, wind_loop "
            "(24 s, seamless, generated by `tools/assets/gen_ambience.py`), fire_loop, water_lap_loop, underwater_loop, "
            "boat_engine_loop, radio_static.", "", "| Role | Files |", "| --- | --- |"]
    out += [f"| {role} | {', '.join(f'`{f}`' for f in files)} |" for role, files in SOUNDS]
    out += ["", "Gaps: no voice for player hurt (impact sounds stand in), no eat/drink sound (use `pickup_2.ogg` / `bait_plop.ogg`).", ""]
    out += ["## Animation clip sets", "", "Exact glTF clip names (Blender truncated some names; match them exactly or by substring).", ""]
    for clips, paths in clip_sets.items():
        out += [f"### Set {set_of[paths[0]]}", "", "Models: " + ", ".join(f"`{p}`" for p in paths), "",
                "Clips: " + ", ".join(f"`{c}`" for c in clips), ""]
    (ASSETS / "CATALOG.md").write_text("\n".join(out), encoding="utf8")
    print(f"roles={sum(len(r) for r in MODELS.values())} models={len(infos)} sounds={sum(len(s[1]) for s in SOUNDS)} clip_sets={len(clip_sets)}")


def credits():
    groups = collections.defaultdict(list)
    for meta in sorted(ASSETS.rglob("*.rmeta")):
        src = json.loads(meta.read_text(encoding="utf8"))["source"]
        assert src["license"] == "CC0-1.0", meta
        asset = str(meta.relative_to(ASSETS).with_suffix("")).replace("\\", "/")
        key = src["url"] if src["url"] and "poly.pizza" not in src["url"] else src["generator"] or "poly.pizza"
        groups[key].append((asset, src["author"], src["url"]))
    out = ["# Credits", "", "Every file in `assets/` is CC0 1.0 (public domain dedication); credit is given as a courtesy.",
           "Per-file author, source URL and license are also stored in each `<file>.rmeta`. "
           "Regenerate with `python -I tools/assets/catalog.py`.", "",
           "| Pack / source | Author | URL | License | Files |", "| --- | --- | --- | --- | --- |"]
    for key, rows in sorted(groups.items(), key=lambda kv: (kv[0] == "poly.pizza", kv[0])):
        if key == "poly.pizza":
            continue
        name = key.rstrip("/").rsplit("/", 1)[-1]
        out.append(f"| {name} | {rows[0][1]} | {rows[0][2] or '`' + key + '` (this repo)'} | CC0-1.0 | {len(rows)} |")
    out += ["", "fisheefects is dual-licensed on OpenGameArt (CC0 / CC-BY); it is used under CC0.",
            "", "## poly.pizza models (CC0 1.0)", "", "| Asset | Author | URL |", "| --- | --- | --- |"]
    out += [f"| `{a}` | {au} | {u} |" for a, au, u in groups["poly.pizza"]]
    (ASSETS / "CREDITS.md").write_text("\n".join(out) + "\n", encoding="utf8")
    return sum(map(len, groups.values()))


if __name__ == "__main__":
    print(f"credited={credits()}")
    main()
