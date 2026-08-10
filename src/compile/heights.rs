//! Fills in the wall textures a height difference exposes.

use crate::compile::MapData;

/// Which of a sidedef's two height-difference slots is being filled.
enum Slot {
    /// The face above the opening, shown where ceilings differ.
    Upper,
    /// The face below the opening, shown where floors differ.
    Lower,
}

/// Returns the sidedef the engine draws the **lower** texture on, given each
/// side's own floor height and sidedef index — or `None` when the floors are
/// equal and no lower is needed at all.
///
/// Doom samples exactly one sidedef per floor difference (`r_segs.c`,
/// `R_StoreWallRange` at the pinned commit): the bottom texture is drawn
/// `if (worldlow > worldbottom)`, both measured from the front sector. That
/// resolves to a simple rule: the **lower** always comes from the sidedef
/// whose own sector has the **lower** floor — so `front` is visible when
/// `back`'s floor is the higher one, and `back` is visible otherwise.
///
/// This is the single place that comparison is made. [`apply_height_textures`]
/// and `crate::rules::check_missing_textures` (the pass that fills a lower
/// texture and the rule that requires one) both call this rather than
/// re-deriving the comparison, so they cannot independently drift on which
/// side is correct — the exact failure mode ("compiles clean, renders
/// broken") this pass exists to close.
#[must_use]
pub fn visible_lower_side(
    front_floor: i32,
    back_floor: i32,
    front: usize,
    back: usize,
) -> Option<usize> {
    match front_floor.cmp(&back_floor) {
        std::cmp::Ordering::Equal => None,
        std::cmp::Ordering::Less => Some(front),
        std::cmp::Ordering::Greater => Some(back),
    }
}

/// Returns the sidedef the engine draws the **upper** texture on, given each
/// side's own ceiling height and sidedef index — or `None` when the ceilings
/// are equal and no upper is needed at all.
///
/// Doom samples exactly one sidedef per ceiling difference (`r_segs.c`,
/// `R_StoreWallRange` at the pinned commit): the top texture is drawn
/// `if (worldhigh < worldtop)`, both measured from the front sector. That
/// resolves to a simple rule: the **upper** always comes from the sidedef
/// whose own sector has the **higher** ceiling — so `front` is visible when
/// `back`'s ceiling is the lower one, and `back` is visible otherwise.
///
/// See [`visible_lower_side`]'s doc comment for why this comparison is
/// factored out into its own function rather than inlined at each call site.
#[must_use]
pub fn visible_upper_side(
    front_ceiling: i32,
    back_ceiling: i32,
    front: usize,
    back: usize,
) -> Option<usize> {
    match front_ceiling.cmp(&back_ceiling) {
        std::cmp::Ordering::Equal => None,
        std::cmp::Ordering::Greater => Some(front),
        std::cmp::Ordering::Less => Some(back),
    }
}

/// Writes the upper and lower textures every two-sided line needs, on the
/// one side the engine actually draws.
///
/// Which side that is comes from [`visible_lower_side`] and
/// [`visible_upper_side`]; see their doc comments for the `r_segs.c`
/// justification. The opposite sidedef is never read by the engine, which is
/// why this fills one side and leaves the other bare — matching what vanilla
/// maps themselves do 98.6% of the time (see the verticality corpus report).
///
/// The texture is the bordering sector's own
/// [`SectorOut::wall_tex`](crate::compile::SectorOut::wall_tex), so a riser
/// matches the room a player is standing in when they see it. The corpus
/// supports this over a dedicated step-texture family: eight of the twelve
/// most common riser textures are ordinary wall textures.
///
/// **Only empty slots are filled.** [`crate::compile::doors::emit_doors`] has
/// already written the theme's door texture onto both door faces' `upper`,
/// and an unconditional pass would replace a door panel with a plain wall.
///
/// Must run after every sector-emitting pass, since it reads final floor and
/// ceiling heights; it neither creates nor moves geometry.
pub fn apply_height_textures(data: &mut MapData) {
    for i in 0..data.linedefs.len() {
        let line = &data.linedefs[i];
        let Some(back) = line.back else { continue };
        let front = line.front;

        let front_sector = data.sidedefs[front].sector;
        let back_sector = data.sidedefs[back].sector;
        let (front_floor, front_ceiling) = {
            let s = &data.sectors[front_sector];
            (s.floor, s.ceiling)
        };
        let (back_floor, back_ceiling) = {
            let s = &data.sectors[back_sector];
            (s.floor, s.ceiling)
        };

        if let Some(visible) = visible_lower_side(front_floor, back_floor, front, back) {
            fill(data, visible, &Slot::Lower);
        }
        if let Some(visible) = visible_upper_side(front_ceiling, back_ceiling, front, back) {
            fill(data, visible, &Slot::Upper);
        }
    }
}

/// Writes `sidedef`'s `slot` from its own sector's wall texture, unless an
/// earlier pass already claimed it.
fn fill(data: &mut MapData, sidedef: usize, slot: &Slot) {
    let side = &data.sidedefs[sidedef];
    let claimed = match slot {
        Slot::Upper => !side.upper.is_empty(),
        Slot::Lower => !side.lower.is_empty(),
    };
    if claimed {
        return;
    }
    let texture = data.sectors[side.sector].wall_tex.clone();
    let side = &mut data.sidedefs[sidedef];
    match slot {
        Slot::Upper => side.upper = texture,
        Slot::Lower => side.lower = texture,
    }
}

#[cfg(test)]
mod tests {
    use crate::compile::heights::apply_height_textures;
    use crate::compile::portals::cut_portals;
    use crate::compile::sectors::emit_sectors;
    use crate::compile::{LinedefOut, MapData, SectorOut, SidedefOut};
    use crate::geom::Pt;
    use crate::ir::Ir;
    use crate::tables::Tables;

    /// Two rooms joined by a plain portal, each height independently
    /// tunable, and each with its own distinctive wall texture so a test can
    /// prove *which* sector a riser was sourced from as well as which side
    /// it landed on.
    fn ir_json(floor_a: i32, ceiling_a: i32, floor_b: i32, ceiling_b: i32) -> String {
        format!(
            r#"{{ "seed":1, "grid":64, "theme":"tech_base",
              "rooms":[
                {{ "id":"a", "footprint":[[0,0],[0,256],[256,256],[256,0]],
                   "floor":{floor_a}, "ceiling":{ceiling_a}, "light":160,
                   "floor_tex":"F", "ceil_tex":"C", "wall_tex":"WA" }},
                {{ "id":"b", "footprint":[[320,0],[320,256],[576,256],[576,0]],
                   "floor":{floor_b}, "ceiling":{ceiling_b}, "light":160,
                   "floor_tex":"F", "ceil_tex":"C", "wall_tex":"WB" }}
              ],
              "portals":[{{ "a":"a", "b":"b", "kind":"plain", "width":128, "at":[256,128] }}] }}"#
        )
    }

    /// Compiles the geometry far enough to run the pass, without the rule
    /// catalog — these tests are about what the pass emits, not whether the
    /// finished map is playable.
    fn emit(json: &str) -> MapData {
        let ir = Ir::from_json(json).expect("ir");
        let tables = Tables::load().expect("tables");
        let mut data = emit_sectors(&ir).expect("sectors");
        cut_portals(&ir, &tables, &mut data).expect("portals");
        apply_height_textures(&mut data);
        data
    }

    /// The (front, back) sidedef indices of the linedef joining sectors `x`
    /// and `y`, in that order. Found by search rather than by hard-coded
    /// index, so a change in emission order does not silently retarget the
    /// assertion at a different wall.
    fn boundary(data: &MapData, x: usize, y: usize) -> (usize, usize) {
        for l in &data.linedefs {
            let Some(back) = l.back else { continue };
            if (data.sidedefs[l.front].sector, data.sidedefs[back].sector) == (x, y) {
                return (l.front, back);
            }
        }
        panic!("no linedef joins sectors {x} and {y}");
    }

    #[test]
    fn a_drop_textures_only_the_lower_rooms_side() {
        // Room b stands 24 units above room a, so the passage sector takes
        // b's floor and room a looks up at a 24-unit riser.
        let data = emit(&ir_json(0, 128, 24, 128));
        let (front, back) = boundary(&data, 0, 2);
        assert_eq!(
            data.sidedefs[front].lower, "WA",
            "the riser is drawn from room a, the lower side, in room a's own texture"
        );
        assert!(
            data.sidedefs[back].lower.is_empty(),
            "the passage side is never sampled by the renderer and stays bare"
        );
    }

    #[test]
    fn the_same_drop_reversed_textures_the_other_side() {
        // The mirror image: room a is now the high side, so the riser must
        // move to room b's boundary. A rule hard-coded to room a passes the
        // test above and fails this one.
        let data = emit(&ir_json(24, 128, 0, 128));
        let (front, back) = boundary(&data, 1, 2);
        assert_eq!(data.sidedefs[front].lower, "WB");
        assert!(data.sidedefs[back].lower.is_empty());
        let (a_front, _) = boundary(&data, 0, 2);
        assert!(
            data.sidedefs[a_front].lower.is_empty(),
            "room a is flush with the passage and needs no riser at all"
        );
    }

    #[test]
    fn a_ceiling_difference_textures_the_taller_side() {
        // Floors are equal, so only the upper branch can fire — isolating it
        // from the lower branch, which would otherwise mask a mutation.
        let data = emit(&ir_json(0, 128, 0, 256));
        let (front, back) = boundary(&data, 1, 2);
        assert_eq!(
            data.sidedefs[front].upper, "WB",
            "room b has the higher ceiling, so its side carries the upper"
        );
        assert!(data.sidedefs[back].upper.is_empty());
        assert!(
            data.sidedefs[front].lower.is_empty(),
            "equal floors must not produce a lower"
        );
    }

    #[test]
    fn both_textures_land_on_the_rooms_side_never_the_passages() {
        // The passage sector always takes the higher floor and the lower
        // ceiling, so it is the inner sector on both counts: a room is
        // simultaneously the lower-floor side and the higher-ceiling side of
        // its own boundary, and both textures land there.
        let data = emit(&ir_json(0, 256, 24, 128));
        let (front, back) = boundary(&data, 0, 2);
        assert_eq!(data.sidedefs[front].lower, "WA");
        assert_eq!(data.sidedefs[front].upper, "WA");
        assert!(data.sidedefs[back].lower.is_empty());
        assert!(data.sidedefs[back].upper.is_empty());
    }

    #[test]
    fn a_higher_front_floor_and_lower_front_ceiling_textures_the_back_side() {
        // The `back` arm of `visible_lower_side`/`visible_upper_side` *is*
        // reachable through the real compiler — a door portal across a
        // floor difference produces it directly. The door sector always
        // takes `min(floors)` (`compile::doors::emit_doors`), and
        // `compile::portals::emit_segment`'s far threshold puts the far
        // neighbor (a room or an alcove) on the front and the door sector
        // on the back; when the far neighbor sits higher, that makes the
        // door sector's own sidedef the visible `back`.
        // `rules::tests::a_door_across_a_floor_difference_puts_the_lower_on_the_doors_own_side`
        // pins exactly this case through `compile`, not a hand-built
        // `MapData`. What the hand-built fixture below buys instead is
        // isolation: two sectors and one two-sided linedef, with `front`'s
        // own sector given the higher floor and the lower ceiling, so both
        // the lower and the upper must land on `back` with nothing else in
        // the fixture that could produce that result by accident — that
        // isolation, not necessity, is why this test exists alongside the
        // real-pipeline one.
        let mut data = MapData {
            vertices: vec![Pt { x: 0, y: 0 }, Pt { x: 0, y: 64 }],
            sectors: vec![
                SectorOut {
                    floor: 32,
                    ceiling: 96,
                    light: 160,
                    floor_tex: "F".to_owned(),
                    ceil_tex: "C".to_owned(),
                    special: 0,
                    tag: 0,
                    wall_tex: "FRONT_TEX".to_owned(),
                },
                SectorOut {
                    floor: 0,
                    ceiling: 128,
                    light: 160,
                    floor_tex: "F".to_owned(),
                    ceil_tex: "C".to_owned(),
                    special: 0,
                    tag: 0,
                    wall_tex: "BACK_TEX".to_owned(),
                },
            ],
            sidedefs: vec![
                SidedefOut {
                    sector: 0,
                    upper: String::new(),
                    middle: String::new(),
                    lower: String::new(),
                    x_offset: 0,
                },
                SidedefOut {
                    sector: 1,
                    upper: String::new(),
                    middle: String::new(),
                    lower: String::new(),
                    x_offset: 0,
                },
            ],
            linedefs: vec![LinedefOut {
                v1: 0,
                v2: 1,
                front: 0,
                back: Some(1),
                blocking: false,
                special: 0,
                tag: 0,
                lower_unpegged: false,
                upper_unpegged: false,
                secret: false,
            }],
        };
        apply_height_textures(&mut data);
        assert_eq!(
            data.sidedefs[1].lower, "BACK_TEX",
            "back has the lower floor, so the lower comes from back's own texture"
        );
        assert_eq!(
            data.sidedefs[1].upper, "BACK_TEX",
            "back has the higher ceiling, so the upper comes from back's own texture too"
        );
        assert!(
            data.sidedefs[0].lower.is_empty(),
            "front is never sampled by the renderer here and stays bare"
        );
        assert!(data.sidedefs[0].upper.is_empty());
    }
}
