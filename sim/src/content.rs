//! Static game content (spec sections 4-14). Balance changes touch only this file.
//! Numbers marked "tuning" are not in the spec and were chosen by the sim author.

use crate::math::V3;
use crate::rng::Rng;
use serde::{Deserialize, Serialize};
use std::f32::consts::TAU;

// ============================================================ world layout (4)

/// Raft tile edge length (m).
pub const TILE: f32 = 2.0;
/// Raft deck top height above the water (m).
pub const DECK_Y: f32 = 0.45;
/// The starting raft is START_TILES x START_TILES tiles centred on the origin.
pub const START_TILES: i32 = 4;
/// The raft's bounding box may grow to at most this many tiles per side.
pub const MAX_RAFT_TILES: i32 = 16;
/// Ocean current velocity (m/s).
pub const CURRENT: V3 = V3::new(1.2, 0.0, 0.0);
/// Horizontal distance within which E works on structures, NPCs, chests, shrines (tuning).
pub const INTERACT_RANGE: f32 = 3.0;

// ============================================================ debris (4, 6.1)

pub const DEBRIS_SPAWN_X: f32 = -70.0;
/// Debris spawns at z in [-DEBRIS_SPAWN_HALF_Z, DEBRIS_SPAWN_HALF_Z].
pub const DEBRIS_SPAWN_HALF_Z: f32 = 40.0;
pub const DEBRIS_DESPAWN_X: f32 = 80.0;
/// Average seconds between debris spawns.
pub const DEBRIS_SPAWN_INTERVAL: f32 = 3.5;
pub const DEBRIS_MAX: usize = 45;
/// Spawn mix of floating debris (tuning).
pub const DEBRIS_WEIGHTS: [(ItemKind, f32); 5] = [
    (ItemKind::Plank, 35.0),
    (ItemKind::PalmBundle, 25.0),
    (ItemKind::ScrapChunk, 20.0),
    (ItemKind::FloatingCrate, 12.0),
    (ItemKind::OilBarrel, 8.0),
];
/// E picks up debris within this distance (tuning).
pub const DEBRIS_PICKUP_RANGE: f32 = 3.0;
/// Speed a harpooned item is dragged toward its player (tuning).
pub const HARPOON_DRAG_SPEED: f32 = 12.0;
/// A dragged item enters the sac within this distance of its player (tuning).
pub const DRAG_ARRIVE_DIST: f32 = 1.5;

// ============================================================ day/night (5)

pub const CYCLE_SECONDS: f32 = 360.0;
pub const DAY_SECONDS: f32 = 300.0;
/// Dawn blend: first DAWN_SECONDS of the day.
pub const DAWN_SECONDS: f32 = 15.0;
/// Dusk blend: last DUSK_SECONDS of the day.
pub const DUSK_SECONDS: f32 = 20.0;
pub const NIGHT_SECONDS: f32 = CYCLE_SECONDS - DAY_SECONDS;

pub const GLOOMFIN_MAX_PER_PLAYER: u32 = 3;
pub const GLOOMFIN_MAX_TOTAL: usize = 12;
/// Gloomfin spawn this far from their target player (tuning).
pub const GLOOMFIN_SPAWN_DIST: f32 = 30.0;
/// Seconds between crawlers climbing aboard while the fire is out at night.
pub const CRAWLER_INTERVAL: f32 = 8.0;

/// Gloomfin allowed per hunted player on `day`: 1 + day/3, max 3.
pub fn gloomfin_per_player(day: u32) -> u32 {
    (1 + day / 3).min(GLOOMFIN_MAX_PER_PLAYER)
}

/// Raft crawlers allowed on `day`: 2 + day/2.
pub fn crawler_max(day: u32) -> u32 {
    2 + day / 2
}

// ============================================================ player (6)

pub const MAX_HEALTH: f32 = 100.0;
pub const MAX_HUNGER: f32 = 100.0;
/// Hunger lost per second (100 per 420 s).
pub const HUNGER_DRAIN: f32 = 100.0 / 420.0;
/// Hunger drain multiplier while sprinting or swimming fast.
pub const HUNGER_FAST_MULT: f32 = 1.5;
/// Health lost per second at hunger 0.
pub const STARVE_DPS: f32 = 1.0;
pub const WALK_SPEED: f32 = 4.5;
pub const SPRINT_SPEED: f32 = 7.0;
pub const SWIM_SPEED: f32 = 4.0;
pub const SWIM_FAST_SPEED: f32 = 5.5;
pub const DIVE_DEPTH: f32 = 3.0;
pub const REVIVE_SECONDS: f32 = 3.0;
pub const REVIVE_RANGE: f32 = 2.0;
pub const REVIVE_HEALTH: f32 = 30.0;
/// Grapple pull speed toward a harpooned surface.
pub const GRAPPLE_SPEED: f32 = 16.0;
/// A lit flare lasts this long.
pub const FLARE_SECONDS: f32 = 30.0;
/// Gloomfin keep this far away from a player holding a lit flare (tuning).
pub const FLARE_RADIUS: f32 = 10.0;

// ============================================================ items (6.2)

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Fish {
    Sardine,
    Mackerel,
    Goldfin,
    Moonray,
    AbyssalKoi,
}

pub struct FishDef {
    pub name: &'static str,
    pub cooked_name: &'static str,
    /// Base catch weight (relative).
    pub weight: f32,
    pub price: u32,
    /// Hunger restored when eaten cooked (raw gives RAW_FISH_FACTOR of it).
    pub cooked_hunger: f32,
    /// Rare fish (Goldfin and rarer) get rod, bait and lure multipliers.
    pub rare: bool,
    /// Minigame green zone width as a fraction of the bar (tuning).
    pub zone: f32,
    /// Minigame marker speed in bar widths per second (tuning).
    pub marker_speed: f32,
}

pub const FISH: [FishDef; 5] = [
    FishDef {
        name: "Sardine",
        cooked_name: "Cooked Sardine",
        weight: 60.0,
        price: 4,
        cooked_hunger: 15.0,
        rare: false,
        zone: 0.30,
        marker_speed: 0.8,
    },
    FishDef {
        name: "Mackerel",
        cooked_name: "Cooked Mackerel",
        weight: 28.0,
        price: 10,
        cooked_hunger: 25.0,
        rare: false,
        zone: 0.24,
        marker_speed: 1.0,
    },
    FishDef {
        name: "Goldfin",
        cooked_name: "Cooked Goldfin",
        weight: 9.0,
        price: 35,
        cooked_hunger: 40.0,
        rare: true,
        zone: 0.18,
        marker_speed: 1.3,
    },
    FishDef {
        name: "Moonray",
        cooked_name: "Cooked Moonray",
        weight: 2.6,
        price: 110,
        cooked_hunger: 60.0,
        rare: true,
        zone: 0.12,
        marker_speed: 1.6,
    },
    FishDef {
        name: "Abyssal Koi",
        cooked_name: "Cooked Abyssal Koi",
        weight: 0.4,
        price: 400,
        cooked_hunger: 100.0,
        rare: true,
        zone: 0.08,
        marker_speed: 2.0,
    },
];

impl Fish {
    pub const ALL: [Fish; 5] = [
        Fish::Sardine,
        Fish::Mackerel,
        Fish::Goldfin,
        Fish::Moonray,
        Fish::AbyssalKoi,
    ];

    pub fn def(self) -> &'static FishDef {
        &FISH[self as usize]
    }
}

/// Every item that can sit in a sac slot. Gear upgrades are not items: they are
/// `Goods` applied to the player on the spot.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum ItemKind {
    // floating debris
    Plank,
    PalmBundle,
    ScrapChunk,
    FloatingCrate,
    OilBarrel,
    // carcasses (also debris: they float and go into the shredder)
    CrawlerCarcass,
    GloomfinCarcass,
    ReefCrabCarcass,
    BruteCarcass,
    // fish
    Fish(Fish),
    CookedFish(Fish),
    // consumables
    Bandage,
    BiomassSalve,
    Bait,
    Flare,
    // quest
    BeaconCore,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ItemClass {
    Debris,
    Fish,
    Food,
    Heal,
    Bait,
    Misc,
    Quest,
}

impl ItemKind {
    /// Every item kind (for UI lists and tests).
    pub fn all() -> Vec<ItemKind> {
        use ItemKind::*;
        let mut v = vec![
            Plank,
            PalmBundle,
            ScrapChunk,
            FloatingCrate,
            OilBarrel,
            CrawlerCarcass,
            GloomfinCarcass,
            ReefCrabCarcass,
            BruteCarcass,
        ];
        v.extend(crate::content::Fish::ALL.iter().map(|f| ItemKind::Fish(*f)));
        v.extend(crate::content::Fish::ALL.iter().map(|f| CookedFish(*f)));
        v.extend([Bandage, BiomassSalve, Bait, Flare, BeaconCore]);
        v
    }

    pub fn name(self) -> &'static str {
        use ItemKind::*;
        match self {
            Plank => "Driftwood Plank",
            PalmBundle => "Palm Bundle",
            ScrapChunk => "Scrap Chunk",
            FloatingCrate => "Floating Crate",
            OilBarrel => "Oil Barrel",
            CrawlerCarcass => "Crawler Carcass",
            GloomfinCarcass => "Gloomfin Carcass",
            ReefCrabCarcass => "Reef Crab Carcass",
            BruteCarcass => "Rig Brute Carcass",
            Fish(f) => f.def().name,
            CookedFish(f) => f.def().cooked_name,
            Bandage => "Bandage",
            BiomassSalve => "Biomass Salve",
            Bait => "Bait",
            Flare => "Flare",
            BeaconCore => "Beacon Core",
        }
    }

    pub fn class(self) -> ItemClass {
        use ItemKind::*;
        match self {
            Plank | PalmBundle | ScrapChunk | FloatingCrate | OilBarrel | CrawlerCarcass
            | GloomfinCarcass | ReefCrabCarcass | BruteCarcass => ItemClass::Debris,
            Fish(_) => ItemClass::Fish,
            CookedFish(_) => ItemClass::Food,
            Bandage | BiomassSalve => ItemClass::Heal,
            Bait => ItemClass::Bait,
            Flare => ItemClass::Misc,
            BeaconCore => ItemClass::Quest,
        }
    }

    /// Max items per sac slot (6.2). Raw fish stack 5; cooked fish count as food (10).
    pub fn stack_size(self) -> u32 {
        match self.class() {
            ItemClass::Debris => 10,
            ItemClass::Fish => 5,
            ItemClass::Food | ItemClass::Heal => 10,
            ItemClass::Bait => 20,
            ItemClass::Misc | ItemClass::Quest => 1,
        }
    }

    /// Floats in the ocean and can be fed to the shredder.
    pub fn is_debris(self) -> bool {
        self.class() == ItemClass::Debris
    }

    /// Hunger restored when eaten, if edible. Raw fish give half.
    pub fn hunger(self) -> Option<f32> {
        match self {
            ItemKind::Fish(f) => Some(f.def().cooked_hunger * RAW_FISH_FACTOR),
            ItemKind::CookedFish(f) => Some(f.def().cooked_hunger),
            _ => None,
        }
    }

    /// Health restored when used from the heal slot.
    pub fn heal(self) -> Option<f32> {
        match self {
            ItemKind::Bandage => Some(30.0),
            ItemKind::BiomassSalve => Some(60.0),
            _ => None,
        }
    }

    /// Barnaby's price per item (raw or cooked fish).
    pub fn sell_price(self) -> Option<u32> {
        match self {
            ItemKind::Fish(f) | ItemKind::CookedFish(f) => Some(f.def().price),
            _ => None,
        }
    }

    pub fn is_fish(self) -> bool {
        matches!(self, ItemKind::Fish(_) | ItemKind::CookedFish(_))
    }
}

// ============================================================ stockpile and goods

/// Shared raft resources; also used as a crafting cost.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Stockpile {
    pub wood: u32,
    pub metal: u32,
    pub biomass: u32,
    pub oil: u32,
}

impl Stockpile {
    pub const fn new(wood: u32, metal: u32, biomass: u32, oil: u32) -> Stockpile {
        Stockpile {
            wood,
            metal,
            biomass,
            oil,
        }
    }

    pub fn covers(&self, cost: &Stockpile) -> bool {
        self.wood >= cost.wood
            && self.metal >= cost.metal
            && self.biomass >= cost.biomass
            && self.oil >= cost.oil
    }

    /// Subtracts `cost` if affordable; changes nothing otherwise.
    pub fn take(&mut self, cost: &Stockpile) -> bool {
        if !self.covers(cost) {
            return false;
        }
        self.wood -= cost.wood;
        self.metal -= cost.metal;
        self.biomass -= cost.biomass;
        self.oil -= cost.oil;
        true
    }

    pub fn add(&mut self, o: &Stockpile) {
        self.wood = self.wood.saturating_add(o.wood);
        self.metal = self.metal.saturating_add(o.metal);
        self.biomass = self.biomass.saturating_add(o.biomass);
        self.oil = self.oil.saturating_add(o.oil);
    }
}

/// Anything a player can receive: from shops, chests, quests and crafting.
/// Apply with `World::give` / `World::give_all`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Goods {
    /// Items into the sac.
    Item(ItemKind, u32),
    /// Coins into the wallet.
    Coins(u32),
    /// Resources into the shared stockpile.
    Stock(Stockpile),
    /// Sac capacity raised to this (never lowered).
    Sac(u32),
    /// Harpoon tier raised to this (1..=4).
    Harpoon(u8),
    /// Melee tier raised to this (1..=4).
    Melee(u8),
    /// Rod tier raised to this (1..=3).
    Rod(u8),
    BarbedTip,
    LuckyLure,
}

impl Goods {
    pub fn label(&self) -> String {
        match *self {
            Goods::Item(k, 1) => k.name().to_string(),
            Goods::Item(k, n) => format!("{} x{n}", k.name()),
            Goods::Coins(n) => format!("{n} coins"),
            Goods::Stock(s) => {
                let parts: Vec<String> = [
                    (s.wood, "wood"),
                    (s.metal, "metal"),
                    (s.biomass, "biomass"),
                    (s.oil, "oil"),
                ]
                .iter()
                .filter(|(n, _)| *n > 0)
                .map(|(n, w)| format!("{n} {w}"))
                .collect();
                parts.join(", ")
            }
            Goods::Sac(c) => format!("Sac {c}"),
            Goods::Harpoon(t) => harpoon(t).name.to_string(),
            Goods::Melee(t) => melee(t).name.to_string(),
            Goods::Rod(t) => rod(t).name.to_string(),
            Goods::BarbedTip => "Barbed Harpoon Tip".to_string(),
            Goods::LuckyLure => "Lucky Lure".to_string(),
        }
    }
}

// ============================================================ hotbar gear (6.1)

pub struct HarpoonDef {
    pub name: &'static str,
    pub range: f32,
    pub damage: f32,
    pub cooldown: f32,
    /// Seconds a hit enemy is stunned (0 = none).
    pub stun: f32,
}

pub const HARPOONS: [HarpoonDef; 4] = [
    HarpoonDef {
        name: "Harpoon T1",
        range: 14.0,
        damage: 15.0,
        cooldown: 1.2,
        stun: 0.0,
    },
    HarpoonDef {
        name: "Harpoon T2",
        range: 20.0,
        damage: 25.0,
        cooldown: 1.2,
        stun: 0.0,
    },
    HarpoonDef {
        name: "Harpoon T3",
        range: 28.0,
        damage: 40.0,
        cooldown: 1.2,
        stun: 0.0,
    },
    HarpoonDef {
        name: "Harpoon T4",
        range: 36.0,
        damage: 60.0,
        cooldown: 1.2,
        stun: 1.5,
    },
];
/// Extra harpoon damage with the Barbed Harpoon Tip.
pub const BARBED_TIP_DAMAGE: f32 = 10.0;
/// A harpooned enemy is yanked this far toward the shooter (tuning).
pub const HARPOON_ENEMY_PULL: f32 = 3.0;

pub struct MeleeDef {
    pub name: &'static str,
    pub damage: f32,
}

pub const MELEE: [MeleeDef; 4] = [
    MeleeDef {
        name: "Driftwood Club",
        damage: 12.0,
    },
    MeleeDef {
        name: "Scrap Machete",
        damage: 22.0,
    },
    MeleeDef {
        name: "Coral Cutlass",
        damage: 35.0,
    },
    MeleeDef {
        name: "Abyss Blade",
        damage: 55.0,
    },
];
pub const MELEE_SWING: f32 = 0.6;
/// Melee reach (tuning).
pub const MELEE_RANGE: f32 = 2.5;

pub struct RodDef {
    pub name: &'static str,
    /// Weight multiplier on rare fish (Goldfin and rarer).
    pub rare_mult: f32,
}

pub const RODS: [RodDef; 3] = [
    RodDef {
        name: "Twig Rod",
        rare_mult: 1.0,
    },
    RodDef {
        name: "Reed Rod",
        rare_mult: 1.6,
    },
    RodDef {
        name: "Carbon Rod",
        rare_mult: 2.5,
    },
];

/// Tier lookups clamp out-of-range tiers.
pub fn harpoon(tier: u8) -> &'static HarpoonDef {
    &HARPOONS[(tier.clamp(1, 4) - 1) as usize]
}
pub fn melee(tier: u8) -> &'static MeleeDef {
    &MELEE[(tier.clamp(1, 4) - 1) as usize]
}
pub fn rod(tier: u8) -> &'static RodDef {
    &RODS[(tier.clamp(1, 3) - 1) as usize]
}

/// Sac capacities in upgrade order; the first is the start.
pub const SAC_CAPACITIES: [u32; 5] = [8, 12, 16, 24, 32];

// ============================================================ fishing (7)

pub const CAST_RANGE: f32 = 12.0;
pub const BITE_MIN: f32 = 4.0;
pub const BITE_MAX: f32 = 10.0;
pub const BAIT_MULT: f32 = 1.5;
pub const LUCKY_LURE_MULT: f32 = 1.5;
/// Night multiplier on Moonray and Abyssal Koi.
pub const NIGHT_MULT: f32 = 2.0;
/// Raw fish restore this fraction of their cooked hunger.
pub const RAW_FISH_FACTOR: f32 = 0.5;
/// A hooked fish escapes if the client sends no ReelIn within this many seconds (tuning).
pub const HOOK_SECONDS: f32 = 10.0;
/// The line breaks when the player is farther than CAST_RANGE + this from the bobber.
pub const LINE_SLACK: f32 = 4.0;

/// Catch weights per `Fish::ALL` index after rod/bait/lure/night modifiers.
/// Rod, bait and lure multiply rare fish (Goldfin and rarer); night doubles Moonray and Koi.
pub fn fish_weights(rod_tier: u8, bait: bool, night: bool, lucky_lure: bool) -> [f32; 5] {
    let mut w = [0.0; 5];
    for (i, f) in Fish::ALL.iter().enumerate() {
        let d = f.def();
        let mut x = d.weight;
        if d.rare {
            x *= rod(rod_tier).rare_mult;
            if bait {
                x *= BAIT_MULT;
            }
            if lucky_lure {
                x *= LUCKY_LURE_MULT;
            }
        }
        if night && matches!(f, Fish::Moonray | Fish::AbyssalKoi) {
            x *= NIGHT_MULT;
        }
        w[i] = x;
    }
    w
}

pub fn roll_fish(rng: &mut Rng, rod_tier: u8, bait: bool, night: bool, lucky_lure: bool) -> Fish {
    Fish::ALL[rng.weighted(&fish_weights(rod_tier, bait, night, lucky_lure))]
}

// ============================================================ shredder (8)

pub const SHRED_SECONDS: f32 = 2.0;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ShredOutput {
    pub stock: Stockpile,
    /// Bait for the depositor's sac.
    pub bait: u32,
    /// Coins for the depositor's wallet.
    pub coins: u32,
}

/// What one shredded item yields; None for items the shredder refuses.
pub fn shred(kind: ItemKind, rng: &mut Rng) -> Option<ShredOutput> {
    use ItemKind::*;
    let stock = |w, m, b, o| {
        Some(ShredOutput {
            stock: Stockpile::new(w, m, b, o),
            ..Default::default()
        })
    };
    match kind {
        Plank => stock(3, 0, 0, 0),
        PalmBundle => stock(2, 0, 0, 0),
        ScrapChunk => stock(0, 2, 0, 0),
        OilBarrel => stock(0, 0, 0, 2),
        FloatingCrate => {
            let mut out = ShredOutput {
                stock: Stockpile::new(rng.range_u32(2, 4), rng.range_u32(1, 2), 0, 0),
                ..Default::default()
            };
            if rng.chance(0.2) {
                out.bait = 2;
            }
            if rng.chance(0.1) {
                out.coins = rng.range_u32(10, 30);
            }
            Some(out)
        }
        CrawlerCarcass => stock(0, 0, 2, 0),
        GloomfinCarcass => stock(0, 0, 3, 0),
        ReefCrabCarcass => stock(0, 0, 1, 0),
        BruteCarcass => stock(0, 0, 15, 0),
        _ => None,
    }
}

// ============================================================ campfire (8)

/// Total upgrade points needed for levels 2, 3, 4, 5.
pub const CAMPFIRE_THRESHOLDS: [u32; 4] = [25, 50, 75, 100];
pub const CAMPFIRE_MAX_LEVEL: u8 = 5;
/// One feed takes this much from the stockpile.
pub const FEED_WOOD: u32 = 5;
pub const FEED_OIL: u32 = 1;
pub const POINTS_PER_WOOD: u32 = 1;
pub const POINTS_PER_OIL: u32 = 5;
pub const FUEL_PER_WOOD: f32 = 20.0;
pub const FUEL_PER_OIL: f32 = 100.0;
pub const FUEL_CAP: f32 = 600.0;
pub const FUEL_START: f32 = 240.0;
/// Light radius per level 1..=5. Out of fuel = no light.
pub const LIGHT_RADIUS: [f32; 5] = [6.0, 8.0, 10.0, 12.0, 15.0];

pub fn campfire_level(points: u32) -> u8 {
    1 + CAMPFIRE_THRESHOLDS.iter().filter(|t| points >= **t).count() as u8
}

pub fn light_radius(level: u8) -> f32 {
    LIGHT_RADIUS[(level.clamp(1, 5) - 1) as usize]
}

// ============================================================ placeables and farms (8, 10)

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum PlaceableKind {
    Campfire,
    Workbench,
    Logbook,
    Shredder,
    WoodFarm,
    MetalFarm,
    OilFarm,
    RadioStation,
}

impl PlaceableKind {
    pub fn name(self) -> &'static str {
        match self {
            PlaceableKind::Campfire => "Campfire",
            PlaceableKind::Workbench => "Workbench",
            PlaceableKind::Logbook => "Captain's Logbook",
            PlaceableKind::Shredder => "Shredder",
            PlaceableKind::WoodFarm => "Wood Farm",
            PlaceableKind::MetalFarm => "Metal Farm",
            PlaceableKind::OilFarm => "Oil Farm",
            PlaceableKind::RadioStation => "Radio Station",
        }
    }

    /// Footprint in tiles (along +X, along +Z) at rotation 0; odd rotations swap it.
    pub fn footprint(self) -> (i32, i32) {
        match self {
            PlaceableKind::WoodFarm | PlaceableKind::MetalFarm | PlaceableKind::OilFarm => (2, 1),
            _ => (1, 1),
        }
    }

    pub fn is_farm(self) -> bool {
        farm(self).is_some()
    }
}

pub struct FarmDef {
    pub kind: PlaceableKind,
    /// Seconds per cycle at rate 1.
    pub cycle: f32,
    pub output: ItemKind,
    pub count: u32,
}

pub const FARMS: [FarmDef; 3] = [
    FarmDef {
        kind: PlaceableKind::WoodFarm,
        cycle: 90.0,
        output: ItemKind::PalmBundle,
        count: 4,
    },
    FarmDef {
        kind: PlaceableKind::MetalFarm,
        cycle: 120.0,
        output: ItemKind::ScrapChunk,
        count: 3,
    },
    FarmDef {
        kind: PlaceableKind::OilFarm,
        cycle: 150.0,
        output: ItemKind::OilBarrel,
        count: 2,
    },
];

pub fn farm(kind: PlaceableKind) -> Option<&'static FarmDef> {
    FARMS.iter().find(|f| f.kind == kind)
}

/// Farm speed with `survivors` rescued: 1 without, else 1 + 0.25 * (survivors - 1).
pub fn farm_rate(survivors: usize) -> f32 {
    if survivors == 0 {
        1.0
    } else {
        1.0 + 0.25 * (survivors - 1) as f32
    }
}

// ============================================================ crafting (10)

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum RecipeId {
    RaftFloor,
    Railing,
    Dinghy,
    ScrapMachete,
    Bandage,
    Bait,
    WoodFarm,
    MetalFarm,
    OilFarm,
    CarbonRod,
    BiomassSalve,
    RadioStation,
    Skiff,
    MotorLaunch,
    AbyssBlade,
    BeaconCore,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecipeOutput {
    /// Placed with `Request::Place` next to an existing tile.
    Floor,
    /// Placed with `Request::PlaceRailing` on a tile edge.
    Railing,
    /// Placed with `Request::Place` on free tiles.
    Structure(PlaceableKind),
    /// `Request::Craft`: spawns at the raft dock.
    Boat(BoatKind),
    /// `Request::Craft`: given to the crafting player.
    Give(Goods),
}

pub struct RecipeDef {
    pub id: RecipeId,
    pub name: &'static str,
    /// Campfire level that unlocks it.
    pub level: u8,
    pub cost: Stockpile,
    pub output: RecipeOutput,
}

impl RecipeDef {
    /// True for recipes made with `Request::Place` / `PlaceRailing` instead of `Craft`.
    pub fn is_placed(&self) -> bool {
        matches!(
            self.output,
            RecipeOutput::Floor | RecipeOutput::Railing | RecipeOutput::Structure(_)
        )
    }
}

const fn s(wood: u32, metal: u32, biomass: u32, oil: u32) -> Stockpile {
    Stockpile::new(wood, metal, biomass, oil)
}

/// Indexed by `RecipeId as usize`.
pub const RECIPES: [RecipeDef; 16] = [
    RecipeDef {
        id: RecipeId::RaftFloor,
        name: "Raft Floor",
        level: 1,
        cost: s(4, 0, 0, 0),
        output: RecipeOutput::Floor,
    },
    RecipeDef {
        id: RecipeId::Railing,
        name: "Railing",
        level: 1,
        cost: s(2, 0, 0, 0),
        output: RecipeOutput::Railing,
    },
    RecipeDef {
        id: RecipeId::Dinghy,
        name: "Dinghy",
        level: 2,
        cost: s(30, 8, 0, 0),
        output: RecipeOutput::Boat(BoatKind::Dinghy),
    },
    RecipeDef {
        id: RecipeId::ScrapMachete,
        name: "Scrap Machete",
        level: 2,
        cost: s(4, 6, 0, 0),
        output: RecipeOutput::Give(Goods::Melee(2)),
    },
    RecipeDef {
        id: RecipeId::Bandage,
        name: "Bandage x2",
        level: 2,
        cost: s(1, 0, 2, 0),
        output: RecipeOutput::Give(Goods::Item(ItemKind::Bandage, 2)),
    },
    RecipeDef {
        id: RecipeId::Bait,
        name: "Bait x3",
        level: 2,
        cost: s(0, 0, 2, 0),
        output: RecipeOutput::Give(Goods::Item(ItemKind::Bait, 3)),
    },
    RecipeDef {
        id: RecipeId::WoodFarm,
        name: "Wood Farm",
        level: 3,
        cost: s(20, 10, 0, 0),
        output: RecipeOutput::Structure(PlaceableKind::WoodFarm),
    },
    RecipeDef {
        id: RecipeId::MetalFarm,
        name: "Metal Farm",
        level: 3,
        cost: s(10, 25, 0, 0),
        output: RecipeOutput::Structure(PlaceableKind::MetalFarm),
    },
    RecipeDef {
        id: RecipeId::OilFarm,
        name: "Oil Farm",
        level: 3,
        cost: s(15, 20, 10, 0),
        output: RecipeOutput::Structure(PlaceableKind::OilFarm),
    },
    RecipeDef {
        id: RecipeId::CarbonRod,
        name: "Carbon Rod",
        level: 3,
        cost: s(0, 12, 6, 0),
        output: RecipeOutput::Give(Goods::Rod(3)),
    },
    RecipeDef {
        id: RecipeId::BiomassSalve,
        name: "Biomass Salve",
        level: 3,
        cost: s(0, 0, 4, 0),
        output: RecipeOutput::Give(Goods::Item(ItemKind::BiomassSalve, 1)),
    },
    RecipeDef {
        id: RecipeId::RadioStation,
        name: "Radio Station",
        level: 4,
        cost: s(0, 20, 15, 10),
        output: RecipeOutput::Structure(PlaceableKind::RadioStation),
    },
    RecipeDef {
        id: RecipeId::Skiff,
        name: "Skiff",
        level: 4,
        cost: s(40, 30, 0, 0),
        output: RecipeOutput::Boat(BoatKind::Skiff),
    },
    RecipeDef {
        id: RecipeId::MotorLaunch,
        name: "Motor Launch",
        level: 5,
        cost: s(20, 40, 0, 20),
        output: RecipeOutput::Boat(BoatKind::MotorLaunch),
    },
    RecipeDef {
        id: RecipeId::AbyssBlade,
        name: "Abyss Blade",
        level: 5,
        cost: s(0, 30, 20, 0),
        output: RecipeOutput::Give(Goods::Melee(4)),
    },
    RecipeDef {
        id: RecipeId::BeaconCore,
        name: "Beacon Core",
        level: 5,
        cost: s(0, 50, 30, 30),
        output: RecipeOutput::Give(Goods::Item(ItemKind::BeaconCore, 1)),
    },
];

impl RecipeId {
    pub fn def(self) -> &'static RecipeDef {
        &RECIPES[self as usize]
    }
}

/// The recipe that places a structure kind, if any.
pub fn recipe_for(kind: PlaceableKind) -> Option<&'static RecipeDef> {
    RECIPES
        .iter()
        .find(|r| r.output == RecipeOutput::Structure(kind))
}

/// Floors, structures and railings are placed within this horizontal distance of the
/// player (the ghost preview's reach, tuning).
pub const PLACE_RANGE: f32 = 8.0;
/// Crafted boats moor this far apart along the dock edge (tuning).
pub const DOCK_SPACING: f32 = 7.0;

// ============================================================ boats (11)

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum BoatKind {
    Dinghy,
    Skiff,
    MotorLaunch,
}

pub struct BoatDef {
    pub name: &'static str,
    /// Seat 0 is the driver.
    pub seats: u8,
    pub speed: f32,
    /// Seconds of driving per 1 oil from the stockpile (None = no fuel).
    pub oil_interval: Option<f32>,
    /// Hull radius for the island push-out (tuning).
    pub radius: f32,
}

pub const BOATS: [BoatDef; 3] = [
    BoatDef {
        name: "Dinghy",
        seats: 2,
        speed: 9.0,
        oil_interval: None,
        radius: 1.5,
    },
    BoatDef {
        name: "Skiff",
        seats: 4,
        speed: 15.0,
        oil_interval: None,
        radius: 2.0,
    },
    BoatDef {
        name: "Motor Launch",
        seats: 8,
        speed: 24.0,
        oil_interval: Some(60.0),
        radius: 3.0,
    },
];

impl BoatKind {
    pub fn def(self) -> &'static BoatDef {
        &BOATS[self as usize]
    }
}

/// E boards a boat within this distance of its hull centre (tuning).
pub const BOARD_RANGE: f32 = 4.0;
/// Leaving a boat whose hull centre is this close to a raft tile centre puts the player
/// on that tile (tuning).
pub const DISEMBARK_RAFT_RANGE: f32 = 6.0;
/// Leaving a boat this close to an island shore (beyond its hull radius) puts the player
/// ashore (tuning).
pub const DISEMBARK_SHORE_RANGE: f32 = 4.0;
/// Seated players' feet height above the water (tuning).
pub const BOAT_SEAT_Y: f32 = 0.4;
/// Distance between seat rows along the hull (tuning).
pub const BOAT_ROW_SPACING: f32 = 1.2;

// ============================================================ shops (12)

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Shop {
    Barnaby,
    Trader,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Offer {
    pub goods: Goods,
    pub price: u32,
}

pub const BARNABY_STOCK: [Offer; 4] = [
    Offer {
        goods: Goods::Rod(2),
        price: 60,
    },
    Offer {
        goods: Goods::Harpoon(2),
        price: 120,
    },
    Offer {
        goods: Goods::Sac(12),
        price: 80,
    },
    Offer {
        goods: Goods::Item(ItemKind::Bait, 5),
        price: 20,
    },
];

pub const TRADER_POOL: [Offer; 9] = [
    Offer {
        goods: Goods::Sac(16),
        price: 150,
    },
    Offer {
        goods: Goods::Sac(24),
        price: 400,
    },
    Offer {
        goods: Goods::Sac(32),
        price: 900,
    },
    Offer {
        goods: Goods::Item(ItemKind::Bait, 5),
        price: 25,
    },
    Offer {
        goods: Goods::Item(ItemKind::Bandage, 3),
        price: 40,
    },
    Offer {
        goods: Goods::Item(ItemKind::Flare, 1),
        price: 60,
    },
    Offer {
        goods: Goods::BarbedTip,
        price: 250,
    },
    Offer {
        goods: Goods::LuckyLure,
        price: 300,
    },
    Offer {
        goods: Goods::Rod(3),
        price: 350,
    },
];
pub const TRADER_OFFERS_PER_VISIT: usize = 5;
/// The trader arrives at the dawn of every day divisible by this (2, 4, 6...).
pub const TRADER_EVERY_DAYS: u32 = 2;
/// Barnaby's shop and quests work within this distance of Spot::Barnaby (tuning).
pub const SHOP_RANGE: f32 = 6.0;
/// The wandering trader sells within this distance of its dock (tuning).
pub const TRADER_RANGE: f32 = 8.0;

// ============================================================ enemies (14)

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum EnemyKind {
    Gloomfin,
    Crawler,
    ReefCrab,
    RigBrute,
    Leviathan,
}

pub struct EnemyDef {
    pub name: &'static str,
    pub hp: f32,
    pub damage: f32,
    /// Seconds between attacks.
    pub attack_interval: f32,
    pub speed: f32,
    /// Bites/hits land within this distance (tuning).
    pub attack_range: f32,
    /// Notices players within this distance (tuning).
    pub aggro_range: f32,
    pub carcass: Option<ItemKind>,
    /// Lives in the water (Gloomfin, Leviathan) instead of on ground.
    pub swims: bool,
}

pub const ENEMIES: [EnemyDef; 5] = [
    EnemyDef {
        name: "Gloomfin",
        hp: 40.0,
        damage: 15.0,
        attack_interval: 1.5,
        speed: 7.0,
        attack_range: 1.8,
        aggro_range: 40.0,
        carcass: Some(ItemKind::GloomfinCarcass),
        swims: true,
    },
    EnemyDef {
        name: "Crawler",
        hp: 30.0,
        damage: 10.0,
        attack_interval: 1.2,
        speed: 3.5,
        attack_range: 1.5,
        aggro_range: 15.0,
        carcass: Some(ItemKind::CrawlerCarcass),
        swims: false,
    },
    EnemyDef {
        name: "Reef Crab",
        hp: 20.0,
        damage: 6.0,
        attack_interval: 1.0,
        speed: 3.0,
        attack_range: 1.3,
        aggro_range: 10.0,
        carcass: Some(ItemKind::ReefCrabCarcass),
        swims: false,
    },
    EnemyDef {
        name: "Rig Brute",
        hp: 300.0,
        damage: 25.0,
        attack_interval: 2.0,
        speed: 3.0,
        attack_range: 2.5,
        aggro_range: 20.0,
        carcass: Some(ItemKind::BruteCarcass),
        swims: false,
    },
    EnemyDef {
        name: "Leviathan",
        hp: 1200.0,
        damage: 35.0,
        attack_interval: 2.5,
        speed: 9.0,
        attack_range: 6.0,
        aggro_range: 80.0,
        carcass: None,
        swims: true,
    },
];

impl EnemyKind {
    pub fn def(self) -> &'static EnemyDef {
        &ENEMIES[self as usize]
    }
}

/// HP multiplier by island tier 0..=4 (tier 0 and open-water/raft enemies use 1.0).
pub const TIER_HP_MULT: [f32; 5] = [1.0, 1.0, 1.3, 1.7, 2.2];
pub const ENEMY_RESPAWN_DAYS: u32 = 2;

pub fn tier_hp_mult(tier: u8) -> f32 {
    TIER_HP_MULT[tier.min(4) as usize]
}

// ============================================================ islands (4, 12)

/// Index into `ISLANDS` (and `Islands::list`).
pub type IslandId = u8;

pub const HOOK_ISLE: IslandId = 0;
pub const PALM_REEF: IslandId = 1;
pub const RUSTED_WRECK: IslandId = 2;
pub const SMOKING_RIG: IslandId = 3;
pub const LIGHTHOUSE_ATOLL: IslandId = 4;
/// Islets are ids 5..15.
pub const FIRST_ISLET: IslandId = 5;

pub fn is_islet(id: IslandId) -> bool {
    id >= FIRST_ISLET
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum IslandStyle {
    Hook,
    Palm,
    Rock,
    Wreck,
    Rig,
    Lighthouse,
}

pub struct IslandDef {
    pub name: &'static str,
    pub x: f32,
    pub z: f32,
    pub radius: f32,
    pub tier: u8,
    pub style: IslandStyle,
    /// A stranded survivor waits here once a Radio Station exists.
    pub survivor: bool,
    pub shrines: u8,
    /// Chest contents (empty = no chest). Each player may loot each chest once.
    pub chest: &'static [Goods],
    /// The chest stays locked until this quest is done.
    pub chest_quest: Option<QuestId>,
    /// Resident enemies, (kind, count), respawned ENEMY_RESPAWN_DAYS after dying.
    pub enemies: &'static [(EnemyKind, u8)],
}

/// Named interaction points on an island. Positions are horizontal (y = 0); the scene
/// places the matching models there and the host checks `dist_xz <= INTERACT_RANGE`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Spot {
    Chest,
    Survivor,
    Shrine(u8),
    Generator,
    Beacon,
    Barnaby,
}

impl IslandDef {
    pub fn center(&self) -> V3 {
        V3::xz(self.x, self.z)
    }

    pub fn spot(&self, spot: Spot) -> V3 {
        let r = self.radius;
        let (dx, dz) = match spot {
            Spot::Chest => (0.35 * r, 0.25 * r),
            Spot::Survivor => (-0.4 * r, 0.1 * r),
            Spot::Shrine(i) => {
                let a = i as f32 * TAU / 3.0;
                (0.7 * r * a.cos(), 0.7 * r * a.sin())
            }
            Spot::Generator => (0.0, -0.3 * r),
            Spot::Beacon => (0.0, 0.0),
            Spot::Barnaby => (2.0, 7.0),
        };
        V3::xz(self.x + dx, self.z + dz)
    }
}

use EnemyKind as E;
use Goods as G;
use ItemKind as I;

pub const ISLANDS: [IslandDef; 15] = [
    IslandDef {
        name: "Hook Isle",
        x: 0.0,
        z: -38.0,
        radius: 11.0,
        tier: 0,
        style: IslandStyle::Hook,
        survivor: false,
        shrines: 0,
        chest: &[],
        chest_quest: None,
        enemies: &[],
    },
    IslandDef {
        name: "Palm Reef",
        x: -420.0,
        z: -560.0,
        radius: 40.0,
        tier: 1,
        style: IslandStyle::Palm,
        survivor: true,
        shrines: 3,
        chest: &[G::Harpoon(3)],
        chest_quest: Some(QuestId::TideShrines),
        enemies: &[(E::ReefCrab, 4)],
    },
    IslandDef {
        name: "Rusted Wreck",
        x: 1150.0,
        z: -600.0,
        radius: 55.0,
        tier: 2,
        style: IslandStyle::Wreck,
        survivor: true,
        shrines: 0,
        chest: &[G::Melee(3), G::Sac(24)],
        chest_quest: Some(QuestId::Generator),
        enemies: &[(E::Crawler, 5)],
    },
    IslandDef {
        name: "Smoking Rig",
        x: 300.0,
        z: 1950.0,
        radius: 45.0,
        tier: 3,
        style: IslandStyle::Rig,
        survivor: true,
        shrines: 0,
        chest: &[G::Harpoon(4), G::Stock(s(0, 0, 0, 20))],
        chest_quest: Some(QuestId::RigBrute),
        enemies: &[(E::RigBrute, 1), (E::Crawler, 2)],
    },
    IslandDef {
        name: "Lighthouse Atoll",
        x: -2100.0,
        z: 1700.0,
        radius: 60.0,
        tier: 4,
        style: IslandStyle::Lighthouse,
        survivor: false,
        shrines: 0,
        chest: &[],
        chest_quest: None,
        enemies: &[],
    },
    IslandDef {
        name: "Gull Rock",
        x: 250.0,
        z: -180.0,
        radius: 9.0,
        tier: 1,
        style: IslandStyle::Rock,
        survivor: false,
        shrines: 0,
        chest: &[G::Coins(40), G::Item(I::Bait, 5)],
        chest_quest: None,
        enemies: &[(E::ReefCrab, 2)],
    },
    IslandDef {
        name: "Coconut Key",
        x: -380.0,
        z: 220.0,
        radius: 12.0,
        tier: 1,
        style: IslandStyle::Palm,
        survivor: false,
        shrines: 0,
        chest: &[G::Item(I::Bandage, 3), G::Item(I::Plank, 6)],
        chest_quest: None,
        enemies: &[(E::ReefCrab, 2)],
    },
    IslandDef {
        name: "Driftwood Cay",
        x: 620.0,
        z: 380.0,
        radius: 10.0,
        tier: 1,
        style: IslandStyle::Palm,
        survivor: true,
        shrines: 0,
        chest: &[G::Coins(60), G::Item(I::PalmBundle, 8)],
        chest_quest: None,
        enemies: &[(E::ReefCrab, 2)],
    },
    IslandDef {
        name: "Barnacle Rock",
        x: -900.0,
        z: -250.0,
        radius: 8.0,
        tier: 2,
        style: IslandStyle::Rock,
        survivor: false,
        shrines: 0,
        chest: &[G::Coins(90), G::Item(I::ScrapChunk, 6)],
        chest_quest: None,
        enemies: &[(E::Crawler, 2)],
    },
    IslandDef {
        name: "Sunken Bell",
        x: 800.0,
        z: -1150.0,
        radius: 11.0,
        tier: 2,
        style: IslandStyle::Rock,
        survivor: false,
        shrines: 0,
        chest: &[G::Item(I::Bait, 10), G::Item(I::Bandage, 4)],
        chest_quest: None,
        enemies: &[(E::Crawler, 3)],
    },
    IslandDef {
        name: "Mangrove Isle",
        x: -1200.0,
        z: 600.0,
        radius: 14.0,
        tier: 2,
        style: IslandStyle::Palm,
        survivor: true,
        shrines: 0,
        chest: &[G::Coins(120), G::Item(I::OilBarrel, 3)],
        chest_quest: None,
        enemies: &[(E::ReefCrab, 3)],
    },
    IslandDef {
        name: "Shellback",
        x: 1500.0,
        z: 700.0,
        radius: 10.0,
        tier: 2,
        style: IslandStyle::Rock,
        survivor: false,
        shrines: 0,
        chest: &[G::Item(I::BiomassSalve, 2), G::Item(I::ScrapChunk, 8)],
        chest_quest: None,
        enemies: &[(E::Crawler, 3)],
    },
    IslandDef {
        name: "Ember Key",
        x: -600.0,
        z: 1500.0,
        radius: 13.0,
        tier: 3,
        style: IslandStyle::Palm,
        survivor: false,
        shrines: 0,
        chest: &[G::Coins(200), G::Item(I::OilBarrel, 4)],
        chest_quest: None,
        enemies: &[(E::Crawler, 3)],
    },
    IslandDef {
        name: "Storm Spire",
        x: 1700.0,
        z: -1500.0,
        radius: 9.0,
        tier: 3,
        style: IslandStyle::Rock,
        survivor: false,
        shrines: 0,
        chest: &[G::Coins(250), G::Item(I::Bait, 10)],
        chest_quest: None,
        enemies: &[(E::Crawler, 4)],
    },
    IslandDef {
        name: "Last Hope Cay",
        x: -1800.0,
        z: 900.0,
        radius: 15.0,
        tier: 3,
        style: IslandStyle::Palm,
        survivor: true,
        shrines: 0,
        chest: &[G::Item(I::BiomassSalve, 3), G::Coins(150)],
        chest_quest: None,
        enemies: &[(E::Crawler, 3)],
    },
];

pub fn island(id: IslandId) -> &'static IslandDef {
    &ISLANDS[id as usize]
}

/// Island whose radius contains (x, z).
pub fn island_at(x: f32, z: f32) -> Option<IslandId> {
    island_near(x, z, 0.0)
}

/// Island whose radius + margin contains (x, z).
pub fn island_near(x: f32, z: f32, margin: f32) -> Option<IslandId> {
    ISLANDS
        .iter()
        .position(|i| V3::xz(x, z).dist_xz(i.center()) <= i.radius + margin)
        .map(|i| i as IslandId)
}

/// Players within this distance of an island's shore discover it (tuning).
pub const DISCOVER_MARGIN: f32 = 120.0;
/// Generator quest metal (Rusted Wreck), delivered as Scrap Chunks from the sac.
pub const GENERATOR_METAL: u32 = 20;
/// Metal one delivered Scrap Chunk counts for.
pub const METAL_PER_SCRAP: u32 = 2;
/// The Leviathan surfaces this far beyond Lighthouse Atoll's shore (tuning).
pub const LEVIATHAN_SPAWN_DIST: f32 = 20.0;

// ============================================================ quests (13)

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum QuestId {
    SellFish,
    CampfireLevel,
    BringGoldfin,
    TideShrines,
    Generator,
    RigBrute,
    Leviathan,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QuestGoal {
    /// Fish sold to Barnaby by anyone (world total since the quest became active).
    SellFish(u32),
    CampfireLevel(u8),
    /// Hand this item from the sac to the giver (consumed on claim).
    BringItem(ItemKind),
    /// Activate every tide shrine on the giver island.
    Shrines(u8),
    /// Deliver this much metal to the Rusted Wreck generator.
    GeneratorMetal(u32),
    DefeatBoss(EnemyKind),
    /// Install the Beacon Core at night and defeat the Leviathan.
    Ending,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QuestUnlock {
    Start,
    After(QuestId),
    /// Becomes active when a player discovers the island.
    Discover(IslandId),
}

pub struct QuestDef {
    pub id: QuestId,
    pub number: u8,
    pub giver: IslandId,
    pub title: &'static str,
    pub goal: QuestGoal,
    pub unlock: QuestUnlock,
    /// Paid by `ClaimQuest` to the claiming player (Barnaby quests), or to every
    /// connected player for the ending. Island quests 4-6 reward via their chest instead.
    pub reward: &'static [Goods],
}

/// Quest 3 pays this instead of Sac 12 when the player's sac is already 12 or bigger.
pub const QUEST3_OWNED_COINS: u32 = 80;

/// Indexed by `QuestId as usize`.
pub const QUESTS: [QuestDef; 7] = [
    QuestDef {
        id: QuestId::SellFish,
        number: 1,
        giver: HOOK_ISLE,
        title: "Sell 3 fish",
        goal: QuestGoal::SellFish(3),
        unlock: QuestUnlock::Start,
        reward: &[G::Coins(30)],
    },
    QuestDef {
        id: QuestId::CampfireLevel,
        number: 2,
        giver: HOOK_ISLE,
        title: "Reach campfire level 2",
        goal: QuestGoal::CampfireLevel(2),
        unlock: QuestUnlock::After(QuestId::SellFish),
        reward: &[G::Coins(40), G::Item(I::Bait, 5)],
    },
    QuestDef {
        id: QuestId::BringGoldfin,
        number: 3,
        giver: HOOK_ISLE,
        title: "Bring a Goldfin",
        goal: QuestGoal::BringItem(I::Fish(Fish::Goldfin)),
        unlock: QuestUnlock::After(QuestId::CampfireLevel),
        reward: &[G::Sac(12)],
    },
    QuestDef {
        id: QuestId::TideShrines,
        number: 4,
        giver: PALM_REEF,
        title: "Activate the 3 tide shrines",
        goal: QuestGoal::Shrines(3),
        unlock: QuestUnlock::Discover(PALM_REEF),
        reward: &[],
    },
    QuestDef {
        id: QuestId::Generator,
        number: 5,
        giver: RUSTED_WRECK,
        title: "Deliver 20 metal to the generator",
        goal: QuestGoal::GeneratorMetal(GENERATOR_METAL),
        unlock: QuestUnlock::Discover(RUSTED_WRECK),
        reward: &[],
    },
    QuestDef {
        id: QuestId::RigBrute,
        number: 6,
        giver: SMOKING_RIG,
        title: "Defeat the Rig Brute",
        goal: QuestGoal::DefeatBoss(E::RigBrute),
        unlock: QuestUnlock::Discover(SMOKING_RIG),
        reward: &[],
    },
    QuestDef {
        id: QuestId::Leviathan,
        number: 7,
        giver: LIGHTHOUSE_ATOLL,
        title: "Light the beacon and defeat the Leviathan",
        goal: QuestGoal::Ending,
        unlock: QuestUnlock::Discover(LIGHTHOUSE_ATOLL),
        reward: &[G::Coins(2000)],
    },
];

impl QuestId {
    pub const ALL: [QuestId; 7] = [
        QuestId::SellFish,
        QuestId::CampfireLevel,
        QuestId::BringGoldfin,
        QuestId::TideShrines,
        QuestId::Generator,
        QuestId::RigBrute,
        QuestId::Leviathan,
    ];

    pub fn def(self) -> &'static QuestDef {
        &QUESTS[self as usize]
    }
}

// ============================================================ saving (9)

/// Coins one save costs: 150 + 75 x campfire level.
pub fn save_cost(campfire_level: u8) -> u32 {
    150 + 75 * campfire_level as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tables_are_indexed_by_their_ids() {
        for (i, r) in RECIPES.iter().enumerate() {
            assert_eq!(r.id as usize, i, "{}", r.name);
            assert!((1..=5).contains(&r.level));
        }
        for (i, q) in QUESTS.iter().enumerate() {
            assert_eq!(q.id as usize, i);
            assert_eq!(q.number as usize, i + 1);
        }
        for (i, f) in Fish::ALL.iter().enumerate() {
            assert_eq!(*f as usize, i);
        }
        assert_eq!(ENEMIES[EnemyKind::Leviathan as usize].hp, 1200.0);
        assert_eq!(BoatKind::MotorLaunch.def().seats, 8);
    }

    #[test]
    fn spec_numbers() {
        assert_eq!(save_cost(1), 225);
        assert_eq!(save_cost(5), 525);
        assert_eq!(campfire_level(0), 1);
        assert_eq!(campfire_level(24), 1);
        assert_eq!(campfire_level(25), 2);
        assert_eq!(campfire_level(100), 5);
        assert_eq!(campfire_level(10_000), 5);
        assert_eq!(light_radius(5), 15.0);
        assert_eq!(farm_rate(0), 1.0);
        assert_eq!(farm_rate(1), 1.0);
        assert_eq!(farm_rate(3), 1.5);
        assert_eq!(gloomfin_per_player(1), 1);
        assert_eq!(gloomfin_per_player(9), 3);
        assert_eq!(crawler_max(4), 4);
        assert_eq!(harpoon(4).stun, 1.5);
        assert_eq!(harpoon(9).damage, 60.0);
        assert_eq!(RecipeId::Dinghy.def().cost, Stockpile::new(30, 8, 0, 0));
        assert_eq!(farm(PlaceableKind::OilFarm).unwrap().cycle, 150.0);
        assert!(!PlaceableKind::Campfire.is_farm());
    }

    #[test]
    fn stack_sizes() {
        assert_eq!(ItemKind::Plank.stack_size(), 10);
        assert_eq!(ItemKind::GloomfinCarcass.stack_size(), 10);
        assert_eq!(ItemKind::Fish(Fish::Sardine).stack_size(), 5);
        assert_eq!(ItemKind::Bandage.stack_size(), 10);
        assert_eq!(ItemKind::Bait.stack_size(), 20);
        assert_eq!(ItemKind::Flare.stack_size(), 1);
        assert_eq!(ItemKind::BeaconCore.stack_size(), 1);
        assert_eq!(ItemKind::Fish(Fish::Goldfin).hunger(), Some(20.0));
        assert_eq!(ItemKind::CookedFish(Fish::Goldfin).hunger(), Some(40.0));
        assert_eq!(ItemKind::all().len(), 24);
        assert!(ItemKind::all().iter().all(|k| !k.name().is_empty()));
    }

    #[test]
    fn fish_modifiers() {
        let base = fish_weights(1, false, false, false);
        assert_eq!(base, [60.0, 28.0, 9.0, 2.6, 0.4]);
        let reed = fish_weights(2, false, false, false);
        assert_eq!(reed[0], 60.0);
        assert!((reed[2] - 9.0 * 1.6).abs() < 1e-4);
        let night = fish_weights(1, false, true, false);
        assert_eq!(night[2], 9.0);
        assert!((night[4] - 0.8).abs() < 1e-6);
        let all = fish_weights(3, true, false, true);
        assert!((all[3] - 2.6 * 2.5 * 1.5 * 1.5).abs() < 1e-3);
    }

    #[test]
    fn shredder_outputs() {
        let mut rng = Rng::new(1);
        assert_eq!(
            shred(ItemKind::Plank, &mut rng).unwrap().stock,
            Stockpile::new(3, 0, 0, 0)
        );
        assert_eq!(
            shred(ItemKind::BruteCarcass, &mut rng)
                .unwrap()
                .stock
                .biomass,
            15
        );
        assert!(shred(ItemKind::Bandage, &mut rng).is_none());
        let (mut bait, mut coins) = (0, 0);
        for _ in 0..2000 {
            let o = shred(ItemKind::FloatingCrate, &mut rng).unwrap();
            assert!((2..=4).contains(&o.stock.wood) && (1..=2).contains(&o.stock.metal));
            assert!(o.coins == 0 || (10..=30).contains(&o.coins));
            bait += (o.bait > 0) as u32;
            coins += (o.coins > 0) as u32;
        }
        assert!((300..500).contains(&bait), "{bait}");
        assert!((120..290).contains(&coins), "{coins}");
    }

    #[test]
    fn islands_match_spec() {
        assert_eq!(ISLANDS.len(), 15);
        let islets = &ISLANDS[FIRST_ISLET as usize..];
        assert_eq!(islets.len(), 10);
        for i in islets {
            let d = i.center().length_xz();
            assert!((300.0..=2400.0).contains(&d), "{} at {d}", i.name);
            assert!((8.0..=15.0).contains(&i.radius) && (1..=3).contains(&i.tier));
            assert!(!i.chest.is_empty());
        }
        assert_eq!(islets.iter().filter(|i| i.survivor).count(), 3);
        assert!(is_islet(FIRST_ISLET) && !is_islet(LIGHTHOUSE_ATOLL));
        assert_eq!(ISLANDS.iter().filter(|i| i.survivor).count(), 6);
        for (a, ia) in ISLANDS.iter().enumerate() {
            for ib in &ISLANDS[a + 1..] {
                assert!(
                    ia.center().dist_xz(ib.center()) > ia.radius + ib.radius + 100.0,
                    "{} / {}",
                    ia.name,
                    ib.name
                );
            }
        }
        assert_eq!(island_at(0.0, -38.0), Some(HOOK_ISLE));
        assert_eq!(island_at(0.0, 0.0), None);
        assert_eq!(island(PALM_REEF).shrines, 3);
        assert!(
            island(HOOK_ISLE)
                .spot(Spot::Barnaby)
                .dist_xz(island(HOOK_ISLE).center())
                < 11.0
        );
    }

    #[test]
    fn goods_labels() {
        assert_eq!(Goods::Item(ItemKind::Bait, 5).label(), "Bait x5");
        assert_eq!(Goods::Melee(3).label(), "Coral Cutlass");
        assert_eq!(Goods::Stock(Stockpile::new(0, 0, 0, 20)).label(), "20 oil");
    }
}
