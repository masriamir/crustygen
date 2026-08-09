//! Opens portals in shared walls and pairs the resulting two-sided linedefs.

use crate::compile::sectors::vertex_index;
use crate::compile::{CompileError, LinedefOut, MapData, SidedefOut};
use crate::geom::{Pt, edges};
use crate::ir::{Ir, Portal, PortalKind};

/// The axis a shared wall runs along.
///
/// `pub(crate)` so `doors` can re-derive a portal's shared-wall geometry via
/// [`resolve_portal`] to carve its own recessed sector.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Axis {
    /// The wall is vertical; X is constant.
    Vertical,
    /// The wall is horizontal; Y is constant.
    Horizontal,
}

impl Axis {
    /// Splits a point into `(along, across)` for this axis: the coordinate
    /// that varies along the wall, and the one held constant across it.
    fn split(self, p: Pt) -> (i32, i32) {
        match self {
            Self::Vertical => (p.y, p.x),
            Self::Horizontal => (p.x, p.y),
        }
    }
}

/// Opens every portal in the shared wall of its two rooms.
///
/// Every portal is resolved and cross-checked before anything is emitted, so
/// a rejected map leaves no partially-cut geometry behind. Each room's solid
/// wall is then split at the opening's two ends — the pieces outside the
/// opening survive with their own endpoints and textures intact — and a
/// single two-sided linedef carries the opening with its front and back
/// sidedefs bound to the two rooms' sectors.
///
/// # Errors
/// Returns [`CompileError::NotAdjacent`] when the rooms share no wall,
/// [`CompileError::PortalOffWall`] when the midpoint is not on a shared wall,
/// [`CompileError::PortalTooWide`] when the opening exceeds the shared span,
/// [`CompileError::OverlappingPortals`] when two openings overlap on the same
/// wall line, and [`CompileError::OpeningNotInAWall`] when no single solid
/// wall of a room spans the opening.
pub fn cut_portals(ir: &Ir, data: &mut MapData) -> Result<(), CompileError> {
    let resolved = ir
        .portals
        .iter()
        .map(|portal| Ok((portal, resolve_portal(ir, portal)?)))
        .collect::<Result<Vec<_>, CompileError>>()?;

    check_no_overlapping_openings(&resolved)?;

    for (portal, geometry) in &resolved {
        cut_one(data, portal, geometry)?;
    }
    Ok(())
}

/// Rejects two portals whose openings overlap along the same wall line.
///
/// Splitting a wall consumes the piece that spans the opening, so a second
/// opening overlapping the first would find no intact wall to split. Catching
/// it here names both portals instead of surfacing the downstream
/// [`CompileError::OpeningNotInAWall`], and mirrors the pairwise
/// `rects_overlap` guard `doors::plan_doors` already applies to door
/// recesses. Keying on `(axis, fixed)` rather than on the room pair covers
/// the collinear case too: two portals into *different* rooms can still share
/// one wall line.
fn check_no_overlapping_openings(
    resolved: &[(&Portal, PortalGeometry)],
) -> Result<(), CompileError> {
    for (i, (first, fg)) in resolved.iter().enumerate() {
        for (second, sg) in &resolved[i + 1..] {
            let collinear = fg.span.axis == sg.span.axis && fg.span.fixed == sg.span.fixed;
            let overlaps = fg.open_lo < sg.open_hi && sg.open_lo < fg.open_hi;
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
    data: &mut MapData,
    portal: &Portal,
    geometry: &PortalGeometry,
) -> Result<(), CompileError> {
    let cut = Cut {
        axis: geometry.span.axis,
        fixed: geometry.span.fixed,
        open_lo: geometry.open_lo,
        open_hi: geometry.open_hi,
    };

    // Split each room's own solid wall at the opening. Splitting rather than
    // dropping-and-recreating is what makes a portal between rooms whose
    // walls are not flush work at all: the shared span is only the part of
    // each wall the two rooms have in common, so the surviving pieces must
    // run out to each wall's *own* endpoints, not to the shared span's.
    split_wall_for_opening(data, &cut, geometry.ia, &portal.a)?;
    split_wall_for_opening(data, &cut, geometry.ib, &portal.b)?;

    // A plain portal's opening is the two-sided line straight across the
    // shared wall. A door portal instead gets a carved-out sector of its own
    // — see `doors::emit_doors`, which runs after `cut_portals` and expects
    // to find flanking walls in place but no opening line yet.
    if portal.kind == PortalKind::Plain {
        emit_opening(
            data,
            &cut,
            geometry.ia,
            geometry.ib,
            geometry.span.a_forward,
        );
    }

    Ok(())
}

/// The span geometry of one portal's cut: which axis and fixed coordinate the
/// wall runs along, and the two along-axis ends of the opening.
///
/// `pub(crate)` (including every field) so `doors` can build a second `Cut`
/// at a recessed `fixed` coordinate for a door sector's far face, reusing
/// this same struct and [`emit_opening`] rather than duplicating them.
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
fn split_wall_for_opening(
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
        });
    }
    Ok(())
}

/// Emits a two-sided line along `cut`'s `fixed` coordinate, front bound to
/// `sector_a`, back to `sector_b`. Returns the new linedef's index.
///
/// For a plain portal this is the opening itself, front on room A, back on
/// room B. `doors::emit_doors` also calls this twice per door portal — once
/// with the door sector standing in for `sector_b` at the original wall
/// coordinate, once with it standing in for `sector_a` at the recessed far
/// coordinate — reusing the exact same orientation rule rather than
/// duplicating it.
///
/// `a_forward` (see `shared_span`) picks the `v1`-to-`v2` direction so
/// `sector_a`'s front sidedef lands on the geometric right regardless of
/// which side of the wall it sits on — the same rule `emit_flanking_walls`
/// applies, so front/back here name the correct bordering sector even after
/// later stages give differing floor or ceiling heights, or write
/// upper/lower textures onto these sidedefs (e.g. door tracks).
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
    });
    let back = data.sidedefs.len();
    data.sidedefs.push(SidedefOut {
        sector: sector_b,
        upper: String::new(),
        middle: String::new(),
        lower: String::new(),
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
    });
    data.linedefs.len() - 1
}

/// One stretch of wall two rooms genuinely share: a run of real, coincident,
/// collinear boundary that both footprints have an edge along.
#[derive(Debug, Clone, Copy)]
pub(crate) struct SharedSpan {
    /// The axis the shared wall runs along.
    pub(crate) axis: Axis,
    /// The coordinate held constant across the wall.
    pub(crate) fixed: i32,
    /// The low end of the shared run.
    pub(crate) lo: i32,
    /// The high end of the shared run.
    pub(crate) hi: i32,
    /// Whether room `a`'s own boundary edge along this wall runs in the
    /// increasing-`along` direction.
    ///
    /// Footprints wind clockwise, so a room's interior is always on the right
    /// of its own edge direction. That makes this single bit the whole
    /// orientation story: emitting a line in room `a`'s edge direction puts
    /// room `a` on the front, and reversing it puts room `b` there.
    pub(crate) a_forward: bool,
}

impl SharedSpan {
    /// The sign, along the across-axis, of the direction that leads from the
    /// wall into room `b`.
    ///
    /// Room `a`'s interior lies to the right of its own edge direction.
    /// Rotating a `+along` direction vector by -90 degrees gives `+across`
    /// for a vertical wall (`along` = Y, `across` = X) but `-across` for a
    /// horizontal wall (`along` = X, `across` = Y) — the two axes are not
    /// mirror images — so the two cases are derived separately. Room `b` is
    /// always on the opposite side from room `a`.
    pub(crate) fn across_sign_toward_b(self) -> i32 {
        match self.axis {
            Axis::Vertical => {
                if self.a_forward {
                    -1
                } else {
                    1
                }
            }
            Axis::Horizontal => {
                if self.a_forward {
                    1
                } else {
                    -1
                }
            }
        }
    }
}

/// Every axis-aligned edge of a footprint, as `(axis, fixed, lo, hi,
/// forward)` where `forward` records whether the edge runs in the increasing
/// `along` direction.
///
/// Edges at 45 degrees are skipped: a diagonal wall cannot host a portal in
/// v1, since the opening's endpoints, the flanking wall pieces, and a door
/// recess would all have to land on non-integer coordinates to stay flush
/// with it. Two rooms meeting only along a diagonal therefore report as not
/// adjacent rather than silently receiving a mis-shaped opening.
fn wall_edges(poly: &[Pt]) -> impl Iterator<Item = (Axis, i32, i32, i32, bool)> + '_ {
    edges(poly).filter_map(|(p, q)| {
        let axis = if p.x == q.x && p.y != q.y {
            Axis::Vertical
        } else if p.y == q.y && p.x != q.x {
            Axis::Horizontal
        } else {
            return None;
        };
        let (along_p, fixed) = axis.split(p);
        let (along_q, _) = axis.split(q);
        Some((
            axis,
            fixed,
            along_p.min(along_q),
            along_p.max(along_q),
            along_q > along_p,
        ))
    })
}

/// Every stretch of wall rooms `ia` and `ib` actually share.
///
/// Adjacency is matched between real coincident collinear *edges* of the two
/// footprints, not between their bounding boxes. A bounding box says only
/// that a room reaches some coordinate, not that it has a wall there: an
/// L-shaped room whose east edge exists for only part of its box was
/// previously reported as adjacent along the whole box side, so a portal cut
/// there emitted sidedefs naming a sector nowhere near the line — the exact
/// invariant the compiler exists to guarantee. The spec admits L-shaped and
/// octagonal footprints, so this is a real shape, not a hypothetical one.
///
/// Rooms may share more than one run of wall (an L wrapped around a
/// rectangle shares two), so every one is returned; [`resolve_portal`] picks
/// the run the portal's midpoint actually lies on. Ordering follows room
/// `ia`'s footprint edges, then room `ib`'s, so the result is deterministic.
///
/// `pub(crate)` so `doors::emit_doors` can re-derive the same shared-wall
/// geometry `cut_portals` already validated, without duplicating this logic.
pub(crate) fn shared_spans(ir: &Ir, ia: usize, ib: usize) -> Vec<SharedSpan> {
    let mut spans = Vec::new();
    for (axis_a, fixed_a, lo_a, hi_a, a_forward) in wall_edges(&ir.rooms[ia].footprint) {
        for (axis_b, fixed_b, lo_b, hi_b, _) in wall_edges(&ir.rooms[ib].footprint) {
            if axis_a != axis_b || fixed_a != fixed_b {
                continue;
            }
            let (lo, hi) = (lo_a.max(lo_b), hi_a.min(hi_b));
            // Strict: edges that meet at a single point share no wall.
            if lo < hi {
                spans.push(SharedSpan {
                    axis: axis_a,
                    fixed: fixed_a,
                    lo,
                    hi,
                    a_forward,
                });
            }
        }
    }
    spans
}

/// Everything a pass needs to know about one portal's placement, resolved and
/// validated once.
///
/// Both `cut_portals` and `doors::plan_doors` read from this rather than each
/// re-deriving the opening's ends: computed twice, the two were free to
/// diverge, and a door's carved recess must sit at exactly the span its
/// flanking walls were cut for.
pub(crate) struct PortalGeometry {
    /// Index of room `a` in `ir.rooms`, which is also its sector index.
    pub(crate) ia: usize,
    /// Index of room `b` in `ir.rooms`, which is also its sector index.
    pub(crate) ib: usize,
    /// The shared wall run the portal sits in.
    pub(crate) span: SharedSpan,
    /// The low end of the opening.
    pub(crate) open_lo: i32,
    /// The high end of the opening.
    pub(crate) open_hi: i32,
}

/// Resolves one portal against the rooms' real shared walls.
///
/// # Errors
/// Returns [`CompileError::NotAdjacent`] when the rooms share no wall at all,
/// [`CompileError::PortalOffWall`] when the midpoint lies on none of the
/// walls they do share, and [`CompileError::PortalTooWide`] when the opening
/// would run past the ends of that wall.
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

    let spans = shared_spans(ir, ia, ib);
    if spans.is_empty() {
        return Err(CompileError::NotAdjacent {
            a: portal.a.clone(),
            b: portal.b.clone(),
        });
    }

    let span = spans
        .into_iter()
        .find(|span| {
            let (on_axis, across) = span.axis.split(portal.at);
            across == span.fixed && on_axis > span.lo && on_axis < span.hi
        })
        .ok_or_else(|| CompileError::PortalOffWall {
            a: portal.a.clone(),
            b: portal.b.clone(),
            x: portal.at.x,
            y: portal.at.y,
        })?;

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

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use crate::compile::portals::cut_portals;
    use crate::compile::sectors::emit_sectors;
    use crate::compile::{CompileError, MapData};
    use crate::geom::{Pt, contains};
    use crate::ir::Ir;

    fn ir_with_portal(width: i32, at: (i32, i32), b_origin: i32) -> String {
        format!(
            r#"{{ "seed":1, "grid":64, "theme":"tech_base",
              "rooms":[
                {{ "id":"a", "footprint":[[0,0],[0,256],[256,256],[256,0]],
                   "floor":0, "ceiling":128, "light":160,
                   "floor_tex":"F", "ceil_tex":"C", "wall_tex":"W" }},
                {{ "id":"b", "footprint":[[{b_origin},0],[{b_origin},256],[{},256],[{},0]],
                   "floor":0, "ceiling":128, "light":160,
                   "floor_tex":"F", "ceil_tex":"C", "wall_tex":"W" }}
              ],
              "portals":[{{ "a":"a", "b":"b", "kind":"plain", "width":{width}, "at":[{},{}] }}] }}"#,
            b_origin + 256,
            b_origin + 256,
            at.0,
            at.1
        )
    }

    #[test]
    fn a_portal_becomes_one_two_sided_linedef_pairing_both_sectors() {
        let ir = Ir::from_json(&ir_with_portal(128, (256, 128), 256)).expect("ir");
        let mut data = emit_sectors(&ir).expect("sectors");
        cut_portals(&ir, &mut data).expect("portals cut");

        let two_sided: Vec<_> = data.linedefs.iter().filter(|l| l.back.is_some()).collect();
        assert_eq!(two_sided.len(), 1, "exactly one two-sided line");

        let l = two_sided[0];
        let front_sector = data.sidedefs[l.front].sector;
        let back_sector = data.sidedefs[l.back.expect("back")].sector;
        assert_ne!(
            front_sector, back_sector,
            "front and back name different sectors"
        );
        assert!(!l.blocking, "a portal does not block movement");
    }

    #[test]
    fn cutting_leaves_each_room_watertight() {
        let ir = Ir::from_json(&ir_with_portal(128, (256, 128), 256)).expect("ir");
        let mut data = emit_sectors(&ir).expect("sectors");
        let before = data.linedefs.len();
        cut_portals(&ir, &mut data).expect("portals cut");
        // Each side's wall splits into two flanking one-sided pieces plus the
        // shared two-sided opening: 8 - 2 + 4 + 1 = 11.
        assert_eq!(
            data.linedefs.len(),
            before - 2 + 5,
            "wall split accounted for"
        );
    }

    #[test]
    fn rejects_a_portal_between_rooms_that_share_no_wall() {
        let ir = Ir::from_json(&ir_with_portal(128, (256, 128), 512)).expect("ir");
        let mut data = emit_sectors(&ir).expect("sectors");
        assert!(matches!(
            cut_portals(&ir, &mut data),
            Err(CompileError::NotAdjacent { .. })
        ));
    }

    #[test]
    fn rejects_an_opening_wider_than_the_shared_wall() {
        let ir = Ir::from_json(&ir_with_portal(512, (256, 128), 256)).expect("ir");
        let mut data = emit_sectors(&ir).expect("sectors");
        assert!(matches!(
            cut_portals(&ir, &mut data),
            Err(CompileError::PortalTooWide { .. })
        ));
    }

    #[test]
    fn rejects_an_opening_that_is_not_on_the_shared_wall() {
        let ir = Ir::from_json(&ir_with_portal(128, (128, 128), 256)).expect("ir");
        let mut data = emit_sectors(&ir).expect("sectors");
        assert!(matches!(
            cut_portals(&ir, &mut data),
            Err(CompileError::PortalOffWall { .. })
        ));
    }

    /// Whether room `room_idx`'s original footprint has its interior on the
    /// right of travel from `p` to `q`.
    ///
    /// This is computed independently of `cut_portals`'s own `Cut`/`a_forward`
    /// bookkeeping: rotate the direction vector -90 degrees (`(dy, -dx)`,
    /// reduced to a one-unit step since every segment under test is
    /// axis-aligned) from the segment's midpoint, and ask `geom::contains`
    /// whether that probe point lands inside the room's footprint. A test
    /// that instead re-derived `a_forward` would just restate the
    /// implementation and could not catch a regression in it.
    fn interior_is_on_the_right(ir: &Ir, room_idx: usize, p: Pt, q: Pt) -> bool {
        let (dx, dy) = (q.x - p.x, q.y - p.y);
        let probe = Pt {
            x: i32::midpoint(p.x, q.x) + dy.signum(),
            y: i32::midpoint(p.y, q.y) - dx.signum(),
        };
        contains(&ir.rooms[room_idx].footprint, probe)
    }

    /// Asserts that every linedef `cut_portals` leaves behind — the
    /// untouched walls, the flanking one-sided pieces, and the two-sided
    /// opening — has its front sidedef naming the sector whose interior is
    /// on the right of `v1`-to-`v2` travel, and (for the two-sided line) its
    /// back sidedef naming the sector on the left. This is the structural
    /// invariant the whole pass exists to guarantee: a linedef's declared
    /// sides must match its actual geometry, or the engine (and crustywad's
    /// own BSP/GL-node builder) attributes the wrong sector to the wrong
    /// side.
    fn assert_sidedefs_face_their_sectors(ir_json: &str) {
        let (ir, data) = assert_well_formed(ir_json);

        for l in &data.linedefs {
            let (p, q) = (data.vertices[l.v1], data.vertices[l.v2]);
            let front_sector = data.sidedefs[l.front].sector;
            assert!(
                interior_is_on_the_right(&ir, front_sector, p, q),
                "front sidedef of line {p:?} -> {q:?} names sector {front_sector}, \
                 but that room's interior is not on the right of travel"
            );
            if let Some(back) = l.back {
                let back_sector = data.sidedefs[back].sector;
                assert!(
                    interior_is_on_the_right(&ir, back_sector, q, p),
                    "back sidedef of line {p:?} -> {q:?} names sector {back_sector}, \
                     but that room's interior is not on the left of travel"
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
    /// that every sector's boundary closes, and hands back the result.
    fn assert_well_formed(ir_json: &str) -> (Ir, MapData) {
        let ir = Ir::from_json(ir_json).expect("ir");
        let mut data = emit_sectors(&ir).expect("sectors");
        cut_portals(&ir, &mut data).expect("portals cut");
        assert_sector_boundaries_are_closed(&data);
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

    /// Two rooms whose shared wall is *not* the full side of either: room
    /// `a` runs y 0..256 and room `b` runs y 128..384, both against x = 256,
    /// so they share only y 128..256.
    const OFFSET_ROOMS: &str = r#"{ "seed":1, "grid":64, "theme":"tech_base",
      "rooms":[
        { "id":"a", "footprint":[[0,0],[0,256],[256,256],[256,0]],
           "floor":0, "ceiling":128, "light":160,
           "floor_tex":"F", "ceil_tex":"C", "wall_tex":"WA" },
        { "id":"b", "footprint":[[256,128],[256,384],[512,384],[512,128]],
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
    fn an_offset_room_pair_shares_only_the_overlapping_stretch_of_wall() {
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
            wall_pieces(&data, 1, 256),
            vec![(128, 160), (224, 384)],
            "room b's wall keeps its own ends (y 128 and y 384) around the opening"
        );
        assert_eq!(
            data.linedefs.iter().filter(|l| l.back.is_some()).count(),
            1,
            "exactly one two-sided opening"
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
        // Room b sits against x = 256 over y 0..128 — inside room a's
        // bounding box but past the end of room a's actual wall, which stops
        // at y = 128. Bounding-box adjacency reported a shared wall here and
        // cut a portal into thin air, naming room a's sector on a line
        // nowhere near room a.
        let ir_json = format!(
            r#"{{ "seed":1, "grid":64, "theme":"tech_base",
              "rooms":[
                {L_SHAPED_A},
                {{ "id":"b", "footprint":[[256,0],[256,128],[384,128],[384,0]],
                   "floor":0, "ceiling":128, "light":160,
                   "floor_tex":"F", "ceil_tex":"C", "wall_tex":"W" }}
              ],
              "portals":[{{ "a":"a", "b":"b", "kind":"plain", "width":64, "at":[256,64] }}] }}"#
        );
        let ir = Ir::from_json(&ir_json).expect("ir");
        let mut data = emit_sectors(&ir).expect("sectors");
        assert!(matches!(
            cut_portals(&ir, &mut data),
            Err(CompileError::NotAdjacent { .. })
        ));
    }

    #[test]
    fn an_l_shaped_room_opens_where_it_does_have_a_wall() {
        // The same L, with room b moved up against the stretch of wall that
        // genuinely exists (y 128..256).
        let ir_json = format!(
            r#"{{ "seed":1, "grid":64, "theme":"tech_base",
              "rooms":[
                {L_SHAPED_A},
                {{ "id":"b", "footprint":[[256,128],[256,256],[384,256],[384,128]],
                   "floor":0, "ceiling":128, "light":160,
                   "floor_tex":"F", "ceil_tex":"C", "wall_tex":"W" }}
              ],
              "portals":[{{ "a":"a", "b":"b", "kind":"plain", "width":64, "at":[256,192] }}] }}"#
        );
        let (_, data) = assert_well_formed(&ir_json);
        assert_eq!(
            data.linedefs.iter().filter(|l| l.back.is_some()).count(),
            1,
            "the portal opened"
        );
        assert_eq!(
            wall_pieces(&data, 0, 256),
            vec![(128, 160), (224, 256)],
            "the L's short wall is split within its own extent"
        );
    }

    /// Two 256x512 rooms sharing the whole of x = 256, with two separate
    /// openings in that one wall.
    fn two_portal_ir(first: i32, first_width: i32, second: i32, second_width: i32) -> String {
        format!(
            r#"{{ "seed":1, "grid":64, "theme":"tech_base",
              "rooms":[
                {{ "id":"a", "footprint":[[0,0],[0,512],[256,512],[256,0]],
                   "floor":0, "ceiling":128, "light":160,
                   "floor_tex":"F", "ceil_tex":"C", "wall_tex":"W" }},
                {{ "id":"b", "footprint":[[256,0],[256,512],[512,512],[512,0]],
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
    fn two_portals_in_one_wall_both_open_without_corrupting_each_other() {
        let (_, data) = assert_well_formed(&two_portal_ir(128, 64, 384, 64));
        assert_eq!(
            data.linedefs.iter().filter(|l| l.back.is_some()).count(),
            2,
            "both portals opened"
        );
        for sector in [0, 1] {
            assert_eq!(
                wall_pieces(&data, sector, 256),
                vec![(0, 96), (160, 352), (416, 512)],
                "sector {sector}'s wall is in three pieces, one between the two openings"
            );
        }
    }

    #[test]
    fn rejects_two_portals_whose_openings_overlap_in_the_same_wall() {
        // y 64..192 and y 128..256 overlap over y 128..192. Cutting the
        // second would find no intact wall where the first already opened.
        let ir = Ir::from_json(&two_portal_ir(128, 128, 192, 128)).expect("ir");
        let mut data = emit_sectors(&ir).expect("sectors");
        assert!(matches!(
            cut_portals(&ir, &mut data),
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
            2,
            "both portals opened"
        );
        for sector in [0, 1] {
            assert_eq!(
                wall_pieces(&data, sector, 256),
                vec![(0, 64), (320, 512)],
                "sector {sector} has no zero-length wall stub between the two openings"
            );
        }
    }

    #[test]
    fn sidedefs_face_their_sectors_when_room_a_is_west_of_a_vertical_wall() {
        assert_sidedefs_face_their_sectors(&ir_with_portal(128, (256, 128), 256));
    }

    #[test]
    fn sidedefs_face_their_sectors_when_room_a_is_east_of_a_vertical_wall() {
        let ir_json = r#"{ "seed":1, "grid":64, "theme":"tech_base",
          "rooms":[
            { "id":"a", "footprint":[[256,0],[256,256],[512,256],[512,0]],
               "floor":0, "ceiling":128, "light":160,
               "floor_tex":"F", "ceil_tex":"C", "wall_tex":"W" },
            { "id":"b", "footprint":[[0,0],[0,256],[256,256],[256,0]],
               "floor":0, "ceiling":128, "light":160,
               "floor_tex":"F", "ceil_tex":"C", "wall_tex":"W" }
          ],
          "portals":[{ "a":"a", "b":"b", "kind":"plain", "width":128, "at":[256,128] }] }"#;
        assert_sidedefs_face_their_sectors(ir_json);
    }

    #[test]
    fn sidedefs_face_their_sectors_when_room_a_is_south_of_a_horizontal_wall() {
        let ir_json = r#"{ "seed":1, "grid":64, "theme":"tech_base",
          "rooms":[
            { "id":"a", "footprint":[[0,0],[0,256],[256,256],[256,0]],
               "floor":0, "ceiling":128, "light":160,
               "floor_tex":"F", "ceil_tex":"C", "wall_tex":"W" },
            { "id":"b", "footprint":[[0,256],[0,512],[256,512],[256,256]],
               "floor":0, "ceiling":128, "light":160,
               "floor_tex":"F", "ceil_tex":"C", "wall_tex":"W" }
          ],
          "portals":[{ "a":"a", "b":"b", "kind":"plain", "width":128, "at":[128,256] }] }"#;
        assert_sidedefs_face_their_sectors(ir_json);
    }

    #[test]
    fn sidedefs_face_their_sectors_when_room_a_is_north_of_a_horizontal_wall() {
        let ir_json = r#"{ "seed":1, "grid":64, "theme":"tech_base",
          "rooms":[
            { "id":"a", "footprint":[[0,256],[0,512],[256,512],[256,256]],
               "floor":0, "ceiling":128, "light":160,
               "floor_tex":"F", "ceil_tex":"C", "wall_tex":"W" },
            { "id":"b", "footprint":[[0,0],[0,256],[256,256],[256,0]],
               "floor":0, "ceiling":128, "light":160,
               "floor_tex":"F", "ceil_tex":"C", "wall_tex":"W" }
          ],
          "portals":[{ "a":"a", "b":"b", "kind":"plain", "width":128, "at":[128,256] }] }"#;
        assert_sidedefs_face_their_sectors(ir_json);
    }
}
