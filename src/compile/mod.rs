//! Compiles the room-graph IR into UDMF map data.

pub mod doors;
pub mod exits;
pub mod heights;
pub mod lifts;
pub mod portals;
pub mod sectors;
pub mod tags;
pub mod teleports;
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
    /// For an island teleport pad, the index of the room sector it is carved
    /// inside. [`sectors::check_no_sector_overlaps`] exempts exactly that
    /// pair — a pad lies inside its host by construction — and tests every
    /// other pair as usual. `None` for every other sector.
    pub host: Option<usize>,
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
    /// Horizontal texture offset in pixels (UDMF `offsetx`).
    ///
    /// Doom derives a wall texture's horizontal position from this plus the
    /// distance along the line from its start vertex, so a nonzero value
    /// shifts which texture column lands at the line's start. Used to centre
    /// a texture wider than the line it sits on — see
    /// [`crate::compile::exits`], which centres an exit's switch.
    pub x_offset: i32,
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
    /// A recess (a walkover exit's alcove, or a teleport's wall pad) would
    /// place a vertex outside the 16-bit map range every Doom map format
    /// uses.
    #[error(
        "the recess behind room `{host}` (an exit alcove or a teleport pad) would place a vertex at ({x}, {y}), outside the map range"
    )]
    RecessOutOfRange {
        /// The host room.
        host: String,
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
    /// An authored thing stands on a teleport pad's square; it would sit in
    /// the pad sector, not the room that declares it.
    #[error("thing `{kind}` at ({x}, {y}) stands on teleport `{id}`'s pad")]
    TeleportThingOnPad {
        /// The teleport whose pad it stands on.
        id: String,
        /// The vocabulary name.
        kind: String,
        /// X coordinate.
        x: i32,
        /// Y coordinate.
        y: i32,
    },
    /// A teleport destination lands in a sector that already carries a tag
    /// for another purpose.
    #[error("teleport `{id}`: destination sector {sector} already carries a tag")]
    TeleportDestinationSectorTagged {
        /// The teleport whose destination it is.
        id: String,
        /// The already-tagged sector index.
        sector: usize,
    },
    /// A destination marker sits closer to its sector's walls than the
    /// largest arriving thing's radius (rule P15).
    #[error(
        "teleport `{id}`: destination ({x}, {y}) has {have:.1} units of clearance but the largest arriving thing needs {need}"
    )]
    TeleportMarkerTooClose {
        /// The teleport(s) delivering here.
        id: String,
        /// X coordinate.
        x: i32,
        /// Y coordinate.
        y: i32,
        /// Available clearance.
        have: f64,
        /// Required radius.
        need: i32,
    },
    /// A destination sector is too short for the largest arriving thing
    /// (rule P15 / P2).
    #[error(
        "teleport `{id}`: the destination sector has {have} units of headroom but the largest arriving thing needs {need}"
    )]
    TeleportMarkerNoHeadroom {
        /// The teleport(s) delivering here.
        id: String,
        /// Available floor-to-ceiling gap.
        have: i32,
        /// Required height.
        need: i32,
    },
    /// Two player starts occupy the same spot.
    #[error("two player starts overlap at ({x}, {y}); they would telefrag on spawn")]
    OverlappingStarts {
        /// X coordinate.
        x: i32,
        /// Y coordinate.
        y: i32,
    },
    /// A lift portal's two rooms differ by no more than the player's step
    /// height, so the platform would carry them somewhere they could
    /// already walk.
    ///
    /// `P_TryMove` lets the player climb any difference up to
    /// `max_step_height` unaided, so a platform under that is inert
    /// scenery with a `downWaitUpStay` special on it. A plain portal says
    /// the same thing with no moving sector at all.
    #[error(
        "portal `{a}` <-> `{b}` is a lift but its rooms' floors differ by {delta}, within the {step}-unit step: use a plain portal"
    )]
    LiftTravelTooShort {
        /// The first room.
        a: String,
        /// The second room.
        b: String,
        /// The floor difference, which is also the platform's travel.
        delta: i32,
        /// The player's step height.
        step: i32,
    },
    /// A barrier's [`crate::ir::Portal::rise`] is no more than the player's
    /// step height, so they could step over the risen platform instead of
    /// riding it.
    ///
    /// The barrier form of [`LiftTravelTooShort`](Self::LiftTravelTooShort):
    /// a barrier's two rooms are level by definition, so its rise *is* its
    /// travel and is what must clear the step.
    #[error(
        "portal `{a}` <-> `{b}` is a barrier but rises only {rise}, within the {step}-unit step: the player would step over it"
    )]
    LiftRiseTooLow {
        /// The first room.
        a: String,
        /// The second room.
        b: String,
        /// The declared rise.
        rise: i32,
        /// The player's step height.
        step: i32,
    },
    /// A lift portal's alcoves leave the platform shallower than the
    /// player's own diameter, so they could not stand on it.
    ///
    /// The platform fills whatever the alcoves leave of the gap (a lift
    /// declares no thickness of its own), so deep alcoves in a narrow gap
    /// squeeze it. Measured against the diameter rather than the radius: the
    /// player is a cylinder that must fit entirely between the two faces.
    ///
    /// `depth` is signed along the direction of travel through the gap, so
    /// alcoves that overrun the gap entirely report a negative depth rather
    /// than the healthy-looking absolute separation of two reversed faces.
    #[error(
        "portal `{a}` <-> `{b}` leaves {depth} units for the platform, but the player is {need} units across"
    )]
    LiftTooShallow {
        /// The first room.
        a: String,
        /// The second room.
        b: String,
        /// The platform's depth along the gap axis.
        depth: i32,
        /// The player's diameter.
        need: i32,
    },
    /// The map emits more platforms than the engine can run at once.
    ///
    /// `MAXPLATS` bounds `p_plats.c`'s active-plat table, and overflowing it
    /// is fatal in the pinned source. Counted over every emitted platform,
    /// not over some estimate of how many could move together: nothing here
    /// can prove a player will not set them all going at once.
    #[error("the map has {count} platforms but the engine allows {max} active at once")]
    TooManyPlats {
        /// The number of platforms emitted.
        count: usize,
        /// `MAXPLATS`.
        max: usize,
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
    /// The teleport destination markers, for rule P15.
    pub markers: Vec<teleports::Marker>,
    /// The emitted platforms — lifts and barriers.
    pub lifts: Vec<lifts::LiftOut>,
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
///    between the two openings with an open passage sector; for a door or
///    lift portal it leaves both flanking walls cut but the gap itself still
///    empty, because that gap is a sector of its own rather than a single
///    line.
/// 4. [`doors::emit_doors`] fills that gap with a dedicated closed sector for
///    every door portal — optionally flanked by up to two trim alcove
///    sectors ([`crate::ir::Portal::alcove_near`]/
///    [`crate::ir::Portal::alcove_far`]) — allocating its tag from a fresh
///    [`TagAllocator`] shared with every other tag-consuming pass. Neither
///    room's own footprint is touched — the gap already exists by
///    construction.
/// 5. [`exits::emit_exits`] carves every level exit into its host room's own
///    wall, using the same [`TagAllocator`]. Runs after doors so a thing's
///    clearance (step 9) is measured against the exit's final geometry too.
/// 6. [`teleports::emit_teleports`] emits every pad (a hosted island sector
///    or a 64-deep recess), tags each destination sector from the same
///    [`TagAllocator`], and returns the destination markers. Runs after
///    exits so a wall pad and an exit compete for wall spans through
///    `portals::split_wall_for_opening` like any two openings, and before
///    the overlap check since it emits sectors.
/// 7. [`lifts::emit_lifts`] fills every lift portal's gap with one
///    `downWaitUpStay` platform sector — again optionally flanked by
///    alcoves — tagging each from the same [`TagAllocator`]. Runs after
///    `cut_portals` left that gap empty (step 3), and before the overlap
///    check since it emits sectors. Its risers must be written before step
///    9: `heights` fills only empty texture slots, and the platform's own
///    top-face riser is invisible at load-time heights, so `heights` would
///    never write it.
/// 8. [`sectors::check_no_sector_overlaps`] rejects any two emitted sectors
///    that overlap in 2-D — a gap sector driven through a third room, or two
///    gap sectors from unrelated portals crossing each other. Must run after
///    every sector-emitting pass (steps 1, 3, 4, 5, 6, 7) and before
///    anything that trusts the geometry is sound, which is everything from
///    here on.
/// 9. [`heights::apply_height_textures`] writes the upper and lower textures
///    every height difference exposes, on the one side `r_segs.c` draws.
///    Runs after every sector-emitting pass because it reads final floor and
///    ceiling heights, and after the overlap check because it trusts the
///    geometry it walks.
/// 10. [`things::place_things`] places every thing, measuring clearance and
///     headroom against the geometry emitted by steps 1–7 — not the IR's
///     declared footprints, which an exit alcove can still make stale even
///     though a door no longer does — so it must run after doors, exits,
///     teleport pads, and platforms are carved, not before. It also places
///     step 6's markers, holding each to the clearance its arriving thing
///     needs.
/// 11. [`tags::check_no_action_at_tag_zero`] rejects any linedef special left
///     at tag 0, which would match every untagged sector in-engine.
/// 12. [`textmap::emit_textmap`] renders the final, validated geometry.
/// 13. [`crate::rules::check_all`] runs the playability catalog over the
///     result and fails the compile if anything is violated.
///
/// Step 13 is part of `compile` rather than a separate call the caller may
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
    let markers = teleports::emit_teleports(ir, tables, &mut data, &mut tags)?;
    let lifts = lifts::emit_lifts(ir, tables, &mut data, &mut tags)?;
    sectors::check_no_sector_overlaps(ir, &data)?;
    heights::apply_height_textures(&mut data);
    let things = things::place_things(ir, tables, &data, &markers, &lifts)?;
    tags::check_no_action_at_tag_zero(&data)?;
    let textmap = textmap::emit_textmap(&data, &things);
    let compiled = Compiled {
        textmap,
        data,
        things,
        tags,
        markers,
        lifts,
    };
    let violations = check_all(ir, tables, &compiled);
    Ok((compiled, violations))
}
