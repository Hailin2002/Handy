//! Commands behind the assistant result card.
//!
//! Phase D produces an [`AssistantAnswerEvent`] and, for `output_mode == Show`,
//! deliberately leaves the overlay window up. Everything the user can then do
//! with that answer lives here.
//!
//! # Why these are Rust commands and not frontend code
//!
//! Recording states keep the overlay non-focusable so that dictation never
//! steals the caret from whatever the user was typing in. On Windows, only the
//! answer card becomes focusable after it appears, so its text and Ctrl/Cmd+C
//! can work. That still rules out the obvious browser-only implementations:
//!
//! - `navigator.clipboard.writeText` requires the document to be focused. It
//!   would reject in exactly the situation the card exists for.
//! - Pasting needs the platform paste policy (method dispatch, the reliable
//!   receipt-sequenced path, clipboard restore, auto-submit) that
//!   [`crate::clipboard::paste`] already owns. Re-deriving any of it here would
//!   give the card a second, subtly different paste.
//!
//! So the card calls into the same machinery dictation uses, and the only thing
//! that is new is the select-all in [`assistant_replace`].

use crate::assistant::AssistantAnswerEvent;
use crate::settings::ASSISTANT_PROFILE_SMART;
use log::{debug, error, info};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tauri::AppHandle;
use tauri_specta::Event as _;
use tokio::sync::oneshot;

/// How long Ctrl/Cmd stays held after the A click in [`assistant_replace`].
/// Mirrors the modifier hold in the paste chords: most applications read the
/// modifier off the key event, but some poll global keyboard state instead.
const SELECT_ALL_HOLD_MS: u64 = 60;

/// Pause between the select-all and the paste that replaces the selection.
/// Without it the paste chord can reach an application that has not finished
/// applying the selection, which pastes *next to* the text instead of over it.
const SELECT_ALL_SETTLE_MS: u64 = 40;

/// Runs `f` on the main thread and waits for its result.
///
/// Both enigo and the clipboard are main-thread-bound on macOS, which is why
/// `actions.rs` hops before every paste. Commands, unlike the fire-and-forget
/// call sites there, have a caller waiting on an answer: the card shows a
/// confirmation, and a confirmation that appears whether or not the work
/// succeeded is worse than none at all. So the result comes back over a oneshot
/// rather than being dropped on the floor.
async fn on_main_thread<T, F>(app: &AppHandle, what: &'static str, f: F) -> Result<T, String>
where
    F: FnOnce() -> Result<T, String> + Send + 'static,
    T: Send + 'static,
{
    let (tx, rx) = oneshot::channel();
    app.run_on_main_thread(move || {
        // The receiver is only gone if the command was cancelled; the work has
        // already happened either way, so there is nothing to report.
        let _ = tx.send(f());
    })
    .map_err(|e| format!("Could not queue the {what} on the main thread: {e}"))?;

    rx.await
        .map_err(|_| format!("The {what} was dropped before it produced a result"))?
}

/// Put an assistant answer on the clipboard.
///
/// Uses the dictation clipboard writer, which owns the platform quirks (Windows
/// delayed rendering, `wl-copy` on Wayland). Nothing is restored afterwards: the
/// answer is meant to stay on the clipboard so the user can paste it themselves.
#[tauri::command]
#[specta::specta]
pub async fn assistant_copy(app: AppHandle, text: String) -> Result<(), String> {
    if text.is_empty() {
        return Err("There is nothing to copy.".to_string());
    }
    let app_for_copy = app.clone();
    on_main_thread(&app, "assistant copy", move || {
        crate::clipboard::write_text_to_clipboard(&app_for_copy, &text)
    })
    .await
    .inspect_err(|error| error!("Failed to copy the assistant answer to the clipboard: {error}"))
}

/// Insert an assistant answer into the application that owned the caret before
/// the answer card appeared.
///
/// Delegates wholesale to [`crate::clipboard::paste`], so the card obeys every
/// paste setting the user has already made — paste method, delays, clipboard
/// handling, trailing space, auto-submit.
#[tauri::command]
#[specta::specta]
pub async fn assistant_paste(app: AppHandle, text: String) -> Result<(), String> {
    if text.is_empty() {
        return Err("There is nothing to paste.".to_string());
    }

    #[cfg(target_os = "windows")]
    {
        on_main_thread(&app, "assistant target focus", || {
            crate::overlay::restore_answer_paste_target()
        })
        .await?;
        tokio::time::sleep(Duration::from_millis(40)).await;
    }

    let app_for_paste = app.clone();
    on_main_thread(&app, "assistant paste", move || {
        crate::clipboard::paste(text, app_for_paste)
    })
    .await
    .inspect_err(|error| error!("Failed to paste the assistant answer: {error}"))
}

/// Select everything in the answer card's target application, then paste the
/// answer over it.
///
/// # What this actually does, and what it cannot promise
///
/// It restores the remembered target window on Windows, then sends two
/// synthetic key chords: Ctrl+A (Cmd+A on macOS), followed by the user's
/// configured paste. It cannot know which control in that application owns the
/// caret or what that control will do with Ctrl+A.
///
/// The consequences are worth stating plainly, because this is the one command
/// in the feature that destroys data:
///
/// - **On Windows the target is the foreground window remembered when the card
///   appeared.** If it has closed or Windows refuses to reactivate it, the
///   command stops with an error. On other platforms the current focus remains
///   the target because their card does not take native focus.
/// - **Ctrl+A is not universally "select all text in this field".** In a file
///   manager it selects every file; in some editors it is bound to
///   beginning-of-line; in a browser with no field focused it selects the whole
///   page and the paste then goes nowhere.
/// - **It is not undoable by this app.** Recovery is the target application's
///   own undo, if it has one.
///
/// Nothing here can be verified before acting, so the command is kept
/// deliberately dumb and the decision is left with the user: it is a separate,
/// explicitly labelled button on the card, never a default and never automatic.
/// It refuses empty text, so a misfire cannot silently blank a document.
#[tauri::command]
#[specta::specta]
pub async fn assistant_replace(app: AppHandle, text: String) -> Result<(), String> {
    // An empty replacement is pure destruction: it would select the document and
    // overwrite it with nothing. There is no situation where that is what the
    // button meant.
    if text.is_empty() {
        return Err("There is nothing to replace the selection with.".to_string());
    }

    #[cfg(target_os = "windows")]
    {
        on_main_thread(&app, "assistant target focus", || {
            crate::overlay::restore_answer_paste_target()
        })
        .await?;
        tokio::time::sleep(Duration::from_millis(40)).await;
    }

    // Two main-thread hops with the settle wait on this task in between: the
    // select-all and the paste are main-thread-bound (enigo, clipboard), the
    // 40 ms wait is not, and sleeping inside the closure stalled the UI run
    // loop - including the overlay's own rendering - for the whole sequence.
    let app_for_select = app.clone();
    on_main_thread(&app, "assistant select-all", move || {
        crate::clipboard::with_enigo(&app_for_select, |enigo| {
            crate::input::send_select_all(enigo, SELECT_ALL_HOLD_MS)
        })
    })
    .await?;
    tokio::time::sleep(Duration::from_millis(SELECT_ALL_SETTLE_MS)).await;
    let app_for_paste = app.clone();
    on_main_thread(&app, "assistant replace", move || {
        crate::clipboard::paste(text, app_for_paste)
    })
    .await
    .inspect_err(|error| {
        error!("Failed to replace the selection with the assistant answer: {error}")
    })
}

/// Close the answer card and hide the overlay.
///
/// The card's Close button. It remains the explicit, cross-platform way to
/// dismiss a card short of starting another dictation.
#[tauri::command]
#[specta::specta]
pub fn assistant_dismiss(app: AppHandle) {
    crate::overlay::hide_answer_overlay(&app);
}

/// Raise the overlay's answer card for an event whose `expects_card` is set.
///
/// Returns whether a card is now on screen. Whether a card is *warranted* is no
/// longer decided here: it travels in the event payload, decided once at emit
/// time. Re-reading the output mode at present time meant a setting changed
/// mid-request could leave the processing spinner up with nothing replacing it.
#[tauri::command]
#[specta::specta]
pub fn assistant_present_answer(app: AppHandle) -> bool {
    crate::overlay::show_answer_overlay(&app)
}

/// Set while an escalation is in flight, so a second one cannot start.
static ESCALATION_IN_FLIGHT: AtomicBool = AtomicBool::new(false);

/// An exclusive claim on the single in-flight escalation.
///
/// The escalate button spends real money and real seconds on a large model, and
/// it lives on a card in a window with no focus ring and no hover-disabled
/// state that a keyboard user could rely on. A double click, or an impatient
/// second click while the first request is still running, would fire two
/// requests whose answers then race to overwrite each other in the card — with
/// the *slower* one winning, since it lands last.
///
/// The slot is released on drop, so an early return, a provider error or a panic
/// inside the request all leave the button usable again.
struct EscalationGuard(&'static AtomicBool);

impl EscalationGuard {
    /// Claims `slot`, or returns `None` if it is already claimed.
    fn try_acquire(slot: &'static AtomicBool) -> Option<Self> {
        slot.compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .ok()
            .map(|_| Self(slot))
    }
}

impl Drop for EscalationGuard {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

/// Ask the same question again, against the smart assistant.
///
/// Returns `true` when this call ran the request and `false` when it was
/// coalesced into one already in flight.
///
/// The answer is emitted as an ordinary [`AssistantAnswerEvent`] and nothing
/// else: no clipboard write, no paste, whatever the smart profile's output mode
/// happens to be. Escalation is a request for a better answer *in the card the
/// user is already looking at*, and it would be a surprising place to have text
/// typed into the document behind it.
#[tauri::command]
#[specta::specta]
pub async fn assistant_escalate(app: AppHandle, question: String) -> Result<bool, String> {
    let Some(_guard) = EscalationGuard::try_acquire(&ESCALATION_IN_FLIGHT) else {
        debug!("Ignoring an escalation request: one is already in flight");
        return Ok(false);
    };

    info!("Escalating an assistant question to '{ASSISTANT_PROFILE_SMART}'");

    // The card that asked. An escalation takes up to the smart profile's
    // timeout (90 s by default) to come back; by then the right question is
    // not "is a card open?" but "is *this* card still open?".
    let card = crate::overlay::answer_card_generation();

    // `run_assistant` never returns `Err`: every failure, including a missing
    // key or a timeout, comes back as an event carrying an actionable message.
    let mut event: AssistantAnswerEvent =
        crate::assistant::run_assistant(&app, ASSISTANT_PROFILE_SMART, &question).await;

    // Rule: dismissing the card cancels the escalation. The alternative -
    // raising the result anyway - puts a card the user explicitly closed back
    // on screen minutes later, over whatever they moved on to.
    if crate::overlay::answer_card_generation() != card {
        debug!("Escalated answer arrived after its card was dismissed; dropping it");
        return Ok(false);
    }

    // The card is still up, so it carries the result regardless of the smart
    // profile's own output mode: escalation is a card-native action, and a
    // `Copy`/`Paste` smart profile must not make its answer vanish.
    event.expects_card = true;
    event
        .emit(&app)
        .map_err(|e| format!("Failed to report the escalated answer: {e}"))?;

    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A slot of this test's own, so the assertions never depend on whether
    /// another test happens to be escalating at the same moment.
    static TEST_SLOT: AtomicBool = AtomicBool::new(false);

    /// The guard's whole job: the second click while a request is running must
    /// not start a second request.
    #[test]
    fn a_second_escalation_is_refused_while_one_is_in_flight() {
        let first = EscalationGuard::try_acquire(&TEST_SLOT).expect("first claim");
        assert!(
            EscalationGuard::try_acquire(&TEST_SLOT).is_none(),
            "a second escalation started while the first was in flight"
        );

        // ...and the slot is usable again once the first finishes.
        drop(first);
        let second = EscalationGuard::try_acquire(&TEST_SLOT).expect("claim after release");
        drop(second);
        assert!(
            EscalationGuard::try_acquire(&TEST_SLOT).is_some(),
            "the slot stayed claimed after its guard was dropped"
        );
        TEST_SLOT.store(false, Ordering::SeqCst);
    }

    /// A request that fails — no API key, a timeout, a panicking provider — must
    /// not wedge the button permanently. Release is on drop precisely so that
    /// every early exit path is covered without being enumerated.
    #[test]
    fn the_slot_is_released_even_when_the_holder_panics() {
        static PANIC_SLOT: AtomicBool = AtomicBool::new(false);

        let result = std::panic::catch_unwind(|| {
            let _guard = EscalationGuard::try_acquire(&PANIC_SLOT).expect("claim");
            panic!("the provider blew up");
        });

        assert!(result.is_err(), "the test's own panic did not happen");
        assert!(
            !PANIC_SLOT.load(Ordering::SeqCst),
            "a panicking escalation left the button permanently disabled"
        );
    }
}
