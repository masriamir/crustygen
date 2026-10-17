@AGENTS.md

# CLAUDE.md — crustygen

Claude-only notes for this repo. The shared, tool-neutral guidance (overview, layout, workflow,
conventions, the data-table sourcing rule, testing) lives in [`AGENTS.md`](AGENTS.md), imported
above.

Two things to keep front of mind while working here:

- **The sourcing rule is absolute.** Never write an engine constant from memory — cite it
  (`source`), derive it (`derivation`), or curate it (`curated`). A wrong constant ships a map
  that loads and misbehaves, and no test catches it because the test reads the same table. See
  `AGENTS.md` § "The data tables are the highest-stakes part".
- **[`KNOWN-GAPS.md`](KNOWN-GAPS.md) is the durable record**, not this file — read it before
  touching geometry, and add to it rather than letting a surprising decision go unexplained.

## Project tracking

Work is tracked on the shared **[Crustywad GitHub Project #5](https://github.com/users/masriamir/projects/5)**
— crustygen issues live on the same board as crustywad and crustyview, with its project-level
`Status` (`Backlog` → `Ready` → `In progress` → `In review` → `Done`) and `Horizon` (`Now` /
`Next` / `Later`) fields.

### Issue status transitions (agent-driven)

<!-- >>> meta:board-transitions -->
Move the GitHub Project board yourself as work progresses and **announce each change** in your reply ("moved #201 → In progress") rather than asking first — board edits are internal and easily reversed.

| Transition | Trigger |
|---|---|
| `Backlog → Ready` | the user says they want to start work on an issue |
| `Ready → In progress` | you begin brainstorming or drafting a plan — **before** any branch or code |
| `In progress → In review` | the PR opens |
| `In review → Done` | the PR merges/closes — **board-automated**, not manual |

`In review` holds through the entire review loop, until human review and merge. Transitions apply only to an issue that is on the board; if one exists but isn't on the board, add it first. Epics carry an **aggregate** Status: `In progress` when their first sub-issue starts work, and `Done` (board-automated) only when every sub-issue closes — set the epic's Status yourself and announce it, since GitHub rolls up completion progress but not the Status field.
<!-- <<< meta:board-transitions -->
