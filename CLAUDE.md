@AGENTS.md

# CLAUDE.md — crustygen

Claude-only notes for this repo. The shared, tool-neutral guidance (overview, layout, workflow,
conventions, the data-table sourcing rule, testing) lives in [`AGENTS.md`](AGENTS.md), imported
above. crustygen is not tracked on a GitHub Project board, so there are no board-transition
mechanics here.

Two things to keep front of mind while working here:

- **The sourcing rule is absolute.** Never write an engine constant from memory — cite it
  (`source`), derive it (`derivation`), or curate it (`curated`). A wrong constant ships a map
  that loads and misbehaves, and no test catches it because the test reads the same table. See
  `AGENTS.md` § "The data tables are the highest-stakes part".
- **[`KNOWN-GAPS.md`](KNOWN-GAPS.md) is the durable record**, not this file — read it before
  touching geometry, and add to it rather than letting a surprising decision go unexplained.
