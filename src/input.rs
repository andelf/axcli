//! Input simulation: mouse, keyboard, and app activation via CGEvent.

use objc2_app_kit::NSRunningApplication;
use objc2_core_foundation::CGPoint;
use objc2_core_graphics::{
    CGAssociateMouseAndMouseCursorPosition, CGEvent, CGEventField, CGEventFlags, CGEventSource,
    CGEventSourceStateID, CGEventTapLocation, CGEventType, CGMouseButton, CGScrollEventUnit,
    CGWarpMouseCursorPosition,
};

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
        s if s.len() == 1 => {
            let ch = s.chars().next().unwrap();
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
        if flags != 0 {
            CGEvent::set_flags(Some(ev), CGEventFlags(flags));
        }
        CGEvent::post(CGEventTapLocation::HIDEventTap, Some(ev));
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
