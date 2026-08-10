//! Compiles the room-graph IR into UDMF map data.

pub mod doors;
pub mod exits;
pub mod heights;
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
    /// The wall texture this sector's faces use.
    ///
    /// **Not emitted to `TEXTMAP`** — a Doom sector carries no wall texture;
    /// walls are a sidedef property. It is recorded here so
    /// [`heights::apply_height_textures`] can source the riser a floor or
    /// ceiling difference exposes, including for sectors the compiler creates
    /// itself (a portal's passage, a door and its alcoves, a walkover exit's
    /// recess), which belong to no room and so have no
    /// [`crate::ir::Room::wall_tex`] of their own to read.
    pub wall_tex: String,
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
#[expect(
    clippy::struct_excessive_bools,
    reason = "each bool mirrors an independent bit of the engine's `maplinedef_t.flags` \
              bitfield, and UDMF's `doom` namespace spells each as its own named boolean \
              field rather than packing them. Collapsing them into a state machine or \
              two-variant enums would misrepresent a format where any combination is legal"
)]
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
    /// `ML_SECRET`: the automap draws this line as solid wall rather than
    /// revealing that a sector lies beyond it.
    ///
    /// Purely cosmetic and purely automap-side — it changes nothing about
    /// movement, sight, or rendering in the world. Set on the threshold
    /// lines of a portal joining a secret room to an ordinary one.
    pub secret: bool,
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
    /// A portal names two rooms that face no wall of each other.
    #[error("portal `{a}` <-> `{b}`: the rooms face no wall of each other")]
    NotAdjacent {
        /// The first room.
        a: String,
        /// The second room.
        b: String,
    },
    /// A portal opening is wider than the wall it sits in.
    #[error("portal `{a}` <-> `{b}`: opening of {width} exceeds the {available} facing wall")]
    PortalTooWide {
        /// The first room.
        a: String,
        /// The second room.
        b: String,
        /// The requested opening width.
        width: i32,
        /// The available facing-wall length.
        available: i32,
    },
    /// A portal midpoint does not lie on a facing wall.
    #[error("portal `{a}` <-> `{b}`: midpoint ({x}, {y}) is not on a facing wall")]
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
    /// A portal's midpoint sits on a diagonal (45-degree) wall, which v1
    /// does not support hosting a portal on — the opening's endpoints, the
    /// flanking wall pieces, and (for a door) the recess would all have to
    /// land on non-integer coordinates to stay flush with it.
    #[error(
        "portal `{a}` <-> `{b}`: the wall at ({x}, {y}) is diagonal; v1 does not support portals on diagonal walls"
    )]
    PortalOnDiagonalWall {
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
    /// A plain portal's two rooms do not overlap vertically by enough for
    /// the player to pass through the passage sector between them.
    ///
    /// The passage takes the higher of the two floors and the lower of the
    /// two ceilings, so `have` is `min(ceilings) - max(floors)`: its own
    /// headroom. A non-positive value means the sector would be inverted
    /// outright — its floor at or above its ceiling.
    ///
    /// This replaces the retired P1, which capped the floor delta between
    /// connected rooms at `max_step_height` in *either* direction.
    /// `P_TryMove` caps only the climb (`tmfloorz - thing->z > 24*FRACUNIT`)
    /// and leaves falling unrestricted, and a corpus sweep of DOOM, DOOM2,
    /// TNT, and PLUTONIA found 37.77% of passable two-sided lines exceeding
    /// that cap — 62.5% of them permanent static drops. A one-way drop is
    /// idiomatic Doom; a passage the player cannot fit through is not.
    ///
    /// Door portals are deliberately not checked here: a door sector's
    /// ceiling is snapped to its floor by construction, so it can never be
    /// inverted, and **P4** already rejects a door opening below the
    /// player's height on a strictly tighter bound.
    #[error("portal `{a}` <-> `{b}` has {have} units of headroom but the player needs {need}")]
    PortalNoHeadroom {
        /// The first room.
        a: String,
        /// The second room.
        b: String,
        /// The passage sector's headroom, `min(ceilings) - max(floors)`.
        have: i32,
        /// The player's height.
        need: i32,
    },
    /// Two emitted sectors — rooms, a portal's gap sector (passage or door),
    /// or a walkover exit's alcove — overlap in the finished geometry.
    ///
    /// [`sectors::overlaps`](crate::compile::sectors) only ever compares IR
    /// room footprints against each other, so it cannot see this: a gap
    /// sector is compiler-generated geometry with no IR footprint of its
    /// own, and can be driven straight through a third room's interior, or
    /// cross another portal's gap sector at a right angle, without either
    /// room named by either portal ever overlapping anything. Checked once,
    /// after every sector-emitting pass has run, over every pair of emitted
    /// sector polygons.
    #[error(
        "{first} and {second} overlap in the emitted geometry (near ({first_x}, {first_y}) and ({second_x}, {second_y}) respectively)"
    )]
    SectorOverlap {
        /// A human-readable label for the first sector.
        first: String,
        /// A representative point inside the first sector.
        first_x: i32,
        /// A representative point inside the first sector.
        first_y: i32,
        /// A human-readable label for the second sector.
        second: String,
        /// A representative point inside the second sector.
        second_x: i32,
        /// A representative point inside the second sector.
        second_y: i32,
    },
    /// A linedef carries a special but no tag.
    #[error("linedef {index} has special {special} at tag 0, which matches every untagged sector")]
    ActionAtTagZero {
        /// Index of the offending linedef.
        index: usize,
        /// The special it carries.
        special: u16,
    },
    /// An exit's `at` does not lie on any wall of its host room.
    #[error("exit in room `{room}`: midpoint ({x}, {y}) is not on any wall of the room")]
    ExitOffWall {
        /// The host room.
        room: String,
        /// Midpoint X.
        x: i32,
        /// Midpoint Y.
        y: i32,
    },
    /// An exit's `at` sits on a diagonal (45-degree) wall, which v1 does not
    /// support carving an exit into.
    #[error(
        "exit in room `{room}`: the wall at ({x}, {y}) is diagonal; v1 does not support exits on diagonal walls"
    )]
    ExitOnDiagonalWall {
        /// The host room.
        room: String,
        /// Midpoint X.
        x: i32,
        /// Midpoint Y.
        y: i32,
    },
    /// An exit's width exceeds the wall it sits in.
    #[error("exit in room `{room}`: width {width} exceeds the {available} available on that wall")]
    ExitTooWide {
        /// The host room.
        room: String,
        /// The requested width.
        width: i32,
        /// The available wall length.
        available: i32,
    },
    /// A walkover exit's alcove would place a vertex outside the 16-bit map
    /// range every Doom map format uses.
    #[error(
        "exit in room `{room}`: the walkover alcove would place a vertex at ({x}, {y}), outside the map range"
    )]
    ExitAlcoveOutOfRange {
        /// The host room.
        room: String,
        /// The out-of-range vertex's X coordinate.
        x: i32,
        /// The out-of-range vertex's Y coordinate.
        y: i32,
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
    /// A theme's `door` texture is not in the project's curated
    /// door-texture catalog.
    ///
    /// Which texture names "read as a door" is an asset-naming convention,
    /// not something sourced from the engine or derivable from it — see
    /// [`crate::tables::Tables::is_door_texture`]'s doc comment. This is the
    /// honest, structural half of validating it (the vocabulary table
    /// itself carries the other half: every curated name was confirmed
    /// present in the Freedoom fixtures — see `vocabulary.toml`'s
    /// `[door_texture_catalog]` `curated` field).
    #[error(
        "theme `{theme}`'s door texture `{texture}` is not in the curated door-texture catalog"
    )]
    NotADoorTexture {
        /// The theme naming the texture.
        theme: String,
        /// The unrecognized texture name.
        texture: String,
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
    /// The compiled map breaks one or more playability rules.
    #[error(
        "map breaks {} playability rule(s): {}",
        .violations.len(),
        .violations.iter().map(ToString::to_string).collect::<Vec<_>>().join("; ")
    )]
    Playability {
        /// Every violation found, not just the first — the whole point of
        /// collecting them is that an author can fix them in one pass.
        violations: Vec<RuleViolation>,
    },
}

use crate::compile::tags::TagAllocator;
use crate::compile::things::ThingOut;
use crate::ir::Ir;
use crate::rules::{RuleViolation, check_all};
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
/// 2. [`sectors::resolve_secret_specials`] turns each room's
///    [`crate::ir::Room::secret`] flag into rule P18's secret sector special,
///    unless [`crate::ir::Room::special`] already set one explicitly. Pure
///    substitution on `.special`, so it can run any time after step 1 and
///    before emission; placed here, immediately after, so it is not
///    forgotten.
/// 3. [`portals::cut_portals`] cuts an opening into *each* room's own wall
///    (rooms are authored apart — see [`crate::ir::Portal`] — so a portal
///    never shares a single coincident wall between its two rooms). For a
///    [`crate::ir::PortalKind::Plain`] portal this also fills the gap
///    between the two openings with an open passage sector; for a door
///    portal it leaves both flanking walls cut but the gap itself still
///    empty, because that gap is a closed sector rather than a single line.
/// 4. [`doors::emit_doors`] fills that gap with a dedicated closed sector for
///    every door portal — optionally flanked by up to two trim alcove
///    sectors ([`crate::ir::Portal::alcove_near`]/
///    [`crate::ir::Portal::alcove_far`]) — allocating its tag from a fresh
///    [`TagAllocator`] shared with every other tag-consuming pass. Neither
///    room's own footprint is touched — the gap already exists by
///    construction.
/// 5. [`exits::emit_exits`] carves every level exit into its host room's own
///    wall, using the same [`TagAllocator`]. Runs after doors so a thing's
///    clearance (step 7) is measured against the exit's final geometry too.
/// 6. [`sectors::check_no_sector_overlaps`] rejects any two emitted sectors
///    that overlap in 2-D — a gap sector driven through a third room, or two
///    gap sectors from unrelated portals crossing each other. Must run after
///    every sector-emitting pass (steps 1, 3, 4, 5) and before anything that
///    trusts the geometry is sound, which is everything from here on.
/// 7. [`heights::apply_height_textures`] writes the upper and lower textures
///    every height difference exposes, on the one side `r_segs.c` draws.
///    Runs after every sector-emitting pass because it reads final floor and
///    ceiling heights, and after the overlap check because it trusts the
///    geometry it walks.
/// 8. [`things::place_things`] places every thing, measuring clearance and
///    headroom against the geometry emitted by steps 1–5 — not the IR's
///    declared footprints, which an exit alcove can still make stale even
///    though a door no longer does — so it must run after doors and exits
///    are carved, not before.
/// 9. [`tags::check_no_action_at_tag_zero`] rejects any linedef special left
///    at tag 0, which would match every untagged sector in-engine.
/// 10. [`textmap::emit_textmap`] renders the final, validated geometry.
/// 11. [`crate::rules::check_all`] runs the playability catalog over the
///     result and fails the compile if anything is violated.
///
/// Step 11 is part of `compile` rather than a separate call the caller may
/// forget, because the design makes playability violations hard errors: "a
/// door the player cannot fit through is a broken map, not a missed target".
/// Leaving `check_all` optional meant every rule in `rules` was inert unless
/// something else remembered to run it, which is exactly the failure the
/// decision was written to prevent. Use [`compile_reporting`] when you want
/// the geometry *and* the violation list — for a conformance report, say —
/// rather than a hard failure.
///
/// # Errors
/// Returns the first [`CompileError`] raised by any pass, or
/// [`CompileError::Playability`] listing every rule the finished map breaks;
/// nothing is returned unless every pass and every rule succeeds.
pub fn compile(ir: &Ir, tables: &Tables) -> Result<Compiled, CompileError> {
    let (compiled, violations) = compile_reporting(ir, tables)?;
    if violations.is_empty() {
        Ok(compiled)
    } else {
        Err(CompileError::Playability { violations })
    }
}

/// Compiles a room graph and reports its playability violations instead of
/// failing on them.
///
/// Structural errors — anything that would make the geometry itself invalid —
/// still fail here exactly as in [`compile`]; only the playability catalog is
/// downgraded to a returned list. That is what a conformance report needs:
/// the emitted map *and* every rule it breaks, rather than the first refusal.
///
/// # Errors
/// Returns the first [`CompileError`] raised by any geometry, thing, or tag
/// pass. Playability violations are returned in the success value, never as
/// an error.
pub fn compile_reporting(
    ir: &Ir,
    tables: &Tables,
) -> Result<(Compiled, Vec<RuleViolation>), CompileError> {
    let mut data = sectors::emit_sectors(ir)?;
    sectors::resolve_secret_specials(ir, tables, &mut data);
    portals::cut_portals(ir, tables, &mut data)?;
    let mut tags = TagAllocator::new();
    doors::emit_doors(ir, tables, &mut data, &mut tags)?;
    exits::emit_exits(ir, tables, &mut data, &mut tags)?;
    sectors::check_no_sector_overlaps(ir, &data)?;
    heights::apply_height_textures(&mut data);
    let things = things::place_things(ir, tables, &data)?;
    tags::check_no_action_at_tag_zero(&data)?;
    let textmap = textmap::emit_textmap(&data, &things);
    let compiled = Compiled {
        textmap,
        data,
        things,
        tags,
    };
    let violations = check_all(ir, tables, &compiled);
    Ok((compiled, violations))
}
