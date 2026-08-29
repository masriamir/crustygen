//! The teleport recognizer: classifies every teleport line of a parsed map
//! from the verifier's [`Scene`], by engine semantics.
//!
//! Recognition, not approximation. A line is resolved the way `EV_Teleport`
//! resolves it — the first tagged sector in declaration order that holds a
//! `teleport_dest`, the first such thing in THINGS order — and classified on
//! what that says: who may cross (the special), whether it survives a
//! crossing, whether the front sector holds monsters (a closet), whether the
//! destination borders an exit line. Geometry — island, alcove, boundary —
//! is reported as a statistic and never gates: expressibility is about the
//! semantics the IR can carry, not the shape the compiler would draw.
//!
//! Two shapes are refused, because the IR would have to misrepresent them:
//! a line that can never fire ([`Refusal::Broken`]: tag 0, no tagged sector,
//! no marker on the tag, or one-sided — `PIT_CheckLine` rejects a one-sided
//! line before `P_CrossSpecialLine` runs) and a line whose two sides name
//! one sector ([`Refusal::SelfReferencing`], a mapping trick with no
//! counterpart in id's own maps: 7 lines across DOOM + DOOM2 against 19.4 %
//! of idgames trigger lines). A resolved sector holding more than one marker
//! is [`TeleportLine::ambiguous`] and reported, not refused — the engine's
//! pick is deterministic and the IR expresses it with one marker.

use std::collections::BTreeSet;

use crate::check::flood::resolve_teleport_destination;
use crate::check::scene::Scene;
use crate::tables::Tables;

/// Who may cross a teleport line, read from its special.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TeleportKind {
    /// Any thing may cross, the player included
    /// ([`Tables::player_teleport_specials`]).
    Player,
    /// Only a non-player thing fires it — `P_CrossSpecialLine` guards both
    /// forms with `if (!thing->player)`
    /// ([`Tables::monster_teleport_specials`]).
    MonstersOnly,
}

/// The shape of the sector behind a teleport line, reported as a statistic
/// and never a gate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Geometry {
    /// A free-standing pad: every edge of the back sector is two-sided and
    /// they all lead to one host sector.
    Island,
    /// A recess: exactly one edge of the back sector is two-sided.
    Alcove,
    /// Any other back sector — the trigger is a plain border between two
    /// rooms rather than the rim of a pad.
    Boundary,
    /// No back sector to classify, which today means the line was refused.
    Other,
}

/// Why a teleport line cannot be expressed as a map-spec teleport.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Refusal {
    /// The line can never fire: it is one-sided (`PIT_CheckLine` rejects it
    /// before `P_CrossSpecialLine` runs), carries tag 0, or its tag resolves
    /// to no sector holding a `teleport_dest` marker.
    Broken,
    /// The line's two sidedefs name one sector — a mapping trick the IR has
    /// no way to state.
    SelfReferencing,
}

/// One recognized teleport line.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "each bool is an independent fact recognized about one line — how it is \
              triggered (repeatable), what its front sector holds (closet), what its \
              destination borders (exit), what its back sector holds (paired), and how many \
              markers the destination holds (ambiguous) — with no state they jointly encode"
)]
pub struct TeleportLine {
    /// Declaration index of the linedef.
    pub linedef: usize,
    /// The linedef's special.
    pub special: i32,
    /// Who may cross it.
    pub kind: TeleportKind,
    /// Whether the line survives a crossing (a `WR`/`W1` distinction: the
    /// one-shot forms clear their own special).
    pub repeatable: bool,
    /// Whether the line's **front** sector holds a monster — a teleport
    /// closet.
    pub closet: bool,
    /// Whether the resolved destination sector borders an exit line.
    pub exit: bool,
    /// The back sector's shape.
    pub geometry: Geometry,
    /// Whether the back sector holds a marker too, so the trigger is one leg
    /// of a two-way pair.
    pub paired: bool,
    /// Whether the resolved destination sector holds more than one marker.
    /// Reported, never refused: the engine's pick is deterministic.
    pub ambiguous: bool,
    /// Declaration index of the sector the line's tag resolves to, engine-
    /// style, or `None` when it resolves to none.
    pub destination: Option<usize>,
    /// Why the line is not expressible, if it is not.
    pub refusal: Option<Refusal>,
}

/// One count per predicate over a map's [`TeleportLine`]s.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize)]
pub struct TeleportCounts {
    /// Teleport lines recognized.
    pub lines: u64,
    /// Lines any thing may cross.
    pub player: u64,
    /// Lines only a non-player thing fires.
    pub monsters_only: u64,
    /// Lines that clear their own special after firing once.
    pub one_shot: u64,
    /// Lines whose front sector holds a monster.
    pub closet: u64,
    /// Lines whose destination sector borders an exit line.
    pub exit: u64,
    /// Lines whose back sector holds a marker.
    pub paired: u64,
    /// Lines behind a free-standing pad.
    pub island: u64,
    /// Lines behind a recess.
    pub alcove: u64,
    /// Lines on a plain border between two rooms.
    pub boundary: u64,
    /// Lines with no back sector to classify.
    pub other: u64,
    /// Lines whose destination holds more than one marker.
    pub ambiguous: u64,
    /// Lines refused as [`Refusal::Broken`].
    pub broken: u64,
    /// Lines refused as [`Refusal::SelfReferencing`].
    pub self_referencing: u64,
}

impl TeleportCounts {
    /// Lines refused for any reason.
    #[must_use]
    pub fn refusals(&self) -> u64 {
        self.broken + self.self_referencing
    }

    /// Field-wise sum, for rolling per-map counts into a corpus total.
    ///
    /// Saturating rather than wrapping: a corpus large enough to overflow a
    /// `u64` line count cannot exist, and a pinned ceiling is a better
    /// report than a wrapped one if that ever stops being true.
    #[must_use]
    pub fn add(&self, other: &Self) -> Self {
        Self {
            lines: self.lines.saturating_add(other.lines),
            player: self.player.saturating_add(other.player),
            monsters_only: self.monsters_only.saturating_add(other.monsters_only),
            one_shot: self.one_shot.saturating_add(other.one_shot),
            closet: self.closet.saturating_add(other.closet),
            exit: self.exit.saturating_add(other.exit),
            paired: self.paired.saturating_add(other.paired),
            island: self.island.saturating_add(other.island),
            alcove: self.alcove.saturating_add(other.alcove),
            boundary: self.boundary.saturating_add(other.boundary),
            other: self.other.saturating_add(other.other),
            ambiguous: self.ambiguous.saturating_add(other.ambiguous),
            broken: self.broken.saturating_add(other.broken),
            self_referencing: self.self_referencing.saturating_add(other.self_referencing),
        }
    }
}

/// What [`recognize`] says about one map's teleport lines.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct TeleportReport {
    /// One entry per recognized teleport line, by sector then boundary
    /// order.
    pub lines: Vec<TeleportLine>,
    /// The census of [`Self::lines`].
    pub counts: TeleportCounts,
}

/// Classifies every teleport line in `scene`.
///
/// # Panics
///
/// If the vocabulary names no `teleport_dest` thing — the same invariant
/// the flood's `resolve_teleport_destination` relies on.
#[must_use]
pub fn recognize(scene: &Scene, tables: &Tables) -> TeleportReport {
    let all: Vec<i32> = tables
        .teleport_specials()
        .into_iter()
        .map(i32::from)
        .collect();
    let player: Vec<i32> = tables
        .player_teleport_specials()
        .into_iter()
        .map(i32::from)
        .collect();
    let repeatable = [
        i32::from(tables.teleport_special(false, true)),
        i32::from(tables.teleport_special(true, true)),
    ];
    let exits: Vec<i32> = [
        tables.exit_switch_special(),
        tables.secret_exit_switch_special(),
        tables.exit_walkover_special(),
        tables.secret_exit_walkover_special(),
    ]
    .into_iter()
    .map(i32::from)
    .collect();
    let marker = i32::from(
        tables
            .thing_id("teleport_dest")
            .expect("`teleport_dest` is in the vocabulary"),
    );
    let holds_monster = |sector: usize| {
        scene.things.iter().any(|t| {
            t.sector == Some(sector)
                && t.name
                    .as_deref()
                    .is_some_and(|n| tables.spawnhealth(n).is_some())
        })
    };
    let markers_in = |sector: usize| {
        scene
            .things
            .iter()
            .filter(|t| t.type_id == marker && t.sector == Some(sector))
            .count()
    };

    let mut lines = Vec::new();
    for (front, sector) in scene.sectors.iter().enumerate() {
        for b in sector
            .boundary
            .iter()
            .filter(|b| b.fronts_this && all.contains(&b.special))
        {
            let kind = if player.contains(&b.special) {
                TeleportKind::Player
            } else {
                TeleportKind::MonstersOnly
            };
            let mut line = TeleportLine {
                linedef: b.linedef,
                special: b.special,
                kind,
                repeatable: repeatable.contains(&b.special),
                closet: holds_monster(front),
                exit: false,
                geometry: Geometry::Other,
                paired: false,
                ambiguous: false,
                destination: None,
                refusal: None,
            };
            match b.neighbor {
                None => line.refusal = Some(Refusal::Broken),
                Some(back) if back == front => line.refusal = Some(Refusal::SelfReferencing),
                Some(back) => {
                    line.geometry = geometry_of(scene, back);
                    line.paired = markers_in(back) > 0;
                }
            }
            if line.refusal.is_none() {
                match resolve_teleport_destination(scene, tables, b.tag) {
                    None => line.refusal = Some(Refusal::Broken),
                    Some(dest) => {
                        line.destination = Some(dest);
                        line.ambiguous = markers_in(dest) > 1;
                        line.exit = scene.sectors[dest]
                            .boundary
                            .iter()
                            .any(|e| exits.contains(&e.special));
                    }
                }
            }
            lines.push(line);
        }
    }
    let counts = count(&lines);
    TeleportReport { lines, counts }
}

/// The back sector's shape: [`Geometry::Island`] when every edge is
/// two-sided and they all lead to one host, [`Geometry::Alcove`] when
/// exactly one edge is two-sided, [`Geometry::Boundary`] otherwise.
fn geometry_of(scene: &Scene, back: usize) -> Geometry {
    let edges = &scene.sectors[back].boundary;
    let two_sided = edges.iter().filter(|e| e.two_sided).count();
    if two_sided == edges.len() {
        let hosts: BTreeSet<usize> = edges.iter().filter_map(|e| e.neighbor).collect();
        if hosts.len() == 1 {
            return Geometry::Island;
        }
    }
    if two_sided == 1 {
        return Geometry::Alcove;
    }
    Geometry::Boundary
}

/// Counts `lines` one field per predicate.
fn count(lines: &[TeleportLine]) -> TeleportCounts {
    let mut c = TeleportCounts::default();
    for line in lines {
        c.lines += 1;
        match line.kind {
            TeleportKind::Player => c.player += 1,
            TeleportKind::MonstersOnly => c.monsters_only += 1,
        }
        c.one_shot += u64::from(!line.repeatable);
        c.closet += u64::from(line.closet);
        c.exit += u64::from(line.exit);
        c.paired += u64::from(line.paired);
        match line.geometry {
            Geometry::Island => c.island += 1,
            Geometry::Alcove => c.alcove += 1,
            Geometry::Boundary => c.boundary += 1,
            Geometry::Other => c.other += 1,
        }
        c.ambiguous += u64::from(line.ambiguous);
        match line.refusal {
            Some(Refusal::Broken) => c.broken += 1,
            Some(Refusal::SelfReferencing) => c.self_referencing += 1,
            None => {}
        }
    }
    c
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::check::fixtures::{TELEPORT_MAP, scene_of};

    #[test]
    fn a_four_edge_island_with_one_marker_is_a_player_island_line_times_four() {
        let (scene, tables) = scene_of(TELEPORT_MAP);
        let r = recognize(&scene, &tables);
        assert_eq!(r.counts.lines, 4);
        assert_eq!(r.counts.island, 4);
        assert_eq!(r.counts.player, 4);
        assert_eq!(r.counts.refusals(), 0);
        assert!(
            r.lines
                .iter()
                .all(|l| l.destination == Some(1) && l.repeatable && !l.closet && !l.paired)
        );
        assert!(
            r.lines.iter().all(|l| l.exit),
            "sector 1 borders the walkover exit"
        );
    }

    #[test]
    fn one_shot_monsters_only_and_closets_are_read_from_the_special_and_the_front_sector() {
        let map = TELEPORT_MAP
            .replace("special = 97;", "special = 125;")
            .replace(
                "thing { x = 32.0; y = 32.0; angle = 90; type = 1; single = true; }",
                "thing { x = 32.0; y = 32.0; angle = 90; type = 1; single = true; }\nthing { x = 32.0; y = 96.0; angle = 0; type = 3001; single = true; }",
            );
        let (scene, tables) = scene_of(&map);
        let r = recognize(&scene, &tables);
        assert_eq!(
            (r.counts.monsters_only, r.counts.one_shot, r.counts.closet),
            (4, 4, 4)
        );
    }

    #[test]
    fn a_dangling_tag_is_broken_and_a_self_referencing_line_is_refused() {
        let (scene, tables) = scene_of(&TELEPORT_MAP.replace("arg0 = 5;", "arg0 = 9;"));
        let r = recognize(&scene, &tables);
        assert_eq!(r.counts.broken, 4);
        assert!(
            r.lines
                .iter()
                .all(|l| l.refusal == Some(Refusal::Broken) && l.destination.is_none())
        );
        assert_eq!(
            r.counts.island, 4,
            "geometry is reported even for a line the recognizer refuses"
        );

        // A one-sided trigger is the other way to be broken: `PIT_CheckLine`
        // rejects it before `P_CrossSpecialLine` runs, so it can never fire.
        // Dropping the back sidedef means dropping the `twosided` flag with
        // it, or `Scene::build` rejects the linedef outright.
        let (scene, tables) = scene_of(&TELEPORT_MAP.replace(
            "linedef { v1 = 4; v2 = 7; sidefront = 4; sideback = 5; twosided = true; special = 97; arg0 = 5; }",
            "linedef { v1 = 4; v2 = 7; sidefront = 4; special = 97; arg0 = 5; }",
        ));
        let r = recognize(&scene, &tables);
        assert_eq!((r.counts.lines, r.counts.broken, r.counts.other), (4, 1, 1));
        let one_sided = r.lines.iter().find(|l| l.linedef == 4).expect("kept");
        assert_eq!(one_sided.refusal, Some(Refusal::Broken));
        assert_eq!(one_sided.geometry, Geometry::Other);
        assert!(
            !one_sided.paired,
            "a line with no back sector pairs nothing"
        );

        // A two-sided line whose both sidedefs name sector 0: give the
        // island's four back sidedefs the host sector, so each trigger's two
        // sides are one sector.
        let (scene, tables) =
            scene_of(&TELEPORT_MAP.replace("sidedef { sector = 3; }", "sidedef { sector = 0; }"));
        let r = recognize(&scene, &tables);
        assert_eq!(r.counts.self_referencing, 4);
        assert_eq!(r.counts.refusals(), 4);
        assert!(r.lines.iter().all(|l| {
            l.refusal == Some(Refusal::SelfReferencing)
                && l.destination.is_none()
                && l.geometry == Geometry::Other
        }));
    }

    #[test]
    fn a_marker_on_a_trigger_pad_marks_the_line_paired_and_two_markers_ambiguous() {
        // A marker inside the 64..96 pad square makes each island line a
        // paired trigger (the return leg lands back on the pad); a second
        // marker in the tagged destination sector makes the engine's pick
        // ambiguous — reported, never refused.
        let map = TELEPORT_MAP.replace(
            "thing { x = 320.0; y = 64.0; angle = 0; type = 14; single = true; }",
            "thing { x = 320.0; y = 64.0; angle = 0; type = 14; single = true; }\nthing { x = 352.0; y = 64.0; angle = 0; type = 14; single = true; }\nthing { x = 80.0; y = 80.0; angle = 0; type = 14; single = true; }",
        );
        let (scene, tables) = scene_of(&map);
        let r = recognize(&scene, &tables);
        assert_eq!(r.counts.lines, 4);
        assert_eq!((r.counts.paired, r.counts.ambiguous), (4, 4));
        assert_eq!(r.counts.refusals(), 0);
        assert!(
            r.lines
                .iter()
                .all(|l| l.destination == Some(1) && l.geometry == Geometry::Island),
            "the engine takes the first marker in the first tagged sector"
        );
    }

    #[test]
    fn an_alcove_and_a_bare_boundary_line_are_classified_by_their_back_sector() {
        // Two edits: the alcove threshold — sector 2's only two-sided edge —
        // becomes a teleport trigger, and one island line is flipped so the
        // pad fronts it, putting the big room (four solid walls and four pad
        // mirrors) on its back.
        let map = TELEPORT_MAP
            .replace(
                "linedef { v1 = 13; v2 = 12; sidefront = 15; sideback = 16; twosided = true; special = 52; arg0 = 1; }",
                "linedef { v1 = 13; v2 = 12; sidefront = 15; sideback = 16; twosided = true; special = 97; arg0 = 5; }",
            )
            .replace(
                "linedef { v1 = 5; v2 = 4; sidefront = 10; sideback = 11; twosided = true; special = 97; arg0 = 5; }",
                "linedef { v1 = 5; v2 = 4; sidefront = 11; sideback = 10; twosided = true; special = 97; arg0 = 5; }",
            );
        let (scene, tables) = scene_of(&map);
        let r = recognize(&scene, &tables);
        assert_eq!(r.counts.lines, 5);
        assert_eq!(
            (
                r.counts.island,
                r.counts.alcove,
                r.counts.boundary,
                r.counts.other
            ),
            (3, 1, 1, 0)
        );
        assert_eq!(r.counts.refusals(), 0);
        assert!(r.lines.iter().all(|l| l.destination == Some(1)));
        assert_eq!(
            r.counts.exit, 0,
            "the walkover exit is now a teleport trigger, so nothing borders an exit"
        );

        // A pad is an island only while its whole rim leads to one host: the
        // probe found 59 of 218 free-standing idgames pads leading to two.
        // Re-hosting one rim edge (its outer sidedef now names sector 2)
        // makes the pad multi-host — the recognizer reads that wiring, not
        // the drawing — and every trigger around it a plain boundary.
        let (scene, tables) = scene_of(&TELEPORT_MAP.replace(
            "linedef { v1 = 5; v2 = 4; sidefront = 10; sideback = 11; twosided = true; special = 97; arg0 = 5; }",
            "linedef { v1 = 5; v2 = 4; sidefront = 16; sideback = 11; twosided = true; special = 97; arg0 = 5; }",
        ));
        let r = recognize(&scene, &tables);
        assert_eq!(
            (r.counts.lines, r.counts.island, r.counts.boundary),
            (4, 0, 4)
        );
    }

    #[test]
    fn a_report_serializes_to_json_with_snake_case_kinds() {
        let (scene, tables) = scene_of(&TELEPORT_MAP.replace("special = 97;", "special = 126;"));
        let json = serde_json::to_value(recognize(&scene, &tables)).expect("serializes");
        assert_eq!(json["counts"]["monsters_only"], 4);
        assert_eq!(json["lines"][0]["kind"], "monsters_only");
        assert_eq!(json["lines"][0]["geometry"], "island");
        assert_eq!(json["lines"][0]["refusal"], serde_json::Value::Null);

        let (scene, tables) =
            scene_of(&TELEPORT_MAP.replace("sidedef { sector = 3; }", "sidedef { sector = 0; }"));
        let json = serde_json::to_value(recognize(&scene, &tables)).expect("serializes");
        assert_eq!(json["lines"][0]["refusal"], "self_referencing");
    }
}
