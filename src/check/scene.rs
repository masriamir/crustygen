//! The verifier's re-derived model of a parsed UDMF map.
//!
//! [`Scene::build`] walks every linedef exactly once, validating its
//! cross-references (`v1`/`v2`, `sidefront`, `sideback`, and each referenced
//! sidedef's `sector`) against the map's own declaration counts rather than
//! trusting them, and turns each valid linedef into one or two [`Boundary`]
//! segments filed under the sector(s) it borders. Later tasks (closure,
//! thing→sector resolution, and the check passes that read this data) build
//! on top of what this module establishes.

use crate::check::{Finding, Severity, Subject};
use crate::tables::Tables;
use crustywad::map::udmf::{UdmfLinedef, UdmfMap};

/// One directed edge of a sector's boundary, built from one side of a
/// linedef.
///
/// A one-sided linedef contributes a single [`Boundary`] to its front
/// sector. A two-sided linedef contributes two mirrored boundaries — one
/// filed under each bordering sector — so each sector always sees its own
/// edges walking `a` → `b` in its own winding: the front mirror runs `v1` →
/// `v2`, the back mirror `v2` → `v1`.
#[derive(Debug, Clone)]
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
    /// Whether the linedef has a back sidedef (UDMF `twosided`, crustywad
    /// flag bit value 4). Compared here as a literal; Task 4 replaces it
    /// with the sourced [`Tables`] constant.
    pub two_sided: bool,
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

    /// Whether the player can cross this edge.
    ///
    /// `two_sided` only for now; Task 4 tightens this to
    /// `two_sided && !blocking` once the `blocking` field exists.
    #[must_use]
    pub fn passable(&self) -> bool {
        self.two_sided
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
    /// Whether the boundary closes into simple loop(s).
    ///
    /// Always `false` here — Task 3 computes and owns this field.
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
    /// The sector containing this thing.
    ///
    /// Always `None` here — Task 3 resolves it.
    pub sector: Option<usize>,
    /// The thing's species/kind name.
    ///
    /// Always `None` here — Task 4's reverse lookup resolves it.
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

/// Bit value of crustywad's `twosided` flag (`doomdata.h`'s `ML_TWOSIDED`
/// position). A literal for now; Task 4 replaces it with the sourced
/// `Tables` constant.
const TWOSIDED_FLAG: u32 = 4;

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

/// Validates linedef `i`'s cross-references and, if it is well-formed,
/// pushes its [`Boundary`] contribution(s) into `sectors`. On any violation,
/// pushes exactly one `"V-S"` [`Finding`] and contributes no boundary.
fn process_linedef(
    i: usize,
    line: &UdmfLinedef,
    map: &UdmfMap,
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

    let two_sided = line.flags & TWOSIDED_FLAG != 0;
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
            special: line.special,
            tag: line.args[0],
            fronts_this: false,
            sidedef: sideback,
        });
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
    #[must_use]
    pub fn build(map: &UdmfMap, tables: &Tables, findings: &mut Vec<Finding>) -> Self {
        // Not read until Task 4 adds `blocking`/`lower_unpegged`, sourced
        // from `tables.linedef_flag`.
        let _ = tables;

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

        let things = map
            .things
            .iter()
            .map(|thing| SceneThing {
                x: thing.x,
                y: thing.y,
                angle: thing.angle,
                type_id: thing.type_id,
                flags: thing.flags,
                sector: None,
                name: None,
            })
            .collect();

        for (i, line) in map.linedefs.iter().enumerate() {
            process_linedef(i, line, map, &mut sectors, findings);
        }

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
}
