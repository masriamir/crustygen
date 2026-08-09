//! Compiles the room-graph IR into UDMF map data.

pub mod portals;
pub mod sectors;
pub mod tags;

use crate::geom::Pt;

/// A sector as it will be emitted.
#[derive(Debug, Clone)]
pub struct SectorOut {
    /// Floor height in map units.
    pub floor: i32,
    /// Ceiling height in map units.
    pub ceiling: i32,
    /// Light level.
    pub light: i32,
    /// Floor flat name.
    pub floor_tex: String,
    /// Ceiling flat name.
    pub ceil_tex: String,
    /// Sector special; 0 for none.
    pub special: u16,
    /// Allocated sector tag; 0 only where no action references it.
    pub tag: u16,
}

/// A sidedef as it will be emitted.
#[derive(Debug, Clone)]
pub struct SidedefOut {
    /// Index into [`MapData::sectors`].
    pub sector: usize,
    /// Upper texture; empty for none.
    pub upper: String,
    /// Middle texture; empty for none.
    pub middle: String,
    /// Lower texture; empty for none.
    pub lower: String,
}

/// A linedef as it will be emitted.
#[derive(Debug, Clone)]
pub struct LinedefOut {
    /// Start vertex index.
    pub v1: usize,
    /// End vertex index.
    pub v2: usize,
    /// Front (right) sidedef index; always present.
    pub front: usize,
    /// Back (left) sidedef index, for two-sided lines.
    pub back: Option<usize>,
    /// Whether the line blocks movement.
    pub blocking: bool,
    /// Line special; 0 for none.
    pub special: u16,
    /// Sector tag the special acts on.
    pub tag: u16,
    /// Lower-unpegged flag.
    pub lower_unpegged: bool,
    /// Upper-unpegged flag.
    pub upper_unpegged: bool,
}

/// The full set of emitted map records.
#[derive(Debug, Default, Clone)]
pub struct MapData {
    /// Deduplicated vertices.
    pub vertices: Vec<Pt>,
    /// Sectors, one per room plus one per door.
    pub sectors: Vec<SectorOut>,
    /// Sidedefs.
    pub sidedefs: Vec<SidedefOut>,
    /// Linedefs.
    pub linedefs: Vec<LinedefOut>,
}

/// Errors raised while compiling geometry.
#[derive(Debug, thiserror::Error)]
pub enum CompileError {
    /// A footprint winds counter-clockwise, so its front sidedefs face outward.
    #[error("room `{room}` footprint is counter-clockwise; Doom requires clockwise")]
    NotClockwise {
        /// The offending room.
        room: String,
    },
    /// A footprint has fewer than three points or encloses no area.
    #[error("room `{room}` footprint is degenerate")]
    Degenerate {
        /// The offending room.
        room: String,
    },
    /// An edge is neither axis-aligned nor at 45 degrees.
    #[error(
        "room `{room}` has an edge from ({x1}, {y1}) to ({x2}, {y2}) that is neither axis-aligned nor diagonal"
    )]
    BadEdge {
        /// The offending room.
        room: String,
        /// Start X.
        x1: i32,
        /// Start Y.
        y1: i32,
        /// End X.
        x2: i32,
        /// End Y.
        y2: i32,
    },
    /// Two footprints overlap.
    #[error("rooms `{a}` and `{b}` overlap")]
    Overlap {
        /// The first room.
        a: String,
        /// The second room.
        b: String,
    },
    /// A portal names two rooms that share no wall.
    #[error("portal `{a}` <-> `{b}`: the rooms share no wall")]
    NotAdjacent {
        /// The first room.
        a: String,
        /// The second room.
        b: String,
    },
    /// A portal opening is wider than the wall it sits in.
    #[error("portal `{a}` <-> `{b}`: opening of {width} exceeds the {available} shared wall")]
    PortalTooWide {
        /// The first room.
        a: String,
        /// The second room.
        b: String,
        /// The requested opening width.
        width: i32,
        /// The available shared-wall length.
        available: i32,
    },
    /// A portal midpoint does not lie on the shared wall.
    #[error("portal `{a}` <-> `{b}`: midpoint ({x}, {y}) is not on the shared wall")]
    PortalOffWall {
        /// The first room.
        a: String,
        /// The second room.
        b: String,
        /// Midpoint X.
        x: i32,
        /// Midpoint Y.
        y: i32,
    },
    /// A linedef carries a special but no tag.
    #[error("linedef {index} has special {special} at tag 0, which matches every untagged sector")]
    ActionAtTagZero {
        /// Index of the offending linedef.
        index: usize,
        /// The special it carries.
        special: u16,
    },
}
