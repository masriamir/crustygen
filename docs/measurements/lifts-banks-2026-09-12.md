# Lift banks — the corpus before and after the bank construct

**Date:** 2026-09-12
**Tool:** `crustygen-corpus` (before: the figures recorded in
[`floors-2026-09-03.md`](floors-2026-09-03.md) and [`lifts-2026-08-30.md`](lifts-2026-08-30.md),
commit `9c90d16`; after: this branch, `feature/50-lift-banks-construct`, at commit `6c6c430`),
crustywad 0.9.6
**Sample:** the same sample of record — crustywad `xtask harvest-sample --seed 20260828
--count 400` → 371 archives on disk, 1,282 unique maps
**Predecessors:** [`floors-2026-09-03.md`](floors-2026-09-03.md), whose *after* column (the honest
six-axis figure) is this document's *before* throughout; [`lifts-2026-08-30.md`](lifts-2026-08-30.md),
whose lift-axis refusal row is identical in both predecessor documents and is this document's
*before* for that section
**Design probe:** [`lift-variants-2026-09-11.md`](lift-variants-2026-09-11.md) §H–§I, already
re-run on this branch as part of the construct's own review (its Method section carries a
2026-09-12 addendum re-baselining §I against the bank-aware recognizer) — not re-run again here;
this document consumes its Method arbiter row and §I Yield table directly
**Contains:** the full `crustygen-corpus` report for this run, at the end; there is no separate
`expressibility-2026-09-12.md`

## Purpose

Project G grows crustygen's vocabulary one construct at a time and re-measures the corpus after
each. Sub-project 4b (issue #72) is shaped differently from every predecessor: it adds no line
special and no axis. A lift bank — one repeatable lift tag naming several sectors — already used
specials 62/88/120/123, which construct 4 made emittable; what changed is `lift::plat`'s judgment
of a shared tag. Before this branch, every member of a multi-sector tag was refused outright as
`Refusal::SharedTag` (precedence 2, immediately after `Dead`, applied regardless of anything else
true about the member). After it, each member is judged alone against the same seven other tests a
single-tag platform gets, and refused only if it passes all of them but no adjacent line calls it
(`Refusal::BankCaller`, precedence 7, immediately before `ConflictingAction`). The matching
authoring capability — `trigger: none` bank members, `Portal::bank`/`Pedestal::bank`, validated and
playtested as `hilera` — is Tasks 1–8's own work; this document measures only the recognizer half,
the half `crustygen-corpus` exercises by reading foreign maps rather than authoring new ones.

This is the roadmap item [`floors-2026-09-03.md`](floors-2026-09-03.md) named and left open: "the
two recognizers now disagree about the same corpus feature" — the floor recognizer already judged
a shared-tag target member by member, where `lift::plat` refused the whole tag on sight. They agree
now: both judge a shared tag member by member, refusing only the member itself found wanting.

Because nothing about the *vocabulary* moved — no new special, no new axis — the "naïve versus
honest" split the floor and lift constructs each reported does not apply here. There is one
recognizer change, not a new gate on top of an old one, so before and after are compared directly
on the same six-axis figure throughout.

## Sample and method

Identical population and gate to the 2026-09-03 run, so the two are directly comparable on every
axis:

| Field | Value |
|---|---|
| seed | `20260828` |
| count | 400 |
| frame rows | 15273 |
| fetch list | `blake3:f3e6453f505ecbed25d39c509903267d00b4b9fba6e7663b3055eac2f6c8759b` |
| archives opened | 371 |
| WAD members read | 403 |
| raw map groups | 1,285 |
| **unique maps** | **1,282** |

The sweep exits 1 by design and wrote **419** `crustygen-corpus:` stderr lines — the same count
every run since 2026-08-28 has written — and the load-failure buckets are unchanged to the unit
(235 `unsupported_format`, 117 `assembly_refused`, 35 `no_maps`, 23 `textmap_unparseable`,
6 `wad_unreadable`, 3 `archive_unreadable`).

> **Status of these numbers: measured practice, not engine fact — and still an upper bound.**
> Membership on the three special/thing axes reads nothing but numeric sets. The teleport, lift and
> floor axes read *geometry* — the plat recognizer resolves every sector a lift line names the way
> `EV_DoPlat` does and now judges each member of a shared tag individually against the same rest,
> trigger and caller facts a single-tag platform is judged against — but everything else about a
> map (room shape, flags, tags outside teleports, platforms and floor targets, texture names) is
> still unmeasured. A geometry-aware lifter can only do worse than this bound, never better. The
> 374-of-400 sample and the overcounted "unloadable" bucket carry over unchanged.

## Before and after

| Axis | Before, all maps | After, all maps | Before, vanilla slice | After, vanilla slice |
|---|---:|---:|---:|---:|
| line specials | 14.6 % (187) | 14.6 % (187) | 18.8 % | 18.8 % |
| sector specials | 60.8 % (780) | 60.8 % (780) | 63.1 % | 63.1 % |
| thing kinds | 74.5 % (955) | 74.5 % (955) | 81.3 % | 81.3 % |
| teleport lines | 80.8 % (1,036) | 80.8 % (1,036) | 82.9 % | 82.9 % |
| lift platforms | 61.1 % (783) | **62.2 % (798)** | 61.9 % | **63.3 %** |
| floor targets | 51.9 % (665) | 51.9 % (665) | 53.8 % | 53.8 % |
| **all axes** | **9.3 % (119)** | **9.3 % (119)** | **11.9 %** | **11.9 %** |

Vanilla-only slice: 77.7 % of unique maps (996 maps), unchanged. Four of the six axes are unchanged
**by construction**: the bank construct touches only `lift::plat`'s per-member judgment, and the
line, sector, thing and floor axes read nothing about how a lift tag resolves. **Only the lift axis
moves** — 783 → 798 maps (+15), 61.1 % → 62.2 % — and the all-axes figure does not move with it: see
below.

## The all-axes figure and the line axis: unchanged, verified directly

The honest all-axes figure is **119 of 1,282 (9.3 %)**, read directly from this run's
`target/expressibility-2026-09-12.json` (`maps[].verdict.expressible`, summed: 119), not inferred
from the unchanged aggregate percentage. The line axis is **187 (14.6 %)**
(`verdict.line_specials_ok`, summed: 187) — unchanged because the bank construct adds no linedef
special to the vocabulary; a bank is built from the same 62/88/120/123 tags construct 4 already
emits and reads.

Because the tool recomputes `expressible` per map fresh rather than by comparing runs, the
unchanged 119 is not an artifact of nothing having moved underneath it — the lift axis's own pass
count *did* move, by +15 maps (below). The unchanged honest figure means, specifically, that **none
of those fifteen newly lift-axis-clean maps also clears every other axis**: each still fails on the
line axis, a sector special, a thing kind, a teleport line, or a different platform on the same or
another lift tag. A map-by-map decomposition is warranted only when the honest figure moves; it did
not, so none is needed — the direct JSON count above already is that decomposition, run against a
difference of zero.

## The lift axis's refusals, before and after

| Measure | Before | After |
|---|---:|---:|
| maps with ≥ 1 lift line | 795 (62.0 % of 1,282) | 795 (62.0 % of 1,282) |
| maps with a refused platform or line | 499 (62.8 % of 795) | **484 (60.9 % of 795)** |
| maps passing the axis | 783 (61.1 % of 1,282) | **798 (62.2 % of 1,282)** |
| platforms resolved (`plats`) | 3,876 | 3,876 |
| — cannot move (`dead`) | 244 | 244 |
| accepted: lifts / pedestals / barriers | 1,498 / 364 / 137 (1,999 = 51.6 % of plats) | **1,679 / 429 / 176 (2,284 = 58.9 % of plats)** |
| maps carrying a lift / pedestal / barrier | 563 / 216 / 85 | 577 / 238 / 90 |
| broken lift lines | 304, in 42 maps | 304, in 42 maps (unchanged — broken-line detection does not read tag sharing) |
| callable from below | 3,421 (88.3 % of plats) | 3,421 (88.3 % of plats) (unchanged) |
| with a top trigger | 1,361 | 1,361 (unchanged) |
| holding ≥ 1 thing | 1,433 | 1,433 (unchanged) |
| driven at `blazeDWUS` speed | 1,056 | 1,056 (unchanged) |

The refusal classes, in the recognizer's own precedence order on each side (the first that applies
wins):

| Refusal | Before | Before share of 3,876 | After | After share of 3,876 |
|---|---:|---:|---:|---:|
| `dead` | 244 | 6.3 % | 244 | 6.3 % |
| `shared_tag` (retired) | 923 | 23.8 % | — | — |
| `one_shot` | 56 | 1.4 % | **181** | 4.7 % |
| `mixed_speed` | 47 | 1.2 % | **91** | 2.3 % |
| `unsupported_rest` | 335 | 8.6 % | **441** | 11.4 % |
| `top_only` | 104 | 2.7 % | **156** | 4.0 % |
| `one_way_barrier` | 104 | 2.7 % | **159** | 4.1 % |
| `bank_caller` (new) | — | — | **231** | 6.0 % |
| `conflicting_action` | 64 | 1.7 % | **89** | 2.3 % |
| **any refusal** | **1,877** | **48.4 %** | **1,592** | **41.1 %** |

**The 923 pre-branch `shared_tag` refusals are fully accounted for, not merely reduced.**
`BankCaller`'s own gate (`p.shared_tag >= 2 && p.low_activator_neighbors().is_empty()`) never fires
on a single-tag platform, and none of the other seven refusal tests reads `shared_tag` at all — so
a single-tag platform's classification is bit-for-bit identical before and after this branch, and
every platform whose reported reason changed must be one of the 1,085 members of a shared tag.
`Dead` (precedence 1, evaluated before the shared-tag question can even be asked) is unaffected —
244 both sides — so the reshuffle is confined to exactly the 923 non-dead members:

| Category | Before | After | Δ |
|---|---:|---:|---:|
| `one_shot` | 56 | 181 | +125 |
| `mixed_speed` | 47 | 91 | +44 |
| `unsupported_rest` | 335 | 441 | +106 |
| `top_only` | 104 | 156 | +52 |
| `one_way_barrier` | 104 | 159 | +55 |
| `conflicting_action` | 64 | 89 | +25 |
| accepted (lift + pedestal + barrier) | 1,999 | 2,284 | **+285** |
| `bank_caller` | 0 | 231 | **+231** |
| **sum of deltas (excluding `dead`)** | | | **923** |

Every one of the 923 members that used to be caught by the blanket `SharedTag` test now falls
through to the same tests a single-tag platform faces: **285 turn out to be an honest lift, pedestal
or barrier once judged alone**; **407 turn out to have a genuine defect of their own** — a one-shot
trigger, mixed speeds, an unsupported rest, no trigger below, or a one-way barrier reach, or a
conflicting tagged action — that the old catch-all had been masking by firing first; and **231 pass
every one of those tests but have no adjacent caller**, the shape `BankCaller` exists to name. None
of the 407 are "recovered" by this construct in any sense — they were never expressible, sharing a
tag or not — but they are now correctly diagnosed instead of misattributed to tag-sharing.

## Reconciliation against the design probe

The design probe (`lift-variants-2026-09-11.md`) was already re-run on this branch during the
construct's own development, so this section cites its arbiter row and Yield table rather than
regenerating them.

**The Method arbiter row matches to the unit.** The probe's own re-derivation of the shipped
recognizer's `bank_caller` count, over the same sample:

| Population | Probe (Method row) | This sweep |
|---|---:|---:|
| DOOM+DOOM2 | 7 | *(not re-run here; unaffected — see Caveats)* |
| Final Doom | 22 | *(not re-run here; unaffected — see Caveats)* |
| idgames sample | 231 | **231** |

**Against the probe's A′ gate.** §I's Yield table prices four hypothetical *group-level* gates — a
bank is only "recovered" under a gate if **every** member of its tag passes it, an all-or-nothing
test the probe applies on top of its own per-member re-derivation. Gate A′ ("neighbor-called": every
member passes alone and some lift line fires from a two-sided neighbor standing at that member's own
`low`, wherever the line sits) is the gate closest to what `BankCaller` actually judges, and the
probe predicted, before this branch shipped:

| Metric (idgames) | Probe's A′ prediction | This sweep's shipped result |
|---|---:|---:|
| honest all-axes figure | 119 (9.3 %), **+0 maps** over "today" | **119 (9.3 %)** |
| historical shared-tag population the gate reasons over | 923 | 923 (historical; unchanged by this branch) |
| platforms the gate recovers | 204, inside 69 fully-qualifying groups | — |
| platforms the shipped recognizer newly accepts | — | **285**, member by member |
| platforms refused for "nobody adjacent calls it" | *(gate has no separate refusal — a group short of the gate is simply not "recovered")* | **231** (`bank_caller`, matches the Method row above) |

Both figures hold exactly: the honest figure stays at **119**, and the shipped `bank_caller` count
is **231**. The **204/69** figure and the **285** figure are deliberately not
the same number, and should not be forced to match: gate A′ is a *whole-group* test — a bank of five
members where four are neighbor-called and one is not contributes **zero** to "recovered", because
the group as a whole does not pass. The shipped recognizer imposes no such requirement — it accepts
each member on its own facts, so a partially-qualifying bank still contributes its qualifying
members. A per-member gate can only be as strict or looser than the same-condition whole-group gate,
so **285 ≥ 204** is the expected relationship, not a discrepancy to close. The probe's own Method
note states the same thing more directly: the `bank_caller` population "is a subset of the same
923/135/58 holding only the members no adjacent line calls (the rest of the 923/135/58 are accepted,
or refused for an earlier reason, by the bank-aware recognizer)" — exactly the 285/407 split derived
independently above.

The probe's separate "member verdict, judged alone" figures (483 accepted, ignoring the caller
question entirely) are not cited as a third quantity to reconcile against: that re-derivation
explicitly skips only the `BankCaller` test while still applying `ConflictingAction` after it, which
is a different precedence than the shipped recognizer's own `BankCaller`-before-`ConflictingAction`
order. A member with neither a caller nor a conflicting action lands as "accepted" under the probe's
skip-and-continue re-derivation but as `bank_caller` under the shipped precedence (which never
reaches the `ConflictingAction` test for it), so the two are not directly comparable member for
member — the probe's own Method note flags this as "a different, smaller population than the
historical `SharedTag` figures" for exactly this reason, and this document follows it in treating
`bank_caller` (231, reproduced above) as the one figure to reconcile, not the 483.

DOOM+DOOM2 and Final Doom are not re-run in this document (see Caveats); both stood at 0 honest maps
under every gate the probe tested before this branch, and nothing about a recognizer refinement
moves a figure that is already zero on a 68- or 64-map population wall-to-wall in tier-3 specials.

## What this construct buys: vocabulary, not maps

On the sample of record, the honest yield in map count is **zero** — exactly what the probe's A′
gate predicted for this construct before it shipped. The yield that is real is in the recognizer's
*vocabulary*: 285 platforms across the sample that used to be unconditionally unstatable now read as
an authored lift, pedestal or barrier; 231 more are now correctly named as "a bank member nobody
calls" instead of being folded into an undifferentiated tag-sharing refusal; and 407 that remain
refused are now refused for their own, previously-hidden reason rather than for sharing a tag. The
lift axis's own pass rate — a real, if narrower, measure than the six-axis conjunction — moved by
fifteen maps (783 → 798, 61.1 % → 62.2 %). None of that fifteen-map or 285-platform movement happens
to unblock a map whose *only* remaining defect was a shared lift tag; every map gaining a platform
still fails elsewhere, almost always on the line axis's own untouched blocker table (109, 103, 48,
31, 2, 117 — see `floors-2026-09-03.md`, unmoved by this construct).

This is not a verdict on the construct's worth: the recognizer's job is to grade *foreign* maps, and
this 400-map draw happens not to reward the refinement with a newly expressible one. The matching
authoring capability — banks the compiler can build, proved by `hilera`'s playtest — is a real gain
this sweep does not measure at all, because `crustygen-corpus` only reads maps, it does not build
them.

## Greedy curves

The line-axis-alone curve is unchanged to the tenth of a point, as expected — no special left or
joined the vocabulary:

| k | Line axis alone (unchanged) |
|---|---:|
| 1 | 16.3 % |
| 5 | 19.7 % |
| 10 | 23.2 % |
| 21 | 32.7 % |
| 51 | 49.5 % |

The conjunction curve (maps already clear on sectors, things, teleports, lifts, and floors) moves by
a few tenths at each checkpoint, because the population it walks grew by the fifteen maps that now
pass the lift axis — not because the honest ceiling moved:

| k | Conjunction, before | Conjunction, after |
|---|---:|---:|
| 1 | 10.1 % | 10.1 % |
| 5 | 11.7 % | 11.8 % |
| 10 | 13.3 % | 13.5 % |
| 21 | 16.1 % | 16.3 % |
| 51 | 19.7 % | 20.0 % |

As `floors-2026-09-03.md` notes about its own curve: this is a different, slightly larger population
than the before-run's, so the small rise is not a claim about the honest ceiling, which the "Before
and after" table already reports directly.

## Caveats

- **Numbers are reported, never tuned.** The probe's A′ prediction was 119 honest maps and 231
  `bank_caller` refusals (via its Method row); the sweep delivers both to the unit, and the 285-versus-204
  divergence is explained above by the per-member/per-group gate difference rather than closed by
  adjusting either figure.
- **The two "recovered" figures are different, differently sized populations**, as the probe's own
  Method note says: 204 (or 69 groups) is a strict, whole-group metric computed by the probe's own
  hypothetical gates; 285 is the shipped recognizer's actual, per-member accepted-count delta,
  computed by this sweep. Neither supersedes the other; they answer different questions.
- **DOOM+DOOM2 and Final Doom are not re-run here.** The probe already reproduced their
  `bank_caller` counts (7 and 22) on this branch and both populations stood at 0 honest maps before
  and after every construct to date; re-running `crustygen-corpus` over retail archives would add
  nothing this document needs.
- **Refusal rows are not row-comparable one-to-one with the probe's own §I breakdown** below the
  `bank_caller`/honest-figure level: the probe's "judged alone" re-derivation skips only the
  `BankCaller` arm while the shipped recognizer's precedence also moves `OneWayBarrier`, `TopOnly`
  and the rest ahead of it; the two sit in the same *order* but answer a subtly different question,
  as the previous section details.
- **UDMF/ZDoom-numbered platforms are unmeasured** — 66 of the 1,282 sample maps are UDMF-origin and
  read Doom-numbered specials only. **Sector extent is a bounding box.** Both caveats carry over
  verbatim from every predecessor.
- **"Unloadable" is still overcounted.** Ingest runs crustywad's *strict* assembly, so maps a
  lenient assembler would load never reach the classifier. crustygen
  [#34](https://github.com/masriamir/crustygen/issues/34) tracks it; closing it adds maps to the
  denominator, so these shares are not comparable with a post-#34 run.
- **The split-as-one-lift reading is a separate, unshipped gate.** The probe's §I prices reading a
  connected, one-floor split tag as a single platform at a further +1 map and +28 recovered
  platforms in 11 of the sample's 25 split groups (`shared_split`, unchanged at 25 by this
  construct). That reading is not part of what shipped; it is named here only so a future construct
  does not re-measure it from zero.

## Method — the exact commands

From the crustygen checkout at `6c6c430`, with the sample already fetched in the crustywad checkout
(`just harvest-sample 20260828 400`). Below, `$SAMPLE` is that checkout's
`xtask/data/samples/20260828-400`:

```console
$ just corpus "$SAMPLE"
```

which runs `cargo run --release --bin crustygen-corpus -- <dir> --report
docs/measurements/expressibility-$(date +%F).md --json target/expressibility-$(date +%F).json`,
exits 1 as designed, and takes about three minutes. Its report is the last section of this document;
the generated `expressibility-2026-09-12.md` was renamed to this file rather than committed beside
it.

The design probe was **not** re-run for this document; it was already re-run on this branch during
the construct's own review, and its Method section carries the resulting 2026-09-12 addendum. The
command, for reference, is:

```console
$ cargo run --release --example liftprobe -- variants idgames "$SAMPLE"
```

The all-axes and lift-axis figures quoted above were read from this run's `--json`:

```console
$ python3 -c "
import json
d = json.load(open('target/expressibility-2026-09-12.json'))
maps = d['maps']
expressible = sum(1 for m in maps if m['verdict']['expressible'])
lifts_ok = sum(1 for m in maps if m['verdict']['lifts_ok'])
line_ok = sum(1 for m in maps if m['verdict']['line_specials_ok'])
print('expressible', expressible, 'lifts_ok', lifts_ok, 'line_ok', line_ok)
"
```

which prints `expressible 119 lifts_ok 798 line_ok 187`, matching the report's own aggregate
percentages and confirming the honest figure directly rather than by inference from an unmoved
percentage.

## Re-running

In the crustywad checkout, `just harvest-sample 20260828 400` re-fetches the same draw (a present,
correctly sized zip is skipped, so it is cheap after the first run). Here, `just corpus
/path/to/20260828-400` writes `docs/measurements/expressibility-<today>.md` plus a gitignored JSON
under `target/`. Exit 1 is expected, not a failure. Compare the new Expressibility table, the Lifts
section and the blocker rankings against the tables above — and re-order the Project G queue from
the new blockers, not from these.

---

*Everything below is the `crustygen-corpus` output for this run, reproduced in full. Its heading
levels are demoted by one so the document keeps a single title; nothing else is changed.*

## Corpus expressibility

> **Status of these numbers: measured practice, not engine fact — and an upper bound.** A map counts as expressible when every non-zero line special and sector special, and every thing type, it carries is in crustygen's emittable vocabulary, every teleport line it carries is one the recognizer can state (see `Teleports`), every platform its lift lines name is one of the three shapes the IR can state (see `Lifts`), and every floor target its floor lines name is one of the three shapes the IR can state (see `Floors`). Beyond those teleport lines, platforms, and floor targets, geometry, flags, tags, and texture names are not measured; a geometry-aware lifter can only do worse, never better, than this bound.

### Sample

- seed `20260828`, count 400, frame rows 15273, fetch list `blake3:f3e6453f505ecbed25d39c509903267d00b4b9fba6e7663b3055eac2f6c8759b`
- ids: 81 102 131 184 206 298 309 359 476 527 556 641 780 819 864 878 908 914 930 957 1006 1019 1084 1115 1231 1237 1253 1381 1408 1488 1533 1771 1823 1850 1909 2111 2198 2324 2350 2420 2491 2497 2652 2661 2663 2713 2739 2872 2888 3089 3286 3370 3426 3494 3610 3641 3659 3740 3800 3803 3827 3836 4250 4279 4280 4301 4309 4377 4849 4985 5116 5173 5374 5663 5755 5869 5899 6002 6079 6104 6115 6116 6234 6247 6281 6373 6392 6393 6402 6417 6419 6591 6665 6722 6888 6915 6950 7056 7106 7140 7227 7236 7387 7445 7496 7516 7554 7557 7828 7914 7917 7973 8049 8072 8152 8194 8207 8218 8389 8590 8720 8721 8793 8812 8861 8862 9006 9060 9122 9318 9470 9493 9531 9585 9656 9811 9887 9894 10109 10231 10303 10509 10730 10773 10817 10857 10900 10923 11001 11091 11096 11177 11300 11351 11431 11453 11477 11569 11594 11626 11655 11771 11784 11887 11903 11905 11971 12027 12069 12151 12166 12215 12228 12257 12284 12311 12419 12481 12494 12553 12561 12571 12646 12737 12779 12788 12792 12820 12845 12849 12927 13032 13126 13136 13178 13203 13326 13336 13405 13427 13428 13455 13476 13530 13575 13583 13642 13663 13686 13718 13723 13780 13782 13808 13822 13836 13892 13893 13920 13995 14055 14169 14200 14248 14271 14294 14340 14377 14384 14442 14447 14468 14484 14543 14547 14566 14587 14600 14626 14667 14694 14698 14737 14800 14816 14867 14994 15093 15124 15282 15352 15387 15388 15402 15425 15453 15492 15571 15584 15614 15642 15677 15812 15850 15913 15970 15984 16084 16085 16096 16157 16213 16246 16254 16258 16270 16334 16359 16395 16461 16477 16617 16678 16733 16756 16771 16839 16858 16906 16985 17012 17017 17070 17073 17077 17106 17152 17182 17188 17270 17287 17312 17464 17585 17630 17633 17678 17695 17814 17852 17872 17895 17937 17942 18016 18026 18046 18135 18151 18177 18217 18244 18254 18271 18455 18610 18623 18638 18679 18828 18880 18951 18982 19005 19012 19022 19065 19149 19175 19211 19279 19297 19381 19447 19478 19504 19547 19669 19737 19847 19968 19981 20102 20156 20268 20335 20358 20359 20389 20403 20419 20475 20556 20620 20625 20629 20636 20668 20679 20766 20797 20828 20899 20911 21130 21202 21314 21333 21368 21403 21405 21422 21445 21479 21480 21654 21721 21763 21826 21853 21861 21870 21878 21880 21898 21952 21976 22012 22051 22070

### Buckets

| Bucket | Count |
|---|---|
| `archives` | 371 |
| `wads` | 403 |
| `maps_raw` | 1285 |
| `maps_unique` | 1282 |
| `archive_unreadable` | 3 |
| `wad_unreadable` | 6 |
| `no_maps` | 35 |
| `unsupported_format` | 235 |
| `assembly_refused` | 117 |
| `textmap_unparseable` | 23 |

### Expressibility

| Axis | All unique maps | Vanilla-only slice |
|---|---|---|
| line specials | 14.6 % | 18.8 % |
| sector specials | 60.8 % | 63.1 % |
| thing kinds | 74.5 % | 81.3 % |
| teleport lines | 80.8 % | 82.9 % |
| lift platforms | 62.2 % | 63.3 % |
| floor targets | 51.9 % | 53.8 % |
| **all axes** | 9.3 % | 11.9 % |

Vanilla-only slice: 77.7 % of unique maps.

### Line-special blockers

| Value | Maps | Share |
|---|---|---|
| 109 | 331 | 25.8 % |
| 103 | 329 | 25.7 % |
| 48 | 312 | 24.3 % |
| 31 | 263 | 20.5 % |
| 2 | 242 | 18.9 % |
| 117 | 237 | 18.5 % |
| 112 | 201 | 15.7 % |
| 32 | 184 | 14.4 % |
| 19 | 180 | 14.0 % |
| 33 | 173 | 13.5 % |
| 63 | 160 | 12.5 % |
| 34 | 155 | 12.1 % |
| 36 | 155 | 12.1 % |
| 71 | 152 | 11.9 % |
| 102 | 136 | 10.6 % |
| 118 | 124 | 9.7 % |
| 46 | 120 | 9.4 % |
| 20 | 119 | 9.3 % |
| 60 | 94 | 7.3 % |
| 61 | 92 | 7.2 % |
| 114 | 90 | 7.0 % |
| 133 | 85 | 6.6 % |
| 135 | 80 | 6.2 % |
| 40 | 76 | 5.9 % |
| 22 | 70 | 5.5 % |

### Sector-special blockers

| Value | Maps | Share |
|---|---|---|
| 2 | 226 | 17.6 % |
| 12 | 155 | 12.1 % |
| 13 | 133 | 10.4 % |
| 4 | 58 | 4.5 % |
| 1024 | 33 | 2.6 % |
| 21 | 24 | 1.9 % |
| 11 | 19 | 1.5 % |
| 115 | 15 | 1.2 % |
| 65 | 14 | 1.1 % |
| 81 | 13 | 1.0 % |
| 256 | 13 | 1.0 % |
| 512 | 11 | 0.9 % |
| 71 | 10 | 0.8 % |
| 72 | 9 | 0.7 % |
| 128 | 7 | 0.5 % |
| 67 | 6 | 0.5 % |
| 64 | 5 | 0.4 % |
| 66 | 5 | 0.4 % |
| 15 | 4 | 0.3 % |
| 77 | 4 | 0.3 % |
| 208 | 4 | 0.3 % |
| 4128 | 4 | 0.3 % |
| 47 | 3 | 0.2 % |
| 80 | 3 | 0.2 % |
| 83 | 3 | 0.2 % |

### Thing-type blockers

| Value | Maps | Share |
|---|---|---|
| 32000 | 112 | 8.7 % |
| 88 | 65 | 5.1 % |
| 72 | 45 | 3.5 % |
| 87 | 35 | 2.7 % |
| 89 | 29 | 2.3 % |
| 9001 | 26 | 2.0 % |
| 0 | 24 | 1.9 % |
| 9025 | 20 | 1.6 % |
| 9044 | 15 | 1.2 % |
| 90 | 13 | 1.0 % |
| 92 | 13 | 1.0 % |
| 9800 | 11 | 0.9 % |
| 9999 | 11 | 0.9 % |
| 5804 | 10 | 0.8 % |
| 5807 | 10 | 0.8 % |
| 94 | 9 | 0.7 % |
| 5004 | 9 | 0.7 % |
| 5803 | 9 | 0.7 % |
| 5809 | 9 | 0.7 % |
| 9045 | 9 | 0.7 % |
| 95 | 8 | 0.6 % |
| 96 | 8 | 0.6 % |
| 9070 | 7 | 0.5 % |
| 9072 | 7 | 0.5 % |
| 9080 | 7 | 0.5 % |

### Teleports

| Measure | Value |
|---|---|
| maps with a teleport line | 797 (62.2 % of unique maps) |
| maps with a refused line (not expressible) | 246 |
| lines: player / monsters-only / one-shot | 15148 / 5343 / 953 |
| lines in closets (front sector holds a monster) | 8294 (542 maps) |
| lines delivering beside an exit | 169 (47 maps) |
| lines on a paired pad | 2490 (238 maps) |
| geometry: island / alcove / boundary / other | 5574 / 1275 / 9706 / 3936 |
| ambiguous (several markers) / broken / self-referencing | 204 / 748 / 3844 |

### Lifts

| Measure | Value |
|---|---|
| maps with a lift line | 795 (62.0 % of unique maps) |
| maps with a refused plat or line (not expressible) | 484 |
| plats | 3876 |
| lifts / pedestals / barriers | 1679 / 429 / 176 |
| maps with a lift / pedestal / barrier | 577 / 238 / 90 |
| refused: dead / one-shot / mixed speed / unsupported rest / top-only / one-way barrier / bank caller / conflicting action | 244 / 181 / 91 / 441 / 156 / 159 / 231 / 89 |
| shared-tag groups that are one platform split | 25 |
| broken lines | 304 |
| callable from below | 3421 (88.3 % of plats) |
| with a top trigger | 1361 |
| holding things | 1433 |
| fast | 1056 |

### Floors

| Measure | Value |
|---|---|
| maps with a floor line | 788 (61.5 % of unique maps) |
| maps with a refused target or line (not expressible) | 617 |
| targets | 9443 |
| drop walls / reveals / bridges | 2172 / 2656 / 166 |
| maps with a drop wall / reveal / bridge | 450 / 474 / 99 |
| refused: gun / conflict / two families / unresolved / dead / closing / mixed / neutral / rider loses / no activator / unsupported shape / neighbors mover | 162 / 1011 / 360 / 23 / 341 / 263 / 182 / 1382 / 110 / 6 / 15 / 594 |
| broken lines: tag 0 / dangling | 418 / 923 |
| shared-tag members accepted | 3409 |
| remote triggers | 16374 |
| targets holding a thing | 3021 |

### Greedy curve — line axis alone

Share is of all unique maps, with sector specials and thing kinds held expressible.

| k | Cumulative share of all unique maps |
|---|---|
| 1 | 16.3 % |
| 5 | 19.7 % |
| 10 | 23.2 % |
| 21 | 32.7 % |
| 51 | 49.5 % |

Order chosen: 117 → 48 → 63 → 114 → 103 → 2 → 31 → 102 → 109 → 112 → 118 → 32 → 33 → 34 → 19 → 71 → 20 → 46 → 53 → 36 → 90 → 61 → 6 → 7 → 133

### Greedy curve — conjunction (maps already ok on sectors, things, teleports, lifts, and floors)

Share is of **all unique maps**, not of the already-ok population this curve walks, so it plateaus below 100 % by exactly the maps blocked on a sector special, a thing kind, a refused teleport line, a refused platform, or a refused floor target.

| k | Cumulative share of all unique maps |
|---|---|
| 1 | 10.1 % |
| 5 | 11.8 % |
| 10 | 13.5 % |
| 21 | 16.3 % |
| 51 | 20.0 % |

Order chosen: 117 → 63 → 114 → 48 → 109 → 112 → 103 → 2 → 31 → 118 → 32 → 33 → 34 → 6 → 25 → 36 → 105 → 255 → 242 → 271 → 7 → 45 → 46 → 53 → 71
