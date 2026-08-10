//! Deterministic UDMF text emission.

use std::fmt::Write as _;

use crate::compile::MapData;
use crate::compile::things::ThingOut;

/// Renders map data as UDMF `TEXTMAP` text in the `doom` namespace.
///
/// Emission order is fixed — vertices, linedefs, sidedefs, sectors, things,
/// each in index order — so identical input yields byte-identical output.
/// `scalex` and `scaley` are never written (P9).
///
/// [`MapData::sidedefs`] may contain entries no longer referenced by any
/// linedef's `front`/`back` — an opening that consumes a room's wall end to
/// end leaves that wall's sidedef record with no surviving piece to inherit
/// it, and the record stays, since renumbering every surviving `front`/`back`
/// index to close the gap is exactly the kind of change that is easy to get
/// subtly wrong. Those entries are written out here like any other: they are
/// inert to the engine (nothing points at them) and their presence keeps
/// every `LinedefOut::front`/`back` index a direct, unrenumbered index into
/// this output's `sidedef` list.
#[must_use]
pub fn emit_textmap(data: &MapData, things: &[ThingOut]) -> String {
    let mut s = String::from("namespace = \"doom\";\n\n");

    for v in &data.vertices {
        let _ = writeln!(s, "vertex {{ x = {}.0; y = {}.0; }}", v.x, v.y);
    }
    s.push('\n');

    for l in &data.linedefs {
        let _ = write!(
            s,
            "linedef {{ v1 = {}; v2 = {}; sidefront = {};",
            l.v1, l.v2, l.front
        );
        if let Some(back) = l.back {
            let _ = write!(s, " sideback = {back}; twosided = true;");
        }
        if l.blocking {
            s.push_str(" blocking = true;");
        }
        if l.special != 0 {
            let _ = write!(s, " special = {}; arg0 = {};", l.special, l.tag);
        }
        if l.lower_unpegged {
            s.push_str(" dontpegbottom = true;");
        }
        if l.upper_unpegged {
            s.push_str(" dontpegtop = true;");
        }
        if l.secret {
            s.push_str(" secret = true;");
        }
        s.push_str(" }\n");
    }
    s.push('\n');

    for sd in &data.sidedefs {
        let _ = write!(s, "sidedef {{ sector = {};", sd.sector);
        for (key, tex) in [
            ("texturetop", &sd.upper),
            ("texturemiddle", &sd.middle),
            ("texturebottom", &sd.lower),
        ] {
            if !tex.is_empty() {
                let _ = write!(s, " {key} = \"{tex}\";");
            }
        }
        s.push_str(" }\n");
    }
    s.push('\n');

    for sec in &data.sectors {
        let _ = write!(
            s,
            "sector {{ heightfloor = {}; heightceiling = {}; texturefloor = \"{}\"; \
             textureceiling = \"{}\"; lightlevel = {};",
            sec.floor, sec.ceiling, sec.floor_tex, sec.ceil_tex, sec.light
        );
        if sec.special != 0 {
            let _ = write!(s, " special = {};", sec.special);
        }
        if sec.tag != 0 {
            let _ = write!(s, " id = {};", sec.tag);
        }
        s.push_str(" }\n");
    }
    s.push('\n');

    for t in things {
        let _ = write!(
            s,
            "thing {{ x = {}.0; y = {}.0; angle = {}; type = {};",
            t.x, t.y, t.angle, t.kind
        );
        // Only the skills this thing actually appears on are written —
        // UDMF's `doom` namespace defaults every `skillN` field to `false`
        // when absent, matching the convention every other boolean field in
        // this function already follows (`blocking`, `dontpegbottom`,
        // `dontpegtop`): state `true`, omit `false`.
        for (present, name) in [
            (t.skills.skill1, "skill1"),
            (t.skills.skill2, "skill2"),
            (t.skills.skill3, "skill3"),
            (t.skills.skill4, "skill4"),
            (t.skills.skill5, "skill5"),
        ] {
            if present {
                let _ = write!(s, " {name} = true;");
            }
        }
        s.push_str(" single = true; }\n");
    }

    s
}

#[cfg(test)]
mod tests {
    use crate::compile::compile;
    use crate::ir::Ir;
    use crate::tables::Tables;

    const TWO_ROOM: &str = r#"{ "seed":1, "grid":64, "theme":"tech_base",
      "rooms":[
        { "id":"a", "footprint":[[0,0],[0,256],[256,256],[256,0]],
          "floor":0, "ceiling":128, "light":160,
          "floor_tex":"FLOOR4_8", "ceil_tex":"CEIL3_5", "wall_tex":"STARTAN3",
          "things":[{ "kind":"player1_start", "at":[128,128], "angle":90 }] },
        { "id":"b", "footprint":[[320,0],[320,256],[576,256],[576,0]],
          "floor":0, "ceiling":128, "light":160,
          "floor_tex":"FLOOR4_8", "ceil_tex":"CEIL3_5", "wall_tex":"STARTAN3" }
      ],
      "portals":[{ "a":"a", "b":"b", "kind":"plain", "width":128, "at":[256,128] }] }"#;

    #[test]
    fn emission_starts_with_the_doom_namespace() {
        let ir = Ir::from_json(TWO_ROOM).expect("ir");
        let tables = Tables::load().expect("tables");
        let out = compile(&ir, &tables).expect("compiles");
        assert!(out.textmap.starts_with("namespace = \"doom\";"));
    }

    #[test]
    fn emission_never_contains_texture_scaling() {
        let ir = Ir::from_json(TWO_ROOM).expect("ir");
        let tables = Tables::load().expect("tables");
        let out = compile(&ir, &tables).expect("compiles");
        assert!(!out.textmap.contains("scalex"), "P9: no texture scaling");
        assert!(!out.textmap.contains("scaley"), "P9: no texture scaling");
    }

    #[test]
    fn the_same_ir_compiles_byte_identically_twice() {
        let ir = Ir::from_json(TWO_ROOM).expect("ir");
        let tables = Tables::load().expect("tables");
        let a = compile(&ir, &tables).expect("first");
        let b = compile(&ir, &tables).expect("second");
        assert_eq!(a.textmap, b.textmap, "S6: emission is deterministic");
    }

    #[test]
    fn a_default_thing_still_emits_all_five_skills_true() {
        // Pins the pre-existing behavior byte-for-byte: a thing with no
        // `skills` key must emit exactly what it always did.
        let ir = Ir::from_json(TWO_ROOM).expect("ir");
        let tables = Tables::load().expect("tables");
        let out = compile(&ir, &tables).expect("compiles");
        assert!(out.textmap.contains(
            "skill1 = true; skill2 = true; skill3 = true; skill4 = true; skill5 = true; \
             single = true;"
        ));
    }

    #[test]
    fn only_the_things_selected_skills_are_emitted() {
        let json = TWO_ROOM.replace(
            "\"angle\":90",
            "\"angle\":90, \"skills\": { \"skill1\": false, \"skill5\": false }",
        );
        let ir = Ir::from_json(&json).expect("ir");
        let tables = Tables::load().expect("tables");
        let out = compile(&ir, &tables).expect("compiles");
        assert!(!out.textmap.contains("skill1 = true"), "skill1 excluded");
        assert!(out.textmap.contains("skill2 = true"), "skill2 kept");
        assert!(out.textmap.contains("skill3 = true"), "skill3 kept");
        assert!(out.textmap.contains("skill4 = true"), "skill4 kept");
        assert!(!out.textmap.contains("skill5 = true"), "skill5 excluded");
        assert!(
            out.textmap.contains("single = true"),
            "single is unaffected"
        );
    }
}
