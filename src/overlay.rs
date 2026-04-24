//! Software cursor overlay — a transparent floating window that shows a
//! crosshair + dot indicator during click/hover operations.  Purely visual;
//! does not generate any input events.
//!
//! Enabled by default.  Disable via `--no-visual-cursor` or `AXCLI_NO_VISUAL_CURSOR=1`.

use std::f64::consts::PI;
use std::time::Instant;

use objc2::MainThreadMarker;
use objc2::rc::Retained;
use objc2::msg_send;
use objc2_app_kit::{
    NSBackingStoreType, NSPanel, NSScreen, NSView, NSWindowAnimationBehavior,
    NSWindowCollectionBehavior, NSWindowStyleMask,
};
use objc2_core_foundation::{CGFloat, CGPoint, CGRect, CGSize};
use objc2_core_graphics::CGContext;

const WINDOW_SIZE: f64 = 48.0;
const DOT_RADIUS: f64 = 6.0;
const CROSS_LEN: f64 = 14.0;
const CROSS_WIDTH: f64 = 1.8;

const COLOR: (f64, f64, f64) = (0.22, 0.50, 1.0);
const DOT_ALPHA: f64 = 0.55;
const CROSS_ALPHA: f64 = 0.85;

// ── Public API ───────────────────────────────────────────────────────────────

pub fn is_enabled() -> bool {
    match std::env::var("AXCLI_NO_VISUAL_CURSOR") {
        Ok(v) => !matches!(v.trim().to_lowercase().as_str(), "1" | "true" | "yes" | "on"),
        Err(_) => true,
    }
}

pub fn animate_to_and_click(target_x: f64, target_y: f64) {
    if !is_enabled() {
        return;
    }
    let start_x = target_x - 40.0;
    let start_y = target_y - 30.0;
    run_overlay(start_x, start_y, target_x, target_y, true);
}

pub fn animate_to(target_x: f64, target_y: f64) {
    if !is_enabled() {
        return;
    }
    let start_x = target_x - 40.0;
    let start_y = target_y - 30.0;
    run_overlay(start_x, start_y, target_x, target_y, false);
}

// ── Implementation ───────────────────────────────────────────────────────────

fn run_overlay(
    start_x: f64,
    start_y: f64,
    end_x: f64,
    end_y: f64,
    click_pulse: bool,
) {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let panel = create_panel(mtm);
    place_panel(&panel, start_x, start_y, mtm);
    panel.orderFront(None);
    panel.setAlphaValue(1.0);

    let move_duration = 0.25;
    let start = Instant::now();
    loop {
        let elapsed = start.elapsed().as_secs_f64();
        let t = (elapsed / move_duration).min(1.0);
        let eased = ease_in_out(t);
        let x = start_x + (end_x - start_x) * eased;
        let y = start_y + (end_y - start_y) * eased;
        place_panel(&panel, x, y, mtm);
        redraw_panel(&panel, 0.0);
        pump_frame();
        if t >= 1.0 {
            break;
        }
    }

    if click_pulse {
        let pulse_duration = 0.15;
        let start = Instant::now();
        loop {
            let elapsed = start.elapsed().as_secs_f64();
            let t = (elapsed / pulse_duration).min(1.0);
            let progress = (t * PI).sin();
            redraw_panel(&panel, progress);
            pump_frame();
            if t >= 1.0 {
                break;
            }
        }
        redraw_panel(&panel, 0.0);
    }

    let fade_duration = 0.18;
    let start = Instant::now();
    loop {
        let elapsed = start.elapsed().as_secs_f64();
        let t = (elapsed / fade_duration).min(1.0);
        panel.setAlphaValue((1.0 - t) as CGFloat);
        pump_frame();
        if t >= 1.0 {
            break;
        }
    }

    panel.orderOut(None);
}

fn ease_in_out(t: f64) -> f64 {
    if t < 0.5 {
        2.0 * t * t
    } else {
        -1.0 + (4.0 - 2.0 * t) * t
    }
}

fn create_panel(mtm: MainThreadMarker) -> Retained<NSPanel> {
    let rect = CGRect::new(
        CGPoint::new(0.0, 0.0),
        CGSize::new(WINDOW_SIZE, WINDOW_SIZE),
    );
    let style = NSWindowStyleMask::Borderless | NSWindowStyleMask::NonactivatingPanel;
    let panel = NSPanel::initWithContentRect_styleMask_backing_defer(
        mtm.alloc::<NSPanel>(),
        rect,
        style,
        NSBackingStoreType::Buffered,
        false,
    );
    panel.setLevel(3);
    let clear = objc2_app_kit::NSColor::clearColor();
    panel.setBackgroundColor(Some(&clear));
    panel.setOpaque(false);
    panel.setHasShadow(false);
    panel.setIgnoresMouseEvents(true);
    panel.setCollectionBehavior(
        NSWindowCollectionBehavior::CanJoinAllSpaces
            | NSWindowCollectionBehavior::FullScreenAuxiliary
            | NSWindowCollectionBehavior::Stationary
            | NSWindowCollectionBehavior::IgnoresCycle,
    );
    panel.setAnimationBehavior(NSWindowAnimationBehavior(1));

    let view = NSView::initWithFrame(mtm.alloc::<NSView>(), rect);
    view.setWantsLayer(true);
    panel.setContentView(Some(&view));

    panel
}

fn place_panel(panel: &NSPanel, target_x: f64, target_y: f64, mtm: MainThreadMarker) {
    let screen_height = NSScreen::screens(mtm)
        .firstObject()
        .map(|s| s.frame().size.height)
        .unwrap_or(1080.0);
    let origin = CGPoint::new(
        target_x - WINDOW_SIZE / 2.0,
        screen_height - target_y - WINDOW_SIZE / 2.0,
    );
    panel.setFrameOrigin(origin);
}

fn redraw_panel(panel: &NSPanel, pulse: f64) {
    let Some(view) = panel.contentView() else {
        return;
    };
    let need_lock: bool = unsafe { msg_send![&view, lockFocusIfCanDraw] };
    if !need_lock {
        return;
    }
    let ctx_opt = objc2_app_kit::NSGraphicsContext::currentContext();
    let Some(ctx) = ctx_opt else {
        let () = unsafe { msg_send![&view, unlockFocus] };
        return;
    };
    let cg = ctx.CGContext();
    let bounds = CGRect::new(
        CGPoint::new(0.0, 0.0),
        CGSize::new(WINDOW_SIZE, WINDOW_SIZE),
    );
    CGContext::clear_rect(Some(&cg), bounds);

    let cx = WINDOW_SIZE / 2.0;
    let cy = WINDOW_SIZE / 2.0;

    // Dot
    let r = DOT_RADIUS + pulse * 3.0;
    let alpha = DOT_ALPHA + pulse * 0.2;
    CGContext::save_g_state(Some(&cg));
    CGContext::set_rgb_fill_color(Some(&cg), COLOR.0, COLOR.1, COLOR.2, alpha);
    let dot_rect = CGRect::new(
        CGPoint::new(cx - r, cy - r),
        CGSize::new(r * 2.0, r * 2.0),
    );
    CGContext::fill_ellipse_in_rect(Some(&cg), dot_rect);
    CGContext::restore_g_state(Some(&cg));

    // Crosshair
    CGContext::save_g_state(Some(&cg));
    CGContext::set_rgb_stroke_color(Some(&cg), COLOR.0, COLOR.1, COLOR.2, CROSS_ALPHA);
    CGContext::set_line_width(Some(&cg), CROSS_WIDTH);
    let gap = r + 2.0;
    // Horizontal
    CGContext::move_to_point(Some(&cg), cx - CROSS_LEN, cy);
    CGContext::add_line_to_point(Some(&cg), cx - gap, cy);
    CGContext::move_to_point(Some(&cg), cx + gap, cy);
    CGContext::add_line_to_point(Some(&cg), cx + CROSS_LEN, cy);
    // Vertical
    CGContext::move_to_point(Some(&cg), cx, cy - CROSS_LEN);
    CGContext::add_line_to_point(Some(&cg), cx, cy - gap);
    CGContext::move_to_point(Some(&cg), cx, cy + gap);
    CGContext::add_line_to_point(Some(&cg), cx, cy + CROSS_LEN);
    CGContext::stroke_path(Some(&cg));
    CGContext::restore_g_state(Some(&cg));

    let () = unsafe { msg_send![&view, unlockFocus] };
    unsafe {
        let _: () = msg_send![panel, flushWindow];
    }
}

fn pump_frame() {
    use objc2_core_foundation::{CFRunLoop, kCFRunLoopDefaultMode};
    let mode = unsafe { kCFRunLoopDefaultMode };
    if let Some(mode) = mode {
        let _ = CFRunLoop::run_in_mode(Some(mode), 1.0 / 120.0, true);
    }
}
