use crate::input;
use crate::settings;
use crate::settings::{OverlayPosition, OverlayStyle};
#[cfg(target_os = "windows")]
use std::sync::atomic::AtomicIsize;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize};

#[cfg(not(target_os = "macos"))]
use log::debug;

#[cfg(not(target_os = "macos"))]
use tauri::WebviewWindowBuilder;

#[cfg(target_os = "macos")]
use tauri::WebviewUrl;

#[cfg(target_os = "macos")]
use tauri_nspanel::{tauri_panel, CollectionBehavior, PanelBuilder, PanelLevel, StyleMask};

#[cfg(target_os = "linux")]
use gtk_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};

#[cfg(target_os = "linux")]
use std::env;

#[cfg(target_os = "macos")]
tauri_panel! {
    panel!(RecordingOverlayPanel {
        config: {
            can_become_key_window: false,
            is_floating_panel: true
        }
    })
}

// Native overlay window sizes (logical points). One window is reused for every
// state and resized in `show_overlay_state`; each size need only be at least as
// large as the card it hosts (the `--ov-*` vars in RecordingOverlay.css). The
// card is CSS-anchored flush to the screen edge, so window height doesn't move
// where the card sits — only OVERLAY_TOP_OFFSET / OVERLAY_BOTTOM_OFFSET do. Keep
// these in sync with the CSS card geometry.
//
// Compact overlay (Minimal / transcribing / processing): the 40h pill animates
// width from 172 (--ov-rest-w) to 216 (--ov-work-w) and expands from center, so
// the window must fit the widest state plus a little slack.
const OVERLAY_WIDTH: f64 = 256.0;
const OVERLAY_HEIGHT: f64 = 46.0;

// Actual is 394x118, just a little extra
const OVERLAY_STREAM_WIDTH: f64 = 400.0;
const OVERLAY_STREAM_HEIGHT: f64 = 120.0;

// Assistant answer card (--ov-answer-w / --ov-answer-max-h). This is the only
// overlay state the user reads rather than glances at, so it carries prose plus
// a wrapping row of actions, and it is the only one that stays up until it is
// dismissed. Same rule as the states above: the window is at least the card's
// max footprint. `overlay_window_fits_every_card` in the tests below re-derives
// these numbers from the CSS, so the two cannot drift apart silently.
const OVERLAY_ANSWER_WIDTH: f64 = 420.0;
const OVERLAY_ANSWER_HEIGHT: f64 = 288.0;

/// The overlay state name for the assistant answer card.
///
/// Shared verbatim with `RecordingOverlay.tsx`, which switches on the same
/// string arriving in the `show-overlay` payload.
const ANSWER_STATE: &str = "answer";

/// Foreground window that owned the caret before the interactive answer card
/// was clicked. Stored as an integer because Win32 HWND pointers are not Send.
#[cfg(target_os = "windows")]
static ANSWER_PASTE_TARGET: AtomicIsize = AtomicIsize::new(0);

/// Overlay window size (logical) for a given UI state.
fn overlay_dimensions(state: &str) -> (f64, f64) {
    if state == "streaming" {
        (OVERLAY_STREAM_WIDTH, OVERLAY_STREAM_HEIGHT)
    } else if state == ANSWER_STATE {
        (OVERLAY_ANSWER_WIDTH, OVERLAY_ANSWER_HEIGHT)
    } else {
        (OVERLAY_WIDTH, OVERLAY_HEIGHT)
    }
}

static LAST_MIC_LEVEL_EMIT: AtomicU64 = AtomicU64::new(0);
const EMIT_THROTTLE_MS: u64 = 33; // ~30 FPS

#[cfg(target_os = "macos")]
const OVERLAY_TOP_OFFSET: f64 = 46.0;
#[cfg(any(target_os = "windows", target_os = "linux"))]
const OVERLAY_TOP_OFFSET: f64 = 4.0;

#[cfg(target_os = "macos")]
const OVERLAY_BOTTOM_OFFSET: f64 = 15.0;

#[cfg(any(target_os = "windows", target_os = "linux"))]
const OVERLAY_BOTTOM_OFFSET: f64 = 40.0;

/// Configures the edge and offset of a GTK layer surface. gtk-layer-shell
/// commits anchor and margin changes itself, including while the surface is
/// mapped, so changing position does not require a manual hide/show cycle.
#[cfg(target_os = "linux")]
fn configure_layer_shell_position(gtk_window: &gtk::ApplicationWindow, position: OverlayPosition) {
    let (edge, opposite_edge, margin) = match position {
        OverlayPosition::Top => (Edge::Top, Edge::Bottom, OVERLAY_TOP_OFFSET),
        OverlayPosition::Bottom => (Edge::Bottom, Edge::Top, OVERLAY_BOTTOM_OFFSET),
    };

    gtk_window.set_anchor(edge, true);
    gtk_window.set_anchor(opposite_edge, false);
    gtk_window.set_layer_shell_margin(edge, margin.round() as i32);
    gtk_window.set_layer_shell_margin(opposite_edge, 0);
}

/// Configures a GTK layer surface before it is shown.
///
/// Tauri's normal `set_size` path calls `gtk_window_resize`, but layer surfaces
/// derive their dimensions from GTK's size request. gtk-layer-shell documents
/// the `set_size_request` + `resize(1, 1)` sequence for forcing a new size.
#[cfg(target_os = "linux")]
fn configure_layer_shell_surface(
    gtk_window: &gtk::ApplicationWindow,
    position: OverlayPosition,
    width: f64,
    height: f64,
) {
    use gtk::prelude::{GtkWindowExt, WidgetExt};

    configure_layer_shell_position(gtk_window, position);

    gtk_window.set_size_request(
        width.round().max(1.0) as i32,
        height.round().max(1.0) as i32,
    );
    gtk_window.resize(1, 1);
}

/// Returns true when the environment variable is set to a truthy value
/// (e.g. "1", "true", "yes", "on").
/// "0", "false", "no", "off" and empty string are treated as falsy (case-insensitive).
/// Returns false when the variable is not set.
#[cfg(target_os = "linux")]
fn env_flag_enabled(name: &str) -> bool {
    match env::var(name) {
        Ok(v) => !matches!(
            v.trim().to_ascii_lowercase().as_str(),
            "" | "0" | "false" | "no" | "off"
        ),
        Err(_) => false,
    }
}

/// Initializes GTK layer shell for Linux overlay window
/// Returns true if layer shell was successfully initialized, false otherwise
#[cfg(target_os = "linux")]
fn init_gtk_layer_shell(overlay_window: &tauri::webview::WebviewWindow) -> bool {
    if env_flag_enabled("HANDY_NO_GTK_LAYER_SHELL") {
        debug!("Skipping GTK layer shell init (HANDY_NO_GTK_LAYER_SHELL is enabled)");
        return false;
    }

    if !gtk_layer_shell::is_supported() {
        return false;
    }

    // Try to get the GTK window from the Tauri webview
    if let Ok(gtk_window) = overlay_window.gtk_window() {
        gtk_window.init_layer_shell();
        gtk_window.set_layer(Layer::Overlay);
        gtk_window.set_keyboard_mode(KeyboardMode::None);
        gtk_window.set_exclusive_zone(0);

        let overlay_position = settings::get_settings(overlay_window.app_handle()).overlay_position;
        configure_layer_shell_surface(&gtk_window, overlay_position, OVERLAY_WIDTH, OVERLAY_HEIGHT);

        let initialized = gtk_window.is_layer_window();
        LAYER_SHELL_ACTIVE.store(initialized, Ordering::SeqCst);
        return initialized;
    }
    false
}

/// Forces a window to be topmost using Win32 API (Windows only)
/// This is more reliable than Tauri's set_always_on_top which can be overridden
#[cfg(target_os = "windows")]
fn force_overlay_topmost(overlay_window: &tauri::webview::WebviewWindow) {
    use windows::Win32::UI::WindowsAndMessaging::{
        SetWindowPos, HWND_TOPMOST, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_SHOWWINDOW,
    };

    // Clone because run_on_main_thread takes 'static
    let overlay_clone = overlay_window.clone();

    // Make sure the Win32 call happens on the UI thread
    let _ = overlay_clone.clone().run_on_main_thread(move || {
        if let Ok(hwnd) = overlay_clone.hwnd() {
            unsafe {
                // Force Z-order: make this window topmost without changing size/pos or stealing focus
                let _ = SetWindowPos(
                    hwnd,
                    Some(HWND_TOPMOST),
                    0,
                    0,
                    0,
                    0,
                    SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_SHOWWINDOW,
                );
            }
        }
    });
}

#[cfg(target_os = "windows")]
fn remember_answer_paste_target(overlay_window: &tauri::webview::WebviewWindow) {
    use windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow;

    let Ok(overlay_hwnd) = overlay_window.hwnd() else {
        return;
    };
    let foreground = unsafe { GetForegroundWindow() };
    if !foreground.0.is_null() && foreground != overlay_hwnd {
        ANSWER_PASTE_TARGET.store(foreground.0 as isize, Ordering::SeqCst);
    }
}

/// Restore the application that owned the caret before the answer card took
/// focus. Card-native Paste/Replace and Ctrl+V call this before injecting keys.
#[cfg(target_os = "windows")]
pub fn restore_answer_paste_target() -> Result<(), String> {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::{IsWindow, SetForegroundWindow};

    let raw = ANSWER_PASTE_TARGET.load(Ordering::SeqCst);
    if raw == 0 {
        return Err("The application that should receive the answer is no longer available. Click the target field, then paste with Ctrl+V.".to_string());
    }
    let target = HWND(raw as *mut std::ffi::c_void);
    if !unsafe { IsWindow(Some(target)) }.as_bool() {
        ANSWER_PASTE_TARGET.store(0, Ordering::SeqCst);
        return Err("The application that should receive the answer was closed. Click a target field, then paste with Ctrl+V.".to_string());
    }
    if !unsafe { SetForegroundWindow(target) }.as_bool() {
        return Err("Windows could not return focus to the target application. Click the target field, then paste with Ctrl+V.".to_string());
    }
    Ok(())
}

fn get_monitor_with_cursor(app_handle: &AppHandle) -> Option<tauri::Monitor> {
    if let Some(mouse_location) = input::get_cursor_position(app_handle) {
        if let Ok(monitors) = app_handle.available_monitors() {
            for monitor in monitors {
                // On Windows both the cursor (enigo -> GetCursorPos) and the
                // monitor bounds are physical pixels, so compare them directly.
                #[cfg(target_os = "windows")]
                if is_mouse_within_monitor(mouse_location, monitor.position(), monitor.size()) {
                    return Some(monitor);
                }

                // macOS/Linux: enigo returns logical coords, so scale the bounds down.
                #[cfg(not(target_os = "windows"))]
                {
                    let scale = monitor.scale_factor();
                    let pos = PhysicalPosition::new(
                        (monitor.position().x as f64 / scale) as i32,
                        (monitor.position().y as f64 / scale) as i32,
                    );
                    let size = PhysicalSize::new(
                        (monitor.size().width as f64 / scale) as u32,
                        (monitor.size().height as f64 / scale) as u32,
                    );
                    if is_mouse_within_monitor(mouse_location, &pos, &size) {
                        return Some(monitor);
                    }
                }
            }
        }
    }

    app_handle.primary_monitor().ok().flatten()
}

fn is_mouse_within_monitor(
    mouse_pos: (i32, i32),
    monitor_pos: &PhysicalPosition<i32>,
    monitor_size: &PhysicalSize<u32>,
) -> bool {
    let (mouse_x, mouse_y) = mouse_pos;
    let PhysicalPosition {
        x: monitor_x,
        y: monitor_y,
    } = *monitor_pos;
    let PhysicalSize {
        width: monitor_width,
        height: monitor_height,
    } = *monitor_size;

    mouse_x >= monitor_x
        && mouse_x < (monitor_x + monitor_width as i32)
        && mouse_y >= monitor_y
        && mouse_y < (monitor_y + monitor_height as i32)
}

/// Returns overlay position in logical coordinates (points on macOS).
///
/// The Bottom anchor uses the macOS work area (visibleFrame) so the overlay
/// tracks the Dock — above it when shown, at the screen edge when hidden.
/// This relies on tauri 2.11's work_area.position.y fix (#14655), the same
/// bug that led PR #969 to abandon work_area for full monitor bounds. Top and
/// the other platforms keep full monitor bounds plus the fixed offsets
/// (work_area is unreliable on Wayland; Windows' offset clears the taskbar).
///
/// We must use LogicalPosition (not PhysicalPosition) because Tauri/tao
/// converts PhysicalPosition using the scale factor of the monitor the window
/// is *currently* on, which is wrong when moving cross-monitor. Windows uses
/// `place_windows_overlay` instead (no single logical space across mixed DPI).
fn calculate_overlay_position(
    app_handle: &AppHandle,
    width: f64,
    height: f64,
) -> Option<(f64, f64)> {
    let monitor = get_monitor_with_cursor(app_handle)?;
    let scale = monitor.scale_factor();
    let monitor_x = monitor.position().x as f64 / scale;
    let monitor_y = monitor.position().y as f64 / scale;
    let monitor_width = monitor.size().width as f64 / scale;

    let settings = settings::get_settings(app_handle);

    let x = monitor_x + (monitor_width - width) / 2.0;
    let y = match settings.overlay_position {
        OverlayPosition::Top => monitor_y + OVERLAY_TOP_OFFSET,
        OverlayPosition::Bottom => {
            // work_area.position shares monitor.position's global coordinate
            // space, so no monitor offset is added.
            #[cfg(target_os = "macos")]
            let bottom = {
                let wa = monitor.work_area();
                (wa.position.y as f64 + wa.size.height as f64) / scale
            };
            #[cfg(not(target_os = "macos"))]
            let bottom = monitor_y + monitor.size().height as f64 / scale;

            bottom - height - OVERLAY_BOTTOM_OFFSET
        }
    };

    Some((x, y))
}

/// Current overlay window size in logical units (points), for repositioning
/// without assuming a fixed size (compact vs. streaming).
#[cfg(not(target_os = "windows"))]
fn current_overlay_logical_size(window: &tauri::webview::WebviewWindow) -> Option<(f64, f64)> {
    let size = window.inner_size().ok()?;
    let scale = window.scale_factor().ok()?;
    Some((size.width as f64 / scale, size.height as f64 / scale))
}

// Which of the three window sizes the overlay currently wears. Windows
// repositions without being told the state (`update_overlay_position_on_main`),
// so the last size class is cached here. An integer rather than the previous
// is-streaming bool because there are now three sizes, and a reposition that
// guesses wrong resizes the window out from under a visible card.
#[cfg(target_os = "windows")]
static WINDOWS_OVERLAY_SIZE_CLASS: std::sync::atomic::AtomicU8 =
    std::sync::atomic::AtomicU8::new(SIZE_CLASS_COMPACT);

#[cfg(target_os = "windows")]
const SIZE_CLASS_COMPACT: u8 = 0;
#[cfg(target_os = "windows")]
const SIZE_CLASS_STREAMING: u8 = 1;
#[cfg(target_os = "windows")]
const SIZE_CLASS_ANSWER: u8 = 2;

#[cfg(target_os = "windows")]
fn windows_size_class(state: &str) -> u8 {
    if state == "streaming" {
        SIZE_CLASS_STREAMING
    } else if state == ANSWER_STATE {
        SIZE_CLASS_ANSWER
    } else {
        SIZE_CLASS_COMPACT
    }
}

/// A state name in the cached size class, for re-deriving dimensions on a
/// reposition that carries no state of its own.
#[cfg(target_os = "windows")]
fn windows_cached_state() -> &'static str {
    match WINDOWS_OVERLAY_SIZE_CLASS.load(Ordering::Relaxed) {
        SIZE_CLASS_STREAMING => "streaming",
        SIZE_CLASS_ANSWER => ANSWER_STATE,
        _ => "recording",
    }
}

/// Overlay rectangle in the destination monitor's physical pixels, so nothing
/// is converted through the window's previous-monitor DPI.
#[cfg(target_os = "windows")]
fn windows_overlay_bounds(
    monitor_position: PhysicalPosition<i32>,
    monitor_size: PhysicalSize<u32>,
    scale: f64,
    logical_width: f64,
    logical_height: f64,
    overlay_position: OverlayPosition,
) -> (i32, i32, i32, i32) {
    let width = (logical_width * scale).round().max(1.0) as i32;
    let height = (logical_height * scale).round().max(1.0) as i32;
    let x = (monitor_position.x as f64 + (monitor_size.width as f64 - width as f64) / 2.0).round()
        as i32;
    let y = match overlay_position {
        OverlayPosition::Top => {
            (monitor_position.y as f64 + OVERLAY_TOP_OFFSET * scale).round() as i32
        }
        OverlayPosition::Bottom => (monitor_position.y as f64 + monitor_size.height as f64
            - height as f64
            - OVERLAY_BOTTOM_OFFSET * scale)
            .round() as i32,
    };

    (x, y, width, height)
}

/// Moves and sizes the overlay in one native SetWindowPos, bypassing tao's
/// current-DPI logical conversion that mislands cross-monitor moves.
#[cfg(target_os = "windows")]
fn place_windows_overlay(
    app_handle: &AppHandle,
    overlay_window: &tauri::webview::WebviewWindow,
    logical_width: f64,
    logical_height: f64,
) -> Result<(), String> {
    use windows::Win32::UI::WindowsAndMessaging::{SetWindowPos, SWP_NOACTIVATE, SWP_NOZORDER};

    let monitor = get_monitor_with_cursor(app_handle)
        .ok_or_else(|| "failed to determine the monitor containing the cursor".to_string())?;
    let (x, y, width, height) = windows_overlay_bounds(
        *monitor.position(),
        *monitor.size(),
        monitor.scale_factor(),
        logical_width,
        logical_height,
        settings::get_settings(app_handle).overlay_position,
    );
    let hwnd = overlay_window
        .hwnd()
        .map_err(|error| format!("failed to get overlay window handle: {error}"))?;

    unsafe {
        SetWindowPos(
            hwnd,
            None,
            x,
            y,
            width,
            height,
            SWP_NOACTIVATE | SWP_NOZORDER,
        )
        .map_err(|error| format!("failed to set overlay bounds: {error}"))?;
    }

    log::debug!(
        "windows overlay bounds: x={} y={} width={} height={} scale={}",
        x,
        y,
        width,
        height,
        monitor.scale_factor()
    );
    Ok(())
}

/// Creates the recording overlay window and keeps it hidden by default
#[cfg(not(target_os = "macos"))]
pub fn create_recording_overlay(app_handle: &AppHandle) {
    // On Linux (Wayland), monitor detection often fails, but we don't need exact coordinates
    // for Layer Shell as we use anchors. On other platforms, we require a monitor.
    #[cfg(not(target_os = "linux"))]
    {
        let position = calculate_overlay_position(app_handle, OVERLAY_WIDTH, OVERLAY_HEIGHT);
        if position.is_none() {
            debug!("Failed to determine overlay position, not creating overlay window");
            return;
        }
    }

    // Position starts unset — update_overlay_position() sets the correct
    // LogicalPosition before the overlay is shown.
    let mut builder = WebviewWindowBuilder::new(
        app_handle,
        "recording_overlay",
        tauri::WebviewUrl::App("src/overlay/index.html".into()),
    )
    .title("Recording")
    .resizable(false)
    .inner_size(OVERLAY_WIDTH, OVERLAY_HEIGHT)
    .shadow(false)
    .maximizable(false)
    .minimizable(false)
    .closable(false)
    .accept_first_mouse(true)
    .decorations(false)
    .always_on_top(true)
    .skip_taskbar(true)
    .transparent(true)
    .focusable(false)
    .focused(false)
    .visible(false);

    if let Some(data_dir) = crate::portable::data_dir() {
        builder = builder.data_directory(data_dir.join("webview"));
    }

    #[allow(unused_variables)]
    match builder.build() {
        Ok(window) => {
            #[cfg(target_os = "linux")]
            {
                // Try to initialize GTK layer shell, ignore errors if compositor doesn't support it
                if init_gtk_layer_shell(&window) {
                    debug!("GTK layer shell initialized for overlay window");
                } else {
                    debug!("GTK layer shell not available, falling back to regular window");
                }
            }

            debug!("Recording overlay window created successfully (hidden)");
        }
        Err(e) => {
            debug!("Failed to create recording overlay window: {}", e);
        }
    }
}

/// Creates the recording overlay panel and keeps it hidden by default (macOS)
#[cfg(target_os = "macos")]
pub fn create_recording_overlay(app_handle: &AppHandle) {
    if let Some((x, y)) = calculate_overlay_position(app_handle, OVERLAY_WIDTH, OVERLAY_HEIGHT) {
        // PanelBuilder creates a Tauri window then converts it to NSPanel.
        // The window remains registered, so get_webview_window() still works.
        match PanelBuilder::<_, RecordingOverlayPanel>::new(app_handle, "recording_overlay")
            .url(WebviewUrl::App("src/overlay/index.html".into()))
            .title("Recording")
            .position(tauri::Position::Logical(tauri::LogicalPosition { x, y }))
            .level(PanelLevel::Status)
            .size(tauri::Size::Logical(tauri::LogicalSize {
                width: OVERLAY_WIDTH,
                height: OVERLAY_HEIGHT,
            }))
            .has_shadow(false)
            .transparent(true)
            .no_activate(true)
            .corner_radius(0.0)
            .style_mask(StyleMask::empty().borderless().nonactivating_panel())
            .with_window(|w| w.decorations(false).transparent(true).focusable(false))
            .collection_behavior(
                CollectionBehavior::new()
                    .can_join_all_spaces()
                    .full_screen_auxiliary(),
            )
            .build()
        {
            Ok(panel) => {
                panel.hide();
            }
            Err(e) => {
                log::error!("Failed to create recording overlay panel: {}", e);
            }
        }
    }
}

fn show_overlay_state(app_handle: &AppHandle, state: &str) {
    // Whether the overlay shows at all is governed by overlay_style; position
    // only chooses Top vs Bottom placement. Checked here (off the main thread)
    // so the common overlay-disabled case never pays for a main-thread hop.
    let settings = settings::get_settings(app_handle);
    if settings.overlay_style == OverlayStyle::None {
        return;
    }

    // Claim or release the answer card's hold on the window. Deliberately after
    // the early return above: with overlays switched off nothing is ever shown,
    // and a flag set for a window that never appears would suppress every later
    // hide forever. Any other state supersedes an open card, which is what makes
    // starting a new dictation a way out of one.
    set_answer_card_open(state == ANSWER_STATE);

    // A state was raised, so anything that was waiting on the *previous* state
    // (the answer-card watchdog in `actions.rs`) must not act on it any more.
    OVERLAY_GENERATION.fetch_add(1, Ordering::SeqCst);

    // The rest queries monitors and the cursor and mutates window geometry. On
    // Linux the monitor/cursor lookups hit GDK/Xlib on the process's shared X11
    // connection, which is only safe from the GTK main thread — running them on
    // a background thread corrupts the connection and hard-crashes the app
    // (issue #227). Hop to the main thread on every platform to keep the
    // geometry path uniform (a no-op cost on Windows, and it also keeps macOS's
    // NSScreen access main-thread-correct). run_on_main_thread runs the closure
    // inline when already on the main thread, so this never deadlocks.
    let handle = app_handle.clone();
    let state = state.to_string();
    let _ = app_handle.run_on_main_thread(move || show_overlay_state_on_main(&handle, &state));
}

fn show_overlay_state_on_main(app_handle: &AppHandle, state: &str) {
    // Size the overlay for this state (compact vs. streaming), then position it.
    let (width, height) = overlay_dimensions(state);
    if let Some(overlay_window) = app_handle.get_webview_window("recording_overlay") {
        // Recording overlays must never steal the caret from the target app.
        // The answer card is different: the user explicitly wants to interact
        // with its text and use Ctrl/Cmd+C. It still opens with NOACTIVATE on
        // Windows; focus moves only when the user clicks the card.
        #[cfg(target_os = "windows")]
        {
            if state == ANSWER_STATE {
                remember_answer_paste_target(&overlay_window);
            }
            if let Err(error) = overlay_window.set_focusable(state == ANSWER_STATE) {
                log::warn!("Failed to update overlay focusability for '{state}': {error}");
            }
        }
        #[cfg(target_os = "linux")]
        let shown_with_layer_shell = if LAYER_SHELL_ACTIVE.load(Ordering::SeqCst) {
            let position = settings::get_settings(app_handle).overlay_position;
            match overlay_window.gtk_window() {
                Ok(gtk_window) => {
                    configure_layer_shell_surface(&gtk_window, position, width, height)
                }
                Err(error) => log::error!("Failed to access GTK overlay window: {error}"),
            }
            let _ = overlay_window.show();
            true
        } else {
            false
        };
        #[cfg(not(target_os = "linux"))]
        let shown_with_layer_shell = false;

        if !shown_with_layer_shell {
            let size_started = std::time::Instant::now();
            #[cfg(not(target_os = "windows"))]
            let _ =
                overlay_window.set_size(tauri::Size::Logical(tauri::LogicalSize { width, height }));
            #[cfg(target_os = "windows")]
            WINDOWS_OVERLAY_SIZE_CLASS.store(windows_size_class(state), Ordering::Relaxed);
            let size_elapsed = size_started.elapsed();

            let pos_started = std::time::Instant::now();
            #[cfg(not(target_os = "windows"))]
            let set_pos_elapsed =
                if let Some((x, y)) = calculate_overlay_position(app_handle, width, height) {
                    let set_pos_started = std::time::Instant::now();
                    let _ = overlay_window
                        .set_position(tauri::Position::Logical(tauri::LogicalPosition { x, y }));
                    set_pos_started.elapsed()
                } else {
                    std::time::Duration::ZERO
                };
            #[cfg(target_os = "windows")]
            let set_pos_elapsed = {
                let set_pos_started = std::time::Instant::now();
                if let Err(error) =
                    place_windows_overlay(app_handle, &overlay_window, width, height)
                {
                    log::error!("Failed to place recording overlay: {error}");
                }
                set_pos_started.elapsed()
            };
            let pos_calc_elapsed = pos_started.elapsed() - set_pos_elapsed;

            let show_started = std::time::Instant::now();
            let _ = overlay_window.show();
            let show_elapsed = show_started.elapsed();

            // On Windows, aggressively re-assert "topmost" in the native Z-order after showing
            #[cfg(target_os = "windows")]
            force_overlay_topmost(&overlay_window);

            // Re-assert bounds after show(): the pre-show move crosses the DPI
            // boundary, and tao's WM_DPICHANGED reflow clobbers the first placement.
            #[cfg(target_os = "windows")]
            if let Err(error) = place_windows_overlay(app_handle, &overlay_window, width, height) {
                log::error!("Failed to re-assert recording overlay position: {error}");
            }

            log::debug!(
                "overlay '{}': set_size={:?} pos_calc={:?} set_pos={:?} show={:?}",
                state,
                size_elapsed,
                pos_calc_elapsed,
                set_pos_elapsed,
                show_elapsed
            );
        }

        let _ = overlay_window.emit(crate::events::SHOW_OVERLAY, state);
    }
}

/// Notify the visible recording overlay that the input stream has delivered its
/// first sample chunk. Audio feedback uses the same backend readiness signal,
/// but this targeted event is skipped when overlays are disabled.
pub fn emit_recording_ready(app_handle: &AppHandle) {
    if !OVERLAY_ENABLED.load(Ordering::Relaxed) {
        return;
    }

    // Showing the overlay is also queued onto the main thread. Queue readiness
    // there as well so a very fast always-on stream cannot overtake show-overlay
    // and then get reset back to the arming state by the frontend.
    let handle = app_handle.clone();
    let _ = app_handle.run_on_main_thread(move || {
        let _ = handle.emit_to("recording_overlay", crate::events::RECORDING_READY, ());
    });
}

/// Shows the recording overlay window with fade-in animation
pub fn show_recording_overlay(app_handle: &AppHandle) {
    show_overlay_state(app_handle, "recording");
}

/// Shows the larger streaming overlay that displays live transcription text
pub fn show_streaming_overlay(app_handle: &AppHandle) {
    show_overlay_state(app_handle, "streaming");
}

/// Shows the meeting overlay: status and duration while a meeting records.
///
/// Plan §6.20 requires the small overlay to keep reporting a meeting even with
/// the main window closed. It carries status and duration only — pause and stop
/// live in the tray and the window, because the overlay is deliberately
/// non-focusable and must not become a second, competing control surface that
/// could be confused with dictation (§6.10).
pub fn show_meeting_overlay(app_handle: &AppHandle, paused: bool) {
    show_overlay_state(
        app_handle,
        if paused { "meeting_paused" } else { "meeting" },
    );
}

/// Hides the meeting overlay again once capture ended.
pub fn hide_meeting_overlay(app_handle: &AppHandle) {
    hide_recording_overlay(app_handle);
}

/// Shows the transcribing overlay window
pub fn show_transcribing_overlay(app_handle: &AppHandle) {
    show_overlay_state(app_handle, "transcribing");
}

/// Shows the processing overlay window
pub fn show_processing_overlay(app_handle: &AppHandle) {
    show_overlay_state(app_handle, "processing");
}

/// Shows the assistant answer card in the overlay window.
///
/// Returns whether the overlay could be shown at all. It cannot when the user
/// has turned the overlay off entirely (`OverlayStyle::None`), and the caller
/// needs to know: the card is the whole output of an assistant run in `Show`
/// mode, so a caller that assumed it appeared would report a delivered answer
/// the user never saw.
///
/// Unlike every other state, this one does not end when the flow that started it
/// ends — see [`ANSWER_CARD_OPEN`].
pub fn show_answer_overlay(app_handle: &AppHandle) -> bool {
    if !answer_card_can_be_shown(app_handle) {
        return false;
    }
    show_overlay_state(app_handle, ANSWER_STATE);
    true
}

/// Whether an answer card could be put on screen right now.
///
/// The one thing that makes it impossible is the user having turned the overlay
/// off (`OverlayStyle::None`, which is also the shipped default on Linux). This
/// has to be answerable *before* a run decides how to deliver its answer:
/// `actions.rs` routes a `Show` run to the clipboard instead when the card that
/// was supposed to carry it cannot appear, and the alternative — planning a card
/// and discovering at present time that there is none — is how an answer ends up
/// delivered nowhere at all.
pub fn answer_card_can_be_shown(app_handle: &AppHandle) -> bool {
    settings::get_settings(app_handle).overlay_style != OverlayStyle::None
}

/// Dismisses an open assistant answer card and hides the overlay.
///
/// The only way to close a card other than superseding it with another overlay
/// state, and therefore the only caller that is allowed past the guard in
/// [`hide_recording_overlay`].
pub fn hide_answer_overlay(app_handle: &AppHandle) {
    set_answer_card_open(false);
    hide_recording_overlay(app_handle);
}

/// Updates the overlay window position based on current settings
pub fn update_overlay_position(app_handle: &AppHandle) {
    // Positioning queries monitors/cursor (GDK/Xlib on Linux) and moves the
    // window, so it must run on the main thread — see show_overlay_state.
    let handle = app_handle.clone();
    let _ = app_handle.run_on_main_thread(move || update_overlay_position_on_main(&handle));
}

fn update_overlay_position_on_main(app_handle: &AppHandle) {
    if let Some(overlay_window) = app_handle.get_webview_window("recording_overlay") {
        #[cfg(target_os = "linux")]
        if LAYER_SHELL_ACTIVE.load(Ordering::SeqCst) {
            let position = settings::get_settings(app_handle).overlay_position;
            match overlay_window.gtk_window() {
                Ok(gtk_window) => configure_layer_shell_position(&gtk_window, position),
                Err(error) => log::error!("Failed to access GTK overlay window: {error}"),
            }
            return;
        }

        #[cfg(target_os = "windows")]
        {
            let state = windows_cached_state();
            let (width, height) = overlay_dimensions(state);
            if let Err(error) = place_windows_overlay(app_handle, &overlay_window, width, height) {
                log::error!("Failed to update recording overlay position: {error}");
            }
        }

        #[cfg(not(target_os = "windows"))]
        {
            // Use the window's current size so centering stays correct whether the
            // overlay is in compact or streaming layout.
            let (width, height) = current_overlay_logical_size(&overlay_window)
                .unwrap_or((OVERLAY_WIDTH, OVERLAY_HEIGHT));
            if let Some((x, y)) = calculate_overlay_position(app_handle, width, height) {
                let _ = overlay_window
                    .set_position(tauri::Position::Logical(tauri::LogicalPosition { x, y }));
            }
        }
    }
}

/// Hides the recording overlay window with fade-out animation
pub fn hide_recording_overlay(app_handle: &AppHandle) {
    // An open answer card outlives the flow that put the overlay on screen, so
    // this is a no-op while one is up. See [`ANSWER_CARD_OPEN`] for the full
    // list of callers this protects the card from.
    if ANSWER_CARD_OPEN.load(Ordering::SeqCst) {
        return;
    }

    // Always hide the overlay regardless of settings - if setting was changed while recording,
    // we still want to hide it properly
    if let Some(overlay_window) = app_handle.get_webview_window("recording_overlay") {
        // Re-read the flag immediately before emitting rather than trusting the
        // check at the top of the function. `get_webview_window` is a lookup
        // through the app's window map, and a card raised in between would get
        // its `answer` payload cleared by the `hide-overlay` listener while the
        // native window stays up — a transparent, click-eating panel.
        if ANSWER_CARD_OPEN.load(Ordering::SeqCst) {
            return;
        }
        // Emit event to trigger fade-out animation
        let _ = overlay_window.emit(crate::events::HIDE_OVERLAY, ());
        // Hide the window after a short delay to allow animation to complete
        let window_clone = overlay_window.clone();
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(300));
            // Re-checked, not just checked on the way in. 300 ms is long enough
            // for a card to be raised inside this fade, and one call site does
            // exactly that: the assistant failure paths in `actions.rs` emit the
            // event and *then* hide, so the frontend raises the error card while
            // this thread is still sleeping. Without this second look the card
            // would appear and vanish a quarter-second later, which reads as a
            // flicker rather than as an error message.
            if ANSWER_CARD_OPEN.load(Ordering::SeqCst) {
                return;
            }
            let _ = window_clone.hide();
        });
    }
}

/// True while an assistant answer card owns the overlay window.
///
/// Every other overlay state is transient and is torn down by the flow that
/// raised it: dictation finishing (`actions.rs`), a meeting stopping
/// (`managers/meeting.rs`), a cancel (`utils::cancel_current_operation`). All of
/// them funnel into [`hide_recording_overlay`]. An answer card is different — it
/// is prose the user is in the middle of reading, and any of those firing
/// underneath it (a meeting ending on a timer, a late paste callback from the
/// dictation that preceded the question) would make the answer vanish
/// mid-sentence with no way to get it back, because assistant runs are
/// deliberately kept out of the history.
///
/// So the ordinary hide becomes a no-op for as long as this is set. A card is
/// closed either by [`hide_answer_overlay`] — the card's own Close button, via
/// the `assistant_dismiss` command — or by being superseded, since
/// [`show_overlay_state`] clears the flag for every state that is not the card.
static ANSWER_CARD_OPEN: AtomicBool = AtomicBool::new(false);

/// Incremented on every open/close transition of [`ANSWER_CARD_OPEN`].
///
/// Identity for a card, which the boolean alone cannot give. An escalation
/// started from a card takes up to its profile's timeout (90 s by default) to
/// come back, and by then "is a card open?" is the wrong question — the right
/// one is "is *the* card still open?". Without this, closing a card and
/// starting a second question would let the first card's escalation land in the
/// second card, and closing it and doing nothing would let the escalation raise
/// a card the user had already dismissed, minutes later, over whatever they had
/// moved on to.
static ANSWER_CARD_GENERATION: AtomicU64 = AtomicU64::new(0);

/// Set the card's hold on the window, bumping [`ANSWER_CARD_GENERATION`] when
/// that actually changes. The single writer for both, so the two can never drift.
fn set_answer_card_open(open: bool) {
    if ANSWER_CARD_OPEN.swap(open, Ordering::SeqCst) != open {
        ANSWER_CARD_GENERATION.fetch_add(1, Ordering::SeqCst);
    }
}

/// A token identifying the card that is open (or the absence of one).
///
/// Capture it before a long operation started from a card and compare
/// afterwards: a different value means the card that asked is gone, and the
/// result belongs to nobody.
pub fn answer_card_generation() -> u64 {
    ANSWER_CARD_GENERATION.load(Ordering::SeqCst)
}

/// Incremented every time any overlay state is raised.
///
/// Lets a deadline armed for one state tell "still mine" from "something else
/// owns the overlay now" — see [`hide_overlay_if_unchanged`].
static OVERLAY_GENERATION: AtomicU64 = AtomicU64::new(0);

/// A token identifying the overlay state that is currently up.
pub fn overlay_generation() -> u64 {
    OVERLAY_GENERATION.load(Ordering::SeqCst)
}

/// Hide the overlay, but only if nothing has raised a new state since
/// `generation` was taken.
///
/// The escape hatch for a state whose owner never arrives. `Show` runs hand the
/// overlay to the frontend, which is expected to turn the processing spinner
/// into an answer card; if that never happens — the IPC call failed, the webview
/// was reloading, the card was refused — the spinner would otherwise stay on
/// screen until the next dictation. The generation check is what keeps that
/// safety net from tearing down a *different* state that legitimately started in
/// the meantime.
pub fn hide_overlay_if_unchanged(app_handle: &AppHandle, generation: u64) {
    if overlay_generation() != generation {
        return;
    }
    log::debug!("Overlay watchdog: no answer card was confirmed, hiding the overlay");
    hide_recording_overlay(app_handle);
}

// Cached "overlay is enabled" flag, kept in sync with overlay_style. Avoids
// reading the Tauri store on every audio callback (~24 Hz during recording).
// Defaults to false so the audio path doesn't emit until lib.rs::setup
// populates the cache from initial settings.
static OVERLAY_ENABLED: AtomicBool = AtomicBool::new(false);

/// Tracks whether gtk-layer-shell was successfully initialized (Linux only).
/// Used to skip layer-shell calls when the window is a regular fallback.
#[cfg(target_os = "linux")]
static LAYER_SHELL_ACTIVE: AtomicBool = AtomicBool::new(false);

/// Update the cached overlay-enabled flag. Called from `lib.rs` at
/// startup after settings load, and from `change_overlay_style_setting`
/// whenever the user changes whether the overlay is shown.
pub fn update_overlay_enabled_cache(enabled: bool) {
    OVERLAY_ENABLED.store(enabled, Ordering::Relaxed);
}

pub fn emit_levels(app_handle: &AppHandle, levels: &[f32]) {
    // Skip emission when the overlay is disabled. The recording_overlay
    // window is created at boot regardless of overlay_style, so without this
    // guard a hidden overlay's WebKit subprocess still
    // processes every event. Each event drives some kind of WebKit
    // C++ allocation that accumulates without bound (mechanism not
    // directly characterized; see issue #1279 for the investigation).
    // For users with `overlay_style: none` (the Linux default) this skip
    // eliminates the upstream driver of that accumulation.
    if !OVERLAY_ENABLED.load(Ordering::Relaxed) {
        return;
    }

    // Throttle to ~30 FPS. Even with the overlay enabled, the raw audio
    // callback fires far faster than the UI needs; capping emission rate
    // cuts the per-frame `eval_script`/IPC volume that drives the wry
    // memory growth in issue #1279 (upstream tauri-apps/wry#1489).
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    let last = LAST_MIC_LEVEL_EMIT.load(Ordering::Relaxed);
    if now.saturating_sub(last) < EMIT_THROTTLE_MS {
        return;
    }
    LAST_MIC_LEVEL_EMIT.store(now, Ordering::Relaxed);

    // Target only the overlay window. In Tauri 2 both `AppHandle::emit`
    // and `WebviewWindow::emit` broadcast to all webviews; Tauri's
    // listener filter then skips webviews with no registered listener
    // for the event, so the settings webview never received `mic-level`.
    // But the previous dual-call pattern still produced two `eval_script`
    // calls to the overlay per audio callback (one from each .emit()).
    // `emit_to` with the overlay's window label produces a single
    // eval_script call per callback, cutting the per-callback WebKit
    // dispatch work in half.
    let _ = app_handle.emit_to("recording_overlay", crate::events::MIC_LEVEL, levels);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The overlay stylesheet, read at compile time so a CSS edit rebuilds and
    /// re-runs the geometry assertions below.
    const OVERLAY_CSS: &str = include_str!("../../src/overlay/RecordingOverlay.css");

    /// Reads a `--ov-*` pixel custom property out of the stylesheet.
    ///
    /// Deliberately strict: an unparseable or renamed variable fails loudly
    /// rather than defaulting to something that would make the assertion pass.
    fn css_px(name: &str) -> f64 {
        let needle = format!("{name}:");
        let start = OVERLAY_CSS
            .find(&needle)
            .unwrap_or_else(|| panic!("{name} is not defined in RecordingOverlay.css"))
            + needle.len();
        let value = OVERLAY_CSS[start..]
            .split(';')
            .next()
            .unwrap_or_default()
            .trim();
        let digits = value
            .strip_suffix("px")
            .unwrap_or_else(|| panic!("{name} is not a px value: {value}"));
        digits
            .trim()
            .parse::<f64>()
            .unwrap_or_else(|_| panic!("{name} is not a number: {value}"))
    }

    /// The native window must be at least as large as the card CSS draws into
    /// it, for every state. Both files carry a comment naming the CSS as the
    /// single source of truth; this is what makes that more than a comment.
    ///
    /// A window that is too small does not clip visibly — the card is simply cut
    /// off at the window edge, which on the answer card would silently swallow
    /// the action buttons.
    #[test]
    fn overlay_window_fits_every_card() {
        // Compact pill: widest at the working width, one control row tall.
        let (w, h) = overlay_dimensions("recording");
        assert!(
            w >= css_px("--ov-work-w"),
            "compact overlay {w} is narrower than --ov-work-w"
        );
        assert!(
            h >= css_px("--ov-base-h"),
            "compact overlay {h} is shorter than --ov-base-h"
        );

        // Live panel: the opened width, and a control row above the text cap.
        let (w, h) = overlay_dimensions("streaming");
        assert!(
            w >= css_px("--ov-open-w"),
            "streaming overlay {w} is narrower than --ov-open-w"
        );
        assert!(
            h >= css_px("--ov-base-h") + css_px("--ov-cap-max-h"),
            "streaming overlay {h} is shorter than the control row plus the text cap"
        );

        // Answer card: its own width and its own hard max height.
        let (w, h) = overlay_dimensions(ANSWER_STATE);
        assert!(
            w >= css_px("--ov-answer-w"),
            "answer overlay {w} is narrower than --ov-answer-w"
        );
        assert!(
            h >= css_px("--ov-answer-max-h"),
            "answer overlay {h} is shorter than --ov-answer-max-h"
        );
    }

    /// The card must not accidentally inherit the compact pill's geometry: it is
    /// selected by an exact string that also has to match `RecordingOverlay.tsx`.
    #[test]
    fn answer_state_has_its_own_dimensions() {
        assert_eq!(ANSWER_STATE, "answer");
        assert_ne!(
            overlay_dimensions(ANSWER_STATE),
            overlay_dimensions("recording")
        );
        assert_ne!(
            overlay_dimensions(ANSWER_STATE),
            overlay_dimensions("streaming")
        );
        // Anything unrecognised still falls back to the compact pill.
        assert_eq!(
            overlay_dimensions("meeting_paused"),
            overlay_dimensions("recording")
        );
    }

    /// The whole point of the flag: an ordinary hide must not close a card, and
    /// the next overlay state must release it so the window is never stuck.
    #[cfg(target_os = "windows")]
    #[test]
    fn windows_size_class_round_trips_every_state() {
        for state in ["recording", "streaming", ANSWER_STATE] {
            WINDOWS_OVERLAY_SIZE_CLASS.store(windows_size_class(state), Ordering::Relaxed);
            assert_eq!(
                overlay_dimensions(windows_cached_state()),
                overlay_dimensions(state),
                "reposition would resize the overlay away from the {state} state"
            );
        }
        WINDOWS_OVERLAY_SIZE_CLASS.store(SIZE_CLASS_COMPACT, Ordering::Relaxed);
    }

    #[test]
    fn monitor_hit_test_uses_half_open_physical_bounds() {
        let position = PhysicalPosition::new(-2560, -200);
        let size = PhysicalSize::new(2560, 1440);

        assert!(is_mouse_within_monitor((-2560, -200), &position, &size));
        assert!(is_mouse_within_monitor((-1, 1239), &position, &size));
        assert!(!is_mouse_within_monitor((0, 0), &position, &size));
        assert!(!is_mouse_within_monitor((-1, 1240), &position, &size));
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn windows_cursor_hit_test_does_not_scale_physical_monitor_bounds() {
        let position = PhysicalPosition::new(1920, 0);
        let size = PhysicalSize::new(3840, 2160);
        let cursor = (5000, 1000);

        assert!(is_mouse_within_monitor(cursor, &position, &size));

        // This is the old mixed-coordinate comparison. It excludes a cursor
        // that is visibly inside a secondary display running at 150%.
        let scale = 1.5;
        let logical_position = PhysicalPosition::new(
            (position.x as f64 / scale) as i32,
            (position.y as f64 / scale) as i32,
        );
        let logical_size = PhysicalSize::new(
            (size.width as f64 / scale) as u32,
            (size.height as f64 / scale) as u32,
        );
        assert!(!is_mouse_within_monitor(
            cursor,
            &logical_position,
            &logical_size
        ));
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn windows_overlay_bounds_use_destination_monitor_scale() {
        let monitor_position = PhysicalPosition::new(1920, 0);
        let monitor_size = PhysicalSize::new(3840, 2160);

        assert_eq!(
            windows_overlay_bounds(
                monitor_position,
                monitor_size,
                1.5,
                OVERLAY_WIDTH,
                OVERLAY_HEIGHT,
                OverlayPosition::Bottom,
            ),
            (3648, 2031, 384, 69)
        );
        assert_eq!(
            windows_overlay_bounds(
                monitor_position,
                monitor_size,
                1.5,
                OVERLAY_WIDTH,
                OVERLAY_HEIGHT,
                OverlayPosition::Top,
            ),
            (3648, 6, 384, 69)
        );
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn windows_overlay_bounds_support_negative_monitor_origins() {
        assert_eq!(
            windows_overlay_bounds(
                PhysicalPosition::new(-2560, -200),
                PhysicalSize::new(2560, 1440),
                1.25,
                OVERLAY_STREAM_WIDTH,
                OVERLAY_STREAM_HEIGHT,
                OverlayPosition::Bottom,
            ),
            (-1530, 1040, 500, 150)
        );
    }
}
