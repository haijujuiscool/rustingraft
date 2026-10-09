//! Raft grid geometry (spec 4, 10). Cell (i, j) covers x in [2i, 2i+2), z in [2j, 2j+2);
//! its centre is ((i + 0.5) * 2, DECK_Y, (j + 0.5) * 2). The start raft is cells -2..2 on
//! both axes, so it is centred on the origin. Placement rules that involve costs and
//! unlocks live in economy.rs; this file only answers geometric questions.

use crate::content::{PlaceableKind, DECK_Y, MAX_RAFT_TILES, START_TILES, TILE};
use crate::math::V3;
use crate::world::{EntityId, World};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub type Cell = (i32, i32);

/// No raft cell is ever this far from the origin. Request cells come from the network,
/// so anything beyond is refused before the neighbour/footprint math can overflow.
const MAX_CELL: i32 = 1024;

fn in_reach(cell: Cell) -> bool {
    (-MAX_CELL..=MAX_CELL).contains(&cell.0) && (-MAX_CELL..=MAX_CELL).contains(&cell.1)
}

/// Tile edge. North = -Z (toward Hook Isle), East = +X (downstream).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Side {
    North,
    East,
    South,
    West,
}

impl Side {
    pub const ALL: [Side; 4] = [Side::North, Side::East, Side::South, Side::West];

    pub fn offset(self) -> Cell {
        match self {
            Side::North => (0, -1),
            Side::East => (1, 0),
            Side::South => (0, 1),
            Side::West => (-1, 0),
        }
    }
}

pub fn neighbor(cell: Cell, side: Side) -> Cell {
    let (di, dj) = side.offset();
    (cell.0 + di, cell.1 + dj)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Railing {
    pub cell: Cell,
    pub side: Side,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Structure {
    pub kind: PlaceableKind,
    /// Min-corner cell of the footprint.
    pub cell: Cell,
    /// Quarter turns 0..4.
    pub rot: u8,
    pub state: StructureState,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum StructureState {
    Plain,
    Farm(FarmState),
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct FarmState {
    /// A cycle is in progress.
    pub running: bool,
    /// Seconds of the current cycle done (at rate 1).
    pub progress: f32,
    /// Output waiting to be collected (no survivors yet).
    pub ready: bool,
}

impl Structure {
    pub fn new(kind: PlaceableKind, cell: Cell, rot: u8) -> Structure {
        let state = if kind.is_farm() {
            StructureState::Farm(FarmState::default())
        } else {
            StructureState::Plain
        };
        Structure {
            kind,
            cell,
            rot: rot % 4,
            state,
        }
    }

    pub fn cells(&self) -> Vec<Cell> {
        footprint(self.kind, self.cell, self.rot)
    }

    /// World position of the footprint centre on the deck.
    pub fn center(&self) -> V3 {
        let cells = self.cells();
        let sum = cells.iter().fold(V3::ZERO, |a, c| a + cell_center(*c));
        sum * (1.0 / cells.len() as f32)
    }

    pub fn farm(&self) -> Option<&FarmState> {
        match &self.state {
            StructureState::Farm(f) => Some(f),
            StructureState::Plain => None,
        }
    }

    pub fn farm_mut(&mut self) -> Option<&mut FarmState> {
        match &mut self.state {
            StructureState::Farm(f) => Some(f),
            StructureState::Plain => None,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Raft {
    pub tiles: BTreeSet<Cell>,
    pub structures: BTreeMap<EntityId, Structure>,
    pub railings: BTreeSet<Railing>,
}

pub fn cell_center(cell: Cell) -> V3 {
    V3::new(
        (cell.0 as f32 + 0.5) * TILE,
        DECK_Y,
        (cell.1 as f32 + 0.5) * TILE,
    )
}

pub fn cell_at(x: f32, z: f32) -> Cell {
    ((x / TILE).floor() as i32, (z / TILE).floor() as i32)
}

/// Cells covered by `kind` with its min corner at `cell`, rotated `rot` quarter turns.
pub fn footprint(kind: PlaceableKind, cell: Cell, rot: u8) -> Vec<Cell> {
    let (w, d) = kind.footprint();
    let (w, d) = if rot % 2 == 1 { (d, w) } else { (w, d) };
    let mut v = Vec::with_capacity((w * d) as usize);
    for i in 0..w {
        for j in 0..d {
            v.push((cell.0 + i, cell.1 + j));
        }
    }
    v
}

impl Raft {
    pub fn has_tile(&self, cell: Cell) -> bool {
        self.tiles.contains(&cell)
    }

    /// True if (x, z) is above a raft tile.
    pub fn covers_xz(&self, x: f32, z: f32) -> bool {
        self.has_tile(cell_at(x, z))
    }

    /// True if `pos` stands on (or just above) the deck.
    pub fn on_deck(&self, pos: V3) -> bool {
        self.covers_xz(pos.x, pos.z) && pos.y > DECK_Y - 0.6 && pos.y < DECK_Y + 2.5
    }

    /// (min cell, max cell) of the tiles, inclusive.
    pub fn bounds(&self) -> Option<(Cell, Cell)> {
        let first = *self.tiles.iter().next()?;
        Some(self.tiles.iter().fold((first, first), |(lo, hi), c| {
            (
                (lo.0.min(c.0), lo.1.min(c.1)),
                (hi.0.max(c.0), hi.1.max(c.1)),
            )
        }))
    }

    /// Centre of the tiles' bounding box on the deck.
    pub fn center(&self) -> V3 {
        match self.bounds() {
            Some((lo, hi)) => (cell_center(lo) + cell_center(hi)) * 0.5,
            None => V3::new(0.0, DECK_Y, 0.0),
        }
    }

    /// A floor tile may go on a free cell next to an existing tile while the raft stays
    /// within MAX_RAFT_TILES per side.
    pub fn can_add_tile(&self, cell: Cell) -> Result<(), &'static str> {
        if !in_reach(cell) {
            return Err("A floor must be next to the raft");
        }
        if self.has_tile(cell) {
            return Err("There is already a tile here");
        }
        if !Side::ALL.iter().any(|s| self.has_tile(neighbor(cell, *s))) {
            return Err("A floor must be next to the raft");
        }
        if let Some((lo, hi)) = self.bounds() {
            let w = hi.0.max(cell.0) - lo.0.min(cell.0) + 1;
            let d = hi.1.max(cell.1) - lo.1.min(cell.1) + 1;
            if w > MAX_RAFT_TILES || d > MAX_RAFT_TILES {
                return Err("The raft cannot grow any larger");
            }
        }
        Ok(())
    }

    /// Structure standing on `cell`.
    pub fn occupant(&self, cell: Cell) -> Option<EntityId> {
        self.structures
            .iter()
            .find(|(_, s)| s.cells().contains(&cell))
            .map(|(id, _)| *id)
    }

    /// Every footprint cell must be a tile with no structure on it.
    pub fn can_place(&self, kind: PlaceableKind, cell: Cell, rot: u8) -> Result<(), &'static str> {
        if !in_reach(cell) {
            return Err("It must stand on the raft");
        }
        for c in footprint(kind, cell, rot) {
            if !self.has_tile(c) {
                return Err("It must stand on the raft");
            }
            if self.occupant(c).is_some() {
                return Err("Something is already there");
            }
        }
        Ok(())
    }

    /// First structure of `kind` (structures are kept in id order).
    pub fn find(&self, kind: PlaceableKind) -> Option<(EntityId, &Structure)> {
        self.structures
            .iter()
            .find(|(_, s)| s.kind == kind)
            .map(|(id, s)| (*id, s))
    }

    pub fn count(&self, kind: PlaceableKind) -> usize {
        self.structures.values().filter(|s| s.kind == kind).count()
    }

    /// An edge of a tile with no tile beyond it.
    pub fn is_outer_edge(&self, cell: Cell, side: Side) -> bool {
        self.has_tile(cell) && !self.has_tile(neighbor(cell, side))
    }

    pub fn can_add_railing(&self, cell: Cell, side: Side) -> Result<(), &'static str> {
        if !self.is_outer_edge(cell, side) {
            return Err("Railings go on the raft's outer edge");
        }
        if self.railings.contains(&Railing { cell, side }) {
            return Err("There is already a railing here");
        }
        Ok(())
    }

    /// Every outer edge (where crawlers climb aboard and railings fit).
    pub fn outer_edges(&self) -> Vec<(Cell, Side)> {
        self.tiles
            .iter()
            .flat_map(|c| Side::ALL.iter().map(move |s| (*c, *s)))
            .filter(|(c, s)| !self.has_tile(neighbor(*c, *s)))
            .collect()
    }

    /// World position on the outer edge of a tile side (deck height).
    pub fn edge_pos(cell: Cell, side: Side) -> V3 {
        let (di, dj) = side.offset();
        cell_center(cell) + V3::xz(di as f32, dj as f32) * (TILE * 0.5)
    }

    /// Where crafted boats spawn: 3 m off the middle of the south (+Z) edge, on the water.
    pub fn dock_pos(&self) -> V3 {
        match self.bounds() {
            Some((lo, hi)) => V3::xz(
                (lo.0 + hi.0 + 1) as f32 * TILE * 0.5,
                (hi.1 + 1) as f32 * TILE + 3.0,
            ),
            None => V3::xz(0.0, 3.0),
        }
    }

    /// Where the wandering trader moors: 4 m off the middle of the east (+X) edge.
    pub fn trader_dock_pos(&self) -> V3 {
        match self.bounds() {
            Some((lo, hi)) => V3::xz(
                (hi.0 + 1) as f32 * TILE + 4.0,
                (lo.1 + hi.1 + 1) as f32 * TILE * 0.5,
            ),
            None => V3::xz(4.0, 0.0),
        }
    }
}

/// Starting layout: 4x4 tiles centred on the origin with campfire, workbench, shredder
/// and logbook.
pub const START_STRUCTURES: [(PlaceableKind, Cell); 4] = [
    (PlaceableKind::Campfire, (0, 0)),
    (PlaceableKind::Workbench, (-2, -2)),
    (PlaceableKind::Shredder, (1, -2)),
    (PlaceableKind::Logbook, (-2, 1)),
];

/// Free start cells players spawn on (by player id, wrapping).
pub const SPAWN_CELLS: [Cell; 8] = [
    (-1, 0),
    (0, -1),
    (-1, -1),
    (1, 0),
    (0, 1),
    (-1, 1),
    (1, 1),
    (-2, 0),
];

pub(crate) fn setup_start(w: &mut World) {
    let half = START_TILES / 2;
    for i in -half..half {
        for j in -half..half {
            w.raft.tiles.insert((i, j));
        }
    }
    for (kind, cell) in START_STRUCTURES {
        let id = w.alloc_id();
        w.raft.structures.insert(id, Structure::new(kind, cell, 0));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn start_layout() {
        let w = World::new(1);
        assert_eq!(w.raft.tiles.len(), 16);
        assert_eq!(w.raft.bounds(), Some(((-2, -2), (1, 1))));
        assert_eq!(w.raft.center(), V3::new(0.0, DECK_Y, 0.0));
        assert_eq!(w.raft.structures.len(), 4);
        assert!(w.raft.find(PlaceableKind::Campfire).is_some());
        for c in SPAWN_CELLS {
            assert!(w.raft.has_tile(c) && w.raft.occupant(c).is_none(), "{c:?}");
        }
        assert_eq!(w.raft.outer_edges().len(), 16);
    }

    #[test]
    fn cell_math() {
        assert_eq!(cell_center((0, 0)), V3::new(1.0, DECK_Y, 1.0));
        assert_eq!(cell_center((-2, -2)), V3::new(-3.0, DECK_Y, -3.0));
        assert_eq!(cell_at(-0.1, 0.1), (-1, 0));
        assert_eq!(cell_at(3.9, -4.0), (1, -2));
        assert_eq!(
            footprint(PlaceableKind::WoodFarm, (0, 0), 0),
            vec![(0, 0), (1, 0)]
        );
        assert_eq!(
            footprint(PlaceableKind::WoodFarm, (0, 0), 1),
            vec![(0, 0), (0, 1)]
        );
        let w = World::new(1);
        assert!(w.raft.on_deck(V3::new(0.5, DECK_Y, 0.5)));
        assert!(!w.raft.on_deck(V3::new(0.5, -1.0, 0.5)));
        assert!(!w.raft.covers_xz(4.1, 0.0));
    }

    #[test]
    fn tile_and_structure_placement() {
        let mut w = World::new(1);
        let r = &mut w.raft;
        assert!(r.can_add_tile((2, 0)).is_ok());
        assert!(r.can_add_tile((3, 0)).is_err(), "not adjacent");
        assert!(r.can_add_tile((0, 0)).is_err(), "taken");
        for i in 2..14 {
            r.tiles.insert((i, 0));
        }
        assert!(r.can_add_tile((14, 0)).is_err(), "17 wide");
        assert!(r.can_place(PlaceableKind::WoodFarm, (2, 0), 0).is_ok());
        assert!(
            r.can_place(PlaceableKind::WoodFarm, (2, 0), 1).is_err(),
            "(2,1) is water"
        );
        assert!(
            r.can_place(PlaceableKind::RadioStation, (0, 0), 0).is_err(),
            "campfire there"
        );
        assert!(r.can_add_railing((1, 1), Side::South).is_ok());
        assert!(r.can_add_railing((0, 0), Side::South).is_err());
        r.railings.insert(Railing {
            cell: (1, 1),
            side: Side::South,
        });
        assert!(r.can_add_railing((1, 1), Side::South).is_err());
    }

    #[test]
    fn far_cells_are_refused_without_overflow() {
        let w = World::new(1);
        for c in [(i32::MAX, i32::MIN), (i32::MIN, 0), (0, i32::MAX)] {
            assert!(w.raft.can_add_tile(c).is_err());
            assert!(w.raft.can_place(PlaceableKind::WoodFarm, c, 1).is_err());
            assert!(w.raft.can_add_railing(c, Side::West).is_err());
        }
    }

    #[test]
    fn docks_are_off_the_raft() {
        let w = World::new(1);
        assert_eq!(w.raft.dock_pos(), V3::xz(0.0, 7.0));
        assert_eq!(w.raft.trader_dock_pos(), V3::xz(8.0, 0.0));
        assert_eq!(
            Raft::edge_pos((1, 1), Side::East),
            V3::new(4.0, DECK_Y, 3.0)
        );
    }
}
