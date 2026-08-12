# crustygen — known gaps and carried decisions

State as of the compiler's completion: IR → validated UDMF `TEXTMAP` → PWAD →
reassembles through crustywad, plus the layer-4 verifier that re-checks the
emitted map (`src/check`, `crustygen-check` — see `docs/check.md`). 475 tests
(439 lib + 7 check_adversarial + 14 check_cli + 4 check_conformance + 1
first_map + 6 golden_textmap + 3 spec_documents + 1 walking_skeleton), plus a
separately-run `#[ignore]`d golden-regeneration generator not included in that
count. This file records what is deliberately absent, what is known-fragile,
and the decisions a future contributor would otherwise have to re-derive.

## Not implemented, by design

The compiler covers structural invariants S1–S6 and playability rules P2,
P3, P4, P7, P8, P9, P11, P13, P14, P19, P24, P25. **P1** is retired — see
`rules.rs`'s module doc and `CompileError::PortalNoHeadroom`, and the gap
entry below. The layer-4 verifier (`src/check`, `docs/check.md`)
independently re-derives those same twelve from the *emitted* map, as
`V-P2`…`V-P25`, and adds `V-P20`.

Deliberately absent, deferred to the next stage: **P5** (lifts), **P6**
(monster mobility), **P10** (clean vertical tiling), **P12** (sky
coherence), **P15** (teleport pairing), **P16**/**P17** (liquids and damage
survivability), **P18** (secret accounting), **P20** (pickup accessibility —
compiler-side only; the verifier covers it, below), **P21** (light sources),
**P22** (hanging decorations), **P23** (barrel safety).

**P20's per-pickup check now exists, at layer 4 rather than in the compiler.**
`check::invariants::check_prop_embedding` measures every collectible against
every blocking prop's radius, and `check_pickup_reachability` requires each
collectible's sector to be one the verifier's own key-aware flood actually
reached. The two halves land differently against the compiler, which still has
no P20 pass of its own: the **embedding** half it covers nowhere at all, while
the **reachability** half it covers only while its own P7 runs — a map with no
player start or no exit runs neither check compile-side. The verifier has no
such hole: its flood either runs or reports a hard `V-P7` finding (see the P7
entry), so the gap never becomes silence there.

Where P7 does run, its coverage check (every room forward-reached from the
start) subsumes P20's "every pickup is reachable": a thing carries its own `at`
position, but a room compiles to one sector with a single, uniform floor, so
reaching the room makes every pickup inside it reachable regardless of where in
the room it sits. That argument is also what the verifier's own reachability
half rests on — it judges sectors, not positions — and it is what stops holding
once intra-room verticality exists: the day a room stops being one flat sector,
both layers need the position and not just the sector.

**P2 (headroom) is now fully covered.** `compile::things::place_things` checks
every room's headroom against the player's own height once per room,
regardless of whether that room places any things at all, in addition to the
existing per-thing check for anything taller than the player. Previously the
check only ran inside the per-thing loop, so a room with no things skipped it
entirely and an empty corridor too short for the player to stand in compiled
clean; `things::tests::p2_an_empty_room_too_short_for_the_player_is_rejected`
pins the fix. P25 (start clearance) is fully covered by the same code path,
since every player start is itself a thing and always goes through the
identical clearance/headroom/overlap checks.

**P18's mechanism exists; its counting rule does not.** `Room::secret`
(`compile::sectors::resolve_secret_specials`) gives a room a high-level way to
carry the sourced secret sector special (`Tables::secret_sector_special`)
instead of requiring an author to write the raw number into `Room::special` —
the two are mutually exclusive, rejected at parse time
(`IrError::SecretWithExplicitSpecial`) rather than resolved by silent
precedence. P18's actual *rule* — "the number of secret sectors equals
`secrets.count`" — is still absent **from the compiler**, and always will be:
`secrets.count` is a map-spec concept with no representation in this IR, so
that check belongs at a stage that reads the map-spec. It now exists there, as
the verifier's `secrets.count` conformance row (`check::conform`, exact count
against `MapStats::secret_sectors`, pinned by
`check_conformance::a_wrong_secret_count_fails_its_row`). Note the layer
difference: that row is a `Fail` verdict in a report, not a finding, so it
does not by itself fail a run — grading a spec violation is
`constraints.enforcement`'s call (`docs/design.md` §9), not the checker's.

Still absent: the conformance **report** (issue #3) — `docs/design.md` §8.1's
`report.md`, the rendered deliverable of a run. Its *content* now exists:
`check::run` returns a `CheckReport` carrying the findings, the spec-vs-actual
rows, the tag manifest and the map stats, and `crustygen-check` prints all of
it as plain lines; what is unwritten is the Markdown table, the sacrifice list
under `constraints.priority`, and any wiring that emits a file alongside the
WAD. (The verifier itself — `docs/design.md` §8 layer 4 — shipped with issue
#2 and is no longer absent, as did the packer, `pack::pack_udmf` and
`pack::pack_udmf_with_nodes`, and an authored map,
`tests/fixtures/entrada_base.json`, built into `maps/entrada.wad`.)
Specials for lifts, teleports, and liquid sector effects, monster
`spawnhealth`, health/armor pickup amounts and caps, the gore prop set, and
the `ML_BLOCKMONSTERS`/`ML_SOUNDBLOCK` linedef flags are all **sourced and
accessible** but nothing emits any of them yet. Doors, exits
(`compile::exits`), and the secret sector special are wired end to end.

**The map-spec parser exists now, and deliberately stops at parsing.**
`src/spec` turns a filled `map-spec.template.md` copy into a typed
`Spec`/`SpecDocument` (see `docs/map-spec.md`), but it records nothing about
the deeper conflicts `docs/design.md` §5.1 expects — `hitscanner_ratio`
feasibility against the per-species min/max ranges, `sectors.max` against
`detail_level` — because those belong to the resolver, not to a stage that
only reads the spec. **The verifier's arrival does not close this.**
`check::conform` judges a spec against a *built map* — `combat.hitscanner_ratio`
becomes an `Info` row carrying its delta — which is a different question from
whether the spec's own numbers are mutually satisfiable before any map exists.
Nothing checks that today: `Tables::hitscan` classifies species by hitscan
behavior and makes the feasibility check cheap to write, and
`aesthetics.detail_level` is only range-validated (`spec::validate`), never
compared against `scale.sectors`.
`map-spec.template.md` itself ships **filled**, not blank, for the reasons
`docs/map-spec.md` gives: a blank template could neither parse nor be
tested, so the filled one is simultaneously the contract, the
documentation, and a parseable artifact.

## Known gaps

**The orphan-sidedef code path is exercised, not merely documented.** It
fires when an opening consumes a wall end to end, so the reusable sidedef
`split_wall_for_opening` sets aside is never taken. `tests/fixtures/entrada_base.json`
reaches it twice — its `armory` room is only 128 units tall along the wall its
`start`-facing portal opens (`width: 128` against a wall exactly 128 units
long), so the portal consumes that whole wall and leaves `armory`'s original
sidedef record unreferenced. Confirmed directly: the compiled map carries 105
sidedef records but only 103 are ever named by a linedef's `front`/`back`
(81/79 before the door-thickness/alcove redesign, 93/91 before the `cache`
secret room was added — each change adds sidedefs, and the orphan count stays
at exactly two throughout, still `armory`'s own two end-to-end-consumed walls,
unaffected by either change).

**A drop over one step is one-way, and P7's flood now checks the map is
still finishable.** `P_TryMove` caps the climb (`tmfloorz - thing->z >
24*FRACUNIT`) and leaves falling unrestricted, so a room reachable only by
falling into it cannot be left the same way. The retired P1 forbade this
by accident; the replacement
(`CompileError::PortalNoHeadroom`) deliberately allows it, because 37.77% of
passable two-sided lines across DOOM, DOOM2, TNT, and PLUTONIA exceed the old
cap and 62.5% of those are permanent static drops. Verifying the player can
still finish is **P7** (`src/reach.rs`): a forward/backward search over
states `(sector, keys-held)` rejects both an unfinishable map (no state
reaches an exit) and a stranding one (a state the player can reach but can no
longer reach an exit from) at compile time. Measurement:
`docs/measurements/verticality-corpus.md`. One deliberate hole remains
compile-side: P7 runs only when the map has a player 1 start and at least one
exit, and passes vacuously otherwise — a missing start or exit is a different
defect, one this rule does not claim to catch. **Layer 4 closes that hole for
an emitted map.** `check::flood::run_flood` has no "elsewhere" to defer to: no
player 1 start, a start that resolves to no sector, and no exit line are each
a hard `V-P7` Error there rather than a silent pass
(`check_adversarial::removing_the_player_start_is_a_hard_error_not_a_vacuous_pass`
pins it). The compiler-side hole stands as recorded — the verifier runs on a
built WAD, so it catches the defect one stage later, not instead.

**P8 has no sky exception.** `r_segs.c` sets `worldtop = worldhigh` when both
sectors' `ceilingpic == skyflatnum`, so a sky-to-sky boundary draws no upper
and needs none — 60.3% of the corpus's absent uppers are exactly this
case — legitimately so. crustygen emits no sky flat, so no fixture can reach
it and the check is deliberately unwritten rather than guessed at — in both
`rules::check_missing_textures` and the verifier's independent
`check::invariants::check_textures` (V-P8), for the same reason. Required
before sky is added.

## Decisions that look wrong without their reason

**P24 is stricter than the engine about key kinds, and P7 is not.**
`EV_VerticalDoor` (pinned `p_doors.c:371-403`) opens a colour's lock for either
the card or the skull — `!p->cards[it_bluecard] && !p->cards[it_blueskull]`
rejects the move only if *neither* is held. `reach.rs` interns lock classes by
colour to match: `graph_from_compiled`'s keyed-special lookup is deliberately
many-to-one, so either key thing of a colour satisfies a `Door` edge's lock.
`rules::check_key_lock_coherence` (P24) does not — it compares the authored
lock string (`Portal::lock`, e.g. `"blue_card"`) against placed thing kinds by
exact string equality. A map that locks a portal `"blue_card"` while placing
only a `blue_skull` thing is therefore genuinely finishable — the skull opens
the door in the engine, and P7 passes it — while P24 still fails it, because
the string `"blue_card"` names a thing that never appears. This is deliberate,
not a bug in either rule: P24 polices *authored intent* (you named a card,
place a card), and P7 polices the *engine's actual behavior* (either key of the
colour works). Recorded here so a future "fix" does not make one rule agree
with the other at the cost of disagreeing with the engine or with the author's
stated intent.

The verifier's own V-P24 (`check::flood::check_key_lock_coherence`) sits on the
engine side of that split, and had no choice: a linedef's emitted `special` is
`26`, which names the *colour class*, not which key kind the author wrote. It
therefore checks class-level coherence — every lock class present has a key of
that colour placed, every placed key opens some door — and the authored-intent
form stays the compile-side rule's job, since the intent is only legible in the
IR. The two are not redundant and are not in conflict; they check different
statements at different layers.

**The crate's own lints are `warn`, and CI is what makes them fatal.**
`Cargo.toml` sets `clippy::all` and `clippy::pedantic` to `warn`, not `deny`,
so a local `cargo clippy` stays readable while you work. The strictness lives
in `.github/workflows/ci.yml` as `cargo clippy --all-targets -- -D warnings`,
on the invocation rather than in a global `RUSTFLAGS` — a global flag would
apply to everything cargo compiles on the way here, not just this crate. The
same split is why `cargo doc` runs under `RUSTDOCFLAGS: -D warnings` in CI and
plainly in local use.

This entry replaces a former gap reading "crustygen runs in no CI", which was
true only while the package lived as a subdirectory of the crustywad worktree,
where the parent's `cargo fmt --all` and `cargo clippy --workspace` could not
reach it because it declares its own `[workspace]`.

**Golden fixtures are pinned to LF, and they have to be.**
`.gitattributes` marks `*.textmap` and `*.json` as `-text` and `*.wad` as
`binary`. `emit_textmap` writes `\n` unconditionally, and both
`tests/golden_textmap.rs` and `tests/first_map.rs` compare compiler output to
a committed artifact **byte for byte** — so any newline translation on
checkout breaks the comparison, and the WAD drift guard would compare
corrupted bytes.

This is not hypothetical. The repository's very first CI run failed three
tests at once on `windows-latest` for exactly this reason: git checked the
goldens out as CRLF while the compiler emitted LF. Nothing on Linux or macOS
could have caught it, and the package had no CI at all until the day it became
its own repository — the first thing CI did was find a bug that had been
latent since the golden fixtures were introduced.

The spec Markdown artifacts (`map-spec.template.md`, `tests/fixtures/*.spec.md`)
joined the pin for a subtler variant of the same failure, again caught only by
Windows CI (PR #17): the parser itself is CRLF-tolerant, but the CRLF-identity
test transforms the `include_str!`'d template with `\n` -> `\r\n`, and on a
CRLF checkout that doubles into `\r\r\n` — the stray `\r` survives
`str::lines` and corrupts every YAML key.

**Entrada keeps every drop climbable, and the compiler now enforces that it
must.** `key_room` sits 16 units below `hub` and `exit_hall` 16 above
`vault` — both within `max_step_height`, so every descent can be reversed. A
drop beyond one step is still legal in general (see the one-way-drop gap
above); it matters here because `key_room` is a dead end holding the blue
card, the only key for the `combat` <-> `vault` door, so any deeper drop would
strand the player. An earlier revision put it at −32, which compiled clean,
passed every test, and was **unfinishable** — the player drops in, takes the
key, and cannot climb the 32 units back out (`P_TryMove` rejects a step over
24). That shipped defect is exactly what **P7** exists to catch —
`rules::tests::p7_a_key_in_a_one_way_pit_is_unfinishable_and_names_the_pit`
regresses it directly — so the constraint on this fixture is now enforced by
the compiler rather than resting on authoring discipline alone; a future
height change to a dead-end room fails the build if it strands the player,
rather than waiting to be caught by hand.

**Entrada's `cache` room is the one place `Room::secret` is exercised.**
Doom counts secrets by sectors carrying the secret special, so before that
room existed the map reported "secrets 0%" at the intermission no matter how
it was played — correct behavior with nothing to find, but indistinguishable
from a broken secret mechanism. The room is optional, off `combat`'s north
wall, and `tests/first_map.rs` pins that exactly one emitted sector carries
`Tables::secret_sector_special`. This exercises P18's *mechanism*; P18's
counting rule lives at layer 4 as a conformance row, never in the compiler
(see above).

**A secret room's portal thresholds also carry `ML_SECRET`, and that is the
only feedback vanilla gives.** Verified against the pinned engine: entering a
secret sector runs `p_spec.c`'s `P_PlayerInSpecialSector` case 9, whose entire
body is `player->secretcount++; sector->special = 0;` — **no sound and no
message**. The "a secret is revealed" cue belongs to Boom/MBF and the ZDoom
family, not to vanilla, and the automap gives a secret *sector* no treatment
at all. What vanilla does offer is `ML_SECRET` (32, `doomdata.h`: "In AutoMap:
don't map as two sided: IT'S A SECRET!") on a *linedef*: `am_map.c`'s
`AM_drawWalls` draws such a line in `WALLCOLORS` rather than falling through
to the floor/ceiling-difference colors that reveal a room beyond, so the
opening reads as solid wall on the automap.
`compile::portals::mark_secret_thresholds` sets it on every threshold of a
portal whose two rooms differ in secrecy — both thresholds of a plain
portal's passage, and all up to four of a door chain's, since a door's
alcove thresholds are built by a different code path than its own faces.
Keyed on the secrecy *difference*, so a portal between two secret rooms is
left unmarked; there is nothing to conceal there. Note the flag is purely
cosmetic — nothing about movement, sight, or world rendering changes — and
`SECRETWALLCOLORS` is itself `#define SECRETWALLCOLORS WALLCOLORS`, so even
vanilla's cheat-mode branch draws no distinct color.

**A locked door's key trim goes on the ALCOVE jambs, never on the door's own
track.** `DOORBLU`/`DOORRED`/`DOORYEL` (8x128) and their `2` variants
(16x128) are vertical trim strips, the same shape as `DOORTRAK` — which is
exactly why they are easy to misapply to the track. They belong on the trim a
player faces walking *up* to the door; the door's track stays the theme's
`door_track` (DOORTRAK) unconditionally, so a custom texture WAD can override
it as the one intended knob. Card versus skull is a measured convention, not
a guess: across the four id/Final Doom IWADs, restricted to maps holding
exactly one key of a colour, the plain name accompanies a keycard 117 times
to 18, and the `2` variant a skull key 92 to 26, in the same direction for
all three colours. Full tally and method in `vocabulary.toml`'s `[key_trim]`.

**A locked portal that declares no alcoves gets no key trim at all.** The trim
has nowhere to live — the door's own track is deliberately excluded (above),
and a portal with `alcove_near`/`alcove_far` both absent has no other surface
in its chain. Such a door is functionally correct and visually unmarked. This
is left as an authoring responsibility rather than a rejection, because
requiring an alcove would forbid the legal minimum-thickness door the
wall-thickness model is built around. Worth a playability rule the day one
exists for "a locked door is visually identifiable".

**Only the exit switch is texture-aligned; nothing else is.**
`compile::exits` sets the switch sidedef's `offsetx` so the texture is
centred on the exit line — `(switch_width - width) / 2`, from
`vocabulary.toml`'s measured `switch_width`. Every other sidedef in the map
is emitted with `offsetx` unset, i.e. 0. That is fine for a texture whose
width divides the wall it sits on and wrong in general: Doom derives a
texture's horizontal position from `offsetx` plus the distance along the line
from its start vertex, so a run of collinear linedefs (a wall split by a
portal opening, say) needs offsets that *accumulate* along the run rather
than restarting at each piece. Proper alignment is a separate piece of work;
what exists here is the narrowest fix for the one surface a playtest showed
reading wrong.

**Two map artifacts ship, and only one of them loads in a vanilla engine.**
`maps/entrada.wad` is the **UDMF** build — `MAP01`, `TEXTMAP`, `ZNODES`,
`ENDMAP` — and needs a ZDoom-family port (GZDoom, Odamex, Eternity).
`maps/entrada_doom.wad` is the binary **Doom-format** twin, carrying real
`THINGS` through `BLOCKMAP`, and is the one to load in Chocolate Doom, DOSBox,
or anything else vanilla-accurate.

Loading the UDMF build in a vanilla port does not report a helpful error; it
dies with `W_LumpLength: <n> >= numlumps`. The reason is `P_SetupLevel`, which
addresses a map's data as **fixed offsets from the marker** (`doomdata.h`'s
`ML_THINGS` = 1 through `ML_BLOCKMAP` = 10) rather than by searching for lump
names. A UDMF group supplies only three lumps after its marker, so the
`BLOCKMAP` slot resolves past the end of the directory. Observed directly with
Chocolate Doom 3.1.1 over `DOOM2.WAD` (2,919 lumps): `MAP01` landed at index
2919, and the engine asked for 2919 + 10 = **2929** against a `numlumps` of
2923.

Both artifacts are produced from the same compiled output — `pack::pack_udmf`
for the un-noded bytes, `pack::pack_udmf_with_nodes` for the UDMF build, and
`cwad convert --to doom --nodes` on the un-noded twin for the Doom build (see
`tests/first_map.rs` and the map-generation report). Neither is redundant: the
UDMF one is the compiler's native output, and the Doom one is the proof that
output survives a downconvert into the format every engine can read.

**Sector footprints wind clockwise.** A linedef's front (right) sidedef only
faces the sector interior under clockwise winding. Verified empirically: 2611
of 2611 sector boundaries across nine Freedoom maps in both IWADs.

**`geom::contains` returns `true` for points exactly on a boundary.** Even-odd
ray casting has no defined tie-break there. `point_on_polygon_boundary` guards
the overlap test, and the radius-clearance check is the backstop everywhere
else. Do not assume it is a strict interior test.

**Rooms are authored apart, not flush — the wall-thickness model.** Two
rooms connected by a portal never share a coincident wall coordinate; instead
`Portal::at`'s across-coordinate is read against room `a`'s own wall, and room
`b`'s facing wall sits some real, solid distance beyond it
(`Ir::MIN_PORTAL_GAP`, currently 8 map units, and the gap must be a whole
multiple of that). `Ir::from_json` validates the gap via
`geom::facing_spans`/`geom::find_facing_span` — the identical geometry
`compile::portals::resolve_portal` later cuts through, so the two can never
disagree about which wall pair a portal resolves to or how wide the gap
between them is. A `PortalKind::Plain` portal fills that gap with a single
new sector (`compile::portals::emit_gap_sector`): an open, walkable passage,
two threshold lines (room `a` <-> new sector, new sector <-> room `b`) and two
one-sided jambs closing the gap's long sides, front bound to the new sector
with solid rock behind. Neither room's own declared footprint is ever
touched. Swapping `a` and `b` on a portal no longer physically relocates
anything — the gap is filled identically regardless of which room is named
first — though `at`'s convention (anchored to room `a`'s wall) still means
the two labels are not *interchangeable* without also updating `at`. See
`ir::Portal`'s doc comment and `docs/geometry.md`
for the full derivation and worked coordinates. A door portal fills the same
gap differently — see the next entry.

**A door portal's gap decomposes into a chain: an optional near alcove, the
door itself, an optional far alcove — the door-thickness/alcove model.**
Superseding this section's own earlier claim that "a door's depth is simply
the wall gap itself": a `PortalKind::Door`/`PortalKind::Locked` portal now
requires `Portal::door_thickness` (one of 8, 16, or 32 map units — see
`Ir::DOOR_DIMENSIONS`) and accepts two optional buffer sectors,
`Portal::alcove_near` (adjacent to room `a`'s wall) and `Portal::alcove_far`
(adjacent to room `b`'s), each from the same three-value set when present.
`Ir::from_json` requires the facing-wall gap to equal
`door_thickness + alcove_near + alcove_far` **exactly** — not merely "at
least", which the feature's own requester proposed and which is unsound: a
gap wider than the sum would leave a stretch of the corridor with no sector
to fill it, disconnecting whatever lies beyond the shortfall, since every
inch of the gap must belong to some emitted sector or the passage breaks.
`compile::doors::emit_doors` builds the chain as one to three
axis-aligned sectors in sequence (near alcove, door, far alcove — any
absent), each via `compile::portals::emit_segment`/`emit_jambs`/`emit_opening`
directly rather than through `emit_gap_sector` (which only ever builds a
single segment spanning the *entire* gap, the shape `cut_portals` still uses
for a plain portal). Only the door segment's own two faces carry the door
special and its sector's tag; an alcove's two faces are a plain,
non-blocking passage exactly like a plain portal's own gap sector, and its
floor, ceiling, light, and floor/ceiling textures copy whichever real room it
directly borders (room `a` for the near alcove, room `b` for the far one) —
not `min`/`max`-blended the way the plain-portal passage sector is, since an
alcove borders only one real room, not two. An alcove's own walls (its
jambs) use the theme's new `trim` texture role (`STARGR2` for `tech_base`);
the door's own jambs — "the track" — use `door_track` (`DOORTRAK`) as
before, and are lower-unpegged by default so the texture stays anchored to
the floor as the door sector's ceiling animates open, now with an explicit
opt-out (`Portal::track_lower_unpegged: false`). The door's own two faces
carry **neither** pegging flag: `ML_DONTPEGBOTTOM` never affects
upper-texture rendering (`r_segs.c`, pinned commit a77dfb96), which is what
a face's visible texture is, and 247/255 door-special lines in DOOM2.WAD
ship unflagged — an earlier revision set `lower_unpegged` on the faces too,
which `check::invariants::check_door_pegging` (V-P11) now flags if it
recurs, at Warning severity: the convention is measured and reasoned, not
sourced as an engine requirement, so a violation is ugly rather than broken
(`docs/check.md`). `compile::doors::validate_door_texture` additionally
rejects a
theme whose `door` texture is not in `vocabulary.toml`'s curated (not
sourced — see that table's own leading comment)
`[door_texture_catalog]`. See `docs/geometry.md`
for the full derivation, worked coordinates, and why "at least" was rejected
in favor of exact equality.

**A door sector always takes room `a`'s `wall_tex`, never room `b`'s.**
`compile::doors::emit_doors` copies `room_a.wall_tex` (and its floor/ceiling
textures) onto the door sector unconditionally, so the lower texture that
sector's own sidedef paints onto its room-`b`-facing side — the "own texture"
`heights::apply_height_textures` sources a riser from — is drawn in room
`a`'s texture, not room `b`'s. This sits alongside, and mildly contradicts,
the alcove convention one paragraph up: an alcove copies whichever real room
it *directly borders* (room `b` for the far alcove), while the door sector in
the middle copies room `a` regardless of which side of the door it faces.
Invisible while a map uses one theme throughout — every fixture today, since
the shipped `vocabulary.toml` defines exactly one — and becomes visible the
moment two rooms sharing a door use different wall textures.
`rules::tests::a_door_across_a_floor_difference_puts_the_lower_on_the_doors_own_side`
pins the current (room-`a`-sourced) behavior so a fix does not silently
change it unnoticed.

**`heights::visible_lower_side`/`visible_upper_side` are the single place the
drawn-side comparison is made compile-side, and nothing *within the compiler*
re-derives it to keep the two callers honest.**
`heights::apply_height_textures` (which fills a texture) and
`rules::check_missing_textures` (P8, which requires one) both call through
these same two functions rather than each computing "which side is visible"
itself, specifically so the pass and the rule cannot independently drift on
the answer. That guarantee rests entirely on both call sites
continuing to call through the shared functions — nothing checks that they
still do, so a future edit that inlines or reimplements the comparison at
either call site would silently break the consistency with no test able to
catch it.

**The layer-4 verifier is now the independent re-derivation this entry said
did not exist.** `check::invariants::check_textures` (V-P8,
`src/check/invariants.rs`) answers "which side draws the texture" from
`r_segs.c`'s `R_StoreWallRange` directly — line 570's `worldhigh < worldtop`
branch selects the *own* sidedef's `toptexture`, line 589's
`worldlow > worldbottom` branch its `bottomtexture`, with the full citation
trail in that function's doc comment — and never calls
`visible_lower_side`/`visible_upper_side`. A wrong answer shared by the fill
pass and the P8 rule now fails at layer 4 against the emitted map. This does
not make the compile-side coupling safe on its own: the verifier runs on a
built WAD, so it catches the drift after the fact rather than preventing it,
and only when someone runs it (`tests/check_adversarial.rs`'s
`blanking_a_visible_lower_texture_is_caught_as_p8` is the standing regression).

**`facing_spans` has no distance bound, which can surprise an author moving a
room away from a fixture that used to be adjacent.** Two walls "face" each
other if they run the same axis, opposite directions, with overlapping
along-ranges — regardless of how far apart they are. A room's *recessed*
wall (an L-shape's inner corner, say) can genuinely face a second room parked
far away in the outward direction, even past another, nearer wall of the same
first room that a naive "closest wall" intuition would expect to win instead.
`compile::portals::tests::an_l_shaped_room_is_not_adjacent_where_it_has_no_wall`
pins exactly this: relocating that fixture's room `b` outward without also
moving it clear of the recessed wall's own along-range turned the intended
`NotAdjacent` case into a `PortalOffWall` one instead (a real facing span
exists between the two rooms, just not at the requested coordinate) — caught
only by re-deriving the geometry by hand, not by intuition.

**`find_facing_span` returns the *first* matching span, not necessarily the
nearest one.** For a genuinely comb- or zigzag-shaped room, `facing_spans`
can return two spans that share the same `near` (room `a`'s own wall
coordinate and along-range) but different `far` values — two structurally
distinct walls of room `b`, one nearer and one farther, both legitimately
facing the same stretch of room `a`'s wall. `Vec::iter().find()`'s
first-match semantics mean whichever one `wall_edges(poly_b)` happens to
enumerate first wins, silently, with no signal to the author that a second,
equally valid candidate existed. This is deliberately left unresolved rather
than fixed: it is not a soundness bug — either candidate is a real, legal
facing wall, so whichever one is picked, the resulting gap sector is
structurally valid and (as of the sector-overlap check above) verified not
to collide with anything — only a *which of two valid walls did you mean*
ambiguity for the rare non-convex room shape that presents it. Resolving it
would need a policy decision (nearest wins? reject ambiguity outright?) that
the spec does not currently call for; flagging it here is the honest
alternative to guessing one.

**Portal `width` and `at` are exempt from the grid rule** that binds footprints.
Real doorways are routinely finer than the 64-unit grid their rooms sit on.

**Tag 0 is rejected on any action line.** It is not "no tag" — it is the tag
every untagged sector already carries, so an action left at zero matches every
untagged sector in the map. One stray zero opens every door.

**Portals and exits on diagonal walls remain unsupported by design, not by
omission.** `wall_edges` (and so `facing_spans`) only ever reported
axis-aligned edges, so a portal or exit requested on a genuinely diagonal wall
used to fall through to `NotAdjacent`/`PortalOffWall`/`ExitOffWall` — messages
that read as "there is no wall here" for a wall an author can plainly see.
`resolve_portal` and `resolve_exit` now check `geom::on_diagonal_wall`
before returning those errors and raise `CompileError::PortalOnDiagonalWall`/
`ExitOnDiagonalWall` instead, naming the exact coordinate. Supporting a portal
or exit *on* a diagonal wall properly would need a wall model wider than
`(axis, fixed coordinate)`, which the opening-splitting, jamb, and recess
machinery all assume; a chamfered room with its portals and doors on its
square walls (the common real case) already works today, both proved by
`portals::tests::a_portal_works_on_the_axis_aligned_wall_of_a_diagonally_shaped_room`
and the equivalent exit/door fixtures.

**Lifts and teleports are repeatable, not one-shot.** A design choice, not a
source fact: P5 requires a lift be operable from both ends, and a one-shot lift
can strand a player. Recorded in the citations; disagree there if you prefer.

**Odd portal widths are rejected rather than rounded**, per the spec's
reject-don't-degrade posture. The same posture governs
`IrError::SecretWithExplicitSpecial`: a room that sets both `Room::secret` and
`Room::special` is rejected outright rather than letting one silently win.

**A thing's unspecified `skillN` fields default to `true`, not `false`.**
Inverted from bare Rust `bool` defaults on purpose: the pre-existing behavior
(every thing on every skill) had to survive a thing that names no `skills` key
at all, and `ThingSkills` needed the same "unless you say otherwise" default
for a partially specified object too, so a per-field serde default function is
used rather than `#[derive(Default)]`.

**A walkover exit carves its own dead-end alcove; a switch exit does not.**
The pinned engine's `PIT_CheckLine` rejects a mover's crossing — and never
reaches the `spechit` bookkeeping that fires a walkover special — for both a
one-sided line and a two-sided `ML_BLOCKING` one. A walkover exit therefore
has to be a genuinely passable two-sided line, and placing one flush on a
room's true perimeter would open the room to the void beyond it. `compile::exits`
carves a small solid-walled recess out of the host room's own wall instead —
the same near-threshold-plus-solid-sides shape `compile::portals::emit_gap_sector`
builds for a two-room gap sector, just with no second room on the far side (a
solid wall instead of a second threshold) — so only the near threshold (front
the room, back the alcove) is passable. A switch exit needs none of this:
`P_UseSpecialLine` fires from a raycast, not a crossing, so the exit stays a
normal solid one-sided wall.

**Every exit is tagged, even though neither `G_ExitLevel` nor
`G_SecretExitLevel` reads a tag.** Mirrors the existing precedent for manual
doors (above): uniform tagging keeps `tags::check_no_action_at_tag_zero` a
single exception-free invariant and the tag manifest complete. Unlike a
manual door, though, the exit's allocated tag resolves to no sector at
all — `compile::exits::emit_switch_exit`/`emit_walkover_exit` write the tag
onto the exit's own linedef and stop there; no sector's `.tag` field is ever
set to match it (a manual door, by contrast, assigns its tag to its own
door sector). This is correct, not an oversight: `G_ExitLevel`/
`G_SecretExitLevel` are declared `void (void)` and read no argument at all
(`g_game.c:1002` and `:1009`, pinned commit
`a77dfb96cb91780ca334d0d4cfd86957558007e0`), and neither the switch path
(`p_switch.c`'s `P_UseSpecialLine`, cases 11/51 — `P_ChangeSwitchTexture`
reads only `line->sidenum[0]`/`line->special`, never a tag) nor the walkover
path (`p_spec.c`'s `P_CrossSpecialLine`, cases 52/124, which call
`G_ExitLevel`/`G_SecretExitLevel` directly) ever performs a
`P_FindSectorFromLineTag`-style lookup for these four specials. The
verifier's `check::invariants::check_tags` (V-P13) therefore exempts
exactly this set — the same four specials `recognized_specials` already
curates — from its "an action line's tag must resolve to a sector"
requirement: an unresolved tag here is not a dead action, since the tag was
never going to be consulted regardless. V-P14 (no action line at tag 0) and
the tag manifest still cover exit lines like any other action line; only
P13's resolution check is scoped to exclude them.

## Sourcing rule — do not relax this

Every value in `data/engine.toml` and `data/vocabulary.toml` carries a `source`
citation, all against the id-Software DOOM release at pinned commit
`a77dfb96cb91780ca334d0d4cfd86957558007e0`. Textures were verified against the
Freedoom IWADs directly.

A wrong constant produces a map that loads, renders correctly, and is
unplayable. **No test can catch it, because the test reads the same table the
compiler does.** If a source is unreachable, leave the value unsourced and say
so — a reported gap beats a plausible guess.

**One deliberate, clearly-labeled exception: `vocabulary.toml`'s
`[door_texture_catalog]`.** Which texture names read as a door is an
asset-naming convention, not an engine constant and not derivable from one —
there is no `linuxdoom-1.10` table of "the door textures" to cite. That table
carries a `curated` field in place of `source` for exactly this reason, and
`Tables::is_door_texture`'s doc comment repeats the distinction at the call
site. Do not add a `source` field to it, and do not extend this exception to
anything that *is* sourceable.

Two engine facts worth keeping visible, both non-obvious and both found only by
reading the source:

- Vanilla triggers **use**-activated specials only from a line's front side
  (`P_UseSpecialLine`: "Only the front sides of lines are usable"). This is not
  true of walkover or shoot specials.
- `EV_Teleport` nonetheless begins `if (side == 1) return 0;` — a teleport line
  is walkover-triggered yet still front-side-only, contrary to the general rule.

## A note on the tests

Sixty-five passing tests once coexisted with four Critical geometry defects,
because every geometry fixture was two equal, flush, axis-aligned 256-unit
squares — including the four "sub-cases" of the orientation test, which were the
same rectangle rotated four ways.

Fixture **diversity** caught what mutation testing on the existing fixtures
could not. When adding a rule here, add a fixture whose shape differs from the
ones already present, and prove the test fails against a deliberately broken
implementation before trusting it.
