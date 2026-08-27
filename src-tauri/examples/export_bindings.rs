//! Regenerates `src/bindings.ts` without launching the application.
//!
//! # Why this exists
//!
//! The bindings were only ever written by `lib::run`, guarded by
//! `#[cfg(debug_assertions)]` — so the sole way to refresh them was to start a
//! debug build of the GUI. Three consequences followed:
//!
//! - Adding a Tauri command and forgetting to run the app left the frontend
//!   calling a binding that does not exist.
//! - CI could not verify the file, because verifying it meant running a windowed
//!   application on a headless runner.
//! - The release build regenerates the bindings before compiling, which hid the
//!   drift from the one job that might have caught it.
//!
//! `cargo run --example export_bindings` (or `bun run bindings:generate`) writes
//! the same file the app would, from the same [`specta_builder`] definition. CI
//! runs it and fails if the committed file differs, which turns the heuristic
//! name-comparison guard in scripts/verify/check-bindings-sync.ts into an exact
//! check of types as well as names.

use speak_app_lib::specta_builder;
use specta_typescript::{BigIntExportBehavior, Typescript};

fn main() {
    // Same path and same formatting options as the in-app export, so running
    // either produces a byte-identical file.
    let out = "../src/bindings.ts";

    specta_builder()
        .export(
            Typescript::default().bigint(BigIntExportBehavior::Number),
            out,
        )
        .unwrap_or_else(|e| panic!("failed to export TypeScript bindings to {out}: {e}"));

    println!("wrote {out}");
}
