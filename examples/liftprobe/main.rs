//! `liftprobe` — the corpus measurements behind Project G sub-project 3
//! (lifts), sub-project 4a (floor actions) and sub-project 4b (lift
//! variants). Not a shipped tool: a reproducible probe, kept so the numbers
//! in `docs/measurements/lift-shapes-2026-08-29.md`,
//! `docs/measurements/floor-shapes-2026-09-02.md` and
//! `docs/measurements/lift-variants-2026-09-11.md` can be re-derived when
//! the sample or the loader changes.
//!
//! ```text
//! cargo run --release --example liftprobe -- census   <label> <dir-or-file>...
//! cargo run --release --example liftprobe -- shapes   <label> <dir-or-file>...
//! cargo run --release --example liftprobe -- floors   <label> <dir-or-file>...
//! cargo run --release --example liftprobe -- variants <label> <dir-or-file>...
//! ```
//!
//! `census` is the first pass (usage, rest, travel, topology, triggers,
//! rendering, conflicts, arbiter); `shapes` the second (per-shape facts and
//! multi-sector tag groups); `floors` the third (sub-project 4a: what a
//! tagged floor action is — destination, effect on the local graph, opening
//! sub-shape, triggers, rendering, chains and its own arbiter); `variants`
//! the fourth (sub-project 4b: the perpetual plat and the one-shot lift —
//! tag resolution, shape, start and stop lines, rendering and cargo,
//! concurrency, one-shot breakouts and the yield ceilings). Each `<dir-or-file>`
//! is swept non-recursively for `.zip` and `.wad` files exactly as
//! `crustygen-corpus` sweeps a sample; several directories form one
//! population, and one that names a file instead is that one archive or
//! WAD. Every pass prints Markdown to stdout; load failures go to stderr.

mod census;
mod common;
mod floors;
mod shapes;
mod variants;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.as_slice() {
        [pass, label, dirs @ ..] if !dirs.is_empty() && pass == "census" => {
            census::run(label, dirs);
        }
        [pass, label, dirs @ ..] if !dirs.is_empty() && pass == "shapes" => {
            shapes::run(label, dirs);
        }
        [pass, label, dirs @ ..] if !dirs.is_empty() && pass == "floors" => {
            floors::run(label, dirs);
        }
        [pass, label, dirs @ ..] if !dirs.is_empty() && pass == "variants" => {
            variants::run(label, dirs);
        }
        _ => {
            eprintln!("usage: liftprobe <census|shapes|floors|variants> <label> <dir-or-file>...");
            std::process::exit(2);
        }
    }
}
