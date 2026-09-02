//! The room-graph intermediate representation.

use std::collections::{HashMap, HashSet};

use serde::Deserialize;

use crate::geom::{
    Axis, Pt, clearance, contains, edges, facing_spans, find_facing_span, outward_sign,
    segment_enters_open_rect, wall_edges,
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
    /// A `downWaitUpStay` platform filling the wall gap: a lift when the rooms'
    /// floors differ (rest floor = the higher room's), a barrier when they are
    /// equal (rest floor = floor + [`Portal::rise`]). Rests high; travels to
    /// the lowest neighbor and back (`p_plats.c`, `EV_DoPlat`).
    Lift,
    /// A sealed wall sector filling the gap between two rooms, lowered once
    /// to the lower room's floor by the trigger [`Portal::fires_on`] names —
    /// `lowerFloorToLowest`. Rests with its floor equal to its ceiling at the
    /// lower of the two rooms' ceilings, so it reads as solid rock until it
    /// fires; its depth along the gap is [`Portal::thickness`].
    ///
    /// The lowered wall only joins the two rooms if their floors are within
    /// a step of each other, which [`Ir::from_json`] does not check: the step
    /// height is an engine-table constant and IR validation loads no table
    /// (see [`Ir::FLAT_TILE`]), so that comparison belongs to compilation.
    DropWall,
    /// A pit strip filling the gap between two rooms at one floor, resting
    /// [`Portal::depth`] below them and raised once to their floor by the
    /// trigger [`Portal::fires_on`] names — `raiseFloorToNearest`.
    Bridge,
}

/// A lift's speed: `downWaitUpStay` (62/88) or `blazeDWUS` (123/120).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LiftSpeed {
    /// `downWaitUpStay` (62/88).
    #[default]
    Normal,
    /// `blazeDWUS` (123/120).
    Fast,
}

/// Where a lift's trigger lines go — the template's three words, given the
/// corpus's meaning (`docs/measurements/lift-shapes-2026-08-29.md` §F).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LiftTrigger {
    /// A use special on the platform's low face; the riser is the switch.
    #[default]
    Switch,
    /// A walkover special on the outer threshold of the low room's alcove.
    Walkover,
    /// `Switch` plus a walkover special on the top face.
    BothEnds,
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
/// [`Self::door_thickness`] only applies to [`PortalKind::Door`]/
/// [`PortalKind::Locked`] — a [`PortalKind::Plain`] portal has no door and so
/// no thickness of its own, and [`Ir::from_json`] rejects one that sets it
/// (see [`IrError::DoorFieldsOnPlainPortal`]); a [`PortalKind::Lift`] is
/// rejected too, but with the more specific [`IrError::DoorThicknessOnLift`],
/// since a lift's own platform sector fills the gap and admits no separate
/// door construction. [`Self::alcove_near`] and [`Self::alcove_far`] are
/// broader: they apply to [`PortalKind::Door`]/[`PortalKind::Locked`] *and*
/// [`PortalKind::Lift`] (a lift's alcove is a buffer, or — on a walkover
/// trigger — the strip that carries the trigger line), so only
/// [`PortalKind::Plain`] rejects them. Rejecting rather than silently
/// ignoring values that would do nothing matches the reject-don't-degrade
/// posture [`Self::width`] already takes on an odd value.
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
    /// otherwise). Meaningless (and rejected) on a [`PortalKind::Plain`]
    /// portal, and on a [`PortalKind::Lift`]
    /// ([`IrError::DoorThicknessOnLift`]).
    #[serde(default)]
    pub door_thickness: Option<i32>,
    /// An optional buffer sector between room `a` and the door or platform,
    /// in map units.
    ///
    /// When present, must be one of [`Ir::DOOR_DIMENSIONS`] on a
    /// [`PortalKind::Door`]/[`PortalKind::Locked`] portal
    /// ([`IrError::InvalidDoorDimension`] otherwise) and one of the wider
    /// [`Ir::LIFT_ALCOVE_DIMENSIONS`] on a [`PortalKind::Lift`]
    /// ([`IrError::InvalidLiftAlcoveDimension`] otherwise). Meaningless (and
    /// rejected) on a [`PortalKind::Plain`] portal. On a [`PortalKind::Lift`]
    /// this is a buffer, or, when the low room is `a` and [`Self::trigger`]
    /// is [`LiftTrigger::Walkover`], the strip that carries the walkover
    /// trigger line ([`IrError::LiftWalkoverNeedsAlcove`] otherwise) —
    /// [`LiftTrigger::BothEnds`] puts its second line on the platform's top
    /// face, not in an alcove, so it needs none. See
    /// [`Self::alcove_far`] for the naming rationale — "near" and "far" name
    /// room `a`'s and room `b`'s own walls, mirroring the compiler's
    /// internal `near`/`far` facing-wall terminology, not a "front"/"behind"
    /// the task that requested this named ambiguously (a corridor is walked
    /// in both directions, so "in front of the door" has no fixed meaning
    /// without picking a travel direction).
    #[serde(default)]
    pub alcove_near: Option<i32>,
    /// An optional buffer sector between the door or platform and room `b`,
    /// in map units. See [`Self::alcove_near`] for the shared constraints
    /// and the near/far naming rationale.
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
    /// Lift speed; meaningless on other kinds (ignored, like a default).
    #[serde(default)]
    pub speed: LiftSpeed,
    /// Lift trigger placement; meaningless on other kinds.
    #[serde(default)]
    pub trigger: LiftTrigger,
    /// Barrier only: how far the platform rests above the two rooms' shared
    /// floor. Required when the floors are equal, rejected when they differ,
    /// and rejected outright on any portal that is not
    /// [`PortalKind::Lift`] ([`IrError::RiseOnNonLiftPortal`]) — unlike
    /// [`Self::speed`]/[`Self::trigger`], which a non-lift kind simply
    /// ignores, this follows the reject-don't-degrade posture
    /// [`Self::door_thickness`] already takes on a plain or locked portal.
    #[serde(default)]
    pub rise: Option<i32>,
    /// Drop wall only: the wall's own depth along the gap, one of
    /// [`Ir::DROP_WALL_THICKNESS`]. Required on a [`PortalKind::DropWall`]
    /// ([`IrError::MissingDropWallThickness`]), rejected on every other kind
    /// ([`IrError::FloorFieldOnOtherPortal`]).
    #[serde(default)]
    pub thickness: Option<i32>,
    /// Bridge only: how far the pit rests below the two rooms' shared floor,
    /// a positive multiple of [`Ir::BRIDGE_DEPTH_STEP`]. Required on a
    /// [`PortalKind::Bridge`] ([`IrError::MissingBridgeDepth`]), rejected on
    /// every other kind ([`IrError::FloorFieldOnOtherPortal`]).
    ///
    /// A pit no deeper than a step would be a bridge not worth raising, but
    /// the step height is a table constant IR validation never loads, so that
    /// half of the rule belongs to compilation — as for
    /// [`PortalKind::DropWall`]'s two floors.
    #[serde(default)]
    pub depth: Option<i32>,
    /// Drop wall and bridge: the id of the [`Trigger`] that fires it.
    /// Required on both ([`IrError::ConstructWithoutTrigger`]) and rejected
    /// on every other kind ([`IrError::FloorFieldOnOtherPortal`]).
    ///
    /// Named `fires_on` rather than `trigger` because [`Self::trigger`] is
    /// already taken: on a lift that word names where the trigger line is
    /// *placed*, not which trigger fires the portal.
    #[serde(default)]
    pub fires_on: Option<String>,
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
    /// The **low corner** (minimum x, minimum y) of a free-standing pad
    /// inside the room: the square is `[x, x + PAD_SIZE] x [y, y +
    /// PAD_SIZE]`. It must sit on the 64-unit flat grid (see
    /// [`Ir::FLAT_TILE`]) and the whole square must lie strictly inside the
    /// footprint.
    Island(Pt),
    /// A point on one of the room's axis-aligned walls, naming the **start**
    /// of the pad's span along that wall (its low `along` coordinate): the
    /// span is `[along, along + PAD_SIZE]`. The pad is recessed
    /// [`Ir::PAD_SIZE`] outward from the wall, exactly as a walkover exit's
    /// alcove is, so the wall's own fixed coordinate is the square's near
    /// edge; both it and the span's start must be multiples of
    /// [`Ir::FLAT_TILE`].
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

/// A raised island inside one room that lowers when used from around it —
/// the corpus's "pedestal" (`AboveAll`, one neighbor at one floor).
#[derive(Debug, Clone, Deserialize)]
pub struct Pedestal {
    /// Unique identifier, used in error messages.
    pub id: String,
    /// Identifier of the room the pedestal sits in.
    pub room: String,
    /// The rectangle's low corner (minimum x, minimum y).
    pub at: Pt,
    /// Width × height; `None` is [`Ir::PEDESTAL_DEFAULT_SIZE`] square.
    #[serde(default)]
    pub size: Option<[i32; 2]>,
    /// Rest floor above the host's, in map units.
    pub rise: i32,
    /// Lift speed the pedestal rides at.
    #[serde(default)]
    pub speed: LiftSpeed,
    /// Things placed on the platform, at the raised floor; each `at` must
    /// fall strictly inside the rectangle.
    #[serde(default)]
    pub things: Vec<IrThing>,
}

impl Pedestal {
    /// The rectangle as `(lo, hi)` corners.
    ///
    /// The high corner saturates rather than overflowing: `size` has no
    /// upper bound of its own below [`IrError::PedestalSizeNotMultipleOf8`],
    /// so `at + size` can exceed `i32` outright on unvalidated input.
    /// [`Ir::from_json`] range-checks `at` and the high corner with checked
    /// arithmetic *before* ever calling this, so a saturated result here
    /// never actually surfaces from validated IR — this only keeps the
    /// method itself panic-free for any [`Pedestal`] `from_json` accepted,
    /// not just one that has already passed that validation.
    #[must_use]
    pub fn rect(&self) -> (Pt, Pt) {
        let [w, h] = self.size.unwrap_or([Ir::PEDESTAL_DEFAULT_SIZE; 2]);
        (
            self.at,
            Pt {
                x: self.at.x.saturating_add(w),
                y: self.at.y.saturating_add(h),
            },
        )
    }
}

/// How a floor-action [`Trigger`] fires.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TriggerKind {
    /// A use line on a room's own wall: `P_UseSpecialLine`, one-shot (S1).
    Switch,
    /// A crossing of a portal's opening line: `P_CrossSpecialLine`, one-shot
    /// (W1), from either side.
    Walkover,
}

/// A placed trigger that one or more floor constructs name by id.
///
/// One trigger is one sector tag and one line special, so every construct
/// naming it moves the same way: all of them lower, or all of them raise
/// ([`IrError::TriggerMixesFamilies`]).
#[derive(Debug, Clone, Deserialize)]
pub struct Trigger {
    /// Unique identifier, named by constructs and used in error messages.
    pub id: String,
    /// How it fires.
    pub kind: TriggerKind,
    /// [`TriggerKind::Switch`] only: the room whose wall carries it.
    #[serde(default)]
    pub room: Option<String>,
    /// [`TriggerKind::Switch`] only: a point on that room's own wall, read
    /// exactly as [`Exit::at`] is — the switch segment is centered there.
    #[serde(default)]
    pub at: Option<Pt>,
    /// [`TriggerKind::Walkover`] only: the two rooms of the portal whose
    /// opening line carries it, in either order.
    #[serde(default)]
    pub portal: Option<[String; 2]>,
}

/// The two rest shapes of a [`Reveal`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RevealKind {
    /// Solid at rest: floor equal to ceiling at the host's ceiling, its
    /// things inside the rock — the monster closet.
    Closet,
    /// A raised block at rest, [`Reveal::rise`] above the host's floor and
    /// under the host's ceiling, its things on top — the prize pedestal.
    Pedestal,
}

/// A sealed island inside one room, lowered once to the host's floor by the
/// trigger it names — `lowerFloorToLowest` with one neighbor.
///
/// The same rectangle a [`Pedestal`] is, placed and validated by the same
/// rules; what differs is that it rests sealed and moves once, on a shared
/// trigger, rather than resting raised and lowering under the player's own
/// use.
#[derive(Debug, Clone, Deserialize)]
pub struct Reveal {
    /// Unique identifier, used in error messages.
    pub id: String,
    /// Identifier of the room the reveal sits in.
    pub room: String,
    /// The rectangle's low corner (minimum x, minimum y).
    pub at: Pt,
    /// Width × height; `None` is [`Ir::PEDESTAL_DEFAULT_SIZE`] square.
    #[serde(default)]
    pub size: Option<[i32; 2]>,
    /// Closet or pedestal.
    pub kind: RevealKind,
    /// [`RevealKind::Pedestal`] only: rest floor above the host's, which must
    /// be positive ([`IrError::RevealRiseNotPositive`]); rejected on a
    /// [`RevealKind::Closet`] ([`IrError::RiseOnCloset`]).
    ///
    /// A pedestal reveal is sealed by height rather than by rock, so it also
    /// has to rise more than a step — the step height is a table constant IR
    /// validation never loads, so that half of the rule belongs to
    /// compilation, as it does for [`Portal::depth`].
    #[serde(default)]
    pub rise: Option<i32>,
    /// Things placed inside the cell, each `at` strictly inside the
    /// rectangle.
    #[serde(default)]
    pub things: Vec<IrThing>,
    /// The id of the [`Trigger`] that lowers it.
    pub trigger: String,
}

impl Reveal {
    /// The rectangle as `(lo, hi)` corners, saturating exactly as
    /// [`Pedestal::rect`] does and for the same reason.
    #[must_use]
    pub fn rect(&self) -> (Pt, Pt) {
        let [w, h] = self.size.unwrap_or([Ir::PEDESTAL_DEFAULT_SIZE; 2]);
        (
            self.at,
            Pt {
                x: self.at.x.saturating_add(w),
                y: self.at.y.saturating_add(h),
            },
        )
    }
}

/// Which way the constructs on one [`Trigger`] move.
///
/// The IR's own word for it: compilation maps this to the engine family the
/// tables name, and the IR only has to know that a bridge rises while a drop
/// wall and a reveal lower.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FloorFamilyIr {
    /// Drop walls and reveals: `lowerFloorToLowest`.
    Lower,
    /// Bridges: `raiseFloorToNearest`.
    Raise,
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
    /// The pedestals: raised islands that lower on use. Absent means none,
    /// so every pre-existing fixture parses unchanged.
    #[serde(default)]
    pub pedestals: Vec<Pedestal>,
    /// The floor-action triggers. Absent means none, so every pre-existing
    /// fixture parses unchanged.
    #[serde(default)]
    pub triggers: Vec<Trigger>,
    /// The reveals: sealed islands lowered once by a trigger. Absent means
    /// none, so every pre-existing fixture parses unchanged.
    #[serde(default)]
    pub reveals: Vec<Reveal>,
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
    /// A [`Portal::door_thickness`], or a
    /// [`Portal::alcove_near`]/[`Portal::alcove_far`] on a
    /// [`PortalKind::Door`]/[`PortalKind::Locked`] portal, is not one of
    /// [`Ir::DOOR_DIMENSIONS`]. A lift's alcove is judged against the wider
    /// [`Ir::LIFT_ALCOVE_DIMENSIONS`] and reports
    /// [`Self::InvalidLiftAlcoveDimension`] instead.
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
    /// A [`PortalKind::Lift`] portal's [`Portal::alcove_near`] or
    /// [`Portal::alcove_far`] value is not one of
    /// [`Ir::LIFT_ALCOVE_DIMENSIONS`].
    ///
    /// Its own variant rather than a widened [`Self::InvalidDoorDimension`]
    /// because the two sets differ — a lift alcove may also be 64 — and one
    /// message naming both would have to lie about whichever kind the caller
    /// actually wrote.
    #[error(
        "portal `{a}` <-> `{b}` is a lift with {field} {value}, which must be 8, 16, 32, or 64 map units"
    )]
    InvalidLiftAlcoveDimension {
        /// The first room.
        a: String,
        /// The second room.
        b: String,
        /// Which field was rejected: `"alcove_near"` or `"alcove_far"`.
        field: &'static str,
        /// The rejected value.
        value: i32,
    },
    /// A [`PortalKind::Plain`] or [`PortalKind::Bridge`] portal sets
    /// [`Portal::door_thickness`], [`Portal::alcove_near`], or
    /// [`Portal::alcove_far`] — fields that only mean something for a door,
    /// and would otherwise be silently ignored.
    #[error(
        "portal `{a}` <-> `{b}` sets a door field ({field}) but has no door; use `door` or `locked`, or remove it"
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
    /// A pad's square does not sit on the 64-unit flat grid.
    #[error(
        "teleport `{id}`: the pad placed at ({at_x}, {at_y}) has its square's low corner at \
         ({x}, {y}), which is not on the 64-unit flat grid; vanilla wraps a flat every 64 units \
         in world space, so a 64x64 pad reads as one tile only when its corners are multiples \
         of 64"
    )]
    TeleportPadOffFlatGrid {
        /// The teleport's identifier.
        id: String,
        /// The X coordinate of the authored point: an island pad's corner,
        /// or a wall pad's span start.
        at_x: i32,
        /// The Y coordinate of the authored point.
        at_y: i32,
        /// The X coordinate of the pad square's low corner. Reported
        /// alongside the authored point because the two differ for a pad
        /// recessed toward -x or -y, where the square's low corner is the
        /// recess's *far* corner and never appears in the IR at all.
        x: i32,
        /// The Y coordinate of the pad square's low corner.
        y: i32,
    },
    /// An island pad's square is not strictly inside its room.
    #[error("teleport `{id}`: the pad at corner ({x}, {y}) does not lie strictly inside its room")]
    TeleportPadOutsideRoom {
        /// The teleport's identifier.
        id: String,
        /// The X coordinate of the pad's low corner.
        x: i32,
        /// The Y coordinate of the pad's low corner.
        y: i32,
    },
    /// A wall pad's point is on no axis-aligned wall of its room, or the
    /// 64-unit span starting there runs past that wall's far end.
    #[error(
        "teleport `{id}`: ({x}, {y}) is not on an axis-aligned wall of its room with 64 units \
         of wall running on from it"
    )]
    TeleportPadOffWall {
        /// The teleport's identifier.
        id: String,
        /// The X coordinate of the span's start on the wall.
        x: i32,
        /// The Y coordinate of the span's start on the wall.
        y: i32,
    },
    /// A wall pad's 64-deep recess would come within [`Ir::MIN_PORTAL_GAP`]
    /// of another room — flush against it, at worst, which would emit two
    /// coincident one-sided linedefs.
    #[error("teleport `{id}`: its recess would come within 8 units of room `{room}`")]
    TeleportPadRecessTooClose {
        /// The teleport's identifier.
        id: String,
        /// The room the recess crowds.
        room: String,
    },
    /// A wall pad's span overlaps or touches another opening cut into the
    /// same wall — a portal's opening or the level exit's segment.
    #[error("teleport `{id}`: its pad span overlaps or touches {opening}")]
    TeleportPadBesideOpening {
        /// The teleport's identifier.
        id: String,
        /// The opening it collides with: ``portal `a` <-> `b` `` or
        /// ``exit in room `r` ``.
        opening: String,
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
    /// A lift or drop-wall portal sets [`Portal::door_thickness`], which
    /// only a door has — the gap is already filled by the platform's or the
    /// wall's own sector.
    #[error("portal `{a}` <-> `{b}` is a lift or drop wall but sets door_thickness")]
    DoorThicknessOnLift {
        /// The first room.
        a: String,
        /// The second room.
        b: String,
    },
    /// A portal that is not [`PortalKind::Lift`] sets [`Portal::rise`],
    /// which only a lift portal has — unlike [`Portal::speed`]/
    /// [`Portal::trigger`], which a non-lift kind simply ignores, this
    /// follows the reject-don't-degrade posture
    /// [`IrError::DoorFieldsOnPlainPortal`] already takes on a door field
    /// set on a plain portal.
    #[error("portal `{a}` <-> `{b}` sets `rise`, which only a lift portal has")]
    RiseOnNonLiftPortal {
        /// The first room.
        a: String,
        /// The second room.
        b: String,
    },
    /// A lift portal whose rooms sit at different floors also sets
    /// [`Portal::rise`], which only means something on a barrier (equal
    /// floors) — an ordinary lift's rest floor is simply the higher room's.
    #[error(
        "portal `{a}` <-> `{b}` joins rooms at different floors, so `rise` is meaningless on it"
    )]
    LiftRiseOnUnequalFloors {
        /// The first room.
        a: String,
        /// The second room.
        b: String,
    },
    /// A lift portal whose rooms sit at one floor (a barrier) names no
    /// [`Portal::rise`], leaving its rest floor undetermined.
    #[error("portal `{a}` <-> `{b}` joins rooms at one floor: a barrier needs `rise`")]
    BarrierMissingRise {
        /// The first room.
        a: String,
        /// The second room.
        b: String,
    },
    /// A [`LiftTrigger::Walkover`] lift's low room names no
    /// [`Portal::alcove_near`]/[`Portal::alcove_far`] (whichever sits on
    /// that room's own wall) to carry the walkover trigger line.
    /// [`LiftTrigger::BothEnds`] never raises this: its second line sits on
    /// the platform's own top face, not in an alcove.
    #[error(
        "portal `{a}` <-> `{b}` is a walkover lift but the low room `{low_room}` has no alcove \
         ({field}) to carry the trigger line"
    )]
    LiftWalkoverNeedsAlcove {
        /// The first room.
        a: String,
        /// The second room.
        b: String,
        /// The identifier of the lift's low room.
        low_room: String,
        /// Which field the low room's alcove would be:
        /// `"alcove_near"` or `"alcove_far"`.
        field: &'static str,
    },
    /// A barrier (a lift portal whose rooms sit at one floor) sets a
    /// [`Portal::trigger`] other than [`LiftTrigger::Switch`] — a barrier has
    /// no low room for a walkover line to sit in front of.
    #[error("portal `{a}` <-> `{b}` is a barrier, which offers only `switch`, not `{trigger}`")]
    BarrierTrigger {
        /// The first room.
        a: String,
        /// The second room.
        b: String,
        /// The rejected trigger: `"walkover"` or `"both_ends"`.
        trigger: &'static str,
    },
    /// Two pedestals share an id.
    #[error("pedestal `{id}` is declared twice")]
    DuplicatePedestal {
        /// The repeated identifier.
        id: String,
    },
    /// A pedestal names a room that does not exist.
    #[error("pedestal `{id}` names unknown room `{room}`")]
    PedestalUnknownRoom {
        /// The pedestal's identifier.
        id: String,
        /// The unresolvable room identifier.
        room: String,
    },
    /// A pedestal's [`Pedestal::rise`] is zero or negative.
    #[error("pedestal `{id}` has rise {rise}; a pedestal rises above its host")]
    PedestalRiseNotPositive {
        /// The pedestal's identifier.
        id: String,
        /// The rejected rise.
        rise: i32,
    },
    /// A pedestal's [`Pedestal::size`] has a side that is not a positive
    /// multiple of 8.
    #[error("pedestal `{id}` is {width}x{height}; each side must be a positive multiple of 8")]
    PedestalSizeNotMultipleOf8 {
        /// The pedestal's identifier.
        id: String,
        /// The rejected width.
        width: i32,
        /// The rejected height.
        height: i32,
    },
    /// A pedestal's rectangle does not lie strictly inside its room.
    #[error("pedestal `{id}` at ({x}, {y}) does not fit strictly inside its room")]
    PedestalOutsideRoom {
        /// The pedestal's identifier.
        id: String,
        /// The X coordinate of the pedestal's low corner.
        x: i32,
        /// The Y coordinate of the pedestal's low corner.
        y: i32,
    },
    /// A thing placed on a pedestal lands outside the pedestal's own
    /// rectangle.
    #[error("pedestal `{id}` places `{kind}` at ({x}, {y}), outside the pedestal's rectangle")]
    PedestalThingOutside {
        /// The pedestal's identifier.
        id: String,
        /// The thing's vocabulary name.
        kind: String,
        /// The X coordinate of the thing.
        x: i32,
        /// The Y coordinate of the thing.
        y: i32,
    },
    /// Two pedestals in one room overlap or touch, or a pedestal and a
    /// teleport pad in one room do — touching squares would emit coincident
    /// linedefs, the same rule [`IrError::TeleportPadsOverlap`] enforces
    /// between two pads.
    #[error("islands `{first}` and `{second}` overlap or touch")]
    PedestalsOverlap {
        /// The first island's identifier (a pedestal or a teleport).
        first: String,
        /// The second island's identifier (a pedestal or a teleport).
        second: String,
    },
    /// A teleport's destination point lies inside, or on the boundary of, a
    /// pedestal's rectangle in the destination's own room.
    ///
    /// Not [`IrError::PedestalsOverlap`]: a destination is a point, not a
    /// square, so the island rule between two rectangles never sees it. The
    /// pedestal's rectangle is its own sector, so the engine would land the
    /// traveler on the raised platform rather than on the floor the author
    /// aimed at — and on the boundary too, where the arrival's own radius
    /// straddles the edge.
    #[error(
        "teleport `{teleport}` delivers onto pedestal `{pedestal}`: a destination inside a pedestal rectangle would arrive on the raised platform"
    )]
    TeleportDestinationOnPedestal {
        /// The teleport whose destination lands on the pedestal.
        teleport: String,
        /// The pedestal it lands on.
        pedestal: String,
    },
    /// A teleport's destination point lies inside, or on the boundary of, a
    /// reveal's rectangle in the destination's own room — the reveal's twin
    /// of [`Self::TeleportDestinationOnPedestal`], and rejected for the same
    /// reason.
    ///
    /// A reveal's rectangle is its own sector, and a sealed one: a closet
    /// rests solid, so the traveler would arrive inside the rock, and a
    /// pedestal reveal rests raised, so the traveler would arrive on top of
    /// the block rather than on the floor the author aimed at.
    #[error(
        "teleport `{teleport}` delivers onto reveal `{reveal}`: a destination inside a reveal rectangle would arrive in the sealed cell"
    )]
    TeleportDestinationOnReveal {
        /// The teleport whose destination lands on the reveal.
        teleport: String,
        /// The reveal it lands on.
        reveal: String,
    },
    /// Two triggers share an id.
    #[error("trigger `{id}` is declared twice")]
    DuplicateTrigger {
        /// The repeated identifier.
        id: String,
    },
    /// A trigger no construct names — a tag and a special that would move
    /// nothing.
    #[error("trigger `{id}` is named by no drop wall, reveal or bridge")]
    TriggerUnused {
        /// The trigger's identifier.
        id: String,
    },
    /// A construct names a trigger that does not exist.
    #[error("{subject} names unknown trigger `{id}`")]
    UnknownTrigger {
        /// The construct that names it: ``portal `a` <-> `b` `` or
        /// ``reveal `id` ``.
        subject: String,
        /// The missing trigger identifier.
        id: String,
    },
    /// One trigger named by both a lowering and a rising construct. One
    /// trigger is one tag and one special, so it can only move one way.
    #[error(
        "trigger `{id}` is named by a bridge and by a drop wall or reveal; one trigger moves one way"
    )]
    TriggerMixesFamilies {
        /// The trigger's identifier.
        id: String,
    },
    /// A trigger that is not placed where its kind can be: a switch whose
    /// point is not on its room's own wall, a switch or walkover that names
    /// the other kind's fields, or a walkover naming a portal that is not
    /// there.
    #[error("trigger `{id}`: {detail}")]
    TriggerOffWall {
        /// The trigger's identifier.
        id: String,
        /// What was wrong with it.
        detail: String,
    },
    /// A walkover trigger names a portal whose opening line cannot carry it.
    ///
    /// A plain portal's opening line can, and so can a bridge's own pit
    /// thresholds — stepping down into the pit is the crossing that raises
    /// it. A door, locked, lift or drop-wall portal cannot.
    #[error("trigger `{id}` names portal `{a}` <-> `{b}`, which is not a plain portal or a bridge")]
    WalkoverOnNonPlainPortal {
        /// The trigger's identifier.
        id: String,
        /// The first room.
        a: String,
        /// The second room.
        b: String,
    },
    /// A walkover trigger's `portal: [a, b]` names more than one portal, so
    /// which opening line would carry the special is undetermined.
    ///
    /// Two portals may legally join one room pair — a second opening in the
    /// same wall is refused only when its span *overlaps* the first's — so
    /// `[a, b]` is not by itself the name of one line. Rejected rather than
    /// resolved to whichever portal comes first in the list: the special ends
    /// up on one specific line, so the author has to be the one who picks it.
    #[error(
        "trigger `{id}` names portal `{a}` <-> `{b}`, which {count} portals join; a walkover needs exactly one"
    )]
    AmbiguousWalkoverPortal {
        /// The trigger's identifier.
        id: String,
        /// The first room.
        a: String,
        /// The second room.
        b: String,
        /// How many portals join the two rooms.
        count: usize,
    },
    /// A drop wall or bridge that names no [`Portal::fires_on`], leaving
    /// nothing to fire it.
    #[error("portal `{a}` <-> `{b}` is a {kind} but names no trigger in `fires_on`")]
    ConstructWithoutTrigger {
        /// The first room.
        a: String,
        /// The second room.
        b: String,
        /// The kind that needs one: `"drop_wall"` or `"bridge"`.
        kind: &'static str,
    },
    /// [`Portal::thickness`], [`Portal::depth`] or [`Portal::fires_on`] on a
    /// portal that is neither a drop wall nor a bridge, or on the one of the
    /// two that has no such field — fields that would otherwise be silently
    /// ignored, the same reject-don't-degrade posture
    /// [`Self::DoorFieldsOnPlainPortal`] takes on a door field.
    #[error("portal `{a}` <-> `{b}` sets `{field}`, which this kind of portal does not have")]
    FloorFieldOnOtherPortal {
        /// The first room.
        a: String,
        /// The second room.
        b: String,
        /// Which field was set: `"thickness"`, `"depth"` or `"fires_on"`.
        field: &'static str,
    },
    /// A drop wall that names no [`Portal::thickness`], leaving the depth of
    /// the sector filling its gap undetermined.
    #[error("portal `{a}` <-> `{b}` is a drop wall but names no thickness")]
    MissingDropWallThickness {
        /// The first room.
        a: String,
        /// The second room.
        b: String,
    },
    /// A [`Portal::thickness`] that is not one of
    /// [`Ir::DROP_WALL_THICKNESS`].
    #[error("portal `{a}` <-> `{b}` has thickness {value}, which must be 8, 16, 32 or 64")]
    InvalidDropWallThickness {
        /// The first room.
        a: String,
        /// The second room.
        b: String,
        /// The rejected value.
        value: i32,
    },
    /// A bridge that names no [`Portal::depth`], leaving its pit floor
    /// undetermined.
    #[error("portal `{a}` <-> `{b}` is a bridge but names no depth")]
    MissingBridgeDepth {
        /// The first room.
        a: String,
        /// The second room.
        b: String,
    },
    /// A [`Portal::depth`] that is not a positive multiple of
    /// [`Ir::BRIDGE_DEPTH_STEP`]. Whether it also clears the step height is
    /// compilation's question, not the IR's.
    #[error(
        "portal `{a}` <-> `{b}` has depth {value}; a bridge rests a positive multiple of 8 below its rooms"
    )]
    InvalidBridgeDepth {
        /// The first room.
        a: String,
        /// The second room.
        b: String,
        /// The rejected value.
        value: i32,
    },
    /// A bridge whose two rooms are not at one floor — the raised pit comes
    /// to rest at their shared floor, so there has to be one.
    #[error(
        "portal `{a}` <-> `{b}` is a bridge between floors {floor_a} and {floor_b}; a bridge joins rooms at one floor"
    )]
    BridgeFloorsDiffer {
        /// The first room.
        a: String,
        /// The second room.
        b: String,
        /// The first room's floor.
        floor_a: i32,
        /// The second room's floor.
        floor_b: i32,
    },
    /// Two reveals share an id.
    #[error("reveal `{id}` is declared twice")]
    DuplicateReveal {
        /// The repeated identifier.
        id: String,
    },
    /// A reveal names a room that does not exist.
    #[error("reveal `{id}` names unknown room `{room}`")]
    RevealUnknownRoom {
        /// The reveal's identifier.
        id: String,
        /// The unresolvable room identifier.
        room: String,
    },
    /// A [`RevealKind::Pedestal`] reveal whose [`Reveal::rise`] is absent,
    /// zero or negative — the pedestal shape's own rule, as
    /// [`Self::PedestalRiseNotPositive`] is for a [`Pedestal`].
    #[error("reveal `{id}` is a pedestal rising {rise}; a pedestal rises above its host")]
    RevealRiseNotPositive {
        /// The reveal's identifier.
        id: String,
        /// The rejected rise (0 when absent).
        rise: i32,
    },
    /// A [`RevealKind::Closet`] reveal that names a [`Reveal::rise`], which
    /// only the pedestal shape has.
    #[error("reveal `{id}` is a closet, which rests solid and takes no `rise`")]
    RiseOnCloset {
        /// The reveal's identifier.
        id: String,
    },
    /// A reveal's rectangle broke one of the rules a [`Pedestal`]'s does: a
    /// side that is not a positive multiple of 8, a rectangle not strictly
    /// inside its room, one overlapping a pedestal or another reveal, or a
    /// thing outside it.
    ///
    /// One variant carrying a `detail` where a [`Pedestal`] has four specific
    /// ones: the two lists share the rectangle rule but not their
    /// vocabulary, and a reveal's own errors read better named after the
    /// reveal than after the pedestal shape it borrows.
    #[error("reveal `{id}`: {detail}")]
    RevealGeometry {
        /// The reveal's identifier.
        id: String,
        /// What was wrong with it.
        detail: String,
    },
    /// More floor actions than the reachability flood can tell apart.
    #[error("the map has {count} floor actions; at most {max} are allowed")]
    TooManyFloorActions {
        /// The actions counted: drop walls, bridges and reveals.
        count: usize,
        /// [`Ir::MAX_FLOOR_ACTIONS`].
        max: usize,
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

/// Which of the two authored islands [`Ir::validate_island_rect`] is judging.
///
/// A [`Pedestal`] and a [`Reveal`] are the same rectangle under the same
/// rules, but each reports a broken one in its own vocabulary, so the shared
/// body has to know which one it is holding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Island {
    /// A [`Pedestal`], reporting the `Pedestal*` error variants.
    Pedestal,
    /// A [`Reveal`], reporting [`IrError::RevealGeometry`].
    Reveal,
}

impl Island {
    /// The word this island calls itself in a message it shares with the
    /// other.
    fn label(self) -> &'static str {
        match self {
            Self::Pedestal => "pedestal",
            Self::Reveal => "reveal",
        }
    }
}

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

    /// The legal values for [`Portal::door_thickness`], and for
    /// [`Portal::alcove_near`]/[`Portal::alcove_far`] on a
    /// [`PortalKind::Door`]/[`PortalKind::Locked`] portal, in map units.
    ///
    /// A compiler-construction constant, like [`Self::MIN_PORTAL_GAP`], not
    /// an engine-sourced one — nothing in the Doom engine constrains door or
    /// alcove depth. Three enumerated sizes (rather than "any multiple of
    /// [`Self::MIN_PORTAL_GAP`]") is a deliberate authoring constraint the
    /// playtester's request itself specified, matching real mapping
    /// practice: a door is built at one of a few conventional depths, not an
    /// arbitrary one.
    ///
    /// A [`PortalKind::Lift`]'s alcoves take the wider
    /// [`Self::LIFT_ALCOVE_DIMENSIONS`] instead.
    pub const DOOR_DIMENSIONS: [i32; 3] = [8, 16, 32];

    /// The legal values for [`Portal::alcove_near`] and [`Portal::alcove_far`]
    /// on a [`PortalKind::Lift`], in map units.
    ///
    /// [`Self::DOOR_DIMENSIONS`] plus 64, because a lift alcove is not only a
    /// buffer the way a door's is. On a [`LiftTrigger::Walkover`] lift it is
    /// the strip the player must stand *inside* for the trigger to fire, so
    /// it has to be deeper than the player's own radius
    /// ([`crate::compile::CompileError::LiftAlcoveTooShallow`]) — which leaves
    /// 32 as the only workable door dimension and nothing above it for an
    /// approach strip the player walks along rather than merely steps into.
    /// 64 is the next size up on the same 8-unit ladder, and the module the
    /// rest of the compiler's own rectangles are built on
    /// ([`Self::PAD_SIZE`], [`Self::PEDESTAL_DEFAULT_SIZE`]).
    pub const LIFT_ALCOVE_DIMENSIONS: [i32; 4] = [8, 16, 32, 64];

    /// The side of every teleport pad, in map units.
    ///
    /// A compiler-construction constant fixed by measurement, not an engine
    /// fact: 77 of the 83 free-standing pads in DOOM.WAD + DOOM2.WAD are
    /// exactly 64×64, and 89 of 94 wall alcoves are 64 wide and 81 of 94 64
    /// deep (docs/measurements/teleports-2026-08-28.md, probe round 2). The corpus
    /// does not vary it, so neither does the IR.
    pub const PAD_SIZE: i32 = 64;
    /// How far a pad's floor sits above its host room's, in map units.
    ///
    /// Same provenance as [`Self::PAD_SIZE`]: +8 is the most common retail
    /// step (36 of 83 island pads; +24 and +16 follow with 22 and 13), and
    /// it is well under the engine's step-up cap, so a pad is always
    /// walkable onto. The 16/24 variants are a recorded follow-up.
    pub const PAD_FLOOR_STEP: i32 = 8;

    /// The side of one flat tile in world space, in map units.
    ///
    /// An engine fact, unlike the corpus-measured [`Self::PAD_SIZE`] beside
    /// it — two separate facts that happen to coincide. Vanilla maps a flat
    /// onto the world by absolute coordinates and wraps it every 64 units:
    /// `R_MapPlane` derives each span's world position
    /// (`linuxdoom-1.10/r_plane.c`, pinned commit a77dfb96),
    ///
    /// ```text
    /// ds_xfrac = viewx + FixedMul(finecosine[angle], length);
    /// ds_yfrac = -viewy - FixedMul(finesine[angle], length);
    /// ```
    ///
    /// and `R_DrawSpan` indexes the 64x64 flat with the low six bits of each
    /// (`linuxdoom-1.10/r_draw.c`),
    ///
    /// ```text
    /// spot = ((yfrac>>(16-6))&(63*64)) + ((xfrac>>16)&63);
    /// ```
    ///
    /// so a 64x64 sector shows the flat as exactly one tile only when its
    /// corners are multiples of 64. Had the corpus put [`Self::PAD_SIZE`] at
    /// 128 this constant would still be 64.
    ///
    /// The same value is carried in `data/engine.toml`'s `[flat] tile` with
    /// its citation, and `tables::tests` asserts the two agree; the IR keeps
    /// its own copy because [`Self::from_json`] validates without loading
    /// any table.
    pub const FLAT_TILE: i32 = 64;

    /// The side of a [`Pedestal`] whose [`Pedestal::size`] is absent, in map
    /// units.
    ///
    /// A compiler-construction constant, like [`Self::PAD_SIZE`], not an
    /// engine-sourced one; a pedestal is authored geometry, not a fixed-size
    /// engine object, so this is only the default an author gets by leaving
    /// `size` unset.
    pub const PEDESTAL_DEFAULT_SIZE: i32 = 64;

    /// The legal values for [`Portal::thickness`], a drop wall's own depth
    /// along the gap it fills, in map units.
    ///
    /// The IR's own copy of `data/engine.toml`'s `[floor]`
    /// `drop_wall_thickness` (curated from the corpus's drop-wall bounding
    /// boxes, and equal to [`Self::LIFT_ALCOVE_DIMENSIONS`]), carried here for
    /// the same reason [`Self::FLAT_TILE`] is: [`Self::from_json`] validates
    /// without loading any table. `ir`'s own tests assert the two agree.
    pub const DROP_WALL_THICKNESS: [i32; 4] = [8, 16, 32, 64];

    /// The grain [`Portal::depth`] — how far a bridge's pit rests below its
    /// two rooms — must be a positive multiple of, in map units.
    ///
    /// The IR's own copy of `data/engine.toml`'s `[floor]`
    /// `bridge_depth_step` (curated: retail bridge travels are multiples of
    /// 8), on the same footing as [`Self::DROP_WALL_THICKNESS`].
    pub const BRIDGE_DEPTH_STEP: i32 = 8;

    /// The most floor actions — drop walls, bridges and reveals — one map may
    /// carry.
    ///
    /// A reachability limit rather than an engine one: the flood has to carry
    /// which actions have already fired as part of the state it floods over,
    /// one bit each beside the eight key classes [`crate::reach::KeyMask`]
    /// already holds, and eight is the budget set aside for them (the mask
    /// widens to carry them when the flood learns floor actions). The IR
    /// refuses a ninth here, where the author can still see which action to
    /// drop, rather than leaving two of them to alias onto one bit.
    pub const MAX_FLOOR_ACTIONS: usize = 8;

    /// The trigger with this id.
    #[must_use]
    pub fn trigger(&self, id: &str) -> Option<&Trigger> {
        self.triggers.iter().find(|t| t.id == id)
    }

    /// Which way the constructs naming trigger `id` move, or `None` when
    /// nothing names it.
    ///
    /// [`Self::from_json`] has already refused a trigger whose constructs
    /// disagree ([`IrError::TriggerMixesFamilies`]), so the first construct
    /// found decides for all of them.
    #[must_use]
    pub fn trigger_family(&self, id: &str) -> Option<FloorFamilyIr> {
        if self.reveals.iter().any(|r| r.trigger == id) {
            return Some(FloorFamilyIr::Lower);
        }
        self.portals
            .iter()
            .find(|p| p.fires_on.as_deref() == Some(id))
            .map(|p| match p.kind {
                PortalKind::Bridge => FloorFamilyIr::Raise,
                _ => FloorFamilyIr::Lower,
            })
    }

    /// Whether `portal` is a barrier: a lift portal between rooms at one
    /// floor.
    #[must_use]
    pub fn is_barrier(&self, portal: &Portal) -> bool {
        portal.kind == PortalKind::Lift
            && self.room(&portal.a).map(|r| r.floor) == self.room(&portal.b).map(|r| r.floor)
    }

    /// The index of the lower room of a lift portal, `None` for a barrier
    /// or a non-lift portal.
    #[must_use]
    pub fn low_room_of(&self, portal: &Portal) -> Option<usize> {
        if portal.kind != PortalKind::Lift || self.is_barrier(portal) {
            return None;
        }
        let ia = self.rooms.iter().position(|r| r.id == portal.a)?;
        let ib = self.rooms.iter().position(|r| r.id == portal.b)?;
        Some(if self.rooms[ia].floor < self.rooms[ib].floor {
            ia
        } else {
            ib
        })
    }

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
    /// [`IrError::InvalidDoorDimension`] for a door's `door_thickness`,
    /// `alcove_near`, or `alcove_far` that is not 8, 16, or 32,
    /// [`IrError::InvalidLiftAlcoveDimension`] for a lift's `alcove_near` or
    /// `alcove_far` that is not 8, 16, 32, or 64,
    /// [`IrError::DoorFieldsOnPlainPortal`] for a plain portal that sets any
    /// of those three fields, [`IrError::DoorGapMismatch`] for a door/locked
    /// portal whose facing-wall gap does not exactly equal `door_thickness +
    /// alcove_near + alcove_far`, the numeric-range variants listed above
    /// (which an exit's `at` is also checked against),
    /// [`IrError::DuplicateTeleport`] for a repeated teleport id,
    /// [`IrError::TeleportUnknownRoom`] for a teleport naming a room that
    /// does not exist as its pad's room or its destination's,
    /// [`IrError::TeleportPadOffFlatGrid`] for a pad whose square's low
    /// corner is not a multiple of [`Self::FLAT_TILE`],
    /// [`IrError::TeleportPadOutsideRoom`] for an
    /// island pad whose square does not lie strictly inside its room,
    /// [`IrError::TeleportPadOffWall`] for a wall pad whose point is on no
    /// axis-aligned wall or whose span runs past that wall's far end,
    /// [`IrError::TeleportPadBesideOpening`] for a wall pad whose span
    /// overlaps or touches a portal opening or the exit segment on the same
    /// wall, [`IrError::TeleportPadRecessTooClose`] for a wall pad whose
    /// recess would come within `MIN_PORTAL_GAP` of another room,
    /// [`IrError::TeleportPadsOverlap`] for two pads in one room that overlap
    /// or touch, [`IrError::TeleportDestinationOutsideRoom`] for a
    /// destination outside its room and on none of its pads,
    /// [`IrError::TeleportDestinationsShareSector`] for two teleports that
    /// deliver to different points of the same emitted sector,
    /// [`IrError::DoorThicknessOnLift`] for a lift portal that sets
    /// `door_thickness`, [`IrError::RiseOnNonLiftPortal`] for a portal that
    /// is not a lift but sets `rise`, [`IrError::LiftRiseOnUnequalFloors`]
    /// for a lift portal whose rooms sit at different floors but that sets
    /// `rise`, [`IrError::BarrierMissingRise`] for a lift portal whose rooms sit at
    /// one floor but that names no `rise`, [`IrError::LiftWalkoverNeedsAlcove`]
    /// for a walkover lift whose low room names no alcove to carry the
    /// trigger line, [`IrError::BarrierTrigger`] for a barrier that sets
    /// a trigger other than `switch`, [`IrError::DuplicatePedestal`] for a
    /// repeated pedestal id, [`IrError::PedestalUnknownRoom`] for a pedestal
    /// naming a room that does not exist, [`IrError::PedestalRiseNotPositive`]
    /// for a pedestal with a zero or negative `rise`,
    /// [`IrError::PedestalSizeNotMultipleOf8`] for a pedestal whose `size`
    /// has a side that is not a positive multiple of 8,
    /// [`IrError::PedestalOutsideRoom`] for a pedestal whose rectangle does
    /// not lie strictly inside its room, [`IrError::PedestalThingOutside`]
    /// for a thing placed on a pedestal but outside its rectangle,
    /// [`IrError::PedestalsOverlap`] for two pedestals, or a pedestal and a
    /// teleport pad, in one room that overlap or touch, and
    /// [`IrError::TeleportDestinationOnPedestal`] for a teleport whose
    /// destination point lies inside or on a pedestal's rectangle.
    ///
    /// The floor actions add [`IrError::DuplicateTrigger`] for a repeated
    /// trigger id, [`IrError::TriggerOffWall`] for a trigger that is not
    /// placed where its kind can be, [`IrError::WalkoverOnNonPlainPortal`]
    /// for a walkover naming a portal that is neither plain nor a bridge,
    /// [`IrError::UnknownTrigger`] for a construct naming a trigger that does
    /// not exist, [`IrError::TriggerMixesFamilies`] for a trigger named by
    /// both a lowering and a rising construct, [`IrError::TriggerUnused`] for
    /// a trigger no construct names, [`IrError::ConstructWithoutTrigger`] for
    /// a drop wall or bridge naming none, [`IrError::FloorFieldOnOtherPortal`]
    /// for `thickness`, `depth` or `fires_on` on a portal that has no such
    /// field, [`IrError::MissingDropWallThickness`] and
    /// [`IrError::InvalidDropWallThickness`] for a drop wall's `thickness`,
    /// [`IrError::MissingBridgeDepth`] and [`IrError::InvalidBridgeDepth`] for
    /// a bridge's `depth`, [`IrError::BridgeFloorsDiffer`] for a bridge
    /// between rooms at two floors, [`IrError::DuplicateReveal`] for a
    /// repeated reveal id, [`IrError::RevealUnknownRoom`] for a reveal naming
    /// a room that does not exist, [`IrError::RevealRiseNotPositive`] and
    /// [`IrError::RiseOnCloset`] for a reveal's `rise`,
    /// [`IrError::AmbiguousWalkoverPortal`] for a walkover naming a room pair
    /// that more than one portal joins, [`IrError::RevealGeometry`] for a
    /// reveal's rectangle (including one overlapping a pedestal, another
    /// reveal or a teleport pad), [`IrError::TeleportDestinationOnReveal`]
    /// for a teleport delivering inside a reveal, and
    /// [`IrError::TooManyFloorActions`] for a map carrying more than
    /// [`Self::MAX_FLOOR_ACTIONS`] of them.
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
        Self::validate_lifts(&ir)?;
        Self::validate_pedestals(&ir, &seen)?;
        Self::validate_triggers(&ir, &seen)?;
        Self::validate_floors(&ir, &seen)?;

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

    /// Validates every portal's door/lift-only fields
    /// ([`Portal::door_thickness`]/[`Portal::alcove_near`]/[`Portal::alcove_far`]):
    /// `door_thickness` present and one of [`Self::DOOR_DIMENSIONS`] for
    /// [`PortalKind::Door`]/[`PortalKind::Locked`], and absent for every
    /// other kind — [`PortalKind::Lift`] and [`PortalKind::DropWall`] report
    /// the more specific [`IrError::DoorThicknessOnLift`], since each already
    /// fills its gap with a sector of its own. The alcoves, when present,
    /// must be one of [`Self::DOOR_DIMENSIONS`] for
    /// [`PortalKind::Door`]/[`PortalKind::Locked`] and one of
    /// [`Self::LIFT_ALCOVE_DIMENSIONS`] for [`PortalKind::Lift`] and
    /// [`PortalKind::DropWall`], and are rejected outright on
    /// [`PortalKind::Plain`] and [`PortalKind::Bridge`], neither of which has
    /// a door construction or (in v1) an alcove.
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
                // A bridge joins its two rooms with a pit, not a door
                // construction, and takes no alcove in v1 either.
                PortalKind::Plain | PortalKind::Bridge => {
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
                // A drop wall's own sector fills the gap exactly as a
                // platform does, so it admits no door but takes a lift's
                // alcoves.
                PortalKind::Lift | PortalKind::DropWall => {
                    if portal.door_thickness.is_some() {
                        return Err(IrError::DoorThicknessOnLift {
                            a: portal.a.clone(),
                            b: portal.b.clone(),
                        });
                    }
                    for (field, value) in [
                        ("alcove_near", portal.alcove_near),
                        ("alcove_far", portal.alcove_far),
                    ] {
                        if let Some(value) = value
                            && !Self::LIFT_ALCOVE_DIMENSIONS.contains(&value)
                        {
                            return Err(IrError::InvalidLiftAlcoveDimension {
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
        for (subject, p) in [
            (format!("teleport `{}`", t.id), point),
            (format!("teleport `{}` destination", t.id), t.to.at),
        ] {
            if !MAP_RANGE.contains(&p.x) || !MAP_RANGE.contains(&p.y) {
                return Err(IrError::CoordinateOutOfRange {
                    subject,
                    x: p.x,
                    y: p.y,
                    min: *MAP_RANGE.start(),
                    max: *MAP_RANGE.end(),
                });
            }
        }
        match t.pad {
            PadPlacement::Island(c) => {
                let (lo, hi) = pad_square(room, t.pad).expect("an island square always resolves");
                let corners = [lo, Pt { x: lo.x, y: hi.y }, hi, Pt { x: hi.x, y: lo.y }];
                let inside = corners
                    .iter()
                    .all(|&p| contains(&room.footprint, p) && clearance(&room.footprint, p) > 0.0);
                let vertex_in_square = room.footprint.iter().any(|&v| square_contains(lo, hi, v));
                // A third test, because neither of the two above sees a
                // *spur* of solid material narrower than the pad running
                // clean through the square: both of its vertices lie beyond
                // the square, and the four corners sit strictly inside the
                // room on either side of it. Such an IR passed every
                // compile pass and died in the nodebuilder at pack time.
                let wall_through_square =
                    edges(&room.footprint).any(|(p, q)| segment_enters_open_rect(p, q, lo, hi));
                if !inside || vertex_in_square || wall_through_square {
                    return Err(IrError::TeleportPadOutsideRoom {
                        id: t.id.clone(),
                        x: c.x,
                        y: c.y,
                    });
                }
            }
            PadPlacement::Wall(at) => {
                let Some((axis, fixed, _, open_lo, open_hi)) = wall_cut(room, at) else {
                    return Err(IrError::TeleportPadOffWall {
                        id: t.id.clone(),
                        x: at.x,
                        y: at.y,
                    });
                };
                Self::validate_wall_pad_openings(ir, t, axis, fixed, (open_lo, open_hi))?;
                Self::validate_wall_pad_neighbors(ir, t, room)?;
            }
        }
        // Last of the pad checks, in both arms, for the reason the island
        // arm's geometry tests come first: a pad flush against a wall, or
        // one straddling an opening, is an independent authoring mistake
        // from a pad half a tile off the flat grid, and reporting the
        // geometric problem is more useful than reporting the coincidental
        // alignment one. `ir.grid` plays no part — 64 subsumes every grid
        // that divides it, and a grid that does *not* divide 64 (48, say)
        // still cannot excuse a pad off the flat grid, since it is the
        // renderer, not the author's grid, that wraps the flat.
        let (lo, _) = pad_square(room, t.pad).expect("resolved by the arm above");
        if lo.x % Self::FLAT_TILE != 0 || lo.y % Self::FLAT_TILE != 0 {
            return Err(IrError::TeleportPadOffFlatGrid {
                id: t.id.clone(),
                at_x: point.x,
                at_y: point.y,
                x: lo.x,
                y: lo.y,
            });
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

    /// Rejects a wall pad whose span on its host wall overlaps *or touches*
    /// another opening cut into that same wall — a portal's opening or the
    /// level exit's segment.
    ///
    /// Touching counts, for the reason two touching pads do
    /// ([`IrError::TeleportPadsOverlap`]): two openings that meet exactly
    /// leave the recess's side wall coincident with the passage's jamb, a
    /// pair of one-sided linedefs on one line. The wall is identified by
    /// `(axis, fixed)`, which is how [`pad_square`] and
    /// [`crate::compile::portals::split_wall_for_opening`] both name it.
    fn validate_wall_pad_openings(
        ir: &Self,
        t: &Teleport,
        axis: Axis,
        fixed: i32,
        span: (i32, i32),
    ) -> Result<(), IrError> {
        for (opening, (lo, hi)) in wall_openings(ir, &t.room, axis, fixed) {
            if span.0 <= hi && lo <= span.1 {
                return Err(IrError::TeleportPadBesideOpening {
                    id: t.id.clone(),
                    opening,
                });
            }
        }
        Ok(())
    }

    /// Rejects a wall pad whose recess would come within
    /// [`Self::MIN_PORTAL_GAP`] of another room.
    ///
    /// The recess is real geometry carved outward from the host's wall into
    /// the void rooms are authored apart across, so it is subject to the
    /// same rule every portal gap is: a neighbor flush against its far wall
    /// would emit two coincident one-sided linedefs, and one merely nearer
    /// than 8 leaves a sliver too thin to read as wall material. Grown by
    /// the gap on every side and tested *open*, so a room exactly
    /// `MIN_PORTAL_GAP` away — the thinnest wall the portal rule itself
    /// accepts — is allowed.
    ///
    /// Only the recess is checked. An island pad is enclosed by its own
    /// room, which the footprint checks already hold apart from every other.
    fn validate_wall_pad_neighbors(ir: &Self, t: &Teleport, room: &Room) -> Result<(), IrError> {
        let (lo, hi) = pad_square(room, t.pad).expect("the wall cut resolved above");
        let gap = Self::MIN_PORTAL_GAP;
        let grown_lo = Pt {
            x: lo.x - gap,
            y: lo.y - gap,
        };
        let grown_hi = Pt {
            x: hi.x + gap,
            y: hi.y + gap,
        };
        for other in ir.rooms.iter().filter(|r| r.id != room.id) {
            let crowded = edges(&other.footprint)
                .any(|(p, q)| segment_enters_open_rect(p, q, grown_lo, grown_hi));
            if crowded {
                return Err(IrError::TeleportPadRecessTooClose {
                    id: t.id.clone(),
                    room: other.id.clone(),
                });
            }
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
                let (sa, sb) = (
                    pad_square(room, a.pad).expect("validated above"),
                    pad_square(room, b.pad).expect("validated above"),
                );
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

    /// Validates every portal's [`Portal::rise`] — rejected outright on
    /// anything but [`PortalKind::Lift`] — and, for every
    /// [`PortalKind::Lift`] portal, its `rise`/`trigger` against its
    /// barrier/lift status.
    fn validate_lifts(ir: &Self) -> Result<(), IrError> {
        for portal in &ir.portals {
            if portal.kind != PortalKind::Lift && portal.rise.is_some() {
                return Err(IrError::RiseOnNonLiftPortal {
                    a: portal.a.clone(),
                    b: portal.b.clone(),
                });
            }
        }
        for portal in ir.portals.iter().filter(|p| p.kind == PortalKind::Lift) {
            let barrier = ir.is_barrier(portal);
            match (barrier, portal.rise) {
                (false, Some(_)) => {
                    return Err(IrError::LiftRiseOnUnequalFloors {
                        a: portal.a.clone(),
                        b: portal.b.clone(),
                    });
                }
                (true, None) => {
                    return Err(IrError::BarrierMissingRise {
                        a: portal.a.clone(),
                        b: portal.b.clone(),
                    });
                }
                _ => {}
            }
            if barrier && portal.trigger != LiftTrigger::Switch {
                return Err(IrError::BarrierTrigger {
                    a: portal.a.clone(),
                    b: portal.b.clone(),
                    trigger: match portal.trigger {
                        LiftTrigger::Walkover => "walkover",
                        LiftTrigger::BothEnds => "both_ends",
                        LiftTrigger::Switch => unreachable!("filtered above"),
                    },
                });
            }
            if portal.trigger == LiftTrigger::Walkover {
                let low = ir.low_room_of(portal).expect("not a barrier");
                let (field, alcove) = if ir.rooms[low].id == portal.a {
                    ("alcove_near", portal.alcove_near)
                } else {
                    ("alcove_far", portal.alcove_far)
                };
                if alcove.is_none() {
                    return Err(IrError::LiftWalkoverNeedsAlcove {
                        a: portal.a.clone(),
                        b: portal.b.clone(),
                        low_room: ir.rooms[low].id.clone(),
                        field,
                    });
                }
            }
        }
        Ok(())
    }

    /// Validates every pedestal: structural fields, containment of its
    /// rectangle and things within its room, and non-overlap with every
    /// other pedestal and every teleport pad in the same room.
    fn validate_pedestals(ir: &Self, seen: &HashSet<&str>) -> Result<(), IrError> {
        let mut ids: HashSet<&str> = HashSet::new();
        for p in &ir.pedestals {
            if !ids.insert(p.id.as_str()) {
                return Err(IrError::DuplicatePedestal { id: p.id.clone() });
            }
            if !seen.contains(p.room.as_str()) {
                return Err(IrError::PedestalUnknownRoom {
                    id: p.id.clone(),
                    room: p.room.clone(),
                });
            }
            if p.rise <= 0 {
                return Err(IrError::PedestalRiseNotPositive {
                    id: p.id.clone(),
                    rise: p.rise,
                });
            }
            let room = ir.room(&p.room).expect("checked above");
            Self::validate_island_rect(room, Island::Pedestal, &p.id, p.at, p.size, &p.things)?;
        }
        // One island rule for pads and pedestals alike.
        for (i, a) in ir.pedestals.iter().enumerate() {
            for b in &ir.pedestals[i + 1..] {
                if a.room == b.room && squares_overlap_or_touch(a.rect(), b.rect()) {
                    return Err(IrError::PedestalsOverlap {
                        first: a.id.clone(),
                        second: b.id.clone(),
                    });
                }
            }
            for t in ir.teleports.iter().filter(|t| t.room == a.room) {
                let room = ir.room(&t.room).expect("validated by validate_teleports");
                let pad = pad_square(room, t.pad).expect("validated by validate_teleports");
                if squares_overlap_or_touch(a.rect(), pad) {
                    return Err(IrError::PedestalsOverlap {
                        first: a.id.clone(),
                        second: t.id.clone(),
                    });
                }
            }
        }
        Self::validate_destinations_off_pedestals(ir)
    }

    /// Rejects a teleport whose destination point lands on a pedestal.
    ///
    /// The last of [`Self::validate_pedestals`]' checks, split out only
    /// because the whole of it no longer fits one function: a destination is
    /// a *point*, so the island rule between two rectangles there never sees
    /// it. Closed on both axes — a point on the rectangle's own edge is one
    /// the arrival's radius straddles.
    fn validate_destinations_off_pedestals(ir: &Self) -> Result<(), IrError> {
        for p in &ir.pedestals {
            let (lo, hi) = p.rect();
            for t in ir.teleports.iter().filter(|t| t.to.room == p.room) {
                if square_contains(lo, hi, t.to.at) {
                    return Err(IrError::TeleportDestinationOnPedestal {
                        teleport: t.id.clone(),
                        pedestal: p.id.clone(),
                    });
                }
            }
        }
        Ok(())
    }

    /// The rectangle rule a [`Pedestal`] and a [`Reveal`] share: a size that
    /// is a positive multiple of 8, corners inside the map range, a rectangle
    /// strictly inside its host room with no vertex or wall crossing it, and
    /// every thing strictly inside it.
    ///
    /// One body, two vocabularies: a pedestal reports the specific
    /// `Pedestal*` variants it has always had, a reveal reports the same
    /// findings as [`IrError::RevealGeometry`] details. [`Island`] picks
    /// which — the two lists must not drift, since both compile to the same
    /// kind of sector inside a room.
    fn validate_island_rect(
        room: &Room,
        island: Island,
        id: &str,
        at: Pt,
        size: Option<[i32; 2]>,
        things: &[IrThing],
    ) -> Result<(), IrError> {
        let [w, h] = size.unwrap_or([Self::PEDESTAL_DEFAULT_SIZE; 2]);
        if w <= 0 || h <= 0 || w % 8 != 0 || h % 8 != 0 {
            return Err(match island {
                Island::Pedestal => IrError::PedestalSizeNotMultipleOf8 {
                    id: id.to_owned(),
                    width: w,
                    height: h,
                },
                Island::Reveal => IrError::RevealGeometry {
                    id: id.to_owned(),
                    detail: format!("is {w}x{h}; each side must be a positive multiple of 8"),
                },
            });
        }
        // Range-checked before the rectangle is built: `size` has no upper
        // bound of its own, so `at + size` can overflow `i32` outright, and
        // the saturating sum `Pedestal::rect`/`Reveal::rect` would report
        // then reads as a plausible-looking corner unless it is compared
        // against `MAP_RANGE` after being computed with checked arithmetic
        // here.
        let out_of_range = |x: i32, y: i32| IrError::CoordinateOutOfRange {
            subject: format!("{} `{id}`", island.label()),
            x,
            y,
            min: *MAP_RANGE.start(),
            max: *MAP_RANGE.end(),
        };
        if !MAP_RANGE.contains(&at.x) || !MAP_RANGE.contains(&at.y) {
            return Err(out_of_range(at.x, at.y));
        }
        let hi_x = at.x.checked_add(w);
        let hi_y = at.y.checked_add(h);
        let in_range = |v: Option<i32>| v.is_some_and(|v| MAP_RANGE.contains(&v));
        if !in_range(hi_x) || !in_range(hi_y) {
            return Err(out_of_range(
                hi_x.unwrap_or(i32::MAX),
                hi_y.unwrap_or(i32::MAX),
            ));
        }
        // The corners `Pedestal::rect`/`Reveal::rect` report, now that the
        // sum above is known to fit.
        let (lo, hi) = (
            at,
            Pt {
                x: at.x.saturating_add(w),
                y: at.y.saturating_add(h),
            },
        );
        // The island pad's containment test, verbatim: every corner strictly
        // inside, no room vertex inside the rectangle, no wall through it.
        let corners = [lo, Pt { x: lo.x, y: hi.y }, hi, Pt { x: hi.x, y: lo.y }];
        let inside = corners
            .iter()
            .all(|&c| contains(&room.footprint, c) && clearance(&room.footprint, c) > 0.0);
        let vertex_in = room.footprint.iter().any(|&v| square_contains(lo, hi, v));
        let wall_through =
            edges(&room.footprint).any(|(s, e)| segment_enters_open_rect(s, e, lo, hi));
        if !inside || vertex_in || wall_through {
            return Err(match island {
                Island::Pedestal => IrError::PedestalOutsideRoom {
                    id: id.to_owned(),
                    x: at.x,
                    y: at.y,
                },
                Island::Reveal => IrError::RevealGeometry {
                    id: id.to_owned(),
                    detail: format!(
                        "at ({}, {}) does not fit strictly inside its room",
                        at.x, at.y
                    ),
                },
            });
        }
        for t in things {
            let strictly_inside = t.at.x > lo.x && t.at.x < hi.x && t.at.y > lo.y && t.at.y < hi.y;
            if !strictly_inside {
                return Err(match island {
                    Island::Pedestal => IrError::PedestalThingOutside {
                        id: id.to_owned(),
                        kind: t.kind.clone(),
                        x: t.at.x,
                        y: t.at.y,
                    },
                    Island::Reveal => IrError::RevealGeometry {
                        id: id.to_owned(),
                        detail: format!(
                            "places `{}` at ({}, {}), outside its rectangle",
                            t.kind, t.at.x, t.at.y
                        ),
                    },
                });
            }
        }
        Ok(())
    }

    /// Validates every trigger and every construct that names one: each
    /// trigger's own placement, that every construct names a trigger that
    /// exists, that the constructs sharing a trigger all move the same way,
    /// and that no trigger is declared which nothing names.
    fn validate_triggers(ir: &Self, seen: &HashSet<&str>) -> Result<(), IrError> {
        let mut ids: HashSet<&str> = HashSet::new();
        for t in &ir.triggers {
            if !ids.insert(t.id.as_str()) {
                return Err(IrError::DuplicateTrigger { id: t.id.clone() });
            }
            Self::validate_one_trigger(ir, seen, t)?;
        }
        Self::validate_trigger_families(ir, &ids)
    }

    /// Validates one trigger's own placement: a switch on a point of its
    /// room's own wall, a walkover on exactly one portal whose opening line
    /// can carry it, and neither carrying the other's fields.
    ///
    /// A switch's point is judged by exactly the test
    /// [`crate::compile::exits`] applies to an exit's — [`wall_edges`] and
    /// [`Axis::split`] — so a switch is placed on a wall the same way an exit
    /// is, and the two can never disagree about what "on the wall" means.
    fn validate_one_trigger(ir: &Self, seen: &HashSet<&str>, t: &Trigger) -> Result<(), IrError> {
        let off_wall = |detail: String| IrError::TriggerOffWall {
            id: t.id.clone(),
            detail,
        };
        match t.kind {
            TriggerKind::Switch => {
                let (Some(room), Some(at)) = (t.room.as_deref(), t.at) else {
                    return Err(off_wall("a switch needs `room` and `at`".to_owned()));
                };
                if t.portal.is_some() {
                    return Err(off_wall("a switch takes no `portal`".to_owned()));
                }
                if !seen.contains(room) {
                    return Err(off_wall(format!("names unknown room `{room}`")));
                }
                let footprint = &ir.room(room).expect("checked above").footprint;
                let on_wall = wall_edges(footprint).any(|(axis, fixed, lo, hi, _)| {
                    let (along, across) = axis.split(at);
                    across == fixed && along > lo && along < hi
                });
                if !on_wall {
                    return Err(off_wall(format!(
                        "({}, {}) is not on room `{room}`'s own wall",
                        at.x, at.y
                    )));
                }
            }
            TriggerKind::Walkover => {
                let Some([a, b]) = t.portal.as_ref() else {
                    return Err(off_wall("a walkover needs `portal: [a, b]`".to_owned()));
                };
                if t.room.is_some() || t.at.is_some() {
                    return Err(off_wall("a walkover takes no `room` or `at`".to_owned()));
                }
                let mut joining = ir
                    .portals
                    .iter()
                    .filter(|p| (p.a == *a && p.b == *b) || (p.a == *b && p.b == *a));
                let Some(portal) = joining.next() else {
                    return Err(off_wall(format!("no portal joins `{a}` and `{b}`")));
                };
                // Before the kind test below, which would otherwise judge a
                // portal that may not be the one the special lands on.
                let count = 1 + joining.count();
                if count > 1 {
                    return Err(IrError::AmbiguousWalkoverPortal {
                        id: t.id.clone(),
                        a: a.clone(),
                        b: b.clone(),
                        count,
                    });
                }
                // A plain portal's opening line, or a bridge's own two pit
                // thresholds: stepping down into the pit is the crossing that
                // raises it, the one bridge trigger that cannot strand the
                // player who takes the drop.
                if !matches!(portal.kind, PortalKind::Plain | PortalKind::Bridge) {
                    return Err(IrError::WalkoverOnNonPlainPortal {
                        id: t.id.clone(),
                        a: portal.a.clone(),
                        b: portal.b.clone(),
                    });
                }
            }
        }
        Ok(())
    }

    /// Rejects a construct naming a trigger that does not exist, a trigger
    /// whose constructs do not agree on which way they move, and a trigger
    /// nothing names.
    ///
    /// One trigger is one sector tag carrying one line special, so a trigger
    /// named by both a bridge and a drop wall would have to be two specials
    /// at once.
    fn validate_trigger_families(ir: &Self, ids: &HashSet<&str>) -> Result<(), IrError> {
        // Keyed by an owned id rather than a borrow: the closure below takes
        // `id` with its own anonymous lifetime, which cannot be stored behind
        // a `&mut` to a map borrowing `ir`.
        let mut family: HashMap<String, FloorFamilyIr> = HashMap::new();
        let mut note = |id: &str, subject: String, fam: FloorFamilyIr| -> Result<(), IrError> {
            if !ids.contains(id) {
                return Err(IrError::UnknownTrigger {
                    subject,
                    id: id.to_owned(),
                });
            }
            match family.get(id) {
                Some(&seen) if seen != fam => {
                    Err(IrError::TriggerMixesFamilies { id: id.to_owned() })
                }
                _ => {
                    family.insert(id.to_owned(), fam);
                    Ok(())
                }
            }
        };
        for p in &ir.portals {
            let fam = match p.kind {
                PortalKind::DropWall => FloorFamilyIr::Lower,
                PortalKind::Bridge => FloorFamilyIr::Raise,
                _ => continue,
            };
            let id = p
                .fires_on
                .as_deref()
                .ok_or_else(|| IrError::ConstructWithoutTrigger {
                    a: p.a.clone(),
                    b: p.b.clone(),
                    kind: if p.kind == PortalKind::Bridge {
                        "bridge"
                    } else {
                        "drop_wall"
                    },
                })?;
            note(id, format!("portal `{}` <-> `{}`", p.a, p.b), fam)?;
        }
        for r in &ir.reveals {
            note(
                &r.trigger,
                format!("reveal `{}`", r.id),
                FloorFamilyIr::Lower,
            )?;
        }
        for t in &ir.triggers {
            if !family.contains_key(&t.id) {
                return Err(IrError::TriggerUnused { id: t.id.clone() });
            }
        }
        Ok(())
    }

    /// Validates the drop walls, bridges and reveals: the fields each kind
    /// does and does not take, the floors a bridge joins, every reveal's own
    /// structure and rectangle, and the map's whole action budget.
    ///
    /// Two rules this deliberately leaves to compilation, both of which need
    /// the engine's step height: a drop wall's two floors within a step of
    /// each other, and a bridge pit (or a pedestal reveal's rise) deeper than
    /// one. [`Self::from_json`] loads no table at all — see
    /// [`Self::FLAT_TILE`] — so a check that needs a table constant cannot
    /// live here.
    fn validate_floors(ir: &Self, seen: &HashSet<&str>) -> Result<(), IrError> {
        for p in &ir.portals {
            let (a, b) = (p.a.clone(), p.b.clone());
            match p.kind {
                PortalKind::DropWall => {
                    if p.depth.is_some() {
                        return Err(IrError::FloorFieldOnOtherPortal {
                            a,
                            b,
                            field: "depth",
                        });
                    }
                    let Some(thickness) = p.thickness else {
                        return Err(IrError::MissingDropWallThickness { a, b });
                    };
                    if !Self::DROP_WALL_THICKNESS.contains(&thickness) {
                        return Err(IrError::InvalidDropWallThickness {
                            a,
                            b,
                            value: thickness,
                        });
                    }
                }
                PortalKind::Bridge => {
                    if p.thickness.is_some() {
                        return Err(IrError::FloorFieldOnOtherPortal {
                            a,
                            b,
                            field: "thickness",
                        });
                    }
                    let Some(depth) = p.depth else {
                        return Err(IrError::MissingBridgeDepth { a, b });
                    };
                    if depth <= 0 || depth % Self::BRIDGE_DEPTH_STEP != 0 {
                        return Err(IrError::InvalidBridgeDepth { a, b, value: depth });
                    }
                    let room_a = ir.room(&p.a).expect("validated by validate_portals");
                    let room_b = ir.room(&p.b).expect("validated by validate_portals");
                    if room_a.floor != room_b.floor {
                        return Err(IrError::BridgeFloorsDiffer {
                            a,
                            b,
                            floor_a: room_a.floor,
                            floor_b: room_b.floor,
                        });
                    }
                }
                _ => {
                    for (field, set) in [
                        ("thickness", p.thickness.is_some()),
                        ("depth", p.depth.is_some()),
                        ("fires_on", p.fires_on.is_some()),
                    ] {
                        if set {
                            return Err(IrError::FloorFieldOnOtherPortal { a, b, field });
                        }
                    }
                }
            }
        }
        Self::validate_reveals(ir, seen)?;
        let count = ir.reveals.len()
            + ir.portals
                .iter()
                .filter(|p| matches!(p.kind, PortalKind::DropWall | PortalKind::Bridge))
                .count();
        if count > Self::MAX_FLOOR_ACTIONS {
            return Err(IrError::TooManyFloorActions {
                count,
                max: Self::MAX_FLOOR_ACTIONS,
            });
        }
        Ok(())
    }

    /// Validates every reveal: its id and room, the rest shape its `rise`
    /// belongs to, its rectangle, that no two islands in one room overlap,
    /// and that no teleport delivers into one.
    fn validate_reveals(ir: &Self, seen: &HashSet<&str>) -> Result<(), IrError> {
        let mut ids: HashSet<&str> = HashSet::new();
        for r in &ir.reveals {
            if !ids.insert(r.id.as_str()) {
                return Err(IrError::DuplicateReveal { id: r.id.clone() });
            }
            if !seen.contains(r.room.as_str()) {
                return Err(IrError::RevealUnknownRoom {
                    id: r.id.clone(),
                    room: r.room.clone(),
                });
            }
            match (r.kind, r.rise) {
                (RevealKind::Closet, Some(_)) => {
                    return Err(IrError::RiseOnCloset { id: r.id.clone() });
                }
                (RevealKind::Pedestal, rise) if rise.unwrap_or(0) <= 0 => {
                    return Err(IrError::RevealRiseNotPositive {
                        id: r.id.clone(),
                        rise: rise.unwrap_or(0),
                    });
                }
                _ => {}
            }
            let room = ir.room(&r.room).expect("checked above");
            Self::validate_island_rect(room, Island::Reveal, &r.id, r.at, r.size, &r.things)?;
        }
        Self::validate_reveal_overlaps(ir)?;
        Self::validate_destinations_off_reveals(ir)
    }

    /// Rejects a reveal that overlaps or touches a pedestal, another reveal
    /// or a teleport pad in the same room — the whole of the island rule
    /// [`IrError::PedestalsOverlap`] already keeps between pedestals and
    /// pads, extended to the third kind of island and in the same order.
    fn validate_reveal_overlaps(ir: &Self) -> Result<(), IrError> {
        for (i, r) in ir.reveals.iter().enumerate() {
            let others = ir
                .pedestals
                .iter()
                .filter(|p| p.room == r.room)
                .map(|p| (p.id.as_str(), p.rect()))
                .chain(
                    ir.reveals[i + 1..]
                        .iter()
                        .filter(|o| o.room == r.room)
                        .map(|o| (o.id.as_str(), o.rect())),
                );
            for (id, rect) in others {
                if squares_overlap_or_touch(r.rect(), rect) {
                    return Err(IrError::RevealGeometry {
                        id: r.id.clone(),
                        detail: format!("overlaps `{id}`"),
                    });
                }
            }
            for t in ir.teleports.iter().filter(|t| t.room == r.room) {
                let room = ir.room(&t.room).expect("validated by validate_teleports");
                let pad = pad_square(room, t.pad).expect("validated by validate_teleports");
                if squares_overlap_or_touch(r.rect(), pad) {
                    return Err(IrError::RevealGeometry {
                        id: r.id.clone(),
                        detail: format!("overlaps teleport pad `{}`", t.id),
                    });
                }
            }
        }
        Ok(())
    }

    /// Rejects a teleport whose destination point lands inside a reveal.
    ///
    /// The reveal's twin of [`Self::validate_destinations_off_pedestals`],
    /// run in the same place in the order and by the same closed-on-both-axes
    /// test: a destination is a *point*, so the island rule between two
    /// rectangles never sees it, and a point on the rectangle's own edge is
    /// one the arrival's radius straddles.
    fn validate_destinations_off_reveals(ir: &Self) -> Result<(), IrError> {
        for r in &ir.reveals {
            let (lo, hi) = r.rect();
            for t in ir.teleports.iter().filter(|t| t.to.room == r.room) {
                if square_contains(lo, hi, t.to.at) {
                    return Err(IrError::TeleportDestinationOnReveal {
                        teleport: t.id.clone(),
                        reveal: r.id.clone(),
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

/// Every *other* opening cut into `room`'s wall at `(axis, fixed)`: each
/// portal's opening and the level exit's segment, as an inclusive `(lo, hi)`
/// span along the wall paired with a name for the error message.
///
/// A portal's opening lands on room `a`'s own wall or room `b`'s depending
/// on which of the two `room` is — [`crate::geom::FacingSpan::near`] and
/// `far` respectively — which is why the span is resolved through
/// [`facing_spans`] rather than read off `portal.at` alone. An exit's wall is
/// found with the same `wall_edges` predicate [`pad_square`] uses.
///
/// # Panics
/// Panics if a portal or exit names a room that does not exist — impossible,
/// since [`Ir::validate_portals`] and [`Ir::validate_exits`] both run first.
fn wall_openings(ir: &Ir, room: &str, axis: Axis, fixed: i32) -> Vec<(String, (i32, i32))> {
    let mut out = Vec::new();
    for portal in ir.portals.iter().filter(|p| p.a == room || p.b == room) {
        let a = ir.room(&portal.a).expect("validated by validate_portals");
        let b = ir.room(&portal.b).expect("validated by validate_portals");
        let spans = facing_spans(&a.footprint, &b.footprint);
        let wall = find_facing_span(&spans, portal.at).map(|s| {
            let across = if portal.a == room { s.near } else { s.far };
            (s.axis, across)
        });
        if wall == Some((axis, fixed)) {
            let (along, _) = axis.split(portal.at);
            let half = portal.width / 2;
            out.push((
                format!("portal `{}` <-> `{}`", portal.a, portal.b),
                (along - half, along + half),
            ));
        }
    }
    for exit in ir.exits.iter().filter(|e| e.room == room) {
        let host = ir.room(&exit.room).expect("validated by validate_exits");
        let wall = wall_edges(&host.footprint)
            .find(|&(a, f, lo, hi, _)| {
                let (along, across) = a.split(exit.at);
                across == f && along > lo && along < hi
            })
            .map(|(a, f, _, _, _)| (a, f));
        if wall == Some((axis, fixed)) {
            let (along, _) = axis.split(exit.at);
            let half = exit.width / 2;
            out.push((
                format!("exit in room `{}`", exit.room),
                (along - half, along + half),
            ));
        }
    }
    out
}

/// Where a wall pad's point cuts its room's wall: `(axis, fixed coordinate,
/// whether the host's own edge runs in the increasing-`along` direction, the
/// cut's low end, its high end)`, or `None` when the point is on no
/// axis-aligned wall or its 64-unit span runs past that wall's high end.
///
/// The point is the span's **start**, not its midpoint: the cut runs
/// `[along, along + PAD_SIZE]`. Only the high end can overrun, since the
/// wall lookup already requires the start to lie strictly inside the wall's
/// own extent — a start exactly on a corner would belong to two edges at
/// once.
///
/// Shared by [`pad_square`] and `compile::teleports::resolve_pad`, which
/// would otherwise re-derive the same wall lookup from the same predicate
/// and could drift apart.
pub(crate) fn wall_cut(room: &Room, at: Pt) -> Option<(Axis, i32, bool, i32, i32)> {
    let (axis, fixed, _, hi, forward) =
        wall_edges(&room.footprint).find(|&(axis, fixed, lo, hi, _)| {
            let (along, across) = axis.split(at);
            across == fixed && along > lo && along < hi
        })?;
    let (along, _) = axis.split(at);
    let (open_lo, open_hi) = (along, along + Ir::PAD_SIZE);
    if open_hi > hi {
        return None;
    }
    Some((axis, fixed, forward, open_lo, open_hi))
}

/// The axis-aligned square a pad occupies once emitted, as `(low corner,
/// high corner)`, or `None` for a wall pad whose point is on no axis-aligned
/// wall or whose 64-unit span runs past that wall's far end.
///
/// An island pad's point *is* the low corner, so its arm is pure addition;
/// see [`PadPlacement`].
///
/// Shared by [`Ir::from_json`]'s validation and `compile::teleports`, so the
/// two can never disagree about where a pad is — the same reason
/// [`crate::geom::facing_spans`] is shared between portal validation and
/// portal cutting.
pub(crate) fn pad_square(room: &Room, pad: PadPlacement) -> Option<(Pt, Pt)> {
    match pad {
        PadPlacement::Island(c) => Some((
            c,
            Pt {
                x: c.x + Ir::PAD_SIZE,
                y: c.y + Ir::PAD_SIZE,
            },
        )),
        PadPlacement::Wall(at) => {
            let (axis, fixed, forward, open_lo, open_hi) = wall_cut(room, at)?;
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
    use super::{
        ExitTrigger, FloorFamilyIr, Ir, IrError, LiftSpeed, LiftTrigger, PortalKind, Pt,
        RevealKind, TriggerKind, destination_sector_key, pad_square,
    };

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

    const ISLAND: &str = r#"{ "id":"t1", "room":"a", "pad":{"island":[64,128]},
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
        assert_eq!(
            (lo, hi),
            (Pt { x: 64, y: 128 }, Pt { x: 128, y: 192 }),
            "the authored point is the square's low corner"
        );
        assert_eq!(Ir::PAD_SIZE, 64);
        assert_eq!(Ir::PAD_FLOOR_STEP, 8);
        assert_eq!(Ir::FLAT_TILE, 64);
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
            (Pt { x: 64, y: 256 }, Pt { x: 128, y: 320 }),
            "north wall, span starting at the authored point, recess to +y"
        );
    }

    /// The other half of [`pad_square`]'s wall arm: a pad on a wall whose
    /// **x** is fixed. The test above pins the horizontal (north) wall; this
    /// one pins the vertical (west) wall, where the recess runs along -x and
    /// the square's span is the y one.
    #[test]
    fn a_wall_pad_on_a_vertical_wall_recesses_along_x() {
        let ir = with_teleports(
            r#"{ "id":"w", "room":"a", "pad":{"wall":[0,128]},
                 "to":{"room":"b","at":[448,128],"angle":90} }"#,
        )
        .expect("parses");
        let (lo, hi) = pad_square(&ir.rooms[0], ir.teleports[0].pad).expect("wall square");
        assert_eq!(
            (lo, hi),
            (Pt { x: -64, y: 128 }, Pt { x: 0, y: 192 }),
            "west wall, recess to -x"
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
        // On the flat grid, so the geometric rule is the only one that can
        // fire: the square's west edge lies exactly on room `a`'s own.
        let err = with_teleports(
            r#"{ "id":"t", "room":"a", "pad":{"island":[0,128]},
                 "to":{"room":"b","at":[448,128],"angle":90} }"#,
        )
        .unwrap_err();
        assert!(
            matches!(err, IrError::TeleportPadOutsideRoom { .. }),
            "{err}"
        );
    }

    /// The `(authored point, square's low corner)` an off-flat-grid rejection
    /// reports, or `None` for any other outcome — a clean parse included.
    ///
    /// The two points differ only for a pad recessed toward -x or -y, where
    /// the square's low corner is the recess's *far* corner and appears
    /// nowhere in the IR. Reporting both is what keeps the message pointing at
    /// something the author actually wrote.
    fn off_flat_grid(result: &Result<Ir, IrError>) -> Option<((i32, i32), (i32, i32))> {
        match result {
            Err(IrError::TeleportPadOffFlatGrid {
                at_x, at_y, x, y, ..
            }) => Some(((*at_x, *at_y), (*x, *y))),
            _ => None,
        }
    }

    /// Half a tile off on both axes: the very address this pad had before the
    /// IR moved to corners, and the offset vanilla's world-space flat wrap
    /// turns into four quarter-tiles. The same square addressed by its corner
    /// is the control.
    #[test]
    fn an_island_pad_half_a_tile_off_the_flat_grid_is_rejected() {
        assert_eq!(
            off_flat_grid(&with_teleports(
                r#"{ "id":"t", "room":"a", "pad":{"island":[96,160]},
                     "to":{"room":"b","at":[448,128],"angle":90} }"#
            )),
            Some(((96, 160), (96, 160))),
            "an island pad's authored point is its square's low corner"
        );
        assert_eq!(
            off_flat_grid(&with_teleports(ISLAND)),
            None,
            "the same square addressed by its corner parses"
        );
    }

    /// The wall arm's `along` half: the span starts 32 units into a tile,
    /// even though the wall it is cut into is itself on the grid.
    #[test]
    fn a_wall_pad_whose_span_starts_off_the_flat_grid_is_rejected() {
        assert_eq!(
            off_flat_grid(&with_teleports(
                r#"{ "id":"w", "room":"a", "pad":{"wall":[96,256]},
                     "to":{"room":"b","at":[448,128],"angle":90} }"#
            )),
            Some(((96, 256), (96, 256))),
            "a pad recessed toward +y keeps the authored point as its low corner"
        );
    }

    /// The other half of the reported pair: this pad is recessed toward -x,
    /// so its square's low corner (-64, 96) is the recess's far corner and
    /// appears nowhere in the IR. The error names the authored point (0, 96)
    /// as well, so the message points at something the author wrote.
    #[test]
    fn a_wall_pad_recessed_backward_reports_both_its_point_and_its_corner() {
        assert_eq!(
            off_flat_grid(&with_teleports(
                r#"{ "id":"w", "room":"a", "pad":{"wall":[0,96]},
                     "to":{"room":"b","at":[448,128],"angle":90} }"#
            )),
            Some(((0, 96), (-64, 96)))
        );
    }

    /// The wall arm's *across* half, which the span alone cannot express:
    /// room `a`'s north wall sits at y = 264, so the recess's own low corner
    /// is 8 units off the tile grid however the span is placed.
    #[test]
    fn a_wall_pad_on_a_wall_off_the_flat_grid_is_rejected() {
        let json = r#"{ "seed":1, "grid":8, "theme":"tech_base",
          "rooms":[
            { "id":"a", "footprint":[[0,0],[0,264],[256,264],[256,0]],
              "floor":0, "ceiling":128, "light":160,
              "floor_tex":"FLOOR4_8", "ceil_tex":"CEIL3_5", "wall_tex":"STARTAN3",
              "things":[ { "kind":"player1_start", "at":[192,64], "angle":90 } ] },
            { "id":"b", "footprint":[[320,0],[320,264],[576,264],[576,0]],
              "floor":0, "ceiling":128, "light":160,
              "floor_tex":"FLOOR4_8", "ceil_tex":"CEIL3_5", "wall_tex":"STARTAN3" }
          ],
          "portals":[],
          "teleports":[
            { "id":"w", "room":"a", "pad":{"wall":[64,264]},
              "to":{"room":"b","at":[448,128],"angle":90} }
          ] }"#;
        assert_eq!(
            off_flat_grid(&Ir::from_json(json)),
            Some(((64, 264), (64, 264)))
        );
    }

    #[test]
    fn two_island_pads_touching_each_other_are_rejected() {
        // Both teleports deliver to the same point, so
        // `TeleportDestinationsShareSector` cannot fire and the overlap rule
        // is the only one under test.
        let err = with_teleports(
            r#"{ "id":"t1", "room":"a", "pad":{"island":[64,64]}, "to":{"room":"b","at":[448,128],"angle":90} },
               { "id":"t2", "room":"a", "pad":{"island":[128,64]}, "to":{"room":"b","at":[448,128],"angle":90} }"#,
        )
        .unwrap_err();
        assert!(matches!(err, IrError::TeleportPadsOverlap { .. }), "{err}");
    }

    #[test]
    fn duplicate_ids_and_unknown_rooms_are_rejected() {
        let err = with_teleports(&format!("{ISLAND}, {ISLAND}")).unwrap_err();
        assert!(matches!(err, IrError::DuplicateTeleport { .. }), "{err}");
        let err = with_teleports(
            r#"{ "id":"t", "room":"zzz", "pad":{"island":[64,128]}, "to":{"room":"b","at":[448,128],"angle":90} }"#,
        )
        .unwrap_err();
        assert!(matches!(err, IrError::TeleportUnknownRoom { .. }), "{err}");
    }

    #[test]
    fn a_destination_outside_its_room_is_rejected() {
        let err = with_teleports(
            r#"{ "id":"t", "room":"a", "pad":{"island":[64,128]}, "to":{"room":"b","at":[100,100],"angle":90} }"#,
        )
        .unwrap_err();
        assert!(
            matches!(err, IrError::TeleportDestinationOutsideRoom { .. }),
            "{err}"
        );
    }

    #[test]
    fn distinct_destinations_in_one_sector_are_rejected_but_identical_ones_share() {
        // One pad per room: two 64-unit pads on the flat grid cannot both
        // fit inside a 256x256 room without touching, and touching is its
        // own error. Both teleports still deliver into room `b` proper —
        // neither point lands on `t2`'s own pad, which spans 448..512 x
        // 128..192.
        let err = with_teleports(
            r#"{ "id":"t1", "room":"a", "pad":{"island":[64,128]}, "to":{"room":"b","at":[448,64],"angle":90} },
               { "id":"t2", "room":"b", "pad":{"island":[448,128]}, "to":{"room":"b","at":[384,192],"angle":0} }"#,
        )
        .unwrap_err();
        assert!(
            matches!(err, IrError::TeleportDestinationsShareSector { .. }),
            "{err}"
        );
        with_teleports(
            r#"{ "id":"t1", "room":"a", "pad":{"island":[64,128]}, "to":{"room":"b","at":[448,64],"angle":90} },
               { "id":"t2", "room":"b", "pad":{"island":[448,128]}, "to":{"room":"b","at":[448,64],"angle":90} }"#,
        )
        .expect("identical destinations share one marker");
    }

    #[test]
    fn a_destination_on_another_pad_keys_to_that_pad() {
        let ir = with_teleports(
            r#"{ "id":"t1", "room":"a", "pad":{"island":[64,128]}, "to":{"room":"b","at":[480,160],"angle":90} },
               { "id":"t2", "room":"b", "pad":{"island":[448,128]}, "to":{"room":"a","at":[96,160],"angle":0} }"#,
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

    #[test]
    fn a_wall_pad_whose_span_overruns_the_wall_is_rejected() {
        // (224, 256) is on room a's north wall (fixed=256, span 0..256), so
        // the `wall_edges().find(..)` lookup itself succeeds — unlike
        // `a_wall_pad_off_any_wall_is_rejected`'s [64,64], which is on no
        // wall at all. What fails here is the second, narrower check: a
        // 64-wide span starting at along=224 ends at 288, past the wall's
        // own hi=256 end.
        let err = with_teleports(
            r#"{ "id":"w", "room":"a", "pad":{"wall":[224,256]},
                 "to":{"room":"b","at":[448,128],"angle":90} }"#,
        )
        .unwrap_err();
        assert!(matches!(err, IrError::TeleportPadOffWall { .. }), "{err}");
    }

    #[test]
    fn an_island_pad_enclosing_a_concave_room_vertex_is_rejected() {
        // Room `a` is a 256x256 square with a rectangular notch bitten out
        // of the south wall (x in 144..176, up to y=144), so it has two
        // reflex vertices at (144,144) and (176,144). The pad at corner
        // (128,128) spans (128,128)-(192,192): all four corners test as
        // strictly inside the room (contains + positive clearance) — each is
        // 16 units clear of the notch — yet both reflex vertices land
        // strictly inside the pad square, so the pad would straddle solid
        // wall that the four-corner check alone cannot see.
        let json = r#"{ "seed":1, "grid":16, "theme":"tech_base",
          "rooms":[
            { "id":"a",
              "footprint":[[0,0],[0,256],[256,256],[256,0],[176,0],[176,144],[144,144],[144,0]],
              "floor":0, "ceiling":128, "light":160,
              "floor_tex":"FLOOR4_8", "ceil_tex":"CEIL3_5", "wall_tex":"STARTAN3",
              "things":[ { "kind":"player1_start", "at":[192,64], "angle":90 } ] },
            { "id":"b", "footprint":[[320,0],[320,256],[576,256],[576,0]],
              "floor":0, "ceiling":128, "light":160,
              "floor_tex":"FLOOR4_8", "ceil_tex":"CEIL3_5", "wall_tex":"STARTAN3" }
          ],
          "portals":[],
          "teleports":[
            { "id":"t", "room":"a", "pad":{"island":[128,128]},
              "to":{"room":"b","at":[448,128],"angle":90} }
          ] }"#;
        let err = Ir::from_json(json).unwrap_err();
        assert!(
            matches!(err, IrError::TeleportPadOutsideRoom { .. }),
            "{err}"
        );
    }

    /// The slab case neither the four-corner test nor the vertex test sees.
    /// Room `a` is a 256x256 square with a 32-wide spur of solid material
    /// driven up from its south wall to y = 224, so the room's own boundary
    /// runs clean *through* the pad square (128,128)-(192,192): the spur's
    /// four vertices all sit outside that square (two at y = 0, two at
    /// y = 224) and all four pad corners sit strictly inside the room, one
    /// pair on each side of the spur. Before the edge test this IR passed
    /// `Ir::from_json` and every compile pass, and died in the nodebuilder
    /// at pack time.
    #[test]
    fn an_island_pad_a_wall_runs_through_is_rejected() {
        let json = r#"{ "seed":1, "grid":16, "theme":"tech_base",
          "rooms":[
            { "id":"a",
              "footprint":[[0,0],[0,256],[256,256],[256,0],[176,0],[176,224],[144,224],[144,0]],
              "floor":0, "ceiling":128, "light":160,
              "floor_tex":"FLOOR4_8", "ceil_tex":"CEIL3_5", "wall_tex":"STARTAN3",
              "things":[ { "kind":"player1_start", "at":[192,64], "angle":90 } ] },
            { "id":"b", "footprint":[[320,0],[320,256],[576,256],[576,0]],
              "floor":0, "ceiling":128, "light":160,
              "floor_tex":"FLOOR4_8", "ceil_tex":"CEIL3_5", "wall_tex":"STARTAN3" }
          ],
          "portals":[],
          "teleports":[
            { "id":"t", "room":"a", "pad":{"island":[128,128]},
              "to":{"room":"b","at":[448,128],"angle":90} }
          ] }"#;
        let err = Ir::from_json(json).unwrap_err();
        assert!(
            matches!(err, IrError::TeleportPadOutsideRoom { .. }),
            "{err}"
        );
    }

    /// [`TELEPORT_BASE`] with room `b` moved 8 units further east and
    /// `openings` written in place of its empty `portals` list. A wall pad
    /// recessed 64 deep into room `a`'s east wall then clears `b` by exactly
    /// [`Ir::MIN_PORTAL_GAP`], which isolates the on-wall opening checks
    /// from the neighbor-proximity one.
    fn east_wall_pad(openings: &str, list: &str) -> Result<Ir, IrError> {
        Ir::from_json(
            &TELEPORT_BASE
                .replace(r#""grid":64"#, r#""grid":8"#)
                .replace(
                    "[[320,0],[320,256],[576,256],[576,0]]",
                    "[[328,0],[328,256],[584,256],[584,0]]",
                )
                .replace(r#""portals":[],"#, openings)
                .replace("TELEPORTS", list),
        )
    }

    const EAST_WALL_PAD: &str = r#"{ "id":"w", "room":"a", "pad":{"wall":[256,128]},
        "to":{"room":"b","at":[448,128],"angle":90} }"#;

    #[test]
    fn a_wall_pad_whose_recess_lands_flush_on_another_room_is_rejected() {
        // Room `b`'s west wall is 64 east of room `a`'s east wall, which is
        // exactly the recess depth: the pad's far wall and `b`'s own wall
        // would be one line carrying two one-sided linedefs.
        let err = with_teleports(EAST_WALL_PAD).unwrap_err();
        assert!(
            matches!(err, IrError::TeleportPadRecessTooClose { .. }),
            "{err}"
        );
    }

    #[test]
    fn a_wall_pad_leaving_the_minimum_gap_to_the_next_room_is_accepted() {
        // The same pad with `b` 72 units away: 64 of recess plus exactly
        // `MIN_PORTAL_GAP` of wall, the thinnest a portal gap may be.
        east_wall_pad(r#""portals":[],"#, EAST_WALL_PAD).expect("8 units of wall is enough");
    }

    /// A plain portal on room `a`'s east wall, 64 wide about (256,96) — an
    /// opening spanning y 64..128.
    const EAST_PORTAL: &str = r#""portals":[
        { "a":"a", "b":"b", "kind":"plain", "width":64, "at":[256,96] }],"#;

    #[test]
    fn a_wall_pad_touching_a_portal_opening_on_the_same_wall_is_rejected() {
        // [`EAST_WALL_PAD`]'s span 128..192 meets the opening's high end
        // exactly, which would leave the recess's south jamb coincident with
        // the passage's north one.
        let err = east_wall_pad(EAST_PORTAL, EAST_WALL_PAD).unwrap_err();
        assert!(
            matches!(err, IrError::TeleportPadBesideOpening { .. }),
            "{err}"
        );
        // The same opening read from room `b`'s side of the gap — the
        // `FacingSpan::far` half of the lookup — with `b`'s own west wall
        // carrying the pad. That wall sits at x = 328, off the flat grid, so
        // this case also pins the check order: the opening conflict is the
        // authoring mistake worth reporting, not the alignment one.
        let err = east_wall_pad(
            EAST_PORTAL,
            r#"{ "id":"w", "room":"b", "pad":{"wall":[328,128]},
                 "to":{"room":"a","at":[128,128],"angle":90} }"#,
        )
        .unwrap_err();
        assert!(
            matches!(err, IrError::TeleportPadBesideOpening { .. }),
            "{err}"
        );
    }

    #[test]
    fn a_wall_pad_clear_of_the_portal_opening_on_its_wall_is_accepted() {
        // Span 192..256, a full 64 units clear of the opening's 64..128.
        east_wall_pad(
            EAST_PORTAL,
            r#"{ "id":"w", "room":"a", "pad":{"wall":[256,192]},
                 "to":{"room":"b","at":[448,128],"angle":90} }"#,
        )
        .expect("clear of the opening");
    }

    #[test]
    fn a_wall_pad_touching_an_exit_span_on_the_same_wall_is_rejected() {
        // The exit spans y 64..128 on room `a`'s east wall; the pad's span
        // 128..192 meets it exactly.
        let err = east_wall_pad(
            r#""portals":[],
               "exits":[{ "room":"a", "trigger":"walkover", "at":[256,96], "width":64 }],"#,
            EAST_WALL_PAD,
        )
        .unwrap_err();
        assert!(
            matches!(err, IrError::TeleportPadBesideOpening { .. }),
            "{err}"
        );
    }

    #[test]
    fn a_wall_pad_clear_of_the_exit_span_on_its_wall_is_accepted() {
        // The same exit, with the pad one tile further along the wall:
        // span 192..256 against the exit's 64..128.
        east_wall_pad(
            r#""portals":[],
               "exits":[{ "room":"a", "trigger":"walkover", "at":[256,96], "width":64 }],"#,
            r#"{ "id":"w", "room":"a", "pad":{"wall":[256,192]},
                 "to":{"room":"b","at":[448,128],"angle":90} }"#,
        )
        .expect("clear of the exit segment");
    }

    #[test]
    fn a_teleport_pad_coordinate_outside_the_binary_map_range_is_rejected() {
        let err = with_teleports(
            r#"{ "id":"t", "room":"a", "pad":{"island":[64,40000]},
                 "to":{"room":"b","at":[448,128],"angle":90} }"#,
        )
        .unwrap_err();
        assert!(matches!(err, IrError::CoordinateOutOfRange { .. }), "{err}");
    }

    #[test]
    fn a_teleport_destination_outside_the_binary_map_range_is_rejected() {
        // Checked before containment, so the range error is what a caller
        // sees rather than the vaguer "outside room `b`" it would otherwise
        // fall through to.
        let err = with_teleports(
            r#"{ "id":"t", "room":"a", "pad":{"island":[64,128]},
                 "to":{"room":"b","at":[448,40000],"angle":90} }"#,
        )
        .unwrap_err();
        assert!(matches!(err, IrError::CoordinateOutOfRange { .. }), "{err}");
    }

    const LIFT_BASE: &str = r#"{ "seed":1, "grid":64, "theme":"tech_base",
      "rooms":[
        { "id":"a", "footprint":[[0,0],[0,256],[256,256],[256,0]], "floor":0, "ceiling":128, "light":160,
          "floor_tex":"FLOOR4_8", "ceil_tex":"CEIL3_5", "wall_tex":"STARTAN3" },
        { "id":"b", "footprint":[[320,0],[320,256],[576,256],[576,0]], "floor":128, "ceiling":256, "light":160,
          "floor_tex":"FLOOR4_8", "ceil_tex":"CEIL3_5", "wall_tex":"STARTAN3" }
      ],
      "portals":[ { "a":"a", "b":"b", "kind":"lift", "width":64, "at":[256,128] } ]
    }"#;

    fn with(json: &str, from: &str, to: &str) -> String {
        assert!(json.contains(from), "fixture must contain {from}");
        json.replacen(from, to, 1)
    }

    #[test]
    fn a_lift_portal_parses_with_its_defaults() {
        let ir = Ir::from_json(LIFT_BASE).expect("parses");
        let p = &ir.portals[0];
        assert_eq!(p.kind, PortalKind::Lift);
        assert_eq!(p.speed, LiftSpeed::Normal);
        assert_eq!(p.trigger, LiftTrigger::Switch);
        assert_eq!(p.rise, None);
        assert!(!ir.is_barrier(p));
        assert_eq!(
            ir.low_room_of(p),
            Some(0),
            "room a, floor 0, is the low room"
        );
        assert!(ir.pedestals.is_empty());
    }

    #[test]
    fn a_lift_rejects_door_thickness_but_accepts_alcoves() {
        let json = with(
            LIFT_BASE,
            r#""at":[256,128] }"#,
            r#""at":[256,128], "door_thickness":32 }"#,
        );
        assert!(matches!(
            Ir::from_json(&json),
            Err(IrError::DoorThicknessOnLift { .. })
        ));
        for depth in Ir::LIFT_ALCOVE_DIMENSIONS {
            let json = with(
                LIFT_BASE,
                r#""at":[256,128] }"#,
                &format!(r#""at":[256,128], "alcove_near":{depth} }}"#),
            );
            assert!(
                Ir::from_json(&json).is_ok(),
                "an alcove on a lift is a buffer or a walkover strip, and {depth} is a legal depth"
            );
        }
        // 64 is the one depth a lift admits and a door does not, so the two
        // sets are genuinely different rather than one widened in place.
        assert!(!Ir::DOOR_DIMENSIONS.contains(&64) && Ir::LIFT_ALCOVE_DIMENSIONS.contains(&64));
        let json = with(
            LIFT_BASE,
            r#""at":[256,128] }"#,
            r#""at":[256,128], "alcove_near":12 }"#,
        );
        assert!(matches!(
            Ir::from_json(&json),
            Err(IrError::InvalidLiftAlcoveDimension {
                field: "alcove_near",
                value: 12,
                ..
            })
        ));
    }

    #[test]
    fn rise_is_rejected_on_non_lift_portals() {
        let plain = ir_with_gap(8).replace(r#""at":[64,32] }"#, r#""at":[64,32], "rise":96 }"#);
        assert!(matches!(
            Ir::from_json(&plain),
            Err(IrError::RiseOnNonLiftPortal { .. })
        ));
        let door = ir_with_door(32, r#", "door_thickness":32, "rise":96"#);
        assert!(matches!(
            Ir::from_json(&door),
            Err(IrError::RiseOnNonLiftPortal { .. })
        ));
    }

    #[test]
    fn rise_is_required_on_equal_floors_and_rejected_on_unequal_ones() {
        let json = with(
            LIFT_BASE,
            r#""at":[256,128] }"#,
            r#""at":[256,128], "rise":96 }"#,
        );
        assert!(matches!(
            Ir::from_json(&json),
            Err(IrError::LiftRiseOnUnequalFloors { .. })
        ));
        let equal = with(
            LIFT_BASE,
            r#""floor":128, "ceiling":256"#,
            r#""floor":0, "ceiling":256"#,
        );
        assert!(matches!(
            Ir::from_json(&equal),
            Err(IrError::BarrierMissingRise { .. })
        ));
        let barrier = with(
            &equal,
            r#""at":[256,128] }"#,
            r#""at":[256,128], "rise":96 }"#,
        );
        let ir = Ir::from_json(&barrier).expect("a barrier");
        assert!(ir.is_barrier(&ir.portals[0]));
        assert_eq!(ir.low_room_of(&ir.portals[0]), None);
    }

    #[test]
    fn a_walkover_lift_needs_the_low_rooms_alcove() {
        let json = with(
            LIFT_BASE,
            r#""at":[256,128] }"#,
            r#""at":[256,128], "trigger":"walkover" }"#,
        );
        assert!(matches!(
            Ir::from_json(&json),
            Err(IrError::LiftWalkoverNeedsAlcove {
                field: "alcove_near",
                ..
            })
        ));
        let json = with(
            LIFT_BASE,
            r#""at":[256,128] }"#,
            r#""at":[256,128], "trigger":"walkover", "alcove_far":16 }"#,
        );
        assert!(
            matches!(
                Ir::from_json(&json),
                Err(IrError::LiftWalkoverNeedsAlcove { .. })
            ),
            "the alcove must be on the low room's side"
        );
        let json = with(
            LIFT_BASE,
            r#""at":[256,128] }"#,
            r#""at":[256,128], "trigger":"walkover", "alcove_near":16 }"#,
        );
        assert!(Ir::from_json(&json).is_ok());
    }

    #[test]
    fn a_both_ends_lift_needs_no_alcove() {
        // Unlike `walkover`, `both_ends`'s second trigger line sits on the
        // platform's own top face, not in the low room's alcove.
        let json = with(
            LIFT_BASE,
            r#""at":[256,128] }"#,
            r#""at":[256,128], "trigger":"both_ends" }"#,
        );
        assert!(Ir::from_json(&json).is_ok());
    }

    #[test]
    fn a_barrier_offers_only_the_switch_trigger() {
        let equal = with(
            LIFT_BASE,
            r#""floor":128, "ceiling":256"#,
            r#""floor":0, "ceiling":256"#,
        );
        for trigger in ["walkover", "both_ends"] {
            let json = with(
                &equal,
                r#""at":[256,128] }"#,
                &format!(r#""at":[256,128], "rise":96, "trigger":"{trigger}" }}"#),
            );
            assert!(
                matches!(Ir::from_json(&json), Err(IrError::BarrierTrigger { .. })),
                "{trigger}"
            );
        }
    }

    const PEDESTAL_BASE: &str = r#"{ "seed":1, "grid":64, "theme":"tech_base",
      "rooms":[
        { "id":"a", "footprint":[[0,0],[0,512],[512,512],[512,0]], "floor":0, "ceiling":256, "light":160,
          "floor_tex":"FLOOR4_8", "ceil_tex":"CEIL3_5", "wall_tex":"STARTAN3" }
      ],
      "portals":[],
      "pedestals":[ { "id":"p", "room":"a", "at":[128,128], "rise":128,
                      "things":[ { "kind":"medikit", "at":[160,160], "angle":0 } ] } ]
    }"#;

    #[test]
    fn a_pedestal_parses_with_its_defaults_and_rect() {
        let ir = Ir::from_json(PEDESTAL_BASE).expect("parses");
        let p = &ir.pedestals[0];
        assert_eq!(p.size, None);
        assert_eq!(p.speed, LiftSpeed::Normal);
        assert_eq!(p.rect(), (Pt { x: 128, y: 128 }, Pt { x: 192, y: 192 }));
    }

    #[test]
    fn pedestal_structural_errors() {
        let dup = PEDESTAL_BASE.replacen(
            r#""pedestals":[ "#,
            r#""pedestals":[ { "id":"p", "room":"a", "at":[320,320], "rise":64 }, "#,
            1,
        );
        assert!(matches!(
            Ir::from_json(&dup),
            Err(IrError::DuplicatePedestal { .. })
        ));
        let unknown = with(
            PEDESTAL_BASE,
            r#""room":"a", "at":[128,128]"#,
            r#""room":"zz", "at":[128,128]"#,
        );
        assert!(matches!(
            Ir::from_json(&unknown),
            Err(IrError::PedestalUnknownRoom { .. })
        ));
        let zero = with(PEDESTAL_BASE, r#""rise":128"#, r#""rise":0"#);
        assert!(matches!(
            Ir::from_json(&zero),
            Err(IrError::PedestalRiseNotPositive { .. })
        ));
        let odd = with(
            PEDESTAL_BASE,
            r#""rise":128,"#,
            r#""rise":128, "size":[60,64],"#,
        );
        assert!(matches!(
            Ir::from_json(&odd),
            Err(IrError::PedestalSizeNotMultipleOf8 { .. })
        ));
        let outside = with(PEDESTAL_BASE, r#""at":[128,128]"#, r#""at":[480,128]"#);
        assert!(
            matches!(
                Ir::from_json(&outside),
                Err(IrError::PedestalOutsideRoom { .. })
            ),
            "the rect must clear the wall"
        );
        let thing_out = with(PEDESTAL_BASE, r#""at":[160,160]"#, r#""at":[200,160]"#);
        assert!(matches!(
            Ir::from_json(&thing_out),
            Err(IrError::PedestalThingOutside { .. })
        ));
        let overlap = PEDESTAL_BASE.replacen(
            r#""pedestals":[ "#,
            r#""pedestals":[ { "id":"q", "room":"a", "at":[192,128], "rise":64 }, "#,
            1,
        );
        assert!(
            matches!(
                Ir::from_json(&overlap),
                Err(IrError::PedestalsOverlap { .. })
            ),
            "touching counts as overlap, as for pads"
        );
    }

    #[test]
    fn a_pedestal_whose_corner_would_overflow_is_rejected_not_panicked() {
        // `at` alone is already outside the binary map range: caught before
        // `rect()` is ever called, so `at + size` (the default 64x64) never
        // executes.
        let huge_at = with(PEDESTAL_BASE, r#""at":[128,128]"#, r#""at":[2147483647,0]"#);
        assert!(matches!(
            Ir::from_json(&huge_at),
            Err(IrError::CoordinateOutOfRange { .. })
        ));
        // `at` is in range and `size` is a legal positive multiple of 8, so
        // only the high corner's own `at + size` sum — computed with
        // checked, not saturating, arithmetic — catches this one.
        let small_at = with(PEDESTAL_BASE, r#""at":[128,128]"#, r#""at":[8,8]"#);
        let huge_size = with(
            &small_at,
            r#""rise":128,"#,
            r#""rise":128, "size":[2147483640,8],"#,
        );
        assert!(matches!(
            Ir::from_json(&huge_size),
            Err(IrError::CoordinateOutOfRange { .. })
        ));
    }

    /// The pad rule's companion for the *destination*: a point, which
    /// [`IrError::PedestalsOverlap`]'s rectangle-against-rectangle test
    /// cannot see. The pad itself sits at (320, 320), well clear of the
    /// pedestal's (128, 128)..(192, 192), so only the destination is at
    /// issue in either case.
    #[test]
    fn a_teleport_may_not_deliver_onto_a_pedestal() {
        let with_destination = |at: &str| {
            PEDESTAL_BASE.replacen(
                r#""portals":[],"#,
                &format!(
                    r#""portals":[], "teleports":[ {{ "id":"t", "room":"a", "pad":{{"island":[320,320]}}, "to":{{"room":"a","at":{at},"angle":0}} }} ],"#
                ),
                1,
            )
        };
        assert!(matches!(
            Ir::from_json(&with_destination("[160,160]")),
            Err(IrError::TeleportDestinationOnPedestal { .. })
        ));
        Ir::from_json(&with_destination("[400,400]"))
            .expect("a destination clear of the pedestal parses");
    }

    #[test]
    fn a_pedestal_and_a_teleport_pad_may_not_overlap() {
        let json = PEDESTAL_BASE.replacen(
            r#""portals":[],"#,
            r#""portals":[], "teleports":[ { "id":"t", "room":"a", "pad":{"island":[128,192]}, "to":{"room":"a","at":[400,400],"angle":0} } ],"#,
            1,
        );
        assert!(matches!(
            Ir::from_json(&json),
            Err(IrError::PedestalsOverlap { .. })
        ));
    }

    /// Three rooms in a row, each authored 64 units clear of the next:
    /// `a` (0..256) — `b` (320..576) — `c` (640..896), all at floor 0. The
    /// floor-action tests substitute their own portals, triggers and reveals
    /// into it, so each one reads as the construct it is about.
    const FLOORS_BASE: &str = r#"{ "seed":1, "grid":64, "theme":"tech_base",
      "rooms":[
        { "id":"a", "footprint":[[0,0],[0,256],[256,256],[256,0]], "floor":0, "ceiling":192, "light":160,
          "floor_tex":"FLOOR4_8", "ceil_tex":"CEIL3_5", "wall_tex":"STARTAN3",
          "things":[ { "kind":"player1_start", "at":[128,128], "angle":0 } ] },
        { "id":"b", "footprint":[[320,0],[320,256],[576,256],[576,0]], "floor":0, "ceiling":192, "light":160,
          "floor_tex":"FLOOR4_8", "ceil_tex":"CEIL3_5", "wall_tex":"STARTAN3" },
        { "id":"c", "footprint":[[640,0],[640,256],[896,256],[896,0]], "floor":0, "ceiling":192, "light":160,
          "floor_tex":"FLOOR4_8", "ceil_tex":"CEIL3_5", "wall_tex":"STARTAN3" }
      ],
      "portals":[ {PORTALS} ],
      "triggers":[ {TRIGGERS} ],
      "reveals":[ {REVEALS} ] }"#;

    /// [`FLOORS_BASE`] with its three lists filled in.
    fn floors(portals: &str, triggers: &str, reveals: &str) -> Result<Ir, IrError> {
        Ir::from_json(
            &FLOORS_BASE
                .replace("{PORTALS}", portals)
                .replace("{TRIGGERS}", triggers)
                .replace("{REVEALS}", reveals),
        )
    }

    const SWITCH_A: &str = r#"{ "id":"t", "kind":"switch", "room":"a", "at":[0,128] }"#;
    const WALK_AB: &str = r#"{ "id":"w", "kind":"walkover", "portal":["a","b"] }"#;
    const PLAIN_AB: &str = r#"{ "a":"a", "b":"b", "kind":"plain", "width":64, "at":[256,128] }"#;
    const WALL_BC: &str = r#"{ "a":"b", "b":"c", "kind":"drop_wall", "width":64, "at":[576,128], "thickness":16, "fires_on":"t" }"#;
    const BRIDGE_BC: &str = r#"{ "a":"b", "b":"c", "kind":"bridge", "width":64, "at":[576,128], "depth":96, "fires_on":"w" }"#;
    const CLOSET_C: &str = r#"{ "id":"pen", "room":"c", "at":[704,64], "kind":"closet",
        "things":[ { "kind":"imp", "at":[736,96], "angle":180 } ], "trigger":"t" }"#;

    #[test]
    fn a_drop_wall_with_a_switch_trigger_parses() {
        let ir = floors(&format!("{PLAIN_AB}, {WALL_BC}"), SWITCH_A, "").expect("parses");
        assert_eq!(ir.portals[1].kind, PortalKind::DropWall);
        assert_eq!(ir.portals[1].thickness, Some(16));
        assert_eq!(ir.portals[1].fires_on.as_deref(), Some("t"));
        assert_eq!(ir.triggers[0].kind, TriggerKind::Switch);
        assert_eq!(ir.trigger("t").map(|t| t.kind), Some(TriggerKind::Switch));
        assert!(ir.trigger("nobody").is_none());
        assert_eq!(ir.trigger_family("t"), Some(FloorFamilyIr::Lower));
        assert_eq!(ir.trigger_family("nobody"), None);
    }

    #[test]
    fn a_bridge_on_a_walkover_and_a_closet_on_the_switch_parse_together() {
        let ir = floors(
            &format!("{PLAIN_AB}, {BRIDGE_BC}"),
            &format!("{SWITCH_A}, {WALK_AB}"),
            CLOSET_C,
        )
        .expect("parses");
        assert_eq!(ir.trigger_family("w"), Some(FloorFamilyIr::Raise));
        assert_eq!(ir.trigger_family("t"), Some(FloorFamilyIr::Lower));
        assert_eq!(ir.reveals[0].kind, RevealKind::Closet);
        assert_eq!(
            ir.reveals[0].rect(),
            (Pt { x: 704, y: 64 }, Pt { x: 768, y: 128 })
        );
    }

    #[test]
    fn a_trigger_nothing_names_is_rejected() {
        let err = floors(PLAIN_AB, SWITCH_A, "").expect_err("unused trigger");
        assert!(
            matches!(err, IrError::TriggerUnused { ref id } if id == "t"),
            "{err}"
        );
    }

    #[test]
    fn a_construct_naming_an_unknown_trigger_is_rejected() {
        let err = floors(&format!("{PLAIN_AB}, {WALL_BC}"), "", "").expect_err("unknown trigger");
        assert!(
            matches!(err, IrError::UnknownTrigger { ref id, .. } if id == "t"),
            "{err}"
        );
    }

    #[test]
    fn a_trigger_driving_a_lowering_and_a_rising_construct_is_rejected() {
        // The bridge names the switch `t`, which the closet (lowering) names too.
        let bridge_on_t = BRIDGE_BC.replace("\"fires_on\":\"w\"", "\"fires_on\":\"t\"");
        let err = floors(&format!("{PLAIN_AB}, {bridge_on_t}"), SWITCH_A, CLOSET_C)
            .expect_err("mixed families");
        assert!(
            matches!(err, IrError::TriggerMixesFamilies { ref id } if id == "t"),
            "{err}"
        );
    }

    #[test]
    fn drop_wall_thickness_and_bridge_depth_are_validated() {
        let bad_thick = WALL_BC.replace("\"thickness\":16", "\"thickness\":24");
        let err = floors(&format!("{PLAIN_AB}, {bad_thick}"), SWITCH_A, "").expect_err("24");
        assert!(
            matches!(err, IrError::InvalidDropWallThickness { value: 24, .. }),
            "{err}"
        );
        let missing = WALL_BC.replace(", \"thickness\":16", "");
        let err = floors(&format!("{PLAIN_AB}, {missing}"), SWITCH_A, "").expect_err("none");
        assert!(
            matches!(err, IrError::MissingDropWallThickness { .. }),
            "{err}"
        );
        // Zero, not 24: the "deeper than a step" half of the rule needs the
        // step height, which is a table value `from_json` never loads, so
        // the IR only holds `depth` to a positive multiple of 8.
        let flat = BRIDGE_BC.replace("\"depth\":96", "\"depth\":0");
        let err = floors(&format!("{PLAIN_AB}, {flat}"), WALK_AB, "").expect_err("no pit at all");
        assert!(
            matches!(err, IrError::InvalidBridgeDepth { value: 0, .. }),
            "{err}"
        );
        let odd = BRIDGE_BC.replace("\"depth\":96", "\"depth\":100");
        let err =
            floors(&format!("{PLAIN_AB}, {odd}"), WALK_AB, "").expect_err("not a multiple of 8");
        assert!(
            matches!(err, IrError::InvalidBridgeDepth { value: 100, .. }),
            "{err}"
        );
    }

    #[test]
    fn a_walkover_must_name_a_plain_portal_and_a_switch_a_wall_point() {
        let on_wall = r#"{ "id":"w", "kind":"walkover", "portal":["b","c"] }"#;
        let err = floors(
            &format!("{PLAIN_AB}, {WALL_BC}"),
            &format!("{SWITCH_A}, {on_wall}"),
            "",
        )
        .expect_err("a walkover on a drop wall");
        assert!(
            matches!(err, IrError::WalkoverOnNonPlainPortal { ref id, .. } if id == "w"),
            "{err}"
        );
        let off_wall = r#"{ "id":"t", "kind":"switch", "room":"a", "at":[64,64] }"#;
        let err =
            floors(&format!("{PLAIN_AB}, {WALL_BC}"), off_wall, "").expect_err("inside the room");
        assert!(
            matches!(err, IrError::TriggerOffWall { ref id, .. } if id == "t"),
            "{err}"
        );
    }

    #[test]
    fn a_bridge_may_carry_its_own_walkover() {
        // The one bridge trigger that cannot strand: its special sits on the
        // pit's own thresholds, so whoever steps in fires the rise.
        let on_itself = r#"{ "id":"w", "kind":"walkover", "portal":["b","c"] }"#;
        let ir = floors(&format!("{PLAIN_AB}, {BRIDGE_BC}"), on_itself, "").expect("parses");
        assert_eq!(ir.trigger_family("w"), Some(FloorFamilyIr::Raise));
    }

    #[test]
    fn trigger_structural_errors() {
        let dup = format!("{SWITCH_A}, {SWITCH_A}");
        let err = floors(&format!("{PLAIN_AB}, {WALL_BC}"), &dup, "").expect_err("two `t`s");
        assert!(
            matches!(err, IrError::DuplicateTrigger { ref id } if id == "t"),
            "{err}"
        );
        for (trigger, needle) in [
            (
                r#"{ "id":"t", "kind":"switch" }"#,
                "a switch needs `room` and `at`",
            ),
            (
                r#"{ "id":"t", "kind":"switch", "room":"a", "at":[0,128], "portal":["a","b"] }"#,
                "a switch takes no `portal`",
            ),
            (
                r#"{ "id":"t", "kind":"switch", "room":"zz", "at":[0,128] }"#,
                "names unknown room `zz`",
            ),
            (
                r#"{ "id":"w", "kind":"walkover" }"#,
                "a walkover needs `portal: [a, b]`",
            ),
            (
                r#"{ "id":"w", "kind":"walkover", "portal":["a","b"], "room":"a" }"#,
                "a walkover takes no `room` or `at`",
            ),
            (
                r#"{ "id":"w", "kind":"walkover", "portal":["a","c"] }"#,
                "no portal joins `a` and `c`",
            ),
        ] {
            let err = floors(&format!("{PLAIN_AB}, {WALL_BC}"), trigger, "").expect_err(needle);
            let IrError::TriggerOffWall { ref detail, .. } = err else {
                panic!("{err}");
            };
            assert!(detail.contains(needle), "{err}");
        }
    }

    #[test]
    fn drop_wall_and_bridge_field_errors() {
        let no_trigger = WALL_BC.replace(", \"fires_on\":\"t\"", "");
        let err = floors(&format!("{PLAIN_AB}, {no_trigger}"), "", "").expect_err("no `fires_on`");
        assert!(
            matches!(
                err,
                IrError::ConstructWithoutTrigger {
                    kind: "drop_wall",
                    ..
                }
            ),
            "{err}"
        );
        let no_trigger = BRIDGE_BC.replace(", \"fires_on\":\"w\"", "");
        let err = floors(&format!("{PLAIN_AB}, {no_trigger}"), "", "").expect_err("no `fires_on`");
        assert!(
            matches!(err, IrError::ConstructWithoutTrigger { kind: "bridge", .. }),
            "{err}"
        );
        let no_depth = BRIDGE_BC.replace("\"depth\":96, ", "");
        let err = floors(&format!("{PLAIN_AB}, {no_depth}"), WALK_AB, "").expect_err("no depth");
        assert!(matches!(err, IrError::MissingBridgeDepth { .. }), "{err}");
        let uneven = FLOORS_BASE
            .replace("{PORTALS}", &format!("{PLAIN_AB}, {BRIDGE_BC}"))
            .replace("{TRIGGERS}", WALK_AB)
            .replace("{REVEALS}", "")
            .replace(
                r#""id":"c", "footprint":[[640,0],[640,256],[896,256],[896,0]], "floor":0"#,
                r#""id":"c", "footprint":[[640,0],[640,256],[896,256],[896,0]], "floor":32"#,
            );
        let err = Ir::from_json(&uneven).expect_err("floors differ by 32");
        assert!(
            matches!(
                err,
                IrError::BridgeFloorsDiffer {
                    floor_a: 0,
                    floor_b: 32,
                    ..
                }
            ),
            "{err}"
        );
        // A plain portal takes none of the three; a drop wall and a bridge
        // take each other's, so each names a trigger that has to exist.
        for (portals, triggers, field) in [
            (
                PLAIN_AB.replace("\"width\":64", "\"width\":64, \"thickness\":16"),
                "",
                "thickness",
            ),
            (
                PLAIN_AB.replace("\"width\":64", "\"width\":64, \"depth\":96"),
                "",
                "depth",
            ),
            (
                PLAIN_AB.replace("\"width\":64", "\"width\":64, \"fires_on\":\"t\""),
                "",
                "fires_on",
            ),
            (
                format!(
                    "{PLAIN_AB}, {}",
                    WALL_BC.replace("\"thickness\":16", "\"depth\":96")
                ),
                SWITCH_A,
                "depth",
            ),
            (
                format!(
                    "{PLAIN_AB}, {}",
                    BRIDGE_BC.replace("\"depth\":96", "\"thickness\":16")
                ),
                WALK_AB,
                "thickness",
            ),
        ] {
            let err = floors(&portals, triggers, "").expect_err(field);
            assert!(
                matches!(err, IrError::FloorFieldOnOtherPortal { field: f, .. } if f == field),
                "{err}"
            );
        }
    }

    #[test]
    fn a_pedestal_reveal_needs_a_positive_rise_and_a_closet_none() {
        let flat = CLOSET_C.replace("\"kind\":\"closet\"", "\"kind\":\"pedestal\", \"rise\":0");
        let err = floors(&format!("{PLAIN_AB}, {WALL_BC}"), SWITCH_A, &flat)
            .expect_err("a pedestal rises");
        assert!(
            matches!(err, IrError::RevealRiseNotPositive { rise: 0, .. }),
            "{err}"
        );
        let with_rise = CLOSET_C.replace("\"kind\":\"closet\"", "\"kind\":\"closet\", \"rise\":64");
        let err =
            floors(&format!("{PLAIN_AB}, {WALL_BC}"), SWITCH_A, &with_rise).expect_err("closet");
        assert!(matches!(err, IrError::RiseOnCloset { .. }), "{err}");
    }

    #[test]
    fn reveal_structural_errors() {
        let portals = format!("{PLAIN_AB}, {WALL_BC}");
        let dup = format!("{CLOSET_C}, {CLOSET_C}");
        let err = floors(&portals, SWITCH_A, &dup).expect_err("two `pen`s");
        assert!(
            matches!(err, IrError::DuplicateReveal { ref id } if id == "pen"),
            "{err}"
        );
        let unknown = CLOSET_C.replace("\"room\":\"c\"", "\"room\":\"zz\"");
        let err = floors(&portals, SWITCH_A, &unknown).expect_err("unknown room");
        assert!(
            matches!(err, IrError::RevealUnknownRoom { ref room, .. } if room == "zz"),
            "{err}"
        );
        for (reveal, needle) in [
            (
                CLOSET_C.replace(
                    "\"kind\":\"closet\"",
                    "\"size\":[60,64], \"kind\":\"closet\"",
                ),
                "each side must be a positive multiple of 8",
            ),
            (
                CLOSET_C.replace("\"at\":[704,64]", "\"at\":[880,64]"),
                "does not fit strictly inside its room",
            ),
            (
                CLOSET_C.replace("\"at\":[736,96]", "\"at\":[800,96]"),
                "outside its rectangle",
            ),
        ] {
            let err = floors(&portals, SWITCH_A, &reveal).expect_err(needle);
            let IrError::RevealGeometry { ref detail, .. } = err else {
                panic!("{err}");
            };
            assert!(detail.contains(needle), "{err}");
        }
        // The range check the pedestals share: `at + size` is computed with
        // checked arithmetic, so a corner past the map range is an error and
        // not a saturated, plausible-looking coordinate.
        let huge = CLOSET_C.replace("\"at\":[704,64]", "\"at\":[2147483647,64]");
        let err = floors(&portals, SWITCH_A, &huge).expect_err("outside the map range");
        assert!(
            matches!(err, IrError::CoordinateOutOfRange { ref subject, .. } if subject == "reveal `pen`"),
            "{err}"
        );
    }

    #[test]
    fn a_reveal_may_not_overlap_a_pedestal_or_another_reveal() {
        // Touching counts, as it does between two pedestals: two cells that
        // meet exactly along one edge emit coincident linedefs.
        let neighbor =
            r#"{ "id":"pen2", "room":"c", "at":[768,64], "kind":"closet", "trigger":"t" }"#;
        let err = floors(
            &format!("{PLAIN_AB}, {WALL_BC}"),
            SWITCH_A,
            &format!("{CLOSET_C}, {neighbor}"),
        )
        .expect_err("two reveals that touch");
        let IrError::RevealGeometry { ref id, ref detail } = err else {
            panic!("{err}");
        };
        assert_eq!(id, "pen");
        assert!(detail.contains("overlaps `pen2`"), "{err}");
        let with_pedestal = FLOORS_BASE
            .replace("{PORTALS}", &format!("{PLAIN_AB}, {WALL_BC}"))
            .replace("{TRIGGERS}", SWITCH_A)
            .replace("{REVEALS}", CLOSET_C)
            .replace(
                r#""reveals":["#,
                r#""pedestals":[ { "id":"ped", "room":"c", "at":[736,64], "rise":64 } ], "reveals":["#,
            );
        let err = Ir::from_json(&with_pedestal).expect_err("a reveal over a pedestal");
        assert!(
            matches!(err, IrError::RevealGeometry { ref detail, .. } if detail.contains("overlaps `ped`")),
            "{err}"
        );
    }

    #[test]
    fn more_than_eight_actions_are_rejected() {
        // Nine closets in room `c`, all on one switch: 8x8 cells on a 64 grid.
        let mut reveals = Vec::new();
        for i in 0..9 {
            reveals.push(format!(
                r#"{{ "id":"r{i}", "room":"c", "at":[{x},{y}], "size":[8,8], "kind":"closet", "trigger":"t" }}"#,
                x = 656 + (i % 3) * 64,
                y = 32 + (i / 3) * 64
            ));
        }
        let err = floors(PLAIN_AB, SWITCH_A, &reveals.join(", ")).expect_err("nine actions");
        assert!(
            matches!(err, IrError::TooManyFloorActions { count: 9, max: 8 }),
            "{err}"
        );
        // Eight of the same cells parse, so the cap is what rejected them.
        floors(PLAIN_AB, SWITCH_A, &reveals[..8].join(", ")).expect("eight actions");
    }

    #[test]
    fn the_ir_floor_grains_match_the_engine_table() {
        // The drift guard `tables::tests` keeps over `Ir::FLAT_TILE`, for the
        // same reason: `from_json` validates without loading a table, so the
        // IR carries its own copy of `[floor]`'s two curated grains.
        let floor = crate::tables::Tables::load().expect("tables load").floor();
        assert_eq!(floor.drop_wall_thickness, Ir::DROP_WALL_THICKNESS);
        assert_eq!(floor.bridge_depth_step, Ir::BRIDGE_DEPTH_STEP);
    }

    #[test]
    fn every_pre_existing_fixture_still_parses() {
        for text in [
            include_str!("../tests/golden/lifts.json"),
            include_str!("../tests/golden/teleports.json"),
            include_str!("../tests/fixtures/entrada_base.json"),
            include_str!("../tests/fixtures/ascensor_base.json"),
        ] {
            let ir = Ir::from_json(text).expect("parses unchanged");
            assert!(ir.triggers.is_empty() && ir.reveals.is_empty());
        }
    }

    /// [`FLOORS_BASE`] with a `teleports` list spliced in beside the rest.
    fn floors_with_teleports(
        portals: &str,
        triggers: &str,
        reveals: &str,
        teleports: &str,
    ) -> Result<Ir, IrError> {
        Ir::from_json(
            &FLOORS_BASE
                .replace("{PORTALS}", portals)
                .replace("{TRIGGERS}", triggers)
                .replace("{REVEALS}", reveals)
                .replace(
                    r#""triggers":["#,
                    &format!(r#""teleports":[ {teleports} ], "triggers":["#),
                ),
        )
    }

    #[test]
    fn a_reveal_may_not_overlap_a_teleport_pad() {
        // The third pairing of the island rule, after reveal-vs-pedestal and
        // reveal-vs-reveal: the pad's square meets the closet's exactly along
        // x = 768, which would emit coincident one-sided linedefs.
        let pad = r#"{ "id":"tp", "room":"c", "pad":{"island":[768,64]},
            "to":{"room":"c","at":[850,200],"angle":0} }"#;
        let err = floors_with_teleports(&format!("{PLAIN_AB}, {WALL_BC}"), SWITCH_A, CLOSET_C, pad)
            .expect_err("the pad touches the closet");
        assert!(
            matches!(err, IrError::RevealGeometry { ref id, ref detail }
                if id == "pen" && detail.contains("overlaps teleport pad `tp`")),
            "{err}"
        );
        // The same pad in the room next door is no one's neighbor.
        let elsewhere = pad.replace(
            "\"room\":\"c\", \"pad\":{\"island\":[768,64]}",
            "\"room\":\"b\", \"pad\":{\"island\":[384,64]}",
        );
        floors_with_teleports(
            &format!("{PLAIN_AB}, {WALL_BC}"),
            SWITCH_A,
            CLOSET_C,
            &elsewhere,
        )
        .expect("a pad clear of the reveal");
    }

    #[test]
    fn a_teleport_may_not_deliver_onto_a_reveal() {
        // The pedestal rule's companion for the *destination*: a point, which
        // the rectangle-against-rectangle test above cannot see. The pad sits
        // in room `b`, well clear of the closet, so only the destination is
        // at issue in either case.
        let pad = r#"{ "id":"tp", "room":"b", "pad":{"island":[384,64]},
            "to":{"room":"c","at":[736,96],"angle":0} }"#;
        let err = floors_with_teleports(&format!("{PLAIN_AB}, {WALL_BC}"), SWITCH_A, CLOSET_C, pad)
            .expect_err("the destination is inside the closet");
        assert!(
            matches!(err, IrError::TeleportDestinationOnReveal { ref teleport, ref reveal }
                if teleport == "tp" && reveal == "pen"),
            "{err}"
        );
        let clear = pad.replace("\"at\":[736,96]", "\"at\":[850,200]");
        floors_with_teleports(
            &format!("{PLAIN_AB}, {WALL_BC}"),
            SWITCH_A,
            CLOSET_C,
            &clear,
        )
        .expect("a destination clear of the reveal");
    }

    #[test]
    fn a_walkover_naming_two_portals_between_one_room_pair_is_rejected() {
        // Two openings in one wall pair are legal as long as their spans do
        // not overlap (`compile::portals` has its own test for exactly that),
        // so `[a, b]` names two lines here and the author has to say which
        // one carries the special.
        let second = r#"{ "a":"a", "b":"b", "kind":"plain", "width":32, "at":[256,32] }"#;
        let err = floors(&format!("{PLAIN_AB}, {second}"), WALK_AB, "")
            .expect_err("two portals join `a` and `b`");
        assert!(
            matches!(err, IrError::AmbiguousWalkoverPortal { ref id, count: 2, .. } if id == "w"),
            "{err}"
        );
    }
}
