//! Always-error validation: rules a filled map-spec template must satisfy
//! that serde's deserialization alone cannot express — range coherence,
//! vocabulary resolution against [`crate::tables::Tables`], and bounds
//! sourced from the pinned engine. See `docs/map-spec.md`'s "Always errors"
//! list for the inventory this module implements; the enforcement-governed
//! set (internally-visible consistency `constraints.enforcement` toggles
//! between rejecting and recording) is a later stage's job, not this one's.
//!
//! Every check here is a small, independently named function taking the
//! parts of [`crate::spec::frontmatter::Frontmatter`] it needs (plus
//! [`crate::tables::Tables`] where a name must resolve against the
//! vocabulary), pushing zero or more [`crate::spec::Violation`]s onto a
//! shared `Vec`. [`always_errors`](crate::spec::validate::always_errors)
//! concatenates all of them. Findings are
//! collected, not first-error, per `docs/map-spec.md`: an author fixing a
//! seventeen-group document deserves the full list in one pass.

use crate::spec::body::Body;
use crate::spec::frontmatter::{
    AmmoPickups, AutoOr, Boss, Enforcement, Facing, Frontmatter, LightEffect,
};
use crate::spec::{Sacrifice, SpecError, Violation};
use crate::tables::Tables;

/// Runs every always-error rule against a parsed frontmatter and returns
/// every violation found; an empty vector means the document is clean.
///
/// `body` is accepted but unused here: secret trigger/reward/hint validation
/// already happens at body-parse time (`spec::body::parse`), so there is
/// nothing left for this pass to check against it. The parameter stays in
/// the signature because the enforcement-governed pass a later stage adds
/// needs it (`secrets.count` versus the number of prose `### Secret`
/// sections), and because a future always-error rule may too.
#[must_use]
pub fn always_errors(fm: &Frontmatter, _body: &Body, tables: &Tables) -> Vec<Violation> {
    let mut v = Vec::new();
    check_ranges(fm, &mut v);
    check_fractions(fm, &mut v);
    check_positives(fm, &mut v);
    check_domains(fm, tables, &mut v);
    check_slot(fm, tables, &mut v);
    check_starts(fm, tables, &mut v);
    check_priority(fm, &mut v);
    check_vocab(fm, tables, &mut v);
    check_placement_coherence(fm, &mut v);
    v
}

/// Pushes a violation at `path` if `min > max`. Shared by every
/// `MinMax`-style range pair and the two ad hoc
/// `combat.monsters[i]`/`aesthetics.lighting.{min,max}` fields that carry
/// the same rule without the `MinMax` wrapper.
fn check_min_max<T: PartialOrd + std::fmt::Display>(
    v: &mut Vec<Violation>,
    path: &str,
    min: &T,
    max: &T,
) {
    if min > max {
        v.push(Violation {
            path: path.to_string(),
            message: format!("min ({min}) must be <= max ({max})"),
        });
    }
}

/// Every declared range pair, per `docs/map-spec.md`'s "Always errors" list:
/// `min <= max` for every `MinMax` field in the template plus the two fields
/// that carry the same rule without the wrapper (`combat.monsters[i]`,
/// `aesthetics.lighting.{min,max}`, reported at `.min`).
fn check_ranges(fm: &Frontmatter, v: &mut Vec<Violation>) {
    check_min_max(v, "scale.rooms", &fm.scale.rooms.min, &fm.scale.rooms.max);
    check_min_max(
        v,
        "scale.sectors",
        &fm.scale.sectors.min,
        &fm.scale.sectors.max,
    );
    check_min_max(
        v,
        "scale.linedefs",
        &fm.scale.linedefs.min,
        &fm.scale.linedefs.max,
    );
    check_min_max(
        v,
        "scale.play_time_minutes",
        &fm.scale.play_time_minutes.min,
        &fm.scale.play_time_minutes.max,
    );
    check_min_max(
        v,
        "scale.vertical_range",
        &fm.scale.vertical_range.min,
        &fm.scale.vertical_range.max,
    );
    check_min_max(
        v,
        "progression.lifts.count",
        &fm.progression.lifts.count.min,
        &fm.progression.lifts.count.max,
    );
    check_min_max(
        v,
        "progression.teleports.count",
        &fm.progression.teleports.count.min,
        &fm.progression.teleports.count.max,
    );
    check_min_max(
        v,
        "progression.switches.count",
        &fm.progression.switches.count.min,
        &fm.progression.switches.count.max,
    );
    check_min_max(
        v,
        "progression.walkover_triggers.count",
        &fm.progression.walkover_triggers.count.min,
        &fm.progression.walkover_triggers.count.max,
    );
    check_min_max(
        v,
        "architecture.overlooks",
        &fm.architecture.overlooks.min,
        &fm.architecture.overlooks.max,
    );
    check_min_max(
        v,
        "combat.ambush.teleport_ambushes",
        &fm.combat.ambush.teleport_ambushes.min,
        &fm.combat.ambush.teleport_ambushes.max,
    );
    for (i, m) in fm.combat.monsters.iter().enumerate() {
        check_min_max(v, &format!("combat.monsters[{i}]"), &m.min, &m.max);
    }
    check_min_max(
        v,
        "vertical.stairs.flights",
        &fm.vertical.stairs.flights.min,
        &fm.vertical.stairs.flights.max,
    );
    check_min_max(
        v,
        "scenery.barrels.count",
        &fm.scenery.barrels.count.min,
        &fm.scenery.barrels.count.max,
    );
    check_min_max(
        v,
        "pacing.encounter_beats",
        &fm.pacing.encounter_beats.min,
        &fm.pacing.encounter_beats.max,
    );
    check_min_max(
        v,
        "pacing.rest_areas",
        &fm.pacing.rest_areas.min,
        &fm.pacing.rest_areas.max,
    );
    check_min_max(
        v,
        "aesthetics.lighting.min",
        &fm.aesthetics.lighting.min,
        &fm.aesthetics.lighting.max,
    );
}

/// Pushes a violation if `value` falls outside `0.0..=1.0`.
fn check_fraction(v: &mut Vec<Violation>, path: &str, value: f64) {
    if !(0.0..=1.0).contains(&value) {
        v.push(Violation {
            path: path.to_string(),
            message: format!("{value} must be within 0.0..=1.0"),
        });
    }
}

/// Every fraction field: `corridor_ratio`, `hitscanner_ratio`, `deaf_ratio`,
/// `outdoor_proportion`, `liquid.coverage`, `peak_position`.
fn check_fractions(fm: &Frontmatter, v: &mut Vec<Violation>) {
    check_fraction(
        v,
        "architecture.corridor_ratio",
        fm.architecture.corridor_ratio,
    );
    check_fraction(v, "combat.hitscanner_ratio", fm.combat.hitscanner_ratio);
    check_fraction(v, "combat.ambush.deaf_ratio", fm.combat.ambush.deaf_ratio);
    check_fraction(v, "flats.outdoor_proportion", fm.flats.outdoor_proportion);
    check_fraction(v, "flats.liquid.coverage", fm.flats.liquid.coverage);
    check_fraction(v, "pacing.peak_position", fm.pacing.peak_position);
}

/// Pushes a violation if `value` is not strictly greater than zero.
fn check_positive_f64(v: &mut Vec<Violation>, path: &str, value: f64) {
    if value <= 0.0 {
        v.push(Violation {
            path: path.to_string(),
            message: format!("{value} must be > 0.0"),
        });
    }
}

/// Pushes a violation if `value` is not strictly greater than zero.
fn check_positive_i32(v: &mut Vec<Violation>, path: &str, value: i32) {
    if value <= 0 {
        v.push(Violation {
            path: path.to_string(),
            message: format!("{value} must be > 0"),
        });
    }
}

/// Every strictly-positive field: the three difficulty scaling factors and
/// `arsenal.ammo.ratio` (`> 0.0`); `identity.grid`, `scale.size.{width,
/// height}`, `progression.lifts.max_travel`,
/// `vertical.{standard_ceiling,door_opening}`,
/// `vertical.stairs.{rise_per_step,tread_depth}` (`> 0`).
fn check_positives(fm: &Frontmatter, v: &mut Vec<Violation>) {
    check_positive_f64(v, "difficulty.scaling.easy", fm.difficulty.scaling.easy);
    check_positive_f64(v, "difficulty.scaling.medium", fm.difficulty.scaling.medium);
    check_positive_f64(v, "difficulty.scaling.hard", fm.difficulty.scaling.hard);
    check_positive_f64(v, "arsenal.ammo.ratio", fm.arsenal.ammo.ratio);

    check_positive_i32(v, "identity.grid", fm.identity.grid);
    check_positive_i32(v, "scale.size.width", fm.scale.size.width);
    check_positive_i32(v, "scale.size.height", fm.scale.size.height);
    check_positive_i32(
        v,
        "progression.lifts.max_travel",
        fm.progression.lifts.max_travel,
    );
    check_positive_i32(v, "vertical.standard_ceiling", fm.vertical.standard_ceiling);
    check_positive_i32(v, "vertical.door_opening", fm.vertical.door_opening);
    check_positive_i32(
        v,
        "vertical.stairs.rise_per_step",
        fm.vertical.stairs.rise_per_step,
    );
    check_positive_i32(
        v,
        "vertical.stairs.tread_depth",
        fm.vertical.stairs.tread_depth,
    );
}

/// Fixed-domain checks: `detail_level` (`1..=5`), `start_facing` degrees
/// (`<= 359`), the four lighting fields against the engine's own light
/// domain (`Tables::light_range`), and `spec_version` (`== 1`).
fn check_domains(fm: &Frontmatter, tables: &Tables, v: &mut Vec<Violation>) {
    if fm.spec_version != 1 {
        v.push(Violation {
            path: "spec_version".to_string(),
            message: format!("{} must be 1", fm.spec_version),
        });
    }

    if !(1..=5).contains(&fm.aesthetics.detail_level) {
        v.push(Violation {
            path: "aesthetics.detail_level".to_string(),
            message: format!("{} must be within 1..=5", fm.aesthetics.detail_level),
        });
    }

    if let Facing::Degrees(n) = fm.players.start_facing
        && n > 359
    {
        v.push(Violation {
            path: "players.start_facing".to_string(),
            message: format!("{n} must be <= 359 degrees"),
        });
    }

    let range = tables.light_range();
    for (label, value) in [
        ("base", fm.aesthetics.lighting.base),
        ("min", fm.aesthetics.lighting.min),
        ("max", fm.aesthetics.lighting.max),
        ("outdoor", fm.aesthetics.lighting.outdoor),
    ] {
        if !range.contains(&value) {
            v.push(Violation {
                path: format!("aesthetics.lighting.{label}"),
                message: format!(
                    "{value} is outside the engine's light domain {}..={}",
                    range.start(),
                    range.end()
                ),
            });
        }
    }
}

/// `identity.slot` must be `MAP` followed by exactly two ASCII digits
/// forming a number in `1..=tables.commercial_map_slots()`.
fn check_slot(fm: &Frontmatter, tables: &Tables, v: &mut Vec<Violation>) {
    let slot = &fm.identity.slot;
    let bound = tables.commercial_map_slots();
    let valid = slot
        .strip_prefix("MAP")
        .filter(|digits| digits.len() == 2 && digits.bytes().all(|b| b.is_ascii_digit()))
        .and_then(|digits| digits.parse::<u32>().ok())
        .is_some_and(|n| (1..=bound).contains(&n));
    if !valid {
        v.push(Violation {
            path: "identity.slot".to_string(),
            message: format!("`{slot}` must be `MAP01`..=`MAP{bound:02}` (`MAP%02d`, 1..={bound})"),
        });
    }
}

/// `players.coop_starts` and `players.dm_starts` must not exceed the
/// engine's own start-thing maxima.
fn check_starts(fm: &Frontmatter, tables: &Tables, v: &mut Vec<Violation>) {
    if fm.players.coop_starts > tables.max_coop_starts() {
        v.push(Violation {
            path: "players.coop_starts".to_string(),
            message: format!(
                "{} exceeds the engine's {} coop start maximum",
                fm.players.coop_starts,
                tables.max_coop_starts()
            ),
        });
    }
    if fm.players.dm_starts > tables.max_dm_starts() {
        v.push(Violation {
            path: "players.dm_starts".to_string(),
            message: format!(
                "{} exceeds the engine's {} deathmatch start maximum",
                fm.players.dm_starts,
                tables.max_dm_starts()
            ),
        });
    }
}

/// The six [`crate::spec::frontmatter::Priority`] variants, in the order
/// `constraints.priority` must contain all of exactly once.
const ALL_PRIORITIES: [crate::spec::frontmatter::Priority; 6] = [
    crate::spec::frontmatter::Priority::ProgressionCorrectness,
    crate::spec::frontmatter::Priority::PlayableBalance,
    crate::spec::frontmatter::Priority::SectorBudget,
    crate::spec::frontmatter::Priority::MonsterCounts,
    crate::spec::frontmatter::Priority::DetailLevel,
    crate::spec::frontmatter::Priority::PlayTime,
];

/// `constraints.priority` must be a total order over all six
/// [`crate::spec::frontmatter::Priority`] variants: each exactly once.
fn check_priority(fm: &Frontmatter, v: &mut Vec<Violation>) {
    for want in ALL_PRIORITIES {
        let count = fm
            .constraints
            .priority
            .iter()
            .filter(|&&p| p == want)
            .count();
        match count {
            1 => {}
            0 => v.push(Violation {
                path: "constraints.priority".to_string(),
                message: format!("missing `{want:?}`"),
            }),
            n => v.push(Violation {
                path: "constraints.priority".to_string(),
                message: format!("`{want:?}` appears {n} times, must appear exactly once"),
            }),
        }
    }
}

/// The `snake_case` name [`Tables::light_effect_special`] expects for a
/// [`LightEffect`] variant, matched by hand against its own
/// `#[serde(rename_all = "snake_case")]` mapping (see `frontmatter.rs`).
/// A drift guard: if a variant is ever added here without a matching
/// `data/engine.toml` entry, [`check_vocab`] reports it here rather than a
/// lookup silently resolving to `None` unnoticed.
fn light_effect_name(effect: LightEffect) -> &'static str {
    match effect {
        LightEffect::Blink => "blink",
        LightEffect::Flicker => "flicker",
        LightEffect::Glow => "glow",
        LightEffect::StrobeSlow => "strobe_slow",
    }
}

/// Every vocabulary-resolution rule from `docs/map-spec.md`'s "Vocabulary
/// resolution" section: content names (species, weapons, ammo pickups,
/// powerups, prop kinds, the theme, light effects, forbidden entries, key
/// names) must resolve against [`Tables`], and `progression.doors.lock_types`
/// must be a subset of `progression.keys`.
fn check_vocab(fm: &Frontmatter, tables: &Tables, v: &mut Vec<Violation>) {
    check_keys_and_locks(fm, tables, v);
    check_species(fm, tables, v);
    check_weapons_ammo_and_powerups(fm, tables, v);
    check_props_theme_effects_and_forbid(fm, tables, v);
}

/// `progression.keys[i]` and `progression.doors.lock_types[i]`: every key
/// name must resolve, and every lock type must also appear in
/// `progression.keys` (rule P24).
fn check_keys_and_locks(fm: &Frontmatter, tables: &Tables, v: &mut Vec<Violation>) {
    for (i, key) in fm.progression.keys.iter().enumerate() {
        if tables.locked_door_special(key).is_none() {
            v.push(Violation {
                path: format!("progression.keys[{i}]"),
                message: format!("`{key}` is not a known key name"),
            });
        }
    }

    for (i, lock) in fm.progression.doors.lock_types.iter().enumerate() {
        if tables.locked_door_special(lock).is_none() {
            v.push(Violation {
                path: format!("progression.doors.lock_types[{i}]"),
                message: format!("`{lock}` is not a known key name"),
            });
        } else if !fm.progression.keys.contains(lock) {
            v.push(Violation {
                path: format!("progression.doors.lock_types[{i}]"),
                message: format!("`{lock}` does not appear in progression.keys"),
            });
        }
    }
}

/// `combat.monsters[i].species` and `combat.boss` (when `Boss::Species`):
/// every species name must resolve against [`Tables::species`].
fn check_species(fm: &Frontmatter, tables: &Tables, v: &mut Vec<Violation>) {
    for (i, m) in fm.combat.monsters.iter().enumerate() {
        if tables.species(&m.species).is_none() {
            v.push(Violation {
                path: format!("combat.monsters[{i}].species"),
                message: format!("`{}` is not a known species", m.species),
            });
        }
    }
    if let Boss::Species(name) = &fm.combat.boss
        && tables.species(name).is_none()
    {
        v.push(Violation {
            path: "combat.boss".to_string(),
            message: format!("`{name}` is not a known species"),
        });
    }
}

/// `arsenal.weapons[i].name`, `arsenal.ammo.pickups.<name>` (when
/// `Explicit`), and `sustain.powerups[i].name`: every name must resolve
/// against [`Tables`].
fn check_weapons_ammo_and_powerups(fm: &Frontmatter, tables: &Tables, v: &mut Vec<Violation>) {
    for (i, w) in fm.arsenal.weapons.iter().enumerate() {
        if tables.thing_id(&w.name).is_none() {
            v.push(Violation {
                path: format!("arsenal.weapons[{i}].name"),
                message: format!("`{}` is not a known weapon", w.name),
            });
        }
    }

    if let AmmoPickups::Explicit(pickups) = &fm.arsenal.ammo.pickups {
        for name in pickups.keys() {
            if tables.ammo_pickup(name).is_none() {
                v.push(Violation {
                    path: format!("arsenal.ammo.pickups.{name}"),
                    message: format!("`{name}` is not a known ammo pickup"),
                });
            }
        }
    }

    for (i, p) in fm.sustain.powerups.iter().enumerate() {
        if tables.thing_id(&p.name).is_none() {
            v.push(Violation {
                path: format!("sustain.powerups[{i}].name"),
                message: format!("`{}` is not a known powerup", p.name),
            });
        }
    }
}

/// `scenery.light_sources.kinds[i]` / `scenery.decorations.kinds[i]` (when
/// `Given`), `aesthetics.theme`, `aesthetics.lighting.effects.allowed[i]`,
/// and `constraints.forbid[i]`: every name must resolve against [`Tables`],
/// or — for `forbid` — be one of the three non-species mechanic names.
fn check_props_theme_effects_and_forbid(fm: &Frontmatter, tables: &Tables, v: &mut Vec<Violation>) {
    if let AutoOr::Given(kinds) = &fm.scenery.light_sources.kinds {
        for (i, kind) in kinds.iter().enumerate() {
            if tables.prop(kind).is_none() {
                v.push(Violation {
                    path: format!("scenery.light_sources.kinds[{i}]"),
                    message: format!("`{kind}` is not a known prop"),
                });
            }
        }
    }
    if let AutoOr::Given(kinds) = &fm.scenery.decorations.kinds {
        for (i, kind) in kinds.iter().enumerate() {
            if tables.prop(kind).is_none() {
                v.push(Violation {
                    path: format!("scenery.decorations.kinds[{i}]"),
                    message: format!("`{kind}` is not a known prop"),
                });
            }
        }
    }

    if tables.texture("wall", &fm.aesthetics.theme).is_none() {
        v.push(Violation {
            path: "aesthetics.theme".to_string(),
            message: format!("`{}` is not a known theme", fm.aesthetics.theme),
        });
    }

    for (i, effect) in fm.aesthetics.lighting.effects.allowed.iter().enumerate() {
        let name = light_effect_name(*effect);
        if tables.light_effect_special(name).is_none() {
            v.push(Violation {
                path: format!("aesthetics.lighting.effects.allowed[{i}]"),
                message: format!("`{name}` has no sourced light-effect special"),
            });
        }
    }

    for (i, name) in fm.constraints.forbid.iter().enumerate() {
        let known = tables.species(name).is_some()
            || matches!(name.as_str(), "crusher" | "dark_maze" | "insta_death_pit");
        if !known {
            v.push(Violation {
                path: format!("constraints.forbid[{i}]"),
                message: format!(
                    "`{name}` is neither a known species nor `crusher`, `dark_maze`, or `insta_death_pit`"
                ),
            });
        }
    }
}

/// A powerup's `count == 0` iff its `placement == Placement::None`, and a
/// weapon's placement must never be `Placement::None` — self-contradictory
/// documents, not preferences.
fn check_placement_coherence(fm: &Frontmatter, v: &mut Vec<Violation>) {
    for (i, p) in fm.sustain.powerups.iter().enumerate() {
        let none_placement = p.placement == crate::spec::frontmatter::Placement::None;
        let zero_count = p.count == 0;
        if none_placement != zero_count {
            v.push(Violation {
                path: format!("sustain.powerups[{i}]"),
                message: format!(
                    "count ({}) and placement ({:?}) contradict each other: count 0 requires placement none, and vice versa",
                    p.count, p.placement
                ),
            });
        }
    }

    for (i, w) in fm.arsenal.weapons.iter().enumerate() {
        if w.placement == crate::spec::frontmatter::Placement::None {
            v.push(Violation {
                path: format!("arsenal.weapons[{i}].placement"),
                message: "a weapon must not be placed none".to_string(),
            });
        }
    }
}

/// Runs every enforcement-governed rule against a parsed frontmatter and
/// body, returning every finding as a [`Sacrifice`]; an empty vector means
/// nothing was sacrificed. Mode-independent: it always computes the full
/// finding list regardless of `fm.constraints.enforcement`. [`run`]
/// interprets the result under the document's actual mode — recording it
/// under `target`, converting it to errors under `strict`.
///
/// See `docs/map-spec.md`'s "Errors and the enforcement split" for the
/// governed set this implements: `secrets.count` versus the prose
/// `### Secret` sections, the lighting band, and locked-door coherence.
#[must_use]
pub fn governed(fm: &Frontmatter, body: &Body) -> Vec<Sacrifice> {
    let mut s = Vec::new();
    check_secrets_count(fm, body, &mut s);
    check_lighting_band(fm, &mut s);
    check_locked_doors(fm, &mut s);
    s
}

/// `secrets.count` versus the number of prose `### Secret` sections in the
/// body: a mismatch is sacrificed rather than rejected, since the prose is
/// the author's real intent and the declared count is a budget hint.
fn check_secrets_count(fm: &Frontmatter, body: &Body, s: &mut Vec<Sacrifice>) {
    let count = fm.secrets.count;
    let n = body.secrets.len();
    if count as usize != n {
        s.push(Sacrifice {
            path: "secrets.count".to_string(),
            target: count.to_string(),
            actual: format!("{n} prose secret sections"),
            message: format!(
                "sacrificed `secrets.count` ({count}) to the prose Secrets section ({n} entries) under `enforcement: target`"
            ),
        });
    }
}

/// `base`, `outdoor`, and `base + corridor_delta` each within
/// `[lighting.min, lighting.max]`: one finding per offender.
fn check_lighting_band(fm: &Frontmatter, s: &mut Vec<Sacrifice>) {
    let l = &fm.aesthetics.lighting;
    // `base` and `corridor_delta` are both unbounded, YAML-supplied `i32`s;
    // `check_domains` sources `base`'s own domain but never
    // `corridor_delta`'s, so a hostile document (both near `i32::MAX`) can
    // make their sum overflow. `saturating_add` rather than bare `+` keeps
    // this panic-free (the dev profile has overflow-checks on) and
    // wrap-free (release would otherwise silently wrap to a small, possibly
    // in-band, number): a saturated sum pins to `i32::MAX`/`i32::MIN`,
    // which is definitionally outside any sane `[min, max]` light band, so
    // it is correctly reported as a governed finding rather than either
    // panicking or silently passing.
    let sum = l.base.saturating_add(l.corridor_delta);
    let offenders: [(&str, i32, String); 3] = [
        ("aesthetics.lighting.base", l.base, l.base.to_string()),
        (
            "aesthetics.lighting.outdoor",
            l.outdoor,
            l.outdoor.to_string(),
        ),
        (
            "aesthetics.lighting.corridor_delta",
            sum,
            format!("base + corridor_delta = {sum}"),
        ),
    ];
    for (path, value, actual) in offenders {
        if value < l.min || value > l.max {
            s.push(Sacrifice {
                path: path.to_string(),
                target: format!("within [{}, {}]", l.min, l.max),
                message: format!(
                    "sacrificed `{path}` ({actual}) to the declared lighting band [{}, {}] under `enforcement: target`",
                    l.min, l.max
                ),
                actual,
            });
        }
    }
}

/// `locked_doors` versus `lock_types` coherence: fewer locked doors than
/// declared lock types means some lock type has no door to carry it.
fn check_locked_doors(fm: &Frontmatter, s: &mut Vec<Sacrifice>) {
    let locked_doors = fm.progression.locked_doors;
    let n = fm.progression.doors.lock_types.len();
    if (locked_doors as usize) < n {
        s.push(Sacrifice {
            path: "progression.locked_doors".to_string(),
            target: format!(">= {n} (one per lock type)"),
            actual: locked_doors.to_string(),
            message: format!(
                "sacrificed `progression.locked_doors` ({locked_doors}) to the {n} declared lock types under `enforcement: target`"
            ),
        });
    }
}

/// The combining entry point: collects [`always_errors`] and, under
/// `enforcement: strict`, converts every [`governed`] finding into a
/// [`Violation`] carrying its `message` too. If any violation exists
/// (always-error or converted-governed), returns `SpecError::Invalid` with
/// all of them; otherwise returns the sacrifices — empty under `strict` by
/// construction, since a nonempty governed finding would have become a
/// violation and taken the error path instead.
///
/// # Errors
///
/// Returns `SpecError::Invalid` if any always-error rule fires, or — under
/// `Enforcement::Strict` — if any enforcement-governed rule fires.
pub fn run(fm: &Frontmatter, body: &Body, tables: &Tables) -> Result<Vec<Sacrifice>, SpecError> {
    let mut violations = always_errors(fm, body, tables);
    let sacrifices = governed(fm, body);

    if fm.constraints.enforcement == Enforcement::Strict {
        violations.extend(sacrifices.iter().map(|s| Violation {
            path: s.path.clone(),
            message: s.message.clone(),
        }));
    }

    if violations.is_empty() {
        Ok(sacrifices)
    } else {
        Err(SpecError::Invalid(violations))
    }
}

#[cfg(test)]
mod tests {
    use crate::spec::Violation;
    use crate::tables::Tables;

    use super::{always_errors, governed, run};

    fn template_yaml() -> String {
        let text = include_str!("../../map-spec.template.md");
        crate::spec::split_frontmatter(text).unwrap().0
    }

    fn template_body() -> crate::spec::body::Body {
        let text = include_str!("../../map-spec.template.md");
        let (_, b) = crate::spec::split_frontmatter(text).unwrap();
        crate::spec::body::parse(&b).unwrap()
    }

    fn violations_for(patched_yaml: &str) -> Vec<Violation> {
        let fm = crate::spec::frontmatter::parse(patched_yaml).unwrap();
        let tables = Tables::load().unwrap();
        always_errors(&fm, &template_body(), &tables)
    }

    /// Replaces `from` with `to` in the template's YAML, asserting the
    /// replacement actually changed something. A `.replace()` whose `from`
    /// no longer matches the template (a typo, or drift after an edit to
    /// `map-spec.template.md`) silently returns the unmodified template —
    /// exactly the failure mode that turned up in the priority-reordering
    /// test during this task's own review (a 2-space-indent target against
    /// the template's real 4-space indent). Every test that patches the
    /// template string goes through this helper rather than calling
    /// `template_yaml().replace(..)` directly, so the guard cannot be
    /// forgotten on a new test.
    fn patched(from: &str, to: &str) -> String {
        let y = template_yaml().replace(from, to);
        assert_ne!(
            y,
            template_yaml(),
            "patch `{from}` -> `{to}` did not change the template"
        );
        y
    }

    #[test]
    fn the_shipped_template_has_no_always_errors() {
        assert_eq!(violations_for(&template_yaml()), vec![]);
    }

    #[test]
    fn an_inverted_room_range_fails_naming_scale_rooms() {
        let y = patched("rooms: { min: 8, max: 14 }", "rooms: { min: 15, max: 14 }");
        let v = violations_for(&y);
        assert!(v.iter().any(|v| v.path == "scale.rooms"), "got {v:?}");
    }

    #[test]
    fn a_room_range_at_the_threshold_passes() {
        let y = patched("rooms: { min: 8, max: 14 }", "rooms: { min: 14, max: 14 }");
        assert!(violations_for(&y).iter().all(|v| v.path != "scale.rooms"));
    }

    #[test]
    fn a_corridor_ratio_just_above_one_fails_and_one_exactly_passes() {
        let hi = patched("corridor_ratio: 0.3", "corridor_ratio: 1.01");
        assert!(
            violations_for(&hi)
                .iter()
                .any(|v| v.path == "architecture.corridor_ratio")
        );
        let at = patched("corridor_ratio: 0.3", "corridor_ratio: 1.0");
        assert!(
            violations_for(&at)
                .iter()
                .all(|v| v.path != "architecture.corridor_ratio")
        );
    }

    #[test]
    fn a_lock_type_absent_from_keys_fails_with_its_index() {
        let y = patched("keys: [blue_card, red_skull]", "keys: [blue_card]");
        let v = violations_for(&y);
        assert!(
            v.iter()
                .any(|v| v.path == "progression.doors.lock_types[1]"),
            "got {v:?}"
        );
    }

    #[test]
    fn a_lock_type_present_in_keys_passes() {
        assert!(
            violations_for(&template_yaml())
                .iter()
                .all(|v| !v.path.starts_with("progression.doors.lock_types"))
        );
    }

    #[test]
    fn an_unknown_species_fails_with_its_row_index() {
        let y = patched("species: pinky", "species: doggo");
        let v = violations_for(&y);
        assert!(
            v.iter().any(|v| v.path == "combat.monsters[3].species"),
            "got {v:?}"
        );
    }

    #[test]
    fn an_unresolvable_boss_species_fails_at_combat_boss() {
        let y = patched("boss: none", "boss: plaid_boss");
        let v = violations_for(&y);
        assert!(v.iter().any(|v| v.path == "combat.boss"), "got {v:?}");
    }

    #[test]
    fn a_theme_the_vocabulary_lacks_is_an_error() {
        let y = patched("theme: tech_base", "theme: hell");
        assert!(
            violations_for(&y)
                .iter()
                .any(|v| v.path == "aesthetics.theme")
        );
    }

    #[test]
    fn a_powerup_count_contradicting_its_placement_is_an_error() {
        let y = patched(
            "- { name: megasphere,      count: 0, placement: none }",
            "- { name: megasphere,      count: 1, placement: none }",
        );
        assert!(
            violations_for(&y)
                .iter()
                .any(|v| v.path.starts_with("sustain.powerups[2]"))
        );
    }

    #[test]
    fn a_powerup_count_and_placement_in_agreement_passes() {
        assert!(
            violations_for(&template_yaml())
                .iter()
                .all(|v| !v.path.starts_with("sustain.powerups[2]"))
        );
    }

    #[test]
    fn a_weapon_placed_none_is_an_error() {
        let y = patched(
            "name: rocket_launcher, placement: secret_only",
            "name: rocket_launcher, placement: none",
        );
        let v = violations_for(&y);
        assert!(
            v.iter().any(|v| v.path == "arsenal.weapons[3].placement"),
            "got {v:?}"
        );
    }

    #[test]
    fn a_weapon_placed_late_instead_of_none_passes() {
        let y = patched(
            "name: rocket_launcher, placement: secret_only",
            "name: rocket_launcher, placement: late",
        );
        assert!(
            violations_for(&y)
                .iter()
                .all(|v| v.path != "arsenal.weapons[3].placement")
        );
    }

    #[test]
    fn a_slot_beyond_the_commercial_bound_is_an_error_and_the_bound_passes() {
        let t = Tables::load().unwrap();
        let over = format!("MAP{:02}", t.commercial_map_slots() + 1);
        let y = patched("slot: MAP01", &format!("slot: {over}"));
        assert!(violations_for(&y).iter().any(|v| v.path == "identity.slot"));
        let last = format!("MAP{:02}", t.commercial_map_slots());
        let y = patched("slot: MAP01", &format!("slot: {last}"));
        assert!(violations_for(&y).iter().all(|v| v.path != "identity.slot"));
    }

    #[test]
    fn a_scaling_factor_of_zero_fails_and_a_small_positive_one_passes() {
        let zero = patched("easy: 0.55", "easy: 0.0");
        assert!(
            violations_for(&zero)
                .iter()
                .any(|v| v.path == "difficulty.scaling.easy")
        );
        let small = patched("easy: 0.55", "easy: 0.05");
        assert!(
            violations_for(&small)
                .iter()
                .all(|v| v.path != "difficulty.scaling.easy")
        );
    }

    #[test]
    fn an_identity_grid_of_zero_fails_and_a_positive_value_passes() {
        // `identity.grid` runs through `check_positive_i32`, distinct from
        // the f64 path `a_scaling_factor_of_zero_...` exercises above — the
        // two comparisons are separate functions and neither proves the
        // other fires.
        let zero = patched("grid: 64", "grid: 0");
        let v = violations_for(&zero);
        assert!(v.iter().any(|v| v.path == "identity.grid"), "got {v:?}");
        let positive = patched("grid: 64", "grid: 1");
        assert!(
            violations_for(&positive)
                .iter()
                .all(|v| v.path != "identity.grid")
        );
    }

    #[test]
    fn a_detail_level_of_zero_fails_and_one_passes() {
        let zero = patched("detail_level: 3", "detail_level: 0");
        assert!(
            violations_for(&zero)
                .iter()
                .any(|v| v.path == "aesthetics.detail_level")
        );
        let one = patched("detail_level: 3", "detail_level: 1");
        assert!(
            violations_for(&one)
                .iter()
                .all(|v| v.path != "aesthetics.detail_level")
        );
    }

    #[test]
    fn a_facing_of_360_degrees_fails_and_359_passes() {
        let over = patched("start_facing: east", "start_facing: 360");
        assert!(
            violations_for(&over)
                .iter()
                .any(|v| v.path == "players.start_facing")
        );
        let at = patched("start_facing: east", "start_facing: 359");
        assert!(
            violations_for(&at)
                .iter()
                .all(|v| v.path != "players.start_facing")
        );
    }

    #[test]
    fn a_light_base_beyond_the_engine_domain_fails_and_the_bound_passes() {
        let t = Tables::load().unwrap();
        let over = t.light_range().end() + 1;
        let y = patched("base: 160", &format!("base: {over}"));
        assert!(
            violations_for(&y)
                .iter()
                .any(|v| v.path == "aesthetics.lighting.base")
        );
        let at = *t.light_range().end();
        let y = patched("base: 160", &format!("base: {at}"));
        assert!(
            violations_for(&y)
                .iter()
                .all(|v| v.path != "aesthetics.lighting.base")
        );
    }

    #[test]
    fn every_allowed_light_effect_in_the_template_resolves() {
        // A drift-guard row: `LightEffect` is a closed Rust enum whose every
        // variant already has a `data/engine.toml` entry, so no valid YAML
        // patch can violate it today. This pins the passing side; the
        // check exists to catch a future variant added without a matching
        // table entry.
        assert!(
            violations_for(&template_yaml())
                .iter()
                .all(|v| !v.path.starts_with("aesthetics.lighting.effects.allowed"))
        );
    }

    #[test]
    fn spec_version_2_fails_and_1_passes() {
        let y = patched("spec_version: 1", "spec_version: 2");
        assert!(violations_for(&y).iter().any(|v| v.path == "spec_version"));
        assert!(
            violations_for(&template_yaml())
                .iter()
                .all(|v| v.path != "spec_version")
        );
    }

    #[test]
    fn coop_starts_beyond_the_maximum_fails_and_the_maximum_passes() {
        // `check_starts` has two independent if-blocks; the dm_starts test
        // below only exercises one of them.
        let t = Tables::load().unwrap();
        let over = t.max_coop_starts() + 1;
        let y = patched("coop_starts: 4", &format!("coop_starts: {over}"));
        assert!(
            violations_for(&y)
                .iter()
                .any(|v| v.path == "players.coop_starts")
        );
        // The template's own `coop_starts: 4` already sits at
        // `tables.max_coop_starts()`, so the boundary-passing case is the
        // unpatched template itself — patching to the same value would be
        // a no-op the `patched` guard correctly rejects.
        assert!(
            violations_for(&template_yaml())
                .iter()
                .all(|v| v.path != "players.coop_starts")
        );
    }

    #[test]
    fn dm_starts_beyond_the_maximum_fails_and_the_maximum_passes() {
        let t = Tables::load().unwrap();
        let over = t.max_dm_starts() + 1;
        let y = patched("dm_starts: 0", &format!("dm_starts: {over}"));
        assert!(
            violations_for(&y)
                .iter()
                .any(|v| v.path == "players.dm_starts")
        );
        let at = t.max_dm_starts();
        let y = patched("dm_starts: 0", &format!("dm_starts: {at}"));
        assert!(
            violations_for(&y)
                .iter()
                .all(|v| v.path != "players.dm_starts")
        );
    }

    #[test]
    fn a_duplicate_priority_entry_fails_and_a_reordering_passes() {
        let y = patched("    - play_time", "    - monster_counts");
        assert!(
            violations_for(&y)
                .iter()
                .any(|v| v.path == "constraints.priority")
        );
        let reordered = patched(
            "    - monster_counts\n    - detail_level\n",
            "    - detail_level\n    - monster_counts\n",
        );
        assert!(
            violations_for(&reordered)
                .iter()
                .all(|v| v.path != "constraints.priority")
        );
    }

    #[test]
    fn an_unknown_key_name_fails_with_its_index_and_a_reordering_passes() {
        let y = patched(
            "keys: [blue_card, red_skull]",
            "keys: [blue_card, plaid_key]",
        );
        let v = violations_for(&y);
        assert!(
            v.iter().any(|v| v.path == "progression.keys[1]"),
            "got {v:?}"
        );
        let reordered = patched(
            "keys: [blue_card, red_skull]",
            "keys: [red_skull, blue_card]",
        );
        assert!(
            violations_for(&reordered)
                .iter()
                .all(|v| !v.path.starts_with("progression.keys"))
        );
    }

    #[test]
    fn an_unknown_weapon_name_fails_and_a_known_one_passes() {
        let y = patched(
            "name: chaingun,        placement: mid",
            "name: plaid_gun,       placement: mid",
        );
        let v = violations_for(&y);
        assert!(
            v.iter().any(|v| v.path == "arsenal.weapons[1].name"),
            "got {v:?}"
        );
        let known = patched(
            "name: chaingun,        placement: mid",
            "name: plasma_rifle,    placement: mid",
        );
        assert!(
            violations_for(&known)
                .iter()
                .all(|v| v.path != "arsenal.weapons[1].name")
        );
    }

    #[test]
    fn an_unknown_ammo_pickup_name_fails_and_a_known_one_passes() {
        let y = patched(
            "pickups: auto            # auto (derived from ratio) | explicit counts per pickup type",
            "pickups: { shells: 4, plaid_ammo: 2 }",
        );
        let v = violations_for(&y);
        assert!(
            v.iter()
                .any(|v| v.path == "arsenal.ammo.pickups.plaid_ammo"),
            "got {v:?}"
        );
        let known = patched(
            "pickups: auto            # auto (derived from ratio) | explicit counts per pickup type",
            "pickups: { shells: 4, rocket: 2 }",
        );
        assert!(
            violations_for(&known)
                .iter()
                .all(|v| !v.path.starts_with("arsenal.ammo.pickups."))
        );
    }

    #[test]
    fn an_unknown_powerup_name_fails_and_a_known_one_passes() {
        let y = patched(
            "name: radsuit,         count: 1, placement: mid",
            "name: plaid_power,     count: 1, placement: mid",
        );
        let v = violations_for(&y);
        assert!(
            v.iter().any(|v| v.path == "sustain.powerups[3].name"),
            "got {v:?}"
        );
        let known = patched(
            "name: radsuit,         count: 1, placement: mid",
            "name: invisibility,    count: 1, placement: mid",
        );
        assert!(
            violations_for(&known)
                .iter()
                .all(|v| v.path != "sustain.powerups[3].name")
        );
    }

    #[test]
    fn an_unknown_prop_kind_fails_and_a_known_one_passes() {
        let y = patched(
            "kinds: auto              # auto (theme-derived) | explicit list",
            "kinds: [floor_lamp, plaid_lamp]",
        );
        let v = violations_for(&y);
        assert!(
            v.iter().any(|v| v.path == "scenery.light_sources.kinds[1]"),
            "got {v:?}"
        );
        let known = patched(
            "kinds: auto              # auto (theme-derived) | explicit list",
            "kinds: [floor_lamp, techno_lamp]",
        );
        assert!(
            violations_for(&known)
                .iter()
                .all(|v| !v.path.starts_with("scenery.light_sources.kinds"))
        );
    }

    #[test]
    fn an_unknown_decoration_kind_fails_and_a_known_one_passes() {
        // `scenery.light_sources.kinds` and `scenery.decorations.kinds` are
        // checked by two separate `if let AutoOr::Given(..)` blocks in
        // `check_props_theme_effects_and_forbid`; the light-sources test
        // above does not exercise this one.
        let y = patched("    kinds: auto\n", "    kinds: [candelabra, plaid_deco]\n");
        let v = violations_for(&y);
        assert!(
            v.iter().any(|v| v.path == "scenery.decorations.kinds[1]"),
            "got {v:?}"
        );
        let known = patched(
            "    kinds: auto\n",
            "    kinds: [candelabra, techno_lamp]\n",
        );
        assert!(
            violations_for(&known)
                .iter()
                .all(|v| !v.path.starts_with("scenery.decorations.kinds"))
        );
    }

    #[test]
    fn an_unknown_forbid_entry_fails_and_a_known_species_passes() {
        let y = patched(
            "forbid: [archvile, crusher, dark_maze, insta_death_pit]",
            "forbid: [plaid_hazard, crusher, dark_maze, insta_death_pit]",
        );
        let v = violations_for(&y);
        assert!(
            v.iter().any(|v| v.path == "constraints.forbid[0]"),
            "got {v:?}"
        );
        let known = patched(
            "forbid: [archvile, crusher, dark_maze, insta_death_pit]",
            "forbid: [cyberdemon, crusher, dark_maze, insta_death_pit]",
        );
        assert!(
            violations_for(&known)
                .iter()
                .all(|v| v.path != "constraints.forbid[0]")
        );
    }

    #[test]
    fn the_shipped_template_yields_no_sacrifices() {
        // template is enforcement: target; a clean doc must produce zero findings
        let fm = crate::spec::frontmatter::parse(&template_yaml()).unwrap();
        let tables = Tables::load().unwrap();
        assert_eq!(run(&fm, &template_body(), &tables).unwrap(), vec![]);
    }

    #[test]
    fn a_count_mismatch_under_target_is_a_sacrifice_naming_secrets_count() {
        let y = patched(
            "count: 3                   #",
            "count: 4                   #",
        );
        let fm = crate::spec::frontmatter::parse(&y).unwrap();
        let tables = Tables::load().unwrap();
        let s = run(&fm, &template_body(), &tables).unwrap();
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].path, "secrets.count");
        assert!(s[0].message.contains("sacrificed"), "got: {}", s[0].message);
    }

    #[test]
    fn the_same_mismatch_under_strict_is_an_error() {
        let y = template_yaml()
            .replace(
                "count: 3                   #",
                "count: 4                   #",
            )
            .replace("enforcement: target", "enforcement: strict");
        assert_ne!(y, template_yaml());
        let fm = crate::spec::frontmatter::parse(&y).unwrap();
        let tables = Tables::load().unwrap();
        let err = run(&fm, &template_body(), &tables).unwrap_err();
        let crate::spec::SpecError::Invalid(v) = err else {
            panic!()
        };
        assert!(v.iter().any(|v| v.path == "secrets.count"));
    }

    #[test]
    fn a_base_light_outside_the_band_is_governed_and_at_the_edge_is_not() {
        let out = patched("base: 160", "base: 209");
        let fm = crate::spec::frontmatter::parse(&out).unwrap();
        assert!(
            governed(&fm, &template_body())
                .iter()
                .any(|s| s.path == "aesthetics.lighting.base")
        );
        let edge = patched("base: 160", "base: 208");
        let fm = crate::spec::frontmatter::parse(&edge).unwrap();
        assert!(
            governed(&fm, &template_body())
                .iter()
                .all(|s| s.path != "aesthetics.lighting.base")
        );
    }

    #[test]
    fn a_base_and_corridor_delta_that_would_overflow_their_sum_is_governed_not_a_panic() {
        // Both near `i32::MAX`: `base + corridor_delta` overflows a bare
        // `i32` addition. This must report a governed finding at
        // `aesthetics.lighting.corridor_delta`, not panic (dev profile has
        // overflow-checks on) or silently wrap to some other value.
        let y = template_yaml()
            .replace("base: 160", "base: 2000000000")
            .replace("corridor_delta: -16", "corridor_delta: 2000000000");
        assert_ne!(y, template_yaml());
        let fm = crate::spec::frontmatter::parse(&y).unwrap();
        let findings = governed(&fm, &template_body());
        assert!(
            findings
                .iter()
                .any(|s| s.path == "aesthetics.lighting.corridor_delta"),
            "got {findings:?}"
        );
    }

    #[test]
    fn fewer_locked_doors_than_lock_types_is_governed() {
        let y = patched("locked_doors: 2", "locked_doors: 1");
        let fm = crate::spec::frontmatter::parse(&y).unwrap();
        assert!(
            governed(&fm, &template_body())
                .iter()
                .any(|s| s.path == "progression.locked_doors")
        );
    }

    #[test]
    fn a_governed_finding_and_an_always_error_both_surface_under_strict() {
        let y = template_yaml()
            .replace("locked_doors: 2", "locked_doors: 1")
            .replace("corridor_ratio: 0.3", "corridor_ratio: 1.5")
            .replace("enforcement: target", "enforcement: strict");
        assert_ne!(y, template_yaml());
        let fm = crate::spec::frontmatter::parse(&y).unwrap();
        let tables = Tables::load().unwrap();
        let crate::spec::SpecError::Invalid(v) = run(&fm, &template_body(), &tables).unwrap_err()
        else {
            panic!()
        };
        assert!(v.iter().any(|v| v.path == "progression.locked_doors"));
        assert!(v.iter().any(|v| v.path == "architecture.corridor_ratio"));
    }
}
