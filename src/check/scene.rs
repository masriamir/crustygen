//! The verifier's re-derived model of a parsed UDMF map.
//!
//! [`Scene::build`] walks every linedef exactly once, validating its
//! cross-references (`v1`/`v2`, `sidefront`, `sideback`, and each referenced
//! sidedef's `sector`) against the map's own declaration counts rather than
//! trusting them, and turns each valid linedef into one or two [`Boundary`]
//! segments filed under the sector(s) it borders. It then closes each
//! sector's boundary (even-degree check) and resolves each thing to the
//! first closed sector containing it. Later tasks (the check passes that
//! read this data) build on top of what this module establishes.

use crate::check::{Finding, Severity, Subject};
use crate::tables::Tables;
use crustywad::map::udmf::{UdmfLinedef, UdmfMap};
use std::collections::HashMap;

/// One directed edge of a sector's boundary, built from one side of a
/// linedef.
///
/// A one-sided linedef contributes a single [`Boundary`] to its front
/// sector. A two-sided linedef contributes two mirrored boundaries — one
/// filed under each bordering sector — so each sector always sees its own
/// edges walking `a` → `b` in its own winding: the front mirror runs `v1` →
/// `v2`, the back mirror `v2` → `v1`.
#[derive(Debug, Clone)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "each bool mirrors an independent bit of the engine's `maplinedef_t.flags` \
              bitfield (two_sided, blocking, upper_unpegged, lower_unpegged) or an independent \
              structural fact about which mirror this is (fronts_this) — the same reasoning \
              LinedefOut gives in compile/mod.rs for the emission side of this exact bitfield"
)]
pub struct Boundary {
    /// The edge's start point, world units.
    pub a: (f64, f64),
    /// The edge's end point, world units.
    pub b: (f64, f64),
    /// Declaration index of the underlying linedef.
    pub linedef: usize,
    /// Declaration index of the sector on the other side of the linedef, if
    /// two-sided.
    pub neighbor: Option<usize>,
    /// Whether the linedef has a back sidedef (UDMF `twosided`, sourced
    /// `ML_TWOSIDED` bit — [`Tables::linedef_flag`]`("two_sided")`).
    pub two_sided: bool,
    /// Whether the linedef is solid (UDMF `blocking`, sourced `ML_BLOCKING`
    /// bit — [`Tables::linedef_flag`]`("blocking")`). A two-sided line can
    /// still carry this flag (e.g. a fence the player cannot walk through
    /// but can see and shoot across), which is why [`Boundary::passable`]
    /// checks both.
    pub blocking: bool,
    /// Whether the linedef's upper texture is unpegged (UDMF `dontpegtop`,
    /// sourced `ML_DONTPEGTOP` bit — [`Tables::linedef_flag`]
    /// `("upper_unpegged")`).
    pub upper_unpegged: bool,
    /// Whether the linedef's lower texture is unpegged (UDMF `dontpegbottom`,
    /// sourced `ML_DONTPEGBOTTOM` bit — [`Tables::linedef_flag`]
    /// `("lower_unpegged")`).
    pub lower_unpegged: bool,
    /// The linedef's special type.
    pub special: i32,
    /// The linedef's sector tag (`args[0]` in the `doom` namespace).
    pub tag: i32,
    /// Whether this mirror was built from the linedef's front side.
    pub fronts_this: bool,
    /// Declaration index of the sidedef this mirror was built from.
    pub sidedef: usize,
}

impl Boundary {
    /// The edge's length, world units.
    #[must_use]
    pub fn len(&self) -> f64 {
        (self.b.0 - self.a.0).hypot(self.b.1 - self.a.1)
    }

    /// Whether the player can cross this edge: two-sided and not flagged
    /// solid (`ML_BLOCKING` can be set on a two-sided line, e.g. a fence the
    /// player can see and shoot across but not walk through).
    #[must_use]
    pub fn passable(&self) -> bool {
        self.two_sided && !self.blocking
    }
}

/// A sector's re-derived geometry and boundary, one entry per declared
/// sector.
#[derive(Debug, Clone)]
pub struct SceneSector {
    /// Floor height.
    pub floor: i32,
    /// Ceiling height.
    pub ceiling: i32,
    /// Light level.
    pub light: i32,
    /// Sector special.
    pub special: i32,
    /// Sector tag (`UdmfSector.id`).
    pub tag: i32,
    /// This sector's boundary edges, one per linedef side that fronts or
    /// backs it.
    pub boundary: Vec<Boundary>,
    /// Whether the boundary closes into simple loop(s): every vertex
    /// endpoint's degree, counted across this sector's boundary segments,
    /// is even.
    pub closed: bool,
}

/// A thing's re-derived placement, one entry per declared thing.
#[derive(Debug, Clone)]
pub struct SceneThing {
    /// X coordinate, world units.
    pub x: f64,
    /// Y coordinate, world units.
    pub y: f64,
    /// Angle in degrees, raw (not normalized).
    pub angle: i32,
    /// The thing's `DoomEd` type number.
    pub type_id: i32,
    /// Doom/Boom-MBF-mapped flag bits (see `UdmfThing::flags`).
    pub flags: u32,
    /// Declaration index of the first (declaration order) closed sector
    /// whose boundary contains this thing, or `None` if no closed sector
    /// does.
    pub sector: Option<usize>,
    /// The thing's species/kind name, resolved from `type_id` via
    /// [`Tables::thing_kinds`]'s reverse map. `None` if `type_id` is
    /// negative, does not fit a `u16`, or is not a doomednum the vocabulary
    /// names.
    pub name: Option<String>,
}

/// The verifier's re-derived model of a UDMF map: per-sector boundaries and
/// per-thing placement, cross-referenced from declaration indices rather
/// than trusted outright.
#[derive(Debug, Clone)]
pub struct Scene {
    /// One entry per declared sector, in declaration order.
    pub sectors: Vec<SceneSector>,
    /// One entry per declared thing, in declaration order.
    pub things: Vec<SceneThing>,
}

/// Builds a [`Finding`] for check `"V-S"` naming `linedef`.
fn reference_error(linedef: usize, message: String) -> Finding {
    Finding {
        check: "V-S",
        severity: Severity::Error,
        subject: Subject::Linedef(linedef),
        message,
    }
}

/// Converts a UDMF declaration index (`i32`, no defined negative meaning
/// here) to a `usize` valid for indexing a `len`-long declaration list.
fn index_in(idx: i32, len: usize) -> Option<usize> {
    usize::try_from(idx).ok().filter(|&i| i < len)
}

/// Resolves a linedef's `field` index (naming it `kind` in the message) to a
/// `usize` valid for `len`. Pushes a `"V-S"` finding naming linedef `i` and
/// returns `None` if `idx` is out of range.
fn resolve_index(
    i: usize,
    field: &str,
    kind: &str,
    idx: i32,
    len: usize,
    findings: &mut Vec<Finding>,
) -> Option<usize> {
    let resolved = index_in(idx, len);
    if resolved.is_none() {
        findings.push(reference_error(
            i,
            format!("{field} references {kind} {idx}, but the map has {len} {kind}s"),
        ));
    }
    resolved
}

/// The sourced `ML_TWOSIDED`/`ML_BLOCKING`/`ML_DONTPEGTOP`/`ML_DONTPEGBOTTOM`
/// bit values, resolved once per [`Scene::build`] call rather than
/// re-looked-up for every linedef.
#[derive(Debug, Clone, Copy)]
struct BoundaryFlagBits {
    two_sided: u32,
    blocking: u32,
    upper_unpegged: u32,
    lower_unpegged: u32,
}

impl BoundaryFlagBits {
    /// Resolves all four bits from `tables`. Panics if any is missing —
    /// they are guaranteed present in `engine.toml`'s `[linedef.flags]`.
    fn resolve(tables: &Tables) -> Self {
        Self {
            two_sided: u32::from(
                tables
                    .linedef_flag("two_sided")
                    .expect("sourced in engine.toml"),
            ),
            blocking: u32::from(
                tables
                    .linedef_flag("blocking")
                    .expect("sourced in engine.toml"),
            ),
            upper_unpegged: u32::from(
                tables
                    .linedef_flag("upper_unpegged")
                    .expect("sourced in engine.toml"),
            ),
            lower_unpegged: u32::from(
                tables
                    .linedef_flag("lower_unpegged")
                    .expect("sourced in engine.toml"),
            ),
        }
    }

    /// Reads `(two_sided, blocking, upper_unpegged, lower_unpegged)` off a
    /// linedef's `flags`.
    fn read(self, flags: u32) -> (bool, bool, bool, bool) {
        (
            flags & self.two_sided != 0,
            flags & self.blocking != 0,
            flags & self.upper_unpegged != 0,
            flags & self.lower_unpegged != 0,
        )
    }
}

/// Validates linedef `i`'s cross-references and, if it is well-formed,
/// pushes its [`Boundary`] contribution(s) into `sectors`. On any violation,
/// pushes exactly one `"V-S"` [`Finding`] and contributes no boundary.
fn process_linedef(
    i: usize,
    line: &UdmfLinedef,
    map: &UdmfMap,
    flag_bits: BoundaryFlagBits,
    sectors: &mut [SceneSector],
    findings: &mut Vec<Finding>,
) {
    let Some(v1) = resolve_index(i, "v1", "vertex", line.v1, map.vertices.len(), findings) else {
        return;
    };
    let Some(v2) = resolve_index(i, "v2", "vertex", line.v2, map.vertices.len(), findings) else {
        return;
    };
    let Some(sidefront) = resolve_index(
        i,
        "sidefront",
        "sidedef",
        line.sidefront,
        map.sidedefs.len(),
        findings,
    ) else {
        return;
    };
    let Some(front_sector) = resolve_index(
        i,
        "sidefront's sector",
        "sector",
        map.sidedefs[sidefront].sector,
        sectors.len(),
        findings,
    ) else {
        return;
    };

    let back = match line.sideback {
        None => None,
        Some(sb) => {
            let Some(sideback) =
                resolve_index(i, "sideback", "sidedef", sb, map.sidedefs.len(), findings)
            else {
                return;
            };
            let Some(back_sector) = resolve_index(
                i,
                "sideback's sector",
                "sector",
                map.sidedefs[sideback].sector,
                sectors.len(),
                findings,
            ) else {
                return;
            };
            Some((sideback, back_sector))
        }
    };

    let (two_sided, blocking, upper_unpegged, lower_unpegged) = flag_bits.read(line.flags);
    if two_sided != back.is_some() {
        findings.push(reference_error(
            i,
            format!(
                "twosided flag is {two_sided} but sideback is {}",
                if back.is_some() { "present" } else { "absent" }
            ),
        ));
        return;
    }

    let v1_pt = (map.vertices[v1].x, map.vertices[v1].y);
    let v2_pt = (map.vertices[v2].x, map.vertices[v2].y);

    sectors[front_sector].boundary.push(Boundary {
        a: v1_pt,
        b: v2_pt,
        linedef: i,
        neighbor: back.map(|(_, back_sector)| back_sector),
        two_sided,
        blocking,
        upper_unpegged,
        lower_unpegged,
        special: line.special,
        tag: line.args[0],
        fronts_this: true,
        sidedef: sidefront,
    });

    if let Some((sideback, back_sector)) = back {
        sectors[back_sector].boundary.push(Boundary {
            a: v2_pt,
            b: v1_pt,
            linedef: i,
            neighbor: Some(front_sector),
            two_sided,
            blocking,
            upper_unpegged,
            lower_unpegged,
            special: line.special,
            tag: line.args[0],
            fronts_this: false,
            sidedef: sideback,
        });
    }
}

/// Degree (count of boundary-segment endpoints) of each vertex in a
/// sector's boundary, keyed by the coordinate pair's raw bit pattern
/// (`f64::to_bits`) rather than by value or with a tolerance.
///
/// Every coordinate here was copied verbatim from `map.vertices` in
/// [`process_linedef`] — never computed — so two boundary endpoints at the
/// "same" vertex are always bit-identical, and bit-equality sidesteps
/// picking an arbitrary epsilon.
fn endpoint_degrees(sector: &SceneSector) -> HashMap<(u64, u64), usize> {
    let mut degree = HashMap::new();
    for b in &sector.boundary {
        *degree
            .entry((b.a.0.to_bits(), b.a.1.to_bits()))
            .or_insert(0) += 1;
        *degree
            .entry((b.b.0.to_bits(), b.b.1.to_bits()))
            .or_insert(0) += 1;
    }
    degree
}

/// Whether sector `i`'s boundary closes into simple loop(s): every vertex
/// endpoint's degree is even. On the first odd-degree vertex found, pushes
/// a `"V-S"` Error naming the sector and that vertex, and returns `false`.
fn sector_is_closed(i: usize, sector: &SceneSector, findings: &mut Vec<Finding>) -> bool {
    for (&(xb, yb), &degree) in &endpoint_degrees(sector) {
        if degree % 2 != 0 {
            let (x, y) = (f64::from_bits(xb), f64::from_bits(yb));
            findings.push(Finding {
                check: "V-S",
                severity: Severity::Error,
                subject: Subject::Sector(i),
                message: format!(
                    "boundary does not close: vertex ({x}, {y}) has odd degree {degree}"
                ),
            });
            return false;
        }
    }
    true
}

/// Even-odd containment of `(x, y)` in `sector`'s boundary segments.
/// Sound only for a closed boundary; callers gate on `sector.closed`.
pub(crate) fn sector_contains(sector: &SceneSector, x: f64, y: f64) -> bool {
    let mut inside = false;
    for b in &sector.boundary {
        let (ax, ay) = b.a;
        let (bx, by) = b.b;
        if (ay > y) != (by > y) {
            let cross_x = ax + (y - ay) * (bx - ax) / (by - ay);
            if x < cross_x {
                inside = !inside;
            }
        }
    }
    inside
}

/// Resolves each thing to the first (declaration order) closed sector
/// whose boundary contains it, pushing a `"V-S"` Error naming the thing
/// when none does.
///
/// A point sitting exactly on a boundary edge is unspecified by even-odd
/// containment ([`sector_contains`]) — the compiler never emits a thing
/// there, so this is not a practical soundness gap.
fn resolve_things(sectors: &[SceneSector], things: &mut [SceneThing], findings: &mut Vec<Finding>) {
    for (i, thing) in things.iter_mut().enumerate() {
        thing.sector = sectors
            .iter()
            .position(|s| s.closed && sector_contains(s, thing.x, thing.y));
        if thing.sector.is_none() {
            findings.push(Finding {
                check: "V-S",
                severity: Severity::Error,
                subject: Subject::Thing(i),
                message: format!(
                    "thing at ({}, {}) is outside every closed sector",
                    thing.x, thing.y
                ),
            });
        }
    }
}

impl Scene {
    /// Builds a [`Scene`] from a parsed UDMF map.
    ///
    /// Validates every cross-reference a linedef makes — `v1`, `v2`,
    /// `sidefront`, `sideback`, and each referenced sidedef's `sector` —
    /// against the map's own declaration counts, and checks that the
    /// two-sided flag agrees with whether a `sideback` is present. A linedef
    /// that fails any of these checks contributes no [`Boundary`] to any
    /// sector; each failure pushes exactly one `"V-S"` [`Finding`] onto
    /// `findings`, naming the offending linedef and index.
    ///
    /// Once every linedef is processed, closes each sector's boundary
    /// (`sector_is_closed`) and resolves each thing to the first closed
    /// sector containing it (`resolve_things`), each pushing its own `"V-S"`
    /// findings.
    #[must_use]
    pub fn build(map: &UdmfMap, tables: &Tables, findings: &mut Vec<Finding>) -> Self {
        let kind_names: HashMap<u16, String> = tables
            .thing_kinds()
            .map(|(name, id)| (id, name.to_owned()))
            .collect();
        let flag_bits = BoundaryFlagBits::resolve(tables);

        let mut sectors: Vec<SceneSector> = map
            .sectors
            .iter()
            .map(|sector| SceneSector {
                floor: sector.heightfloor,
                ceiling: sector.heightceiling,
                light: sector.lightlevel,
                special: sector.special,
                tag: sector.id,
                boundary: Vec::new(),
                closed: false,
            })
            .collect();

        let mut things: Vec<SceneThing> = map
            .things
            .iter()
            .map(|thing| SceneThing {
                x: thing.x,
                y: thing.y,
                angle: thing.angle,
                type_id: thing.type_id,
                flags: thing.flags,
                sector: None,
                name: u16::try_from(thing.type_id)
                    .ok()
                    .and_then(|id| kind_names.get(&id).cloned()),
            })
            .collect();

        for (i, line) in map.linedefs.iter().enumerate() {
            process_linedef(i, line, map, flag_bits, &mut sectors, findings);
        }

        for (i, sector) in sectors.iter_mut().enumerate() {
            sector.closed = sector_is_closed(i, sector, findings);
        }

        resolve_things(&sectors, &mut things, findings);

        Self { sectors, things }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tables::Tables;
    use crustywad::Limits;
    use crustywad::map::udmf::parse_udmf;

    /// Two unit-square sectors sharing edge x=64: sector 0 left, sector 1 right.
    /// Sidedefs: 0 → sector 0 (shared line front), 1 → sector 1 (shared line back),
    /// then one per perimeter wall.
    const TWO_BOX: &str = r#"namespace = "doom";
vertex { x = 0.000; y = 0.000; }
vertex { x = 64.000; y = 0.000; }
vertex { x = 128.000; y = 0.000; }
vertex { x = 128.000; y = 64.000; }
vertex { x = 64.000; y = 64.000; }
vertex { x = 0.000; y = 64.000; }
linedef { v1 = 1; v2 = 4; sidefront = 0; sideback = 1; twosided = true; }
linedef { v1 = 0; v2 = 1; sidefront = 2; blocking = true; }
linedef { v1 = 4; v2 = 5; sidefront = 3; blocking = true; }
linedef { v1 = 5; v2 = 0; sidefront = 4; blocking = true; }
linedef { v1 = 1; v2 = 2; sidefront = 5; blocking = true; }
linedef { v1 = 2; v2 = 3; sidefront = 6; blocking = true; }
linedef { v1 = 3; v2 = 4; sidefront = 7; blocking = true; }
sidedef { sector = 0; texturemiddle = "-"; }
sidedef { sector = 1; texturemiddle = "-"; }
sidedef { sector = 0; texturemiddle = "STARTAN2"; }
sidedef { sector = 0; texturemiddle = "STARTAN2"; }
sidedef { sector = 0; texturemiddle = "STARTAN2"; }
sidedef { sector = 1; texturemiddle = "STARTAN2"; }
sidedef { sector = 1; texturemiddle = "STARTAN2"; }
sidedef { sector = 1; texturemiddle = "STARTAN2"; }
sector { texturefloor = "FLOOR4_8"; textureceiling = "CEIL3_5"; heightceiling = 128; lightlevel = 160; }
sector { texturefloor = "FLOOR4_8"; textureceiling = "CEIL3_5"; heightceiling = 128; lightlevel = 160; }
thing { x = 32.000; y = 32.000; type = 1; skill1 = true; skill2 = true; skill3 = true; skill4 = true; skill5 = true; single = true; dm = true; coop = true; }
"#;

    fn scene_of(text: &str) -> (Scene, Vec<crate::check::Finding>) {
        let map = parse_udmf(text, Limits::default()).expect("fixture parses");
        let tables = Tables::load().expect("tables");
        let mut findings = Vec::new();
        let scene = Scene::build(&map, &tables, &mut findings);
        (scene, findings)
    }

    #[test]
    fn a_shared_edge_appears_in_both_sectors_boundaries_with_mirrored_neighbors() {
        let (scene, findings) = scene_of(TWO_BOX);
        assert!(findings.is_empty(), "clean fixture: {findings:?}");
        assert_eq!(scene.sectors.len(), 2);
        assert_eq!(scene.sectors[0].boundary.len(), 4);
        assert_eq!(scene.sectors[1].boundary.len(), 4);
        let shared0 = scene.sectors[0]
            .boundary
            .iter()
            .find(|b| b.linedef == 0)
            .expect("present");
        let shared1 = scene.sectors[1]
            .boundary
            .iter()
            .find(|b| b.linedef == 0)
            .expect("present");
        assert_eq!(shared0.neighbor, Some(1));
        assert!(shared0.fronts_this);
        assert_eq!(shared1.neighbor, Some(0));
        assert!(!shared1.fronts_this);
        assert!((shared0.len() - 64.0).abs() < 1e-9);
    }

    #[test]
    fn a_twosided_line_flagged_blocking_is_not_passable() {
        // The shared edge stays two-sided but also carries ML_BLOCKING — a
        // fence the player can see and shoot across but not walk through.
        let blocked = TWO_BOX.replace(
            "linedef { v1 = 1; v2 = 4; sidefront = 0; sideback = 1; twosided = true; }",
            "linedef { v1 = 1; v2 = 4; sidefront = 0; sideback = 1; twosided = true; blocking = true; }",
        );
        let (scene, findings) = scene_of(&blocked);
        assert!(findings.is_empty(), "clean fixture: {findings:?}");
        let shared = scene.sectors[0]
            .boundary
            .iter()
            .find(|b| b.linedef == 0)
            .expect("present");
        assert!(shared.two_sided, "still two-sided");
        assert!(
            shared.blocking,
            "blocking flag read back off the sourced bit"
        );
        assert!(
            !shared.lower_unpegged,
            "the fixture never sets dontpegbottom"
        );
        assert!(
            !shared.passable(),
            "a blocking two-sided line is not passable"
        );
    }

    #[test]
    fn a_dangling_sidedef_sector_index_is_reported_and_the_linedef_is_skipped() {
        let broken = TWO_BOX.replace(
            "sidedef { sector = 1; texturemiddle = \"-\"; }",
            "sidedef { sector = 9; texturemiddle = \"-\"; }",
        );
        let (_, findings) = scene_of(&broken);
        assert!(
            findings
                .iter()
                .any(|f| f.check == "V-S" && matches!(f.subject, crate::check::Subject::Linedef(0)))
        );
    }

    #[test]
    fn a_dangling_v1_vertex_index_is_reported_and_the_linedef_is_skipped() {
        let broken = TWO_BOX.replace(
            "linedef { v1 = 1; v2 = 4; sidefront = 0; sideback = 1; twosided = true; }",
            "linedef { v1 = 99; v2 = 4; sidefront = 0; sideback = 1; twosided = true; }",
        );
        let (scene, findings) = scene_of(&broken);
        assert!(
            findings.iter().any(|f| f.check == "V-S"
                && matches!(f.subject, crate::check::Subject::Linedef(0))
                && f.message.contains("v1 references vertex 99")),
            "expected a v1 reference-validity finding: {findings:?}"
        );
        assert!(
            scene.sectors[0].boundary.iter().all(|b| b.linedef != 0),
            "the broken linedef contributes no boundary"
        );
    }

    #[test]
    fn a_dangling_v2_vertex_index_is_reported_and_the_linedef_is_skipped() {
        let broken = TWO_BOX.replace(
            "linedef { v1 = 1; v2 = 4; sidefront = 0; sideback = 1; twosided = true; }",
            "linedef { v1 = 1; v2 = 99; sidefront = 0; sideback = 1; twosided = true; }",
        );
        let (_, findings) = scene_of(&broken);
        assert!(
            findings.iter().any(|f| f.check == "V-S"
                && matches!(f.subject, crate::check::Subject::Linedef(0))
                && f.message.contains("v2 references vertex 99")),
            "expected a v2 reference-validity finding: {findings:?}"
        );
    }

    #[test]
    fn a_dangling_sidefront_index_is_reported_and_the_linedef_is_skipped() {
        let broken = TWO_BOX.replace(
            "linedef { v1 = 0; v2 = 1; sidefront = 2; blocking = true; }",
            "linedef { v1 = 0; v2 = 1; sidefront = 99; blocking = true; }",
        );
        let (_, findings) = scene_of(&broken);
        assert!(
            findings.iter().any(|f| f.check == "V-S"
                && matches!(f.subject, crate::check::Subject::Linedef(1))
                && f.message.contains("sidefront references sidedef 99")),
            "expected a sidefront reference-validity finding: {findings:?}"
        );
    }

    #[test]
    fn a_dangling_sideback_index_is_reported_and_the_linedef_is_skipped() {
        let broken = TWO_BOX.replace(
            "linedef { v1 = 1; v2 = 4; sidefront = 0; sideback = 1; twosided = true; }",
            "linedef { v1 = 1; v2 = 4; sidefront = 0; sideback = 99; twosided = true; }",
        );
        let (_, findings) = scene_of(&broken);
        assert!(
            findings.iter().any(|f| f.check == "V-S"
                && matches!(f.subject, crate::check::Subject::Linedef(0))
                && f.message.contains("sideback references sidedef 99")),
            "expected a sideback reference-validity finding: {findings:?}"
        );
    }

    #[test]
    fn a_twosided_flag_disagreeing_with_sidedef_presence_is_reported() {
        // Perimeter line 1 claims twosided but has no back sidedef.
        let broken = TWO_BOX.replace(
            "linedef { v1 = 0; v2 = 1; sidefront = 2; blocking = true; }",
            "linedef { v1 = 0; v2 = 1; sidefront = 2; twosided = true; }",
        );
        let (_, findings) = scene_of(&broken);
        assert!(
            findings
                .iter()
                .any(|f| f.check == "V-S" && matches!(f.subject, crate::check::Subject::Linedef(1)))
        );
    }

    #[test]
    fn closure_holds_for_both_boxes_and_breaks_when_a_wall_is_removed() {
        let (scene, _) = scene_of(TWO_BOX);
        assert!(scene.sectors.iter().all(|s| s.closed));
        // Drop sector 1's east wall (linedef 4: v1=1,v2=2... pick the line and delete it).
        let broken = TWO_BOX.replace(
            "linedef { v1 = 2; v2 = 3; sidefront = 6; blocking = true; }\n",
            "",
        );
        let (scene, findings) = scene_of(&broken);
        assert!(!scene.sectors[1].closed);
        assert!(
            findings
                .iter()
                .any(|f| f.check == "V-S" && matches!(f.subject, crate::check::Subject::Sector(1)))
        );
    }

    #[test]
    fn a_thing_resolves_to_the_sector_that_contains_it() {
        let (scene, _) = scene_of(TWO_BOX);
        assert_eq!(scene.things.len(), 1);
        assert_eq!(
            scene.things[0].sector,
            Some(0),
            "thing at (32,32) is in the left box"
        );
    }

    #[test]
    fn a_thing_outside_every_sector_is_reported() {
        let stray = TWO_BOX.replace(
            "thing { x = 32.000; y = 32.000;",
            "thing { x = 500.000; y = 500.000;",
        );
        let (scene, findings) = scene_of(&stray);
        assert_eq!(scene.things[0].sector, None);
        assert!(
            findings
                .iter()
                .any(|f| f.check == "V-S" && matches!(f.subject, crate::check::Subject::Thing(0)))
        );
    }

    #[test]
    fn a_things_name_resolves_from_the_reverse_thing_lookup() {
        let (scene, _) = scene_of(TWO_BOX);
        assert_eq!(
            scene.things[0].name.as_deref(),
            Some("player1_start"),
            "type 1 resolves through Tables::thing_kinds"
        );

        let unknown = TWO_BOX.replace(
            "thing { x = 32.000; y = 32.000; type = 1;",
            "thing { x = 32.000; y = 32.000; type = 31337;",
        );
        let (scene, _) = scene_of(&unknown);
        assert_eq!(
            scene.things[0].name, None,
            "a type_id the vocabulary does not name resolves to no name"
        );
    }

    #[test]
    fn an_l_shaped_sector_contains_its_notch_correctly() {
        // Non-convex single sector: L-shape. Point in the notch (outside) vs in the leg.
        // 6-vertex L: (0,0)(96,0)(96,32)(32,32)(32,96)(0,96), all one-sided walls.
        let l_shape = r#"namespace = "doom";
vertex { x = 0.000; y = 0.000; }
vertex { x = 96.000; y = 0.000; }
vertex { x = 96.000; y = 32.000; }
vertex { x = 32.000; y = 32.000; }
vertex { x = 32.000; y = 96.000; }
vertex { x = 0.000; y = 96.000; }
linedef { v1 = 0; v2 = 1; sidefront = 0; blocking = true; }
linedef { v1 = 1; v2 = 2; sidefront = 1; blocking = true; }
linedef { v1 = 2; v2 = 3; sidefront = 2; blocking = true; }
linedef { v1 = 3; v2 = 4; sidefront = 3; blocking = true; }
linedef { v1 = 4; v2 = 5; sidefront = 4; blocking = true; }
linedef { v1 = 5; v2 = 0; sidefront = 5; blocking = true; }
sidedef { sector = 0; texturemiddle = "STARTAN2"; }
sidedef { sector = 0; texturemiddle = "STARTAN2"; }
sidedef { sector = 0; texturemiddle = "STARTAN2"; }
sidedef { sector = 0; texturemiddle = "STARTAN2"; }
sidedef { sector = 0; texturemiddle = "STARTAN2"; }
sidedef { sector = 0; texturemiddle = "STARTAN2"; }
sector { texturefloor = "FLOOR4_8"; textureceiling = "CEIL3_5"; heightceiling = 128; lightlevel = 160; }
thing { x = 16.000; y = 64.000; type = 1; single = true; }
thing { x = 64.000; y = 64.000; type = 1; single = true; }
"#;
        let (scene, _) = scene_of(l_shape);
        assert_eq!(scene.things[0].sector, Some(0), "in the vertical leg");
        assert_eq!(scene.things[1].sector, None, "the notch is outside the L");
    }

    #[test]
    fn a_sector_in_sector_ring_resolves_a_thing_in_its_inner_sector() {
        // Sector 0 is a "ring": a single sector whose boundary is TWO
        // disjoint loops — the outer 160x160 square, plus a second,
        // independent loop around a 40x40 hole in the middle (both filed
        // under sector 0, one-sided walls, so no shared linedef ties them
        // together). Sector 1 is a wholly separate room filling that same
        // hole. No other fixture in this module gives one sector a
        // multi-loop boundary, so `sector_contains`'s even-odd rule has
        // never been exercised against a hole before: a bug that dropped
        // the hole loop's contribution would make the ring wrongly claim
        // the hole (and, since sector 0 is declared first, `resolve_things`
        // would resolve a thing in the hole to the ring instead of the
        // nested inner sector).
        let ring = r#"namespace = "doom";
vertex { x = 0.000; y = 0.000; }
vertex { x = 160.000; y = 0.000; }
vertex { x = 160.000; y = 160.000; }
vertex { x = 0.000; y = 160.000; }
vertex { x = 60.000; y = 60.000; }
vertex { x = 100.000; y = 60.000; }
vertex { x = 100.000; y = 100.000; }
vertex { x = 60.000; y = 100.000; }
vertex { x = 60.000; y = 60.000; }
vertex { x = 100.000; y = 60.000; }
vertex { x = 100.000; y = 100.000; }
vertex { x = 60.000; y = 100.000; }
linedef { v1 = 0; v2 = 1; sidefront = 0; blocking = true; }
linedef { v1 = 1; v2 = 2; sidefront = 1; blocking = true; }
linedef { v1 = 2; v2 = 3; sidefront = 2; blocking = true; }
linedef { v1 = 3; v2 = 0; sidefront = 3; blocking = true; }
linedef { v1 = 4; v2 = 5; sidefront = 4; blocking = true; }
linedef { v1 = 5; v2 = 6; sidefront = 5; blocking = true; }
linedef { v1 = 6; v2 = 7; sidefront = 6; blocking = true; }
linedef { v1 = 7; v2 = 4; sidefront = 7; blocking = true; }
linedef { v1 = 8; v2 = 9; sidefront = 8; blocking = true; }
linedef { v1 = 9; v2 = 10; sidefront = 9; blocking = true; }
linedef { v1 = 10; v2 = 11; sidefront = 10; blocking = true; }
linedef { v1 = 11; v2 = 8; sidefront = 11; blocking = true; }
sidedef { sector = 0; texturemiddle = "STARTAN2"; }
sidedef { sector = 0; texturemiddle = "STARTAN2"; }
sidedef { sector = 0; texturemiddle = "STARTAN2"; }
sidedef { sector = 0; texturemiddle = "STARTAN2"; }
sidedef { sector = 0; texturemiddle = "STARTAN2"; }
sidedef { sector = 0; texturemiddle = "STARTAN2"; }
sidedef { sector = 0; texturemiddle = "STARTAN2"; }
sidedef { sector = 0; texturemiddle = "STARTAN2"; }
sidedef { sector = 1; texturemiddle = "STARTAN2"; }
sidedef { sector = 1; texturemiddle = "STARTAN2"; }
sidedef { sector = 1; texturemiddle = "STARTAN2"; }
sidedef { sector = 1; texturemiddle = "STARTAN2"; }
sector { texturefloor = "FLOOR4_8"; textureceiling = "CEIL3_5"; heightceiling = 128; lightlevel = 160; }
sector { texturefloor = "FLOOR4_8"; textureceiling = "CEIL3_5"; heightceiling = 128; lightlevel = 160; }
thing { x = 20.000; y = 20.000; type = 1; single = true; }
thing { x = 80.000; y = 80.000; type = 1; single = true; }
"#;
        let (scene, findings) = scene_of(ring);
        assert!(findings.is_empty(), "clean ring fixture: {findings:?}");
        assert!(scene.sectors[0].closed, "the ring's two loops both close");
        assert!(scene.sectors[1].closed, "the inner room closes");
        assert_eq!(
            scene.things[0].sector,
            Some(0),
            "(20, 20) is in the ring, well clear of the hole"
        );
        assert_eq!(
            scene.things[1].sector,
            Some(1),
            "(80, 80) sits in the hole: the ring's hole loop must exclude it, and it must \
             resolve to the nested inner sector instead"
        );
    }
}
