//! The room-graph intermediate representation.

use std::collections::HashSet;

use serde::Deserialize;

use crate::geom::{
    Axis, Pt, clearance, contains, facing_spans, find_facing_span, outward_sign, wall_edges,
};

impl<'de> Deserialize<'de> for Pt {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let [x, y] = <[i32; 2]>::deserialize(d)?;
        Ok(Self { x, y })
    }
}

/// How two rooms connect.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PortalKind {
    /// An open passage through the wall gap, with no door sector — the
    /// passage itself is still a real sector spanning the gap, just an open,
    /// walkable one with no special.
    Plain,
    /// A manual door filling the wall gap with its own closed sector.
    Door,
    /// A door requiring a key.
    Locked,
}

/// Returns `true`, for use as a serde field default so an absent
/// `skill1..skill5` key in a partially specified [`ThingSkills`] object
/// means "appears on this skill" rather than "excluded from it".
const fn skill_default() -> bool {
    true
}

/// Which of Doom's five skill levels a thing appears on.
///
/// UDMF's `doom` namespace spells these as five independent booleans
/// (`skill1`..`skill5`, ITYTD through Nightmare); this mirrors that shape
/// directly rather than packing them, matching the convention
/// [`crate::compile::textmap::emit_textmap`] already uses for other
/// per-object flags. Every field defaults to `true`, both at the whole-struct
/// level (an [`IrThing`] with no `skills` key at all) and per-field (a
/// `skills` object that names only some keys) — so a thing that never
/// mentions skills keeps the compiler's original "appears on every skill"
/// behavior, and every fixture written before this field existed is
/// unaffected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "these five booleans are not independent flags to model as a state machine — they \
              are a direct, one-to-one mirror of UDMF's own skill1..skill5 fields, and any other \
              shape would need translating back and forth at the emission boundary for no benefit"
)]
pub struct ThingSkills {
    /// UDMF `skill1` — I'm Too Young To Die.
    #[serde(default = "skill_default")]
    pub skill1: bool,
    /// UDMF `skill2` — Hey, Not Too Rough.
    #[serde(default = "skill_default")]
    pub skill2: bool,
    /// UDMF `skill3` — Hurt Me Plenty.
    #[serde(default = "skill_default")]
    pub skill3: bool,
    /// UDMF `skill4` — Ultra-Violence.
    #[serde(default = "skill_default")]
    pub skill4: bool,
    /// UDMF `skill5` — Nightmare!.
    #[serde(default = "skill_default")]
    pub skill5: bool,
}

impl Default for ThingSkills {
    /// All five skills — the behavior every fixture predating this type saw.
    fn default() -> Self {
        Self {
            skill1: true,
            skill2: true,
            skill3: true,
            skill4: true,
            skill5: true,
        }
    }
}

/// A thing placed inside a room.
#[derive(Debug, Clone, Deserialize)]
pub struct IrThing {
    /// Vocabulary name, resolved to a thing ID at compile time.
    pub kind: String,
    /// Position in map units.
    pub at: Pt,
    /// Facing angle in degrees.
    pub angle: u16,
    /// Which skill levels this thing appears on. Defaults to all five when
    /// the key is absent — see [`ThingSkills`].
    #[serde(default)]
    pub skills: ThingSkills,
    /// `MTF_AMBUSH`: the thing wakes on sight only, never on sound
    /// (`data/engine.toml` `[thing.flags]`). Emitted as UDMF `ambush`.
    #[serde(default)]
    pub ambush: bool,
}

/// One room: a closed footprint plus its surfaces and contents.
#[derive(Debug, Clone, Deserialize)]
pub struct Room {
    /// Unique identifier, used by portals and error messages.
    pub id: String,
    /// Clockwise, grid-snapped boundary.
    pub footprint: Vec<Pt>,
    /// Floor height in map units.
    pub floor: i32,
    /// Ceiling height in map units.
    pub ceiling: i32,
    /// Sector light level.
    pub light: i32,
    /// Floor flat name.
    pub floor_tex: String,
    /// Ceiling flat name.
    pub ceil_tex: String,
    /// Wall texture name.
    pub wall_tex: String,
    /// Sector special, if any.
    ///
    /// This is the escape hatch: an author who sets it directly gets exactly
    /// that raw value, with no interpretation. It is mutually exclusive with
    /// [`Self::secret`] — [`Ir::from_json`] rejects a room that sets both,
    /// rather than picking a silent precedence between them (the same
    /// reject-don't-degrade posture [`Portal::width`] already takes on an odd
    /// width). Use `secret` for the common case; use `special` directly for
    /// anything `secret` cannot express.
    #[serde(default)]
    pub special: Option<u16>,
    /// Whether this sector carries rule P18's secret special
    /// (`Tables::secret_sector_special`) rather than a plain 0.
    ///
    /// This is the high-level path: it spares an author from writing the raw
    /// engine-sourced special number into [`Self::special`] by hand. See
    /// that field's doc comment for how the two interact.
    #[serde(default)]
    pub secret: bool,
    /// Things placed in this room.
    #[serde(default)]
    pub things: Vec<IrThing>,
}

/// Returns `true`, for use as a serde field default so an absent
/// `track_lower_unpegged` key means "lower-unpegged", matching real door
/// track construction (the texture must stay anchored to the floor rather
/// than sliding as the door sector's ceiling animates open).
const fn track_lower_unpegged_default() -> bool {
    true
}

/// A connection between two rooms.
///
/// Rooms are authored apart, not flush: `a`'s wall and `b`'s wall face each
/// other across a real void at least [`Ir::MIN_PORTAL_GAP`] units wide (see
/// [`IrError::InvalidPortalGap`]), which the compiler fills with a passage —
/// or, for [`PortalKind::Door`]/[`PortalKind::Locked`], a door construction —
/// that spans the whole gap. `a` and `b` are not fully interchangeable:
/// [`Self::at`] is always read against room `a`'s own wall coordinate (see
/// that field's doc comment), so swapping the two labels without also
/// updating `at` moves which room's wall the portal is measured from. The
/// resulting geometry itself is symmetric — unlike the old flush-wall
/// design, filling the gap no longer carves into either room's own
/// territory, so which room is named `a` versus `b` no longer changes the
/// map's real playable shape.
///
/// [`Self::door_thickness`], [`Self::alcove_near`], and [`Self::alcove_far`]
/// only apply to [`PortalKind::Door`]/[`PortalKind::Locked`] — a
/// [`PortalKind::Plain`] portal has no door and so no thickness or alcove of
/// its own; [`Ir::from_json`] rejects a plain portal that sets any of them
/// (see [`IrError::DoorFieldsOnPlainPortal`]) rather than silently ignoring
/// values that would do nothing, matching the reject-don't-degrade posture
/// [`Self::width`] already takes on an odd value.
#[derive(Debug, Clone, Deserialize)]
pub struct Portal {
    /// Identifier of the first room.
    pub a: String,
    /// Identifier of the second room.
    pub b: String,
    /// The kind of connection.
    pub kind: PortalKind,
    /// Key name when `kind` is [`PortalKind::Locked`].
    #[serde(default)]
    pub lock: Option<String>,
    /// Clear opening width in map units.
    ///
    /// Must be positive and even: the opening is centered on [`Self::at`],
    /// so an odd width could not be split into two equal integer halves, and
    /// a zero or negative one would emit a degenerate or inverted opening.
    /// [`Ir::from_json`] rejects both.
    pub width: i32,
    /// A point on room `a`'s own wall: the coordinate that varies along the
    /// wall gives the opening's along-wall midpoint, and the coordinate held
    /// constant across it must equal room `a`'s own wall position exactly
    /// (not room `b`'s, and not the gap's midpoint).
    pub at: Pt,
    /// The door's own depth along the gap axis, in map units.
    ///
    /// Required for [`PortalKind::Door`]/[`PortalKind::Locked`]
    /// ([`IrError::MissingDoorThickness`] otherwise) and must be one of
    /// [`Ir::DOOR_DIMENSIONS`] ([`IrError::InvalidDoorDimension`]
    /// otherwise). Meaningless (and rejected) on a
    /// [`PortalKind::Plain`] portal.
    #[serde(default)]
    pub door_thickness: Option<i32>,
    /// An optional buffer sector between room `a` and the door, in map
    /// units.
    ///
    /// When present, must be one of [`Ir::DOOR_DIMENSIONS`]
    /// ([`IrError::InvalidDoorDimension`] otherwise). Meaningless (and
    /// rejected) on a [`PortalKind::Plain`] portal. See [`Self::alcove_far`]
    /// for the naming rationale — "near" and "far" name room `a`'s and room
    /// `b`'s own walls, mirroring the compiler's internal `near`/`far`
    /// facing-wall terminology, not a "front"/"behind" the task that
    /// requested this named ambiguously (a corridor is walked in both
    /// directions, so "in front of the door" has no fixed meaning without
    /// picking a travel direction).
    #[serde(default)]
    pub alcove_near: Option<i32>,
    /// An optional buffer sector between the door and room `b`, in map
    /// units. See [`Self::alcove_near`] for the shared constraints and the
    /// near/far naming rationale.
    #[serde(default)]
    pub alcove_far: Option<i32>,
    /// Whether the door's track (the linedefs exposed as the door sector's
    /// ceiling rises) is emitted lower-unpegged, so its `DOORTRAK` texture
    /// stays anchored to the floor rather than sliding as the door opens.
    ///
    /// Defaults to `true` — real door track construction is lower-unpegged
    /// unless an author deliberately opts out. Meaningless on a
    /// [`PortalKind::Plain`] portal, which has no door track at all; unlike
    /// [`Self::door_thickness`]/[`Self::alcove_near`]/[`Self::alcove_far`]
    /// this is not rejected there, since a bare `bool` default carries no
    /// signal that an author deliberately set it (see [`ThingSkills`] for
    /// the same reasoning applied to `skillN` defaults).
    #[serde(default = "track_lower_unpegged_default")]
    pub track_lower_unpegged: bool,
}

/// How the player triggers a level exit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExitTrigger {
    /// A switch the player presses (use-activated; vanilla only honors this
    /// from a linedef's front side).
    Switch,
    /// A line the player walks across.
    Walkover,
    /// A walkover exit line in a room reachable only by teleport — the
    /// player teleports in and steps across it (rule P26; TNT MAP23's
    /// shape). Emits the same specials as [`Self::Walkover`].
    Teleport,
}

/// The level exit: a linedef special carved into one wall of one room.
///
/// Unlike [`Portal`], an exit connects to no second room — it is a special on
/// a segment of its own room's boundary wall, not a passage between two. See
/// [`crate::compile::exits`] for the construction.
#[derive(Debug, Clone, Deserialize)]
pub struct Exit {
    /// Identifier of the room whose wall carries the exit.
    pub room: String,
    /// How the exit is triggered.
    pub trigger: ExitTrigger,
    /// Whether this is the secret exit (`G_SecretExitLevel`) rather than the
    /// normal one (`G_ExitLevel`).
    #[serde(default)]
    pub secret: bool,
    /// Clear width of the exit segment in map units.
    ///
    /// Same positive-and-even constraint as [`Portal::width`], for the same
    /// reason: the segment is centered on [`Self::at`], so an odd width could
    /// not be split into two equal integer halves. [`Ir::from_json`] rejects
    /// both zero-or-negative and odd values.
    pub width: i32,
    /// Midpoint of the exit segment on the room's wall.
    pub at: Pt,
}

/// Where a teleport pad sits relative to its room.
///
/// Both placements emit the same pad — see [`Ir::PAD_SIZE`] — so the choice
/// is purely where the square goes: free-standing inside the room, or
/// pushed into one of its walls with three sides solid. Retail id maps use
/// both, islands about four times as often (51 % vs 14 % of trigger lines
/// in DOOM + DOOM2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PadPlacement {
    /// The center of a free-standing pad inside the room. Must be on the
    /// grid, and the whole square must lie strictly inside the footprint.
    Island(Pt),
    /// A point on one of the room's axis-aligned walls; the pad is recessed
    /// outward from it, exactly as a walkover exit's alcove is.
    Wall(Pt),
}

/// Where a teleport delivers the thing that crosses it: a point in a room
/// (or on one of that room's pads, for a two-way pair) and the facing the
/// arrival takes (`EV_Teleport`: `thing->angle = m->angle`).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Destination {
    /// Identifier of the room the destination lies in.
    pub room: String,
    /// The arrival point, in map units; must lie inside `room`'s footprint
    /// or inside one of `room`'s pad squares.
    pub at: Pt,
    /// The arrival facing, in degrees.
    pub angle: u16,
}

/// Returns `true`, the serde default for [`Teleport::repeatable`].
const fn repeatable_default() -> bool {
    true
}

/// One teleporter: a pad in a room whose four trigger edges deliver to
/// [`Self::to`].
///
/// The pad is always the trigger line's *back* sector: `EV_Teleport` refuses
/// a back-side crossing (`if (side == 1) return 0;`), so entering the pad
/// fires and leaving it does not — which is what lets a two-way pair land
/// the arrival on the other pad. See `data/vocabulary.toml`
/// `[specials.teleport]`.
#[derive(Debug, Clone, Deserialize)]
pub struct Teleport {
    /// Unique identifier, used in error messages and the tag manifest.
    pub id: String,
    /// Identifier of the room the pad sits in.
    pub room: String,
    /// Where the pad goes.
    pub pad: PadPlacement,
    /// Where crossing the pad delivers.
    pub to: Destination,
    /// Emit the monsters-only special (126/125) rather than the one any
    /// thing may cross (97/39). Retail uses it for pens the player never
    /// enters (7 of 8 sealed DOOM + DOOM2 pens).
    #[serde(default)]
    pub monsters_only: bool,
    /// Emit the repeatable ("WR") special rather than the one-shot ("W1")
    /// form, which clears itself after its first crossing.
    #[serde(default = "repeatable_default")]
    pub repeatable: bool,
}

/// A complete room graph.
#[derive(Debug, Clone, Deserialize)]
pub struct Ir {
    /// Seed recorded for reproducibility.
    pub seed: u64,
    /// Grid size every coordinate snaps to.
    pub grid: i32,
    /// Theme name, resolved against the vocabulary table.
    pub theme: String,
    /// The rooms.
    pub rooms: Vec<Room>,
    /// The connections between rooms.
    pub portals: Vec<Portal>,
    /// The level exit(s). Usually one, but nothing here forbids more (e.g. a
    /// normal exit alongside a separate secret exit).
    #[serde(default)]
    pub exits: Vec<Exit>,
    /// The teleporters. Absent means none, so every pre-existing fixture
    /// parses unchanged.
    #[serde(default)]
    pub teleports: Vec<Teleport>,
}

/// Errors raised while loading or validating an IR document.
#[derive(Debug, thiserror::Error)]
pub enum IrError {
    /// The document is not valid JSON, or does not match the schema.
    #[error("invalid IR JSON: {0}")]
    Json(#[from] serde_json::Error),
    /// Two rooms share an identifier.
    #[error("duplicate room id `{id}`")]
    DuplicateRoom {
        /// The repeated identifier.
        id: String,
    },
    /// A portal names a room that does not exist.
    #[error("portal references unknown room `{id}`")]
    UnknownRoom {
        /// The unresolvable identifier.
        id: String,
    },
    /// The document declares a grid size that is zero or negative.
    #[error("grid must be positive, got {grid}")]
    InvalidGrid {
        /// The rejected grid size.
        grid: i32,
    },
    /// A coordinate is not a multiple of the grid.
    #[error("room `{room}` has an off-grid coordinate ({x}, {y}); grid is {grid}")]
    OffGrid {
        /// The offending room.
        room: String,
        /// The X coordinate.
        x: i32,
        /// The Y coordinate.
        y: i32,
        /// The configured grid size.
        grid: i32,
    },
    /// A coordinate does not fit the 16-bit vertex range every Doom map
    /// format uses.
    #[error("`{subject}` has a coordinate ({x}, {y}) outside the map range {min}..={max}")]
    CoordinateOutOfRange {
        /// The room or portal that carries it.
        subject: String,
        /// The X coordinate.
        x: i32,
        /// The Y coordinate.
        y: i32,
        /// The lowest representable coordinate.
        min: i32,
        /// The highest representable coordinate.
        max: i32,
    },
    /// A room's ceiling is not above its floor, so it encloses no volume.
    #[error("room `{room}` has ceiling {ceiling} at or below floor {floor}")]
    InvertedRoom {
        /// The offending room.
        room: String,
        /// Its declared floor height.
        floor: i32,
        /// Its declared ceiling height.
        ceiling: i32,
    },
    /// A room height does not fit the 16-bit range every Doom map format
    /// uses for sector planes.
    #[error("room `{room}` has a height {height} outside the map range {min}..={max}")]
    HeightOutOfRange {
        /// The offending room.
        room: String,
        /// The rejected floor or ceiling height.
        height: i32,
        /// The lowest representable height.
        min: i32,
        /// The highest representable height.
        max: i32,
    },
    /// A portal declares a width that is zero or negative.
    #[error("portal `{a}` <-> `{b}` has width {width}, which must be positive")]
    InvalidPortalWidth {
        /// The first room.
        a: String,
        /// The second room.
        b: String,
        /// The rejected width.
        width: i32,
    },
    /// A portal declares an odd width, which cannot be centered on `at`
    /// without landing half a unit off the integer grid.
    #[error("portal `{a}` <-> `{b}` has odd width {width}; widths must be even")]
    OddPortalWidth {
        /// The first room.
        a: String,
        /// The second room.
        b: String,
        /// The rejected width.
        width: i32,
    },
    /// A [`PortalKind::Locked`] portal does not name the key that opens it.
    #[error("portal `{a}` <-> `{b}` is locked but names no key")]
    LockedWithoutKey {
        /// The first room.
        a: String,
        /// The second room.
        b: String,
    },
    /// A portal's facing walls are separated by a void narrower than
    /// [`Ir::MIN_PORTAL_GAP`] units, or by a gap that is not a whole
    /// multiple of it — rooms are authored apart, and the void between them
    /// is real, solid wall, not the zero-thickness construction two flush
    /// rooms produce.
    #[error(
        "portal `{a}` <-> `{b}` has a {gap}-unit gap between its facing walls; the gap must be \
         at least 8 units and a multiple of 8"
    )]
    InvalidPortalGap {
        /// The first room.
        a: String,
        /// The second room.
        b: String,
        /// The rejected gap width.
        gap: i32,
    },
    /// A room sets both [`Room::secret`] and an explicit [`Room::special`],
    /// leaving no way to tell which one the author actually wants.
    #[error("room `{room}` sets both `secret` and an explicit `special`; use one or the other")]
    SecretWithExplicitSpecial {
        /// The offending room.
        room: String,
    },
    /// An exit names a room that does not exist.
    #[error("exit references unknown room `{room}`")]
    ExitUnknownRoom {
        /// The unresolvable identifier.
        room: String,
    },
    /// An exit declares a width that is zero or negative.
    #[error("exit in room `{room}` has width {width}, which must be positive")]
    InvalidExitWidth {
        /// The room.
        room: String,
        /// The rejected width.
        width: i32,
    },
    /// An exit declares an odd width, which cannot be centered on `at`
    /// without landing half a unit off the integer grid.
    #[error("exit in room `{room}` has odd width {width}; widths must be even")]
    OddExitWidth {
        /// The room.
        room: String,
        /// The rejected width.
        width: i32,
    },
    /// A [`PortalKind::Door`]/[`PortalKind::Locked`] portal names no
    /// [`Portal::door_thickness`].
    #[error("portal `{a}` <-> `{b}` is a door but names no door_thickness")]
    MissingDoorThickness {
        /// The first room.
        a: String,
        /// The second room.
        b: String,
    },
    /// A [`Portal::door_thickness`], [`Portal::alcove_near`], or
    /// [`Portal::alcove_far`] value is not one of [`Ir::DOOR_DIMENSIONS`].
    #[error("portal `{a}` <-> `{b}` has {field} {value}, which must be 8, 16, or 32 map units")]
    InvalidDoorDimension {
        /// The first room.
        a: String,
        /// The second room.
        b: String,
        /// Which field was rejected: `"door_thickness"`, `"alcove_near"`, or
        /// `"alcove_far"`.
        field: &'static str,
        /// The rejected value.
        value: i32,
    },
    /// A [`PortalKind::Plain`] portal sets [`Portal::door_thickness`],
    /// [`Portal::alcove_near`], or [`Portal::alcove_far`] — fields that only
    /// mean something for a door, and would otherwise be silently ignored.
    #[error(
        "portal `{a}` <-> `{b}` is plain but sets a door field ({field}); use `door` or `locked`, or remove it"
    )]
    DoorFieldsOnPlainPortal {
        /// The first room.
        a: String,
        /// The second room.
        b: String,
        /// Which field was set: `"door_thickness"`, `"alcove_near"`, or
        /// `"alcove_far"`.
        field: &'static str,
    },
    /// A [`PortalKind::Door`]/[`PortalKind::Locked`] portal's facing-wall gap
    /// does not exactly equal [`Portal::door_thickness`] plus
    /// [`Portal::alcove_near`] plus [`Portal::alcove_far`] (each alcove
    /// counted as 0 when absent).
    ///
    /// Exact equality, not merely "at least" — see this variant's citation in
    /// the door-redesign report for why a gap wider than the sum is
    /// unsound, not just untidy: the leftover span would have no sector to
    /// fill it, breaking the passage between the door and whichever real
    /// room or alcove sits beyond the shortfall.
    #[error(
        "portal `{a}` <-> `{b}` has a {gap}-unit gap, but door_thickness {door_thickness} + \
         alcove_near {alcove_near} + alcove_far {alcove_far} = {needed}; the gap must equal \
         that sum exactly"
    )]
    DoorGapMismatch {
        /// The first room.
        a: String,
        /// The second room.
        b: String,
        /// The gap actually measured between the two facing walls.
        gap: i32,
        /// The door's own thickness.
        door_thickness: i32,
        /// The near alcove's length (0 when absent).
        alcove_near: i32,
        /// The far alcove's length (0 when absent).
        alcove_far: i32,
        /// `door_thickness + alcove_near + alcove_far`.
        needed: i32,
    },
    /// Two teleports share an id.
    #[error("duplicate teleport id `{id}`")]
    DuplicateTeleport {
        /// The repeated identifier.
        id: String,
    },
    /// A teleport names a room that does not exist, as its pad's room or
    /// its destination's.
    #[error("teleport `{id}` references unknown room `{room}`")]
    TeleportUnknownRoom {
        /// The teleport's identifier.
        id: String,
        /// The unresolvable room identifier.
        room: String,
    },
    /// An island pad's center is off the grid.
    #[error("teleport `{id}` has an off-grid pad center ({x}, {y}); grid is {grid}")]
    TeleportPadOffGrid {
        /// The teleport's identifier.
        id: String,
        /// The X coordinate.
        x: i32,
        /// The Y coordinate.
        y: i32,
        /// The configured grid size.
        grid: i32,
    },
    /// An island pad's square is not strictly inside its room.
    #[error(
        "teleport `{id}`: the pad centered at ({x}, {y}) does not lie strictly inside its room"
    )]
    TeleportPadOutsideRoom {
        /// The teleport's identifier.
        id: String,
        /// The X coordinate of the pad's center.
        x: i32,
        /// The Y coordinate of the pad's center.
        y: i32,
    },
    /// A wall pad's point is on no axis-aligned wall of its room, or its
    /// 64-unit span runs past the wall's ends.
    #[error(
        "teleport `{id}`: ({x}, {y}) is not on an axis-aligned wall of its room with 64 units \
         of wall around it"
    )]
    TeleportPadOffWall {
        /// The teleport's identifier.
        id: String,
        /// The X coordinate of the wall point.
        x: i32,
        /// The Y coordinate of the wall point.
        y: i32,
    },
    /// Two pads in one room overlap or touch (touching would emit coincident
    /// linedefs).
    #[error("teleports `{first}` and `{second}` have pads that overlap or touch")]
    TeleportPadsOverlap {
        /// The first teleport's identifier.
        first: String,
        /// The second teleport's identifier.
        second: String,
    },
    /// A destination point is outside its room and on none of its pads.
    #[error(
        "teleport `{id}`: destination ({x}, {y}) is outside room `{room}` and on none of its pads"
    )]
    TeleportDestinationOutsideRoom {
        /// The teleport's identifier.
        id: String,
        /// The destination's room identifier.
        room: String,
        /// The X coordinate of the destination.
        x: i32,
        /// The Y coordinate of the destination.
        y: i32,
    },
    /// Two teleports deliver to different points of one sector; the engine
    /// takes the first marker it finds, so the IR refuses the ambiguity.
    #[error("teleports `{first}` and `{second}` deliver to different points of the same sector")]
    TeleportDestinationsShareSector {
        /// The first teleport's identifier.
        first: String,
        /// The second teleport's identifier.
        second: String,
    },
}

/// The inclusive coordinate and height range every Doom map format stores in
/// a signed 16-bit field.
///
/// UDMF itself is textual and would accept wider values, but the design's
/// second output is a Doom binary WAD produced by `cwad convert --to doom`
/// (see the spec's architecture diagram), whose `VERTEXES` and `SECTORS`
/// lumps are `i16`. Rejecting here keeps the two outputs equivalent instead
/// of letting the binary one silently wrap.
const MAP_RANGE: std::ops::RangeInclusive<i32> = (i16::MIN as i32)..=(i16::MAX as i32);

impl Ir {
    /// The minimum width of the void between two rooms' facing walls, in map
    /// units, and the granularity every gap must be a whole multiple of.
    ///
    /// A compiler-construction constant, not an engine-sourced one — nothing
    /// in the Doom engine constrains wall thickness. 8 is the smallest
    /// distance that reads as real wall material rather than a rounding
    /// artifact, and keeps every gap on the same fine grid door tracks and
    /// jambs are commonly built on.
    pub const MIN_PORTAL_GAP: i32 = 8;

    /// The legal values for [`Portal::door_thickness`], [`Portal::alcove_near`],
    /// and [`Portal::alcove_far`], in map units.
    ///
    /// A compiler-construction constant, like [`Self::MIN_PORTAL_GAP`], not
    /// an engine-sourced one — nothing in the Doom engine constrains door or
    /// alcove depth. Three enumerated sizes (rather than "any multiple of
    /// [`Self::MIN_PORTAL_GAP`]") is a deliberate authoring constraint the
    /// playtester's request itself specified, matching real mapping
    /// practice: a door is built at one of a few conventional depths, not an
    /// arbitrary one.
    pub const DOOR_DIMENSIONS: [i32; 3] = [8, 16, 32];

    /// The side of every teleport pad, in map units.
    ///
    /// A compiler-construction constant fixed by measurement, not an engine
    /// fact: 77 of the 83 free-standing pads in DOOM.WAD + DOOM2.WAD are
    /// exactly 64×64, and 89 of 94 wall alcoves are 64 wide and 81 of 94 64
    /// deep (docs/measurements/teleports-*.md, probe round 2). The corpus
    /// does not vary it, so neither does the IR.
    pub const PAD_SIZE: i32 = 64;
    /// How far a pad's floor sits above its host room's, in map units.
    ///
    /// Same provenance as [`Self::PAD_SIZE`]: +8 is the most common retail
    /// step (36 of 83 island pads; +24 and +16 follow with 22 and 13), and
    /// it is well under the engine's step-up cap, so a pad is always
    /// walkable onto. The 16/24 variants are a recorded follow-up.
    pub const PAD_FLOOR_STEP: i32 = 8;

    /// Parses and validates an IR document.
    ///
    /// Every numeric field a later pass divides by, halves, or writes into a
    /// fixed-width map record is bounds-checked here, at the untrusted-input
    /// boundary, so no downstream pass has to defend itself: `grid`
    /// ([`IrError::InvalidGrid`]) because `x % 0` panics, `width`
    /// ([`IrError::InvalidPortalWidth`], [`IrError::OddPortalWidth`]) because
    /// a zero width emits a zero-length two-sided linedef and a negative one
    /// inverts the opening, room heights ([`IrError::InvertedRoom`]) because
    /// a ceiling at or below its floor encloses no playable volume, and both
    /// coordinates and heights ([`IrError::CoordinateOutOfRange`],
    /// [`IrError::HeightOutOfRange`]) because the binary Doom output stores
    /// them in `i16`.
    ///
    /// # Errors
    /// Returns [`IrError::Json`] for malformed input, [`IrError::DuplicateRoom`]
    /// for a repeated room id, [`IrError::UnknownRoom`] for a portal naming a
    /// room that does not exist, [`IrError::OffGrid`] for any coordinate that
    /// is not a multiple of `grid`, [`IrError::LockedWithoutKey`] for a locked
    /// portal that names no key, [`IrError::InvalidPortalGap`] for a portal
    /// whose facing walls are separated by less than [`Self::MIN_PORTAL_GAP`]
    /// units or by a gap that is not a whole multiple of it,
    /// [`IrError::SecretWithExplicitSpecial`] for a
    /// room that sets both `secret` and `special`, [`IrError::ExitUnknownRoom`]
    /// for an exit naming a room that does not exist,
    /// [`IrError::InvalidExitWidth`]/[`IrError::OddExitWidth`] for a
    /// non-positive or odd exit width, [`IrError::MissingDoorThickness`] for
    /// a door/locked portal that names no `door_thickness`,
    /// [`IrError::InvalidDoorDimension`] for a `door_thickness`,
    /// `alcove_near`, or `alcove_far` that is not 8, 16, or 32,
    /// [`IrError::DoorFieldsOnPlainPortal`] for a plain portal that sets any
    /// of those three fields, [`IrError::DoorGapMismatch`] for a door/locked
    /// portal whose facing-wall gap does not exactly equal `door_thickness +
    /// alcove_near + alcove_far`, the numeric-range variants listed above
    /// (which an exit's `at` is also checked against),
    /// [`IrError::DuplicateTeleport`] for a repeated teleport id,
    /// [`IrError::TeleportUnknownRoom`] for a teleport naming a room that
    /// does not exist as its pad's room or its destination's,
    /// [`IrError::TeleportPadOffGrid`] for an island pad whose center is not
    /// a multiple of `grid`, [`IrError::TeleportPadOutsideRoom`] for an
    /// island pad whose square does not lie strictly inside its room,
    /// [`IrError::TeleportPadOffWall`] for a wall pad whose point is on no
    /// axis-aligned wall or whose span runs past the wall's ends,
    /// [`IrError::TeleportPadsOverlap`] for two pads in one room that overlap
    /// or touch, [`IrError::TeleportDestinationOutsideRoom`] for a
    /// destination outside its room and on none of its pads, and
    /// [`IrError::TeleportDestinationsShareSector`] for two teleports that
    /// deliver to different points of the same emitted sector.
    pub fn from_json(s: &str) -> Result<Self, IrError> {
        let ir: Self = serde_json::from_str(s)?;

        // Before anything divides by it: `x % 0` panics in Rust, and this is
        // the untrusted-input boundary, so it must return an error instead.
        if ir.grid <= 0 {
            return Err(IrError::InvalidGrid { grid: ir.grid });
        }

        let seen = Self::validate_rooms(&ir)?;
        Self::validate_portals(&ir, &seen)?;
        Self::validate_portal_gaps(&ir)?;
        Self::validate_door_dimensions(&ir)?;
        Self::validate_door_gap(&ir)?;
        Self::validate_exits(&ir, &seen)?;
        Self::validate_teleports(&ir, &seen)?;

        Ok(ir)
    }

    /// Validates every room and returns the set of ids seen, for
    /// [`Self::validate_portals`] and [`Self::validate_exits`] to check their
    /// own room references against.
    fn validate_rooms(ir: &Self) -> Result<HashSet<&str>, IrError> {
        let mut seen = HashSet::new();
        for room in &ir.rooms {
            if !seen.insert(room.id.as_str()) {
                return Err(IrError::DuplicateRoom {
                    id: room.id.clone(),
                });
            }
            if room.ceiling <= room.floor {
                return Err(IrError::InvertedRoom {
                    room: room.id.clone(),
                    floor: room.floor,
                    ceiling: room.ceiling,
                });
            }
            if room.secret && room.special.is_some() {
                return Err(IrError::SecretWithExplicitSpecial {
                    room: room.id.clone(),
                });
            }
            for height in [room.floor, room.ceiling] {
                if !MAP_RANGE.contains(&height) {
                    return Err(IrError::HeightOutOfRange {
                        room: room.id.clone(),
                        height,
                        min: *MAP_RANGE.start(),
                        max: *MAP_RANGE.end(),
                    });
                }
            }
            for p in &room.footprint {
                if p.x % ir.grid != 0 || p.y % ir.grid != 0 {
                    return Err(IrError::OffGrid {
                        room: room.id.clone(),
                        x: p.x,
                        y: p.y,
                        grid: ir.grid,
                    });
                }
                if !MAP_RANGE.contains(&p.x) || !MAP_RANGE.contains(&p.y) {
                    return Err(IrError::CoordinateOutOfRange {
                        subject: room.id.clone(),
                        x: p.x,
                        y: p.y,
                        min: *MAP_RANGE.start(),
                        max: *MAP_RANGE.end(),
                    });
                }
            }
        }
        Ok(seen)
    }

    /// Validates every portal against the room ids [`Self::validate_rooms`]
    /// already collected.
    fn validate_portals(ir: &Self, seen: &HashSet<&str>) -> Result<(), IrError> {
        for portal in &ir.portals {
            for id in [&portal.a, &portal.b] {
                if !seen.contains(id.as_str()) {
                    return Err(IrError::UnknownRoom { id: id.clone() });
                }
            }
            if portal.width <= 0 {
                return Err(IrError::InvalidPortalWidth {
                    a: portal.a.clone(),
                    b: portal.b.clone(),
                    width: portal.width,
                });
            }
            if portal.width % 2 != 0 {
                return Err(IrError::OddPortalWidth {
                    a: portal.a.clone(),
                    b: portal.b.clone(),
                    width: portal.width,
                });
            }
            if !MAP_RANGE.contains(&portal.at.x) || !MAP_RANGE.contains(&portal.at.y) {
                return Err(IrError::CoordinateOutOfRange {
                    subject: format!("{} <-> {}", portal.a, portal.b),
                    x: portal.at.x,
                    y: portal.at.y,
                    min: *MAP_RANGE.start(),
                    max: *MAP_RANGE.end(),
                });
            }
            if portal.kind == PortalKind::Locked && portal.lock.is_none() {
                return Err(IrError::LockedWithoutKey {
                    a: portal.a.clone(),
                    b: portal.b.clone(),
                });
            }
        }
        Ok(())
    }

    /// Validates every portal's facing-wall gap.
    ///
    /// Only a portal whose `at` actually resolves to a real facing-wall pair
    /// is checked here — [`crate::geom::find_facing_span`] applied to
    /// [`crate::geom::facing_spans`] over the two rooms' footprints, the
    /// exact geometry [`crate::compile::portals::resolve_portal`] uses at
    /// compile time, so the two can never disagree about which span a point
    /// resolves to. A portal naming rooms that share no facing wall at all,
    /// or whose `at` lands off of every one they do share, is left for that
    /// later, more specific structural error (`NotAdjacent`/`PortalOffWall`)
    /// to report — this pass only judges the gap once a span is found.
    ///
    /// Runs after [`Self::validate_portals`], which already rejects an
    /// unknown room id, so both room lookups below are guaranteed to
    /// succeed.
    fn validate_portal_gaps(ir: &Self) -> Result<(), IrError> {
        for portal in &ir.portals {
            let room_a = ir.room(&portal.a).expect("validated by validate_portals");
            let room_b = ir.room(&portal.b).expect("validated by validate_portals");

            let spans = facing_spans(&room_a.footprint, &room_b.footprint);
            let Some(span) = find_facing_span(&spans, portal.at) else {
                continue;
            };

            let gap = span.gap();
            if gap < Self::MIN_PORTAL_GAP || gap % Self::MIN_PORTAL_GAP != 0 {
                return Err(IrError::InvalidPortalGap {
                    a: portal.a.clone(),
                    b: portal.b.clone(),
                    gap,
                });
            }
        }
        Ok(())
    }

    /// Validates every portal's door-only fields
    /// ([`Portal::door_thickness`]/[`Portal::alcove_near`]/[`Portal::alcove_far`]):
    /// present and one of [`Self::DOOR_DIMENSIONS`] for
    /// [`PortalKind::Door`]/[`PortalKind::Locked`], and absent for
    /// [`PortalKind::Plain`].
    ///
    /// Runs unconditionally over every portal — unlike [`Self::validate_door_gap`],
    /// this does not depend on `at` resolving to a real facing span, since a
    /// malformed field (a missing thickness, or one set on a plain portal)
    /// is wrong regardless of where the portal sits.
    fn validate_door_dimensions(ir: &Self) -> Result<(), IrError> {
        for portal in &ir.portals {
            let fields: [(&'static str, Option<i32>); 3] = [
                ("door_thickness", portal.door_thickness),
                ("alcove_near", portal.alcove_near),
                ("alcove_far", portal.alcove_far),
            ];
            match portal.kind {
                PortalKind::Plain => {
                    for (field, value) in fields {
                        if value.is_some() {
                            return Err(IrError::DoorFieldsOnPlainPortal {
                                a: portal.a.clone(),
                                b: portal.b.clone(),
                                field,
                            });
                        }
                    }
                }
                PortalKind::Door | PortalKind::Locked => {
                    if portal.door_thickness.is_none() {
                        return Err(IrError::MissingDoorThickness {
                            a: portal.a.clone(),
                            b: portal.b.clone(),
                        });
                    }
                    for (field, value) in fields {
                        if let Some(value) = value
                            && !Self::DOOR_DIMENSIONS.contains(&value)
                        {
                            return Err(IrError::InvalidDoorDimension {
                                a: portal.a.clone(),
                                b: portal.b.clone(),
                                field,
                                value,
                            });
                        }
                    }
                }
            }
        }
        Ok(())
    }

    /// Validates every [`PortalKind::Door`]/[`PortalKind::Locked`] portal's
    /// facing-wall gap against [`Portal::door_thickness`] +
    /// [`Portal::alcove_near`] + [`Portal::alcove_far`].
    ///
    /// Runs after [`Self::validate_door_dimensions`], so every value it reads
    /// is already known to be present and one of [`Self::DOOR_DIMENSIONS`].
    /// Like [`Self::validate_portal_gaps`], only a portal whose `at` actually
    /// resolves to a real facing-wall pair is checked here — a portal naming
    /// rooms that share no facing wall, or whose `at` lands off of every one
    /// they do share, is left for the later, more specific structural error
    /// (`NotAdjacent`/`PortalOffWall`) to report.
    fn validate_door_gap(ir: &Self) -> Result<(), IrError> {
        for portal in &ir.portals {
            let Some(door_thickness) = portal.door_thickness else {
                continue;
            };
            let room_a = ir.room(&portal.a).expect("validated by validate_portals");
            let room_b = ir.room(&portal.b).expect("validated by validate_portals");

            let spans = facing_spans(&room_a.footprint, &room_b.footprint);
            let Some(span) = find_facing_span(&spans, portal.at) else {
                continue;
            };

            let alcove_near = portal.alcove_near.unwrap_or(0);
            let alcove_far = portal.alcove_far.unwrap_or(0);
            let needed = door_thickness + alcove_near + alcove_far;
            let gap = span.gap();
            if gap != needed {
                return Err(IrError::DoorGapMismatch {
                    a: portal.a.clone(),
                    b: portal.b.clone(),
                    gap,
                    door_thickness,
                    alcove_near,
                    alcove_far,
                    needed,
                });
            }
        }
        Ok(())
    }

    /// Validates every exit against the room ids [`Self::validate_rooms`]
    /// already collected.
    fn validate_exits(ir: &Self, seen: &HashSet<&str>) -> Result<(), IrError> {
        for exit in &ir.exits {
            if !seen.contains(exit.room.as_str()) {
                return Err(IrError::ExitUnknownRoom {
                    room: exit.room.clone(),
                });
            }
            if exit.width <= 0 {
                return Err(IrError::InvalidExitWidth {
                    room: exit.room.clone(),
                    width: exit.width,
                });
            }
            if exit.width % 2 != 0 {
                return Err(IrError::OddExitWidth {
                    room: exit.room.clone(),
                    width: exit.width,
                });
            }
            if !MAP_RANGE.contains(&exit.at.x) || !MAP_RANGE.contains(&exit.at.y) {
                return Err(IrError::CoordinateOutOfRange {
                    subject: format!("exit in room `{}`", exit.room),
                    x: exit.at.x,
                    y: exit.at.y,
                    min: *MAP_RANGE.start(),
                    max: *MAP_RANGE.end(),
                });
            }
        }
        Ok(())
    }

    /// Validates every teleport: ids, rooms, pad placement, destination
    /// containment, and one destination point per sector.
    fn validate_teleports(ir: &Self, seen: &HashSet<&str>) -> Result<(), IrError> {
        let mut ids = HashSet::new();
        for t in &ir.teleports {
            if !ids.insert(t.id.as_str()) {
                return Err(IrError::DuplicateTeleport { id: t.id.clone() });
            }
            Self::validate_one_teleport(ir, seen, t)?;
        }
        Self::validate_pad_overlaps(ir)?;
        Self::validate_destination_sectors(ir)?;
        Ok(())
    }

    /// Validates one teleport's own room references, pad placement, and
    /// destination containment — everything that does not require comparing
    /// it against another teleport.
    fn validate_one_teleport(ir: &Self, seen: &HashSet<&str>, t: &Teleport) -> Result<(), IrError> {
        for room in [&t.room, &t.to.room] {
            if !seen.contains(room.as_str()) {
                return Err(IrError::TeleportUnknownRoom {
                    id: t.id.clone(),
                    room: room.clone(),
                });
            }
        }
        let room = ir.room(&t.room).expect("checked above");
        let point = match t.pad {
            PadPlacement::Island(c) | PadPlacement::Wall(c) => c,
        };
        if !MAP_RANGE.contains(&point.x) || !MAP_RANGE.contains(&point.y) {
            return Err(IrError::CoordinateOutOfRange {
                subject: format!("teleport `{}`", t.id),
                x: point.x,
                y: point.y,
                min: *MAP_RANGE.start(),
                max: *MAP_RANGE.end(),
            });
        }
        match t.pad {
            PadPlacement::Island(c) => {
                // Checked before the grid: a pad flush against a wall (its
                // edge, not its off-grid center, is what a mapper reasons
                // about) is rejected for that reason even when its center
                // also happens to be off-grid — the two conditions are
                // independent authoring mistakes, and this order is what
                // makes the "flush against the wall" case report the
                // geometric problem rather than the coincidental grid one.
                let (lo, hi) = pad_square(room, t.pad).expect("an island square always resolves");
                let corners = [lo, Pt { x: lo.x, y: hi.y }, hi, Pt { x: hi.x, y: lo.y }];
                let inside = corners
                    .iter()
                    .all(|&p| contains(&room.footprint, p) && clearance(&room.footprint, p) > 0.0);
                let vertex_in_square = room.footprint.iter().any(|&v| square_contains(lo, hi, v));
                if !inside || vertex_in_square {
                    return Err(IrError::TeleportPadOutsideRoom {
                        id: t.id.clone(),
                        x: c.x,
                        y: c.y,
                    });
                }
                if c.x % ir.grid != 0 || c.y % ir.grid != 0 {
                    return Err(IrError::TeleportPadOffGrid {
                        id: t.id.clone(),
                        x: c.x,
                        y: c.y,
                        grid: ir.grid,
                    });
                }
            }
            PadPlacement::Wall(at) => {
                if pad_square(room, t.pad).is_none() {
                    return Err(IrError::TeleportPadOffWall {
                        id: t.id.clone(),
                        x: at.x,
                        y: at.y,
                    });
                }
            }
        }
        if destination_sector_key(ir, &t.to).is_none() {
            return Err(IrError::TeleportDestinationOutsideRoom {
                id: t.id.clone(),
                room: t.to.room.clone(),
                x: t.to.at.x,
                y: t.to.at.y,
            });
        }
        Ok(())
    }

    /// Rejects any two pads in one room that overlap or touch — touching
    /// squares would emit coincident linedefs.
    fn validate_pad_overlaps(ir: &Self) -> Result<(), IrError> {
        for (i, a) in ir.teleports.iter().enumerate() {
            for b in &ir.teleports[i + 1..] {
                if a.room != b.room {
                    continue;
                }
                let room = ir.room(&a.room).expect("validated above");
                let (Some(sa), Some(sb)) = (pad_square(room, a.pad), pad_square(room, b.pad))
                else {
                    continue;
                };
                if squares_overlap_or_touch(sa, sb) {
                    return Err(IrError::TeleportPadsOverlap {
                        first: a.id.clone(),
                        second: b.id.clone(),
                    });
                }
            }
        }
        Ok(())
    }

    /// Rejects two teleports that deliver to different points of one
    /// emitted sector — the engine takes the first marker it finds, so an
    /// ambiguous sector has no defined arrival.
    fn validate_destination_sectors(ir: &Self) -> Result<(), IrError> {
        for (i, a) in ir.teleports.iter().enumerate() {
            for b in &ir.teleports[i + 1..] {
                let (ka, kb) = (
                    destination_sector_key(ir, &a.to),
                    destination_sector_key(ir, &b.to),
                );
                if ka.is_some() && ka == kb && a.to != b.to {
                    return Err(IrError::TeleportDestinationsShareSector {
                        first: a.id.clone(),
                        second: b.id.clone(),
                    });
                }
            }
        }
        Ok(())
    }

    /// Looks up a room by identifier.
    #[must_use]
    pub fn room(&self, id: &str) -> Option<&Room> {
        self.rooms.iter().find(|r| r.id == id)
    }
}

/// The axis-aligned square a pad occupies once emitted, as `(low corner,
/// high corner)`, or `None` for a wall pad whose point is on no axis-aligned
/// wall or whose 64-unit span runs past the wall's ends.
///
/// Shared by [`Ir::from_json`]'s validation and `compile::teleports`, so the
/// two can never disagree about where a pad is — the same reason
/// [`crate::geom::facing_spans`] is shared between portal validation and
/// portal cutting.
pub(crate) fn pad_square(room: &Room, pad: PadPlacement) -> Option<(Pt, Pt)> {
    let half = Ir::PAD_SIZE / 2;
    match pad {
        PadPlacement::Island(c) => Some((
            Pt {
                x: c.x - half,
                y: c.y - half,
            },
            Pt {
                x: c.x + half,
                y: c.y + half,
            },
        )),
        PadPlacement::Wall(at) => {
            let (axis, fixed, lo, hi, forward) =
                wall_edges(&room.footprint).find(|&(axis, fixed, lo, hi, _)| {
                    let (along, across) = axis.split(at);
                    across == fixed && along > lo && along < hi
                })?;
            let (along, _) = axis.split(at);
            let (open_lo, open_hi) = (along - half, along + half);
            if open_lo < lo || open_hi > hi {
                return None;
            }
            let far = fixed + outward_sign(axis, forward) * Ir::PAD_SIZE;
            let (near_x, far_x) = (fixed.min(far), fixed.max(far));
            Some(match axis {
                Axis::Vertical => (
                    Pt {
                        x: near_x,
                        y: open_lo,
                    },
                    Pt {
                        x: far_x,
                        y: open_hi,
                    },
                ),
                Axis::Horizontal => (
                    Pt {
                        x: open_lo,
                        y: near_x,
                    },
                    Pt {
                        x: open_hi,
                        y: far_x,
                    },
                ),
            })
        }
    }
}

/// Whether `p` lies inside (or on) the closed square `(lo, hi)`.
pub(crate) fn square_contains(lo: Pt, hi: Pt, p: Pt) -> bool {
    p.x >= lo.x && p.x <= hi.x && p.y >= lo.y && p.y <= hi.y
}

/// Whether two axis-aligned closed squares, each given as `(low corner, high
/// corner)`, overlap or share so much as a boundary edge or corner.
///
/// The squares are apart only when one lies strictly beyond the other along
/// some axis (a real gap on that axis); anything else — including two
/// squares that meet exactly along one edge, which would emit coincident
/// linedefs — counts as touching.
pub(crate) fn squares_overlap_or_touch(a: (Pt, Pt), b: (Pt, Pt)) -> bool {
    let (alo, ahi) = a;
    let (blo, bhi) = b;
    let apart = alo.x > bhi.x || blo.x > ahi.x || alo.y > bhi.y || blo.y > ahi.y;
    !apart
}

/// Which emitted sector a destination lands in: `(room index, Some(pad
/// index))` when `to.at` lies on one of that room's pads, `(room index,
/// None)` when it lies in the room proper; `None` when the room is unknown
/// or the point is outside both.
pub(crate) fn destination_sector_key(ir: &Ir, to: &Destination) -> Option<(usize, Option<usize>)> {
    let room_idx = ir.rooms.iter().position(|r| r.id == to.room)?;
    let room = &ir.rooms[room_idx];
    let pad = ir
        .teleports
        .iter()
        .enumerate()
        .filter(|(_, t)| t.room == to.room)
        .find(|(_, t)| {
            pad_square(room, t.pad).is_some_and(|(lo, hi)| square_contains(lo, hi, to.at))
        })
        .map(|(i, _)| i);
    if pad.is_none() && !contains(&room.footprint, to.at) {
        return None;
    }
    Some((room_idx, pad))
}

#[cfg(test)]
mod tests {
    use super::{ExitTrigger, Ir, IrError, PortalKind, Pt, destination_sector_key, pad_square};

    // Room `b` sits a full grid step (64 units, a clean multiple of
    // `Ir::MIN_PORTAL_GAP`) east of room `a`'s own east wall (still at
    // x = 256, so `at` is unchanged from the pre-gap-model fixture): rooms
    // are authored apart now, not flush.
    const TWO_ROOM: &str = r#"{
      "seed": 1, "grid": 64, "theme": "tech_base",
      "rooms": [
        { "id": "a", "footprint": [[0,0],[0,256],[256,256],[256,0]],
          "floor": 0, "ceiling": 128, "light": 160,
          "floor_tex": "FLOOR4_8", "ceil_tex": "CEIL3_5", "wall_tex": "STARTAN3",
          "things": [{ "kind": "player1_start", "at": [128,128], "angle": 90 }] },
        { "id": "b", "footprint": [[320,0],[320,256],[576,256],[576,0]],
          "floor": 0, "ceiling": 128, "light": 160,
          "floor_tex": "FLOOR4_8", "ceil_tex": "CEIL3_5", "wall_tex": "STARTAN3",
          "things": [] }
      ],
      "portals": [
        { "a": "a", "b": "b", "kind": "plain", "width": 128, "at": [256,128] }
      ]
    }"#;

    #[test]
    fn parses_a_two_room_graph() {
        let ir = Ir::from_json(TWO_ROOM).expect("ir parses");
        assert_eq!(ir.rooms.len(), 2);
        assert_eq!(ir.portals.len(), 1);
        assert_eq!(ir.portals[0].kind, PortalKind::Plain);
        assert_eq!(ir.room("b").expect("room b").floor, 0);
        assert_eq!(ir.rooms[0].things[0].kind, "player1_start");
    }

    #[test]
    fn rejects_duplicate_room_ids() {
        let dup = TWO_ROOM.replace("\"id\": \"b\"", "\"id\": \"a\"");
        assert!(matches!(
            Ir::from_json(&dup),
            Err(IrError::DuplicateRoom { .. })
        ));
    }

    #[test]
    fn rejects_a_portal_naming_an_unknown_room() {
        let bad = TWO_ROOM.replace("\"b\": \"b\"", "\"b\": \"ghost\"");
        assert!(matches!(
            Ir::from_json(&bad),
            Err(IrError::UnknownRoom { .. })
        ));
    }

    #[test]
    fn rejects_off_grid_coordinates() {
        let off = TWO_ROOM.replace("[0,256]", "[0,250]");
        assert!(matches!(Ir::from_json(&off), Err(IrError::OffGrid { .. })));
    }

    #[test]
    fn rejects_a_non_positive_grid_instead_of_panicking() {
        // `x % 0` panics in Rust, so this must be caught before the room walk.
        let zero = TWO_ROOM.replace("\"grid\": 64", "\"grid\": 0");
        assert!(matches!(
            Ir::from_json(&zero),
            Err(IrError::InvalidGrid { grid: 0 })
        ));
        let negative = TWO_ROOM.replace("\"grid\": 64", "\"grid\": -64");
        assert!(matches!(
            Ir::from_json(&negative),
            Err(IrError::InvalidGrid { grid: -64 })
        ));
    }

    #[test]
    fn rejects_malformed_json() {
        assert!(matches!(Ir::from_json("{ not json"), Err(IrError::Json(_))));
    }

    #[test]
    fn rejects_a_zero_or_negative_portal_width() {
        // Width 0 emits a two-sided linedef whose two vertices are the same
        // point; a negative width inverts the opening so `open_lo > open_hi`.
        // Both compiled clean before this guard existed — the same class of
        // hole as the unguarded `grid` above.
        let zero = TWO_ROOM.replace("\"width\": 128", "\"width\": 0");
        assert!(matches!(
            Ir::from_json(&zero),
            Err(IrError::InvalidPortalWidth { width: 0, .. })
        ));
        let negative = TWO_ROOM.replace("\"width\": 128", "\"width\": -64");
        assert!(matches!(
            Ir::from_json(&negative),
            Err(IrError::InvalidPortalWidth { width: -64, .. })
        ));
    }

    #[test]
    fn rejects_an_odd_portal_width() {
        // `width / 2` truncates, so an odd width silently emitted a span one
        // unit narrower than the value P3 validates.
        let odd = TWO_ROOM.replace("\"width\": 128", "\"width\": 63");
        assert!(matches!(
            Ir::from_json(&odd),
            Err(IrError::OddPortalWidth { width: 63, .. })
        ));
    }

    /// Two rooms whose facing walls are separated by exactly `gap` units, on
    /// a grid fine enough to express any `gap` from 0 upward.
    fn ir_with_gap(gap: i32) -> String {
        format!(
            r#"{{ "seed":1, "grid":4, "theme":"tech_base",
              "rooms":[
                {{ "id":"a", "footprint":[[0,0],[0,64],[64,64],[64,0]],
                   "floor":0, "ceiling":128, "light":160,
                   "floor_tex":"F", "ceil_tex":"C", "wall_tex":"W" }},
                {{ "id":"b", "footprint":[[{},0],[{},64],[{},64],[{},0]],
                   "floor":0, "ceiling":128, "light":160,
                   "floor_tex":"F", "ceil_tex":"C", "wall_tex":"W" }}
              ],
              "portals":[{{ "a":"a", "b":"b", "kind":"plain", "width":32, "at":[64,32] }}] }}"#,
            64 + gap,
            64 + gap,
            64 + gap + 64,
            64 + gap + 64,
        )
    }

    #[test]
    fn rejects_two_rooms_authored_flush_with_no_gap_at_all() {
        // Gap 0 is exactly the zero-thickness construction this model
        // exists to forbid — the two walls are still recognized as facing
        // each other (coincident, opposite winding), just with an invalid
        // gap, rather than falling through to a generic "not adjacent"
        // error that would misdescribe a wall that is plainly there.
        assert!(matches!(
            Ir::from_json(&ir_with_gap(0)),
            Err(IrError::InvalidPortalGap { gap: 0, .. })
        ));
    }

    #[test]
    fn rejects_a_gap_narrower_than_the_minimum() {
        assert!(matches!(
            Ir::from_json(&ir_with_gap(4)),
            Err(IrError::InvalidPortalGap { gap: 4, .. })
        ));
    }

    #[test]
    fn rejects_a_gap_that_is_not_a_multiple_of_the_minimum() {
        // 12 clears the minimum (8) but is not a whole multiple of it.
        assert!(matches!(
            Ir::from_json(&ir_with_gap(12)),
            Err(IrError::InvalidPortalGap { gap: 12, .. })
        ));
    }

    #[test]
    fn accepts_a_gap_at_exactly_the_minimum() {
        assert!(
            Ir::from_json(&ir_with_gap(8)).is_ok(),
            "8 is the minimum, not a rejection boundary"
        );
    }

    #[test]
    fn accepts_a_gap_that_is_a_larger_multiple_of_the_minimum() {
        assert!(Ir::from_json(&ir_with_gap(16)).is_ok());
    }

    #[test]
    fn a_portal_whose_at_does_not_land_on_any_facing_span_is_not_gap_checked() {
        // `at` sits on room a's own wall coordinate but outside the wall's
        // along-range entirely (y = 128, past the 0..64 wall run) — no
        // facing span matches, so gap validation has nothing to check and
        // must not itself raise `InvalidPortalGap`; the later, more
        // specific `CompileError::PortalOffWall` is `compile`'s job, not
        // `Ir::from_json`'s.
        let json = ir_with_gap(8).replace("\"at\":[64,32]", "\"at\":[64,128]");
        assert!(
            !matches!(Ir::from_json(&json), Err(IrError::InvalidPortalGap { .. })),
            "no facing span matched, so the gap check must stay silent"
        );
    }

    /// Two rooms facing each other across a legal, grid-aligned gap of
    /// `gap` units (a fine grid of 4, so any `gap` from 8 upward is
    /// expressible), with a door portal whose optional door-only fields are
    /// injected verbatim — `""` for a field to omit entirely.
    fn ir_with_door(gap: i32, door_fields: &str) -> String {
        format!(
            r#"{{ "seed":1, "grid":4, "theme":"tech_base",
              "rooms":[
                {{ "id":"a", "footprint":[[0,0],[0,64],[64,64],[64,0]],
                   "floor":0, "ceiling":128, "light":160,
                   "floor_tex":"F", "ceil_tex":"C", "wall_tex":"W" }},
                {{ "id":"b", "footprint":[[{},0],[{},64],[{},64],[{},0]],
                   "floor":0, "ceiling":128, "light":160,
                   "floor_tex":"F", "ceil_tex":"C", "wall_tex":"W" }}
              ],
              "portals":[{{ "a":"a", "b":"b", "kind":"door", "width":32, "at":[64,32]{door_fields} }}] }}"#,
            64 + gap,
            64 + gap,
            64 + gap + 64,
            64 + gap + 64,
        )
    }

    #[test]
    fn a_door_portal_missing_door_thickness_is_rejected() {
        assert!(matches!(
            Ir::from_json(&ir_with_door(32, "")),
            Err(IrError::MissingDoorThickness { .. })
        ));
    }

    #[test]
    fn a_door_portal_with_door_thickness_matching_the_gap_is_accepted() {
        assert!(Ir::from_json(&ir_with_door(32, r#", "door_thickness":32"#)).is_ok());
    }

    #[test]
    fn a_door_thickness_not_8_16_or_32_is_rejected() {
        // 24 clears every other bound (positive, less than the gap) but is
        // not one of the three enumerated sizes.
        let err = Ir::from_json(&ir_with_door(32, r#", "door_thickness":24"#))
            .expect_err("24 is not a legal door_thickness");
        assert!(matches!(
            err,
            IrError::InvalidDoorDimension {
                field: "door_thickness",
                value: 24,
                ..
            }
        ));
    }

    #[test]
    fn an_alcove_length_not_8_16_or_32_is_rejected() {
        let near = Ir::from_json(&ir_with_door(
            64,
            r#", "door_thickness":32, "alcove_near":24"#,
        ))
        .expect_err("24 is not a legal alcove_near");
        assert!(matches!(
            near,
            IrError::InvalidDoorDimension {
                field: "alcove_near",
                value: 24,
                ..
            }
        ));
        let far = Ir::from_json(&ir_with_door(
            64,
            r#", "door_thickness":32, "alcove_far":24"#,
        ))
        .expect_err("24 is not a legal alcove_far");
        assert!(matches!(
            far,
            IrError::InvalidDoorDimension {
                field: "alcove_far",
                value: 24,
                ..
            }
        ));
    }

    #[test]
    fn a_plain_portal_setting_a_door_field_is_rejected() {
        let json = ir_with_door(32, r#", "door_thickness":32"#).replace("\"door\"", "\"plain\"");
        assert!(matches!(
            Ir::from_json(&json),
            Err(IrError::DoorFieldsOnPlainPortal {
                field: "door_thickness",
                ..
            })
        ));
    }

    #[test]
    fn a_plain_portal_setting_an_alcove_alone_is_also_rejected() {
        // Pins that the check covers each of the three door-only fields
        // independently, not just `door_thickness` — a mutation that only
        // checked one field would still pass the test above.
        let json = ir_with_door(32, r#", "alcove_near":16"#).replace("\"door\"", "\"plain\"");
        assert!(matches!(
            Ir::from_json(&json),
            Err(IrError::DoorFieldsOnPlainPortal {
                field: "alcove_near",
                ..
            })
        ));
    }

    #[test]
    fn a_door_gap_wider_than_thickness_plus_alcoves_is_rejected() {
        // The task that requested alcoves phrased the rule as "at least" —
        // gap >= door_thickness + alcove_near + alcove_far. That is unsound:
        // a gap wider than the sum leaves a stretch of the corridor with no
        // sector to fill it, disconnecting whatever lies beyond the
        // shortfall (see the door-redesign report). `Ir::from_json`
        // therefore requires exact equality — a gap of 64 with only 32
        // units of door_thickness/alcove declared (32 short) is rejected,
        // not silently accepted as "at least enough".
        let err = Ir::from_json(&ir_with_door(64, r#", "door_thickness":32"#))
            .expect_err("a 64-unit gap with only 32 units of chain declared must be rejected");
        assert!(matches!(
            err,
            IrError::DoorGapMismatch {
                gap: 64,
                door_thickness: 32,
                alcove_near: 0,
                alcove_far: 0,
                needed: 32,
                ..
            }
        ));
    }

    #[test]
    fn a_door_gap_narrower_than_thickness_plus_alcoves_is_also_rejected() {
        let err = Ir::from_json(&ir_with_door(
            32,
            r#", "door_thickness":32, "alcove_near":16"#,
        ))
        .expect_err("a 32-unit gap cannot hold a 48-unit chain");
        assert!(matches!(
            err,
            IrError::DoorGapMismatch {
                gap: 32,
                needed: 48,
                ..
            }
        ));
    }

    #[test]
    fn a_locked_portal_also_requires_the_gap_to_match_exactly() {
        // `validate_door_gap` reads `Portal::door_thickness` directly,
        // unconditional on `kind` beyond it being `Some` — this pins that a
        // `Locked` portal (not just `Door`) is checked too.
        let json = ir_with_door(64, r#", "lock":"blue_card", "door_thickness":32"#)
            .replace("\"kind\":\"door\"", "\"kind\":\"locked\"");
        assert!(matches!(
            Ir::from_json(&json),
            Err(IrError::DoorGapMismatch { .. })
        ));
    }

    #[test]
    fn track_lower_unpegged_defaults_to_true_and_can_be_disabled() {
        let default_on = Ir::from_json(&ir_with_door(32, r#", "door_thickness":32"#)).expect("ir");
        assert!(
            default_on.portals[0].track_lower_unpegged,
            "absent track_lower_unpegged defaults to true"
        );
        let disabled = Ir::from_json(&ir_with_door(
            32,
            r#", "door_thickness":32, "track_lower_unpegged":false"#,
        ))
        .expect("ir");
        assert!(
            !disabled.portals[0].track_lower_unpegged,
            "explicit false is honored"
        );
    }

    #[test]
    fn rejects_a_room_whose_ceiling_is_not_above_its_floor() {
        let flat = TWO_ROOM.replace("\"ceiling\": 128", "\"ceiling\": 0");
        assert!(matches!(
            Ir::from_json(&flat),
            Err(IrError::InvertedRoom {
                floor: 0,
                ceiling: 0,
                ..
            })
        ));
        let inverted = TWO_ROOM.replace("\"ceiling\": 128", "\"ceiling\": -8");
        assert!(matches!(
            Ir::from_json(&inverted),
            Err(IrError::InvertedRoom { ceiling: -8, .. })
        ));
    }

    #[test]
    fn rejects_coordinates_and_heights_outside_the_binary_map_range() {
        let far = TWO_ROOM.replace("[576,256]", "[65536,256]");
        assert!(matches!(
            Ir::from_json(&far),
            Err(IrError::CoordinateOutOfRange { x: 65536, .. })
        ));
        let high = TWO_ROOM.replace("\"ceiling\": 128", "\"ceiling\": 40000");
        assert!(matches!(
            Ir::from_json(&high),
            Err(IrError::HeightOutOfRange { height: 40000, .. })
        ));
        let off_wall = TWO_ROOM.replace("\"at\": [256,128]", "\"at\": [256,40000]");
        assert!(matches!(
            Ir::from_json(&off_wall),
            Err(IrError::CoordinateOutOfRange { y: 40000, .. })
        ));
    }

    #[test]
    fn rejects_a_locked_portal_that_names_no_key() {
        let locked = TWO_ROOM.replace("\"kind\": \"plain\"", "\"kind\": \"locked\"");
        assert!(matches!(
            Ir::from_json(&locked),
            Err(IrError::LockedWithoutKey { .. })
        ));
        let keyed = TWO_ROOM.replace(
            "\"kind\": \"plain\"",
            "\"kind\": \"locked\", \"lock\": \"blue_card\", \"door_thickness\": 32, \
             \"alcove_near\": 16, \"alcove_far\": 16",
        );
        assert!(Ir::from_json(&keyed).is_ok(), "a named key is accepted");
    }

    /// `TWO_ROOM` with an `exits` array spliced in ahead of `portals`,
    /// naming room `a`'s south wall (y = 0, x in 0..256).
    fn with_exit(exit_json: &str) -> String {
        TWO_ROOM.replace(
            "\"portals\": [",
            &format!("\"exits\": [{exit_json}], \"portals\": ["),
        )
    }

    #[test]
    fn parses_an_exit_and_defaults_secret_to_false() {
        let json = with_exit(r#"{ "room":"a", "trigger":"switch", "width":64, "at":[128,0] }"#);
        let ir = Ir::from_json(&json).expect("ir");
        assert_eq!(ir.exits.len(), 1);
        assert!(!ir.exits[0].secret, "secret defaults to false");
        assert_eq!(ir.exits[0].trigger, crate::ir::ExitTrigger::Switch);
    }

    #[test]
    fn rejects_an_exit_naming_an_unknown_room() {
        let json = with_exit(r#"{ "room":"ghost", "trigger":"switch", "width":64, "at":[128,0] }"#);
        assert!(matches!(
            Ir::from_json(&json),
            Err(IrError::ExitUnknownRoom { .. })
        ));
    }

    #[test]
    fn rejects_a_zero_or_negative_exit_width() {
        let zero = with_exit(r#"{ "room":"a", "trigger":"switch", "width":0, "at":[128,0] }"#);
        assert!(matches!(
            Ir::from_json(&zero),
            Err(IrError::InvalidExitWidth { width: 0, .. })
        ));
        let negative =
            with_exit(r#"{ "room":"a", "trigger":"switch", "width":-64, "at":[128,0] }"#);
        assert!(matches!(
            Ir::from_json(&negative),
            Err(IrError::InvalidExitWidth { width: -64, .. })
        ));
    }

    #[test]
    fn rejects_an_odd_exit_width() {
        let odd = with_exit(r#"{ "room":"a", "trigger":"switch", "width":63, "at":[128,0] }"#);
        assert!(matches!(
            Ir::from_json(&odd),
            Err(IrError::OddExitWidth { width: 63, .. })
        ));
    }

    #[test]
    fn rejects_an_exit_coordinate_outside_the_binary_map_range() {
        let json =
            with_exit(r#"{ "room":"a", "trigger":"walkover", "width":64, "at":[128,40000] }"#);
        assert!(matches!(
            Ir::from_json(&json),
            Err(IrError::CoordinateOutOfRange { y: 40000, .. })
        ));
    }

    #[test]
    fn rejects_a_room_that_sets_both_secret_and_an_explicit_special() {
        let json = TWO_ROOM.replace(
            "\"id\": \"a\", \"footprint\"",
            "\"id\": \"a\", \"secret\": true, \"special\": 9, \"footprint\"",
        );
        assert!(matches!(
            Ir::from_json(&json),
            Err(IrError::SecretWithExplicitSpecial { .. })
        ));
    }

    #[test]
    fn a_room_may_set_secret_alone_or_special_alone() {
        let secret_only = TWO_ROOM.replace(
            "\"id\": \"a\", \"footprint\"",
            "\"id\": \"a\", \"secret\": true, \"footprint\"",
        );
        assert!(Ir::from_json(&secret_only).is_ok(), "secret alone is fine");

        let special_only = TWO_ROOM.replace(
            "\"id\": \"a\", \"footprint\"",
            "\"id\": \"a\", \"special\": 9, \"footprint\"",
        );
        assert!(
            Ir::from_json(&special_only).is_ok(),
            "special alone is fine"
        );
    }

    #[test]
    fn a_thing_with_no_skills_key_defaults_to_all_five() {
        let ir = Ir::from_json(TWO_ROOM).expect("ir");
        let skills = ir.rooms[0].things[0].skills;
        assert!(skills.skill1 && skills.skill2 && skills.skill3 && skills.skill4 && skills.skill5);
    }

    #[test]
    fn a_thing_may_partially_specify_skills_defaulting_the_rest_true() {
        let json = TWO_ROOM.replace(
            "\"kind\": \"player1_start\", \"at\": [128,128], \"angle\": 90",
            "\"kind\": \"player1_start\", \"at\": [128,128], \"angle\": 90, \
             \"skills\": { \"skill1\": false, \"skill5\": false }",
        );
        let ir = Ir::from_json(&json).expect("ir");
        let skills = ir.rooms[0].things[0].skills;
        assert!(!skills.skill1, "explicitly excluded");
        assert!(
            skills.skill2 && skills.skill3 && skills.skill4,
            "unmentioned keys default true"
        );
        assert!(!skills.skill5, "explicitly excluded");
    }

    const TELEPORT_BASE: &str = r#"{ "seed":1, "grid":64, "theme":"tech_base",
      "rooms":[
        { "id":"a", "footprint":[[0,0],[0,256],[256,256],[256,0]],
          "floor":0, "ceiling":128, "light":160,
          "floor_tex":"FLOOR4_8", "ceil_tex":"CEIL3_5", "wall_tex":"STARTAN3",
          "things":[ { "kind":"player1_start", "at":[192,64], "angle":90, "ambush":true } ] },
        { "id":"b", "footprint":[[320,0],[320,256],[576,256],[576,0]],
          "floor":0, "ceiling":128, "light":160,
          "floor_tex":"FLOOR4_8", "ceil_tex":"CEIL3_5", "wall_tex":"STARTAN3" }
      ],
      "portals":[],
      "teleports":[ TELEPORTS ] }"#;

    fn with_teleports(list: &str) -> Result<Ir, IrError> {
        Ir::from_json(&TELEPORT_BASE.replace("TELEPORTS", list))
    }

    const ISLAND: &str = r#"{ "id":"t1", "room":"a", "pad":{"island":[64,192]},
        "to":{"room":"b","at":[448,128],"angle":90} }"#;

    #[test]
    fn a_teleport_parses_with_defaults_and_the_pad_square_is_64_wide() {
        let ir = with_teleports(ISLAND).expect("parses");
        let t = &ir.teleports[0];
        assert!(
            t.repeatable && !t.monsters_only,
            "defaults: repeatable, any thing"
        );
        assert!(ir.rooms[0].things[0].ambush, "the ambush flag parses");
        let (lo, hi) = pad_square(&ir.rooms[0], t.pad).expect("island square");
        assert_eq!((lo, hi), (Pt { x: 32, y: 160 }, Pt { x: 96, y: 224 }));
        assert_eq!(Ir::PAD_SIZE, 64);
        assert_eq!(Ir::PAD_FLOOR_STEP, 8);
    }

    #[test]
    fn a_wall_pad_square_is_recessed_outward_from_the_wall() {
        let ir = with_teleports(
            r#"{ "id":"w", "room":"a", "pad":{"wall":[64,256]},
                 "to":{"room":"b","at":[448,128],"angle":90} }"#,
        )
        .expect("parses");
        let (lo, hi) = pad_square(&ir.rooms[0], ir.teleports[0].pad).expect("wall square");
        assert_eq!(
            (lo, hi),
            (Pt { x: 32, y: 256 }, Pt { x: 96, y: 320 }),
            "north wall, recess to +y"
        );
    }

    #[test]
    fn a_wall_pad_off_any_wall_is_rejected() {
        let err = with_teleports(
            r#"{ "id":"w", "room":"a", "pad":{"wall":[64,64]},
                 "to":{"room":"b","at":[448,128],"angle":90} }"#,
        )
        .unwrap_err();
        assert!(matches!(err, IrError::TeleportPadOffWall { .. }), "{err}");
    }

    #[test]
    fn an_island_pad_touching_the_room_wall_is_rejected() {
        let err = with_teleports(
            r#"{ "id":"t", "room":"a", "pad":{"island":[32,128]},
                 "to":{"room":"b","at":[448,128],"angle":90} }"#,
        )
        .unwrap_err();
        assert!(
            matches!(err, IrError::TeleportPadOutsideRoom { .. }),
            "{err}"
        );
    }

    #[test]
    fn an_off_grid_island_center_is_rejected() {
        let err = with_teleports(
            r#"{ "id":"t", "room":"a", "pad":{"island":[70,192]},
                 "to":{"room":"b","at":[448,128],"angle":90} }"#,
        )
        .unwrap_err();
        assert!(matches!(err, IrError::TeleportPadOffGrid { .. }), "{err}");
    }

    #[test]
    fn two_island_pads_touching_each_other_are_rejected() {
        let err = with_teleports(
            r#"{ "id":"t1", "room":"a", "pad":{"island":[64,192]}, "to":{"room":"b","at":[448,128],"angle":90} },
               { "id":"t2", "room":"a", "pad":{"island":[128,192]}, "to":{"room":"b","at":[384,128],"angle":90} }"#,
        )
        .unwrap_err();
        assert!(matches!(err, IrError::TeleportPadsOverlap { .. }), "{err}");
    }

    #[test]
    fn duplicate_ids_and_unknown_rooms_are_rejected() {
        let err = with_teleports(&format!("{ISLAND}, {ISLAND}")).unwrap_err();
        assert!(matches!(err, IrError::DuplicateTeleport { .. }), "{err}");
        let err = with_teleports(
            r#"{ "id":"t", "room":"zzz", "pad":{"island":[64,192]}, "to":{"room":"b","at":[448,128],"angle":90} }"#,
        )
        .unwrap_err();
        assert!(matches!(err, IrError::TeleportUnknownRoom { .. }), "{err}");
    }

    #[test]
    fn a_destination_outside_its_room_is_rejected() {
        let err = with_teleports(
            r#"{ "id":"t", "room":"a", "pad":{"island":[64,192]}, "to":{"room":"b","at":[100,100],"angle":90} }"#,
        )
        .unwrap_err();
        assert!(
            matches!(err, IrError::TeleportDestinationOutsideRoom { .. }),
            "{err}"
        );
    }

    #[test]
    fn distinct_destinations_in_one_sector_are_rejected_but_identical_ones_share() {
        let err = with_teleports(
            r#"{ "id":"t1", "room":"a", "pad":{"island":[64,192]}, "to":{"room":"b","at":[448,128],"angle":90} },
               { "id":"t2", "room":"a", "pad":{"island":[192,192]}, "to":{"room":"b","at":[384,64],"angle":0} }"#,
        )
        .unwrap_err();
        assert!(
            matches!(err, IrError::TeleportDestinationsShareSector { .. }),
            "{err}"
        );
        with_teleports(
            r#"{ "id":"t1", "room":"a", "pad":{"island":[64,192]}, "to":{"room":"b","at":[448,128],"angle":90} },
               { "id":"t2", "room":"a", "pad":{"island":[192,192]}, "to":{"room":"b","at":[448,128],"angle":90} }"#,
        )
        .expect("identical destinations share one marker");
    }

    #[test]
    fn a_destination_on_another_pad_keys_to_that_pad() {
        let ir = with_teleports(
            r#"{ "id":"t1", "room":"a", "pad":{"island":[64,192]}, "to":{"room":"b","at":[448,128],"angle":90} },
               { "id":"t2", "room":"b", "pad":{"island":[448,128]}, "to":{"room":"a","at":[64,192],"angle":0} }"#,
        )
        .expect("a two-way pair");
        assert_eq!(
            destination_sector_key(&ir, &ir.teleports[0].to),
            Some((1, Some(1)))
        );
        assert_eq!(
            destination_sector_key(&ir, &ir.teleports[1].to),
            Some((0, Some(0)))
        );
    }

    #[test]
    fn the_teleport_exit_trigger_parses() {
        let ir = Ir::from_json(&TELEPORT_BASE.replace("TELEPORTS", ISLAND).replace(
            r#""portals":[],"#,
            r#""portals":[], "exits":[{ "room":"b", "trigger":"teleport", "at":[448,256], "width":64 }],"#,
        ))
        .expect("parses");
        assert_eq!(ir.exits[0].trigger, ExitTrigger::Teleport);
    }
}
