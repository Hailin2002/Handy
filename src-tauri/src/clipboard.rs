use crate::input::{self, EnigoState};
#[cfg(target_os = "linux")]
use crate::settings::TypingTool;
use crate::settings::{get_settings, AutoSubmitKey, ClipboardHandling, PasteMethod};
use enigo::{Direction, Enigo, Key, Keyboard};
use log::{info, warn};
use std::fmt;
use std::process::Command;
#[cfg(target_os = "linux")]
use std::sync::OnceLock;
use std::sync::{Mutex, MutexGuard, TryLockError};
use std::time::{Duration, Instant};
use tauri::{image::Image, AppHandle, Manager};
use tauri_plugin_clipboard_manager::ClipboardExt;

#[cfg(target_os = "linux")]
use crate::utils::{is_kde_wayland, is_wayland};

/// Upper bound on how long any single paste may wait on a contended resource
/// (the enigo input lock, or the system clipboard held open by another
/// process). Every wait in the dictation paste path is derived from this, so a
/// stuck clipboard manager or input consumer degrades the paste into a typed
/// error instead of hanging the transcription pipeline indefinitely.
const CONTENDED_WAIT_TIMEOUT: Duration = Duration::from_millis(1000);
/// Poll interval while waiting on a contended resource.
const CONTENDED_POLL_INTERVAL: Duration = Duration::from_millis(20);

/// Typed failure of a clipboard-mediated paste.
///
/// Contended-clipboard behavior is deterministic: contention never hangs and
/// never silently substitutes content. Every phase either completes, retries
/// until its bound ([`CONTENDED_WAIT_TIMEOUT`]), or fails with exactly one of
/// these variants. At the module boundary the variant is rendered through
/// [`Display`](fmt::Display) into the `paste-error` message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ClipboardPasteError {
    /// A required resource stayed contended for the whole bounded retry
    /// window while `operation` ran. For `save` and `publish` the clipboard
    /// was never modified by this paste; the caller's data is untouched.
    Busy {
        operation: &'static str,
        attempts: u32,
        last_error: String,
    },
    /// Key injection failed after the transcript had been written to the
    /// clipboard. The previous content was still restored best-effort.
    InjectionFailed(String),
    /// The previous clipboard content could not be restored within the
    /// bounded retry window: the user's prior copy may be lost and the
    /// transcript may remain on the clipboard. Always accompanied by a
    /// warning log at the moment of the last failed attempt.
    RestoreFailed { attempts: u32, last_error: String },
}

impl fmt::Display for ClipboardPasteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Busy {
                operation,
                attempts,
                last_error,
            } => write!(
                f,
                "clipboard stayed busy: '{operation}' failed after {attempts} attempts \
                 within the retry window ({last_error}); paste aborted without touching \
                 the clipboard"
            ),
            Self::InjectionFailed(error) => {
                write!(f, "paste key injection failed: {error}")
            }
            Self::RestoreFailed {
                attempts,
                last_error,
            } => write!(
                f,
                "could not restore the previous clipboard content after {attempts} \
                 attempts within the retry window: {last_error}"
            ),
        }
    }
}

/// Locks `lock` with a hard upper bound. A plain `Mutex::lock` here is an
/// unbounded wait: a wedged input consumer would hang the paste forever.
fn lock_bounded<'a, T>(
    lock: &'a Mutex<T>,
    timeout: Duration,
    interval: Duration,
    what: &'static str,
) -> Result<MutexGuard<'a, T>, String> {
    let deadline = Instant::now() + timeout;
    loop {
        match lock.try_lock() {
            Ok(guard) => return Ok(guard),
            Err(TryLockError::Poisoned(_)) => {
                return Err(format!("{what} state poisoned"));
            }
            Err(TryLockError::WouldBlock) => {
                if Instant::now() >= deadline {
                    return Err(format!(
                        "{what} stayed locked by another thread for {timeout:?}; \
                         giving up instead of waiting indefinitely"
                    ));
                }
                std::thread::sleep(interval);
            }
        }
    }
}

fn with_enigo<T>(
    app_handle: &AppHandle,
    f: impl FnOnce(&mut Enigo) -> Result<T, String>,
) -> Result<T, String> {
    let enigo_state = app_handle
        .try_state::<EnigoState>()
        .ok_or("Enigo state not initialized")?;
    let mut enigo = lock_bounded(
        &enigo_state.0,
        CONTENDED_WAIT_TIMEOUT,
        CONTENDED_POLL_INTERVAL,
        "input",
    )
    .map_err(|e| format!("Failed to acquire enigo lock: {e}"))?;
    f(&mut enigo)
}

fn write_text_to_clipboard(app_handle: &AppHandle, text: &str) -> Result<(), String> {
    #[cfg(target_os = "linux")]
    if is_wayland() && is_wl_copy_available() {
        info!("Using wl-copy for clipboard write on Wayland");
        return write_clipboard_via_wl_copy(text);
    }

    app_handle
        .clipboard()
        .write_text(text)
        .map_err(|e| format!("Failed to write to clipboard: {}", e))
}

/// Snapshot of whatever the user had on the clipboard before the transcript
/// was written over it.
#[derive(Debug)]
pub(crate) enum SavedClipboard {
    Text(String),
    Image(Image<'static>),
    Empty,
}

/// Minimal clipboard surface used by the paste path. Abstracted so the
/// save / publish / restore logic (retry bounds, typed errors) can be
/// unit-tested without an app handle or a real system clipboard.
pub(crate) trait ClipboardOps {
    /// Current text, if any. `Err` means the clipboard could not be read
    /// (typically held open by another process).
    fn read_text(&self) -> Result<Option<String>, String>;
    /// Current image, if any. Only probed when there is no text.
    fn read_image(&self) -> Result<Option<Image<'static>>, String>;
    fn write_text(&self, text: &str) -> Result<(), String>;
    fn write_image(&self, image: &Image<'static>) -> Result<(), String>;
    fn clear(&self) -> Result<(), String>;
}

/// Real clipboard behind the Tauri app handle.
struct SystemClipboard<'a> {
    app_handle: &'a AppHandle,
}

impl ClipboardOps for SystemClipboard<'_> {
    fn read_text(&self) -> Result<Option<String>, String> {
        self.app_handle
            .clipboard()
            .read_text()
            .map(|t| if t.is_empty() { None } else { Some(t) })
            .map_err(|e| format!("failed to read clipboard text: {e}"))
    }

    fn read_image(&self) -> Result<Option<Image<'static>>, String> {
        self.app_handle
            .clipboard()
            .read_image()
            .map(|image| Some(image.to_owned()))
            .map_err(|e| format!("failed to read clipboard image: {e}"))
    }

    fn write_text(&self, text: &str) -> Result<(), String> {
        write_text_to_clipboard(self.app_handle, text)
    }

    fn write_image(&self, image: &Image<'static>) -> Result<(), String> {
        self.app_handle
            .clipboard()
            .write_image(image)
            .map_err(|e| format!("failed to write clipboard image: {e}"))
    }

    fn clear(&self) -> Result<(), String> {
        self.app_handle
            .clipboard()
            .clear()
            .map_err(|e| format!("failed to clear clipboard: {e}"))
    }
}

/// Retry bounds for one paste transaction's contended-resource waits.
#[derive(Clone, Copy, Debug)]
struct PasteTiming {
    paste_delay_ms: u64,
    paste_delay_after_ms: u64,
    /// Upper bound per save/publish/restore retry series.
    op_timeout: Duration,
    poll_interval: Duration,
}

impl Default for PasteTiming {
    fn default() -> Self {
        Self {
            paste_delay_ms: 0,
            paste_delay_after_ms: 0,
            op_timeout: CONTENDED_WAIT_TIMEOUT,
            poll_interval: CONTENDED_POLL_INTERVAL,
        }
    }
}

/// Runs `op` until it succeeds or the deadline passes. Always attempts at
/// least once; returns `(attempts, last_error)` on exhaustion.
fn retry_until_deadline(
    mut op: impl FnMut() -> Result<(), String>,
    timeout: Duration,
    interval: Duration,
) -> Result<u32, (u32, String)> {
    let deadline = Instant::now() + timeout;
    let mut attempts = 0u32;
    loop {
        attempts += 1;
        match op() {
            Ok(()) => return Ok(attempts),
            Err(error) => {
                if Instant::now() >= deadline {
                    return Err((attempts, error));
                }
                std::thread::sleep(interval);
            }
        }
    }
}

/// Retries a fallible clipboard read until it succeeds or the deadline
/// passes. Always attempts at least once.
fn retry_read_until_deadline<T>(
    mut read: impl FnMut() -> Result<T, String>,
    timing: &PasteTiming,
    operation: &'static str,
) -> Result<T, ClipboardPasteError> {
    let deadline = Instant::now() + timing.op_timeout;
    let mut attempts = 0u32;
    loop {
        attempts += 1;
        match read() {
            Ok(value) => return Ok(value),
            Err(last_error) => {
                if Instant::now() >= deadline {
                    return Err(ClipboardPasteError::Busy {
                        operation,
                        attempts,
                        last_error,
                    });
                }
                std::thread::sleep(timing.poll_interval);
            }
        }
    }
}

/// Reads the user's prior clipboard content. A read failure is treated as
/// contention and aborts the paste: we refuse to overwrite content we could
/// not observe first, so nothing can be silently destroyed.
fn save_clipboard(
    clipboard: &dyn ClipboardOps,
    timing: &PasteTiming,
) -> Result<SavedClipboard, ClipboardPasteError> {
    // Text takes priority so this path stays identical to the previous
    // behavior; an image is only probed when there is no text to restore.
    // Text is by far the common case, and reading an image decodes the full
    // bitmap, so this keeps the text path exactly as cheap as it was before.
    if let Some(text) = retry_read_until_deadline(|| clipboard.read_text(), timing, "save")? {
        return Ok(SavedClipboard::Text(text));
    }

    match retry_read_until_deadline(|| clipboard.read_image(), timing, "save")? {
        Some(image) => Ok(SavedClipboard::Image(image)),
        None => Ok(SavedClipboard::Empty),
    }
}

/// Writes the transcript to the clipboard with bounded retries. Failure here
/// means contention; nothing has been overwritten yet, so the typed error
/// deterministically leaves the user's clipboard exactly as it was.
fn publish_transcript(
    clipboard: &dyn ClipboardOps,
    text: &str,
    timing: &PasteTiming,
) -> Result<(), ClipboardPasteError> {
    retry_until_deadline(
        || clipboard.write_text(text),
        timing.op_timeout,
        timing.poll_interval,
    )
    .map(|_| ())
    .map_err(|(attempts, last_error)| ClipboardPasteError::Busy {
        operation: "publish",
        attempts,
        last_error,
    })
}

/// Restores the prior clipboard content with bounded retries, logging every
/// failed attempt. On exhaustion returns [`ClipboardPasteError::RestoreFailed`]
/// so the caller can surface the loss instead of reporting success while the
/// transcript still sits on the clipboard.
fn restore_clipboard(
    clipboard: &dyn ClipboardOps,
    saved: &SavedClipboard,
    timing: &PasteTiming,
) -> Result<(), ClipboardPasteError> {
    let outcome = match saved {
        SavedClipboard::Text(text) => {
            let restore = || clipboard.write_text(text);
            retry_until_deadline(restore, timing.op_timeout, timing.poll_interval)
        }
        SavedClipboard::Image(image) => {
            let restore = || clipboard.write_image(image);
            retry_until_deadline(restore, timing.op_timeout, timing.poll_interval)
        }
        SavedClipboard::Empty => {
            // Nothing was there to begin with — don't leave the transcription behind.
            let restore = || clipboard.clear();
            retry_until_deadline(restore, timing.op_timeout, timing.poll_interval)
        }
    };
    if let Err((attempts, last_error)) = &outcome {
        warn!(
            "Clipboard restore gave up after {attempts} attempts within {:?}: {last_error}",
            timing.op_timeout
        );
    }
    outcome.map(|_| ()).map_err(
        |(attempts, last_error)| ClipboardPasteError::RestoreFailed {
            attempts,
            last_error,
        },
    )
}

/// The legacy clipboard round-trip, platform-independent and testable:
///
/// 1. save the prior content (aborting on unreadable clipboard),
/// 2. publish the transcript with bounded retries,
/// 3. inject the paste chord after `paste_delay_ms`,
/// 4. restore the prior content after `paste_delay_after_ms`, with bounded
///    retries and a typed error when restoration fails.
///
/// Delivery errors take precedence over restore errors in the return value;
/// a restore failure that coexists with an injection failure is logged but
/// the injection error is what propagates.
fn run_clipboard_paste(
    text: &str,
    clipboard: &dyn ClipboardOps,
    inject_chord: impl FnOnce() -> Result<(), String>,
    timing: &PasteTiming,
) -> Result<(), ClipboardPasteError> {
    let saved = save_clipboard(clipboard, timing)?;
    publish_transcript(clipboard, text, timing)?;

    std::thread::sleep(Duration::from_millis(timing.paste_delay_ms));

    // Capture key injection errors so the original clipboard is restored before
    // they are propagated to the caller.
    let paste_result = inject_chord();

    std::thread::sleep(Duration::from_millis(timing.paste_delay_after_ms));

    let restore_result = restore_clipboard(clipboard, &saved, timing);

    match (paste_result, restore_result) {
        (Ok(()), Ok(())) => Ok(()),
        (Ok(()), Err(restore)) => Err(restore),
        (Err(injection), restore_result) => {
            if let Err(restore) = restore_result {
                warn!("Clipboard restore also failed alongside injection error: {restore}");
            }
            Err(ClipboardPasteError::InjectionFailed(injection))
        }
    }
}

/// Sends the configured paste key combo via Linux-native tools or enigo.
fn send_paste_chord(app_handle: &AppHandle, paste_method: &PasteMethod) -> Result<(), String> {
    #[cfg(target_os = "linux")]
    let key_combo_sent = try_send_key_combo_linux(paste_method)?;

    #[cfg(not(target_os = "linux"))]
    let key_combo_sent = false;

    // Fall back to enigo if no native tool handled it
    if !key_combo_sent {
        with_enigo(app_handle, |enigo| match paste_method {
            // The legacy path cannot detect a mistimed chord, so it keeps the
            // conservative 100ms modifier hold.
            PasteMethod::CtrlV => input::send_paste_ctrl_v(enigo, 100),
            PasteMethod::CtrlShiftV => input::send_paste_ctrl_shift_v(enigo, 100),
            PasteMethod::ShiftInsert => input::send_paste_shift_insert(enigo, 100),
            _ => Err("Invalid paste method for clipboard paste".into()),
        })?;
    }

    Ok(())
}

/// Pastes text using the clipboard: saves current content, writes text, sends paste keystroke, restores clipboard.
///
/// All waits are bounded ([`CONTENDED_WAIT_TIMEOUT`]); contention surfaces as
/// a [`ClipboardPasteError`] rendered into the returned message instead of a
/// hang or silent content loss.
fn paste_via_clipboard(
    text: &str,
    app_handle: &AppHandle,
    paste_method: &PasteMethod,
    paste_delay_ms: u64,
    paste_delay_after_ms: u64,
) -> Result<(), String> {
    let clipboard = SystemClipboard { app_handle };
    let timing = PasteTiming {
        paste_delay_ms,
        paste_delay_after_ms,
        ..Default::default()
    };
    run_clipboard_paste(
        text,
        &clipboard,
        || send_paste_chord(app_handle, paste_method),
        &timing,
    )
    .map_err(|error| {
        warn!("Clipboard-mediated paste failed: {error}");
        error.to_string()
    })
}

/// Attempts to send a key combination using Linux-native tools.
/// Returns `Ok(true)` if a native tool handled it, `Ok(false)` to fall back to enigo.
#[cfg(target_os = "linux")]
fn try_send_key_combo_linux(paste_method: &PasteMethod) -> Result<bool, String> {
    if is_wayland() {
        // Wayland: prefer wtype (but not on KDE), then dotool, then ydotool
        // Note: wtype doesn't work on KDE (no zwp_virtual_keyboard_manager_v1 support)
        if !is_kde_wayland() && is_wtype_available() {
            info!("Using wtype for key combo");
            send_key_combo_via_wtype(paste_method)?;
            return Ok(true);
        }
        if is_dotool_available() {
            info!("Using dotool for key combo");
            send_key_combo_via_dotool(paste_method)?;
            return Ok(true);
        }
        if is_ydotool_available() {
            info!("Using ydotool for key combo");
            send_key_combo_via_ydotool(paste_method)?;
            return Ok(true);
        }
    } else {
        // X11: prefer xdotool, then ydotool
        if is_xdotool_available() {
            info!("Using xdotool for key combo");
            send_key_combo_via_xdotool(paste_method)?;
            return Ok(true);
        }
        if is_ydotool_available() {
            info!("Using ydotool for key combo");
            send_key_combo_via_ydotool(paste_method)?;
            return Ok(true);
        }
    }

    Ok(false)
}

/// Attempts to type text directly using Linux-native tools.
/// Returns `Ok(true)` if a native tool handled it, `Ok(false)` to fall back to enigo.
#[cfg(target_os = "linux")]
fn try_direct_typing_linux(text: &str, preferred_tool: TypingTool) -> Result<bool, String> {
    // If user specified a tool, try only that one
    if preferred_tool != TypingTool::Auto {
        return match preferred_tool {
            TypingTool::Wtype if is_wtype_available() => {
                info!("Using user-specified wtype");
                type_text_via_wtype(text)?;
                Ok(true)
            }
            TypingTool::Kwtype if is_kwtype_available() => {
                info!("Using user-specified kwtype");
                type_text_via_kwtype(text)?;
                Ok(true)
            }
            TypingTool::Dotool if is_dotool_available() => {
                info!("Using user-specified dotool");
                type_text_via_dotool(text)?;
                Ok(true)
            }
            TypingTool::Ydotool if is_ydotool_available() => {
                info!("Using user-specified ydotool");
                type_text_via_ydotool(text)?;
                Ok(true)
            }
            TypingTool::Xdotool if is_xdotool_available() => {
                info!("Using user-specified xdotool");
                type_text_via_xdotool(text)?;
                Ok(true)
            }
            _ => Err(format!(
                "Typing tool {:?} is not available on this system",
                preferred_tool
            )),
        };
    }

    // Auto mode - existing fallback chain
    if is_wayland() {
        // KDE Wayland: prefer kwtype (uses KDE Fake Input protocol, supports umlauts)
        if is_kde_wayland() && is_kwtype_available() {
            info!("Using kwtype for direct text input on KDE Wayland");
            type_text_via_kwtype(text)?;
            return Ok(true);
        }
        // Wayland: prefer wtype, then dotool, then ydotool
        // Note: wtype doesn't work on KDE (no zwp_virtual_keyboard_manager_v1 support)
        if !is_kde_wayland() && is_wtype_available() {
            info!("Using wtype for direct text input");
            type_text_via_wtype(text)?;
            return Ok(true);
        }
        if is_dotool_available() {
            info!("Using dotool for direct text input");
            type_text_via_dotool(text)?;
            return Ok(true);
        }
        if is_ydotool_available() {
            info!("Using ydotool for direct text input");
            type_text_via_ydotool(text)?;
            return Ok(true);
        }
    } else {
        // X11: prefer xdotool, then ydotool
        if is_xdotool_available() {
            info!("Using xdotool for direct text input");
            type_text_via_xdotool(text)?;
            return Ok(true);
        }
        if is_ydotool_available() {
            info!("Using ydotool for direct text input");
            type_text_via_ydotool(text)?;
            return Ok(true);
        }
    }

    Ok(false)
}

/// Returns the list of available typing tools on this system.
/// Always includes "auto" as the first entry.
#[cfg(target_os = "linux")]
pub fn get_available_typing_tools() -> Vec<String> {
    let mut tools = vec!["auto".to_string()];
    if is_wtype_available() {
        tools.push("wtype".to_string());
    }
    if is_kwtype_available() {
        tools.push("kwtype".to_string());
    }
    if is_dotool_available() {
        tools.push("dotool".to_string());
    }
    if is_ydotool_available() {
        tools.push("ydotool".to_string());
    }
    if is_xdotool_available() {
        tools.push("xdotool".to_string());
    }
    tools
}

/// Check if wtype is available (Wayland text input tool)
#[cfg(target_os = "linux")]
fn is_wtype_available() -> bool {
    Command::new("which")
        .arg("wtype")
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

/// Check if dotool is available (another Wayland text input tool)
#[cfg(target_os = "linux")]
fn is_dotool_available() -> bool {
    Command::new("which")
        .arg("dotool")
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

#[cfg(target_os = "linux")]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum YdotoolKeySyntax {
    Symbolic,
    RawKeycodes,
}

#[cfg(target_os = "linux")]
const YDOTOOL_UNKNOWN_HELP_FALLBACK: YdotoolKeySyntax = YdotoolKeySyntax::RawKeycodes;

#[cfg(target_os = "linux")]
static YDOTOOL_KEY_SYNTAX: OnceLock<YdotoolKeySyntax> = OnceLock::new();

/// Classifies `ydotool key --help` output without relying on version or distro metadata.
#[cfg(target_os = "linux")]
fn classify_ydotool_key_syntax(help: &str) -> Option<YdotoolKeySyntax> {
    let help = help.to_ascii_lowercase();

    // Check modern markers first in case a future help message mentions the legacy syntax.
    if help.contains("syntax: <keycode>:<pressed>")
        || help.contains("[keycodes]")
        || help.contains("using raw keycodes")
    {
        Some(YdotoolKeySyntax::RawKeycodes)
    } else if help.contains("separated by plus (+)")
        || (help.contains("<key sequence>") && help.contains("alt+r"))
    {
        Some(YdotoolKeySyntax::Symbolic)
    } else {
        None
    }
}

/// Detects and caches a recognized ydotool key syntax. Unknown or failed probes are not cached,
/// allowing a transient daemon or PATH problem to recover on a later paste attempt.
#[cfg(target_os = "linux")]
fn detect_ydotool_key_syntax() -> YdotoolKeySyntax {
    if let Some(syntax) = YDOTOOL_KEY_SYNTAX.get() {
        return *syntax;
    }

    match Command::new("ydotool").args(["key", "--help"]).output() {
        Ok(output) => {
            // ydotool 0.x writes help to stderr and its exit status varies by build.
            let mut help = String::from_utf8_lossy(&output.stdout).into_owned();
            if !help.is_empty() && !output.stderr.is_empty() {
                help.push('\n');
            }
            help.push_str(&String::from_utf8_lossy(&output.stderr));

            if let Some(syntax) = classify_ydotool_key_syntax(&help) {
                *YDOTOOL_KEY_SYNTAX.get_or_init(|| {
                    info!("Detected ydotool key syntax: {:?}", syntax);
                    syntax
                })
            } else {
                // Preserve Handy's existing behavior and compatibility with current ydotool.
                log::warn!(
                    "Could not recognize ydotool key --help output (exit status {:?}); using raw-keycode syntax",
                    output.status.code()
                );
                YDOTOOL_UNKNOWN_HELP_FALLBACK
            }
        }
        Err(error) => {
            log::warn!(
                "Could not query ydotool key syntax: {}; using raw-keycode syntax",
                error
            );
            YDOTOOL_UNKNOWN_HELP_FALLBACK
        }
    }
}

/// Check if ydotool is available (uinput-based, works on both Wayland and X11)
#[cfg(target_os = "linux")]
fn is_ydotool_available() -> bool {
    Command::new("which")
        .arg("ydotool")
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

#[cfg(target_os = "linux")]
fn is_xdotool_available() -> bool {
    Command::new("which")
        .arg("xdotool")
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

/// Check if kwtype is available (KDE Wayland virtual keyboard input tool)
#[cfg(target_os = "linux")]
fn is_kwtype_available() -> bool {
    Command::new("which")
        .arg("kwtype")
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

/// Check if wl-copy is available (Wayland clipboard tool)
#[cfg(target_os = "linux")]
fn is_wl_copy_available() -> bool {
    Command::new("which")
        .arg("wl-copy")
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

/// Type text directly via wtype on Wayland.
#[cfg(target_os = "linux")]
fn type_text_via_wtype(text: &str) -> Result<(), String> {
    let output = Command::new("wtype")
        .arg("--") // Protect against text starting with -
        .arg(text)
        .output()
        .map_err(|e| format!("Failed to execute wtype: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("wtype failed: {}", stderr));
    }

    Ok(())
}

/// Type text directly via xdotool on X11.
#[cfg(target_os = "linux")]
fn type_text_via_xdotool(text: &str) -> Result<(), String> {
    let output = Command::new("xdotool")
        .arg("type")
        .arg("--clearmodifiers")
        .arg("--")
        .arg(text)
        .output()
        .map_err(|e| format!("Failed to execute xdotool: {}", e))?;

    // `--clearmodifiers` restores the modifiers that were held when xdotool
    // started. If the user releases one while xdotool is typing, that synthetic
    // restore can leave the modifier latched on the XTEST keyboard (#1817).
    // Release both sides of Handy's supported push-style modifiers to clear any
    // stale restore. Lock keys are intentionally excluded because key events
    // toggle them.
    //
    // The release is unconditional, so it can make a modifier that is still
    // physically held appear released until the next physical event. This is
    // preferable to leaving a synthetic modifier latched system-wide.
    //
    // Clean up before checking the typing status because xdotool may have
    // changed modifier state before returning an error. Cleanup remains
    // best-effort because the text may already have been partially or fully
    // typed, but failures are logged so they can be diagnosed.
    match Command::new("xdotool")
        .arg("keyup")
        .args([
            "Control_L",
            "Control_R",
            "Shift_L",
            "Shift_R",
            "Alt_L",
            "Alt_R",
            "Super_L",
            "Super_R",
        ])
        .output()
    {
        Ok(cleanup_output) if !cleanup_output.status.success() => {
            let stderr = String::from_utf8_lossy(&cleanup_output.stderr);
            log::warn!(
                "xdotool modifier cleanup failed with status {:?}: {}",
                cleanup_output.status.code(),
                stderr.trim()
            );
        }
        Err(error) => log::warn!("Failed to execute xdotool modifier cleanup: {}", error),
        _ => {}
    }

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("xdotool failed: {}", stderr));
    }

    Ok(())
}

/// Type text directly via dotool (works on both Wayland and X11 via uinput).
#[cfg(target_os = "linux")]
fn type_text_via_dotool(text: &str) -> Result<(), String> {
    use std::io::Write;
    use std::process::Stdio;

    let mut child = Command::new("dotool")
        .stdin(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Failed to spawn dotool: {}", e))?;

    if let Some(mut stdin) = child.stdin.take() {
        // dotool uses "type <text>" command
        writeln!(stdin, "type {}", text)
            .map_err(|e| format!("Failed to write to dotool stdin: {}", e))?;
    }

    let output = child
        .wait_with_output()
        .map_err(|e| format!("Failed to wait for dotool: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("dotool failed: {}", stderr));
    }

    Ok(())
}

/// Type text directly via ydotool (uinput-based, requires ydotoold daemon).
#[cfg(target_os = "linux")]
fn type_text_via_ydotool(text: &str) -> Result<(), String> {
    let output = Command::new("ydotool")
        .arg("type")
        .arg("--")
        .arg(text)
        .output()
        .map_err(|e| format!("Failed to execute ydotool: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("ydotool failed: {}", stderr));
    }

    Ok(())
}

/// Type text directly via kwtype (KDE Wayland virtual keyboard, uses KDE Fake Input protocol).
#[cfg(target_os = "linux")]
fn type_text_via_kwtype(text: &str) -> Result<(), String> {
    let output = Command::new("kwtype")
        .arg("--")
        .arg(text)
        .output()
        .map_err(|e| format!("Failed to execute kwtype: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("kwtype failed: {}", stderr));
    }

    Ok(())
}

/// Write text to clipboard via wl-copy (Wayland clipboard tool).
/// Uses Stdio::null() to avoid blocking on repeated calls — wl-copy forks a
/// daemon that inherits piped fds, causing read_to_end to hang indefinitely.
#[cfg(target_os = "linux")]
fn write_clipboard_via_wl_copy(text: &str) -> Result<(), String> {
    use std::process::Stdio;
    let status = Command::new("wl-copy")
        .arg("--")
        .arg(text)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|e| format!("Failed to execute wl-copy: {}", e))?;

    if !status.success() {
        return Err("wl-copy failed".into());
    }

    Ok(())
}

/// Send a key combination (e.g., Ctrl+V) via wtype on Wayland.
#[cfg(target_os = "linux")]
fn send_key_combo_via_wtype(paste_method: &PasteMethod) -> Result<(), String> {
    let args: Vec<&str> = match paste_method {
        PasteMethod::CtrlV => vec!["-M", "ctrl", "-k", "v", "-m", "ctrl"],
        PasteMethod::ShiftInsert => vec!["-M", "shift", "-k", "Insert", "-m", "shift"],
        PasteMethod::CtrlShiftV => vec![
            "-M", "ctrl", "-M", "shift", "-k", "v", "-m", "shift", "-m", "ctrl",
        ],
        _ => return Err("Unsupported paste method".into()),
    };

    let output = Command::new("wtype")
        .args(&args)
        .output()
        .map_err(|e| format!("Failed to execute wtype: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("wtype failed: {}", stderr));
    }

    Ok(())
}

/// Send a key combination (e.g., Ctrl+V) via dotool.
#[cfg(target_os = "linux")]
fn send_key_combo_via_dotool(paste_method: &PasteMethod) -> Result<(), String> {
    let command;
    match paste_method {
        PasteMethod::CtrlV => command = "echo key ctrl+v | dotool",
        PasteMethod::ShiftInsert => command = "echo key shift+insert | dotool",
        PasteMethod::CtrlShiftV => command = "echo key ctrl+shift+v | dotool",
        _ => return Err("Unsupported paste method".into()),
    }
    use std::process::Stdio;
    let status = Command::new("sh")
        .arg("-c")
        .arg(command)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|e| format!("Failed to execute dotool: {}", e))?;
    if !status.success() {
        return Err("dotool failed".into());
    }

    Ok(())
}

#[cfg(target_os = "linux")]
fn ydotool_key_args(
    paste_method: &PasteMethod,
    syntax: YdotoolKeySyntax,
) -> Result<&'static [&'static str], String> {
    let args = match (paste_method, syntax) {
        (PasteMethod::CtrlV, YdotoolKeySyntax::Symbolic) => &["key", "ctrl+v"][..],
        (PasteMethod::CtrlShiftV, YdotoolKeySyntax::Symbolic) => &["key", "ctrl+shift+v"][..],
        (PasteMethod::ShiftInsert, YdotoolKeySyntax::Symbolic) => &["key", "shift+insert"][..],
        (PasteMethod::CtrlV, YdotoolKeySyntax::RawKeycodes) => {
            &["key", "29:1", "47:1", "47:0", "29:0"][..]
        }
        (PasteMethod::CtrlShiftV, YdotoolKeySyntax::RawKeycodes) => {
            &["key", "29:1", "42:1", "47:1", "47:0", "42:0", "29:0"][..]
        }
        (PasteMethod::ShiftInsert, YdotoolKeySyntax::RawKeycodes) => {
            &["key", "42:1", "110:1", "110:0", "42:0"][..]
        }
        _ => return Err("Unsupported paste method".into()),
    };

    Ok(args)
}

/// Send a key combination (e.g., Ctrl+V) via ydotool (requires ydotoold daemon).
#[cfg(target_os = "linux")]
fn send_key_combo_via_ydotool(paste_method: &PasteMethod) -> Result<(), String> {
    let syntax = detect_ydotool_key_syntax();
    let args = ydotool_key_args(paste_method, syntax)?;

    let output = Command::new("ydotool")
        .args(args)
        .output()
        .map_err(|e| format!("Failed to execute ydotool: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("ydotool failed: {}", stderr));
    }

    Ok(())
}

/// Send a key combination (e.g., Ctrl+V) via xdotool on X11.
#[cfg(target_os = "linux")]
fn send_key_combo_via_xdotool(paste_method: &PasteMethod) -> Result<(), String> {
    let key_combo = match paste_method {
        PasteMethod::CtrlV => "ctrl+v",
        PasteMethod::CtrlShiftV => "ctrl+shift+v",
        PasteMethod::ShiftInsert => "shift+Insert",
        _ => return Err("Unsupported paste method".into()),
    };

    let output = Command::new("xdotool")
        .arg("key")
        .arg("--clearmodifiers")
        .arg(key_combo)
        .output()
        .map_err(|e| format!("Failed to execute xdotool: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("xdotool failed: {}", stderr));
    }

    Ok(())
}

/// Pastes text by invoking an external script.
/// The script receives the text to paste as a single argument.
fn paste_via_external_script(text: &str, script_path: &str) -> Result<(), String> {
    info!("Pasting via external script: {}", script_path);

    let output = Command::new(script_path)
        .arg(text)
        .output()
        .map_err(|e| format!("Failed to execute external script '{}': {}", script_path, e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        return Err(format!(
            "External script '{}' failed with exit code {:?}. stderr: {}, stdout: {}",
            script_path,
            output.status.code(),
            stderr.trim(),
            stdout.trim()
        ));
    }

    Ok(())
}

/// Types text directly by simulating individual key presses.
fn paste_direct(
    text: &str,
    app_handle: &AppHandle,
    #[cfg(target_os = "linux")] typing_tool: TypingTool,
) -> Result<(), String> {
    #[cfg(target_os = "linux")]
    {
        if try_direct_typing_linux(text, typing_tool)? {
            return Ok(());
        }
        info!("Falling back to enigo for direct text input");
    }

    with_enigo(app_handle, |enigo| input::paste_text_direct(enigo, text))
}

pub(crate) fn send_return_key(enigo: &mut Enigo, key_type: AutoSubmitKey) -> Result<(), String> {
    match key_type {
        AutoSubmitKey::Enter => {
            enigo
                .key(Key::Return, Direction::Press)
                .map_err(|e| format!("Failed to press Return key: {}", e))?;
            enigo
                .key(Key::Return, Direction::Release)
                .map_err(|e| format!("Failed to release Return key: {}", e))?;
        }
        AutoSubmitKey::CtrlEnter => {
            enigo
                .key(Key::Control, Direction::Press)
                .map_err(|e| format!("Failed to press Control key: {}", e))?;
            enigo
                .key(Key::Return, Direction::Press)
                .map_err(|e| format!("Failed to press Return key: {}", e))?;
            enigo
                .key(Key::Return, Direction::Release)
                .map_err(|e| format!("Failed to release Return key: {}", e))?;
            enigo
                .key(Key::Control, Direction::Release)
                .map_err(|e| format!("Failed to release Control key: {}", e))?;
        }
        AutoSubmitKey::CmdEnter => {
            enigo
                .key(Key::Meta, Direction::Press)
                .map_err(|e| format!("Failed to press Meta/Cmd key: {}", e))?;
            enigo
                .key(Key::Return, Direction::Press)
                .map_err(|e| format!("Failed to press Return key: {}", e))?;
            enigo
                .key(Key::Return, Direction::Release)
                .map_err(|e| format!("Failed to release Return key: {}", e))?;
            enigo
                .key(Key::Meta, Direction::Release)
                .map_err(|e| format!("Failed to release Meta/Cmd key: {}", e))?;
        }
    }

    Ok(())
}

fn should_send_auto_submit(auto_submit: bool, paste_method: PasteMethod) -> bool {
    auto_submit && paste_method != PasteMethod::None
}

pub fn paste(text: String, app_handle: AppHandle) -> Result<(), String> {
    let settings = get_settings(&app_handle);
    let paste_method = settings.paste_method;
    let paste_delay_ms = settings.paste_delay_ms;
    let paste_delay_after_ms = settings.paste_delay_after_ms;

    // Append trailing space if setting is enabled
    let text = if settings.append_trailing_space {
        format!("{} ", text)
    } else {
        text
    };

    info!(
        "Using paste method: {:?}, delay before: {}ms, delay after: {}ms",
        paste_method, paste_delay_ms, paste_delay_after_ms
    );

    // Perform the paste operation
    match paste_method {
        PasteMethod::None => {
            info!("PasteMethod::None selected - skipping paste action");
        }
        PasteMethod::Direct => {
            paste_direct(
                &text,
                &app_handle,
                #[cfg(target_os = "linux")]
                settings.typing_tool,
            )?;
        }
        PasteMethod::CtrlV | PasteMethod::CtrlShiftV | PasteMethod::ShiftInsert => {
            // Debug-gated receipt-sequenced paste (#502): restore the clipboard
            // after the target actually reads the transcript, not on a timer.
            // On success it fully handles the paste (including auto-submit and
            // clipboard handling) asynchronously; on failure fall through to
            // the legacy path untouched.
            #[cfg(any(target_os = "macos", target_os = "windows"))]
            if settings.reliable_paste {
                let reliable_result = with_enigo(&app_handle, |enigo| {
                    crate::paste_tx::try_reliable_paste(
                        &text,
                        &app_handle,
                        &paste_method,
                        enigo,
                        settings.auto_submit,
                        settings.auto_submit_key,
                        settings.clipboard_handling,
                    )
                });
                match reliable_result {
                    Ok(()) => return Ok(()),
                    Err(e) => {
                        log::warn!("Reliable paste unavailable ({e}); falling back to legacy paste")
                    }
                }
            }
            paste_via_clipboard(
                &text,
                &app_handle,
                &paste_method,
                paste_delay_ms,
                paste_delay_after_ms,
            )?
        }
        PasteMethod::ExternalScript => {
            let script_path = settings
                .external_script_path
                .as_ref()
                .filter(|p| !p.is_empty())
                .ok_or("External script path is not configured")?;
            paste_via_external_script(&text, script_path)?;
        }
    }

    if should_send_auto_submit(settings.auto_submit, paste_method) {
        std::thread::sleep(Duration::from_millis(50));
        if let Err(error) = with_enigo(&app_handle, |enigo| {
            send_return_key(enigo, settings.auto_submit_key)
        }) {
            log::warn!("Paste succeeded, but auto-submit failed: {error}");
        }
    }

    // After pasting, optionally copy to clipboard based on settings
    if settings.clipboard_handling == ClipboardHandling::CopyToClipboard {
        write_text_to_clipboard(&app_handle, &text)?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_os = "linux")]
    const YDOTOOL_0_1_8_HELP: &str = r#"
Usage: key [--delay <ms>] [--key-delay <ms>] [--repeat <times>] [--repeat-delay <ms>] <key sequence> ...
Each key sequence can be any number of modifiers and keys, separated by plus (+)
For example: alt+r Alt+F4 CTRL+alt+f3 aLT+1+2+3 ctrl+Backspace
"#;

    #[cfg(target_os = "linux")]
    const YDOTOOL_1_0_4_HELP: &str = r#"
Usage: key [OPTION]... [KEYCODES]...
Since there's no way to know how many keyboard layouts are there in the world,
we're using raw keycodes now.
Syntax: <keycode>:<pressed>
e.g. 28:1 28:0 means pressing on the Enter button on a standard US keyboard.
"#;

    #[cfg(target_os = "linux")]
    #[test]
    fn classifies_ydotool_0_1_8_symbolic_help() {
        assert_eq!(
            classify_ydotool_key_syntax(YDOTOOL_0_1_8_HELP),
            Some(YdotoolKeySyntax::Symbolic)
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn classifies_ydotool_1_0_4_raw_keycode_help() {
        assert_eq!(
            classify_ydotool_key_syntax(YDOTOOL_1_0_4_HELP),
            Some(YdotoolKeySyntax::RawKeycodes)
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn unknown_ydotool_help_falls_back_to_raw_keycodes() {
        let syntax = classify_ydotool_key_syntax("unrecognized help output")
            .unwrap_or(YDOTOOL_UNKNOWN_HELP_FALLBACK);

        assert_eq!(syntax, YdotoolKeySyntax::RawKeycodes);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn generates_symbolic_ydotool_arguments_for_all_paste_methods() {
        assert_eq!(
            ydotool_key_args(&PasteMethod::CtrlV, YdotoolKeySyntax::Symbolic).unwrap(),
            ["key", "ctrl+v"]
        );
        assert_eq!(
            ydotool_key_args(&PasteMethod::CtrlShiftV, YdotoolKeySyntax::Symbolic).unwrap(),
            ["key", "ctrl+shift+v"]
        );
        assert_eq!(
            ydotool_key_args(&PasteMethod::ShiftInsert, YdotoolKeySyntax::Symbolic).unwrap(),
            ["key", "shift+insert"]
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn generates_raw_ydotool_arguments_for_all_paste_methods() {
        assert_eq!(
            ydotool_key_args(&PasteMethod::CtrlV, YdotoolKeySyntax::RawKeycodes).unwrap(),
            ["key", "29:1", "47:1", "47:0", "29:0"]
        );
        assert_eq!(
            ydotool_key_args(&PasteMethod::CtrlShiftV, YdotoolKeySyntax::RawKeycodes).unwrap(),
            ["key", "29:1", "42:1", "47:1", "47:0", "42:0", "29:0"]
        );
        assert_eq!(
            ydotool_key_args(&PasteMethod::ShiftInsert, YdotoolKeySyntax::RawKeycodes).unwrap(),
            ["key", "42:1", "110:1", "110:0", "42:0"]
        );
    }

    #[test]
    fn auto_submit_requires_setting_enabled() {
        assert!(!should_send_auto_submit(false, PasteMethod::CtrlV));
        assert!(!should_send_auto_submit(false, PasteMethod::Direct));
    }

    #[test]
    fn auto_submit_skips_none_paste_method() {
        assert!(!should_send_auto_submit(true, PasteMethod::None));
    }

    #[test]
    fn auto_submit_runs_for_active_paste_methods() {
        assert!(should_send_auto_submit(true, PasteMethod::CtrlV));
        assert!(should_send_auto_submit(true, PasteMethod::Direct));
        assert!(should_send_auto_submit(true, PasteMethod::CtrlShiftV));
        assert!(should_send_auto_submit(true, PasteMethod::ShiftInsert));
    }

    #[test]
    fn clipboard_is_restored_before_key_injection_error_is_returned() {
        let clipboard = FakeClipboard::with_text("user secret");
        let timing = fast_timing();
        let result = run_clipboard_paste(
            "TRANSCRIPT",
            &clipboard,
            || Err("input failed".into()),
            &timing,
        );

        assert_eq!(
            result,
            Err(ClipboardPasteError::InjectionFailed("input failed".into()))
        );
        // Restore still ran before the error was returned.
        assert_eq!(clipboard.current_text(), Some("user secret".to_string()));
    }

    // ---- Fake clipboard + timing used by the paste-path tests below ----

    struct FakeClipboard {
        state: Mutex<FakeState>,
    }

    struct FakeState {
        text: Option<String>,
        image: Option<Image<'static>>,
        /// Number of remaining writes of a given text that fail (contention).
        fail_text_writes: Mutex<Vec<(String, u32)>>,
        /// Whether all reads currently fail (clipboard held open).
        fail_reads: bool,
        ops: Vec<String>,
    }

    impl FakeClipboard {
        fn empty() -> Self {
            Self {
                state: Mutex::new(FakeState {
                    text: None,
                    image: None,
                    fail_text_writes: Mutex::new(Vec::new()),
                    fail_reads: false,
                    ops: Vec::new(),
                }),
            }
        }

        fn with_text(text: &str) -> Self {
            let clipboard = Self::empty();
            clipboard.state.lock().unwrap().text = Some(text.to_string());
            clipboard
        }

        fn fail_next_writes_of(&self, text: &str, count: u32) {
            self.state
                .lock()
                .unwrap()
                .fail_text_writes
                .lock()
                .unwrap()
                .push((text.to_string(), count));
        }

        fn hold_contended(&self) {
            self.state.lock().unwrap().fail_reads = true;
        }

        fn current_text(&self) -> Option<String> {
            self.state.lock().unwrap().text.clone()
        }

        fn ops(&self) -> Vec<String> {
            self.state.lock().unwrap().ops.clone()
        }
    }

    impl ClipboardOps for FakeClipboard {
        fn read_text(&self) -> Result<Option<String>, String> {
            let mut state = self.state.lock().unwrap();
            state.ops.push("read_text".into());
            if state.fail_reads {
                return Err("clipboard held open by another process".into());
            }
            Ok(state.text.clone())
        }

        fn read_image(&self) -> Result<Option<Image<'static>>, String> {
            let mut state = self.state.lock().unwrap();
            state.ops.push("read_image".into());
            if state.fail_reads {
                return Err("clipboard held open by another process".into());
            }
            Ok(state.image.clone())
        }

        fn write_text(&self, text: &str) -> Result<(), String> {
            // Decide under the failures lock, then drop it before mutating
            // `state` (both are fields of the same outer mutex guard).
            let forced_failure = {
                let state = self.state.lock().unwrap();
                let mut failures = state.fail_text_writes.lock().unwrap();
                failures
                    .iter_mut()
                    .find(|(pattern, remaining)| *pattern == text && *remaining > 0)
                    .map(|entry| {
                        entry.1 -= 1;
                    })
                    .is_some()
            };
            let mut state = self.state.lock().unwrap();
            if forced_failure {
                let failed = format!("write_text({}) FAILED", Log(text));
                state.ops.push(failed);
                return Err("clipboard held open by another process".into());
            }
            let logged = format!("write_text({})", Log(text));
            state.ops.push(logged);
            state.text = Some(text.to_string());
            Ok(())
        }

        fn write_image(&self, _image: &Image<'static>) -> Result<(), String> {
            self.state.lock().unwrap().ops.push("write_image".into());
            Ok(())
        }

        fn clear(&self) -> Result<(), String> {
            let mut state = self.state.lock().unwrap();
            state.ops.push("clear".into());
            state.text = None;
            state.image = None;
            Ok(())
        }
    }

    /// Wraps clipboard content in test op logs without leaking transcript text.
    struct Log<'a>(&'a str);
    impl fmt::Display for Log<'_> {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "{} chars", self.0.chars().count())
        }
    }

    fn fast_timing() -> PasteTiming {
        PasteTiming {
            paste_delay_ms: 0,
            paste_delay_after_ms: 0,
            op_timeout: Duration::from_millis(60),
            poll_interval: Duration::from_millis(5),
        }
    }

    // ---- Gate G1: prior clipboard content is restored on the success path ----

    #[test]
    fn success_path_restores_prior_text_after_paste() {
        let clipboard = FakeClipboard::with_text("user secret");
        let timing = fast_timing();

        let result = run_clipboard_paste("TRANSCRIPT", &clipboard, || Ok(()), &timing);

        assert_eq!(result, Ok(()));
        // The transcript was published (9-char log entry)...
        let ops = clipboard.ops();
        assert!(ops
            .iter()
            .any(|op| op == &format!("write_text({})", Log("TRANSCRIPT"))));
        // ...and the prior content is what remains afterwards, restored last.
        assert_eq!(clipboard.current_text(), Some("user secret".to_string()));
        assert_eq!(
            ops.last().unwrap(),
            &format!("write_text({})", Log("user secret"))
        );
    }

    #[test]
    fn success_path_clears_empty_prior_clipboard() {
        let clipboard = FakeClipboard::empty();
        let timing = fast_timing();

        let result = run_clipboard_paste("TRANSCRIPT", &clipboard, || Ok(()), &timing);

        assert_eq!(result, Ok(()));
        assert_eq!(clipboard.current_text(), None);
        assert_eq!(clipboard.ops().last().unwrap(), "clear");
    }

    #[test]
    fn restore_retries_within_bound_then_succeeds() {
        let clipboard = FakeClipboard::with_text("user secret");
        // The restore write (of the saved text) fails twice, then succeeds.
        // The transcript write itself is unaffected.
        clipboard.fail_next_writes_of("user secret", 2);
        let timing = fast_timing();

        let result = run_clipboard_paste("TRANSCRIPT", &clipboard, || Ok(()), &timing);

        assert_eq!(result, Ok(()));
        assert_eq!(clipboard.current_text(), Some("user secret".to_string()));
    }

    // ---- Gate G3: contended clipboard yields a typed error, never a hang ----
    // ---- or silently pasted/replaced wrong content ----

    #[test]
    fn unreadable_clipboard_aborts_before_any_write_with_busy_error() {
        let clipboard = FakeClipboard::with_text("user secret");
        clipboard.hold_contended();
        let timing = fast_timing();
        let start = Instant::now();

        let result = run_clipboard_paste("TRANSCRIPT", &clipboard, || Ok(()), &timing);
        let elapsed = start.elapsed();

        match result {
            Err(ClipboardPasteError::Busy { operation, .. }) => {
                assert_eq!(operation, "save")
            }
            other => panic!("expected Busy(save), got {other:?}"),
        }
        // Deterministic no-loss behavior: nothing was ever published.
        assert!(!clipboard
            .ops()
            .iter()
            .any(|op| op.starts_with("write_text") && !op.contains("FAILED")));
        assert!(
            elapsed < Duration::from_secs(5),
            "wait was unbounded: {elapsed:?}"
        );
    }

    #[test]
    fn contended_publish_aborts_with_busy_error_and_keeps_prior_content() {
        let clipboard = FakeClipboard::with_text("user secret");
        clipboard.fail_next_writes_of("TRANSCRIPT", u32::MAX);
        let timing = fast_timing();
        let start = Instant::now();

        let result = run_clipboard_paste("TRANSCRIPT", &clipboard, || Ok(()), &timing);
        let elapsed = start.elapsed();

        match result {
            Err(ClipboardPasteError::Busy { operation, .. }) => {
                assert_eq!(operation, "publish")
            }
            other => panic!("expected Busy(publish), got {other:?}"),
        }
        assert_eq!(clipboard.current_text(), Some("user secret".to_string()));
        assert!(
            elapsed < Duration::from_secs(5),
            "wait was unbounded: {elapsed:?}"
        );
    }

    #[test]
    fn contended_restore_fails_with_typed_error_within_bound() {
        let clipboard = FakeClipboard::with_text("user secret");
        clipboard.fail_next_writes_of("user secret", u32::MAX);
        let timing = fast_timing();
        let start = Instant::now();

        let result = run_clipboard_paste("TRANSCRIPT", &clipboard, || Ok(()), &timing);
        let elapsed = start.elapsed();

        match result {
            Err(ClipboardPasteError::RestoreFailed { attempts, .. }) => {
                assert!(attempts >= 1)
            }
            other => panic!("expected RestoreFailed, got {other:?}"),
        }
        // Bounded: far below any plausible hang.
        assert!(
            elapsed < Duration::from_secs(5),
            "restore wait was unbounded: {elapsed:?}"
        );
    }

    // ---- Gate G2: bounded lock acquisition ----

    #[test]
    fn bounded_lock_gives_up_after_timeout_instead_of_waiting_forever() {
        let lock = Mutex::new(());
        let _guard = lock.lock().unwrap();
        let start = Instant::now();

        let result = lock_bounded(
            &lock,
            Duration::from_millis(80),
            Duration::from_millis(5),
            "input",
        );
        let elapsed = start.elapsed();

        assert!(result.is_err());
        assert!(elapsed >= Duration::from_millis(80));
        assert!(
            elapsed < Duration::from_secs(5),
            "lock wait was unbounded: {elapsed:?}"
        );
    }

    #[test]
    fn bounded_lock_succeeds_when_uncontended() {
        let lock = Mutex::new(41u8);
        let mut guard = lock_bounded(
            &lock,
            Duration::from_millis(100),
            Duration::from_millis(5),
            "input",
        )
        .unwrap();
        *guard += 1;
        assert_eq!(*guard, 42);
    }
}
