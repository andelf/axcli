//! Probe Lark "TD 闲聊小群": try every bg-click strategy, verify by checking
//! if the chat panel title changes.
//!
//! Usage:
//!   cargo run --example probe_lark
//!
//! Keep Lark in the BACKGROUND.

use std::ffi::c_void;
use std::sync::atomic::{AtomicI64, Ordering};
use std::time::Duration;

use objc2_app_kit::{NSEvent, NSEventModifierFlags, NSEventType, NSRunningApplication};
use objc2_core_foundation::CGPoint;
use objc2_core_graphics::{CGEvent, CGEventField, CGEventFlags, CGEventType, CGMouseButton, CGEventSource, CGEventSourceStateID, CGEventTapLocation};
use objc2_foundation::NSPoint;

use axcli::accessibility::{self, AXNode};

// --- Private APIs ---

type CGEventSetWindowLocationFn = unsafe extern "C" fn(event: *const c_void, point: CGPoint);
const RTLD_DEFAULT: *mut c_void = -2isize as *mut c_void;
unsafe extern "C" {
    fn dlsym(handle: *mut c_void, symbol: *const i8) -> *mut c_void;
}

fn cg_event_set_window_location() -> Option<CGEventSetWindowLocationFn> {
    unsafe {
        let name = c"CGEventSetWindowLocation";
        let ptr = dlsym(RTLD_DEFAULT, name.as_ptr());
        if ptr.is_null() { None }
        else { Some(std::mem::transmute::<*mut c_void, CGEventSetWindowLocationFn>(ptr)) }
    }
}

static EVENT_COUNTER: AtomicI64 = AtomicI64::new(1);
fn next_event_number() -> i64 { EVENT_COUNTER.fetch_add(1, Ordering::Relaxed) }

// --- Helpers ---

fn app_is_active(pid: i32) -> bool {
    NSRunningApplication::runningApplicationWithProcessIdentifier(pid)
        .map_or(false, |a| a.isActive())
}

fn make_nsevent(
    ty: NSEventType, screen: CGPoint, flags: NSEventModifierFlags, wnum: isize,
) -> Option<objc2::rc::Retained<CGEvent>> {
    let ns_point = NSPoint::new(screen.x, screen.y);
    let ts = objc2_foundation::NSProcessInfo::processInfo().systemUptime();
    NSEvent::mouseEventWithType_location_modifierFlags_timestamp_windowNumber_context_eventNumber_clickCount_pressure(
        ty, ns_point, flags, ts, wnum, None, next_event_number() as isize, 1, 1.0,
    )?.CGEvent()
}

fn make_cgevent(ty: CGEventType, screen: CGPoint) -> Option<objc2_core_foundation::CFRetained<CGEvent>> {
    let source = CGEventSource::new(CGEventSourceStateID::HIDSystemState);
    CGEvent::new_mouse_event(source.as_deref(), ty, screen, CGMouseButton::Left)
}

fn tag_fields(ev: &CGEvent, wid: i64, local: CGPoint) {
    let set_win_loc = cg_event_set_window_location();
    CGEvent::set_integer_value_field(Some(ev), CGEventField::MouseEventWindowUnderMousePointer, wid);
    CGEvent::set_integer_value_field(Some(ev), CGEventField::MouseEventWindowUnderMousePointerThatCanHandleThisEvent, wid);
    CGEvent::set_integer_value_field(Some(ev), CGEventField::MouseEventSubtype, 3);
    CGEvent::set_integer_value_field(Some(ev), CGEventField::MouseEventButtonNumber, 0);
    if let Some(fptr) = set_win_loc {
        unsafe { fptr(ev as *const CGEvent as *const c_void, local); }
    }
}

fn full_location_pipeline(ev: &CGEvent, screen: CGPoint, win_origin: CGPoint) {
    CGEvent::set_location(Some(ev), screen);
    let local = CGPoint::new(screen.x - win_origin.x, screen.y - win_origin.y);
    if let Some(fptr) = cg_event_set_window_location() {
        unsafe { fptr(ev as *const CGEvent as *const c_void, local); }
    }
}

fn click_pid(pid: i32, down: &CGEvent, up: &CGEvent) {
    CGEvent::post_to_pid(pid, Some(down));
    std::thread::sleep(Duration::from_millis(50));
    CGEvent::post_to_pid(pid, Some(up));
}

fn click_hid(down: &CGEvent, up: &CGEvent) {
    CGEvent::post(CGEventTapLocation::HIDEventTap, Some(down));
    std::thread::sleep(Duration::from_millis(50));
    CGEvent::post(CGEventTapLocation::HIDEventTap, Some(up));
}

// --- Verify: check if chat panel now shows "TD 闲聊" ---

fn verify_click(pid: i32) -> &'static str {
    std::thread::sleep(Duration::from_millis(800));
    let active = app_is_active(pid);
    let app = AXNode::app(pid);
    // Check if chat panel header shows TD 闲聊
    let has_td = app.locate(r#"webarea[title*="messenger-chat"] >> text~="TD 闲聊""#).is_some()
        || app.locate(r#"text~="TD 闲聊小群""#)
            .and_then(|n| {
                // Look for it in the chat header area, not sidebar
                let parent = n.parent()?;
                let pp = parent.parent()?;
                let ppp = pp.parent()?;
                // If grandparent has class "chat-header" or similar, it's the chat panel
                let classes = ppp.dom_classes();
                if classes.iter().any(|c| c.contains("header") || c.contains("title")) {
                    Some(true)
                } else {
                    None
                }
            })
            .is_some();

    if active && has_td {
        "❌ 激活+切换 (不是后台点击)"
    } else if active && !has_td {
        "❌ 激活但未切换"
    } else if !active && has_td {
        "✅✅ 后台+切换成功!"
    } else {
        "⬚ 后台,未切换"
    }
}

// --- Target ---

struct Target {
    pid: i32,
    wid: u32,
    screen: CGPoint,
    local: CGPoint,
    win_origin: CGPoint,
}

fn find_target() -> Option<Target> {
    let mtm = unsafe { objc2::MainThreadMarker::new_unchecked() };
    let (pid, name) = accessibility::find_app_by_name(mtm, "Lark")?;
    eprintln!("App: {name} (pid={pid}), active={}", app_is_active(pid));

    let app = AXNode::app(pid);
    let node = app.locate(r#".feed-shortcut-item:has-text("TD 闲聊")"#)?;
    eprintln!("Element: role={:?} actions={:?}", node.role(), node.actions());

    let wid = {
        let mut cur = Some(AXNode::new(node.0.clone()));
        loop {
            match cur {
                Some(ref n) => match n.window_id() {
                    Some(w) => break w,
                    None => cur = n.parent(),
                },
                None => { eprintln!("ERROR: no window ID"); return None; }
            }
        }
    };

    let (px, py) = node.position()?;
    let (sw, sh) = node.size()?;
    let cx = px + sw / 2.0;
    let cy = py + sh / 2.0;

    let win_origin = {
        let mut w = node.parent();
        loop {
            match w {
                Some(ref n) if n.role().as_deref() == Some("AXWindow") =>
                    break CGPoint::new(n.position().unwrap_or((0.0, 0.0)).0, n.position().unwrap_or((0.0, 0.0)).1),
                Some(ref n) => w = n.parent(),
                None => break CGPoint::new(0.0, 0.0),
            }
        }
    };

    let screen = CGPoint::new(cx, cy);
    let local = CGPoint::new(cx - win_origin.x, cy - win_origin.y);

    eprintln!("Target: wid={wid} screen=({cx:.0},{cy:.0}) local=({:.0},{:.0}) win=({:.0},{:.0})\n",
        local.x, local.y, win_origin.x, win_origin.y);

    Some(Target { pid, wid, screen, local, win_origin })
}

// First click somewhere else to reset state (click first item which is not TD 闲聊)
fn reset_chat(pid: i32) {
    let app = AXNode::app(pid);
    if let Some(first) = app.locate(".feed-shortcut-item >> nth=0") {
        let text = first.locate(r#"text~="TD 闲聊""#);
        if text.is_some() {
            // First item IS TD 闲聊, click second instead
            if let Some(second) = app.locate(".feed-shortcut-item >> nth=1") {
                if let Some((px, py)) = second.position() {
                    if let Some((sw, sh)) = second.size() {
                        let wid = second.window_id().unwrap_or(0);
                        if wid != 0 {
                            let s = CGPoint::new(px + sw/2.0, py + sh/2.0);
                            let l = CGPoint::new(s.x, s.y); // approximate
                            eprintln!("  (reset: clicking 2nd shortcut)");
                            let flags = NSEventModifierFlags::Command;
                            if let Some(down) = make_nsevent(NSEventType::LeftMouseDown, s, flags, wid as isize) {
                                tag_fields(&down, wid as i64, l);
                                if let Some(up) = make_nsevent(NSEventType::LeftMouseUp, s, flags, wid as isize) {
                                    tag_fields(&up, wid as i64, l);
                                    click_pid(pid, &down, &up);
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    std::thread::sleep(Duration::from_millis(500));
}

fn main() {
    if !accessibility::is_trusted() {
        eprintln!("ERROR: Accessibility not trusted");
        std::process::exit(1);
    }

    let t = match find_target() {
        Some(t) => t,
        None => { eprintln!("找不到 Lark TD 闲聊"); std::process::exit(1); }
    };

    if app_is_active(t.pid) {
        eprintln!("⚠ Lark 当前是前台! 请切到别的窗口.\n");
    }

    let wid = t.wid as i64;
    let pause = Duration::from_secs(4);

    // =======================================================
    eprintln!("=== [0] AXPress ===");
    {
        let app = AXNode::app(t.pid);
        if let Some(node) = app.locate(r#".feed-shortcut-item:has-text("TD 闲聊")"#) {
            let actions = node.actions();
            eprintln!("  actions: {actions:?}");
            if actions.iter().any(|a| a == "AXPress") {
                let ok = accessibility::perform_action(&node.0, "AXPress");
                eprintln!("  AXPress => {ok}");
            } else {
                eprintln!("  AXPress not available, trying AXShowDefaultUI");
                let ok = accessibility::perform_action(&node.0, "AXShowDefaultUI");
                eprintln!("  AXShowDefaultUI => {ok}");
            }
        }
    }
    let r = verify_click(t.pid);
    eprintln!("    {r}");
    std::thread::sleep(pause);

    // =======================================================
    eprintln!("\n=== [1] 当前实现: NSEvent+Cmd+tags → PostToPid ===");
    reset_chat(t.pid);
    {
        let flags = NSEventModifierFlags::Command;
        if let Some(down) = make_nsevent(NSEventType::LeftMouseDown, t.screen, flags, wid as isize) {
            tag_fields(&down, wid, t.local);
            if let Some(up) = make_nsevent(NSEventType::LeftMouseUp, t.screen, flags, wid as isize) {
                tag_fields(&up, wid, t.local);
                click_pid(t.pid, &down, &up);
            }
        }
    }
    let r = verify_click(t.pid);
    eprintln!("    {r}");
    std::thread::sleep(pause);

    // =======================================================
    eprintln!("\n=== [2] 文档完整流程: NSEvent + full location pipeline ===");
    reset_chat(t.pid);
    {
        let inactive = !app_is_active(t.pid);
        let flags = if inactive { NSEventModifierFlags::Command } else { NSEventModifierFlags::empty() };
        if let Some(down) = make_nsevent(NSEventType::LeftMouseDown, t.screen, flags, wid as isize) {
            CGEvent::set_integer_value_field(Some(&down), CGEventField::MouseEventButtonNumber, 0);
            CGEvent::set_integer_value_field(Some(&down), CGEventField::MouseEventSubtype, 3);
            CGEvent::set_integer_value_field(Some(&down), CGEventField::MouseEventWindowUnderMousePointer, wid);
            CGEvent::set_integer_value_field(Some(&down), CGEventField::MouseEventWindowUnderMousePointerThatCanHandleThisEvent, wid);
            full_location_pipeline(&down, t.screen, t.win_origin);
            if let Some(up) = make_nsevent(NSEventType::LeftMouseUp, t.screen, flags, wid as isize) {
                CGEvent::set_integer_value_field(Some(&up), CGEventField::MouseEventButtonNumber, 0);
                CGEvent::set_integer_value_field(Some(&up), CGEventField::MouseEventSubtype, 3);
                CGEvent::set_integer_value_field(Some(&up), CGEventField::MouseEventWindowUnderMousePointer, wid);
                CGEvent::set_integer_value_field(Some(&up), CGEventField::MouseEventWindowUnderMousePointerThatCanHandleThisEvent, wid);
                full_location_pipeline(&up, t.screen, t.win_origin);
                click_pid(t.pid, &down, &up);
            }
        }
    }
    let r = verify_click(t.pid);
    eprintln!("    {r}");
    std::thread::sleep(pause);

    // =======================================================
    eprintln!("\n=== [3] plain CGEvent+tags+Cmd → PostToPid ===");
    reset_chat(t.pid);
    {
        if let Some(down) = make_cgevent(CGEventType::LeftMouseDown, t.screen) {
            tag_fields(&down, wid, t.local);
            CGEvent::set_flags(Some(&down), CGEventFlags(0x100000));
            if let Some(up) = make_cgevent(CGEventType::LeftMouseUp, t.screen) {
                tag_fields(&up, wid, t.local);
                CGEvent::set_flags(Some(&up), CGEventFlags(0x100000));
                click_pid(t.pid, &down, &up);
            }
        }
    }
    let r = verify_click(t.pid);
    eprintln!("    {r}");
    std::thread::sleep(pause);

    // =======================================================
    eprintln!("\n=== [4] NSEvent+tags+Cmd → HID (不激活, 点到最上面窗口) ===");
    reset_chat(t.pid);
    {
        let flags = NSEventModifierFlags::Command;
        if let Some(down) = make_nsevent(NSEventType::LeftMouseDown, t.screen, flags, wid as isize) {
            tag_fields(&down, wid, t.local);
            if let Some(up) = make_nsevent(NSEventType::LeftMouseUp, t.screen, flags, wid as isize) {
                tag_fields(&up, wid, t.local);
                click_hid(&down, &up);
            }
        }
    }
    let r = verify_click(t.pid);
    eprintln!("    {r}");
    std::thread::sleep(pause);

    // =======================================================
    eprintln!("\n=== [5] NSEvent 无flag无tags → PostToPid (最简) ===");
    reset_chat(t.pid);
    {
        if let Some(down) = make_nsevent(NSEventType::LeftMouseDown, t.screen, NSEventModifierFlags::empty(), wid as isize) {
            if let Some(up) = make_nsevent(NSEventType::LeftMouseUp, t.screen, NSEventModifierFlags::empty(), wid as isize) {
                click_pid(t.pid, &down, &up);
            }
        }
    }
    let r = verify_click(t.pid);
    eprintln!("    {r}");
    std::thread::sleep(pause);

    // =======================================================
    eprintln!("\n=== [6] CGEvent(PrivateSource)+tags+Cmd → PostToPid ===");
    reset_chat(t.pid);
    {
        let source = CGEventSource::new(CGEventSourceStateID(0));
        let down = CGEvent::new_mouse_event(source.as_deref(), CGEventType::LeftMouseDown, t.screen, CGMouseButton::Left);
        let up = CGEvent::new_mouse_event(source.as_deref(), CGEventType::LeftMouseUp, t.screen, CGMouseButton::Left);
        if let (Some(d), Some(u)) = (down, up) {
            tag_fields(&d, wid, t.local);
            CGEvent::set_flags(Some(&d), CGEventFlags(0x100000));
            tag_fields(&u, wid, t.local);
            CGEvent::set_flags(Some(&u), CGEventFlags(0x100000));
            click_pid(t.pid, &d, &u);
        }
    }
    let r = verify_click(t.pid);
    eprintln!("    {r}");
    std::thread::sleep(pause);

    // =======================================================
    eprintln!("\n=== [7] NSEvent(flags=0) + 事后 CGEventSetFlags(Cmd) + tags → PostToPid ===");
    reset_chat(t.pid);
    {
        if let Some(down) = make_nsevent(NSEventType::LeftMouseDown, t.screen, NSEventModifierFlags::empty(), wid as isize) {
            CGEvent::set_flags(Some(&down), CGEventFlags(0x100000));
            tag_fields(&down, wid, t.local);
            if let Some(up) = make_nsevent(NSEventType::LeftMouseUp, t.screen, NSEventModifierFlags::empty(), wid as isize) {
                CGEvent::set_flags(Some(&up), CGEventFlags(0x100000));
                tag_fields(&up, wid, t.local);
                click_pid(t.pid, &down, &up);
            }
        }
    }
    let r = verify_click(t.pid);
    eprintln!("    {r}");

    eprintln!("\n=== 全部完成 ===");
}
