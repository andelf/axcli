//! Input simulation: mouse, keyboard, and app activation via CGEvent.

use std::ffi::c_void;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicI64, Ordering};

use objc2_app_kit::{NSEvent, NSEventModifierFlags, NSEventType, NSRunningApplication};
use objc2_core_foundation::CGPoint;
use objc2_core_graphics::{
    CGAssociateMouseAndMouseCursorPosition, CGEvent, CGEventField, CGEventFlags, CGEventSource,
    CGEventSourceStateID, CGEventTapLocation, CGEventType, CGMouseButton, CGScrollEventUnit,
    CGWarpMouseCursorPosition,
};
use objc2_foundation::NSPoint;

// ---------------------------------------------------------------------------
// Private CoreGraphics: CGEventSetWindowLocation
// ---------------------------------------------------------------------------
//
// The event carries both a screen-space location (via CGEventCreateMouseEvent)
// and a window-local location that AppKit's `-[NSWindow sendEvent:]` uses to
// route to the right view.  The window-local setter is not in the public SDK.
// It's resolved at runtime via dlsym(RTLD_DEFAULT, "CGEventSetWindowLocation").
// Stable symbol since at least 10.10; used by the original SWaveAXRaceDemoApp
// reverse-engineering work (https://github.com/Lakr233/bgclick-rev-skill).

type CGEventSetWindowLocationFn = unsafe extern "C" fn(event: *const c_void, point: CGPoint);

const RTLD_DEFAULT: *mut c_void = -2isize as *mut c_void;

unsafe extern "C" {
    fn dlsym(handle: *mut c_void, symbol: *const i8) -> *mut c_void;
}

fn cg_event_set_window_location() -> Option<CGEventSetWindowLocationFn> {
    static CACHED: OnceLock<Option<CGEventSetWindowLocationFn>> = OnceLock::new();
    *CACHED.get_or_init(|| unsafe {
        let name = c"CGEventSetWindowLocation";
        let ptr = dlsym(RTLD_DEFAULT, name.as_ptr());
        if ptr.is_null() {
            None
        } else {
            Some(std::mem::transmute::<*mut c_void, CGEventSetWindowLocationFn>(ptr))
        }
    })
}

/// Bring an application to the foreground by PID.
pub fn activate_app(pid: i32) {
    let ns_app = NSRunningApplication::runningApplicationWithProcessIdentifier(pid);
    if let Some(ns_app) = ns_app {
        #[allow(deprecated)]
        ns_app.activateWithOptions(
            objc2_app_kit::NSApplicationActivationOptions::ActivateIgnoringOtherApps,
        );
    }
}

/// Get current mouse cursor position.
pub fn get_mouse_position() -> (f64, f64) {
    let source = CGEventSource::new(CGEventSourceStateID::HIDSystemState);
    let event = CGEvent::new(source.as_deref());
    match event {
        Some(ref ev) => {
            let loc = CGEvent::location(Some(ev));
            (loc.x, loc.y)
        }
        None => (0.0, 0.0),
    }
}

/// Move the mouse cursor to (x, y) screen coordinates.
///
/// Uses CGWarpMouseCursorPosition for reliable cross-screen movement,
/// followed by a CGEvent MouseMoved to notify the window server.
/// CGWarp alone doesn't generate mouse events; CGEvent alone can fail
/// when moving across display boundaries.
pub fn mouse_move(x: f64, y: f64) {
    let point = CGPoint { x, y };

    // Step 1: Dissociate mouse and cursor so the warp isn't fought by the OS
    CGAssociateMouseAndMouseCursorPosition(false);

    // Step 2: Warp cursor to the exact position (works reliably across screens)
    CGWarpMouseCursorPosition(point);

    // Step 3: Re-associate mouse and cursor
    CGAssociateMouseAndMouseCursorPosition(true);

    // Step 4: Post a MouseMoved event so the app under the cursor gets the
    // hover/mouseEnter notification.  Small delay lets the warp settle.
    std::thread::sleep(std::time::Duration::from_millis(10));
    let source = CGEventSource::new(CGEventSourceStateID::HIDSystemState);
    let event = CGEvent::new_mouse_event(
        source.as_deref(),
        CGEventType::MouseMoved,
        point,
        CGMouseButton::Left,
    );
    if let Some(ref ev) = event {
        CGEvent::post(CGEventTapLocation::HIDEventTap, Some(ev));
    }
}

/// Single left-click at (x, y) screen coordinates.
pub fn mouse_click(x: f64, y: f64) {
    let source = CGEventSource::new(CGEventSourceStateID::HIDSystemState);
    let point = CGPoint { x, y };
    let down = CGEvent::new_mouse_event(
        source.as_deref(),
        CGEventType::LeftMouseDown,
        point,
        CGMouseButton::Left,
    );
    let up = CGEvent::new_mouse_event(
        source.as_deref(),
        CGEventType::LeftMouseUp,
        point,
        CGMouseButton::Left,
    );
    if let Some(ref ev) = down {
        CGEvent::post(CGEventTapLocation::HIDEventTap, Some(ev));
    }
    std::thread::sleep(std::time::Duration::from_millis(50));
    if let Some(ref ev) = up {
        CGEvent::post(CGEventTapLocation::HIDEventTap, Some(ev));
    }
}

/// Check whether an app (by pid) is currently active.
fn app_is_active(pid: i32) -> bool {
    NSRunningApplication::runningApplicationWithProcessIdentifier(pid)
        .map_or(false, |a| a.isActive())
}

/// Monotonic event number for synthesized NSEvents.
fn next_event_number() -> i64 {
    static COUNTER: AtomicI64 = AtomicI64::new(1);
    COUNTER.fetch_add(1, Ordering::Relaxed)
}

/// Build a synthetic mouse event via `+[NSEvent mouseEventWithType:...]` and
/// extract its `CGEventRef`.  NSEvent auto-fills fields 0/1/2/41/43/44/50/51/55/59/102/108
/// that plain `CGEventCreateMouseEvent` leaves blank — notably field 55
/// (window number), which AppKit relies on for routing.
fn make_mouse_event_via_nsevent(
    ty: NSEventType,
    screen: CGPoint,
    modifier_flags: NSEventModifierFlags,
    window_number: isize,
    click_count: isize,
) -> Option<objc2::rc::Retained<CGEvent>> {
    let ns_point = NSPoint::new(screen.x, screen.y);
    let timestamp = objc2_foundation::NSProcessInfo::processInfo().systemUptime();
    let ns_ev = NSEvent::mouseEventWithType_location_modifierFlags_timestamp_windowNumber_context_eventNumber_clickCount_pressure(
        ty,
        ns_point,
        modifier_flags,
        timestamp,
        window_number,
        None,
        next_event_number() as isize,
        click_count,
        1.0,
    )?;
    ns_ev.CGEvent()
}

/// Post a left-click to the target process via `CGEventPostToPid`, using the
/// full SWaveAX reverse-engineered recipe:
///
/// 1. Event is built via `+[NSEvent mouseEventWithType:...]` so the 12
///    auto-fill fields (including field 55 = windowNumber) are populated.
/// 2. Window ID fields 91/92, subtype 7, button number 3 are explicitly set.
/// 3. Window-local coordinates written via private `CGEventSetWindowLocation`
///    (dlsym'd from `CoreGraphics`).
/// 4. When the target app is inactive, modifier flags carry
///    `kCGEventFlagMaskCommand` (`0x00100000`) as a WindowServer-bypass signal.
/// 5. Delivered via `CGEventPostToPid`, no focus steal.
///
/// Reference: <https://github.com/Lakr233/bgclick-rev-skill>.  No cursor
/// movement is performed.  Requires Accessibility permission.
pub fn mouse_click_bg(pid: i32, window_id: u32, screen: CGPoint, local: CGPoint) {
    let wid = window_id as i64;
    let set_win_loc = cg_event_set_window_location();
    let inactive = !app_is_active(pid);
    let flags = if inactive {
        NSEventModifierFlags::Command
    } else {
        NSEventModifierFlags::empty()
    };

    let tag = |ev: &CGEvent, button_idx: i64| {
        CGEvent::set_integer_value_field(Some(ev), CGEventField::MouseEventWindowUnderMousePointer, wid);
        CGEvent::set_integer_value_field(
            Some(ev),
            CGEventField::MouseEventWindowUnderMousePointerThatCanHandleThisEvent,
            wid,
        );
        CGEvent::set_integer_value_field(Some(ev), CGEventField::MouseEventSubtype, 3);
        CGEvent::set_integer_value_field(Some(ev), CGEventField::MouseEventButtonNumber, button_idx);
        if let Some(fptr) = set_win_loc {
            unsafe { fptr(ev as *const CGEvent as *const c_void, local); }
        }
    };

    if let Some(down) = make_mouse_event_via_nsevent(NSEventType::LeftMouseDown, screen, flags, wid as isize, 1) {
        CGEvent::set_location(Some(&down), screen);
        tag(&down, 0);
        CGEvent::post_to_pid(pid, Some(&down));
    }
    std::thread::sleep(std::time::Duration::from_millis(50));
    if let Some(up) = make_mouse_event_via_nsevent(NSEventType::LeftMouseUp, screen, flags, wid as isize, 1) {
        CGEvent::set_location(Some(&up), screen);
        tag(&up, 0);
        CGEvent::post_to_pid(pid, Some(&up));
    }
}

/// Post a left-double-click to the target process via `CGEventPostToPid`,
/// using the same SWaveAX recipe as `mouse_click_bg`.  Two down/up pairs
/// are sent with clickCount 1 then 2.
pub fn mouse_dblclick_bg(pid: i32, window_id: u32, screen: CGPoint, local: CGPoint) {
    let wid = window_id as i64;
    let set_win_loc = cg_event_set_window_location();
    let inactive = !app_is_active(pid);
    let flags = if inactive {
        NSEventModifierFlags::Command
    } else {
        NSEventModifierFlags::empty()
    };

    let tag = |ev: &CGEvent| {
        CGEvent::set_integer_value_field(Some(ev), CGEventField::MouseEventWindowUnderMousePointer, wid);
        CGEvent::set_integer_value_field(
            Some(ev),
            CGEventField::MouseEventWindowUnderMousePointerThatCanHandleThisEvent,
            wid,
        );
        CGEvent::set_integer_value_field(Some(ev), CGEventField::MouseEventSubtype, 3);
        CGEvent::set_integer_value_field(Some(ev), CGEventField::MouseEventButtonNumber, 0);
        if let Some(fptr) = set_win_loc {
            unsafe { fptr(ev as *const CGEvent as *const c_void, local); }
        }
        CGEvent::set_location(Some(ev), screen);
    };

    // First click (clickCount=1)
    if let Some(down) = make_mouse_event_via_nsevent(NSEventType::LeftMouseDown, screen, flags, wid as isize, 1) {
        tag(&down);
        CGEvent::post_to_pid(pid, Some(&down));
    }
    std::thread::sleep(std::time::Duration::from_millis(30));
    if let Some(up) = make_mouse_event_via_nsevent(NSEventType::LeftMouseUp, screen, flags, wid as isize, 1) {
        tag(&up);
        CGEvent::post_to_pid(pid, Some(&up));
    }
    std::thread::sleep(std::time::Duration::from_millis(30));

    // Second click (clickCount=2)
    if let Some(down) = make_mouse_event_via_nsevent(NSEventType::LeftMouseDown, screen, flags, wid as isize, 2) {
        tag(&down);
        CGEvent::post_to_pid(pid, Some(&down));
    }
    std::thread::sleep(std::time::Duration::from_millis(30));
    if let Some(up) = make_mouse_event_via_nsevent(NSEventType::LeftMouseUp, screen, flags, wid as isize, 2) {
        tag(&up);
        CGEvent::post_to_pid(pid, Some(&up));
    }
}

/// Double left-click at (x, y) screen coordinates.
pub fn mouse_dblclick(x: f64, y: f64) {
    let source = CGEventSource::new(CGEventSourceStateID::HIDSystemState);
    let point = CGPoint { x, y };

    // First click
    let down1 = CGEvent::new_mouse_event(
        source.as_deref(),
        CGEventType::LeftMouseDown,
        point,
        CGMouseButton::Left,
    );
    let up1 = CGEvent::new_mouse_event(
        source.as_deref(),
        CGEventType::LeftMouseUp,
        point,
        CGMouseButton::Left,
    );
    if let Some(ref ev) = down1 {
        CGEvent::post(CGEventTapLocation::HIDEventTap, Some(ev));
    }
    std::thread::sleep(std::time::Duration::from_millis(30));
    if let Some(ref ev) = up1 {
        CGEvent::post(CGEventTapLocation::HIDEventTap, Some(ev));
    }
    std::thread::sleep(std::time::Duration::from_millis(30));

    // Second click with click count = 2
    let down2 = CGEvent::new_mouse_event(
        source.as_deref(),
        CGEventType::LeftMouseDown,
        point,
        CGMouseButton::Left,
    );
    let up2 = CGEvent::new_mouse_event(
        source.as_deref(),
        CGEventType::LeftMouseUp,
        point,
        CGMouseButton::Left,
    );
    if let Some(ref ev) = down2 {
        CGEvent::set_integer_value_field(Some(ev), CGEventField::MouseEventClickState, 2);
        CGEvent::post(CGEventTapLocation::HIDEventTap, Some(ev));
    }
    std::thread::sleep(std::time::Duration::from_millis(30));
    if let Some(ref ev) = up2 {
        CGEvent::set_integer_value_field(Some(ev), CGEventField::MouseEventClickState, 2);
        CGEvent::post(CGEventTapLocation::HIDEventTap, Some(ev));
    }
}

/// Type text using CGEvent unicode input (chunks of 20 UTF-16 code units).
pub fn type_text(text: &str) {
    let source = CGEventSource::new(CGEventSourceStateID::HIDSystemState);
    let utf16: Vec<u16> = text.encode_utf16().collect();
    for chunk in utf16.chunks(20) {
        let down = CGEvent::new_keyboard_event(source.as_deref(), 0, true);
        if let Some(ref ev) = down {
            unsafe {
                CGEvent::keyboard_set_unicode_string(Some(ev), chunk.len() as _, chunk.as_ptr());
            }
            CGEvent::post(CGEventTapLocation::HIDEventTap, Some(ev));
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
        let up = CGEvent::new_keyboard_event(source.as_deref(), 0, false);
        if let Some(ref ev) = up {
            unsafe {
                CGEvent::keyboard_set_unicode_string(Some(ev), chunk.len() as _, chunk.as_ptr());
            }
            CGEvent::post(CGEventTapLocation::HIDEventTap, Some(ev));
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

/// Type text directly to a specific pid via `CGEventPostToPid`.
///
/// Background-safe counterpart to `type_text` — delivers Unicode keystrokes
/// to the target process's first responder without activating the app or
/// stealing focus.  Mirrors `press_key_combo_bg` for parameterised keys.
///
/// The chunk size (20 UTF-16 code units) matches `type_text`; per-chunk
/// sleeps are the same so timing-sensitive apps see identical pacing on
/// both paths.  Confirmed working on TextEdit's document body with the
/// app fully occluded behind another window.  Unicode (Chinese, emoji)
/// is supported because `CGEventKeyboardSetUnicodeString` feeds UTF-16
/// directly, bypassing the keycode table.
pub fn type_text_bg(pid: i32, text: &str) {
    let source = CGEventSource::new(CGEventSourceStateID::HIDSystemState);
    let utf16: Vec<u16> = text.encode_utf16().collect();
    for chunk in utf16.chunks(20) {
        let down = CGEvent::new_keyboard_event(source.as_deref(), 0, true);
        if let Some(ref ev) = down {
            unsafe {
                CGEvent::keyboard_set_unicode_string(Some(ev), chunk.len() as _, chunk.as_ptr());
            }
            CGEvent::post_to_pid(pid, Some(ev));
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
        let up = CGEvent::new_keyboard_event(source.as_deref(), 0, false);
        if let Some(ref ev) = up {
            unsafe {
                CGEvent::keyboard_set_unicode_string(Some(ev), chunk.len() as _, chunk.as_ptr());
            }
            CGEvent::post_to_pid(pid, Some(ev));
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

/// Parse a key combo string like "Control+a", "Command+Shift+v", "Enter"
/// into (keycode, modifier_flags).
pub fn parse_key_combo(combo: &str) -> (u16, u64) {
    let parts: Vec<&str> = combo.split('+').map(|s| s.trim()).collect();
    let mut flags: u64 = 0;
    let mut key_name = "";

    for part in &parts {
        match part.to_lowercase().as_str() {
            "control" | "ctrl" => flags |= 0x40000,
            "shift" => flags |= 0x20000,
            "option" | "alt" => flags |= 0x80000,
            "command" | "cmd" | "super" => flags |= 0x100000,
            _ => key_name = part,
        }
    }

    let keycode = match key_name.to_lowercase().as_str() {
        "return" | "enter" => 36,
        "tab" => 48,
        "space" => 49,
        "delete" | "backspace" => 51,
        "escape" | "esc" => 53,
        "left" => 123,
        "right" => 124,
        "down" => 125,
        "up" => 126,
        "home" => 115,
        "end" => 119,
        "pageup" => 116,
        "pagedown" => 121,
        "f1" => 122, "f2" => 120, "f3" => 99, "f4" => 118,
        "f5" => 96, "f6" => 97, "f7" => 98, "f8" => 100,
        "f9" => 101, "f10" => 109, "f11" => 103, "f12" => 111,
        s if s.chars().count() == 1 => {
            let ch = s.chars().next().expect("already verified single char");
            match ch {
                'a' => 0, 's' => 1, 'd' => 2, 'f' => 3, 'h' => 4,
                'g' => 5, 'z' => 6, 'x' => 7, 'c' => 8, 'v' => 9,
                'b' => 11, 'q' => 12, 'w' => 13, 'e' => 14, 'r' => 15,
                'y' => 16, 't' => 17, '1' => 18, '2' => 19, '3' => 20,
                '4' => 21, '6' => 22, '5' => 23, '=' => 24, '9' => 25,
                '7' => 26, '-' => 27, '8' => 28, '0' => 29, ']' => 30,
                'o' => 31, 'u' => 32, '[' => 33, 'i' => 34, 'p' => 35,
                'l' => 37, 'j' => 38, '\'' => 39, 'k' => 40, ';' => 41,
                '\\' => 42, ',' => 43, '/' => 44, 'n' => 45, 'm' => 46,
                '.' => 47,
                _ => {
                    eprintln!("warning: unknown key '{ch}', using keycode 0");
                    0
                }
            }
        }
        _ => {
            eprintln!("warning: unknown key '{key_name}', using keycode 0");
            0
        }
    };

    (keycode, flags)
}

/// Press a key combo (keycode + modifier flags).
pub fn press_key_combo(keycode: u16, flags: u64) {
    let source = CGEventSource::new(CGEventSourceStateID::HIDSystemState);

    let down = CGEvent::new_keyboard_event(source.as_deref(), keycode, true);
    if let Some(ref ev) = down {
        if flags != 0 {
            CGEvent::set_flags(Some(ev), CGEventFlags(flags));
        }
        CGEvent::post(CGEventTapLocation::HIDEventTap, Some(ev));
    }
    std::thread::sleep(std::time::Duration::from_millis(20));
    let up = CGEvent::new_keyboard_event(source.as_deref(), keycode, false);
    if let Some(ref ev) = up {
        CGEvent::set_flags(Some(ev), CGEventFlags(0));
        CGEvent::post(CGEventTapLocation::HIDEventTap, Some(ev));
    }
}

/// Press a key combo, delivered directly to a specific pid via
/// `CGEventPostToPid`.
///
/// Empirically verified on macOS 14.x against Calculator and TextEdit with
/// Finder kept frontmost — keys landed on the target's first responder
/// without activation or focus steal.  See `docs/research/background-click.md`
/// appendix A.3 for data and A.4 for the architectural reason (keyboard
/// events route via AppKit's first-responder chain and don't depend on
/// WindowServer hit-testing, unlike mouse events).
pub fn press_key_combo_bg(pid: i32, keycode: u16, flags: u64) {
    let source = CGEventSource::new(CGEventSourceStateID::HIDSystemState);

    let down = CGEvent::new_keyboard_event(source.as_deref(), keycode, true);
    if let Some(ref ev) = down {
        if flags != 0 {
            CGEvent::set_flags(Some(ev), CGEventFlags(flags));
        }
        CGEvent::post_to_pid(pid, Some(ev));
    }
    std::thread::sleep(std::time::Duration::from_millis(20));
    let up = CGEvent::new_keyboard_event(source.as_deref(), keycode, false);
    if let Some(ref ev) = up {
        CGEvent::set_flags(Some(ev), CGEventFlags(0));
        CGEvent::post_to_pid(pid, Some(ev));
    }
}

/// Scroll the mouse wheel at screen position (x, y) by (dx, dy) pixels.
/// Positive dy = scroll up, negative dy = scroll down.
pub fn scroll_wheel(x: f64, y: f64, dx: i32, dy: i32) {
    let source = CGEventSource::new(CGEventSourceStateID::HIDSystemState);
    let event = CGEvent::new_scroll_wheel_event2(
        source.as_deref(),
        CGScrollEventUnit::Pixel,
        2,  // wheel_count (2 = vertical + horizontal)
        dy, // wheel1 (vertical)
        dx, // wheel2 (horizontal)
        0,  // wheel3
    );
    if let Some(ref ev) = event {
        CGEvent::set_location(Some(ev), CGPoint { x, y });
        CGEvent::post(CGEventTapLocation::HIDEventTap, Some(ev));
    }
}

/// Scroll via `CGEventPostToPid` — background-safe, no activation, no cursor
/// movement.
///
/// Strategy (Probe A — confirmed working on Lark/Electron 2026-04-25):
/// 1. Send a MouseMoved event (built via NSEvent factory with full window
///    tags) to the target pid — this updates the process's internal "window
///    under cursor" tracking state.
/// 2. Send a raw CGEvent scroll wheel event with window fields + location.
///
/// Scroll events in macOS rely on the process's cached "window under cursor"
/// state rather than per-event routing fields.  The MouseMoved pre-send
/// tricks AppKit into thinking the cursor is over our target window.
pub fn scroll_wheel_bg(pid: i32, window_id: u32, screen: CGPoint, local: CGPoint, dx: i32, dy: i32) {
    let wid = window_id as i64;
    let set_win_loc = cg_event_set_window_location();
    let inactive = !app_is_active(pid);
    let flags = if inactive {
        NSEventModifierFlags::Command
    } else {
        NSEventModifierFlags::empty()
    };

    // Step 1: MouseMoved pre-send to establish window tracking state
    let moved = make_mouse_event_via_nsevent(
        NSEventType::MouseMoved, screen, flags, wid as isize, 0,
    );
    if let Some(ref ev) = moved {
        CGEvent::set_location(Some(ev), screen);
        CGEvent::set_integer_value_field(Some(ev), CGEventField::MouseEventWindowUnderMousePointer, wid);
        CGEvent::set_integer_value_field(
            Some(ev),
            CGEventField::MouseEventWindowUnderMousePointerThatCanHandleThisEvent,
            wid,
        );
        CGEvent::set_integer_value_field(Some(ev), CGEventField::MouseEventSubtype, 3);
        CGEvent::set_integer_value_field(Some(ev), CGEventField::MouseEventButtonNumber, 0);
        if let Some(fptr) = set_win_loc {
            unsafe { fptr(&**ev as *const CGEvent as *const c_void, local); }
        }
        CGEvent::post_to_pid(pid, Some(ev));
    }

    std::thread::sleep(std::time::Duration::from_millis(100));

    // Step 2: scroll event with window tags
    let source = CGEventSource::new(CGEventSourceStateID::HIDSystemState);
    let scroll = CGEvent::new_scroll_wheel_event2(
        source.as_deref(),
        CGScrollEventUnit::Pixel,
        2,
        dy,
        dx,
        0,
    );
    if let Some(ref ev) = scroll {
        CGEvent::set_location(Some(ev), screen);
        CGEvent::set_integer_value_field(Some(ev), CGEventField::MouseEventWindowUnderMousePointer, wid);
        CGEvent::set_integer_value_field(
            Some(ev),
            CGEventField::MouseEventWindowUnderMousePointerThatCanHandleThisEvent,
            wid,
        );
        CGEvent::set_integer_value_field(Some(ev), CGEventField(51), wid);
        if let Some(fptr) = set_win_loc {
            unsafe { fptr(&**ev as *const CGEvent as *const c_void, local); }
        }
        if inactive {
            CGEvent::set_flags(Some(ev), CGEventFlags(0x00100000));
        }
        CGEvent::post_to_pid(pid, Some(ev));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_single_key_enter() {
        let (keycode, flags) = parse_key_combo("Enter");
        assert_eq!(keycode, 36);
        assert_eq!(flags, 0);
    }

    #[test]
    fn parse_single_key_tab() {
        let (keycode, flags) = parse_key_combo("Tab");
        assert_eq!(keycode, 48);
        assert_eq!(flags, 0);
    }

    #[test]
    fn parse_single_key_escape() {
        let (keycode, flags) = parse_key_combo("Escape");
        assert_eq!(keycode, 53);
        assert_eq!(flags, 0);
    }

    #[test]
    fn parse_control_a() {
        let (keycode, flags) = parse_key_combo("Control+a");
        assert_eq!(keycode, 0); // 'a'
        assert_eq!(flags, 0x40000); // Control
    }

    #[test]
    fn parse_command_shift_v() {
        let (keycode, flags) = parse_key_combo("Command+Shift+v");
        assert_eq!(keycode, 9); // 'v'
        assert_eq!(flags, 0x100000 | 0x20000); // Command + Shift
    }

    #[test]
    fn parse_command_a() {
        let (keycode, flags) = parse_key_combo("Command+a");
        assert_eq!(keycode, 0); // 'a'
        assert_eq!(flags, 0x100000); // Command
    }

    #[test]
    fn parse_alt_option() {
        let (_keycode, flags) = parse_key_combo("Alt+a");
        assert_eq!(flags, 0x80000); // Option

        let (_, flags2) = parse_key_combo("Option+a");
        assert_eq!(flags2, 0x80000);
    }

    #[test]
    fn parse_function_keys() {
        assert_eq!(parse_key_combo("F1").0, 122);
        assert_eq!(parse_key_combo("F5").0, 96);
        assert_eq!(parse_key_combo("F12").0, 111);
    }

    #[test]
    fn parse_arrow_keys() {
        assert_eq!(parse_key_combo("Left").0, 123);
        assert_eq!(parse_key_combo("Right").0, 124);
        assert_eq!(parse_key_combo("Down").0, 125);
        assert_eq!(parse_key_combo("Up").0, 126);
    }

    #[test]
    fn parse_delete_backspace() {
        assert_eq!(parse_key_combo("Delete").0, 51);
        assert_eq!(parse_key_combo("Backspace").0, 51);
    }

    #[test]
    fn parse_space() {
        assert_eq!(parse_key_combo("Space").0, 49);
    }

    #[test]
    fn parse_single_letter_keys() {
        assert_eq!(parse_key_combo("a").0, 0);
        assert_eq!(parse_key_combo("z").0, 6);
        assert_eq!(parse_key_combo("q").0, 12);
    }

    #[test]
    fn parse_number_keys() {
        assert_eq!(parse_key_combo("1").0, 18);
        assert_eq!(parse_key_combo("0").0, 29);
    }
}
