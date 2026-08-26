//! ZIP import against the committed attack corpus (plan §12, Phase 9).
//!
//! The corpus under `tests/fixtures/import-security/corpus/` is generated from
//! `docs/reviews/import-security-spec`. Its manifest states, per archive,
//! whether a safe importer must accept or reject it. These tests assert that
//! the *wired* import path — not just the validator in isolation — honours
//! that verdict, because the whole point of the hardening is that it sits
//! between the user and the AssetStore.

use serde_json::Value;
use speak_app_lib::managers::whatsapp_importer::{extract_entry, inspect_zip, ZipLimits};
use std::path::{Path, PathBuf};

fn fixtures_root() -> PathBuf {
    // `CARGO_MANIFEST_DIR` is `src-tauri`; the corpus lives beside it.
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root")
        .join("tests/fixtures/import-security")
}

fn manifest() -> Value {
    let raw = std::fs::read_to_string(fixtures_root().join("manifest.json"))
        .expect("import-security manifest");
    serde_json::from_str(&raw).expect("manifest json")
}

/// Models what the import path actually does: validate the central directory,
/// then inflate every member.
///
/// Both halves matter. Structural attacks (traversal, symlinks, bombs) are
/// caught during validation, while a corrupt local header or a wrong CRC only
/// shows up on inflation — and the corpus expects both kinds to be refused, so
/// checking validation alone would let real corruption through.
fn accepts(bytes: &[u8]) -> bool {
    let limits = ZipLimits::default();
    let Ok(entries) = inspect_zip(bytes, &limits) else {
        return false;
    };
    entries
        .iter()
        .filter(|entry| !entry.name.ends_with('/'))
        .all(|entry| extract_entry(bytes, entry, &limits).is_ok())
}

#[test]
fn every_hostile_archive_in_the_corpus_is_rejected() {
    let root = fixtures_root();
    let manifest = manifest();
    let fixtures = manifest["fixtures"].as_array().expect("fixtures array");

    let mut checked = 0;
    let mut wrongly_accepted = Vec::new();
    for fixture in fixtures {
        let file = fixture["file"].as_str().expect("fixture file");
        if !file.ends_with(".zip") {
            continue; // Text fixtures exercise the parser, not the archive path.
        }
        if fixture["expected"]["decision"].as_str() != Some("REJECT") {
            continue;
        }
        let path = root.join(file);
        let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("read {file}: {e}"));
        checked += 1;
        if accepts(&bytes) {
            wrongly_accepted.push(file.to_string());
        }
    }

    assert!(checked > 0, "no REJECT archives found in the corpus");
    assert!(
        wrongly_accepted.is_empty(),
        "these hostile archives were accepted: {wrongly_accepted:?}"
    );
}

#[test]
fn benign_archives_stay_importable() {
    let root = fixtures_root();
    let manifest = manifest();
    let fixtures = manifest["fixtures"].as_array().expect("fixtures array");

    let mut checked = 0;
    let mut wrongly_rejected = Vec::new();
    for fixture in fixtures {
        let file = fixture["file"].as_str().expect("fixture file");
        if !file.ends_with(".zip") {
            continue;
        }
        if fixture["expected"]["decision"].as_str() != Some("ACCEPT") {
            continue;
        }
        let path = root.join(file);
        let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("read {file}: {e}"));
        checked += 1;
        if !accepts(&bytes) {
            wrongly_rejected.push(file.to_string());
        }
    }

    assert!(checked > 0, "no ACCEPT archives found in the corpus");
    assert!(
        wrongly_rejected.is_empty(),
        "hardening must not break real exports: {wrongly_rejected:?}"
    );
}

/// A traversal attempt must never reach the AssetStore, which is the concrete
/// consequence the plan cares about ("Pfadtraversal, Symlinks und
/// Dekompressionsbomben erreichen den AssetStore nicht").
#[test]
fn path_traversal_never_yields_an_entry_list() {
    let bytes = std::fs::read(fixtures_root().join("corpus/attack_traversal_basic.zip"))
        .expect("traversal fixture");
    assert!(
        inspect_zip(&bytes, &ZipLimits::default()).is_err(),
        "a traversal archive must not produce entries to extract"
    );
}
