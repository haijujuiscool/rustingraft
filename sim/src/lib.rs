//! Rusting Raft game rules. Pure Rust, no engine types.
//!
//! The host owns the authoritative `World` and drives it with `World::step` (60 Hz) and
//! `World::apply` (player requests); clients mirror it through `replicate`. See
//! `CONTRACT.md` for module ownership and conventions.

pub mod boats;
pub mod clock;
pub mod content;
pub mod debris;
pub mod economy;
pub mod enemies;
pub mod fishing;
pub mod inventory;
pub mod islands;
pub mod math;
pub mod msg;
pub mod players;
pub mod raft;
pub mod replicate;
pub mod rng;
pub mod save;
pub mod survivors;
pub mod trader;
pub mod world;

pub use clock::{Clock, Phase};
pub use inventory::{Sac, Stack};
pub use math::V3;
pub use msg::{Event, Hand, Request, SoundKind, Target};
pub use players::Player;
pub use replicate::{DynamicSnapshot, SectionId};
pub use rng::Rng;
pub use world::{EntityId, FlatGround, Ground, IslandGround, PlayerId, World};
