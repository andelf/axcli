//! Probe: try various CGEvent flag/field combinations for background clicking
//! on Electron/Lark.
//!
//! Usage:
//!   cargo run --example probe_bg_click
//!
//! Keep Lark in the BACKGROUND (another window on top) and watch which
//! strategy actually delivers the click to the "TD 闲聊" chat entry.

use std::ffi::c_void;
use std::sync::atomic::{AtomicI64, Ordering};
use std::time::Duration;

use objc2_app_kit::{NSEvent, NSEventModifierFlags, NSEventType};
use objc2_core_foundation::CGPoint;
use objc2_core_graphics::{
    CGEvent, CGEventField, CGEventFlags, CGEventSource, CGEventSourceStateID, CGEventTapLocation,
    CGEventType, CGMouseButton,
};
use objc2_foundation::NSPoint;

use axcli::accessibility::{self, AXNode};

// ---------------------------------------------------------------------------
// Private APIs (same as input.rs)
// ---------------------------------------------------------------------------

type CGEventSetWindowLocationFn = unsafe extern "C" fn(event: *const c_void, point: CGPoint);

const RTLD_DEFAULT: *mut c_void = -2isize as *mut c_void;

unsafe extern "C" {
    fn dlsym(handle: *mut c_void, symbol: *const i8) -> *mut c_void;
}

fn cg_event_set_window_location() -> Option<CGEventSetWindowLocationFn> {
    unsafe {
        let name = c"CGEventSetWindowLocation";
        let ptr = dlsym(RTLD_DEFAULT, name.as_ptr());
        if ptr.is_null() {
            None
        } else {
            Some(std::mem::transmute::<*mut c_void, CGEventSetWindowLocationFn>(ptr))
        }
    }
}

static EVENT_COUNTER: AtomicI64 = AtomicI64::new(1);

fn next_event_number() -> i64 {
    EVENT_COUNTER.fetch_add(1, Ordering::Relaxed)
}

// ---------------------------------------------------------------------------
// Event construction helpers
// ---------------------------------------------------------------------------

fn make_nsevent(
    ty: NSEventType,
    screen: CGPoint,
    flags: NSEventModifierFlags,
    wnum: isize,
) -> Option<objc2::rc::Retained<CGEvent>> {
    let ns_point = NSPoint::new(screen.x, screen.y);
    let ts = objc2_foundation::NSProcessInfo::processInfo().systemUptime();
    let ns_ev = NSEvent::mouseEventWithType_location_modifierFlags_timestamp_windowNumber_context_eventNumber_clickCount_pressure(
        ty, ns_point, flags, ts, wnum, None, next_event_number() as isize, 1, 1.0,
    )?;
    ns_ev.CGEvent()
}

fn make_cgevent(
    ty: CGEventType,
    screen: CGPoint,
) -> Option<objc2_core_foundation::CFRetained<CGEvent>> {
    let source = CGEventSource::new(CGEventSourceStateID::HIDSystemState);
    CGEvent::new_mouse_event(source.as_deref(), ty, screen, CGMouseButton::Left)
}

fn tag_event(ev: &CGEvent, wid: i64, local: CGPoint) {
    let set_win_loc = cg_event_set_window_location();
    CGEvent::set_integer_value_field(
        Some(ev), CGEventField::MouseEventWindowUnderMousePointer, wid,
    );
    CGEvent::set_integer_value_field(
        Some(ev), CGEventField::MouseEventWindowUnderMousePointerThatCanHandleThisEvent, wid,
    );
    CGEvent::set_integer_value_field(Some(ev), CGEventField::MouseEventSubtype, 3);
    CGEvent::set_integer_value_field(Some(ev), CGEventField::MouseEventButtonNumber, 0);
    if let Some(fptr) = set_win_loc {
        unsafe { fptr(ev as *const CGEvent as *const c_void, local); }
    }
}

fn post_click_to_pid(pid: i32, down: &CGEvent, up: &CGEvent) {
    CGEvent::post_to_pid(pid, Some(down));
    std::thread::sleep(Duration::from_millis(50));
    CGEvent::post_to_pid(pid, Some(up));
}

fn post_click_hid(down: &CGEvent, up: &CGEvent) {
    CGEvent::post(CGEventTapLocation::HIDEventTap, Some(down));
    std::thread::sleep(Duration::from_millis(50));
    CGEvent::post(CGEventTapLocation::HIDEventTap, Some(up));
}

// ---------------------------------------------------------------------------
// Probes
// ---------------------------------------------------------------------------

struct ClickTarget {
    pid: i32,
    wid: u32,
    screen: CGPoint,
    local: CGPoint,
}

fn probe_baseline_cgpid(t: &ClickTarget) {
    eprintln!("  [1] baseline: NSEvent + CGEventPostToPid + Command flag (current impl)");
    let flags = NSEventModifierFlags::Command;
    let wid = t.wid as i64;
    if let Some(down) = make_nsevent(NSEventType::LeftMouseDown, t.screen, flags, wid as isize) {
        tag_event(&down, wid, t.local);
        if let Some(up) = make_nsevent(NSEventType::LeftMouseUp, t.screen, flags, wid as isize) {
            tag_event(&up, wid, t.local);
            post_click_to_pid(t.pid, &down, &up);
        }
    }
}

fn probe_nsevent_no_cmd(t: &ClickTarget) {
    eprintln!("  [2] NSEvent + CGEventPostToPid, NO Command flag");
    let flags = NSEventModifierFlags::empty();
    let wid = t.wid as i64;
    if let Some(down) = make_nsevent(NSEventType::LeftMouseDown, t.screen, flags, wid as isize) {
        tag_event(&down, wid, t.local);
        if let Some(up) = make_nsevent(NSEventType::LeftMouseUp, t.screen, flags, wid as isize) {
            tag_event(&up, wid, t.local);
            post_click_to_pid(t.pid, &down, &up);
        }
    }
}

fn probe_cgevent_plain_pid(t: &ClickTarget) {
    eprintln!("  [3] plain CGEventCreateMouseEvent + CGEventPostToPid (no NSEvent, no tags)");
    if let Some(down) = make_cgevent(CGEventType::LeftMouseDown, t.screen) {
        if let Some(up) = make_cgevent(CGEventType::LeftMouseUp, t.screen) {
            post_click_to_pid(t.pid, &down, &up);
        }
    }
}

fn probe_cgevent_tagged_pid(t: &ClickTarget) {
    eprintln!("  [4] CGEventCreateMouseEvent + tags + CGEventPostToPid");
    let wid = t.wid as i64;
    if let Some(down) = make_cgevent(CGEventType::LeftMouseDown, t.screen) {
        tag_event(&down, wid, t.local);
        CGEvent::set_flags(Some(&down), CGEventFlags(0x100000)); // Command
        if let Some(up) = make_cgevent(CGEventType::LeftMouseUp, t.screen) {
            tag_event(&up, wid, t.local);
            CGEvent::set_flags(Some(&up), CGEventFlags(0x100000));
            post_click_to_pid(t.pid, &down, &up);
        }
    }
}

fn probe_nsevent_hid_no_activate(t: &ClickTarget) {
    eprintln!("  [5] NSEvent + CGEventPost(HID) — no activation, no tags");
    let flags = NSEventModifierFlags::empty();
    let wid = t.wid as i64;
    if let Some(down) = make_nsevent(NSEventType::LeftMouseDown, t.screen, flags, wid as isize) {
        if let Some(up) = make_nsevent(NSEventType::LeftMouseUp, t.screen, flags, wid as isize) {
            post_click_hid(&down, &up);
        }
    }
}

fn probe_nsevent_hid_tagged(t: &ClickTarget) {
    eprintln!("  [6] NSEvent + tags + CGEventPost(HID) — no activation");
    let flags = NSEventModifierFlags::empty();
    let wid = t.wid as i64;
    if let Some(down) = make_nsevent(NSEventType::LeftMouseDown, t.screen, flags, wid as isize) {
        tag_event(&down, wid, t.local);
        if let Some(up) = make_nsevent(NSEventType::LeftMouseUp, t.screen, flags, wid as isize) {
            tag_event(&up, wid, t.local);
            post_click_hid(&down, &up);
        }
    }
}

fn probe_noncoalesced_flag(t: &ClickTarget) {
    eprintln!("  [7] NSEvent + NonCoalesced(0x100) flag + CGEventPostToPid");
    let flags = NSEventModifierFlags(0x100);
    let wid = t.wid as i64;
    if let Some(down) = make_nsevent(NSEventType::LeftMouseDown, t.screen, flags, wid as isize) {
        tag_event(&down, wid, t.local);
        if let Some(up) = make_nsevent(NSEventType::LeftMouseUp, t.screen, flags, wid as isize) {
            tag_event(&up, wid, t.local);
            post_click_to_pid(t.pid, &down, &up);
        }
    }
}

fn probe_subtype_zero(t: &ClickTarget) {
    eprintln!("  [8] NSEvent + Command + subtype=0 (default) + CGEventPostToPid");
    let flags = NSEventModifierFlags::Command;
    let wid = t.wid as i64;
    let set_win_loc = cg_event_set_window_location();
    if let Some(down) = make_nsevent(NSEventType::LeftMouseDown, t.screen, flags, wid as isize) {
        CGEvent::set_integer_value_field(
            Some(&down), CGEventField::MouseEventWindowUnderMousePointer, wid,
        );
        CGEvent::set_integer_value_field(
            Some(&down), CGEventField::MouseEventWindowUnderMousePointerThatCanHandleThisEvent, wid,
        );
        CGEvent::set_integer_value_field(Some(&down), CGEventField::MouseEventSubtype, 0);
        CGEvent::set_integer_value_field(Some(&down), CGEventField::MouseEventButtonNumber, 0);
        if let Some(fptr) = set_win_loc {
            unsafe { fptr(&*down as *const CGEvent as *const c_void, t.local); }
        }
        if let Some(up) = make_nsevent(NSEventType::LeftMouseUp, t.screen, flags, wid as isize) {
            CGEvent::set_integer_value_field(
                Some(&up), CGEventField::MouseEventWindowUnderMousePointer, wid,
            );
            CGEvent::set_integer_value_field(
                Some(&up), CGEventField::MouseEventWindowUnderMousePointerThatCanHandleThisEvent, wid,
            );
            CGEvent::set_integer_value_field(Some(&up), CGEventField::MouseEventSubtype, 0);
            CGEvent::set_integer_value_field(Some(&up), CGEventField::MouseEventButtonNumber, 0);
            if let Some(fptr) = set_win_loc {
                unsafe { fptr(&*up as *const CGEvent as *const c_void, t.local); }
            }
            post_click_to_pid(t.pid, &down, &up);
        }
    }
}

fn probe_high_flags(t: &ClickTarget, extra_flags: u64, label: &str) {
    eprintln!("  [H] NSEvent + Command + extra flags 0x{extra_flags:x} ({label}) + CGEventPostToPid");
    let combined = 0x100000 | extra_flags; // Command + extra
    let flags = NSEventModifierFlags(combined as _);
    let wid = t.wid as i64;
    if let Some(down) = make_nsevent(NSEventType::LeftMouseDown, t.screen, flags, wid as isize) {
        tag_event(&down, wid, t.local);
        if let Some(up) = make_nsevent(NSEventType::LeftMouseUp, t.screen, flags, wid as isize) {
            tag_event(&up, wid, t.local);
            post_click_to_pid(t.pid, &down, &up);
        }
    }
}

fn probe_extra_fields(t: &ClickTarget, field_idx: u32, field_val: i64, label: &str) {
    eprintln!("  [F] NSEvent + Command + field[{field_idx}]={field_val} ({label}) + CGEventPostToPid");
    let flags = NSEventModifierFlags::Command;
    let wid = t.wid as i64;
    if let Some(down) = make_nsevent(NSEventType::LeftMouseDown, t.screen, flags, wid as isize) {
        tag_event(&down, wid, t.local);
        CGEvent::set_integer_value_field(Some(&down), CGEventField(field_idx), field_val);
        if let Some(up) = make_nsevent(NSEventType::LeftMouseUp, t.screen, flags, wid as isize) {
            tag_event(&up, wid, t.local);
            CGEvent::set_integer_value_field(Some(&up), CGEventField(field_idx), field_val);
            post_click_to_pid(t.pid, &down, &up);
        }
    }
}

fn probe_session_tap(t: &ClickTarget) {
    eprintln!("  [S] NSEvent + tags + CGEventPost(SessionEventTap) — no activation");
    let flags = NSEventModifierFlags::empty();
    let wid = t.wid as i64;
    if let Some(down) = make_nsevent(NSEventType::LeftMouseDown, t.screen, flags, wid as isize) {
        tag_event(&down, wid, t.local);
        if let Some(up) = make_nsevent(NSEventType::LeftMouseUp, t.screen, flags, wid as isize) {
            tag_event(&up, wid, t.local);
            CGEvent::post(CGEventTapLocation::AnnotatedSessionEventTap, Some(&down));
            std::thread::sleep(Duration::from_millis(50));
            CGEvent::post(CGEventTapLocation::AnnotatedSessionEventTap, Some(&up));
        }
    }
}

fn probe_private_source(t: &ClickTarget) {
    eprintln!("  [P] CGEvent(PrivateSource) + tags + CGEventPostToPid");
    let source = CGEventSource::new(CGEventSourceStateID(0)); // kCGEventSourceStatePrivate
    let wid = t.wid as i64;
    let down = CGEvent::new_mouse_event(
        source.as_deref(), CGEventType::LeftMouseDown, t.screen, CGMouseButton::Left,
    );
    let up = CGEvent::new_mouse_event(
        source.as_deref(), CGEventType::LeftMouseUp, t.screen, CGMouseButton::Left,
    );
    if let (Some(d), Some(u)) = (down, up) {
        tag_event(&d, wid, t.local);
        CGEvent::set_flags(Some(&d), CGEventFlags(0x100000));
        tag_event(&u, wid, t.local);
        CGEvent::set_flags(Some(&u), CGEventFlags(0x100000));
        post_click_to_pid(t.pid, &d, &u);
    }
}

fn probe_wnum_zero(t: &ClickTarget) {
    eprintln!("  [W0] NSEvent windowNumber=0 + tags (wid in field 91/92) + CGEventPostToPid");
    let flags = NSEventModifierFlags::Command;
    let wid = t.wid as i64;
    if let Some(down) = make_nsevent(NSEventType::LeftMouseDown, t.screen, flags, 0) {
        tag_event(&down, wid, t.local);
        if let Some(up) = make_nsevent(NSEventType::LeftMouseUp, t.screen, flags, 0) {
            tag_event(&up, wid, t.local);
            post_click_to_pid(t.pid, &down, &up);
        }
    }
}

// ---------------------------------------------------------------------------
// Resolve target
// ---------------------------------------------------------------------------

fn find_lark_chat_target() -> Option<ClickTarget> {
    let mtm = unsafe { objc2::MainThreadMarker::new_unchecked() };
    let (pid, name) = accessibility::find_app_by_name(mtm, "Lark")?;
    eprintln!("Found: {name} (pid={pid})");

    let app = AXNode::app(pid);
    // Try to find "TD 闲聊" in the sidebar
    let node = app.locate(r#"text~="TD 闲聊""#)
        .or_else(|| app.locate(r#":has-text("TD 闲聊")"#));

    let node = match node {
        Some(n) => {
            eprintln!(
                "Found element: role={:?} title={:?} classes={:?}",
                n.role(), n.title(), n.dom_classes(),
            );
            n
        }
        None => {
            eprintln!("ERROR: Could not find 'TD 闲聊' in Lark's accessibility tree.");
            eprintln!("Trying to snapshot top-level to help debug...");
            return None;
        }
    };

    let wid = match node.window_id() {
        Some(w) => w,
        None => {
            // Walk up to find window
            let mut cur = node.parent();
            loop {
                match cur {
                    Some(ref p) => {
                        if let Some(w) = p.window_id() {
                            break w;
                        }
                        cur = p.parent();
                    }
                    None => {
                        eprintln!("ERROR: could not resolve window ID");
                        return None;
                    }
                }
            }
        }
    };

    let (px, py) = node.position().unwrap_or((0.0, 0.0));
    let (sw, sh) = node.size().unwrap_or((0.0, 0.0));
    let cx = px + sw / 2.0;
    let cy = py + sh / 2.0;

    // Find owning window position for local coords
    let mut win_node = node.parent();
    let (wx, wy) = loop {
        match win_node {
            Some(ref w) => {
                if w.role().as_deref() == Some("AXWindow") {
                    break w.position().unwrap_or((0.0, 0.0));
                }
                win_node = w.parent();
            }
            None => break (0.0, 0.0),
        }
    };

    let screen = CGPoint::new(cx, cy);
    let local = CGPoint::new(cx - wx, cy - wy);

    eprintln!("Target: wid={wid} screen=({cx:.0},{cy:.0}) local=({:.0},{:.0})", local.x, local.y);

    Some(ClickTarget { pid, wid, screen, local })
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

fn main() {
    if !accessibility::is_trusted() {
        eprintln!("ERROR: Not trusted for Accessibility. Grant permission in System Settings.");
        std::process::exit(1);
    }

    let target = match find_lark_chat_target() {
        Some(t) => t,
        None => {
            eprintln!("Failed to find target. Make sure Lark is running and 'TD 闲聊' is visible in sidebar.");
            std::process::exit(1);
        }
    };

    eprintln!("\n=== Background Click Probe ===");
    eprintln!("Keep Lark in the BACKGROUND. Each probe waits 2s for you to observe.\n");

    let probes: Vec<(&str, Box<dyn Fn(&ClickTarget)>)> = vec![
        ("baseline (current impl)", Box::new(|t| probe_baseline_cgpid(t))),
        ("no Command flag", Box::new(|t| probe_nsevent_no_cmd(t))),
        ("plain CGEvent + PostToPid", Box::new(|t| probe_cgevent_plain_pid(t))),
        ("CGEvent + tags + PostToPid", Box::new(|t| probe_cgevent_tagged_pid(t))),
        ("NSEvent + HID (no activate)", Box::new(|t| probe_nsevent_hid_no_activate(t))),
        ("NSEvent + tags + HID", Box::new(|t| probe_nsevent_hid_tagged(t))),
        ("NonCoalesced flag", Box::new(|t| probe_noncoalesced_flag(t))),
        ("subtype=0", Box::new(|t| probe_subtype_zero(t))),
        ("SessionEventTap", Box::new(|t| probe_session_tap(t))),
        ("PrivateSource + tags", Box::new(|t| probe_private_source(t))),
        ("windowNumber=0", Box::new(|t| probe_wnum_zero(t))),
    ];

    let flag_probes: Vec<(u64, &str)> = vec![
        (0x200,     "bit9 (unknown)"),
        (0x400,     "bit10 (unknown)"),
        (0x800,     "bit11 (unknown)"),
        (0x1000,    "bit12 (unknown)"),
        (0x2000,    "bit13 = NX_DEVICERCTLKEYMASK"),
        (0x4000,    "bit14 (unknown)"),
        (0x8000,    "bit15 (unknown)"),
        (0x1000000, "bit24 = NX_ALPHASHIFT_STATELESS"),
        (0x2000000, "bit25 (unknown)"),
        (0x4000000, "bit26 (unknown)"),
        (0x8000000, "bit27 (unknown)"),
        (0x10000000, "bit28 (unknown)"),
        (0x20000000, "bit29 (unknown)"),
        (0x40000000, "bit30 (unknown)"),
        (0x80000000, "bit31 (unknown)"),
    ];

    let field_probes: Vec<(u32, i64, &str)> = vec![
        (40, target.pid as i64, "TargetUnixProcessID = our pid"),
        (45, 0, "SourceStateID = Private(0)"),
        (45, 1, "SourceStateID = HID(1)"),
        (42, 0xCAFE, "SourceUserData = magic"),
        // Undocumented fields in the gap (46-87)
        (46, 1, "field46 = 1"),
        (47, 1, "field47 = 1"),
        (48, 1, "field48 = 1"),
        (49, 1, "field49 = 1"),
        (50, 1, "field50 = 1"),
        (51, 1, "field51 = 1"),
        (52, 1, "field52 = 1"),
        (53, 1, "field53 = 1"),
        (54, 1, "field54 = 1"),
        (56, 1, "field56 = 1"),
        (57, 1, "field57 = 1"),
        (58, 1, "field58 = 1"),
        (60, 1, "field60 = 1"),
        (87, 1, "field87 = 1"),
        // Fields around 100-110
        (101, 1, "field101 = 1"),
        (103, 1, "field103 = 1"),
        (104, 1, "field104 = 1"),
        (105, 1, "field105 = 1"),
        (106, 1, "field106 = 1"),
        (107, 1, "field107 = 1"),
        (109, 1, "field109 = 1"),
        (110, 1, "field110 = 1"),
    ];

    // Run menu
    let args: Vec<String> = std::env::args().collect();
    let mode = args.get(1).map(|s| s.as_str()).unwrap_or("menu");

    match mode {
        "all" => {
            for (i, (name, probe)) in probes.iter().enumerate() {
                eprintln!("\n--- Probe {i}: {name} ---");
                probe(&target);
                eprintln!("  (waiting 2s...)");
                std::thread::sleep(Duration::from_secs(2));
            }
        }
        "flags" => {
            for (flag, label) in &flag_probes {
                probe_high_flags(&target, *flag, label);
                eprintln!("  (waiting 1.5s...)");
                std::thread::sleep(Duration::from_millis(1500));
            }
        }
        "fields" => {
            for (idx, val, label) in &field_probes {
                probe_extra_fields(&target, *idx, *val, label);
                eprintln!("  (waiting 1.5s...)");
                std::thread::sleep(Duration::from_millis(1500));
            }
        }
        n if n.parse::<usize>().is_ok() => {
            let i = n.parse::<usize>().unwrap();
            if i < probes.len() {
                let (name, probe) = &probes[i];
                eprintln!("\n--- Probe {i}: {name} ---");
                probe(&target);
            } else {
                eprintln!("Probe index out of range (0..{})", probes.len() - 1);
            }
        }
        "dump" => {
            eprintln!("\n--- Dumping CGEvent fields for NSEvent-constructed mouse event ---");
            let flags = NSEventModifierFlags::Command;
            let wid = target.wid as i64;
            if let Some(ev) = make_nsevent(NSEventType::LeftMouseDown, target.screen, flags, wid as isize) {
                tag_event(&ev, wid, target.local);
                for i in 0..120 {
                    let val = CGEvent::integer_value_field(Some(&ev), CGEventField(i));
                    if val != 0 {
                        eprintln!("  field[{i:3}] = {val} (0x{val:x})");
                    }
                }
            }
            eprintln!("\n--- Dumping CGEvent fields for plain CGEventCreateMouseEvent ---");
            if let Some(ev) = make_cgevent(CGEventType::LeftMouseDown, target.screen) {
                for i in 0..120 {
                    let val = CGEvent::integer_value_field(Some(&ev), CGEventField(i));
                    if val != 0 {
                        eprintln!("  field[{i:3}] = {val} (0x{val:x})");
                    }
                }
            }
        }
        _ => {
            eprintln!("Usage: probe_bg_click <mode>");
            eprintln!("  all       — run all basic probes sequentially");
            eprintln!("  flags     — try undocumented CGEventFlags bits");
            eprintln!("  fields    — try undocumented CGEventField values");
            eprintln!("  dump      — dump field values for NSEvent vs CGEvent");
            eprintln!("  0..10     — run specific probe by index");
            eprintln!("\nProbes:");
            for (i, (name, _)) in probes.iter().enumerate() {
                eprintln!("  {i}: {name}");
            }
        }
    }

    eprintln!("\nDone.");
}
