//! Compiles the room-graph IR into UDMF map data.

pub mod doors;
pub mod portals;
pub mod sectors;
pub mod tags;
pub mod textmap;
pub mod things;

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
    ///
    /// May contain entries no longer referenced by any linedef's
    /// `front`/`back`. `portals::split_wall_for_opening` hands the split
    /// wall's own sidedef to the first surviving piece, so the usual split
    /// orphans nothing; but an opening that consumes a wall end to end
    /// leaves no piece to inherit it, and the record stays rather than every
    /// surviving index being renumbered. See [`textmap::emit_textmap`]'s doc
    /// comment for the full rationale.
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
    /// Two portals' openings overlap on the same wall line, so cutting the
    /// second would find no intact wall left where the first already opened.
    #[error("portals `{first}` and `{second}` have overlapping openings in the same wall")]
    OverlappingPortals {
        /// The first portal, as `a <-> b`.
        first: String,
        /// The second portal, as `a <-> b`.
        second: String,
    },
    /// No single solid wall of a room spans a portal's opening.
    #[error("portal opening at ({x}, {y}) does not lie within one solid wall of room `{room}`")]
    OpeningNotInAWall {
        /// The room whose wall was searched.
        room: String,
        /// Midpoint X of the opening.
        x: i32,
        /// Midpoint Y of the opening.
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
    /// A door portal's recess would be at least as deep as room `b`, which
    /// would punch through or invert its far wall instead of stopping short
    /// of it.
    #[error(
        "door `{a}` <-> `{b}` needs {needed} units of depth in room `{b}` but only {available} are available"
    )]
    DoorTooDeep {
        /// The first room.
        a: String,
        /// The second room, whose wall is recessed.
        b: String,
        /// The depth the recess needs.
        needed: i32,
        /// The depth actually available in room `b` along that axis.
        available: i32,
    },
    /// Two door portals recess into the same room and their carved
    /// rectangles overlap, which would produce self-intersecting geometry.
    #[error(
        "door portals `{first_a}` <-> `{room}` and `{second_a}` <-> `{room}` both recess into room `{room}` and their carved areas overlap"
    )]
    OverlappingDoorRecesses {
        /// The shared room both portals recess into.
        room: String,
        /// The far side of the first portal.
        first_a: String,
        /// The far side of the second portal.
        second_a: String,
    },
    /// A thing lies outside the polygon of the room that declares it.
    #[error("thing `{kind}` at ({x}, {y}) is outside room `{room}`")]
    ThingOutsideRoom {
        /// The room that declared it.
        room: String,
        /// The vocabulary name.
        kind: String,
        /// X coordinate.
        x: i32,
        /// Y coordinate.
        y: i32,
    },
    /// A thing sits closer to a wall than its own radius.
    #[error(
        "thing `{kind}` at ({x}, {y}) in room `{room}` has {have:.1} units of clearance but needs {need}"
    )]
    ThingTooClose {
        /// The room.
        room: String,
        /// The vocabulary name.
        kind: String,
        /// X coordinate.
        x: i32,
        /// Y coordinate.
        y: i32,
        /// Available clearance.
        have: f64,
        /// Required radius.
        need: i32,
    },
    /// A room is shorter than something that must stand in it.
    #[error("room `{room}` has {have} units of headroom but `{kind}` needs {need}")]
    NoHeadroom {
        /// The room.
        room: String,
        /// The vocabulary name.
        kind: String,
        /// Available floor-to-ceiling gap.
        have: i32,
        /// Required height.
        need: i32,
    },
    /// A thing name is not in the vocabulary table.
    #[error("unknown thing `{kind}` in room `{room}`")]
    UnknownThing {
        /// The room.
        room: String,
        /// The unresolvable name.
        kind: String,
    },
    /// The IR's theme resolves to no texture set in the vocabulary table.
    #[error("unknown theme `{theme}`")]
    UnknownTheme {
        /// The unresolvable theme name.
        theme: String,
    },
    /// A locked portal names a key the vocabulary has no door special for.
    #[error("portal `{a}` <-> `{b}` is locked by `{lock}`, which opens no known door type")]
    UnknownLock {
        /// The first room.
        a: String,
        /// The second room.
        b: String,
        /// The unresolvable key name.
        lock: String,
    },
    /// A room's sector has no emitted linedef bordering it, so nothing can
    /// be measured against it.
    #[error("room `{room}` has no emitted geometry to measure clearance against")]
    UnboundedRoom {
        /// The room.
        room: String,
    },
    /// Two player starts occupy the same spot.
    #[error("two player starts overlap at ({x}, {y}); they would telefrag on spawn")]
    OverlappingStarts {
        /// X coordinate.
        x: i32,
        /// Y coordinate.
        y: i32,
    },
}

use crate::compile::tags::TagAllocator;
use crate::compile::things::ThingOut;
use crate::ir::Ir;
use crate::tables::Tables;

/// Everything one compilation produced.
#[derive(Debug)]
pub struct Compiled {
    /// The emitted UDMF text.
    pub textmap: String,
    /// The map records behind it.
    pub data: MapData,
    /// The placed things.
    pub things: Vec<ThingOut>,
    /// The tag manifest.
    pub tags: TagAllocator,
}

/// Compiles a room graph into UDMF `TEXTMAP` text.
///
/// Passes run in a fixed order, each depending on the last:
///
/// 1. [`sectors::emit_sectors`] turns every room footprint into a closed,
///    one-sided sector — a room is watertight by construction before any
///    portal touches it.
/// 2. [`portals::cut_portals`] opens every portal's shared wall. For a
///    [`crate::ir::PortalKind::Plain`] portal this also emits the two-sided
///    opening line; for a door portal it leaves the flanking walls cut but
///    the opening itself unemitted, because that opening is a carved sector
///    rather than a single line.
/// 3. [`doors::emit_doors`] carves that dedicated sector for every door
///    portal out of room `b`, allocating its tag from a fresh
///    [`TagAllocator`] shared with every other tag-consuming pass.
/// 4. [`things::place_things`] places every thing, measuring clearance and
///    headroom against the geometry emitted by steps 1–3 — not the IR's
///    declared footprints, which a door recess can make stale — so it must
///    run after doors are carved, not before.
/// 5. [`tags::check_no_action_at_tag_zero`] rejects any linedef special left
///    at tag 0, which would match every untagged sector in-engine.
/// 6. [`textmap::emit_textmap`] renders the final, validated geometry.
///
/// # Errors
/// Returns the first [`CompileError`] raised by any pass; nothing is emitted
/// unless every pass succeeds.
pub fn compile(ir: &Ir, tables: &Tables) -> Result<Compiled, CompileError> {
    let mut data = sectors::emit_sectors(ir)?;
    portals::cut_portals(ir, &mut data)?;
    let mut tags = TagAllocator::new();
    doors::emit_doors(ir, tables, &mut data, &mut tags)?;
    let things = things::place_things(ir, tables, &data)?;
    tags::check_no_action_at_tag_zero(&data)?;
    let textmap = textmap::emit_textmap(&data, &things);
    Ok(Compiled {
        textmap,
        data,
        things,
        tags,
    })
}
