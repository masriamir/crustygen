//! The floor-action pass: placed triggers and the opening constructs they
//! fire — the drop wall, the reveal and the bridge — lowered to one
//! [`FloorActionOut`] per target.
//!
//! Every action this pass emits is one-way. `EV_DoFloor` (`p_floor.c`) hangs
//! a `T_MoveFloor` thinker on the target sector and `T_MoveFloor` removes it
//! once the floor reaches `floordestheight`, and the four specials written
//! here are the one-shot forms — 23/38 (`lowerFloorToLowest`, S1/W1) and
//! 18/119 (`raiseFloorToNearest`, S1/W1); see `data/vocabulary.toml`'s
//! `[specials.floor]` citation for each. One-shot is also how the corpus
//! authors them: W1 + S1 are 77 % of sample floor lines
//! (`docs/measurements/floor-shapes-2026-09-02.md` §E).
//!
//! One trigger is one tag is one special, so every construct naming a trigger
//! moves the same way — [`crate::ir::Ir::from_json`] has already refused a
//! trigger whose constructs disagree, and this pass allocates exactly one tag
//! per trigger and stamps it on every target.
//!
//! The gap a drop wall or a bridge fills is already open when this pass
//! runs: [`crate::compile::portals::cut_portals`] cuts both rooms' own walls
//! for either kind but leaves the void between them empty, exactly as it does
//! for a door or a lift — the wall and the pit are sectors, not lines.

use crate::compile::heights::visible_lower_side;
use crate::compile::portals::{
    Cut, PortalGeometry, emit_gap_sector, emit_jambs, emit_opening, emit_segment, emit_switch_line,
    find_opening_line, mark_secret_thresholds, resolve_portal, sector_like, split_wall_for_opening,
};
use crate::compile::tags::TagAllocator;
use crate::compile::teleports::emit_island_edges;
use crate::compile::{CompileError, MapData};
use crate::geom::wall_edges;
use crate::ir::{FloorFamilyIr, Ir, Portal, PortalKind, Reveal, RevealKind, Trigger, TriggerKind};
use crate::tables::{FloorFamily, Tables};

/// What an emitted action is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FloorShape {
    /// A sealed wall between two rooms that lowers once.
    DropWall,
    /// A solid cell inside a room that lowers once, its things inside.
    Closet,
    /// A raised block inside a room that lowers once, its things on top.
    Pedestal,
    /// A pit strip between two rooms that rises once.
    Bridge,
}

/// A floor construct as the two places that name one to an author see it:
/// the authored portal or reveal behind an emitted action.
#[derive(Debug, Clone, Copy)]
pub enum NamedConstruct<'a> {
    /// A drop-wall or bridge portal.
    Portal(&'a Portal),
    /// A closet or pedestal reveal.
    Reveal(&'a Reveal),
}

/// What a floor construct is called wherever one is named to an author:
/// `"drop wall a <-> b"`, `"bridge a <-> b"` or `"reveal pen"`.
///
/// One function because two places quote it and an author reading a tag
/// manifest beside a P7 violation must see the same words in both — the tag
/// manifest [`emit_floors`] writes, and
/// [`BuiltGraph::action_names`](crate::reach::BuiltGraph::action_names),
/// which words the reachability rule's violations.
#[must_use]
pub fn construct_name(construct: NamedConstruct<'_>) -> String {
    match construct {
        NamedConstruct::Portal(p) => {
            let kind = if p.kind == PortalKind::Bridge {
                "bridge"
            } else {
                "drop wall"
            };
            format!("{kind} {} <-> {}", p.a, p.b)
        }
        NamedConstruct::Reveal(r) => format!("reveal {}", r.id),
    }
}

/// One emitted trigger line.
#[derive(Debug, Clone)]
pub struct TriggerOut {
    /// The IR id.
    pub id: String,
    /// The tag every target it drives carries.
    pub tag: u16,
    /// The family its targets share.
    pub family: FloorFamily,
    /// The linedef carrying the special — the *first* one written, since a
    /// walkover naming a bridge writes both of the bridge's thresholds.
    pub line: usize,
    /// The sector the player fires it from: a switch's room, or the front
    /// room of the walkover's portal (a walkover fires from either side;
    /// the flood adds the back side itself).
    pub activator: usize,
    /// Walkover (W1) rather than switch (S1).
    pub walkover: bool,
}

/// One emitted floor action, for `reach`, the rules, `place_things` and the
/// conformance report.
#[derive(Debug, Clone)]
pub struct FloorActionOut {
    /// The target sector.
    pub sector: usize,
    /// Which engine type moves it.
    pub family: FloorFamily,
    /// Its floor at load.
    pub rest: i32,
    /// The floor the engine's search must land on.
    pub dest: i32,
    /// The trigger's tag.
    pub tag: u16,
    /// Index into [`crate::compile::Compiled::triggers`].
    pub trigger: usize,
    /// What it is.
    pub shape: FloorShape,
    /// Index into [`Ir::portals`] for a drop wall or bridge.
    pub portal: Option<usize>,
    /// Index into [`Ir::reveals`] for a closet or pedestal.
    pub reveal: Option<usize>,
    /// The two thresholds of a drop wall's or bridge's gap segment (near,
    /// far), where a walkover naming a bridge writes its special; empty for
    /// a reveal.
    pub lines: Vec<usize>,
}

/// The engine family the IR's own word for a direction maps to.
fn family_of(f: FloorFamilyIr) -> FloorFamily {
    match f {
        FloorFamilyIr::Lower => FloorFamily::LowerToLowest,
        FloorFamilyIr::Raise => FloorFamily::RaiseToNearest,
    }
}

/// Where one trigger's line goes, resolved — and, for a switch, carved out
/// of its room's wall — before this pass records a single linedef index.
///
/// The split is why this is a separate step rather than part of emitting the
/// line: [`split_wall_for_opening`] *removes* the wall it splits, which
/// shifts every linedef index above it, and the constructs emitted in between
/// hand back thresholds by index. Every removal this pass makes therefore
/// happens before the first index is recorded, in the same
/// "resolve everything, then emit" order [`crate::compile::portals`] and
/// [`crate::compile::exits`] already follow.
///
/// The same rule holds one level up: this pass runs before
/// [`crate::compile::lifts`] precisely because it splits walls and
/// `emit_lifts` records linedef indices of its own — see the comment at
/// `compile_reporting`'s call site.
enum TriggerPlacement {
    /// A switch: its span is already split out of the room's wall, waiting
    /// for [`emit_switch_line`].
    Switch {
        /// The room whose wall carries it, which is also its sector.
        room_idx: usize,
        /// The span split out of that wall.
        cut: Cut,
        /// Whether the wall's own edge runs in the increasing-along
        /// direction ([`crate::geom::FacingSpan::a_forward`]'s single-room
        /// analogue).
        forward: bool,
    },
    /// A walkover: the portal whose opening line carries it.
    Walkover {
        /// Index into [`Ir::portals`].
        portal: usize,
        /// That portal's resolved placement.
        geometry: PortalGeometry,
    },
}

/// The index in `triggers` of the trigger `id` names.
///
/// # Panics
/// Panics if nothing carries that id, which [`Ir::from_json`] rejects
/// ([`crate::ir::IrError::UnknownTrigger`]) before this pass runs.
fn trigger_index(triggers: &[TriggerOut], id: &str) -> usize {
    triggers
        .iter()
        .position(|t| t.id == id)
        .expect("validated by Ir::from_json")
}

/// Emits every trigger and every construct one fires.
///
/// Four steps, and each one's position is load-bearing:
///
/// 1. **A tag per trigger**, so every construct can be stamped with its
///    trigger's tag as it is emitted rather than in a second pass over the
///    map. The target sector does not exist yet, so the manifest entry is
///    pointed at it in step 4 ([`TagAllocator::rename_sector`]).
/// 2. **Every switch's wall split** (`resolve_trigger`), which is the only
///    thing this pass does that *removes* a linedef — and so the last thing
///    it may do before any index is written down. See `TriggerPlacement`'s
///    own doc comment for why that matters.
/// 3. **The constructs**, each handing back the thresholds of its own gap
///    segment.
/// 4. **The trigger lines**, last because a walkover that names a bridge
///    writes its special onto that bridge's own two thresholds, which exist
///    only once step 3 has run.
///
/// # Errors
/// Returns [`CompileError::UnknownTheme`] when `ir.theme` resolves to no
/// texture set, [`CompileError::TriggerWallNotFound`]
/// when a switch trigger's `at` matches no wall segment of its room,
/// [`CompileError::DropWallTooThick`] and
/// [`CompileError::DropWallFloorsDiffer`] for a drop wall the player could
/// not pass once it has fired, [`CompileError::BridgeDepthTooLow`] for a pit
/// the player could step out of and [`CompileError::BridgeTooShallow`] for a
/// bridge they could not stand on, [`CompileError::RevealRiseTooLow`] for a
/// pedestal reveal the player could climb rather than wait for and
/// [`CompileError::RevealRiseTooHigh`] for one that would rest at or above
/// its host's ceiling, [`CompileError::TriggerLineAlreadyClaimed`]
/// when two walkovers would write onto one line, and whatever
/// `resolve_portal` (`NotAdjacent`,
/// `PortalOffWall`, `PortalOnDiagonalWall`, `PortalTooWide`) or
/// `split_wall_for_opening` ([`CompileError::OpeningNotInAWall`], for a
/// switch overlapping an opening already cut into the same wall) raise.
///
/// # Panics
/// Panics if a construct names a trigger, or a trigger names a room or a
/// portal, that [`Ir::from_json`] did not validate.
pub fn emit_floors(
    ir: &Ir,
    tables: &Tables,
    data: &mut MapData,
    tags: &mut TagAllocator,
) -> Result<(Vec<TriggerOut>, Vec<FloorActionOut>), CompileError> {
    // Resolved unconditionally, mirroring `exits::emit_exits`: an
    // unresolvable theme is an authoring error that should surface the same
    // way regardless of which triggers (if any) are switches, or which
    // constructs (if any) are bridges. Both halves of the switch's appearance
    // come out of one lookup, so one bad theme is one error rather than two
    // paths to the same one.
    let unknown_theme = || CompileError::UnknownTheme {
        theme: ir.theme.clone(),
    };
    let (switch_tex, switch_width) = tables
        .texture("switch", &ir.theme)
        .zip(tables.switch_width(&ir.theme))
        .ok_or_else(unknown_theme)?;
    let switch_tex = switch_tex.to_owned();
    // A bridge's pit shows the same riser a lift's platform does: both are a
    // floor the player watches rise past a wall, and the corpus draws them
    // from one texture family (`vocabulary.toml`'s `lift_riser_source`).
    let riser = tables
        .texture("lift_riser", &ir.theme)
        .ok_or_else(unknown_theme)?
        .to_owned();

    let mut triggers: Vec<TriggerOut> = ir
        .triggers
        .iter()
        .map(|t| {
            let family = family_of(
                ir.trigger_family(&t.id)
                    .expect("Ir::from_json: every trigger is named"),
            );
            let named: Vec<String> = ir
                .portals
                .iter()
                .filter(|p| p.fires_on.as_deref() == Some(t.id.as_str()))
                .map(|p| construct_name(NamedConstruct::Portal(p)))
                .chain(
                    ir.reveals
                        .iter()
                        .filter(|r| r.trigger == t.id)
                        .map(|r| construct_name(NamedConstruct::Reveal(r))),
                )
                .collect();
            // `usize::MAX` is a placeholder: the trigger's tag is allocated
            // before its first target sector exists, and the loop at the end
            // of this function points the manifest entry at that sector.
            let tag = tags.allocate(
                usize::MAX,
                &format!("trigger {}: {}", t.id, named.join(", ")),
            );
            TriggerOut {
                id: t.id.clone(),
                tag,
                family,
                line: usize::MAX,
                activator: usize::MAX,
                walkover: t.kind == TriggerKind::Walkover,
            }
        })
        .collect();

    // Every wall a switch trigger carves, before anything records a linedef
    // index — see [`TriggerPlacement`] for why that ordering is load-bearing.
    let placements = ir
        .triggers
        .iter()
        .map(|t| resolve_trigger(ir, data, t, switch_width))
        .collect::<Result<Vec<_>, CompileError>>()?;

    let out = emit_constructs(ir, tables, data, &triggers, &riser)?;

    for (i, placement) in placements.iter().enumerate() {
        let (line, activator) = emit_trigger_line(
            tables,
            data,
            placement,
            &triggers[i].id,
            triggers[i].family,
            triggers[i].tag,
            &out,
            &switch_tex,
            switch_width,
        )?;
        triggers[i].line = line;
        triggers[i].activator = activator;
        // The manifest names one sector per tag, and a trigger's tag is on
        // every target it drives: the first is the representative one, the
        // way a door's or a platform's own sector is.
        if let Some(action) = out.iter().find(|f| f.trigger == i) {
            tags.rename_sector(triggers[i].tag, action.sector);
        }
    }
    Ok((triggers, out))
}

/// Step 3 of [`emit_floors`]: every construct a trigger fires, in the order
/// `ir.portals` then `ir.reveals`.
///
/// A function of its own rather than two loops inline: it runs to completion
/// before step 4 begins, since a walkover naming a bridge writes its special
/// onto that bridge's own thresholds and so needs every construct already
/// emitted, and pulling it out keeps that boundary visible instead of
/// implied by where one loop ends and the next begins.
///
/// # Errors
/// Returns whatever [`emit_drop_wall`], [`emit_bridge`] or [`emit_reveal`]
/// raise.
///
/// # Panics
/// Panics if a drop wall or a bridge names no trigger, or a construct names a
/// trigger [`Ir::from_json`] did not validate.
fn emit_constructs(
    ir: &Ir,
    tables: &Tables,
    data: &mut MapData,
    triggers: &[TriggerOut],
    riser: &str,
) -> Result<Vec<FloorActionOut>, CompileError> {
    let mut out = Vec::new();
    for (pi, portal) in ir.portals.iter().enumerate() {
        match portal.kind {
            PortalKind::DropWall => {
                let id = portal
                    .fires_on
                    .as_deref()
                    .expect("Ir::from_json requires a trigger on a drop wall");
                let ti = trigger_index(triggers, id);
                out.push(emit_drop_wall(ir, tables, data, pi, ti, triggers[ti].tag)?);
            }
            PortalKind::Bridge => {
                let id = portal
                    .fires_on
                    .as_deref()
                    .expect("Ir::from_json requires a trigger on a bridge");
                let ti = trigger_index(triggers, id);
                out.push(emit_bridge(
                    ir,
                    tables,
                    data,
                    pi,
                    ti,
                    triggers[ti].tag,
                    riser,
                )?);
            }
            PortalKind::Plain | PortalKind::Door | PortalKind::Locked | PortalKind::Lift => {}
        }
    }
    for (ri, reveal) in ir.reveals.iter().enumerate() {
        let ti = trigger_index(triggers, &reveal.trigger);
        out.push(emit_reveal(
            ir,
            tables,
            data,
            ri,
            reveal,
            ti,
            triggers[ti].tag,
        )?);
    }
    Ok(out)
}

/// Resolves where one trigger's line goes, splitting a switch's span out of
/// its room's wall on the way.
///
/// # Errors
/// Returns [`CompileError::TriggerWallNotFound`] when a switch's `at` matches
/// no wall segment its room emitted, whatever [`split_wall_for_opening`]
/// raises when that span is not free (an opening already cut into the same
/// wall), and whatever [`resolve_portal`] raises for a walkover's portal.
///
/// # Panics
/// Panics if the trigger names a room or a portal [`Ir::from_json`] did not
/// validate, or leaves out a field its own kind requires.
fn resolve_trigger(
    ir: &Ir,
    data: &mut MapData,
    t: &Trigger,
    switch_width: i32,
) -> Result<TriggerPlacement, CompileError> {
    match t.kind {
        TriggerKind::Switch => {
            let room_id = t
                .room
                .as_deref()
                .expect("Ir::from_json requires a room on a switch trigger");
            let at =
                t.at.expect("Ir::from_json requires a point on a switch trigger");
            let room_idx = ir
                .rooms
                .iter()
                .position(|r| r.id == room_id)
                .expect("validated by Ir::from_json");
            let (axis, fixed, lo, hi, forward) = wall_edges(&ir.rooms[room_idx].footprint)
                .find(|&(axis, fixed, lo, hi, _)| {
                    let (along, across) = axis.split(at);
                    across == fixed && along > lo && along < hi
                })
                .ok_or_else(|| CompileError::TriggerWallNotFound {
                    id: t.id.clone(),
                    x: at.x,
                    y: at.y,
                })?;
            // One switch texture wide, centered on `at` and clipped to the
            // wall's own ends. Clipped rather than refused (as
            // `exits::resolve_exit` refuses an exit too wide for its wall):
            // a switch is pressed, not walked through, so a segment a corner
            // has shortened is still a switch, and `emit_switch_line` centers
            // the texture on whatever span it is given.
            let (along, _) = axis.split(at);
            let half = switch_width / 2;
            let cut = Cut {
                axis,
                fixed,
                open_lo: (along - half).max(lo),
                open_hi: (along + half).min(hi),
            };
            split_wall_for_opening(data, &cut, room_idx, room_id)?;
            Ok(TriggerPlacement::Switch {
                room_idx,
                cut,
                forward,
            })
        }
        TriggerKind::Walkover => {
            let [a, b] = t
                .portal
                .as_ref()
                .expect("Ir::from_json requires a portal on a walkover trigger");
            let portal = ir
                .portals
                .iter()
                .position(|p| (p.a == *a && p.b == *b) || (p.a == *b && p.b == *a))
                .expect("Ir::from_json validated that exactly one portal joins the pair");
            let geometry = resolve_portal(ir, &ir.portals[portal])?;
            Ok(TriggerPlacement::Walkover { portal, geometry })
        }
    }
}

/// Emits one trigger's line: a switch line on the span already split out of
/// its room's wall, or the walkover special on the named plain portal's
/// opening line — or, when the walkover names a bridge, on both of that
/// bridge's own pit thresholds.
///
/// Returns `(the first line written, the activator sector)`. The activator is
/// the room whose side the player fires it from: a switch's own room, or the
/// front room of the walkover's portal — a walkover fires from either side,
/// and the flood adds the back side from the line itself.
///
/// # Errors
/// Returns [`CompileError::TriggerLineAlreadyClaimed`] when the line a
/// walkover would write onto already carries a special — see that variant.
///
/// # Panics
/// Panics if a plain portal's own opening line is not where `cut_portals` put
/// it, which nothing between the two passes can move.
#[expect(
    clippy::too_many_arguments,
    reason = "each parameter is one resolved input — the tables, the accumulating map, the \
              trigger's own resolved placement, its id and the two values its tag allocation \
              fixed, the constructs a bridge walkover writes onto, and the two hoisted out of \
              the per-map theme lookup"
)]
fn emit_trigger_line(
    tables: &Tables,
    data: &mut MapData,
    placement: &TriggerPlacement,
    id: &str,
    family: FloorFamily,
    tag: u16,
    actions: &[FloorActionOut],
    switch_tex: &str,
    switch_width: i32,
) -> Result<(usize, usize), CompileError> {
    match placement {
        TriggerPlacement::Switch {
            room_idx,
            cut,
            forward,
        } => {
            let special = tables.floor_special(family, true);
            let line = emit_switch_line(
                data,
                cut,
                *room_idx,
                *forward,
                special,
                tag,
                switch_tex,
                switch_width,
            );
            Ok((line, *room_idx))
        }
        TriggerPlacement::Walkover { portal, geometry } => {
            let special = tables.floor_special(family, false);
            // A walkover writes onto a line something else emitted, so unlike
            // the switch — which emits its own — it has to check that the line
            // is free. See `CompileError::TriggerLineAlreadyClaimed`.
            let set = |data: &mut MapData, line: usize| -> Result<(), CompileError> {
                if data.linedefs[line].special != 0 {
                    return Err(CompileError::TriggerLineAlreadyClaimed {
                        id: id.to_owned(),
                        line,
                    });
                }
                data.linedefs[line].special = special;
                data.linedefs[line].tag = tag;
                Ok(())
            };

            // A bridge names itself: stepping down into the pit is the
            // crossing that raises it, so the special goes on both of the
            // pit's thresholds, whichever side the player steps off.
            if let Some(bridge) = actions
                .iter()
                .find(|f| f.shape == FloorShape::Bridge && f.portal == Some(*portal))
            {
                for &line in &bridge.lines {
                    set(data, line)?;
                }
                return Ok((bridge.lines[0], geometry.ia));
            }

            // Otherwise the portal is a plain one, and the line is the
            // threshold `cut_portals` emitted between room `a` and the
            // passage filling the gap. Either of that passage's two
            // thresholds would fire — `P_CrossSpecialLine` runs from both
            // sides of a walkover — so room `a`'s is taken, the side
            // `PortalGeometry` itself calls near.
            let cut = Cut {
                axis: geometry.span.axis,
                fixed: geometry.span.near,
                open_lo: geometry.open_lo,
                open_hi: geometry.open_hi,
            };
            let line = find_opening_line(data, &cut, geometry.ia)
                .expect("cut_portals emitted the plain portal's threshold on room a");
            set(data, line)?;
            Ok((line, geometry.ia))
        }
    }
}

/// Emits one drop wall: a sealed sector filling the middle of the portal's
/// gap, with a piece of each room's own floor leading up to it.
///
/// The wall rests with its floor at its ceiling — the lower of the two rooms'
/// ceilings, so neither room's opening is taller than the rock in it — and
/// lowers once to the lower of the two floors. That destination is what the
/// engine will compute rather than what this pass asserts: the wall's only
/// two-sided neighbors are the two passages, each at its own room's floor, so
/// `P_FindLowestFloorSurrounding` is exactly `min(room floors)` by
/// construction.
///
/// The passages are the reason for the three-position construction rather
/// than one gap-spanning sector: a wall centered in its gap leaves the player
/// standing room on both sides of it, and gives each room's opening a floor
/// at that room's own height. Where the wall fills its free gap exactly there
/// is no room for them, and the wall borders the two rooms directly — the
/// same "`unwrap_or` the room" an absent lift alcove takes.
///
/// # Errors
/// Returns [`CompileError::DropWallTooThick`] when the wall is deeper than
/// the gap its alcoves leave, [`CompileError::DropWallFloorsDiffer`] when the
/// dropped wall would be a one-way drop, and whatever `resolve_portal`
/// raises.
#[expect(
    clippy::too_many_lines,
    reason = "the wall construction (up to three sectors, its own two faces and jambs, each \
              passage's outer threshold and jambs, and the two faces' textures) is one coherent \
              unit of work per drop wall, exactly as `lifts::emit_portal_lift` is per lift \
              portal; splitting it would scatter the sequential dependency between pos0..pos3 \
              across call boundaries"
)]
fn emit_drop_wall(
    ir: &Ir,
    tables: &Tables,
    data: &mut MapData,
    pi: usize,
    ti: usize,
    tag: u16,
) -> Result<FloorActionOut, CompileError> {
    let portal = &ir.portals[pi];
    let geometry = resolve_portal(ir, portal)?;
    let (room_a, room_b) = (&ir.rooms[geometry.ia], &ir.rooms[geometry.ib]);

    // A wall between rooms at different floors drops into a one-way passage:
    // it comes to rest at the lower floor, and `P_TryMove` refuses the climb
    // back up out of it. Judged here rather than in the IR because the step
    // height is a table constant IR validation never loads.
    let step = tables.step_height();
    if (room_a.floor - room_b.floor).abs() > step {
        return Err(CompileError::DropWallFloorsDiffer {
            a: portal.a.clone(),
            b: portal.b.clone(),
            floor_a: room_a.floor,
            floor_b: room_b.floor,
            step,
        });
    }

    let low_room = if room_b.floor < room_a.floor {
        room_b
    } else {
        room_a
    };
    let rest = room_a.ceiling.min(room_b.ceiling);
    let dest = low_room.floor;
    let thickness = portal
        .thickness
        .expect("Ir::from_json requires thickness on a drop wall");

    // The wall sits centered in what the alcoves leave of the gap, so what is
    // left over is split evenly on both sides: a 16-deep wall in a 64 gap has
    // 24 units of room-floor passage before and after it. Positions run from
    // room `a`'s own wall (`pos0`) to room `b`'s (`pos3`), as in
    // `lifts::emit_portal_lift`.
    let dir = (geometry.span.far - geometry.span.near).signum();
    let gap = (geometry.span.far - geometry.span.near) * dir;
    let alcove_near = portal.alcove_near.unwrap_or(0);
    let alcove_far = portal.alcove_far.unwrap_or(0);
    let free = gap - alcove_near - alcove_far;
    if free < thickness {
        return Err(CompileError::DropWallTooThick {
            a: portal.a.clone(),
            b: portal.b.clone(),
            thickness,
            gap: free,
        });
    }
    let lead = (free - thickness) / 2;
    let pos0 = geometry.span.near;
    let pos1 = pos0 + dir * (alcove_near + lead);
    let pos2 = pos1 + dir * thickness;
    let pos3 = geometry.span.far;

    // A passage before and after the wall, each a piece of its own room
    // (floor, ceiling, flat, light), exactly as a lift's alcoves are. A
    // declared alcove is absorbed into the passage on its side rather than
    // emitted separately: both would be the same sector of the same room, so
    // splitting them would only add a seam. What an alcove still does is
    // reserve its depth, which is why it is taken out of `free` above.
    let before = (pos1 != pos0).then(|| {
        let idx = data.sectors.len();
        data.sectors.push(sector_like(
            room_a,
            room_a.floor,
            room_a.ceiling,
            &room_a.wall_tex,
            0,
        ));
        idx
    });
    let after = (pos3 != pos2).then(|| {
        let idx = data.sectors.len();
        data.sectors.push(sector_like(
            room_b,
            room_b.floor,
            room_b.ceiling,
            &room_b.wall_tex,
            0,
        ));
        idx
    });
    let wall = data.sectors.len();
    data.sectors
        .push(sector_like(low_room, rest, rest, &low_room.wall_tex, tag));

    let axis = geometry.span.axis;
    let a_forward = geometry.span.a_forward;
    let (open_lo, open_hi) = (geometry.open_lo, geometry.open_hi);

    // The wall's own two faces and jambs, in one call — neither face is
    // shared with a third segment, so `emit_segment`'s once-per-boundary rule
    // is satisfied. Each passage's *outer* threshold and jambs are built
    // directly below, exactly as `doors::emit_doors` and
    // `lifts::emit_portal_lift` build an alcove's.
    let near_neighbor = before.unwrap_or(geometry.ia);
    let far_neighbor = after.unwrap_or(geometry.ib);
    let seg = emit_segment(
        data,
        axis,
        open_lo,
        open_hi,
        a_forward,
        pos1,
        pos2,
        near_neighbor,
        wall,
        far_neighbor,
        &low_room.wall_tex,
    );
    debug_assert_eq!(
        seg.sector, wall,
        "the wall segment was pushed at the predicted index"
    );

    let mut thresholds = vec![seg.near_line, seg.far_line];
    if let Some(passage) = before {
        let line = emit_opening(
            data,
            &Cut {
                axis,
                fixed: pos0,
                open_lo,
                open_hi,
            },
            geometry.ia,
            passage,
            a_forward,
        );
        emit_jambs(
            data,
            axis,
            open_lo,
            open_hi,
            a_forward,
            pos0,
            pos1,
            passage,
            &room_a.wall_tex,
        );
        thresholds.push(line);
    }
    if let Some(passage) = after {
        let line = emit_opening(
            data,
            &Cut {
                axis,
                fixed: pos3,
                open_lo,
                open_hi,
            },
            geometry.ib,
            passage,
            !a_forward,
        );
        emit_jambs(
            data,
            axis,
            open_lo,
            open_hi,
            a_forward,
            pos2,
            pos3,
            passage,
            &room_b.wall_tex,
        );
        thresholds.push(line);
    }

    // The faces. On each of the wall's two thresholds the room side is the
    // front sidedef (`emit_segment` binds `sector_near`/`sector_far` there),
    // and it is the side the engine draws at rest: its own sector has the
    // lower floor and, where it is taller, the higher ceiling. The lower
    // carries that side's wall texture as the full-height face at rest, and
    // both pegging flags stay clear so the face rides down with the floor —
    // the corpus does that on 95 % of drop-wall boundaries
    // (`docs/measurements/floor-shapes-2026-09-02.md` §F).
    // `heights::apply_height_textures` fills only empty slots, so these are
    // written here.
    //
    // Fired, the wall comes to rest at the *lower* room's floor, so toward a
    // passage whose own floor stands above that the wall's side becomes the
    // lower one and `r_segs.c` draws the wall side's lower instead. No later
    // pass fills it — `apply_height_textures` reads the geometry at load,
    // where the wall stands at its ceiling and the room side is always the
    // lower — so between rooms whose floors differ by up to a step the fired
    // wall would show a blank strip (a HOM) up to a step tall. The wall
    // sector's own texture, the lower room's, goes on that back sidedef for
    // exactly the faces that need it.
    let wall_tex = data.sectors[wall].wall_tex.clone();
    for (line, room_side_sector) in [(seg.near_line, near_neighbor), (seg.far_line, far_neighbor)] {
        let side = data.linedefs[line].front;
        debug_assert_eq!(data.sidedefs[side].sector, room_side_sector);
        let tex = data.sectors[room_side_sector].wall_tex.clone();
        if data.sectors[room_side_sector].ceiling > rest {
            data.sidedefs[side].upper.clone_from(&tex);
        }
        if data.sectors[room_side_sector].floor > dest {
            let back = data.linedefs[line].back.expect("a wall face is two-sided");
            debug_assert_eq!(data.sidedefs[back].sector, wall);
            data.sidedefs[back].lower.clone_from(&wall_tex);
        }
        data.sidedefs[side].lower = tex;
    }

    mark_secret_thresholds(data, room_a.secret != room_b.secret, thresholds);

    Ok(FloorActionOut {
        sector: wall,
        family: FloorFamily::LowerToLowest,
        rest,
        dest,
        tag,
        trigger: ti,
        shape: FloorShape::DropWall,
        portal: Some(pi),
        reveal: None,
        lines: vec![seg.near_line, seg.far_line],
    })
}

/// Emits one bridge: a pit strip filling the portal's gap, resting
/// [`depth`](crate::ir::Portal::depth) below the two rooms' shared floor and
/// raised once to meet it.
///
/// Like the drop wall's, the destination is what the engine will compute
/// rather than what this pass asserts: `P_FindNextHighestFloor` from the pit
/// is the rooms' own floor, because [`emit_gap_sector`] gives the strip no
/// two-sided neighbors but those two rooms and
/// [`Ir::from_json`](crate::ir::Ir::from_json) has already refused a bridge
/// whose rooms' floors differ ([`crate::ir::IrError::BridgeFloorsDiffer`]),
/// so the search lands on that floor by construction.
///
/// **The pit is a drop the player may take before the bridge rises, and
/// nothing here forbids it** — the corpus builds pits both escapable and
/// not. Rule P7's flood arbitrates: a pit whose trigger cannot be fired from
/// inside it, and which has no other way out, strands whoever drops in, and
/// the flood is what decides that rather than this emitter. The one bridge
/// trigger that cannot strand anyone is a walkover naming the bridge itself,
/// whose special `emit_trigger_line` writes onto *both* of the thresholds
/// handed back in [`FloorActionOut::lines`].
///
/// # Errors
/// Returns [`CompileError::BridgeDepthTooLow`] when the pit is no deeper than
/// the player's step, so they would step out of it rather than wait,
/// [`CompileError::BridgeTooShallow`] when the gap leaves the strip narrower
/// than the player's own diameter, so they could not stand on the risen
/// bridge, and whatever `resolve_portal` raises.
///
/// # Panics
/// Panics on any of three things it takes as given, each unreachable by
/// construction:
/// - the portal's `depth`, which
///   [`Ir::from_json`](crate::ir::Ir::from_json) requires of every bridge
///   ([`crate::ir::IrError::MissingBridgeDepth`]) before this pass runs;
/// - both of the gap segment's thresholds being two-sided, which is what
///   [`emit_gap_sector`] emits there — a threshold is a room/pit boundary,
///   never a wall; and
/// - one of those two sides being the lower-floored one, since the pit rests
///   `depth` below a floor both rooms share and `depth` has just been held
///   above the step, so it is strictly below both and
///   [`visible_lower_side`] cannot return `None`.
fn emit_bridge(
    ir: &Ir,
    tables: &Tables,
    data: &mut MapData,
    pi: usize,
    ti: usize,
    tag: u16,
    riser: &str,
) -> Result<FloorActionOut, CompileError> {
    let portal = &ir.portals[pi];
    let geometry = resolve_portal(ir, portal)?;
    let (room_a, room_b) = (&ir.rooms[geometry.ia], &ir.rooms[geometry.ib]);
    let depth = portal
        .depth
        .expect("Ir::from_json requires depth on a bridge");

    // A pit the player can simply step out of is a dip, not a bridge:
    // `P_TryMove` lets them climb any difference up to `max_step_height`
    // unaided, so the rising strip would be scenery carrying a
    // `raiseFloorToNearest` special. Judged here rather than in the IR — which
    // requires only a positive multiple of 8 — because the step height is a
    // table constant IR validation never loads, the same split
    // `DropWallFloorsDiffer` and `RevealRiseTooLow` are on.
    let step = tables.step_height();
    if depth <= step {
        return Err(CompileError::BridgeDepthTooLow {
            a: portal.a.clone(),
            b: portal.b.clone(),
            depth,
            step,
        });
    }

    // And a strip the player cannot stand on is not a bridge either. The pit
    // fills the whole gap — a bridge declares neither alcoves
    // (`IrError::DoorFieldsOnPlainPortal`) nor a thickness
    // (`IrError::FloorFieldOnOtherPortal`), so unlike a drop wall there is
    // nothing between the rooms to squeeze it — which makes the gap itself
    // what must hold the player. Measured against their diameter, as
    // `LiftTooShallow` measures a platform: they are a cylinder that must fit
    // entirely between the two rooms' walls.
    let player = tables.player();
    let gap = (geometry.span.far - geometry.span.near).abs();
    let need = player.radius * 2;
    if gap < need {
        return Err(CompileError::BridgeTooShallow {
            a: portal.a.clone(),
            b: portal.b.clone(),
            depth: gap,
            need,
        });
    }

    // The two rooms share a floor, so room `a`'s is the pair's. The ceiling
    // is the lower of the two, as every gap sector's is, and the strip takes
    // room `a`'s light and flats, as a plain passage does — but the riser as
    // its wall texture, since every face it shows is a face of the pit.
    let dest = room_a.floor;
    let rest = dest - depth;
    let seg = emit_gap_sector(
        data,
        &geometry.span,
        geometry.open_lo,
        geometry.open_hi,
        geometry.ia,
        geometry.ib,
        sector_like(room_a, rest, room_a.ceiling.min(room_b.ceiling), riser, tag),
        // The jambs — the pit's own two long side walls — take room `a`'s
        // wall texture rather than the riser. They are the chasm's sides:
        // one-sided rock from the pit floor to the ceiling, standing whether
        // the bridge is up or down, so they are wall rather than the moving
        // face a riser marks, and a plain portal's passage takes the same
        // texture for its own jambs (`portals::cut_one`). Only the two
        // thresholds below get the riser.
        &room_a.wall_tex,
    );

    // The riser is the pit's own wall, seen by the player standing in it.
    // `r_segs.c`'s `R_StoreWallRange` draws a lower on the sidedef whose own
    // sector has the lower floor — at rest that is the pit's, on both
    // thresholds — and `heights::visible_lower_side` is exactly that
    // comparison, called rather than re-derived here.
    //
    // Left pegged (`ML_DONTPEGBOTTOM` clear), which anchors a lower texture
    // to the *back* sector's floor — the sector on the far side of the face
    // being drawn. Here that face is the pit's own and the sector behind it
    // is the room, whose floor never moves, so the riser hangs from a fixed
    // top at the room's floor and is covered from below as the pit rises. It
    // is a lift's *top* face, not its low one (`lifts`' module doc works
    // through both). Clear is also what the corpus does: a bridge-walkway
    // boundary carries `ML_DONTPEGBOTTOM` on 5.6 % of the idgames sample's
    // lines, 42 % and 34 % of the two retail ones
    // (`docs/measurements/floor-shapes-2026-09-02.md` §F), which also finds
    // the walkway side's own lower blank 90 % of the time — as it is here.
    // The room-side lowers are left bare: the engine never draws them,
    // and `heights::apply_height_textures` fills only the visible side.
    for line in [seg.near_line, seg.far_line] {
        let l = &data.linedefs[line];
        let back = l.back.expect("emit_segment emits two-sided thresholds");
        let floor_of = |side: usize| data.sectors[data.sidedefs[side].sector].floor;
        let gap_side = visible_lower_side(floor_of(l.front), floor_of(back), l.front, back)
            .expect("the pit rests a positive depth below both rooms");
        riser.clone_into(&mut data.sidedefs[gap_side].lower);
    }

    mark_secret_thresholds(
        data,
        room_a.secret != room_b.secret,
        [seg.near_line, seg.far_line],
    );

    Ok(FloorActionOut {
        sector: seg.sector,
        family: FloorFamily::RaiseToNearest,
        rest,
        dest,
        tag,
        trigger: ti,
        shape: FloorShape::Bridge,
        portal: Some(pi),
        reveal: None,
        lines: vec![seg.near_line, seg.far_line],
    })
}

/// Emits one reveal: a sealed island inside its host room, lowered once to
/// the host's floor.
///
/// A [`closet`](RevealKind::Closet) rests at floor == ceiling == the host's
/// ceiling — solid rock, sealed by the rock itself. A
/// [`pedestal`](RevealKind::Pedestal) reveal rests
/// [`Reveal::rise`](crate::ir::Reveal::rise) above the host's floor under the
/// host's ceiling, sealed by height instead, which is why only that form is
/// held to the step. Both lower to the
/// host's floor, and that destination is what the engine computes rather than
/// what this pass asserts: the island's only two-sided neighbors are its four
/// own edges, every one of them onto the host, so
/// `P_FindLowestFloorSurrounding` is the host's floor by construction.
///
/// Nothing here is a portal, so unlike [`emit_drop_wall`] this emits no
/// passages, no jambs and no thresholds: an island is four two-sided edges
/// and one sector, exactly as `lifts::emit_lifts` cuts a pedestal.
/// [`FloorActionOut::lines`] is therefore empty — there is no gap segment for
/// a walkover to write onto, and a walkover naming a reveal fires from its
/// own portal's threshold as usual.
///
/// **The overlap exemption (spec §10.2) needs no change.** The island is a
/// hole in its host ([`crate::compile::SectorOut::host`]), the one pair
/// [`crate::compile::sectors::check_no_sector_overlaps`] skips. That function
/// compares only the two sectors' polygons — `sector_polygon` reads a room's
/// footprint or the bounding box of the sector's own linedef vertices, and
/// the loop reads nothing but `host` and those polygons — so no floor or
/// ceiling is ever consulted and a floor == ceiling cell passes it exactly as
/// a pedestal's raised one does (read at the pinned working tree,
/// 2026-09-02; no code change was needed there).
///
/// **No size floor, and no resting-headroom rule.** The pedestal *lift* has
/// both, because the player stands on that platform and rides it; neither
/// premise holds here. A reveal comes to rest flush with its host, so the
/// player never stands inside the sealed cell — they walk over where it was
/// — and nobody rides a block that is unclimbable by construction (the step
/// rule below is what makes it so). Holding a reveal to the pedestal's
/// bounds would refuse the corpus's most common one: the 16x16 sunken
/// pedestal, retail DOOM's modal reveal and 351 of the sampled targets
/// (`docs/measurements/floor-shapes-2026-09-02.md` §D), a pillar the player
/// straddles rather than enters. What the cell's cargo *is* held to lives in
/// [`crate::compile::things`]'s `place_reveal_things`, against the host.
/// What a pedestal reveal's `rise` is still held to is the host's own
/// height, top and bottom — see the two `RevealRise*` errors; those bound
/// the *sector*, not the player.
///
/// # Errors
/// Returns [`CompileError::RevealRiseTooLow`] when a pedestal reveal rises
/// no more than a step, which would leave it climbable and so not sealed at
/// all, and [`CompileError::RevealRiseTooHigh`] when it rises to its host's
/// ceiling or beyond, which would emit an inverted sector.
///
/// # Panics
/// Panics if the reveal names a room that does not exist, or is a pedestal
/// with no `rise` — both refused by
/// [`Ir::from_json`](crate::ir::Ir::from_json) before this pass runs.
fn emit_reveal(
    ir: &Ir,
    tables: &Tables,
    data: &mut MapData,
    ri: usize,
    reveal: &Reveal,
    ti: usize,
    tag: u16,
) -> Result<FloorActionOut, CompileError> {
    let host = ir
        .rooms
        .iter()
        .position(|r| r.id == reveal.room)
        .expect("validated by Ir::from_json");
    let room = &ir.rooms[host];

    // No size bound: see the doc comment. A 16x16 cell is the corpus's own
    // sunken pedestal, not a mistake.
    let (lo, hi) = reveal.rect();

    let (floor, shape) = match reveal.kind {
        RevealKind::Closet => (room.ceiling, FloorShape::Closet),
        RevealKind::Pedestal => {
            let rise = reveal
                .rise
                .expect("Ir::from_json requires rise on a pedestal reveal");
            // Rock seals a closet whatever its height; a pedestal reveal is
            // sealed only by being too tall to step onto. Judged here rather
            // than in the IR because the step height is a table constant IR
            // validation never loads.
            let step = tables.step_height();
            if rise <= step {
                return Err(CompileError::RevealRiseTooLow {
                    reveal: reveal.id.clone(),
                    rise,
                    step,
                });
            }
            // And bounded above by the host's own height, because nothing
            // downstream would catch it: `check_no_sector_overlaps` compares
            // polygons, `apply_height_textures` fills whichever side is
            // lower, and the rules judge the emitted map, so a rise past the
            // ceiling would ship a sector whose floor is above it. Rejected
            // at the ceiling rather than one unit past: a block resting
            // exactly at the ceiling is a closet, and is authored as one.
            let max = room.ceiling - room.floor;
            if rise >= max {
                return Err(CompileError::RevealRiseTooHigh {
                    reveal: reveal.id.clone(),
                    rise,
                    max,
                });
            }
            (room.floor + rise, FloorShape::Pedestal)
        }
    };

    let sector = data.sectors.len();
    let mut s = sector_like(room, floor, room.ceiling, &room.wall_tex, tag);
    s.host = Some(host);
    data.sectors.push(s);

    // Four two-sided edges, the host on the front — `emit_island_edges`
    // winds them so. The host-side lower is the face that shows, since the
    // island's floor is the higher one whichever form this is
    // (`heights::visible_lower_side`), and it rides down with the floor:
    // both pegging flags stay clear, as on a drop wall's faces. No upper is
    // ever needed — the island keeps its host's ceiling.
    for line in emit_island_edges(data, lo, hi, host, sector) {
        let front = data.linedefs[line].front;
        room.wall_tex.clone_into(&mut data.sidedefs[front].lower);
    }

    Ok(FloorActionOut {
        sector,
        family: FloorFamily::LowerToLowest,
        rest: floor,
        dest: room.floor,
        tag,
        trigger: ti,
        shape,
        portal: None,
        reveal: Some(ri),
        lines: Vec::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::{FloorActionOut, FloorShape, TriggerOut, emit_floors};
    use crate::compile::tags::TagAllocator;
    use crate::compile::{
        CompileError, LinedefOut, MapData, compile, compile_reporting, doors, exits, portals,
        sectors, teleports,
    };
    use crate::geom::Pt;
    use crate::ir::Ir;
    use crate::tables::{FloorFamily, Tables};

    /// Two rooms 64 units apart, sealed by a 16-deep drop wall that one
    /// switch on room `a`'s far wall lowers.
    ///
    /// Room `b`'s ceiling is the taller of the two deliberately: the wall
    /// rests at the *lower* ceiling, so room `b`'s face is the one that also
    /// needs an upper.
    const WALL: &str = r#"{ "seed":1, "grid":64, "theme":"tech_base",
      "rooms":[
        { "id":"a", "footprint":[[0,0],[0,256],[256,256],[256,0]], "floor":0, "ceiling":192, "light":160,
          "floor_tex":"FLOOR4_8", "ceil_tex":"CEIL3_5", "wall_tex":"STARTAN3",
          "things":[ { "kind":"player1_start", "at":[128,128], "angle":0 } ] },
        { "id":"b", "footprint":[[320,0],[320,256],[576,256],[576,0]], "floor":0, "ceiling":256, "light":144,
          "floor_tex":"FLOOR4_8", "ceil_tex":"CEIL3_5", "wall_tex":"STARTAN3",
          "things":[ { "kind":"imp", "at":[448,128], "angle":180 } ] }
      ],
      "portals":[ { "a":"a", "b":"b", "kind":"drop_wall", "width":64, "at":[256,128], "thickness":16, "fires_on":"t" } ],
      "triggers":[ { "id":"t", "kind":"switch", "room":"a", "at":[0,128] } ],
      "exits":[ { "room":"b", "trigger":"switch", "at":[576,128], "width":64 } ] }"#;

    /// Rooms `a`, `b` and `c` in a row: a plain portal joins `a` and `b`, and
    /// its opening line is the walkover that drops the wall between `b` and
    /// `c`.
    const WALKOVER: &str = r#"{ "seed":1, "grid":64, "theme":"tech_base",
      "rooms":[
        { "id":"a", "footprint":[[0,0],[0,256],[256,256],[256,0]], "floor":0, "ceiling":192, "light":160,
          "floor_tex":"FLOOR4_8", "ceil_tex":"CEIL3_5", "wall_tex":"STARTAN3",
          "things":[ { "kind":"player1_start", "at":[128,128], "angle":0 } ] },
        { "id":"b", "footprint":[[320,0],[320,256],[576,256],[576,0]], "floor":0, "ceiling":192, "light":144,
          "floor_tex":"FLOOR4_8", "ceil_tex":"CEIL3_5", "wall_tex":"STARTAN3" },
        { "id":"c", "footprint":[[640,0],[640,256],[896,256],[896,0]], "floor":0, "ceiling":192, "light":144,
          "floor_tex":"FLOOR4_8", "ceil_tex":"CEIL3_5", "wall_tex":"STARTAN3" }
      ],
      "portals":[
        { "a":"a", "b":"b", "kind":"plain", "width":64, "at":[256,128] },
        { "a":"b", "b":"c", "kind":"drop_wall", "width":64, "at":[576,128], "thickness":16, "fires_on":"t" }
      ],
      "triggers":[ { "id":"t", "kind":"walkover", "portal":["a","b"] } ],
      "exits":[ { "room":"c", "trigger":"switch", "at":[896,128], "width":64 } ] }"#;

    /// The same drop wall on the other axis and the other direction: room
    /// `a` is *north* of room `b`, so the portal's span runs along Y with
    /// `span.far < span.near` and `dir` is negative. The two rooms differ in
    /// every borrowed property — light, both flats, the wall texture — so a
    /// passage or a face that took the wrong room's cannot pass.
    const WALL_SOUTHWARD: &str = r#"{ "seed":1, "grid":64, "theme":"tech_base",
      "rooms":[
        { "id":"a", "footprint":[[0,320],[0,576],[256,576],[256,320]], "floor":0, "ceiling":192, "light":160,
          "floor_tex":"FLOOR4_8", "ceil_tex":"CEIL3_5", "wall_tex":"STARTAN3",
          "things":[ { "kind":"player1_start", "at":[128,448], "angle":0 } ] },
        { "id":"b", "footprint":[[0,0],[0,256],[256,256],[256,0]], "floor":0, "ceiling":256, "light":144,
          "floor_tex":"FLAT1", "ceil_tex":"FLAT20", "wall_tex":"BROWN1" }
      ],
      "portals":[ { "a":"a", "b":"b", "kind":"drop_wall", "width":64, "at":[128,320], "thickness":16, "fires_on":"t" } ],
      "triggers":[ { "id":"t", "kind":"switch", "room":"a", "at":[128,576] } ],
      "exits":[ { "room":"b", "trigger":"switch", "at":[128,0], "width":64 } ] }"#;

    /// Two rooms 64 units apart with a bridge between them: a pit resting 96
    /// below their shared floor that one switch on room `a`'s far wall
    /// raises to meet it.
    ///
    /// Its two rooms are alike, so which room the pit borrows from is not
    /// tested here — [`BRIDGE_SOUTHWARD`] varies every borrowed property, and
    /// [`BRIDGE_WALKOVER`] is this same map on the other trigger form.
    const BRIDGE: &str = r#"{ "seed":1, "grid":64, "theme":"tech_base",
      "rooms":[
        { "id":"a", "footprint":[[0,0],[0,256],[256,256],[256,0]], "floor":0, "ceiling":192, "light":160,
          "floor_tex":"FLOOR4_8", "ceil_tex":"CEIL3_5", "wall_tex":"STARTAN3",
          "things":[ { "kind":"player1_start", "at":[64,64], "angle":0 } ] },
        { "id":"b", "footprint":[[320,0],[320,256],[576,256],[576,0]], "floor":0, "ceiling":192, "light":160,
          "floor_tex":"FLOOR4_8", "ceil_tex":"CEIL3_5", "wall_tex":"STARTAN3" }
      ],
      "portals":[ { "a":"a", "b":"b", "kind":"bridge", "width":64, "at":[256,128], "depth":96, "fires_on":"t" } ],
      "triggers":[ { "id":"t", "kind":"switch", "room":"a", "at":[0,128] } ],
      "exits":[ { "room":"b", "trigger":"switch", "at":[576,128], "width":64 } ] }"#;

    /// The same bridge on the trigger form that cannot strand anyone: a
    /// walkover naming the bridge's *own* two rooms, whose special lands on
    /// both of the pit's thresholds, so whoever steps down into the pit —
    /// from either side — raises it under themselves.
    const BRIDGE_WALKOVER: &str = r#"{ "seed":1, "grid":64, "theme":"tech_base",
      "rooms":[
        { "id":"a", "footprint":[[0,0],[0,256],[256,256],[256,0]], "floor":0, "ceiling":192, "light":160,
          "floor_tex":"FLOOR4_8", "ceil_tex":"CEIL3_5", "wall_tex":"STARTAN3",
          "things":[ { "kind":"player1_start", "at":[64,64], "angle":0 } ] },
        { "id":"b", "footprint":[[320,0],[320,256],[576,256],[576,0]], "floor":0, "ceiling":192, "light":160,
          "floor_tex":"FLOOR4_8", "ceil_tex":"CEIL3_5", "wall_tex":"STARTAN3" }
      ],
      "portals":[ { "a":"a", "b":"b", "kind":"bridge", "width":64, "at":[256,128], "depth":96, "fires_on":"t" } ],
      "triggers":[ { "id":"t", "kind":"walkover", "portal":["a","b"] } ],
      "exits":[ { "room":"b", "trigger":"switch", "at":[576,128], "width":64 } ] }"#;

    /// The same bridge on the other axis and the other direction: room `a` is
    /// *north* of room `b`, so the portal's span runs along Y with
    /// `span.far < span.near`.
    ///
    /// The two rooms also differ in every property the pit could borrow —
    /// light, both flats, the wall texture and the ceiling — which
    /// [`BRIDGE`]'s matched pair cannot test: the pit must take room `a`'s
    /// look and the *lower* of the two ceilings. Only the floors match, as
    /// [`crate::ir::IrError::BridgeFloorsDiffer`] requires of every bridge.
    const BRIDGE_SOUTHWARD: &str = r#"{ "seed":1, "grid":64, "theme":"tech_base",
      "rooms":[
        { "id":"a", "footprint":[[0,320],[0,576],[256,576],[256,320]], "floor":0, "ceiling":192, "light":160,
          "floor_tex":"FLOOR4_8", "ceil_tex":"CEIL3_5", "wall_tex":"STARTAN3",
          "things":[ { "kind":"player1_start", "at":[128,448], "angle":0 } ] },
        { "id":"b", "footprint":[[0,0],[0,256],[256,256],[256,0]], "floor":0, "ceiling":256, "light":144,
          "floor_tex":"FLAT1", "ceil_tex":"FLAT20", "wall_tex":"BROWN1" }
      ],
      "portals":[ { "a":"a", "b":"b", "kind":"bridge", "width":64, "at":[128,320], "depth":96, "fires_on":"t" } ],
      "triggers":[ { "id":"t", "kind":"switch", "room":"a", "at":[128,576] } ],
      "exits":[ { "room":"b", "trigger":"switch", "at":[128,0], "width":64 } ] }"#;

    /// One room holding one 64x64 closet, lowered by a switch on the room's
    /// west wall, with an imp sealed inside the rock.
    ///
    /// Deliberately portal-free: a reveal is an island in a single room, so
    /// nothing about it needs a second room, and the four island edges are
    /// then the map's only two-sided lines.
    const CLOSET: &str = r#"{ "seed":1, "grid":64, "theme":"tech_base",
      "rooms":[
        { "id":"a", "footprint":[[0,0],[0,256],[256,256],[256,0]], "floor":0, "ceiling":192, "light":160,
          "floor_tex":"FLOOR4_8", "ceil_tex":"CEIL3_5", "wall_tex":"STARTAN3",
          "things":[ { "kind":"player1_start", "at":[64,64], "angle":0 } ] }
      ],
      "portals":[],
      "triggers":[ { "id":"t", "kind":"switch", "room":"a", "at":[0,128] } ],
      "reveals":[ { "id":"pen", "room":"a", "at":[128,128], "kind":"closet",
                    "things":[ { "kind":"imp", "at":[160,160], "angle":180 } ], "trigger":"t" } ],
      "exits":[ { "room":"a", "trigger":"switch", "at":[256,64], "width":64 } ] }"#;

    /// Everything the passes up to and including `emit_floors` produced.
    #[derive(Debug)]
    struct Built {
        data: MapData,
        tags: TagAllocator,
        triggers: Vec<TriggerOut>,
        floors: Vec<FloorActionOut>,
    }

    /// Runs the passes exactly as `compile_reporting` does, up to and
    /// including `emit_floors` — which is the last of them to emit anything
    /// but `emit_lifts`, the one pass that now runs after it — surfacing only
    /// this pass's own errors.
    fn build(json: &str) -> Result<Built, CompileError> {
        let ir = Ir::from_json(json).expect("ir");
        let tables = Tables::load().expect("tables");
        let mut data = sectors::emit_sectors(&ir).expect("sectors");
        sectors::resolve_secret_specials(&ir, &tables, &mut data);
        portals::cut_portals(&ir, &tables, &mut data).expect("portals");
        let mut tags = TagAllocator::new();
        doors::emit_doors(&ir, &tables, &mut data, &mut tags).expect("doors");
        exits::emit_exits(&ir, &tables, &mut data, &mut tags).expect("exits");
        teleports::emit_teleports(&ir, &tables, &mut data, &mut tags).expect("teleports");
        let (triggers, floors) = emit_floors(&ir, &tables, &mut data, &mut tags)?;
        Ok(Built {
            data,
            tags,
            triggers,
            floors,
        })
    }

    /// [`build`] for a map the pass is expected to accept.
    fn compile_data(json: &str) -> Built {
        build(json).expect("floors")
    }

    /// Every two-sided line, as `(the coordinate it holds constant, front
    /// sector, back sector)`, ordered by that coordinate.
    ///
    /// A drop wall's chain is a run of thresholds across the gap, each one
    /// perpendicular to the gap axis, so its constant coordinate is its
    /// position along that axis and the sorted list reads as the chain the
    /// player walks: room `a`, the near passage, the wall, the far passage,
    /// room `b` — or the reverse, on a portal whose span runs the other way.
    fn chain(data: &MapData) -> Vec<(i32, usize, usize)> {
        let mut lines: Vec<(i32, usize, usize)> = data
            .linedefs
            .iter()
            .filter_map(|l| {
                let back = l.back?;
                let (v1, v2) = (data.vertices[l.v1], data.vertices[l.v2]);
                let fixed = if v1.x == v2.x { v1.x } else { v1.y };
                Some((
                    fixed,
                    data.sidedefs[l.front].sector,
                    data.sidedefs[back].sector,
                ))
            })
            .collect();
        lines.sort_unstable();
        lines
    }

    /// One two-sided line's `(gap-side, room-side)` sidedefs, given which of
    /// the two sectors is the gap's own.
    ///
    /// Which of `front`/`back` is which is [`emit_opening`]'s business, not a
    /// caller's, so the tests read it off the emitted sidedefs rather than
    /// assuming an order.
    fn gap_and_room_sides(data: &MapData, line: usize, gap: usize) -> (usize, usize) {
        let l = &data.linedefs[line];
        let back = l.back.expect("a threshold is two-sided");
        if data.sidedefs[l.front].sector == gap {
            (l.front, back)
        } else {
            (back, l.front)
        }
    }

    /// The extent of `sector`'s own one-sided lines (its jambs) along the
    /// axis the gap runs on, as `(min, max)`.
    fn jamb_extent(data: &MapData, sector: usize, along_x: bool) -> (i32, i32) {
        let coords: Vec<i32> = data
            .linedefs
            .iter()
            .filter(|l| l.back.is_none() && data.sidedefs[l.front].sector == sector)
            .flat_map(|l| {
                let (v1, v2) = (data.vertices[l.v1], data.vertices[l.v2]);
                if along_x { [v1.x, v2.x] } else { [v1.y, v2.y] }
            })
            .collect();
        (
            *coords.iter().min().expect("the wall has jambs"),
            *coords.iter().max().expect("the wall has jambs"),
        )
    }

    #[test]
    fn a_drop_wall_rests_solid_at_the_lower_ceiling_and_lowers_to_the_floor_on_its_switch() {
        let Built {
            data,
            tags,
            triggers,
            floors,
        } = compile_data(WALL);

        assert_eq!(triggers.len(), 1);
        let t = &triggers[0];
        assert_eq!(
            (t.id.as_str(), t.family, t.walkover, t.activator),
            ("t", FloorFamily::LowerToLowest, false, 0)
        );
        let line = &data.linedefs[t.line];
        assert_eq!(
            (line.special, line.tag, line.back),
            (23, t.tag, None),
            "an S1 lowerFloorToLowest on a one-sided use line"
        );
        assert_eq!(data.sidedefs[line.front].middle, "SW1STARG");

        assert_eq!(floors.len(), 1);
        let f = &floors[0];
        let s = &data.sectors[f.sector];
        assert_eq!(
            (s.floor, s.ceiling, s.tag),
            (192, 192, t.tag),
            "solid at the lower of the two ceilings"
        );
        assert_eq!(
            (f.rest, f.dest, f.shape, f.family),
            (192, 0, FloorShape::DropWall, FloorFamily::LowerToLowest)
        );
        assert_eq!(f.portal, Some(0));
        assert_eq!(f.trigger, 0);

        // Both faces: the room-side lower is the wall texture, pegged.
        let faces: Vec<&LinedefOut> = data
            .linedefs
            .iter()
            .filter(|l| {
                l.back.is_some_and(|b| data.sidedefs[b].sector == f.sector)
                    || (data.sidedefs[l.front].sector == f.sector && l.back.is_some())
            })
            .collect();
        assert_eq!(faces.len(), 2, "the wall's only two-sided lines");
        assert_eq!(f.lines.len(), 2);
        for l in faces {
            let room_side = if data.sidedefs[l.front].sector == f.sector {
                l.back.expect("two-sided")
            } else {
                l.front
            };
            assert_eq!(data.sidedefs[room_side].lower, "STARTAN3");
            assert!(!l.lower_unpegged, "the face rides down with the floor");
        }
        // Room `b` stands 64 units taller than the wall, so its face also
        // needs an upper; room `a`'s ceiling is the wall's own, so it does not.
        let uppers: Vec<&str> = f
            .lines
            .iter()
            .map(|&i| {
                let l = &data.linedefs[i];
                data.sidedefs[l.front].upper.as_str()
            })
            .collect();
        assert_eq!(uppers, ["", "STARTAN3"]);

        // The manifest names the sector the tag ended up on, not the
        // placeholder it was allocated with.
        let entry = tags
            .manifest()
            .iter()
            .find(|e| e.tag == t.tag)
            .expect("the trigger's tag is in the manifest");
        assert_eq!(entry.sector, f.sector);
        assert_eq!(entry.purpose, "trigger t: drop wall a <-> b");
    }

    /// [`WALL`] with each room's floor tunable and a distinct wall texture
    /// per room, so a face that took the wrong room's cannot pass.
    ///
    /// Both ceilings are 192 here — [`WALL`] already varies them — because
    /// what this shape is for is the *floors*: a wall between rooms within a
    /// step of each other comes to rest at the lower room's floor, so one of
    /// its two faces then looks up at a passage standing above it, which no
    /// level-room fixture can reach.
    fn wall_between(floor_a: i32, floor_b: i32) -> String {
        format!(
            r#"{{ "seed":1, "grid":64, "theme":"tech_base",
      "rooms":[
        {{ "id":"a", "footprint":[[0,0],[0,256],[256,256],[256,0]], "floor":{floor_a}, "ceiling":192, "light":160,
          "floor_tex":"FLOOR4_8", "ceil_tex":"CEIL3_5", "wall_tex":"STARTAN3",
          "things":[ {{ "kind":"player1_start", "at":[128,128], "angle":0 }} ] }},
        {{ "id":"b", "footprint":[[320,0],[320,256],[576,256],[576,0]], "floor":{floor_b}, "ceiling":192, "light":144,
          "floor_tex":"FLAT1", "ceil_tex":"FLAT20", "wall_tex":"BROWN1" }}
      ],
      "portals":[ {{ "a":"a", "b":"b", "kind":"drop_wall", "width":64, "at":[256,128], "thickness":16, "fires_on":"t" }} ],
      "triggers":[ {{ "id":"t", "kind":"switch", "room":"a", "at":[0,128] }} ],
      "exits":[ {{ "room":"b", "trigger":"switch", "at":[576,128], "width":64 }} ] }}"#
        )
    }

    /// A wall between rooms at different floors rests at the shared ceiling
    /// but comes down to the *lower* room's floor, so toward the higher
    /// passage its own side is the one the engine draws. Both sides of both
    /// faces are checked, at 0 and at both signs of a 16 and a full 24 step.
    ///
    /// The level pair leads the list as the control: with nothing standing
    /// above where the wall lands, no back lower is written at all — which
    /// is why the committed goldens, whose rooms are level, are untouched.
    #[test]
    fn a_drop_wall_writes_its_own_lower_toward_a_passage_above_where_it_lands() {
        for (floor_a, floor_b) in [(0, 0), (24, 0), (0, 24), (16, 0), (0, 16)] {
            let Built { data, floors, .. } = compile_data(&wall_between(floor_a, floor_b));
            let f = &floors[0];
            let low = floor_a.min(floor_b);
            assert_eq!(
                (f.rest, f.dest),
                (192, low),
                "({floor_a}, {floor_b}): solid at the shared ceiling, down to the lower floor"
            );
            // The wall is a piece of the lower room — room `b` when it is the
            // lower one, which is the arm the level fixtures never take.
            let wall_tex = if floor_b < floor_a {
                "BROWN1"
            } else {
                "STARTAN3"
            };
            assert_eq!(data.sectors[f.sector].wall_tex, wall_tex);

            for (&line, (room_tex, room_floor)) in f
                .lines
                .iter()
                .zip([("STARTAN3", floor_a), ("BROWN1", floor_b)])
            {
                let l = &data.linedefs[line];
                let back = l.back.expect("a face is two-sided");
                assert_eq!(data.sidedefs[back].sector, f.sector);
                assert_eq!(
                    data.sidedefs[l.front].lower, room_tex,
                    "({floor_a}, {floor_b}): the room side carries its own texture at rest"
                );
                let want = if room_floor > low { wall_tex } else { "" };
                assert_eq!(
                    data.sidedefs[back].lower, want,
                    "({floor_a}, {floor_b}): the wall's own lower is written toward a passage \
                     standing above where it comes to rest, and nowhere else"
                );
            }
        }
    }

    #[test]
    fn a_drop_walls_gap_is_a_chain_of_two_passages_around_the_wall() {
        let Built { data, floors, .. } = compile_data(WALL);
        let f = &floors[0];

        // The thickness is the wall's depth; the rest of the gap is passage.
        assert_eq!(
            jamb_extent(&data, f.sector, true),
            (280, 296),
            "16 units centered in the 64-unit gap"
        );

        // The whole chain, in the order the player walks it: room `a`, the
        // near passage, the wall, the far passage, room `b`. Four thresholds,
        // at the two rooms' own walls and the wall's own two faces.
        let chain = chain(&data);
        assert_eq!(chain.len(), 4, "a | before | wall | after | b");
        let (before, after) = (chain[0].2, chain[3].2);
        assert_eq!(
            chain,
            [
                (256, 0, before),
                (280, before, f.sector),
                (296, after, f.sector),
                (320, 1, after),
            ]
        );
        // Each passage is a piece of its own room — the room's floor,
        // ceiling, light, flats and wall texture, and no tag of its own,
        // since only the wall moves. The two rooms' lights and ceilings
        // differ, so a passage that took the other room's would fail here.
        let near = &data.sectors[before];
        assert_eq!(
            (near.floor, near.ceiling, near.light, near.tag),
            (0, 192, 160, 0),
            "a piece of room `a`"
        );
        assert_eq!(
            (
                near.floor_tex.as_str(),
                near.ceil_tex.as_str(),
                near.wall_tex.as_str()
            ),
            ("FLOOR4_8", "CEIL3_5", "STARTAN3")
        );
        let far = &data.sectors[after];
        assert_eq!(
            (far.floor, far.ceiling, far.light, far.tag),
            (0, 256, 144, 0),
            "a piece of room `b`"
        );
    }

    #[test]
    fn a_walkover_trigger_goes_on_the_named_plain_portals_opening_line() {
        let Built {
            data,
            triggers,
            floors,
            ..
        } = compile_data(WALKOVER);

        let t = &triggers[0];
        assert!(t.walkover);
        assert_eq!(t.activator, 0, "the plain portal's own room `a`");
        let line = &data.linedefs[t.line];
        assert_eq!(line.special, 38, "a W1 lowerFloorToLowest");
        assert_eq!(line.tag, t.tag);
        let back = line
            .back
            .expect("the plain portal's opening line is two-sided");
        assert_eq!(
            (data.sidedefs[line.front].sector, data.sidedefs[back].sector),
            (0, 3),
            "room `a` in front, the plain portal's passage behind"
        );

        assert_eq!(floors.len(), 1);
        let f = &floors[0];
        assert_eq!(f.tag, t.tag, "the wall carries its trigger's tag");
        assert_eq!(data.sectors[f.sector].tag, t.tag);
        assert_eq!(
            (data.sectors[f.sector].floor, data.sectors[f.sector].ceiling),
            (192, 192),
            "the drop wall between `b` and `c` is sealed at rest"
        );
    }

    #[test]
    fn a_drop_wall_on_the_other_axis_and_the_other_direction_builds_the_same_chain() {
        // The rotation-blind failure this repo has already paid for once
        // (KNOWN-GAPS: 65 green tests over one rectangle hid four Critical
        // geometry defects). Here the gap runs along Y and room `a` is the
        // *far* end of it, so `dir` is negative and every position walks
        // downward from `span.near`.
        let Built {
            data,
            triggers,
            floors,
            ..
        } = compile_data(WALL_SOUTHWARD);

        let t = &triggers[0];
        assert_eq!(
            (t.family, t.walkover, t.activator),
            (FloorFamily::LowerToLowest, false, 0)
        );
        let switch = &data.linedefs[t.line];
        assert_eq!((switch.special, switch.tag, switch.back), (23, t.tag, None));
        assert_eq!(data.sidedefs[switch.front].middle, "SW1STARG");
        assert_eq!(
            (data.vertices[switch.v1].y, data.vertices[switch.v2].y),
            (576, 576),
            "the switch is on room `a`'s own horizontal north wall"
        );

        let f = &floors[0];
        let s = &data.sectors[f.sector];
        assert_eq!((s.floor, s.ceiling, s.tag), (192, 192, t.tag));
        assert_eq!((f.rest, f.dest, f.shape), (192, 0, FloorShape::DropWall));
        assert_eq!(
            (s.floor_tex.as_str(), s.wall_tex.as_str()),
            ("FLOOR4_8", "STARTAN3"),
            "the wall takes the low room's look, which with level floors is room `a`"
        );

        // The chain, read in increasing Y — which here runs from room `b` up
        // to room `a`, the reverse of the fixture's own near-to-far order.
        let chain = chain(&data);
        assert_eq!(chain.len(), 4, "b | after | wall | before | a");
        let (after, before) = (chain[0].2, chain[3].2);
        assert_eq!(
            chain,
            [
                (256, 1, after),
                (280, after, f.sector),
                (296, before, f.sector),
                (320, 0, before),
            ]
        );
        assert_eq!(
            jamb_extent(&data, f.sector, false),
            (280, 296),
            "16 units centered in the 64-unit gap, measured on Y this time"
        );

        // Each passage still takes its own room's look, and the two rooms
        // share none of it.
        let near = &data.sectors[before];
        assert_eq!(
            (
                near.ceiling,
                near.light,
                near.floor_tex.as_str(),
                near.wall_tex.as_str()
            ),
            (192, 160, "FLOOR4_8", "STARTAN3"),
            "a piece of room `a`"
        );
        let far = &data.sectors[after];
        assert_eq!(
            (
                far.ceiling,
                far.light,
                far.floor_tex.as_str(),
                far.wall_tex.as_str()
            ),
            (256, 144, "FLAT1", "BROWN1"),
            "a piece of room `b`"
        );

        // The faces: `lines` is (near, far) in the portal's own order, so the
        // near one is room `a`'s side at Y 296 and the far one room `b`'s at
        // 280. Each shows its own side's wall texture, and only room `b`'s
        // side — the taller — also shows an upper.
        let face = |i: usize| {
            let l = &data.linedefs[f.lines[i]];
            let side = &data.sidedefs[l.front];
            (
                data.vertices[l.v1].y,
                side.lower.as_str(),
                side.upper.as_str(),
                l.lower_unpegged,
                l.upper_unpegged,
            )
        };
        assert_eq!(face(0), (296, "STARTAN3", "", false, false));
        assert_eq!(face(1), (280, "BROWN1", "BROWN1", false, false));
    }

    #[test]
    fn a_walkover_refuses_a_line_that_already_carries_a_special() {
        // `Ir::from_json` refuses two walkovers on one portal
        // (`WalkoverPortalClaimedTwice`), so no authored map reaches this —
        // the pass is driven directly, the way
        // `emit_floors_refuses_a_theme_the_texture_table_does_not_name` drives
        // it past a check `compile` makes first. The plain portal's two
        // thresholds are the only two-sided lines standing before
        // `emit_floors` runs, so marking every one of them marks exactly the
        // line the walkover is about to claim.
        let ir = Ir::from_json(WALKOVER).expect("ir");
        let tables = Tables::load().expect("tables");
        let mut data = sectors::emit_sectors(&ir).expect("sectors");
        sectors::resolve_secret_specials(&ir, &tables, &mut data);
        portals::cut_portals(&ir, &tables, &mut data).expect("portals");
        let mut tags = TagAllocator::new();
        doors::emit_doors(&ir, &tables, &mut data, &mut tags).expect("doors");
        exits::emit_exits(&ir, &tables, &mut data, &mut tags).expect("exits");
        teleports::emit_teleports(&ir, &tables, &mut data, &mut tags).expect("teleports");

        let claimed: Vec<usize> = data
            .linedefs
            .iter()
            .enumerate()
            .filter(|(_, l)| l.back.is_some())
            .map(|(i, _)| i)
            .collect();
        assert_eq!(claimed.len(), 2, "the plain portal's two thresholds");
        for &i in &claimed {
            data.linedefs[i].special = 1;
        }

        let err = emit_floors(&ir, &tables, &mut data, &mut tags)
            .expect_err("the opening line is already claimed");
        assert!(
            matches!(&err, CompileError::TriggerLineAlreadyClaimed { id, line }
                if id == "t" && claimed.contains(line)),
            "expected TriggerLineAlreadyClaimed naming the trigger and the line, got {err}"
        );
    }

    #[test]
    fn a_drop_wall_whose_rooms_floors_differ_by_more_than_a_step_is_rejected() {
        let json = WALL.replace(
            r#""id":"b", "footprint":[[320,0],[320,256],[576,256],[576,0]], "floor":0"#,
            r#""id":"b", "footprint":[[320,0],[320,256],[576,256],[576,0]], "floor":32"#,
        );
        let err = build(&json).expect_err("a 32-unit difference clears the 24-unit step");
        assert!(
            matches!(
                &err,
                CompileError::DropWallFloorsDiffer { a, b, floor_a, floor_b, step }
                    if a == "a" && b == "b" && *floor_a == 0 && *floor_b == 32 && *step == 24
            ),
            "expected DropWallFloorsDiffer naming both floors, got {err}"
        );
    }

    #[test]
    fn a_drop_wall_deeper_than_its_gap_is_rejected() {
        // A 32-unit alcove leaves half of the 64-unit gap for a 64-deep wall.
        let json = WALL.replace(r#""thickness":16"#, r#""thickness":64, "alcove_near":32"#);
        let err = build(&json).expect_err("a 64-deep wall does not fit a 32-unit gap");
        assert!(
            matches!(
                &err,
                CompileError::DropWallTooThick { thickness, gap, .. } if *thickness == 64 && *gap == 32
            ),
            "expected DropWallTooThick naming the thickness and the free gap, got {err}"
        );
    }

    #[test]
    fn a_bridge_rests_depth_below_its_rooms_and_rises_to_their_floor() {
        let Built {
            data,
            triggers,
            floors,
            ..
        } = compile_data(BRIDGE);

        let t = &triggers[0];
        let switch = &data.linedefs[t.line];
        assert_eq!(
            (t.family, switch.special, switch.tag, switch.back),
            (FloorFamily::RaiseToNearest, 18, t.tag, None),
            "an S1 raiseFloorToNearest on a one-sided use line"
        );

        assert_eq!(floors.len(), 1);
        let f = &floors[0];
        let s = &data.sectors[f.sector];
        assert_eq!(
            (s.floor, s.ceiling, s.tag),
            (-96, 192, t.tag),
            "a pit 96 below the rooms' shared floor, under their ceiling"
        );
        assert_eq!(
            (f.rest, f.dest, f.shape, f.family),
            (-96, 0, FloorShape::Bridge, FloorFamily::RaiseToNearest)
        );
        assert_eq!((f.portal, f.reveal, f.trigger), (Some(0), None, 0));
        assert_eq!(
            (s.floor_tex.as_str(), s.wall_tex.as_str(), s.light),
            ("FLOOR4_8", "SUPPORT3", 160),
            "room `a`'s look, with the riser as the pit's own wall texture"
        );

        // The pit fills the whole gap — a bridge portal declares no alcoves
        // and no thickness — so its jambs run the gap's full 64 units, and
        // they are the chasm's rock rather than a riser: the room's wall
        // texture, as a plain portal's passage takes.
        assert_eq!(jamb_extent(&data, f.sector, true), (256, 320));
        let jambs: Vec<&str> = data
            .linedefs
            .iter()
            .filter(|l| l.back.is_none() && data.sidedefs[l.front].sector == f.sector)
            .map(|l| data.sidedefs[l.front].middle.as_str())
            .collect();
        assert_eq!(
            jambs,
            ["STARTAN3", "STARTAN3"],
            "the pit's two side walls are wall, not the riser its faces carry"
        );

        // Its only two-sided neighbors are the two rooms, which is what makes
        // `P_FindNextHighestFloor` from the pit exactly their shared floor.
        assert_eq!(
            chain(&data),
            [(256, 0, f.sector), (320, 1, f.sector)],
            "room `a` | the pit | room `b`, and nothing else"
        );
        assert_eq!(f.lines.len(), 2);
        for &line in &f.lines {
            let l = &data.linedefs[line];
            assert_eq!(
                (l.special, l.tag),
                (0, 0),
                "a switch-triggered bridge carries its special on the switch, not on the pit"
            );
        }

        // The gap-side lowers toward each room carry the riser, pegged; the
        // room-side lowers are left blank for `heights` (nothing shows at
        // rest, since the pit is the lower floor on both thresholds).
        // Re-derived from the emitted lines rather than read off `f.lines`,
        // so a construct that recorded the wrong indices cannot pass.
        let faces: Vec<usize> = data
            .linedefs
            .iter()
            .enumerate()
            .filter(|(_, l)| {
                l.back.is_some_and(|b| data.sidedefs[b].sector == f.sector)
                    || (data.sidedefs[l.front].sector == f.sector && l.back.is_some())
            })
            .map(|(i, _)| i)
            .collect();
        assert_eq!(faces, f.lines, "the pit's only two-sided lines");
        for line in faces {
            let (gap_side, room_side) = gap_and_room_sides(&data, line, f.sector);
            assert_eq!(data.sidedefs[gap_side].lower, "SUPPORT3");
            assert!(
                data.sidedefs[room_side].lower.is_empty(),
                "the side the engine never draws is left bare"
            );
            assert!(
                !data.linedefs[line].lower_unpegged,
                "the riser rides up with the pit floor"
            );
        }
    }

    #[test]
    fn a_walkover_naming_a_bridge_writes_its_special_on_both_pit_thresholds() {
        let Built {
            data,
            triggers,
            floors,
            ..
        } = compile_data(BRIDGE_WALKOVER);

        let t = &triggers[0];
        assert_eq!(
            (t.family, t.walkover, t.activator),
            (FloorFamily::RaiseToNearest, true, 0),
            "the bridge portal's own room `a` fronts the near threshold"
        );

        let f = &floors[0];
        assert_eq!((f.shape, f.rest, f.dest), (FloorShape::Bridge, -96, 0));
        assert_eq!(f.lines.len(), 2);
        for &line in &f.lines {
            let l = &data.linedefs[line];
            assert_eq!(
                (l.special, l.tag),
                (119, t.tag),
                "a W1 raiseFloorToNearest on both of the pit's thresholds"
            );
            let back = l.back.expect("a pit threshold is two-sided");
            assert!(
                data.sidedefs[l.front].sector == f.sector || data.sidedefs[back].sector == f.sector,
                "both written lines border the pit"
            );
        }
        assert!(
            f.lines.contains(&t.line),
            "the recorded trigger line is one of the two written"
        );
    }

    #[test]
    fn a_bridge_on_the_other_axis_takes_room_as_look_and_the_lower_ceiling() {
        // The rotation-blind failure this repo has already paid for once
        // (KNOWN-GAPS: 65 green tests over one rectangle hid four Critical
        // geometry defects). Here the gap runs along Y with room `a` at the
        // *far* end of it, and the two rooms share nothing but their floor.
        let Built { data, floors, .. } = compile_data(BRIDGE_SOUTHWARD);
        let f = &floors[0];
        let s = &data.sectors[f.sector];
        assert_eq!(
            (s.floor, s.ceiling, s.light),
            (-96, 192, 160),
            "96 below the shared floor, under the lower of the two ceilings, in room `a`'s light"
        );
        assert_eq!(
            (
                s.floor_tex.as_str(),
                s.ceil_tex.as_str(),
                s.wall_tex.as_str()
            ),
            ("FLOOR4_8", "CEIL3_5", "SUPPORT3"),
            "room `a`'s flats, and the riser as the pit's own wall texture"
        );
        assert_eq!((f.rest, f.dest, f.shape), (-96, 0, FloorShape::Bridge));

        // The chain, read in increasing Y — which here runs from room `b` up
        // to room `a`, the reverse of the fixture's own near-to-far order.
        assert_eq!(
            chain(&data),
            [(256, 1, f.sector), (320, 0, f.sector)],
            "room `b` | the pit | room `a`, and nothing else"
        );
        assert_eq!(
            jamb_extent(&data, f.sector, false),
            (256, 320),
            "the gap's full 64 units, measured on Y this time"
        );
        for &line in &f.lines {
            let (gap_side, room_side) = gap_and_room_sides(&data, line, f.sector);
            assert_eq!(data.sidedefs[gap_side].lower, "SUPPORT3");
            assert!(
                data.sidedefs[room_side].lower.is_empty(),
                "the side the engine never draws is left bare, on either room"
            );
            assert!(!data.linedefs[line].lower_unpegged);
        }
    }

    /// The accept path runs through [`compile`], which raises
    /// [`CompileError::Playability`] on any violation: a walkover-raised
    /// bridge is a clean map end to end, P7 included, now that the flood
    /// carries a fired floor action in its state.
    #[test]
    fn a_bridges_riser_survives_the_passes_that_run_after_it() {
        let ir = Ir::from_json(BRIDGE_WALKOVER).expect("ir");
        let tables = Tables::load().expect("tables");
        let out = compile(&ir, &tables).expect("a pit the walkover raises is a legal map");

        // `heights::apply_height_textures` fills only empty slots, so the
        // riser this pass wrote onto the pit side is still there afterward —
        // and the room side, which the engine never draws, is still bare.
        let f = &out.floors[0];
        for &line in &f.lines {
            let (gap_side, room_side) = gap_and_room_sides(&out.data, line, f.sector);
            assert_eq!(out.data.sidedefs[gap_side].lower, "SUPPORT3");
            assert!(out.data.sidedefs[room_side].lower.is_empty());
        }
    }

    #[test]
    fn a_bridge_no_deeper_than_a_step_is_rejected() {
        // 24 is exactly the step: the player would climb straight out of the
        // pit, so the rising strip is scenery rather than a bridge.
        let shallow_pit = BRIDGE.replace(r#""depth":96"#, r#""depth":24"#);
        let err = build(&shallow_pit).expect_err("a 24-unit pit is within the 24-unit step");
        assert!(
            matches!(
                &err,
                CompileError::BridgeDepthTooLow { a, b, depth, step }
                    if a == "a" && b == "b" && *depth == 24 && *step == 24
            ),
            "expected BridgeDepthTooLow naming both rooms, the depth and the step, got {err}"
        );

        // 32 — the next multiple of 8 above the step, which is all the IR
        // requires of a depth — is the first accepted one, and it emits.
        let one_step_over = BRIDGE.replace(r#""depth":96"#, r#""depth":32"#);
        let Built { data, floors, .. } = compile_data(&one_step_over);
        assert_eq!(
            (data.sectors[floors[0].sector].floor, floors[0].rest),
            (-32, -32),
            "a pit one tile deeper than the step still rests below it"
        );
    }

    #[test]
    fn a_bridge_whose_gap_is_narrower_than_the_player_is_rejected() {
        // Room `b` pulled in to 24 units from room `a`, on an 8-unit grid
        // since the fixture's own 64 cannot express it. The pit fills the
        // whole gap, so a 24-unit gap is a 24-unit strip — and the player is
        // 32 units across, so they could never stand on the risen bridge.
        let narrow = BRIDGE
            .replace(r#""grid":64"#, r#""grid":8"#)
            .replace(
                r"[[320,0],[320,256],[576,256],[576,0]]",
                r"[[280,0],[280,256],[536,256],[536,0]]",
            )
            .replace(r#""at":[576,128]"#, r#""at":[536,128]"#);
        let err = build(&narrow).expect_err("a 24-unit gap is narrower than the player");
        assert!(
            matches!(
                &err,
                CompileError::BridgeTooShallow { a, b, depth, need }
                    if a == "a" && b == "b" && *depth == 24 && *need == 32
            ),
            "expected BridgeTooShallow naming the gap and the player's diameter, got {err}"
        );
    }

    #[test]
    fn a_closet_rests_solid_at_the_hosts_ceiling_and_lowers_to_its_floor() {
        let Built {
            data,
            triggers,
            floors,
            ..
        } = compile_data(CLOSET);
        let f = &floors[0];
        let s = &data.sectors[f.sector];
        assert_eq!(
            (s.floor, s.ceiling, s.tag, s.host),
            (192, 192, triggers[0].tag, Some(0)),
            "solid rock at the host's ceiling, carrying the trigger's tag, hosted by room `a`"
        );
        assert_eq!(
            (f.rest, f.dest, f.shape, f.reveal, f.portal),
            (192, 0, FloorShape::Closet, Some(0), None)
        );
        assert!(
            f.lines.is_empty(),
            "a reveal has no gap segment, so it hands back no thresholds"
        );
        let edges: Vec<&LinedefOut> = data
            .linedefs
            .iter()
            .filter(|l| l.back.is_some_and(|b| data.sidedefs[b].sector == f.sector))
            .collect();
        assert_eq!(edges.len(), 4, "an island's four edges, host on the front");

        // The rectangle `Reveal::rect` names, walked counter-clockwise from
        // its low corner — east along the south edge first, which is the
        // winding `emit_island_edges` documents and the one a hole in a Doom
        // sector needs. Read off `data.vertices` rather than off the IR, so
        // a `rect()` miswired into `emit_island_edges` cannot pass.
        let corners = [
            Pt { x: 128, y: 128 },
            Pt { x: 192, y: 128 },
            Pt { x: 192, y: 192 },
            Pt { x: 128, y: 192 },
        ];
        for (k, l) in edges.iter().enumerate() {
            assert_eq!(
                (data.vertices[l.v1], data.vertices[l.v2]),
                (corners[k], corners[(k + 1) % corners.len()]),
                "edge {k} runs counter-clockwise between the rectangle's own corners"
            );
            assert_eq!(
                data.sidedefs[l.front].sector, 0,
                "the host fronts every edge"
            );
            assert_eq!(data.sidedefs[l.front].lower, "STARTAN3");
            assert_eq!(
                l.special, 0,
                "a reveal's own edges carry no special; the trigger is elsewhere"
            );
            assert!(!l.lower_unpegged);
        }
    }

    #[test]
    fn a_pedestal_reveal_rests_its_rise_above_the_host_under_the_hosts_ceiling() {
        let json = CLOSET.replace(r#""kind":"closet","#, r#""kind":"pedestal", "rise":64,"#);
        let Built { data, floors, .. } = compile_data(&json);
        let s = &data.sectors[floors[0].sector];
        assert_eq!((s.floor, s.ceiling), (64, 192));
        assert_eq!(
            (floors[0].rest, floors[0].dest, floors[0].shape),
            (64, 0, FloorShape::Pedestal)
        );
    }

    /// The accept path runs through [`compile`], which raises
    /// [`CompileError::Playability`] on any violation: an **empty** sealed
    /// closet the switch opens is a clean map end to end, P7 included, now
    /// that the flood carries a fired floor action in its state.
    ///
    /// Empty because a closet can hold nothing (ruling R28): at rest its
    /// floor is its ceiling, and the engine will not lower a floor a thing
    /// does not fit in — see [`CompileError::RevealNoHeadroom`] for the
    /// pinned lines. The refusal itself is
    /// `things::tests::a_closet_holds_nothing_because_a_blocked_floor_never_lowers`;
    /// what this pins is that emptying it leaves a map that still compiles.
    #[test]
    fn an_empty_closet_compiles_end_to_end() {
        let empty = CLOSET.replace(
            r#""things":[ { "kind":"imp", "at":[160,160], "angle":180 } ], "#,
            "",
        );
        assert_ne!(empty, CLOSET, "the patch changed nothing");
        let ir = Ir::from_json(&empty).expect("ir");
        let tables = Tables::load().expect("tables");
        let out = compile(&ir, &tables).expect("an empty closet is a clean map");
        assert_eq!(
            out.floors.len(),
            1,
            "the closet is still emitted; only its cargo is gone"
        );
        let imp = tables.thing_id("imp").expect("the vocabulary names an imp");
        assert!(
            !out.things.iter().any(|t| t.kind == imp),
            "nothing is sealed in the rock"
        );
    }

    /// Clearance is measured against the **host's** boundary, not the
    /// cell's, and it is measured **before** headroom — so a cargo that
    /// fails both is reported for the clearance, which is what this pins.
    ///
    /// The cell (96,184)-(160,248) hugs the room's north wall, 8 units clear
    /// of it, and the imp inside stands at (120,244): 12 units from that
    /// wall and only 4 from the cell's own north edge, against its 20-unit
    /// radius. Both distances fail the radius, so the *reported* `have` is
    /// what separates the two rules — 12.0 is the wall, 4.0 would be the
    /// edge — and it is asserted below rather than elided.
    #[test]
    fn a_reveals_cargo_is_cleared_against_the_host_wall_before_it_is_measured_for_height() {
        let tables = Tables::load().expect("tables");
        let hugging = CLOSET
            .replace(
                r#""at":[128,128], "kind":"closet""#,
                r#""at":[96,184], "kind":"closet""#,
            )
            .replace(r#""at":[160,160]"#, r#""at":[120,244]"#);
        let ir = Ir::from_json(&hugging).expect("ir");
        let err = compile_reporting(&ir, &tables).expect_err("no clearance from the host's wall");
        let CompileError::ThingTooClose {
            room,
            kind,
            x,
            y,
            have,
            need,
        } = &err
        else {
            panic!("expected ThingTooClose naming the host room, got {err}");
        };
        assert_eq!(
            (room.as_str(), kind.as_str(), *x, *y, *need),
            ("a", "imp", 120, 244, 20)
        );
        assert!(
            (*have - 12.0).abs() < 1e-9,
            "the 12 units to the room's north wall, not the 4 to the cell's own north edge: \
             {have}"
        );
    }

    #[test]
    fn a_room_thing_standing_on_a_reveal_is_refused() {
        let json = CLOSET.replace(
            r#""things":[ { "kind":"player1_start", "at":[64,64], "angle":0 } ]"#,
            r#""things":[ { "kind":"player1_start", "at":[64,64], "angle":0 }, { "kind":"medikit", "at":[150,150], "angle":0 } ]"#,
        );
        let ir = Ir::from_json(&json).expect("ir");
        let tables = Tables::load().expect("tables");
        let err = compile(&ir, &tables).expect_err("on the closet");
        assert!(
            matches!(err, CompileError::ThingOnReveal { ref reveal, .. } if reveal == "pen"),
            "expected ThingOnReveal naming the reveal, got {err}"
        );
    }

    #[test]
    fn a_pedestal_reveal_reaching_its_hosts_ceiling_is_rejected() {
        // Room `a` is 192 tall, so 192 puts the cell's floor exactly on its
        // ceiling — an inverted sector that would load and render as
        // garbage, since nothing downstream reads an island's heights. A
        // block resting at the ceiling is a closet, not a pedestal.
        let at_ceiling = CLOSET.replace(r#""kind":"closet","#, r#""kind":"pedestal", "rise":192,"#);
        let err = build(&at_ceiling).expect_err("a 192 rise reaches a 192 ceiling");
        assert!(
            matches!(
                &err,
                CompileError::RevealRiseTooHigh { reveal, rise, max }
                    if reveal == "pen" && *rise == 192 && *max == 192
            ),
            "expected RevealRiseTooHigh naming the reveal, the rise and the host's height, got \
             {err}"
        );

        // One tile under it is the last accepted rise, and it emits.
        let under = CLOSET.replace(r#""kind":"closet","#, r#""kind":"pedestal", "rise":184,"#);
        let Built { data, floors, .. } = compile_data(&under);
        let s = &data.sectors[floors[0].sector];
        assert_eq!(
            (s.floor, s.ceiling),
            (184, 192),
            "a rise strictly under the host's height still rests below its ceiling"
        );
    }

    #[test]
    fn a_pedestal_reveal_within_a_step_of_its_host_is_rejected() {
        // 24 is exactly the step: the player would walk onto the block
        // rather than wait for the trigger to drop it.
        let json = CLOSET.replace(r#""kind":"closet","#, r#""kind":"pedestal", "rise":24,"#);
        let err = build(&json).expect_err("a 24-unit rise is within the 24-unit step");
        assert!(
            matches!(
                &err,
                CompileError::RevealRiseTooLow { reveal, rise, step }
                    if reveal == "pen" && *rise == 24 && *step == 24
            ),
            "expected RevealRiseTooLow naming the reveal, the rise and the step, got {err}"
        );
    }

    #[test]
    fn emit_floors_refuses_a_theme_the_texture_table_does_not_name() {
        // `compile` never reaches this pass with an unresolvable theme —
        // `emit_sectors` refuses it first — so the switch lookups are
        // reachable only by driving the pass directly. Every earlier pass
        // gets the real theme; only `emit_floors` sees the bad one.
        let mut ir = Ir::from_json(WALL).expect("ir");
        let tables = Tables::load().expect("tables");
        let mut data = sectors::emit_sectors(&ir).expect("sectors");
        sectors::resolve_secret_specials(&ir, &tables, &mut data);
        portals::cut_portals(&ir, &tables, &mut data).expect("portals");
        let mut tags = TagAllocator::new();
        doors::emit_doors(&ir, &tables, &mut data, &mut tags).expect("doors");
        exits::emit_exits(&ir, &tables, &mut data, &mut tags).expect("exits");
        teleports::emit_teleports(&ir, &tables, &mut data, &mut tags).expect("teleports");

        ir.theme = "no_such_theme".to_owned();
        let err = emit_floors(&ir, &tables, &mut data, &mut tags)
            .expect_err("the theme resolves to no texture set");
        assert!(
            matches!(&err, CompileError::UnknownTheme { theme } if theme == "no_such_theme"),
            "expected UnknownTheme naming the theme asked for, got {err}"
        );
    }
}
