//! Cuts openings into two rooms' own facing walls and fills the void between
//! them.
//!
//! Rooms are authored apart, never flush (see [`crate::ir::Portal`]'s doc
//! comment and [`crate::ir::Ir::MIN_PORTAL_GAP`]), so a portal's two rooms
//! never share a single coincident wall the way the pre-gap-model compiler
//! assumed. Instead, room `a`'s wall and room `b`'s wall face each other
//! across a real void, and a portal fills it: an opening is cut into *each*
//! room's own wall, and a new sector spans the gap between the two openings.
//! For a [`crate::ir::PortalKind::Plain`] portal that new sector is an open,
//! walkable passage; [`crate::compile::doors`] builds the same shape for a
//! door, just with a closed sector and a special on its threshold lines
//! instead of an open one with neither.

use crate::compile::sectors::vertex_index;
use crate::compile::{CompileError, LinedefOut, MapData, SectorOut, SidedefOut};
use crate::geom::{Axis, FacingSpan, Pt, facing_spans, find_facing_span, on_diagonal_wall};
use crate::ir::{Ir, Portal, PortalKind};
use crate::tables::Tables;

/// Opens every portal into the facing walls of its two rooms and fills the
/// gap between them.
///
/// Every portal is resolved and cross-checked before anything is emitted, so
/// a rejected map leaves no partially-cut geometry behind. Each room's solid
/// wall is then split at the opening's two ends — the pieces outside the
/// opening survive with their own endpoints and textures intact. For a
/// [`PortalKind::Plain`] portal this also fills the gap with an open passage
/// sector (see `emit_gap_sector`); a door portal's gap is instead filled by
/// [`crate::compile::doors::emit_doors`], which runs afterward and expects to
/// find both rooms' flanking walls already cut but the gap itself still
/// empty.
///
/// # Errors
/// Returns [`CompileError::PortalOnDiagonalWall`] when the opening sits on a
/// diagonal wall, which v1 does not support hosting a portal on,
/// [`CompileError::NotAdjacent`] when the rooms share no facing wall,
/// [`CompileError::PortalOffWall`] when the midpoint is not on a facing wall,
/// [`CompileError::PortalTooWide`] when the opening exceeds the facing span,
/// [`CompileError::OverlappingPortals`] when two openings overlap on the same
/// wall line, [`CompileError::OpeningNotInAWall`] when no single solid wall
/// of a room spans the opening, and [`CompileError::PortalNoHeadroom`] when a
/// plain portal's passage sector would have too little (or no) headroom for
/// the player.
pub fn cut_portals(ir: &Ir, tables: &Tables, data: &mut MapData) -> Result<(), CompileError> {
    let resolved = ir
        .portals
        .iter()
        .map(|portal| Ok((portal, resolve_portal(ir, portal)?)))
        .collect::<Result<Vec<_>, CompileError>>()?;

    check_no_overlapping_openings(&resolved)?;
    check_headroom(ir, tables, &resolved)?;

    for (portal, geometry) in &resolved {
        cut_one(ir, data, portal, geometry)?;
    }
    Ok(())
}

/// Rejects a plain portal whose two rooms leave too little headroom in the
/// passage sector between them.
///
/// Runs here, in the pre-emission pass alongside
/// [`check_no_overlapping_openings`], over every resolved portal before any
/// wall is split — not inside `cut_one`, which runs after
/// `split_wall_for_opening` has already mutated `data`. Checking it here is
/// what makes this module's own doc comment ("every portal is resolved and
/// cross-checked before anything is emitted") actually true: a rejected map
/// leaves no partially-cut geometry behind. Only [`PortalKind::Plain`]
/// portals are checked — see [`CompileError::PortalNoHeadroom`]'s doc
/// comment for why a door portal is exempt.
fn check_headroom(
    ir: &Ir,
    tables: &Tables,
    resolved: &[(&Portal, PortalGeometry)],
) -> Result<(), CompileError> {
    let need = tables.player().height;
    for (portal, geometry) in resolved {
        if portal.kind != PortalKind::Plain {
            continue;
        }
        let room_a = &ir.rooms[geometry.ia];
        let room_b = &ir.rooms[geometry.ib];
        let have = room_a.ceiling.min(room_b.ceiling) - room_a.floor.max(room_b.floor);
        if have < need {
            return Err(CompileError::PortalNoHeadroom {
                a: portal.a.clone(),
                b: portal.b.clone(),
                have,
                need,
            });
        }
    }
    Ok(())
}

/// Rejects two portals whose openings overlap along the same wall line.
///
/// Splitting a wall consumes the piece that spans the opening, so a second
/// opening overlapping the first would find no intact wall to split. Catching
/// it here names both portals instead of surfacing the downstream
/// [`CompileError::OpeningNotInAWall`]. Every portal contributes *two*
/// wall-line cuts now — room `a`'s own wall and room `b`'s own wall, which
/// (rooms being authored apart) are never the same coordinate — so both are
/// checked against every other portal's own two. Keying on `(axis, fixed)`
/// rather than on the room pair covers the collinear case too: two portals
/// into *different* rooms can still share one wall line.
fn check_no_overlapping_openings(
    resolved: &[(&Portal, PortalGeometry)],
) -> Result<(), CompileError> {
    let cuts: Vec<(&Portal, Axis, i32, i32, i32)> = resolved
        .iter()
        .flat_map(|(portal, g)| {
            [
                (*portal, g.span.axis, g.span.near, g.open_lo, g.open_hi),
                (*portal, g.span.axis, g.span.far, g.open_lo, g.open_hi),
            ]
        })
        .collect();

    for (i, (first, axis1, fixed1, lo1, hi1)) in cuts.iter().enumerate() {
        for (second, axis2, fixed2, lo2, hi2) in &cuts[i + 1..] {
            let collinear = axis1 == axis2 && fixed1 == fixed2;
            let overlaps = lo1 < hi2 && lo2 < hi1;
            if collinear && overlaps {
                return Err(CompileError::OverlappingPortals {
                    first: format!("{} <-> {}", first.a, first.b),
                    second: format!("{} <-> {}", second.a, second.b),
                });
            }
        }
    }
    Ok(())
}

fn cut_one(
    ir: &Ir,
    data: &mut MapData,
    portal: &Portal,
    geometry: &PortalGeometry,
) -> Result<(), CompileError> {
    let near_cut = Cut {
        axis: geometry.span.axis,
        fixed: geometry.span.near,
        open_lo: geometry.open_lo,
        open_hi: geometry.open_hi,
    };
    let far_cut = Cut {
        axis: geometry.span.axis,
        fixed: geometry.span.far,
        open_lo: geometry.open_lo,
        open_hi: geometry.open_hi,
    };

    // Split each room's own solid wall at the opening — room `a`'s at its
    // own wall coordinate, room `b`'s at its own. Splitting rather than
    // dropping-and-recreating is what makes a portal between rooms whose
    // walls do not run the same full length work at all: the facing span is
    // only the part of each wall the two rooms have in common, so the
    // surviving pieces must run out to each wall's *own* endpoints, not to
    // the facing span's.
    split_wall_for_opening(data, &near_cut, geometry.ia, &portal.a)?;
    split_wall_for_opening(data, &far_cut, geometry.ib, &portal.b)?;

    // A plain portal fills the gap with an open passage sector immediately.
    // A door portal instead gets a closed sector of its own — see
    // `doors::emit_doors`, which runs after `cut_portals` and expects to
    // find both flanking walls already cut but the gap itself still empty.
    if portal.kind == PortalKind::Plain {
        let room_a = &ir.rooms[geometry.ia];
        let room_b = &ir.rooms[geometry.ib];
        // `check_headroom` has already rejected any plain portal whose
        // rooms leave too little headroom, before `cut_portals` cut a
        // single wall — see that function's doc comment.
        let floor = room_a.floor.max(room_b.floor);
        let ceiling = room_a.ceiling.min(room_b.ceiling);
        let sector_out = SectorOut {
            floor,
            ceiling,
            light: room_a.light,
            floor_tex: room_a.floor_tex.clone(),
            ceil_tex: room_a.ceil_tex.clone(),
            special: 0,
            tag: 0,
            wall_tex: room_a.wall_tex.clone(),
        };
        let segment = emit_gap_sector(
            data,
            &geometry.span,
            geometry.open_lo,
            geometry.open_hi,
            geometry.ia,
            geometry.ib,
            sector_out,
            &room_a.wall_tex,
        );
        mark_secret_thresholds(
            data,
            room_a.secret != room_b.secret,
            [segment.near_line, segment.far_line],
        );
    }

    Ok(())
}

/// The span geometry of one wall's cut: which axis and fixed coordinate the
/// wall runs along, and the two along-axis ends of the opening.
///
/// `pub(crate)` (including every field) so `exits` can build one directly
/// from a single room's own wall, without going through [`emit_gap_sector`]'s
/// two-room construction.
pub(crate) struct Cut {
    /// The axis the wall runs along.
    pub(crate) axis: Axis,
    /// The coordinate held constant along the wall (X for vertical, Y for
    /// horizontal).
    pub(crate) fixed: i32,
    /// The low end of the opening.
    pub(crate) open_lo: i32,
    /// The high end of the opening.
    pub(crate) open_hi: i32,
}

impl Cut {
    /// The point at `along` distance along this cut's axis.
    pub(crate) fn pt(&self, along: i32) -> Pt {
        match self.axis {
            Axis::Vertical => Pt {
                x: self.fixed,
                y: along,
            },
            Axis::Horizontal => Pt {
                x: along,
                y: self.fixed,
            },
        }
    }
}

/// Splits the one solid wall of `sector` that spans the opening into the
/// pieces lying outside it, leaving the opening itself bare for
/// [`emit_opening`] (or, for a door portal, for `doors::emit_doors`) to fill.
///
/// Splitting in place is what makes an opening narrower than a room's wall
/// work. The earlier drop-and-recreate approach only recognized a wall whose
/// endpoints matched the shared span exactly, so any wall extending past the
/// span — the normal case for rooms that are not identical and perfectly
/// flush — survived untouched, and the flanking pieces and the opening were
/// then stacked on top of it: a portal sealed behind a wall, with coincident
/// overlapping linedefs and no error raised.
///
/// The surviving pieces keep the original wall's `v1`-to-`v2` direction, so
/// their front sidedefs still face the sector interior without this function
/// needing to know which side of the wall that is. The first piece reuses the
/// original sidedef record (textures included, so each room keeps its *own*
/// wall texture rather than inheriting room A's); a second piece, when the
/// opening does not reach either end, gets a clone of it.
///
/// # Errors
/// Returns [`CompileError::OpeningNotInAWall`] when no single one-sided wall
/// of `sector` lies on the cut's line and covers the whole opening.
///
/// `pub(crate)` so `exits::emit_exits` can carve an exit's span out of its
/// host room's wall with the exact same machinery `cut_portals` and
/// `doors::emit_doors` already use — an exit is "the same machinery, minus
/// the second room".
pub(crate) fn split_wall_for_opening(
    data: &mut MapData,
    cut: &Cut,
    sector: usize,
    room: &str,
) -> Result<(), CompileError> {
    let index = data
        .linedefs
        .iter()
        .position(|line| {
            if line.back.is_some() || data.sidedefs[line.front].sector != sector {
                return false;
            }
            let (along_1, across_1) = cut.axis.split(data.vertices[line.v1]);
            let (along_2, across_2) = cut.axis.split(data.vertices[line.v2]);
            across_1 == cut.fixed
                && across_2 == cut.fixed
                && along_1.min(along_2) <= cut.open_lo
                && along_1.max(along_2) >= cut.open_hi
        })
        .ok_or_else(|| {
            let midpoint = cut.pt(i32::midpoint(cut.open_lo, cut.open_hi));
            CompileError::OpeningNotInAWall {
                room: room.to_owned(),
                x: midpoint.x,
                y: midpoint.y,
            }
        })?;

    let wall = data.linedefs.remove(index);
    let template = data.sidedefs[wall.front].clone();
    let (start, _) = cut.axis.split(data.vertices[wall.v1]);
    let (end, _) = cut.axis.split(data.vertices[wall.v2]);

    // Walk the wall in its original direction, skipping the opening.
    let pieces = if end > start {
        [(start, cut.open_lo), (cut.open_hi, end)]
    } else {
        [(start, cut.open_hi), (cut.open_lo, end)]
    };

    let mut reusable = Some(wall.front);
    for (from, to) in pieces {
        if from == to {
            continue;
        }
        let v1 = vertex_index(&mut data.vertices, cut.pt(from));
        let v2 = vertex_index(&mut data.vertices, cut.pt(to));
        let front = if let Some(existing) = reusable.take() {
            existing
        } else {
            data.sidedefs.push(template.clone());
            data.sidedefs.len() - 1
        };
        data.linedefs.push(LinedefOut {
            v1,
            v2,
            front,
            back: None,
            blocking: true,
            special: 0,
            tag: 0,
            lower_unpegged: wall.lower_unpegged,
            upper_unpegged: wall.upper_unpegged,
            secret: false,
        });
    }
    Ok(())
}

/// Emits a two-sided line along `cut`'s `fixed` coordinate, front bound to
/// `sector_a`, back to `sector_b`. Returns the new linedef's index.
///
/// [`emit_gap_sector`] calls this twice per gap sector — once for the near
/// threshold (the real room named by `sector_a`, at that room's own wall
/// coordinate) and once for the far threshold (with the two sector
/// parameters and the direction both reversed) — reusing the exact same
/// orientation rule rather than duplicating it.
///
/// `a_forward` (see [`FacingSpan::a_forward`]) picks the `v1`-to-`v2`
/// direction so `sector_a`'s front sidedef lands on the geometric right
/// regardless of which side of the wall it sits on, so front/back here name
/// the correct bordering sector even after later stages give differing floor
/// or ceiling heights, or write upper/lower textures onto these sidedefs
/// (e.g. door tracks).
pub(crate) fn emit_opening(
    data: &mut MapData,
    cut: &Cut,
    sector_a: usize,
    sector_b: usize,
    a_forward: bool,
) -> usize {
    let (p1, p2) = if a_forward {
        (cut.pt(cut.open_lo), cut.pt(cut.open_hi))
    } else {
        (cut.pt(cut.open_hi), cut.pt(cut.open_lo))
    };
    let v1 = vertex_index(&mut data.vertices, p1);
    let v2 = vertex_index(&mut data.vertices, p2);
    let front = data.sidedefs.len();
    data.sidedefs.push(SidedefOut {
        sector: sector_a,
        upper: String::new(),
        middle: String::new(),
        lower: String::new(),
        x_offset: 0,
    });
    let back = data.sidedefs.len();
    data.sidedefs.push(SidedefOut {
        sector: sector_b,
        upper: String::new(),
        middle: String::new(),
        lower: String::new(),
        x_offset: 0,
    });
    data.linedefs.push(LinedefOut {
        v1,
        v2,
        front,
        back: Some(back),
        blocking: false,
        special: 0,
        tag: 0,
        lower_unpegged: false,
        upper_unpegged: false,
        secret: false,
    });
    data.linedefs.len() - 1
}

/// Everything a pass needs to know about one portal's placement, resolved and
/// validated once.
///
/// Both `cut_portals` and `doors::emit_doors` read from this rather than each
/// re-deriving the opening's ends: computed twice, the two were free to
/// diverge, and a door's own sector must sit at exactly the span its flanking
/// walls were cut for.
pub(crate) struct PortalGeometry {
    /// Index of room `a` in `ir.rooms`, which is also its sector index.
    pub(crate) ia: usize,
    /// Index of room `b` in `ir.rooms`, which is also its sector index.
    pub(crate) ib: usize,
    /// The facing wall pair the portal sits in.
    pub(crate) span: FacingSpan,
    /// The low end of the opening.
    pub(crate) open_lo: i32,
    /// The high end of the opening.
    pub(crate) open_hi: i32,
}

/// Resolves one portal against the rooms' real facing walls.
///
/// # Errors
/// Returns [`CompileError::PortalOnDiagonalWall`] when `portal.at` sits on a
/// diagonal edge of either room — a real wall, just not one
/// [`crate::geom::wall_edges`] (and so [`facing_spans`]) considers, since v1
/// cannot cut a portal into one; checked before the two errors below so a
/// diagonal wall is never misreported as "no wall" or "off the wall" when it
/// demonstrably is a wall. Returns [`CompileError::NotAdjacent`] when the
/// rooms share no facing wall at all, [`CompileError::PortalOffWall`] when
/// the midpoint lies on none of the facing walls they do share, and
/// [`CompileError::PortalTooWide`] when the opening would run past the ends
/// of that wall.
///
/// # Panics
/// Panics if the portal names a room absent from `ir.rooms` — unreachable,
/// since [`Ir::from_json`] rejects that.
pub(crate) fn resolve_portal(ir: &Ir, portal: &Portal) -> Result<PortalGeometry, CompileError> {
    let ia = ir
        .rooms
        .iter()
        .position(|r| r.id == portal.a)
        .expect("validated in Ir::from_json");
    let ib = ir
        .rooms
        .iter()
        .position(|r| r.id == portal.b)
        .expect("validated in Ir::from_json");

    let spans = facing_spans(&ir.rooms[ia].footprint, &ir.rooms[ib].footprint);
    let matched = find_facing_span(&spans, portal.at);

    let Some(span) = matched else {
        // Before reporting the less specific errors below, check whether
        // the requested opening actually sits on a diagonal wall of either
        // room: that is a real wall v1 simply cannot cut a portal into (see
        // `crate::geom::wall_edges`'s doc comment), which deserves an
        // honest, specific message rather than `NotAdjacent`'s "the rooms
        // face no wall of each other" for a wall that is demonstrably there,
        // or `PortalOffWall`'s "not on a facing wall" for a point that is
        // exactly on *a* wall, just not one this pass considers.
        if on_diagonal_wall(&ir.rooms[ia].footprint, portal.at)
            || on_diagonal_wall(&ir.rooms[ib].footprint, portal.at)
        {
            return Err(CompileError::PortalOnDiagonalWall {
                a: portal.a.clone(),
                b: portal.b.clone(),
                x: portal.at.x,
                y: portal.at.y,
            });
        }
        return Err(if spans.is_empty() {
            CompileError::NotAdjacent {
                a: portal.a.clone(),
                b: portal.b.clone(),
            }
        } else {
            CompileError::PortalOffWall {
                a: portal.a.clone(),
                b: portal.b.clone(),
                x: portal.at.x,
                y: portal.at.y,
            }
        });
    };

    // `width` is positive and even, so the halves are exact (Ir::from_json).
    let half = portal.width / 2;
    let (on_axis, _) = span.axis.split(portal.at);
    let (open_lo, open_hi) = (on_axis - half, on_axis + half);
    if open_lo < span.lo || open_hi > span.hi {
        return Err(CompileError::PortalTooWide {
            a: portal.a.clone(),
            b: portal.b.clone(),
            width: portal.width,
            available: span.hi - span.lo,
        });
    }

    Ok(PortalGeometry {
        ia,
        ib,
        span,
        open_lo,
        open_hi,
    })
}

/// Everything about one segment of a gap-filling chain: an already-pushed
/// sector, its own near/far threshold linedef indices, and its two jamb
/// linedef indices. See [`emit_segment`].
pub(crate) struct Segment {
    /// Index of the segment's own sector.
    pub(crate) sector: usize,
    /// Index of the threshold linedef bordering `sector_near`.
    pub(crate) near_line: usize,
    /// Index of the threshold linedef bordering `sector_far`.
    pub(crate) far_line: usize,
    /// Indices of the two one-sided jamb linedefs closing the segment's long
    /// sides.
    pub(crate) jamb_lines: [usize; 2],
}

/// Emits the two one-sided jambs closing one rectangular segment's long
/// sides — the sides parallel to the direction of travel through the
/// gap — front bound to `sector` with solid rock behind, over the range
/// `near_pos`..`far_pos` along `axis`. Returns their two linedef indices.
///
/// The same pattern `exits::emit_walkover_exit` uses for its alcove's side
/// walls: a segment is a passage with no far wall of its own (its far side
/// is a threshold instead), so only its two long sides are solid.
/// Deliberately separated from threshold emission (unlike [`emit_segment`],
/// which bundles both): [`crate::compile::doors::emit_doors`] needs a
/// segment's jambs and its *outer* threshold built independently when that
/// segment's *inner* threshold is actually another segment's own boundary,
/// already emitted by that other segment's own call — see that function's
/// module documentation for the full chain construction.
#[expect(
    clippy::too_many_arguments,
    reason = "each parameter names an independent piece of the segment's geometry; bundling them \
              would just move the same count into a throwaway struct"
)]
pub(crate) fn emit_jambs(
    data: &mut MapData,
    axis: Axis,
    open_lo: i32,
    open_hi: i32,
    a_forward: bool,
    near_pos: i32,
    far_pos: i32,
    sector: usize,
    jamb_tex: &str,
) -> [usize; 2] {
    let near_cut = Cut {
        axis,
        fixed: near_pos,
        open_lo,
        open_hi,
    };
    let far_cut = Cut {
        axis,
        fixed: far_pos,
        open_lo,
        open_hi,
    };
    let (near_start, near_end) = if a_forward {
        (open_hi, open_lo)
    } else {
        (open_lo, open_hi)
    };
    let jamb_end = emit_side_wall(
        data,
        near_cut.pt(near_end),
        far_cut.pt(near_end),
        sector,
        jamb_tex,
    );
    let jamb_start = emit_side_wall(
        data,
        far_cut.pt(near_start),
        near_cut.pt(near_start),
        sector,
        jamb_tex,
    );
    [jamb_end, jamb_start]
}

/// Emits one rectangular segment of a gap-filling chain: a near threshold
/// (`sector_near` <-> `this_sector`, at `near_pos`), a far threshold
/// (`this_sector` <-> `sector_far`, at `far_pos`), and its two one-sided
/// jambs ([`emit_jambs`]).
///
/// `this_sector` must already be pushed onto `data.sectors` — this function
/// only emits the segment's *boundary* geometry, not the sector record
/// itself. `near_pos` and `far_pos` need not be
/// [`FacingSpan::near`]/[`FacingSpan::far`] themselves — only the segment's
/// own two boundary coordinates. `sector_near`/`sector_far` are whichever two
/// sectors border this segment on each side (a real room, or — for
/// [`emit_gap_sector`]'s single-segment case — the two rooms directly).
///
/// Only safe to call once per *boundary*: if `this_sector` is one segment of
/// a longer chain whose neighbor on one side is *another* compiler-generated
/// segment rather than a real room, that shared boundary must be built by
/// exactly one of the two segments, never both, or the same physical wall is
/// emitted twice as two coincident, overlapping linedefs. This is exactly
/// why [`crate::compile::doors::emit_doors`] calls this for the door segment
/// only — the door's own two faces are never shared with a third segment —
/// and builds an alcove's own outer threshold and jambs directly via
/// [`emit_opening`]/[`emit_jambs`] instead, leaving the alcove's inner
/// threshold to the door's own call.
///
/// Both threshold lines put `sector_near`/`sector_far` on the front and
/// `this_sector` on the back — `a_forward` for the near one, its opposite for
/// the far one, mirroring [`emit_opening`]'s own orientation rule — which is
/// what lets a door's threshold carry a special that only triggers from a
/// line's front side (`P_UseSpecialLine`), regardless of whether the
/// neighboring segment is a real room or another compiler-generated sector.
#[expect(
    clippy::too_many_arguments,
    reason = "each parameter names an independent piece of the segment's geometry; bundling them \
              would just move the same count into a throwaway struct"
)]
pub(crate) fn emit_segment(
    data: &mut MapData,
    axis: Axis,
    open_lo: i32,
    open_hi: i32,
    a_forward: bool,
    near_pos: i32,
    far_pos: i32,
    sector_near: usize,
    this_sector: usize,
    sector_far: usize,
    jamb_tex: &str,
) -> Segment {
    let near_cut = Cut {
        axis,
        fixed: near_pos,
        open_lo,
        open_hi,
    };
    let far_cut = Cut {
        axis,
        fixed: far_pos,
        open_lo,
        open_hi,
    };

    let near_line = emit_opening(data, &near_cut, sector_near, this_sector, a_forward);
    let far_line = emit_opening(data, &far_cut, sector_far, this_sector, !a_forward);
    let jamb_lines = emit_jambs(
        data,
        axis,
        open_lo,
        open_hi,
        a_forward,
        near_pos,
        far_pos,
        this_sector,
        jamb_tex,
    );

    Segment {
        sector: this_sector,
        near_line,
        far_line,
        jamb_lines,
    }
}

/// Emits a sector spanning the *entire* void between `span.near` (room
/// `sector_a`'s own wall) and `span.far` (room `sector_b`'s own wall) — the
/// single-segment case of [`emit_segment`], with `sector_a`/`sector_b` as its
/// only two neighbors. Returns the new sector's index.
///
/// `cut_portals` uses this for a [`PortalKind::Plain`] passage, which is
/// always a single sector spanning the whole gap, open and carrying no
/// special — so the caller has no further per-boundary bookkeeping to do
/// with the returned index. `doors::emit_doors` does not use this at all: a
/// door's own gap may be split into up to three segments (an optional near
/// alcove, the door, an optional far alcove — see
/// [`crate::ir::Portal::door_thickness`]), each needing its own properties
/// filled in afterward, so it calls [`emit_segment`] directly, once per
/// component, and reads the boundary/jamb indices back out of each
/// [`Segment`] instead of this wrapper.
#[expect(
    clippy::too_many_arguments,
    reason = "each parameter names an independent piece of the gap sector's geometry or its \
              caller-supplied properties; bundling them would just move the same count into a \
              throwaway struct"
)]
pub(crate) fn emit_gap_sector(
    data: &mut MapData,
    span: &FacingSpan,
    open_lo: i32,
    open_hi: i32,
    sector_a: usize,
    sector_b: usize,
    sector_out: SectorOut,
    jamb_tex: &str,
) -> Segment {
    let sector = data.sectors.len();
    data.sectors.push(sector_out);
    emit_segment(
        data,
        span.axis,
        open_lo,
        open_hi,
        span.a_forward,
        span.near,
        span.far,
        sector_a,
        sector,
        sector_b,
        jamb_tex,
    )
}

/// Sets `ML_SECRET` on `lines` when `crosses_boundary` — that is, when the
/// portal these lines belong to joins a secret room to an ordinary one.
///
/// The flag is purely automap-side (`am_map.c`, `AM_drawWalls`: a two-sided
/// line carrying it is drawn in `WALLCOLORS` instead of falling through to
/// the floor/ceiling-difference colors that reveal a room beyond), so this
/// changes nothing about movement or rendering in the world — it only stops
/// the automap from advertising the way in.
///
/// Applied to *both* thresholds of the chain rather than only the outer one:
/// concealing the opening from one side alone would still betray the room to
/// a player who has mapped the passage between them.
///
/// Deliberately keyed on the secrecy *difference* rather than on either
/// room's own flag, so a portal joining two secret rooms — one secret area
/// subdivided — is left unmarked, exactly as a portal joining two ordinary
/// rooms is. There is nothing to conceal at such a boundary.
pub(crate) fn mark_secret_thresholds(
    data: &mut MapData,
    crosses_boundary: bool,
    lines: impl IntoIterator<Item = usize>,
) {
    if !crosses_boundary {
        return;
    }
    for line in lines {
        data.linedefs[line].secret = true;
    }
}

/// Emits a one-sided wall from `p1` to `p2`, front bound to `sector`, with
/// solid rock behind. Returns the new linedef's index.
///
/// The construction [`emit_gap_sector`]'s two jambs, [`crate::compile::exits`]'s
/// walkover alcove, and a switch exit's own line (which needs the returned
/// index to attach its special and tag afterward) all need this.
pub(crate) fn emit_side_wall(
    data: &mut MapData,
    p1: Pt,
    p2: Pt,
    sector: usize,
    texture: &str,
) -> usize {
    let v1 = vertex_index(&mut data.vertices, p1);
    let v2 = vertex_index(&mut data.vertices, p2);
    let front = data.sidedefs.len();
    data.sidedefs.push(SidedefOut {
        sector,
        upper: String::new(),
        middle: texture.to_owned(),
        lower: String::new(),
        x_offset: 0,
    });
    data.linedefs.push(LinedefOut {
        v1,
        v2,
        front,
        back: None,
        blocking: true,
        special: 0,
        tag: 0,
        lower_unpegged: false,
        upper_unpegged: false,
        secret: false,
    });
    data.linedefs.len() - 1
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use crate::compile::portals::cut_portals;
    use crate::compile::sectors::emit_sectors;
    use crate::compile::{CompileError, MapData};
    use crate::geom::{Pt, contains};
    use crate::ir::Ir;
    use crate::tables::Tables;

    /// Room `a` is the fixed 256-unit square at the origin; room `b` is a
    /// same-size square whose west wall sits at `b_near`, which must be at
    /// least `256 + Ir::MIN_PORTAL_GAP` for the two to face each other with a
    /// legal gap. `at` is room `a`'s own reference point, per
    /// [`crate::ir::Portal::at`]'s doc comment — unaffected by `b_near`.
    fn ir_with_portal(width: i32, at: (i32, i32), b_near: i32) -> String {
        format!(
            r#"{{ "seed":1, "grid":64, "theme":"tech_base",
              "rooms":[
                {{ "id":"a", "footprint":[[0,0],[0,256],[256,256],[256,0]],
                   "floor":0, "ceiling":128, "light":160,
                   "floor_tex":"F", "ceil_tex":"C", "wall_tex":"W" }},
                {{ "id":"b", "footprint":[[{b_near},0],[{b_near},256],[{},256],[{},0]],
                   "floor":0, "ceiling":128, "light":160,
                   "floor_tex":"F", "ceil_tex":"C", "wall_tex":"W" }}
              ],
              "portals":[{{ "a":"a", "b":"b", "kind":"plain", "width":{width}, "at":[{},{}] }}] }}"#,
            b_near + 256,
            b_near + 256,
            at.0,
            at.1
        )
    }

    #[test]
    fn a_portal_becomes_a_passage_with_two_thresholds_and_a_new_sector() {
        let ir = Ir::from_json(&ir_with_portal(128, (256, 128), 320)).expect("ir");
        let tables = Tables::load().expect("tables");
        let mut data = emit_sectors(&ir).expect("sectors");
        cut_portals(&ir, &tables, &mut data).expect("portals cut");

        assert_eq!(data.sectors.len(), 3, "room a, room b, and the passage");
        let two_sided: Vec<_> = data.linedefs.iter().filter(|l| l.back.is_some()).collect();
        assert_eq!(two_sided.len(), 2, "a near threshold and a far threshold");

        let passage = 2;
        let mut touches_passage = 0;
        for l in &two_sided {
            let front_sector = data.sidedefs[l.front].sector;
            let back_sector = data.sidedefs[l.back.expect("back")].sector;
            assert_ne!(
                front_sector, back_sector,
                "front and back name different sectors"
            );
            assert!(!l.blocking, "a threshold does not block movement");
            if front_sector == passage || back_sector == passage {
                touches_passage += 1;
            }
        }
        assert_eq!(
            touches_passage, 2,
            "both thresholds border the passage sector"
        );
    }

    #[test]
    fn the_passage_sectors_jambs_are_one_sided_with_solid_rock_behind() {
        let (_, data) = assert_well_formed(&ir_with_portal(128, (256, 128), 320));
        let passage = 2;
        let jambs: Vec<_> = data
            .linedefs
            .iter()
            .filter(|l| l.back.is_none() && data.sidedefs[l.front].sector == passage)
            .collect();
        assert_eq!(
            jambs.len(),
            2,
            "the passage has exactly two one-sided jambs"
        );
        assert!(
            jambs.iter().all(|l| l.blocking),
            "a jamb is solid, like any other one-sided wall"
        );
        assert!(
            jambs
                .iter()
                .all(|l| !data.sidedefs[l.front].middle.is_empty()),
            "a jamb carries a real wall texture rather than rendering as a hole"
        );
    }

    #[test]
    fn the_passage_sector_takes_room_as_own_textures_light_and_floor_ceiling() {
        // Room a and room b differ in every texture/light field, and — load
        // bearing for pinning `max`/`min` rather than merely restating them —
        // in floor and ceiling too: room a is [0,128], room b is [16,112], so
        // `max(0,16)=16` and `min(128,112)=112` are each distinguishable from
        // either room's own value alone (a fixture with matching floors/
        // ceilings on both rooms cannot tell `min` and `max` apart at all,
        // since both formulas agree there).
        let json = r#"{ "seed":1, "grid":64, "theme":"tech_base",
          "rooms":[
            { "id":"a", "footprint":[[0,0],[0,256],[256,256],[256,0]],
               "floor":0, "ceiling":128, "light":160,
               "floor_tex":"FA", "ceil_tex":"CA", "wall_tex":"WA" },
            { "id":"b", "footprint":[[320,0],[320,256],[576,256],[576,0]],
               "floor":16, "ceiling":112, "light":40,
               "floor_tex":"FB", "ceil_tex":"CB", "wall_tex":"WB" }
          ],
          "portals":[{ "a":"a", "b":"b", "kind":"plain", "width":128, "at":[256,128] }] }"#;
        let (ir, data) = assert_well_formed(json);
        let passage = &data.sectors[2];
        assert_eq!(
            passage.floor,
            ir.rooms[0].floor.max(ir.rooms[1].floor),
            "the passage floor never sits below either room's own floor"
        );
        assert_eq!(
            passage.ceiling,
            ir.rooms[0].ceiling.min(ir.rooms[1].ceiling),
            "the passage ceiling never rises above either room's own ceiling"
        );
        assert_eq!(
            passage.light, ir.rooms[0].light,
            "passage light matches room a's"
        );
        assert_eq!(
            passage.floor_tex, ir.rooms[0].floor_tex,
            "passage floor texture matches room a's"
        );
        assert_eq!(
            passage.ceil_tex, ir.rooms[0].ceil_tex,
            "passage ceiling texture matches room a's"
        );

        let jamb = data
            .linedefs
            .iter()
            .find(|l| l.back.is_none() && data.sidedefs[l.front].sector == 2)
            .expect("a jamb exists");
        assert_eq!(
            data.sidedefs[jamb.front].middle, ir.rooms[0].wall_tex,
            "a jamb carries room a's own wall texture, not room b's"
        );
    }

    #[test]
    fn cutting_leaves_each_room_watertight() {
        let ir = Ir::from_json(&ir_with_portal(128, (256, 128), 320)).expect("ir");
        let tables = Tables::load().expect("tables");
        let mut data = emit_sectors(&ir).expect("sectors");
        let before = data.linedefs.len();
        cut_portals(&ir, &tables, &mut data).expect("portals cut");
        // Each side's wall splits into two flanking one-sided pieces (4
        // total); the passage sector between them adds two thresholds and
        // two jambs (4 more): 8 - 2 + 4 + 4 = 14.
        assert_eq!(
            data.linedefs.len(),
            before - 2 + 8,
            "wall split and passage sector accounted for"
        );
    }

    #[test]
    fn rejects_a_portal_between_rooms_that_share_no_wall() {
        // Room b sits far away in both X and Y — a genuine gap in X exists,
        // but the two rooms' Y ranges never overlap, so no wall of either
        // faces a wall of the other at all.
        let ir_json = r#"{ "seed":1, "grid":64, "theme":"tech_base",
          "rooms":[
            { "id":"a", "footprint":[[0,0],[0,256],[256,256],[256,0]],
               "floor":0, "ceiling":128, "light":160,
               "floor_tex":"F", "ceil_tex":"C", "wall_tex":"W" },
            { "id":"b", "footprint":[[320,512],[320,768],[576,768],[576,512]],
               "floor":0, "ceiling":128, "light":160,
               "floor_tex":"F", "ceil_tex":"C", "wall_tex":"W" }
          ],
          "portals":[{ "a":"a", "b":"b", "kind":"plain", "width":128, "at":[256,128] }] }"#;
        let ir = Ir::from_json(ir_json).expect("ir");
        let tables = Tables::load().expect("tables");
        let mut data = emit_sectors(&ir).expect("sectors");
        assert!(matches!(
            cut_portals(&ir, &tables, &mut data),
            Err(CompileError::NotAdjacent { .. })
        ));
    }

    #[test]
    fn rejects_an_opening_wider_than_the_facing_wall() {
        let ir = Ir::from_json(&ir_with_portal(512, (256, 128), 320)).expect("ir");
        let tables = Tables::load().expect("tables");
        let mut data = emit_sectors(&ir).expect("sectors");
        assert!(matches!(
            cut_portals(&ir, &tables, &mut data),
            Err(CompileError::PortalTooWide { .. })
        ));
    }

    #[test]
    fn rejects_an_opening_that_is_not_on_the_facing_wall() {
        let ir = Ir::from_json(&ir_with_portal(128, (128, 128), 320)).expect("ir");
        let tables = Tables::load().expect("tables");
        let mut data = emit_sectors(&ir).expect("sectors");
        assert!(matches!(
            cut_portals(&ir, &tables, &mut data),
            Err(CompileError::PortalOffWall { .. })
        ));
    }

    /// The point used to decide which side of `p -> q` is "the right":
    /// offset one unit from the segment's midpoint, in the direction
    /// obtained by rotating `p -> q` -90 degrees (reduced to a one-unit step
    /// via `.signum()`, since every segment under test is axis-aligned).
    fn probe(p: Pt, q: Pt) -> Pt {
        let (dx, dy) = (q.x - p.x, q.y - p.y);
        Pt {
            x: i32::midpoint(p.x, q.x) + dy.signum(),
            y: i32::midpoint(p.y, q.y) - dx.signum(),
        }
    }

    /// Whether room `room_idx`'s original footprint has its interior on the
    /// right of travel from `p` to `q`.
    ///
    /// This is computed independently of `cut_portals`'s own `Cut`/`a_forward`
    /// bookkeeping: probe one unit off the segment's midpoint and ask
    /// `geom::contains` whether that point lands inside the room's
    /// footprint. A test that instead re-derived `a_forward` would just
    /// restate the implementation and could not catch a regression in it.
    fn interior_is_on_the_right(ir: &Ir, room_idx: usize, p: Pt, q: Pt) -> bool {
        contains(&ir.rooms[room_idx].footprint, probe(p, q))
    }

    /// The axis-aligned bounding rectangle of every vertex touching
    /// `sector`'s boundary, as `(x_lo, x_hi, y_lo, y_hi)`.
    ///
    /// Valid for any compiler-generated sector whose own shape is a plain
    /// axis-aligned rectangle — every gap sector [`emit_gap_sector`] builds,
    /// for a plain portal's passage or a door. Recovering the rectangle
    /// straight from the emitted geometry, rather than from a
    /// fixture-specific hand computation, is what lets
    /// [`sector_interior_is_on_the_right`] run against every fixture in this
    /// suite uniformly — a gap sector has no IR footprint for
    /// [`interior_is_on_the_right`] to check against.
    fn gap_sector_bbox(data: &MapData, sector: usize) -> (i32, i32, i32, i32) {
        let verts: Vec<Pt> = data
            .linedefs
            .iter()
            .filter(|l| {
                data.sidedefs[l.front].sector == sector
                    || l.back.is_some_and(|b| data.sidedefs[b].sector == sector)
            })
            .flat_map(|l| [data.vertices[l.v1], data.vertices[l.v2]])
            .collect();
        (
            verts
                .iter()
                .map(|v| v.x)
                .min()
                .expect("sector has geometry"),
            verts
                .iter()
                .map(|v| v.x)
                .max()
                .expect("sector has geometry"),
            verts
                .iter()
                .map(|v| v.y)
                .min()
                .expect("sector has geometry"),
            verts
                .iter()
                .map(|v| v.y)
                .max()
                .expect("sector has geometry"),
        )
    }

    /// Whether `sector`'s interior is on the right of travel from `p` to
    /// `q`: [`interior_is_on_the_right`] for a real room, or a bounding-box
    /// containment test via [`gap_sector_bbox`] for a compiler-generated gap
    /// sector (any sector index at or past `ir.rooms.len()`).
    fn sector_interior_is_on_the_right(
        ir: &Ir,
        data: &MapData,
        sector: usize,
        p: Pt,
        q: Pt,
    ) -> bool {
        if sector < ir.rooms.len() {
            interior_is_on_the_right(ir, sector, p, q)
        } else {
            let (x_lo, x_hi, y_lo, y_hi) = gap_sector_bbox(data, sector);
            let pt = probe(p, q);
            pt.x >= x_lo && pt.x <= x_hi && pt.y >= y_lo && pt.y <= y_hi
        }
    }

    /// Asserts that every linedef in `data` — the untouched walls, the
    /// flanking one-sided pieces, and the two-sided openings — has its front
    /// sidedef naming the sector whose interior is on the right of
    /// `v1`-to-`v2` travel, and (for a two-sided line) its back sidedef
    /// naming the sector on the left. This is the structural invariant the
    /// whole pass exists to guarantee: a linedef's declared sides must match
    /// its actual geometry, or the engine (and crustywad's own BSP/GL-node
    /// builder) attributes the wrong sector to the wrong side.
    ///
    /// Called from `assert_well_formed` itself, alongside
    /// `assert_sector_boundaries_are_closed`, so every fixture that compiles
    /// through it — including a portal's own compiler-generated passage
    /// sector, not just the rooms named in the IR — is checked against this
    /// invariant too.
    fn assert_sidedefs_face_their_sectors(ir: &Ir, data: &MapData) {
        for l in &data.linedefs {
            let (p, q) = (data.vertices[l.v1], data.vertices[l.v2]);
            let front_sector = data.sidedefs[l.front].sector;
            assert!(
                sector_interior_is_on_the_right(ir, data, front_sector, p, q),
                "front sidedef of line {p:?} -> {q:?} names sector {front_sector}, \
                 but that sector's interior is not on the right of travel"
            );
            if let Some(back) = l.back {
                let back_sector = data.sidedefs[back].sector;
                assert!(
                    sector_interior_is_on_the_right(ir, data, back_sector, q, p),
                    "back sidedef of line {p:?} -> {q:?} names sector {back_sector}, \
                     but that sector's interior is not on the left of travel"
                );
            }
        }
    }

    /// Asserts every sector's boundary is a closed loop.
    ///
    /// For each sector, the lines that bound it are walked as directed edges
    /// — `v1 -> v2` where the sector is on the front, `v2 -> v1` where it is
    /// on the back — and every vertex must be entered exactly as many times
    /// as it is left. A sector whose boundary has a gap, a duplicated
    /// segment, or a wall left standing across an opening fails this, whatever
    /// its shape.
    ///
    /// This is deliberately shape-agnostic where the fixed-count assertions
    /// elsewhere in this suite are not. Counting linedefs only pins the one
    /// footprint the count was derived from; balance holds for every closed
    /// polygon, which is what let the same assertion catch the offset,
    /// L-shaped, and two-portal cases below.
    fn assert_sector_boundaries_are_closed(data: &MapData) {
        let mut balance: HashMap<(usize, Pt), i32> = HashMap::new();
        for line in &data.linedefs {
            let (p, q) = (data.vertices[line.v1], data.vertices[line.v2]);
            *balance
                .entry((data.sidedefs[line.front].sector, p))
                .or_default() += 1;
            *balance
                .entry((data.sidedefs[line.front].sector, q))
                .or_default() -= 1;
            if let Some(back) = line.back {
                let sector = data.sidedefs[back].sector;
                *balance.entry((sector, q)).or_default() += 1;
                *balance.entry((sector, p)).or_default() -= 1;
            }
        }
        for ((sector, point), net) in balance {
            assert_eq!(
                net, 0,
                "sector {sector}'s boundary is not a closed loop: vertex {point:?} is left \
                 {net} more times than it is entered"
            );
        }
    }

    /// Compiles a fixture through `emit_sectors` and `cut_portals`, asserting
    /// that every sector's boundary closes and every sidedef faces its real
    /// sector, and hands back the result.
    fn assert_well_formed(ir_json: &str) -> (Ir, MapData) {
        let ir = Ir::from_json(ir_json).expect("ir");
        let tables = Tables::load().expect("tables");
        let mut data = emit_sectors(&ir).expect("sectors");
        cut_portals(&ir, &tables, &mut data).expect("portals cut");
        assert_sector_boundaries_are_closed(&data);
        assert_sidedefs_face_their_sectors(&ir, &data);
        (ir, data)
    }

    /// Every one-sided linedef lying on the wall line `fixed`, as
    /// `(along_lo, along_hi)` pairs, for the room whose sector is `sector`.
    fn wall_pieces(data: &MapData, sector: usize, fixed: i32) -> Vec<(i32, i32)> {
        let mut pieces: Vec<(i32, i32)> = data
            .linedefs
            .iter()
            .filter(|l| l.back.is_none() && data.sidedefs[l.front].sector == sector)
            .map(|l| (data.vertices[l.v1], data.vertices[l.v2]))
            .filter(|(p, q)| p.x == fixed && q.x == fixed)
            .map(|(p, q)| (p.y.min(q.y), p.y.max(q.y)))
            .collect();
        pieces.sort_unstable();
        pieces
    }

    /// Two rooms whose facing wall is *not* the full side of either: room
    /// `a` runs y 0..256 against x = 256, room `b` runs y 128..384 against
    /// x = 320 (a legal 64-unit gap east of room `a`), so they face each
    /// other only over y 128..256.
    const OFFSET_ROOMS: &str = r#"{ "seed":1, "grid":64, "theme":"tech_base",
      "rooms":[
        { "id":"a", "footprint":[[0,0],[0,256],[256,256],[256,0]],
           "floor":0, "ceiling":128, "light":160,
           "floor_tex":"F", "ceil_tex":"C", "wall_tex":"WA" },
        { "id":"b", "footprint":[[320,128],[320,384],[576,384],[576,128]],
           "floor":0, "ceiling":128, "light":160,
           "floor_tex":"F", "ceil_tex":"C", "wall_tex":"WB" }
      ],
      "portals":[{ "a":"a", "b":"b", "kind":"plain", "width":64, "at":[256,192] }] }"#;

    /// An L-shaped room `a`: its bounding box reaches x = 256 over the whole
    /// height, but its actual wall there covers only y 128..256. Below that
    /// the room stops at x = 128.
    const L_SHAPED_A: &str = r#"{ "id":"a",
        "footprint":[[0,0],[0,256],[256,256],[256,128],[128,128],[128,0]],
        "floor":0, "ceiling":128, "light":160,
        "floor_tex":"F", "ceil_tex":"C", "wall_tex":"W" }"#;

    #[test]
    fn an_offset_room_pair_faces_only_the_overlapping_stretch_of_wall() {
        let (_, data) = assert_well_formed(OFFSET_ROOMS);

        // Room a's wall runs its own full height and is split at the
        // opening; the same for room b's, over its own different extent.
        // Splitting in place is what preserves each wall's own ends — the
        // earlier drop-and-recreate matched only a wall whose endpoints were
        // exactly the shared span, so with these rooms neither wall was
        // touched and the opening was emitted on top of two intact walls.
        assert_eq!(
            wall_pieces(&data, 0, 256),
            vec![(0, 160), (224, 256)],
            "room a's wall keeps its own ends (y 0 and y 256) around the opening"
        );
        assert_eq!(
            wall_pieces(&data, 1, 320),
            vec![(128, 160), (224, 384)],
            "room b's wall keeps its own ends (y 128 and y 384) around the opening"
        );
        assert_eq!(
            data.linedefs.iter().filter(|l| l.back.is_some()).count(),
            2,
            "exactly one near threshold and one far threshold"
        );
    }

    #[test]
    fn each_room_keeps_its_own_wall_texture_through_a_split() {
        let (_, data) = assert_well_formed(OFFSET_ROOMS);
        for (sector, expected) in [(0, "WA"), (1, "WB")] {
            for line in data
                .linedefs
                .iter()
                .filter(|l| l.back.is_none() && data.sidedefs[l.front].sector == sector)
            {
                assert_eq!(
                    data.sidedefs[line.front].middle, expected,
                    "sector {sector}'s walls keep its own texture"
                );
            }
        }
    }

    #[test]
    fn an_l_shaped_room_is_not_adjacent_where_it_has_no_wall() {
        // Room b sits well clear of every real wall room a has (both its
        // outer wall at x = 256, y 128..256, and its recessed inner wall at
        // x = 128, y 0..128) — nowhere close enough in Y to face either one,
        // however far east it sits in X. `facing_spans` matches real walls
        // at any distance, not a bounding box, so simply placing room b
        // "east of x = 256" the way earlier fixtures do is not by itself
        // enough to prove non-adjacency here: room a's *recessed* wall would
        // still genuinely face a room b placed at y 0..128, however far
        // east — the exact bounding-box-shaped mistake this test exists to
        // catch, just relocated by the gap model rather than eliminated.
        let ir_json = format!(
            r#"{{ "seed":1, "grid":64, "theme":"tech_base",
              "rooms":[
                {L_SHAPED_A},
                {{ "id":"b", "footprint":[[320,-192],[320,-64],[448,-64],[448,-192]],
                   "floor":0, "ceiling":128, "light":160,
                   "floor_tex":"F", "ceil_tex":"C", "wall_tex":"W" }}
              ],
              "portals":[{{ "a":"a", "b":"b", "kind":"plain", "width":64, "at":[256,64] }}] }}"#
        );
        let ir = Ir::from_json(&ir_json).expect("ir");
        let tables = Tables::load().expect("tables");
        let mut data = emit_sectors(&ir).expect("sectors");
        assert!(matches!(
            cut_portals(&ir, &tables, &mut data),
            Err(CompileError::NotAdjacent { .. })
        ));
    }

    #[test]
    fn an_l_shaped_room_opens_where_it_does_have_a_wall() {
        // The same L, with room b moved up against the stretch of wall that
        // genuinely exists (y 128..256), and out a legal 64-unit gap.
        let ir_json = format!(
            r#"{{ "seed":1, "grid":64, "theme":"tech_base",
              "rooms":[
                {L_SHAPED_A},
                {{ "id":"b", "footprint":[[320,128],[320,256],[448,256],[448,128]],
                   "floor":0, "ceiling":128, "light":160,
                   "floor_tex":"F", "ceil_tex":"C", "wall_tex":"W" }}
              ],
              "portals":[{{ "a":"a", "b":"b", "kind":"plain", "width":64, "at":[256,192] }}] }}"#
        );
        let (_, data) = assert_well_formed(&ir_json);
        assert_eq!(
            data.linedefs.iter().filter(|l| l.back.is_some()).count(),
            2,
            "the portal opened: a near threshold and a far threshold"
        );
        assert_eq!(
            wall_pieces(&data, 0, 256),
            vec![(128, 160), (224, 256)],
            "the L's short wall is split within its own extent"
        );
    }

    /// Two 256x512 rooms facing each other across a legal 64-unit gap (room
    /// `a`'s east wall at x = 256, room `b`'s west wall at x = 320), with two
    /// separate openings in that one wall pair.
    fn two_portal_ir(first: i32, first_width: i32, second: i32, second_width: i32) -> String {
        format!(
            r#"{{ "seed":1, "grid":64, "theme":"tech_base",
              "rooms":[
                {{ "id":"a", "footprint":[[0,0],[0,512],[256,512],[256,0]],
                   "floor":0, "ceiling":128, "light":160,
                   "floor_tex":"F", "ceil_tex":"C", "wall_tex":"W" }},
                {{ "id":"b", "footprint":[[320,0],[320,512],[576,512],[576,0]],
                   "floor":0, "ceiling":128, "light":160,
                   "floor_tex":"F", "ceil_tex":"C", "wall_tex":"W" }}
              ],
              "portals":[
                {{ "a":"a", "b":"b", "kind":"plain", "width":{first_width}, "at":[256,{first}] }},
                {{ "a":"a", "b":"b", "kind":"plain", "width":{second_width}, "at":[256,{second}] }}
              ] }}"#
        )
    }

    #[test]
    fn two_portals_in_one_wall_pair_both_open_without_corrupting_each_other() {
        let (_, data) = assert_well_formed(&two_portal_ir(128, 64, 384, 64));
        assert_eq!(
            data.linedefs.iter().filter(|l| l.back.is_some()).count(),
            4,
            "both portals opened, each with a near and a far threshold"
        );
        assert_eq!(
            wall_pieces(&data, 0, 256),
            vec![(0, 96), (160, 352), (416, 512)],
            "room a's wall is in three pieces, one between the two openings"
        );
        assert_eq!(
            wall_pieces(&data, 1, 320),
            vec![(0, 96), (160, 352), (416, 512)],
            "room b's wall is in three pieces, one between the two openings"
        );
    }

    #[test]
    fn rejects_two_portals_whose_openings_overlap_in_the_same_wall() {
        // y 64..192 and y 128..256 overlap over y 128..192. Cutting the
        // second would find no intact wall where the first already opened.
        let ir = Ir::from_json(&two_portal_ir(128, 128, 192, 128)).expect("ir");
        let tables = Tables::load().expect("tables");
        let mut data = emit_sectors(&ir).expect("sectors");
        assert!(matches!(
            cut_portals(&ir, &tables, &mut data),
            Err(CompileError::OverlappingPortals { .. })
        ));
    }

    #[test]
    fn two_openings_meeting_end_to_end_are_allowed() {
        // y 64..192 and y 192..320 touch at a point but share no length, so
        // the wall between them is zero-length and simply not emitted.
        let (_, data) = assert_well_formed(&two_portal_ir(128, 128, 256, 128));
        assert_eq!(
            data.linedefs.iter().filter(|l| l.back.is_some()).count(),
            4,
            "both portals opened, each with a near and a far threshold"
        );
        assert_eq!(
            wall_pieces(&data, 0, 256),
            vec![(0, 64), (320, 512)],
            "room a has no zero-length wall stub between the two openings"
        );
        assert_eq!(
            wall_pieces(&data, 1, 320),
            vec![(0, 64), (320, 512)],
            "room b has no zero-length wall stub between the two openings"
        );
    }

    #[test]
    fn sidedefs_face_their_sectors_when_room_a_is_west_of_a_vertical_wall() {
        assert_well_formed(&ir_with_portal(128, (256, 128), 320));
    }

    #[test]
    fn sidedefs_face_their_sectors_when_room_a_is_east_of_a_vertical_wall() {
        // Room a's own wall stays at x = 256; room b (to a's west) is
        // pushed a further 64 units west, to x = -64..192, for a legal gap.
        let ir_json = r#"{ "seed":1, "grid":64, "theme":"tech_base",
          "rooms":[
            { "id":"a", "footprint":[[256,0],[256,256],[512,256],[512,0]],
               "floor":0, "ceiling":128, "light":160,
               "floor_tex":"F", "ceil_tex":"C", "wall_tex":"W" },
            { "id":"b", "footprint":[[-64,0],[-64,256],[192,256],[192,0]],
               "floor":0, "ceiling":128, "light":160,
               "floor_tex":"F", "ceil_tex":"C", "wall_tex":"W" }
          ],
          "portals":[{ "a":"a", "b":"b", "kind":"plain", "width":128, "at":[256,128] }] }"#;
        assert_well_formed(ir_json);
    }

    #[test]
    fn sidedefs_face_their_sectors_when_room_a_is_south_of_a_horizontal_wall() {
        // Room a's own wall stays at y = 256; room b (to a's north) is
        // pushed a further 64 units north, to y = 320..576.
        let ir_json = r#"{ "seed":1, "grid":64, "theme":"tech_base",
          "rooms":[
            { "id":"a", "footprint":[[0,0],[0,256],[256,256],[256,0]],
               "floor":0, "ceiling":128, "light":160,
               "floor_tex":"F", "ceil_tex":"C", "wall_tex":"W" },
            { "id":"b", "footprint":[[0,320],[0,576],[256,576],[256,320]],
               "floor":0, "ceiling":128, "light":160,
               "floor_tex":"F", "ceil_tex":"C", "wall_tex":"W" }
          ],
          "portals":[{ "a":"a", "b":"b", "kind":"plain", "width":128, "at":[128,256] }] }"#;
        assert_well_formed(ir_json);
    }

    #[test]
    fn sidedefs_face_their_sectors_when_room_a_is_north_of_a_horizontal_wall() {
        // Room a's own wall stays at y = 256; room b (to a's south) is
        // pushed a further 64 units south, to y = -64..192.
        let ir_json = r#"{ "seed":1, "grid":64, "theme":"tech_base",
          "rooms":[
            { "id":"a", "footprint":[[0,256],[0,512],[256,512],[256,256]],
               "floor":0, "ceiling":128, "light":160,
               "floor_tex":"F", "ceil_tex":"C", "wall_tex":"W" },
            { "id":"b", "footprint":[[0,-64],[0,192],[256,192],[256,-64]],
               "floor":0, "ceiling":128, "light":160,
               "floor_tex":"F", "ceil_tex":"C", "wall_tex":"W" }
          ],
          "portals":[{ "a":"a", "b":"b", "kind":"plain", "width":128, "at":[128,256] }] }"#;
        assert_well_formed(ir_json);
    }

    /// An octagon: a 256-unit square chamfered by 64 units at each corner,
    /// with no portals. The spec's `architecture.room_shapes` names
    /// octagonal rooms explicitly, but before this, no fixture anywhere in
    /// this crate had a diagonal edge (see `KNOWN-GAPS.md`'s "no fixture
    /// anywhere has a 45-degree edge"). Routed through `assert_well_formed`
    /// so the sidedef-facing invariant itself runs against a genuinely
    /// diagonal-edged room, not merely the closure/count checks
    /// `sectors::tests` already covers for the same shape.
    const OCTAGON_ROOM: &str = r#"{ "seed":1, "grid":64, "theme":"tech_base",
      "rooms":[
        { "id":"a",
          "footprint":[[0,64],[0,192],[64,256],[192,256],[256,192],[256,64],[192,0],[64,0]],
          "floor":0, "ceiling":128, "light":160,
          "floor_tex":"F", "ceil_tex":"C", "wall_tex":"W" }
      ],
      "portals":[] }"#;

    #[test]
    fn an_octagonal_room_with_no_portals_is_well_formed() {
        let (_, data) = assert_well_formed(OCTAGON_ROOM);
        assert_eq!(data.sectors.len(), 1);
        assert_eq!(data.linedefs.len(), 8, "eight walls, four of them diagonal");
        assert!(
            data.linedefs.iter().all(|l| l.back.is_none()),
            "no portals means every wall, diagonal included, stays one-sided"
        );
    }

    /// Room a is a pentagon: a 256-unit square with just its NE corner
    /// chamfered by 64 units, leaving its west, south, and most of its
    /// north/east walls axis-aligned. Room b is a plain square sharing room
    /// a's *west* wall — nowhere near the chamfer — proving a portal still
    /// works normally on the surviving axis-aligned wall of a diagonally
    /// shaped room, which is the common real case the project's decision to
    /// leave diagonal-wall portals unsupported rests on.
    const CHAMFERED_ROOM_WITH_PORTAL: &str = r#"{ "seed":1, "grid":64, "theme":"tech_base",
      "rooms":[
        { "id":"a", "footprint":[[0,0],[0,256],[192,256],[256,192],[256,0]],
          "floor":0, "ceiling":128, "light":160,
          "floor_tex":"F", "ceil_tex":"C", "wall_tex":"W" },
        { "id":"b", "footprint":[[-320,0],[-320,256],[-64,256],[-64,0]],
          "floor":0, "ceiling":128, "light":160,
          "floor_tex":"F", "ceil_tex":"C", "wall_tex":"W" }
      ],
      "portals":[{ "a":"a", "b":"b", "kind":"plain", "width":128, "at":[0,128] }] }"#;

    #[test]
    fn a_portal_works_on_the_axis_aligned_wall_of_a_diagonally_shaped_room() {
        let (_, data) = assert_well_formed(CHAMFERED_ROOM_WITH_PORTAL);
        assert_eq!(
            data.linedefs.iter().filter(|l| l.back.is_some()).count(),
            2,
            "the portal opened normally, away from the chamfer, with a near and far threshold"
        );
    }

    /// Room b is the full octagon fixture (`sectors::tests::OCTAGON`,
    /// `OCTAGON_ROOM` above, and `doors::tests::OCTAGON_ROOM_B`): a
    /// 256-unit square chamfered by 64 units at *every* corner, not just
    /// one. `CHAMFERED_ROOM_WITH_PORTAL` above only ever exercises a
    /// single-corner chamfer for a plain portal; the full octagon is
    /// otherwise only proven for a door portal
    /// (`doors::tests::a_door_into_an_octagonal_room_on_its_axis_aligned_wall_works`)
    /// and for no portal at all
    /// (`an_octagonal_room_with_no_portals_is_well_formed`) — this closes
    /// that gap for the plain-portal case specifically. The door sits on
    /// the octagon's west wall (x = 64, y in 64..192), a legal 64-unit gap
    /// east of room a's own east wall (x = 0).
    const OCTAGON_ROOM_WITH_PLAIN_PORTAL: &str = r#"{ "seed":1, "grid":64, "theme":"tech_base",
      "rooms":[
        { "id":"a", "footprint":[[-256,0],[-256,256],[0,256],[0,0]],
           "floor":0, "ceiling":128, "light":160,
           "floor_tex":"F", "ceil_tex":"C", "wall_tex":"W" },
        { "id":"b",
          "footprint":[[64,64],[64,192],[128,256],[256,256],[320,192],[320,64],[256,0],[128,0]],
           "floor":0, "ceiling":128, "light":160,
           "floor_tex":"F", "ceil_tex":"C", "wall_tex":"W" }
      ],
      "portals":[{ "a":"a", "b":"b", "kind":"plain", "width":64, "at":[0,128] }] }"#;

    #[test]
    fn a_plain_portal_works_on_the_axis_aligned_wall_of_a_full_octagonal_room() {
        let (_, data) = assert_well_formed(OCTAGON_ROOM_WITH_PLAIN_PORTAL);
        assert_eq!(
            data.linedefs.iter().filter(|l| l.back.is_some()).count(),
            2,
            "the portal opened normally, away from every chamfer, with a near and far threshold"
        );
        assert_eq!(
            data.sectors.len(),
            3,
            "room a, the octagon, and the passage"
        );
    }

    /// Two right triangles splitting a 64-unit square along its own
    /// diagonal: room a is the upper-left half, room b the lower-right
    /// half. They share the *entire* diagonal (0,0)-(64,64) as a real wall
    /// — but `crate::geom::wall_edges` filters diagonal edges out of
    /// `crate::geom::facing_spans` entirely, so before this fix a portal
    /// placed here reported
    /// `NotAdjacent` ("the rooms face no wall of each other"), which is
    /// simply false: they share exactly this wall, just not one v1 can cut
    /// a portal into.
    const DIAGONAL_TWIN_TRIANGLES: &str = r#"{ "seed":1, "grid":64, "theme":"tech_base",
      "rooms":[
        { "id":"a", "footprint":[[0,0],[0,64],[64,64]],
           "floor":0, "ceiling":128, "light":160,
           "floor_tex":"F", "ceil_tex":"C", "wall_tex":"W" },
        { "id":"b", "footprint":[[0,0],[64,64],[64,0]],
           "floor":0, "ceiling":128, "light":160,
           "floor_tex":"F", "ceil_tex":"C", "wall_tex":"W" }
      ],
      "portals":[{ "a":"a", "b":"b", "kind":"plain", "width":16, "at":[32,32] }] }"#;

    #[test]
    fn a_portal_on_a_wall_two_rooms_share_only_diagonally_names_the_diagonal_wall() {
        let ir = Ir::from_json(DIAGONAL_TWIN_TRIANGLES).expect("ir");
        let tables = Tables::load().expect("tables");
        let mut data = emit_sectors(&ir).expect("sectors");
        let err = cut_portals(&ir, &tables, &mut data)
            .expect_err("a diagonal wall cannot host a portal in v1");
        match err {
            CompileError::PortalOnDiagonalWall { a, b, x, y } => {
                assert_eq!(a, "a");
                assert_eq!(b, "b");
                assert_eq!((x, y), (32, 32), "names the requested opening's location");
            }
            other => panic!(
                "expected PortalOnDiagonalWall naming the shared diagonal, got {other:?} \
                 instead — a diagonal wall must never be reported as \"no wall\" or \"off the \
                 wall\" when it demonstrably is a wall"
            ),
        }
    }

    /// The octagon fixture again, but paired with a room `b` that shares no
    /// wall with it at all — the portal's `at` sits on room `a`'s NW
    /// chamfer alone. Unlike the twin-triangle case above, where *both*
    /// rooms have a diagonal edge at the requested point, this pins down
    /// that `resolve_portal` checks each room independently
    /// (`on_diagonal_wall(a) || on_diagonal_wall(b)`) rather than only ever
    /// checking one side — a mutation that dropped either half of that `||`
    /// would still pass the twin-triangle test above, since both sides are
    /// true there, but not this one.
    const OCTAGON_DIAGONAL_UNRELATED_B: &str = r#"{ "seed":1, "grid":64, "theme":"tech_base",
      "rooms":[
        { "id":"a",
          "footprint":[[0,64],[0,192],[64,256],[192,256],[256,192],[256,64],[192,0],[64,0]],
          "floor":0, "ceiling":128, "light":160,
          "floor_tex":"F", "ceil_tex":"C", "wall_tex":"W" },
        { "id":"b", "footprint":[[1024,1024],[1024,1088],[1088,1088],[1088,1024]],
          "floor":0, "ceiling":128, "light":160,
          "floor_tex":"F", "ceil_tex":"C", "wall_tex":"W" }
      ],
      "portals":[{ "a":"a", "b":"b", "kind":"plain", "width":16, "at":[32,224] }] }"#;

    #[test]
    fn a_portal_on_room_as_own_diagonal_wall_is_flagged_even_when_room_b_is_unrelated() {
        let ir = Ir::from_json(OCTAGON_DIAGONAL_UNRELATED_B).expect("ir");
        let tables = Tables::load().expect("tables");
        let mut data = emit_sectors(&ir).expect("sectors");
        assert!(
            matches!(
                cut_portals(&ir, &tables, &mut data),
                Err(CompileError::PortalOnDiagonalWall { x: 32, y: 224, .. })
            ),
            "room a's own diagonal wall must be flagged even though room b never touches it"
        );
    }

    /// A portal into a secret room marks both of its threshold lines
    /// `ML_SECRET`, so the automap draws them as solid wall instead of
    /// revealing a room beyond.
    ///
    /// Both thresholds, not just the outer one: the flag is what conceals
    /// the opening, and concealing it from only one side would still betray
    /// the room to a player who has mapped the passage.
    #[test]
    fn a_portal_into_a_secret_room_marks_its_thresholds_secret() {
        let json = r#"{ "seed":1, "grid":64, "theme":"tech_base",
          "rooms":[
            { "id":"a", "footprint":[[0,0],[0,256],[256,256],[256,0]],
              "floor":0, "ceiling":128, "light":160,
              "floor_tex":"F", "ceil_tex":"C", "wall_tex":"W" },
            { "id":"b", "footprint":[[320,0],[320,256],[576,256],[576,0]],
              "floor":0, "ceiling":128, "light":160,
              "floor_tex":"F", "ceil_tex":"C", "wall_tex":"W", "secret":true }
          ],
          "portals":[{ "a":"a", "b":"b", "kind":"plain", "width":128, "at":[256,128] }] }"#;
        let ir = Ir::from_json(json).expect("ir");
        let tables = Tables::load().expect("tables");
        let mut data = emit_sectors(&ir).expect("sectors");
        cut_portals(&ir, &tables, &mut data).expect("portals");

        let thresholds: Vec<_> = data.linedefs.iter().filter(|l| l.back.is_some()).collect();
        assert_eq!(thresholds.len(), 2, "a plain portal emits two thresholds");
        assert!(
            thresholds.iter().all(|l| l.secret),
            "both thresholds of a portal into a secret room are ML_SECRET"
        );
        assert!(
            data.linedefs
                .iter()
                .filter(|l| l.back.is_none())
                .all(|l| !l.secret),
            "one-sided walls are never marked secret — the flag only changes how a \
             two-sided line is drawn"
        );
    }

    /// The flag is tied to the secrecy *difference* across a portal, not to
    /// the presence of a portal. A map with no secret room marks nothing.
    #[test]
    fn a_portal_between_two_ordinary_rooms_marks_nothing_secret() {
        let json = r#"{ "seed":1, "grid":64, "theme":"tech_base",
          "rooms":[
            { "id":"a", "footprint":[[0,0],[0,256],[256,256],[256,0]],
              "floor":0, "ceiling":128, "light":160,
              "floor_tex":"F", "ceil_tex":"C", "wall_tex":"W" },
            { "id":"b", "footprint":[[320,0],[320,256],[576,256],[576,0]],
              "floor":0, "ceiling":128, "light":160,
              "floor_tex":"F", "ceil_tex":"C", "wall_tex":"W" }
          ],
          "portals":[{ "a":"a", "b":"b", "kind":"plain", "width":128, "at":[256,128] }] }"#;
        let ir = Ir::from_json(json).expect("ir");
        let tables = Tables::load().expect("tables");
        let mut data = emit_sectors(&ir).expect("sectors");
        cut_portals(&ir, &tables, &mut data).expect("portals");
        assert!(data.linedefs.iter().all(|l| !l.secret));
    }

    /// Every emitted sector records the wall texture its faces use — rooms
    /// their own, and the compiler-created passage sector the one belonging
    /// to room `a`, whose jamb texture it already borrows.
    #[test]
    fn every_emitted_sector_records_a_wall_texture() {
        let json = r#"{ "seed":1, "grid":64, "theme":"tech_base",
          "rooms":[
            { "id":"a", "footprint":[[0,0],[0,256],[256,256],[256,0]],
              "floor":0, "ceiling":128, "light":160,
              "floor_tex":"F", "ceil_tex":"C", "wall_tex":"WA" },
            { "id":"b", "footprint":[[320,0],[320,256],[576,256],[576,0]],
              "floor":0, "ceiling":128, "light":160,
              "floor_tex":"F", "ceil_tex":"C", "wall_tex":"WB" }
          ],
          "portals":[{ "a":"a", "b":"b", "kind":"plain", "width":128, "at":[256,128] }] }"#;
        let ir = Ir::from_json(json).expect("ir");
        let tables = Tables::load().expect("tables");
        let mut data = emit_sectors(&ir).expect("sectors");
        cut_portals(&ir, &tables, &mut data).expect("portals");

        assert_eq!(data.sectors[0].wall_tex, "WA", "room a keeps its own");
        assert_eq!(data.sectors[1].wall_tex, "WB", "room b keeps its own");
        assert_eq!(
            data.sectors[2].wall_tex, "WA",
            "the passage sector inherits room a's, matching its jamb texture"
        );
    }
}
