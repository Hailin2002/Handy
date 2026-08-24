//! Concurrency and behavior-equivalence regression tests for the
//! transactional settings refactor (b677107 / a7226b6).
//!
//! The refactor replaced the racy `get_settings` + `write_settings`
//! read-modify-write pairs with lock-guarded `update_settings` /
//! `try_update_settings` transactions. These tests pin:
//!
//! 1. Race-freedom: concurrent competing writers resolve without lost
//!    updates or deadlock ([`Self::concurrent_competing_updates_all_survive`],
//!    [`Self::racing_single_field_updates_last_writer_wins`]).
//! 2. The stale-value re-check pattern used by model-selection fallbacks
//!    ([`Self::stale_snapshot_recheck_aborts_when_user_moved_on`]).
//! 3. Error propagation of `try_update_settings` without persisting.
//! 4. Equivalence of the dictation-adjacent microphone-fallback flow
//!    (`managers/audio.rs::persist_default_microphone_after_fallback`) with
//!    its pre-refactor contract, including its bool return value.
//!
//! The tests run the real transaction core (`store::run_transaction` /
//! `store::try_run_transaction`) — the exact functions `update_settings` and
//! `try_update_settings` delegate to, sharing the same process-wide
//! transaction lock — against an in-memory stand-in for the plugin store.

use std::collections::HashSet;
use std::sync::{mpsc, Arc, Barrier, Mutex};
use std::time::Duration;

use super::schema::{get_default_settings, AppSettings};
use super::store::{run_transaction, try_run_transaction};
use crate::managers::audio::clear_unavailable_selected_microphone;

/// In-memory stand-in for the tauri-plugin-store JSON document. Values
/// round-trip through `serde_json` on persist, mirroring what the real store
/// does, so a test cannot accidentally rely on in-memory aliasing that disk
/// persistence would break.
#[derive(Clone)]
struct FakeStore {
    stored: Arc<Mutex<serde_json::Value>>,
}

impl FakeStore {
    fn new() -> Self {
        Self {
            stored: Arc::new(Mutex::new(
                serde_json::to_value(get_default_settings()).unwrap(),
            )),
        }
    }

    fn load(&self) -> AppSettings {
        let value = self.stored.lock().unwrap().clone();
        serde_json::from_value(value).expect("fake store always holds valid settings")
    }

    fn persist(&self, settings: &AppSettings) {
        let value = serde_json::to_value(settings).unwrap();
        let reparsed: AppSettings =
            serde_json::from_value(value).expect("persisted settings must round-trip");
        *self.stored.lock().unwrap() = serde_json::to_value(&reparsed).unwrap();
    }

    fn snapshot(&self) -> AppSettings {
        self.load()
    }
}

fn fake_update<R>(store: &FakeStore, update: impl FnOnce(&mut AppSettings) -> R) -> R {
    run_transaction(|| store.load(), |settings| store.persist(settings), update)
}

fn fake_try_update<R, E>(
    store: &FakeStore,
    update: impl FnOnce(&mut AppSettings) -> Result<R, E>,
) -> Result<R, E> {
    try_run_transaction(|| store.load(), |settings| store.persist(settings), update)
}

/// Runs `body` with a watchdog: if it has not finished within the timeout the
/// test fails instead of hanging CI forever — a deadlock in the transaction
/// lock must surface as a failed assertion, not a stuck runner.
fn with_deadlock_watchdog(timeout: Duration, body: impl FnOnce() + Send + 'static) {
    let (done_tx, done_rx) = mpsc::channel::<()>();
    let joiner = std::thread::spawn(move || {
        body();
        done_tx.send(()).ok();
    });
    done_rx
        .recv_timeout(timeout)
        .expect("settings transactions deadlocked (watchdog timeout)");
    joiner.join().expect("watchdog thread must not panic");
}

/// G2: many threads perform competing read-modify-write transactions at the
/// same time. Every update must survive: no lost updates (the pre-refactor
/// get/write pairing silently dropped them) and no deadlock while threads
/// hold the transaction lock across their whole mutation. A shared counter is
/// incremented under contention; any dropped write shows up as a short final
/// count, and per-writer unique tokens prove every single transaction landed.
#[test]
fn concurrent_competing_updates_all_survive() {
    let store = FakeStore::new();
    let baseline = store.snapshot();
    const WRITERS: usize = 8;
    const UPDATES_PER_WRITER: usize = 25;

    let shared_store = Arc::new(store);
    let barrier = Arc::new(Barrier::new(WRITERS));
    let mut handles = Vec::new();

    for writer in 0..WRITERS {
        let store = Arc::clone(&shared_store);
        let barrier = Arc::clone(&barrier);
        handles.push(std::thread::spawn(move || {
            barrier.wait();
            for i in 0..UPDATES_PER_WRITER {
                let token = format!("w{writer}-{i}");
                fake_update(&store, |settings| {
                    // Yield inside the transaction to maximize interleaving
                    // pressure. Safe only because atomicity comes from the
                    // transaction lock, not from cooperativeness.
                    std::thread::yield_now();
                    settings.custom_words.push(token.clone());
                    settings.history_limit += 1;
                });
            }
        }));
    }

    with_deadlock_watchdog(Duration::from_secs(120), move || {
        for handle in handles {
            handle.join().expect("writer thread must not panic");
        }
    });

    let final_settings = shared_store.snapshot();
    let expected_increments = WRITERS * UPDATES_PER_WRITER;
    assert_eq!(
        final_settings.history_limit,
        baseline.history_limit + expected_increments,
        "lost update detected: the shared counter lost increments"
    );

    let tokens: HashSet<String> = final_settings.custom_words.iter().cloned().collect();
    assert_eq!(
        tokens.len(),
        expected_increments,
        "lost update detected: not every racing writer's token survived"
    );
    for writer in 0..WRITERS {
        for i in 0..UPDATES_PER_WRITER {
            assert!(
                tokens.contains(&format!("w{writer}-{i}")),
                "token w{writer}-{i} was lost to a racing writer"
            );
        }
    }
}

/// G2: two writers race to set the same field; exactly one value wins
/// (last-writer-wins), and the winning transaction's returned value matches
/// the persisted state — proving the return value describes a state that was
/// actually written, which callers like
/// `persist_default_microphone_after_fallback` rely on.
#[test]
fn racing_single_field_updates_last_writer_wins() {
    let store = FakeStore::new();
    let shared_store = Arc::new(store);
    let barrier = Arc::new(Barrier::new(2));

    let mut handles = Vec::new();
    for writer in 0..2 {
        let store = Arc::clone(&shared_store);
        let barrier = Arc::clone(&barrier);
        handles.push(std::thread::spawn(move || {
            barrier.wait();
            let candidate = format!("winner-{writer}");
            let persisted = fake_update(&store, |settings| {
                settings.selected_model = candidate.clone();
                candidate.clone()
            });
            (candidate, persisted)
        }));
    }

    with_deadlock_watchdog(Duration::from_secs(60), move || {
        let returns: Vec<(String, String)> =
            handles.into_iter().map(|h| h.join().unwrap()).collect();
        let final_model = shared_store.snapshot().selected_model;
        assert!(
            returns
                .iter()
                .any(|(candidate, _)| *candidate == final_model),
            "final selected_model '{final_model}' must be one of the racing writes"
        );
        let winner = returns
            .into_iter()
            .find(|(candidate, _)| *candidate == final_model)
            .expect("winner found above");
        assert_eq!(
            winner.1, winner.0,
            "the winning transaction's return value must equal what it wrote"
        );
    });
}

/// The stale-value re-check path from b677107 ("Model-selection paths
/// additionally re-check the current value inside the transaction so a stale
/// fallback cannot undo a newer user choice"): a recovery/fallback writer that
/// captured an old snapshot must abort when the live value moved on, and must
/// proceed when it did not.
#[test]
fn stale_snapshot_recheck_aborts_when_user_moved_on() {
    let store = FakeStore::new();

    // The user picked a new model while the fallback writer held a stale one.
    fake_update(&store, |settings| {
        settings.selected_model = "user-choice".into()
    });
    let changed = fake_update(&store, |settings| {
        if settings.selected_model != "stale-model" {
            return false;
        }
        settings.selected_model = "stale-model".into();
        true
    });
    assert!(!changed, "stale re-check must report 'no change'");
    assert_eq!(store.snapshot().selected_model, "user-choice");

    // Control: same re-check proceeds when the live value still matches the
    // stale snapshot.
    fake_update(&store, |settings| {
        settings.selected_model = "stale-model".into()
    });
    let changed = fake_update(&store, |settings| {
        if settings.selected_model != "stale-model" {
            return false;
        }
        settings.selected_model = "stale-model".into();
        true
    });
    assert!(changed);
    assert_eq!(store.snapshot().selected_model, "stale-model");
}

/// `try_update_settings` propagates the mutation's error verbatim to the
/// caller and persists nothing; the lock stays usable for later transactions.
#[test]
fn failed_try_update_propagates_and_persists_nothing() {
    let store = FakeStore::new();
    fake_update(&store, |settings| {
        settings.selected_language = "de".into();
        settings.history_limit += 7;
    });
    let before = store.snapshot();

    #[derive(Debug, PartialEq)]
    enum Rejection {
        Invalid,
    }

    let result: Result<(), Rejection> = fake_try_update(&store, |settings| {
        settings.selected_language = "fr".into();
        Err(Rejection::Invalid)
    });

    assert_eq!(result, Err(Rejection::Invalid));
    assert_eq!(
        serde_json::to_value(store.snapshot()).unwrap(),
        serde_json::to_value(before).unwrap(),
        "a failed try_update transaction must leave the store untouched"
    );

    // The lock was released cleanly: subsequent transactions work.
    let ok: Result<&str, Rejection> = fake_try_update(&store, |settings| {
        settings.selected_language = "fr".into();
        Ok("fr")
    });
    assert_eq!(ok, Ok("fr"));
    assert_eq!(store.snapshot().selected_language, "fr");
}

/// Sequential transactions must chain: every closure observes exactly what
/// the previous one persisted (the pre-refactor flow relied on this via
/// plain `get_settings` between writes). Both the mutation result and the
/// values seen *inside* each closure are pinned.
#[test]
fn sequential_update_closures_see_fresh_values() {
    let store = FakeStore::new();

    // What each closure actually saw on entry.
    let mut observed_languages: Vec<String> = Vec::new();
    let mut observed_limits: Vec<usize> = Vec::new();

    for i in 0..4 {
        let returned = fake_update(&store, |settings| {
            observed_languages.push(settings.selected_language.clone());
            observed_limits.push(settings.history_limit);
            settings.selected_language = format!("lang-{i}");
            settings.history_limit += 10;
            format!("lang-{i}")
        });
        // The closure's return value comes back verbatim, matching the
        // pre-refactor contract where callers read back what they wrote.
        assert_eq!(returned, format!("lang-{i}"));
    }

    assert_eq!(
        observed_languages,
        vec!["auto", "lang-0", "lang-1", "lang-2"],
        "each sequential closure must see the previous call's persisted language"
    );
    let baseline_limit = get_default_settings().history_limit;
    assert_eq!(
        observed_limits,
        (0..4).map(|i| baseline_limit + 10 * i).collect::<Vec<_>>(),
        "each sequential closure must see all prior increments"
    );

    let after = store.snapshot();
    assert_eq!(after.selected_language, "lang-3");
    assert_eq!(after.history_limit, baseline_limit + 40);

    // Same freshness guarantee for the fallible variant: a successful
    // try_update persists, so the next closure observes its effect.
    let _: Result<(), ()> = fake_try_update(&store, |settings| {
        settings.custom_words.push("w1".into());
        Ok(())
    });
    let saw_word = fake_update(&store, |settings| {
        settings.custom_words.contains(&"w1".to_string())
    });
    assert!(
        saw_word,
        "try_update's persisted write must be visible to the next closure"
    );
}

/// G2 regression proof, deterministic form: replays the pre-refactor
/// (`b677107^`) `get_settings` + `write_settings` interleaving — both callers
/// load before either persists — against the very same [`FakeStore`] the
/// lock-guarded tests use. The second persist clobbers the first: a lost
/// update. This pins the exact failure mode
/// [`Self::concurrent_competing_updates_all_survive`] guards against, so if
/// someone removes the transaction lock, that test degrades to this
/// documented behavior instead of passing vacuously.
#[test]
fn unguarded_load_then_persist_interleaving_loses_update() {
    let store_a = FakeStore::new();
    let store_b = store_a.clone();
    fake_update(&store_a, |settings| {
        settings.selected_microphone = Some("Shared Mic".into())
    });

    // Strict deterministic interleaving of the legacy two-step flow:
    //   B loads  ->  A loads, A persists  ->  B persists.
    let (b_loaded_tx, b_loaded_rx) = mpsc::channel::<()>();
    let (a_persisted_tx, a_persisted_rx) = mpsc::channel::<()>();

    let writer_b = std::thread::spawn(move || {
        let stale_snapshot = store_b.load();
        b_loaded_tx.send(()).expect("main thread alive");
        a_persisted_rx
            .recv_timeout(Duration::from_secs(30))
            .expect("writer A finished");
        // Legacy step 2: blind write of the snapshot taken before A wrote.
        let mut settings = stale_snapshot;
        settings.history_limit += 100;
        store_b.persist(&settings);
    });

    b_loaded_rx
        .recv_timeout(Duration::from_secs(30))
        .expect("writer B finished loading");
    {
        let mut settings = store_a.load();
        settings.selected_microphone = None;
        store_a.persist(&settings);
    }
    a_persisted_tx.send(()).expect("writer B alive");
    writer_b.join().expect("writer B must not panic");

    let final_settings = store_a.snapshot();
    // Writer B's increment landed...
    assert_eq!(
        final_settings.history_limit,
        get_default_settings().history_limit + 100,
    );
    // ...but writer A's microphone clearing was silently lost — exactly the
    // bug the b677107/a7226b6 transaction lock eliminates.
    assert_eq!(
        final_settings.selected_microphone,
        Some("Shared Mic".into()),
        "control: without the transaction lock this interleaving MUST lose \
         writer A's update; if this assertion ever fails, the FakeStore or \
         ordering harness changed and the lock-guarded tests no longer prove \
         anything"
    );
}

/// Reproduces the pre-refactor (`b677107^`) flow of
/// `managers/audio.rs::persist_default_microphone_after_fallback` so the new
/// flow can be compared against it field by field.
fn legacy_persist_default_microphone(store: &FakeStore, unavailable_name: &str) {
    // Old code: get_settings; early-return unless the selection still matches;
    // clear + write_settings only then.
    let mut settings = store.load();
    if settings.selected_microphone.as_deref() != Some(unavailable_name) {
        return;
    }
    settings.selected_microphone = None;
    store.persist(&settings);
}

fn store_with_microphone(name: &str) -> FakeStore {
    let store = FakeStore::new();
    fake_update(&store, |settings| {
        settings.selected_microphone = Some(name.to_string())
    });
    store
}

/// G3: pins the bool-returning contract used by
/// `persist_default_microphone_after_fallback`. When the persisted selection
/// is still the fallen-back microphone, the transaction clears it, persists
/// that, and reports `true` (the emit side effect's gate).
#[test]
fn fallback_bool_contract_clears_matching_selection_and_returns_true() {
    let store = store_with_microphone("Broken Mic");

    let changed = fake_update(&store, |settings| {
        clear_unavailable_selected_microphone(settings, "Broken Mic")
    });

    assert!(changed, "matching selection must be reported as changed");
    let after = store.snapshot();
    assert_eq!(after.selected_microphone, None);

    // Equivalence with the pre-refactor contract, applied to an identical
    // starting state.
    let legacy_store = store_with_microphone("Broken Mic");
    legacy_persist_default_microphone(&legacy_store, "Broken Mic");
    assert_eq!(
        serde_json::to_value(after).unwrap(),
        serde_json::to_value(legacy_store.snapshot()).unwrap(),
        "new flow must end in exactly the pre-refactor state"
    );
}

/// G3: when the user already chose another microphone while the stream was
/// being rebuilt, the fallback must not touch anything and must return
/// `false` — matching the pre-refactor early return (which also skipped the
/// store write and the settings-changed event).
#[test]
fn fallback_bool_contract_leaves_other_selection_untouched_and_returns_false() {
    let store = store_with_microphone("User's New Mic");
    let before = store.snapshot();

    let changed = fake_update(&store, |settings| {
        clear_unavailable_selected_microphone(settings, "Broken Mic")
    });

    assert!(!changed);
    assert_eq!(
        serde_json::to_value(store.snapshot()).unwrap(),
        serde_json::to_value(before).unwrap(),
        "a non-matching selection must not be written"
    );

    let legacy_store = store_with_microphone("User's New Mic");
    legacy_persist_default_microphone(&legacy_store, "Broken Mic");
    assert_eq!(
        serde_json::to_value(store.snapshot()).unwrap(),
        serde_json::to_value(legacy_store.snapshot()).unwrap()
    );
}

/// G3: a `None` selection (already on the system default) also yields `false`
/// without writing, exactly like the pre-refactor comparison against
/// `Some(unavailable_name)`.
#[test]
fn fallback_bool_contract_noop_for_already_default_selection() {
    let store = FakeStore::new(); // default: selected_microphone == None
    let before = store.snapshot();

    let changed = fake_update(&store, |settings| {
        clear_unavailable_selected_microphone(settings, "Broken Mic")
    });

    assert!(!changed);
    assert_eq!(
        serde_json::to_value(store.snapshot()).unwrap(),
        serde_json::to_value(before).unwrap()
    );
}

/// Compile-time pin of the public transactional API shapes that callers
/// depend on: `update_settings` returns whatever the closure returns (here
/// the `bool` consumed by the audio fallback), and `try_update_settings`
/// wraps it in `Result`. This function is never called; it fails to compile
/// if the signatures drift away from the contracts pinned above.
#[allow(dead_code)]
fn pin_public_transaction_signatures(app: &tauri::AppHandle) -> (bool, Result<bool, String>) {
    let changed: bool =
        super::update_settings(app, |s| clear_unavailable_selected_microphone(s, "x"));
    let checked: Result<bool, String> =
        super::try_update_settings(app, |s| Ok(clear_unavailable_selected_microphone(s, "x")));
    (changed, checked)
}
