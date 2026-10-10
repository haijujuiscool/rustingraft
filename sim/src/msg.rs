//! Requests (player actions, client -> host) and events (host -> view / clients).
//!
//! Append-only: system agents add variants inside their own group (above the
//! `append <system>` marker) and add the routing arm in `World::apply`.

use crate::clock::Phase;
use crate::content::{EnemyKind, Fish, Goods, IslandId, QuestId, RecipeId, Shop};
use crate::math::V3;
use crate::raft::{Cell, Side};
use crate::world::{EntityId, PlayerId};
use serde::{Deserialize, Serialize};

/// Hotbar hands that hold a stack (keys 4 and 5).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Hand {
    Food,
    Heal,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Request {
    // ---- players.rs
    /// Client-authoritative movement, 20 Hz. Ignored while downed or seated.
    PlayerTransform {
        pos: V3,
        yaw: f32,
        in_water: bool,
        sprinting: bool,
    },
    /// Eat one item from the food hand.
    Eat,
    /// Use one item from the heal hand.
    Heal,
    /// Move a sac slot into a hand (merge or swap).
    Equip {
        slot: usize,
        hand: Hand,
    },
    /// Move a hand's stack back into the sac.
    Unequip(Hand),
    MoveItem {
        from: usize,
        to: usize,
    },
    /// Debris kinds float away as debris; other items are destroyed.
    DropItem {
        slot: usize,
    },
    UseFlare {
        slot: usize,
    },
    /// Cook the raw fish stack in a sac slot (None = the food hand) at the lit campfire.
    Cook {
        slot: Option<usize>,
    },
    /// Start holding E on a downed player (progress ticks in `players::step`).
    StartRevive(PlayerId),
    StopRevive,
    /// F looking at nothing: one item of the kind that entered the sac last
    /// (`Sac::last_out`, never Quest items) drops DROP_AHEAD in front of the player.
    DropLast,
    // append players

    // ---- debris.rs
    /// E on floating debris within DEBRIS_PICKUP_RANGE.
    PickUpDebris(EntityId),
    /// Harpoon hit floating debris: drag it to the player.
    HarpoonDebris(EntityId),
    // append debris

    // ---- enemies.rs
    /// Harpoon hit an enemy: damage it and pull it toward the player.
    HarpoonEnemy(EntityId),
    MeleeHit(EntityId),
    // append enemies

    // ---- fishing.rs
    /// Cast the bobber at a water point within CAST_RANGE.
    CastLine {
        at: V3,
    },
    /// Result of the client's timing minigame after `Event::FishBite`.
    ReelIn {
        success: bool,
    },
    CancelFishing,
    // append fishing

    // ---- economy.rs
    DepositShredder,
    FeedCampfire {
        oil: bool,
    },
    /// Non-placed recipes (items, gear, boats).
    Craft(RecipeId),
    /// Placed recipes: RaftFloor (rot ignored) and structures.
    Place {
        recipe: RecipeId,
        cell: Cell,
        rot: u8,
    },
    PlaceRailing {
        cell: Cell,
        side: Side,
    },
    StartFarm(EntityId),
    CollectFarm(EntityId),
    // append economy

    // ---- save.rs
    /// Pay `save_cost` at the logbook; the host writes the file on `Event::SaveRequested`.
    SaveWorld,
    // append save

    // ---- trader.rs (Barnaby's shop and the wandering trader)
    /// Sell the fish stack in a sac slot to Barnaby.
    Sell {
        slot: usize,
    },
    SellAllFish,
    Buy {
        shop: Shop,
        offer: usize,
    },
    // append trader

    // ---- islands.rs (chests, shrines, generator, beacon, quests)
    ClaimQuest(QuestId),
    OpenChest(IslandId),
    ActivateShrine {
        island: IslandId,
        shrine: u8,
    },
    DeliverGeneratorMetal,
    InstallBeacon,
    // append islands

    // ---- survivors.rs
    TalkToSurvivor(IslandId),
    // append survivors

    // ---- boats.rs
    BoardBoat(EntityId),
    LeaveBoat,
    /// Driver-authoritative boat transform while driving.
    BoatTransform {
        pos: V3,
        yaw: f32,
        speed: f32,
    },
    // append boats
}

/// Something that can take damage.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Target {
    Player(PlayerId),
    Enemy(EntityId),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SoundKind {
    HarpoonHit,
    Splash,
    Pickup,
    Shred,
    CampfireFeed,
    CampfireLevelUp,
    CampfireOut,
    Craft,
    Place,
    Coins,
    Eat,
    Heal,
    Hurt,
    Downed,
    Revive,
    FishBite,
    FishCatch,
    FishEscape,
    EnemyHit,
    EnemyDie,
    Bite,
    ChestOpen,
    Shrine,
    Generator,
    TraderHorn,
    Radio,
    Save,
    Flare,
    BossRoar,
    Denied,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Event {
    /// A request was rejected and changed nothing.
    Denied {
        player: PlayerId,
        reason: String,
    },
    /// HUD message; None = everyone.
    Toast {
        player: Option<PlayerId>,
        text: String,
    },
    Sound {
        kind: SoundKind,
        pos: V3,
    },
    /// A player received goods (items, coins, upgrades, stockpile resources).
    Gained {
        player: PlayerId,
        goods: Goods,
    },
    Damage {
        target: Target,
        amount: f32,
        pos: V3,
    },
    PlayerDowned(PlayerId),
    PlayerRevived(PlayerId),
    /// Every connected player is downed. The host reloads the last save or quits.
    GameOver,
    /// Host-forced move; the owning client must move its controller there.
    PlayerTeleport {
        player: PlayerId,
        pos: V3,
    },
    /// The host must write `World::to_json` now (coins are already paid).
    SaveRequested {
        by: PlayerId,
    },
    PhaseChanged(Phase),
    /// Dawn of this day number.
    DayStarted(u32),
    NightStarted,
    CampfireLevelUp(u8),
    CampfireOut,
    /// The host rolled this fish; the client runs the minigame with its zone/speed.
    FishBite {
        player: PlayerId,
        fish: Fish,
    },
    EnemySpawned {
        id: EntityId,
        kind: EnemyKind,
        pos: V3,
    },
    EnemyKilled {
        id: EntityId,
        kind: EnemyKind,
        pos: V3,
        by: Option<PlayerId>,
    },
    TraderArrived,
    TraderLeft,
    IslandDiscovered(IslandId),
    QuestUpdated(QuestId),
    /// A survivor's radio signal appeared (Radio Station built).
    SurvivorSignal(IslandId),
    SurvivorRescued(IslandId),
    EndingReached,
    // append events
}

pub(crate) fn deny(ev: &mut Vec<Event>, player: PlayerId, reason: impl Into<String>) {
    ev.push(Event::Denied {
        player,
        reason: reason.into(),
    });
}

pub(crate) fn toast(ev: &mut Vec<Event>, player: Option<PlayerId>, text: impl Into<String>) {
    ev.push(Event::Toast {
        player,
        text: text.into(),
    });
}

pub(crate) fn sound(ev: &mut Vec<Event>, kind: SoundKind, pos: V3) {
    ev.push(Event::Sound { kind, pos });
}
