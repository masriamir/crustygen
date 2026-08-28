# Corpus expressibility — before and after the complete decoration set

**Date:** 2026-08-28
**Tool:** `crustygen-corpus` (before: commit `d6da4dc`; after: commit `dbbd950`), crustywad 0.9.6
**Sample:** crustywad `xtask harvest-sample --seed 20260828 --count 400` → 374 archives on disk (26 failed: size mismatch against the fetch list), provenance below
**Method:** every `*.zip` in the sample directory opens through crustywad's archive reader
(lenient options, CRCs still verified) and every `.wad` member is read; each map group then
goes through crustygen's shared strict `ingest::load_map` — a binary group through crustywad's
strict `Map::assemble` and a UDMF round-trip, a `TEXTMAP` group parsed directly — followed by
`lift::survey` for the per-map census and `Vocabulary::classify` for the verdict. Maps are
deduplicated by a `sha256:` hash over their lumps (name, length, bytes), so a map repackaged in
several zips counts once. Both runs exit 1 by design: a real idgames sample always carries load
failures, and each one is named on stderr and counted in a bucket instead of aborting the sweep.
The two runs differ only in the vocabulary the binary was built with — same directory, same
sample, byte-identical stderr (419 lines both times).

> **Status of these numbers: measured practice, not engine fact — and an upper bound.**
> A map counts as expressible when every non-zero linedef special it carries, every non-zero
> sector special it carries, and **every** thing type it carries — zero included — is in
> crustygen's emittable vocabulary. `lift::survey` skips zero on the two special axes only;
> thing types are histogrammed as they come, so a thing of type 0 blocks a map like any other
> unknown value. Geometry, flags, tags, and texture names are not measured; a geometry-aware
> lifter can only do worse, never better, than this membership test. Two further reasons to read every share below as a ceiling rather than a
> score:
>
> - **The sample of record is 374 archives, not 400.** 26 of the seeded 400 failed to download
>   against the size the fetch list declares and are absent from the directory; both reports
>   still echo all 400 manifest ids. See [Sample provenance](#sample-provenance).
> - **"Unloadable" is overcounted.** Ingest runs crustywad's *strict* assembly, so maps a
>   lenient assembler would load — a `REJECT` lump one byte short, a zlib-compressed
>   extended-node stream this build cannot inflate — are refused and never reach the
>   classifier. crustygen [#34](https://github.com/masriamir/crustygen/issues/34) tracks that.
>   Closing it adds maps to the denominator, so these shares are not directly comparable to a
>   post-#34 run.

## Sample provenance

Read from `sample-manifest.json` in the sample directory (crustywad
`xtask/data/samples/20260828-400/`):

| Field | Value |
|---|---|
| seed | `20260828` |
| count | 400 |
| frame rows | 15273 |
| fetch list | `blake3:f3e6453f505ecbed25d39c509903267d00b4b9fba6e7663b3055eac2f6c8759b` |
| entries `ok` | 374 |
| entries `failed` | 26 |

Every one of the 26 failures is a size mismatch between the size the fetch list declares and the
body the mirror returned — 17 `body exceeded declared size`, 9 `short body`. The manifest records
one fetch pass and nothing else (every success is plain `ok`; no entry is marked as already
present), so it is no evidence either way about whether a retry would succeed. The sample of
record is the 374 archives that are on disk:

| id | file | manifest status |
|---|---|---|
| 3659 | `m2all.zip` | `failed:short body: got 286718 bytes, wanted 780153` |
| 12792 | `0verpw_x.zip` | `failed:short body: got 291191 bytes, wanted 331461` |
| 13455 | `joker.zip` | `failed:short body: got 16033 bytes, wanted 23420` |
| 13642 | `reversi.zip` | `failed:short body: got 124488 bytes, wanted 124513` |
| 14484 | `test.zip` | `failed:short body: got 1808 bytes, wanted 2407` |
| 14600 | `de-pkg.zip` | `failed:body exceeded declared size` |
| 14816 | `pc_hellk.zip` | `failed:body exceeded declared size` |
| 14994 | `eden.zip` | `failed:body exceeded declared size` |
| 15584 | `hpolis.zip` | `failed:short body: got 1176865 bytes, wanted 2258959` |
| 15913 | `rslvenge.zip` | `failed:body exceeded declared size` |
| 16085 | `qthreat.zip` | `failed:body exceeded declared size` |
| 17073 | `rslpagan.zip` | `failed:body exceeded declared size` |
| 17270 | `hymn.zip` | `failed:short body: got 4772153 bytes, wanted 5645880` |
| 18217 | `monument.zip` | `failed:body exceeded declared size` |
| 18623 | `lilium.zip` | `failed:body exceeded declared size` |
| 19297 | `dbp8_mbg.zip` | `failed:body exceeded declared size` |
| 19478 | `phcmplex.zip` | `failed:short body: got 4820577 bytes, wanted 4820709` |
| 19504 | `cumaapia.zip` | `failed:short body: got 972384 bytes, wanted 32260160` |
| 19847 | `crud.zip` | `failed:body exceeded declared size` |
| 19981 | `zone400.zip` | `failed:body exceeded declared size` |
| 20419 | `codepers.zip` | `failed:body exceeded declared size` |
| 20828 | `ichinichi.zip` | `failed:body exceeded declared size` |
| 21202 | `votn.zip` | `failed:body exceeded declared size` |
| 21314 | `halogen.zip` | `failed:body exceeded declared size` |
| 21721 | `faviltri.zip` | `failed:body exceeded declared size` |
| 21853 | `tekonme.zip` | `failed:body exceeded declared size` |

The 374 ids actually present, sorted:

```
81 102 131 184 206 298 309 359 476 527 556 641 780 819 864 878 908 914 930 957 1006 1019
1084 1115 1231 1237 1253 1381 1408 1488 1533 1771 1823 1850 1909 2111 2198 2324 2350
2420 2491 2497 2652 2661 2663 2713 2739 2872 2888 3089 3286 3370 3426 3494 3610 3641
3740 3800 3803 3827 3836 4250 4279 4280 4301 4309 4377 4849 4985 5116 5173 5374 5663
5755 5869 5899 6002 6079 6104 6115 6116 6234 6247 6281 6373 6392 6393 6402 6417 6419
6591 6665 6722 6888 6915 6950 7056 7106 7140 7227 7236 7387 7445 7496 7516 7554 7557
7828 7914 7917 7973 8049 8072 8152 8194 8207 8218 8389 8590 8720 8721 8793 8812 8861
8862 9006 9060 9122 9318 9470 9493 9531 9585 9656 9811 9887 9894 10109 10231 10303 10509
10730 10773 10817 10857 10900 10923 11001 11091 11096 11177 11300 11351 11431 11453
11477 11569 11594 11626 11655 11771 11784 11887 11903 11905 11971 12027 12069 12151
12166 12215 12228 12257 12284 12311 12419 12481 12494 12553 12561 12571 12646 12737
12779 12788 12820 12845 12849 12927 13032 13126 13136 13178 13203 13326 13336 13405
13427 13428 13476 13530 13575 13583 13663 13686 13718 13723 13780 13782 13808 13822
13836 13892 13893 13920 13995 14055 14169 14200 14248 14271 14294 14340 14377 14384
14442 14447 14468 14543 14547 14566 14587 14626 14667 14694 14698 14737 14800 14867
15093 15124 15282 15352 15387 15388 15402 15425 15453 15492 15571 15614 15642 15677
15812 15850 15970 15984 16084 16096 16157 16213 16246 16254 16258 16270 16334 16359
16395 16461 16477 16617 16678 16733 16756 16771 16839 16858 16906 16985 17012 17017
17070 17077 17106 17152 17182 17188 17287 17312 17464 17585 17630 17633 17678 17695
17814 17852 17872 17895 17937 17942 18016 18026 18046 18135 18151 18177 18244 18254
18271 18455 18610 18638 18679 18828 18880 18951 18982 19005 19012 19022 19065 19149
19175 19211 19279 19381 19447 19547 19669 19737 19968 20102 20156 20268 20335 20358
20359 20389 20403 20475 20556 20620 20625 20629 20636 20668 20679 20766 20797 20899
20911 21130 21333 21368 21403 21405 21422 21445 21479 21480 21654 21763 21826 21861
21870 21878 21880 21898 21952 21976 22012 22051 22070
```

## Load failures

The buckets the run counted — identical in both runs, since the failure classes do not depend
on vocabulary:

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

371 of the 374 archives on disk opened; they yielded 403 WAD members and 1,285 raw map groups,
of which **1,282 are unique** by content hash. Two denominators are in play below: the
"All unique maps" column, every blocker table and both greedy curves are against those 1,282
maps, while the "Vanilla-only slice" column is against the slice itself — 77.7 % of 1,282, i.e.
**996 maps**. So the after run's 8.3 % on the slice is 83 of 996, not 83 of 1,282.

Normalizing the 419 stderr lines (digits replaced, source path stripped) gives the failure
classes behind those buckets:

| Failure class | Lines | Bucket | What it is |
|---|---|---|---|
| `unsupported binary map format Hexen` | 235 | `unsupported_format` | Hexen-format binary map; refused because Hexen-style specials are a different numbering the checks do not model |
| `NODES uses an unsupported extended node encoding ZNOD` | 47 | `assembly_refused` | zlib-compressed ZDoom extended nodes; this build has no `extended-nodes-zlib` |
| `no map groups` | 35 | `no_maps` | resource WAD — ordinary corpus content, named on stderr but not a load failure |
| `REJECT lump is N bytes; N bytes required for N sectors` | 33 | `assembly_refused` | undersized `REJECT`; 25 of the 33 are short by exactly one byte, the rest by 72–65,536 |
| `sector index N referenced from sidedef is out of range (N available)` | 21 | `assembly_refused` | out-of-range sector index — 65535 in 20 of the 21 lines, 65516 in the other |
| `failed to render assembled map as UDMF: linedef #N has no front sidedef` | 18 | `textmap_unparseable` | the UDMF round-trip half of ingest: UDMF cannot represent a linedef with no front sidedef |
| `linedef index N referenced from blockmap block is out of range (N available)` | 11 | `assembly_refused` | out-of-range linedef index in `BLOCKMAP` |
| `member ... uses unsupported compression method implode` | 6 | `wad_unreadable` | zip member stored with implode; the reader supports stored and deflate only |
| `failed to parse TEXTMAP: syntax error at line N, column N` | 4 | `textmap_unparseable` | UDMF text the parser refuses (expected a field name or `}` inside a block) |
| `not an archive: no zip local-header signature ...` | 3 | `archive_unreadable` | a `.zip` that is not a zip (no local header, no end-of-central-directory) |
| `map group is missing required lump VERTEXES` | 3 | `assembly_refused` | a marker-plus-lumps group that is not a map |
| `failed to parse TEXTMAP: semantic error: linedef block is missing required field 'sidefront'` | 1 | `textmap_unparseable` | UDMF that parses but is not a well-formed map |
| `SSECTORS uses an unsupported extended node encoding ZGLN` | 1 | `assembly_refused` | compressed GL extended nodes, same cause as `ZNOD` |
| `linedef index N referenced from seg is out of range (N available)` | 1 | `assembly_refused` | out-of-range linedef index in `SEGS` |
| **total** | **419** | | |

The classes reconcile with the buckets exactly: 235 `unsupported_format`, 117
`assembly_refused` (47 + 33 + 21 + 11 + 3 + 1 + 1), 23 `textmap_unparseable` (18 + 4 + 1), 35
`no_maps`, 6 `wad_unreadable`, 3 `archive_unreadable`. The two largest `assembly_refused`
classes — compressed extended nodes (48 lines) and the undersized `REJECT` (33) — are the ones
crustygen [#34](https://github.com/masriamir/crustygen/issues/34) is about: strict assembly
refuses maps a lenient pass would load, so `assembly_refused` overstates what is genuinely
unreadable.

## Before the decoration rows (commit `d6da4dc`)

### Expressibility

| Axis | All unique maps | Vanilla-only slice |
|---|---|---|
| line specials | 7.3 % | 9.3 % |
| sector specials | 60.8 % | 63.1 % |
| thing kinds | 22.9 % | 24.9 % |
| **all three** | 5.1 % | 6.6 % |

Vanilla-only slice: 77.7 % of unique maps.

### Line-special blockers

| Value | Maps | Share |
|---|---|---|
| 97 | 723 | 56.4 % |
| 62 | 616 | 48.0 % |
| 23 | 386 | 30.1 % |
| 109 | 331 | 25.8 % |
| 103 | 329 | 25.7 % |
| 88 | 326 | 25.4 % |
| 48 | 312 | 24.3 % |
| 38 | 309 | 24.1 % |
| 123 | 308 | 24.0 % |
| 31 | 263 | 20.5 % |
| 2 | 242 | 18.9 % |
| 117 | 237 | 18.5 % |
| 126 | 222 | 17.3 % |
| 112 | 201 | 15.7 % |
| 32 | 184 | 14.4 % |
| 19 | 180 | 14.0 % |
| 33 | 173 | 13.5 % |
| 63 | 160 | 12.5 % |
| 34 | 155 | 12.1 % |
| 36 | 155 | 12.1 % |
| 71 | 152 | 11.9 % |
| 102 | 136 | 10.6 % |
| 120 | 134 | 10.5 % |
| 118 | 124 | 9.7 % |
| 46 | 120 | 9.4 % |

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
| 46 | 412 | 32.1 % |
| 57 | 315 | 24.6 % |
| 54 | 287 | 22.4 % |
| 43 | 254 | 19.8 % |
| 45 | 242 | 18.9 % |
| 44 | 236 | 18.4 % |
| 48 | 228 | 17.8 % |
| 86 | 197 | 15.4 % |
| 47 | 196 | 15.3 % |
| 56 | 190 | 14.8 % |
| 26 | 188 | 14.7 % |
| 41 | 188 | 14.7 % |
| 25 | 180 | 14.0 % |
| 55 | 173 | 13.5 % |
| 70 | 166 | 12.9 % |
| 42 | 153 | 11.9 % |
| 81 | 133 | 10.4 % |
| 30 | 116 | 9.0 % |
| 63 | 116 | 9.0 % |
| 59 | 114 | 8.9 % |
| 32000 | 112 | 8.7 % |
| 31 | 104 | 8.1 % |
| 36 | 103 | 8.0 % |
| 50 | 103 | 8.0 % |
| 60 | 102 | 8.0 % |

### Greedy curve — line axis alone

Share is of all unique maps, with sector specials and thing kinds held expressible.

| k | Cumulative share of all unique maps |
|---|---|
| 1 | 8.8 % |
| 5 | 14.6 % |
| 10 | 17.7 % |
| 21 | 24.9 % |
| 51 | 43.1 % |

Order chosen: 97 → 62 → 123 → 117 → 88 → 48 → 63 → 103 → 114 → 31 → 2 → 23 → 38 → 120 → 109 → 112 → 118 → 32 → 102 → 33 → 34 → 71 → 18 → 46 → 126

### Greedy curve — conjunction (maps already ok on sectors and things)

Share is of **all unique maps**, not of the already-ok population this curve walks, so it plateaus below 100 % by exactly the maps blocked on a sector special or a thing kind.

| k | Cumulative share of all unique maps |
|---|---|
| 1 | 5.7 % |
| 5 | 7.1 % |
| 10 | 8.0 % |
| 21 | 10.2 % |
| 51 | 13.6 % |

Order chosen: 97 → 62 → 123 → 117 → 2 → 23 → 18 → 63 → 88 → 114 → 103 → 48 → 271 → 242 → 255 → 31 → 6 → 38 → 53 → 102 → 39 → 25 → 109 → 120 → 112

## After the decoration rows (commit `dbbd950`)

### Expressibility

| Axis | All unique maps | Vanilla-only slice |
|---|---|---|
| line specials | 7.3 % | 9.3 % |
| sector specials | 60.8 % | 63.1 % |
| thing kinds | 74.5 % | 81.3 % |
| **all three** | 6.5 % | 8.3 % |

Vanilla-only slice: 77.7 % of unique maps.

### Line-special blockers

| Value | Maps | Share |
|---|---|---|
| 97 | 723 | 56.4 % |
| 62 | 616 | 48.0 % |
| 23 | 386 | 30.1 % |
| 109 | 331 | 25.8 % |
| 103 | 329 | 25.7 % |
| 88 | 326 | 25.4 % |
| 48 | 312 | 24.3 % |
| 38 | 309 | 24.1 % |
| 123 | 308 | 24.0 % |
| 31 | 263 | 20.5 % |
| 2 | 242 | 18.9 % |
| 117 | 237 | 18.5 % |
| 126 | 222 | 17.3 % |
| 112 | 201 | 15.7 % |
| 32 | 184 | 14.4 % |
| 19 | 180 | 14.0 % |
| 33 | 173 | 13.5 % |
| 63 | 160 | 12.5 % |
| 34 | 155 | 12.1 % |
| 36 | 155 | 12.1 % |
| 71 | 152 | 11.9 % |
| 102 | 136 | 10.6 % |
| 120 | 134 | 10.5 % |
| 118 | 124 | 9.7 % |
| 46 | 120 | 9.4 % |

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

### Greedy curve — line axis alone

Share is of all unique maps, with sector specials and thing kinds held expressible.

| k | Cumulative share of all unique maps |
|---|---|
| 1 | 8.8 % |
| 5 | 14.6 % |
| 10 | 17.7 % |
| 21 | 24.9 % |
| 51 | 43.1 % |

Order chosen: 97 → 62 → 123 → 117 → 88 → 48 → 63 → 103 → 114 → 31 → 2 → 23 → 38 → 120 → 109 → 112 → 118 → 32 → 102 → 33 → 34 → 71 → 18 → 46 → 126

### Greedy curve — conjunction (maps already ok on sectors and things)

Share is of **all unique maps**, not of the already-ok population this curve walks, so it plateaus below 100 % by exactly the maps blocked on a sector special or a thing kind.

| k | Cumulative share of all unique maps |
|---|---|
| 1 | 7.6 % |
| 5 | 11.7 % |
| 10 | 13.9 % |
| 21 | 18.1 % |
| 51 | 27.0 % |

Order chosen: 97 → 62 → 123 → 117 → 88 → 63 → 48 → 114 → 23 → 38 → 103 → 120 → 109 → 112 → 118 → 32 → 53 → 102 → 31 → 33 → 34 → 2 → 126 → 36 → 46

## Reading

### Before, against the 2026-08-26 spike

The spike that motivated Project G was a throwaway script over a *different* seeded draw of 400
zips (1,192 maps → 1,177 unique). Its numbers and this run's "before" numbers:

| Statistic | 2026-08-26 spike | This tool, before |
|---|---|---|
| unique maps classified | 1,177 | 1,282 |
| line specials | 5.4 % | 7.3 % |
| thing kinds | ~20 % | 22.9 % |
| conjunction | 3.9 % (line ∧ thing) | 5.1 % (line ∧ sector ∧ thing) |
| vanilla-only slice | 72 % | 77.7 % |

Every axis reads higher here, and the conjunction reads higher even though this one is the
stricter statistic — three axes rather than two. That is a divergence worth recording rather
than smoothing. Candidate reasons, none of them established:

- **Different draw.** A different seed is a different set of archives. The spike drew from a
  fetch list of the same 15,273 rows, but its fetch-list hash is not recorded, so "the same
  list" is an inference from the row count rather than a checked fact. Neither run computed a
  confidence interval, so sampling noise can be neither ruled in nor ruled out as the cause of
  the gap.
- **The conjunctions are not the same statistic.** The spike's 3.9 % was line ∧ thing; this
  tool's 5.1 % adds the sector axis, which on its own passes 60.8 % of maps. Comparing them
  directly compares two different tests.
- **Different ingest, therefore different survivors.** The spike counted 33 unsupported-format
  WADs, 31 map-free WADs and 53 assembly refusals; this run refuses 235 maps for format, 35
  WADs for having no maps and 117 maps at assembly — and counts formats and refusals per *map*,
  not per WAD. The surviving populations differ in composition, not only in size, and which
  maps are excluded moves the shares.
- **Different dedup.** Both deduplicate (1,192 → 1,177 there, 1,285 → 1,282 here), but the
  spike's key is not recorded.
- **The spike is not re-runnable.** Its script was session scratch and was not kept, so the two
  runs cannot be differenced to attribute the gap to any of the above.

One candidate the doc *can* rule out: **the vocabulary was not different.** The spike recorded
the emittable line set as {1, 11, 26, 27, 28, 51, 52, 124} and 86 thing doomednums; at
`d6da4dc`, `Tables::emittable_line_specials()` yields exactly that set (door 1, exits 11/51/52/124,
locked doors 26/27/28) and `data/vocabulary.toml` carries exactly 86 integer `[things]` entries.
The two runs measured the same vocabulary.

### The noun effect

The decoration rows grew the emittable thing set from **86 to 119 doomednums** (integer entries
under `[things]` in `data/vocabulary.toml`, `d6da4dc` → `dbbd950`). Over all 1,282 unique maps:

| Axis | Before | After |
|---|---|---|
| line specials | 7.3 % | 7.3 % |
| sector specials | 60.8 % | 60.8 % |
| thing kinds | 22.9 % | 74.5 % |
| **all three** | 5.1 % | 6.5 % |

On the vanilla-only slice (77.7 % of unique maps, unchanged): thing kinds 24.9 % → 81.3 %, all
three 6.6 % → 8.3 %.

The line and sector axes are unchanged **by construction** — the rows add thing kinds only, and
`Vocabulary::classify` reads each axis from its own set, so nouns cannot move the other two.
The thing axis went from the tightest constraint of the three to the loosest: before, 77.1 %
of maps carried a thing kind outside the set; after, it passes close to three maps in four. What did not follow is a
matching jump in the conjunction, which moved 5.1 % → 6.5 % — because the line axis caps it at
7.3 %. All three is now within 0.8 points of the line axis alone: of the maps that clear the
line axis, most now clear the other two as well.

### The binding constraint is the line axis

The after-run line-special blocker ranking, unchanged from the before run (the top of the
table; the reports carry 25 rows each):

| Value | Maps | Share |
|---|---|---|
| 97 | 723 | 56.4 % |
| 62 | 616 | 48.0 % |
| 23 | 386 | 30.1 % |
| 109 | 331 | 25.8 % |
| 103 | 329 | 25.7 % |
| 88 | 326 | 25.4 % |

The greedy conjunction curve — maps already clear on sectors and things, cumulative against all
1,282 unique maps — shows what those values are now worth:

| k | Before | After |
|---|---|---|
| 1 | 5.7 % | 7.6 % |
| 5 | 7.1 % | 11.7 % |
| 10 | 8.0 % | 13.9 % |
| 21 | 10.2 % | 18.1 % |
| 51 | 13.6 % | 27.0 % |

The nouns roughly doubled the aggregate return by k = 51: the cumulative share is 1.33×, 1.65×,
1.74×, 1.77× and 1.99× the before-run share at the five checkpoints. Only those cumulative
shares are comparable — **the two curves do not add the same specials.** Each is greedy over its
own already-ok population, so they diverge from the fifth pick (before: 97 → 62 → 123 → 117 → 2;
after: 97 → 62 → 123 → 117 → 88), differ by 12 values in each direction by k = 51, and run to
different lengths before exhausting (139 steps before, 357 after). "The same 51 specials" is not
a comparison this table supports.

With the decoration set in place, the first special alone (97) takes the conjunction from 6.5 %
to 7.6 % and the first five reach 11.7 %. That ordering is the input to Project G sub-project 2:
**teleports (97) first, then lifts (62 and 88)**, taken from this table rather than from
memory.

### Residual thing blockers

After the rows, the thing-type blocker table opens at 8.7 % where it previously opened at
32.1 % — no residual value blocks even one map in ten:

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

The tail (90, 92, 94, 95, 96, 9800, 9999, 5004, 5803, 5804, 5807, 5809, 9045, 9070, 9072, 9080)
is each at or below 1.0 %.

The residue splits in two, and the split matters for what to do about it.

**Four values are vanilla, and their absence is a scoping decision already made.** 88 (65 maps),
72 (45), 87 (35) and 89 (29) are `mobjinfo` doomednums — `MT_BOSSBRAIN`, `MT_KEEN`,
`MT_BOSSTARGET` and `MT_BOSSSPIT`, read from `linuxdoom-1.10/info.c` at entries 1758, 1732,
1810 and 1784. The decoration task deliberately left them out: 72 carries `MF_COUNTKILL`, so it is a
monster rather than a prop, and 87/88/89 are the three parts of the Icon of Sin mechanism, not
scenery. They are not missing vocabulary — they are vocabulary the noun slice declined.

**The rest are non-vanilla by definition.** Vanilla `mobjinfo` doomednums run 5–89 (11 absent)
plus the 2001–2049 and 3001–3006 blocks; player starts 1–4 and 11 are handled outside
`mobjinfo`. So 0, 90, 92, 94, 95, 96, every 5xxx and 9xxx value, and 32000 lie outside that
space entirely — Boom/ZDoom/DeHackEd-range editor numbers, plus `0`, which is not a doomednum at
all and is counted because the thing axis does not skip zero. Summing map-hits over the 25
rows (521 in total): the four vanilla values account for 174, the 5xxx/9xxx/32000 ranges for
272, the 90–96 values for 51, and type 0 for 24 — so 347 of the 521 hits are non-vanilla
values that no vanilla-scoped vocabulary would ever cover.

## Re-running

In the crustywad checkout, `just harvest-sample 20260828 400` re-fetches the same draw (a
present, correctly sized zip is skipped, so it is cheap after the first run); here,
`just corpus /path/to/20260828-400` writes `docs/measurements/expressibility-<today>.md` plus a
gitignored JSON under `target/`. Exit 1 is expected, not a failure. Compare the new
Expressibility table and blocker rankings against the tables above — and re-order the Project G
queue from the new blockers, not from these.
