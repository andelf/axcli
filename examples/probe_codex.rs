//! Probe Codex "Update" button with the EXACT bgclick-rev-skill recipe.
//!
//! Differences found vs our current impl:
//!   1. CGWindowID source: doc uses CGWindowListCopyWindowInfo, we use _AXUIElementGetWindow
//!   2. Location pipeline: doc does explicit CGEventSetLocation AFTER NSEvent construction
//!   3. Window-local calc: doc uses CGWindowList rect, we use AX position
//!
//! Usage:
//!   cargo run --example probe_codex
//!
//! Keep Codex in the BACKGROUND.

use std::ffi::c_void;
use std::sync::atomic::{AtomicI64, Ordering};
use std::time::Duration;

use objc2_app_kit::{
    NSEvent, NSEventModifierFlags, NSEventType, NSRunningApplication,
};
use objc2_core_foundation::CGPoint;
use objc2_core_graphics::{
    CGEvent, CGEventField, CGEventFlags, CGEventSource, CGEventSourceStateID, CGEventTapLocation,
    CGEventType, CGMouseButton,
};
use objc2_foundation::NSPoint;

use axcli::accessibility::{self, AXNode};

// --- Private APIs ---

type CGEventSetWindowLocationFn = unsafe extern "C" fn(event: *const c_void, point: CGPoint);
type CGEventSetLocationFn = unsafe extern "C" fn(event: *const c_void, point: CGPoint);

const RTLD_DEFAULT: *mut c_void = -2isize as *mut c_void;
unsafe extern "C" {
    fn dlsym(handle: *mut c_void, symbol: *const i8) -> *mut c_void;
}

fn resolve_fn<F>(name: &std::ffi::CStr) -> Option<F> {
    unsafe {
        let ptr = dlsym(RTLD_DEFAULT, name.as_ptr());
        if ptr.is_null() { None }
        else { Some(std::mem::transmute_copy::<*mut c_void, F>(&ptr)) }
    }
}

fn cg_event_set_window_location() -> Option<CGEventSetWindowLocationFn> {
    resolve_fn(c"CGEventSetWindowLocation")
}

static EVENT_COUNTER: AtomicI64 = AtomicI64::new(1);
fn next_event_number() -> i64 { EVENT_COUNTER.fetch_add(1, Ordering::Relaxed) }

// --- CGWindowList via CGWindowListCopyWindowInfo (raw C API) ---

use objc2_core_foundation::{CFIndex, CFNumberType, CFString, CFType};
use objc2_core_graphics::CGWindowListOption;

fn cg_window_info(pid: i32) -> Vec<(u32, f64, f64, f64, f64)> {
    use objc2_core_graphics::CGWindowListCopyWindowInfo;
    use objc2_core_foundation::{CFArray, CFNumber};

    let mut results = Vec::new();
    let info = unsafe { CGWindowListCopyWindowInfo(CGWindowListOption(1), 0) };
    let Some(info) = info else { return results };
    let count = info.len();

    let key_pid = CFString::from_str("kCGWindowOwnerPID");
    let key_num = CFString::from_str("kCGWindowNumber");
    let key_bounds = CFString::from_str("kCGWindowBounds");
    let key_x = CFString::from_str("X");
    let key_y = CFString::from_str("Y");
    let key_w = CFString::from_str("Width");
    let key_h = CFString::from_str("Height");

    for i in 0..count {
        let ptr = unsafe { info.as_opaque().value_at_index(i as CFIndex) };
        if ptr.is_null() { continue; }

        // Use CFDictionaryGetValue via raw FFI
        let dict_ptr = ptr as *const c_void;

        let get_val = |key: &CFString| -> *const c_void {
            unsafe { CFDictionaryGetValue(dict_ptr, key as *const CFString as *const c_void) }
        };

        let pid_ptr = get_val(&key_pid);
        if pid_ptr.is_null() { continue; }
        let pid_num = unsafe { &*(pid_ptr as *const CFNumber) };
        let mut owner_pid: i32 = 0;
        unsafe { pid_num.value(CFNumberType(3), &mut owner_pid as *mut i32 as *mut _); }
        if owner_pid != pid { continue; }

        let num_ptr = get_val(&key_num);
        if num_ptr.is_null() { continue; }
        let num = unsafe { &*(num_ptr as *const CFNumber) };
        let mut wid: i32 = 0;
        unsafe { num.value(CFNumberType(3), &mut wid as *mut i32 as *mut _); }

        let bounds_ptr = get_val(&key_bounds);
        if bounds_ptr.is_null() { continue; }

        let read_f64 = |key: &CFString| -> f64 {
            let p = unsafe { CFDictionaryGetValue(bounds_ptr, key as *const CFString as *const c_void) };
            if p.is_null() { return 0.0; }
            let n = unsafe { &*(p as *const CFNumber) };
            let mut v: f64 = 0.0;
            unsafe { n.value(CFNumberType(13), &mut v as *mut f64 as *mut _); }
            v
        };

        let x = read_f64(&key_x);
        let y = read_f64(&key_y);
        let w = read_f64(&key_w);
        let h = read_f64(&key_h);

        results.push((wid as u32, x, y, w, h));
    }
    results
}

unsafe extern "C" {
    fn CFDictionaryGetValue(dict: *const c_void, key: *const c_void) -> *const c_void;
}

// --- Helpers ---

fn app_is_active(pid: i32) -> bool {
    NSRunningApplication::runningApplicationWithProcessIdentifier(pid)
        .map_or(false, |a| a.isActive())
}

fn check_state(pid: i32, label: &str) {
    std::thread::sleep(Duration::from_millis(500));
    let active = app_is_active(pid);
    if active {
        eprintln!("    => ❌ 窗口被激活  [{label}]");
    } else {
        eprintln!("    => ✅ 窗口仍后台 (观察 UI 有无反应)  [{label}]");
    }
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

/// Full location pipeline per bgclick-rev-skill doc:
///   1. CGEventSetLocation(screenPoint)
///   2. read back CGEvent.location
///   3. transform: local = screen - windowOrigin
///   4. CGEventSetWindowLocation(localPoint)
fn full_location_pipeline(ev: &CGEvent, screen: CGPoint, win_origin: CGPoint) {
    // Step 1: explicit CGEventSetLocation
    CGEvent::set_location(Some(ev), screen);
    // Step 2: read back (verification)
    let readback = CGEvent::location(Some(ev));
    let _ = readback; // used for debugging
    // Step 3: transform
    let local = CGPoint::new(screen.x - win_origin.x, screen.y - win_origin.y);
    // Step 4: CGEventSetWindowLocation
    if let Some(fptr) = cg_event_set_window_location() {
        unsafe { fptr(ev as *const CGEvent as *const c_void, local); }
    }
}

fn click_pid(pid: i32, down: &CGEvent, up: &CGEvent) {
    CGEvent::post_to_pid(pid, Some(down));
    std::thread::sleep(Duration::from_millis(50));
    CGEvent::post_to_pid(pid, Some(up));
}

// --- Target resolution ---

struct Target {
    pid: i32,
    // From AX API
    ax_wid: Option<u32>,
    ax_screen: CGPoint,
    ax_local: CGPoint,
    ax_win_origin: CGPoint,
    // From CGWindowList API
    cg_wid: Option<u32>,
    cg_win_origin: CGPoint,
    cg_local: CGPoint,
}

fn find_target() -> Option<Target> {
    let mtm = unsafe { objc2::MainThreadMarker::new_unchecked() };
    let (pid, name) = accessibility::find_app_by_name(mtm, "Codex")?;
    eprintln!("App: {name} (pid={pid}), active={}", app_is_active(pid));

    let app = AXNode::app(pid);
    let node = app.locate(r#"text="Update""#)?;
    eprintln!("Element: role={:?} actions={:?}", node.role(), node.actions());

    // AX-based coordinates
    let (px, py) = node.position()?;
    let (sw, sh) = node.size()?;
    let cx = px + sw / 2.0;
    let cy = py + sh / 2.0;
    let ax_screen = CGPoint::new(cx, cy);

    // AX-based window ID + origin
    let ax_wid = {
        let mut cur = Some(AXNode::new(node.0.clone()));
        loop {
            match cur {
                Some(ref n) => match n.window_id() {
                    Some(w) => break Some(w),
                    None => cur = n.parent(),
                },
                None => break None,
            }
        }
    };

    let ax_win_origin = {
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
    let ax_local = CGPoint::new(cx - ax_win_origin.x, cy - ax_win_origin.y);

    // CGWindowList-based window info
    let cg_windows = cg_window_info(pid);
    eprintln!("\nCGWindowList windows for pid={pid}:");
    for (wid, x, y, w, h) in &cg_windows {
        let matches_ax = ax_wid.map_or(false, |aw| aw == *wid);
        eprintln!("  wid={wid} rect=({x:.0},{y:.0},{w:.0},{h:.0}){}", if matches_ax { " ← matches AX" } else { "" });
    }

    // Pick the CGWindowList entry matching our AX wid, or the largest window
    let cg_entry = ax_wid
        .and_then(|aw| cg_windows.iter().find(|(w, ..)| *w == aw))
        .or_else(|| cg_windows.iter().max_by(|a, b| (a.3 * a.4).partial_cmp(&(b.3 * b.4)).unwrap()));

    let (cg_wid, cg_win_origin) = match cg_entry {
        Some(&(wid, x, y, _, _)) => (Some(wid), CGPoint::new(x, y)),
        None => (None, CGPoint::new(0.0, 0.0)),
    };
    let cg_local = CGPoint::new(cx - cg_win_origin.x, cy - cg_win_origin.y);

    eprintln!("\n--- Comparison ---");
    eprintln!("  AX:  wid={:?} win_origin=({:.0},{:.0}) local=({:.0},{:.0})",
        ax_wid, ax_win_origin.x, ax_win_origin.y, ax_local.x, ax_local.y);
    eprintln!("  CG:  wid={:?} win_origin=({:.0},{:.0}) local=({:.0},{:.0})",
        cg_wid, cg_win_origin.x, cg_win_origin.y, cg_local.x, cg_local.y);
    eprintln!("  screen=({cx:.0},{cy:.0})\n");

    Some(Target {
        pid,
        ax_wid, ax_screen, ax_local, ax_win_origin,
        cg_wid, cg_win_origin, cg_local,
    })
}

// --- Probes ---

fn main() {
    if !accessibility::is_trusted() {
        eprintln!("ERROR: Accessibility not trusted");
        std::process::exit(1);
    }

    let t = match find_target() {
        Some(t) => t,
        None => { eprintln!("找不到 Codex Update 按钮"); std::process::exit(1); }
    };

    if app_is_active(t.pid) {
        eprintln!("⚠ Codex 当前是前台! 请切到别的窗口再运行.\n");
    }

    let pause = Duration::from_secs(3);

    // =======================================================
    eprintln!("=== [0] AXPress ===");
    {
        let mtm = unsafe { objc2::MainThreadMarker::new_unchecked() };
        let (pid, _) = accessibility::find_app_by_name(mtm, "Codex").unwrap();
        let app = AXNode::app(pid);
        if let Some(node) = app.locate(r#"text="Update""#) {
            eprintln!("  actions: {:?}", node.actions());
            let ok = accessibility::perform_action(&node.0, "AXPress");
            eprintln!("  AXPress => {ok}");
        }
    }
    check_state(t.pid, "AXPress");
    std::thread::sleep(pause);

    // =======================================================
    // Current implementation (AX wid, our local calc)
    eprintln!("\n=== [1] 当前实现: NSEvent+Cmd+tags(AX wid) → PostToPid ===");
    if let Some(ax_wid) = t.ax_wid {
        let wid = ax_wid as i64;
        let flags = NSEventModifierFlags::Command;
        if let Some(down) = make_nsevent(NSEventType::LeftMouseDown, t.ax_screen, flags, wid as isize) {
            tag_fields(&down, wid, t.ax_local);
            if let Some(up) = make_nsevent(NSEventType::LeftMouseUp, t.ax_screen, flags, wid as isize) {
                tag_fields(&up, wid, t.ax_local);
                click_pid(t.pid, &down, &up);
            }
        }
    } else {
        eprintln!("  SKIP: no AX window ID");
    }
    check_state(t.pid, "当前实现(AX)");
    std::thread::sleep(pause);

    // =======================================================
    // Doc recipe: CGWindowList wid + full location pipeline
    eprintln!("\n=== [2] 文档完整流程: NSEvent + CGWindowList wid + 完整 location pipeline ===");
    if let Some(cg_wid) = t.cg_wid {
        let wid = cg_wid as i64;
        let inactive = !app_is_active(t.pid);
        let flags = if inactive { NSEventModifierFlags::Command } else { NSEventModifierFlags::empty() };

        if let Some(down) = make_nsevent(NSEventType::LeftMouseDown, t.ax_screen, flags, wid as isize) {
            // Fields 91/92 + subtype + button (per doc: 4 explicit writes)
            CGEvent::set_integer_value_field(Some(&down), CGEventField::MouseEventButtonNumber, 0);
            CGEvent::set_integer_value_field(Some(&down), CGEventField::MouseEventSubtype, 3);
            CGEvent::set_integer_value_field(Some(&down), CGEventField::MouseEventWindowUnderMousePointer, wid);
            CGEvent::set_integer_value_field(Some(&down), CGEventField::MouseEventWindowUnderMousePointerThatCanHandleThisEvent, wid);
            // Full location pipeline (doc step 4)
            full_location_pipeline(&down, t.ax_screen, t.cg_win_origin);

            if let Some(up) = make_nsevent(NSEventType::LeftMouseUp, t.ax_screen, flags, wid as isize) {
                CGEvent::set_integer_value_field(Some(&up), CGEventField::MouseEventButtonNumber, 0);
                CGEvent::set_integer_value_field(Some(&up), CGEventField::MouseEventSubtype, 3);
                CGEvent::set_integer_value_field(Some(&up), CGEventField::MouseEventWindowUnderMousePointer, wid);
                CGEvent::set_integer_value_field(Some(&up), CGEventField::MouseEventWindowUnderMousePointerThatCanHandleThisEvent, wid);
                full_location_pipeline(&up, t.ax_screen, t.cg_win_origin);

                click_pid(t.pid, &down, &up);
            }
        }
    } else {
        eprintln!("  SKIP: no CG window ID");
    }
    check_state(t.pid, "文档完整流程(CG wid)");
    std::thread::sleep(pause);

    // =======================================================
    // Hybrid: AX wid + full location pipeline
    eprintln!("\n=== [3] 混合: AX wid + 完整 location pipeline(CG rect) ===");
    if let Some(ax_wid) = t.ax_wid {
        let wid = ax_wid as i64;
        let inactive = !app_is_active(t.pid);
        let flags = if inactive { NSEventModifierFlags::Command } else { NSEventModifierFlags::empty() };

        if let Some(down) = make_nsevent(NSEventType::LeftMouseDown, t.ax_screen, flags, wid as isize) {
            CGEvent::set_integer_value_field(Some(&down), CGEventField::MouseEventButtonNumber, 0);
            CGEvent::set_integer_value_field(Some(&down), CGEventField::MouseEventSubtype, 3);
            CGEvent::set_integer_value_field(Some(&down), CGEventField::MouseEventWindowUnderMousePointer, wid);
            CGEvent::set_integer_value_field(Some(&down), CGEventField::MouseEventWindowUnderMousePointerThatCanHandleThisEvent, wid);
            full_location_pipeline(&down, t.ax_screen, t.cg_win_origin);

            if let Some(up) = make_nsevent(NSEventType::LeftMouseUp, t.ax_screen, flags, wid as isize) {
                CGEvent::set_integer_value_field(Some(&up), CGEventField::MouseEventButtonNumber, 0);
                CGEvent::set_integer_value_field(Some(&up), CGEventField::MouseEventSubtype, 3);
                CGEvent::set_integer_value_field(Some(&up), CGEventField::MouseEventWindowUnderMousePointer, wid);
                CGEvent::set_integer_value_field(Some(&up), CGEventField::MouseEventWindowUnderMousePointerThatCanHandleThisEvent, wid);
                full_location_pipeline(&up, t.ax_screen, t.cg_win_origin);

                click_pid(t.pid, &down, &up);
            }
        }
    } else {
        eprintln!("  SKIP: no AX window ID");
    }
    check_state(t.pid, "AX wid + CG location pipeline");
    std::thread::sleep(pause);

    // =======================================================
    // No Command flag variant
    eprintln!("\n=== [4] 文档流程但无 Command flag ===");
    if let Some(cg_wid) = t.cg_wid {
        let wid = cg_wid as i64;
        let flags = NSEventModifierFlags::empty();

        if let Some(down) = make_nsevent(NSEventType::LeftMouseDown, t.ax_screen, flags, wid as isize) {
            CGEvent::set_integer_value_field(Some(&down), CGEventField::MouseEventButtonNumber, 0);
            CGEvent::set_integer_value_field(Some(&down), CGEventField::MouseEventSubtype, 3);
            CGEvent::set_integer_value_field(Some(&down), CGEventField::MouseEventWindowUnderMousePointer, wid);
            CGEvent::set_integer_value_field(Some(&down), CGEventField::MouseEventWindowUnderMousePointerThatCanHandleThisEvent, wid);
            full_location_pipeline(&down, t.ax_screen, t.cg_win_origin);

            if let Some(up) = make_nsevent(NSEventType::LeftMouseUp, t.ax_screen, flags, wid as isize) {
                CGEvent::set_integer_value_field(Some(&up), CGEventField::MouseEventButtonNumber, 0);
                CGEvent::set_integer_value_field(Some(&up), CGEventField::MouseEventSubtype, 3);
                CGEvent::set_integer_value_field(Some(&up), CGEventField::MouseEventWindowUnderMousePointer, wid);
                CGEvent::set_integer_value_field(Some(&up), CGEventField::MouseEventWindowUnderMousePointerThatCanHandleThisEvent, wid);
                full_location_pipeline(&up, t.ax_screen, t.cg_win_origin);

                click_pid(t.pid, &down, &up);
            }
        }
    } else {
        eprintln!("  SKIP: no CG window ID");
    }
    check_state(t.pid, "文档流程 无Command");
    std::thread::sleep(pause);

    // =======================================================
    // Explicit CGEventSetFlags after NSEvent construction
    eprintln!("\n=== [5] NSEvent(无flag) + 事后 CGEventSetFlags(Command) + 文档流程 ===");
    if let Some(cg_wid) = t.cg_wid {
        let wid = cg_wid as i64;
        // Build NSEvent WITHOUT Command flag, then set it via CGEventSetFlags
        if let Some(down) = make_nsevent(NSEventType::LeftMouseDown, t.ax_screen, NSEventModifierFlags::empty(), wid as isize) {
            CGEvent::set_flags(Some(&down), CGEventFlags(0x100000));
            CGEvent::set_integer_value_field(Some(&down), CGEventField::MouseEventButtonNumber, 0);
            CGEvent::set_integer_value_field(Some(&down), CGEventField::MouseEventSubtype, 3);
            CGEvent::set_integer_value_field(Some(&down), CGEventField::MouseEventWindowUnderMousePointer, wid);
            CGEvent::set_integer_value_field(Some(&down), CGEventField::MouseEventWindowUnderMousePointerThatCanHandleThisEvent, wid);
            full_location_pipeline(&down, t.ax_screen, t.cg_win_origin);

            if let Some(up) = make_nsevent(NSEventType::LeftMouseUp, t.ax_screen, NSEventModifierFlags::empty(), wid as isize) {
                CGEvent::set_flags(Some(&up), CGEventFlags(0x100000));
                CGEvent::set_integer_value_field(Some(&up), CGEventField::MouseEventButtonNumber, 0);
                CGEvent::set_integer_value_field(Some(&up), CGEventField::MouseEventSubtype, 3);
                CGEvent::set_integer_value_field(Some(&up), CGEventField::MouseEventWindowUnderMousePointer, wid);
                CGEvent::set_integer_value_field(Some(&up), CGEventField::MouseEventWindowUnderMousePointerThatCanHandleThisEvent, wid);
                full_location_pipeline(&up, t.ax_screen, t.cg_win_origin);

                click_pid(t.pid, &down, &up);
            }
        }
    } else {
        eprintln!("  SKIP: no CG window ID");
    }
    check_state(t.pid, "NSEvent(0)+SetFlags(Cmd)+文档流程");
    std::thread::sleep(pause);

    // =======================================================
    // Dump field diff: our impl vs doc recipe
    eprintln!("\n=== [D] 字段 dump 对比 ===");
    if let Some(cg_wid) = t.cg_wid {
        let wid = cg_wid as i64;
        let flags = NSEventModifierFlags::Command;

        eprintln!("  --- 当前实现 (tag_fields) ---");
        if let Some(ev) = make_nsevent(NSEventType::LeftMouseDown, t.ax_screen, flags, wid as isize) {
            tag_fields(&ev, wid, t.ax_local);
            dump_fields(&ev);
        }

        eprintln!("  --- 文档流程 (full_location_pipeline) ---");
        if let Some(ev) = make_nsevent(NSEventType::LeftMouseDown, t.ax_screen, flags, wid as isize) {
            CGEvent::set_integer_value_field(Some(&ev), CGEventField::MouseEventButtonNumber, 0);
            CGEvent::set_integer_value_field(Some(&ev), CGEventField::MouseEventSubtype, 3);
            CGEvent::set_integer_value_field(Some(&ev), CGEventField::MouseEventWindowUnderMousePointer, wid);
            CGEvent::set_integer_value_field(Some(&ev), CGEventField::MouseEventWindowUnderMousePointerThatCanHandleThisEvent, wid);
            full_location_pipeline(&ev, t.ax_screen, t.cg_win_origin);
            dump_fields(&ev);
        }
    }

    eprintln!("\n=== 全部完成 ===");
}

fn dump_fields(ev: &CGEvent) {
    for i in 0u32..120 {
        let val = CGEvent::integer_value_field(Some(ev), CGEventField(i));
        if val != 0 {
            eprintln!("    field[{i:3}] = {val} (0x{val:x})");
        }
    }
    let loc = CGEvent::location(Some(ev));
    eprintln!("    location = ({:.1}, {:.1})", loc.x, loc.y);
    let flags = CGEvent::flags(Some(ev));
    eprintln!("    flags = 0x{:x}", flags.0);
}
