//! Software cursor overlay — a transparent floating window that shows an
//! animated cursor glyph during click/hover operations.  Purely visual; does
//! not generate any input events.
//!
//! Enable via `--visual-cursor` flag or `AXCLI_VISUAL_CURSOR=1`.

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
use objc2_core_graphics::{
    CGColorSpace, CGContext, CGGradient, CGGradientDrawingOptions, CGLineJoin,
};

// ── Metrics ──────────────────────────────────────────────────────────────────

const WINDOW_SIZE: f64 = 126.0;
const TIP_ANCHOR_X: f64 = 60.35;
const TIP_ANCHOR_Y: f64 = 70.3;
const POINTER_SIZE: f64 = 21.0;
const POINTER_OFFSET_X: f64 = 2.6;
const POINTER_OFFSET_Y: f64 = -3.2;
const FOG_DIAMETER: f64 = 66.0;

const POINTER_FILL: (f64, f64, f64, f64) = (0.20, 0.48, 0.95, 0.95);
const POINTER_STROKE: (f64, f64, f64, f64) = (1.0, 1.0, 1.0, 0.95);

// ── Public API ───────────────────────────────────────────────────────────────

pub fn is_enabled() -> bool {
    match std::env::var("AXCLI_VISUAL_CURSOR") {
        Ok(v) => !matches!(v.trim().to_lowercase().as_str(), "0" | "false" | "no" | "off"),
        Err(_) => false,
    }
}

pub fn animate_to_and_click(target_x: f64, target_y: f64) {
    if !is_enabled() {
        return;
    }
    let start_x = target_x - 60.0;
    let start_y = target_y - 50.0;
    run_overlay(start_x, start_y, target_x, target_y, true);
}

pub fn animate_to(target_x: f64, target_y: f64) {
    if !is_enabled() {
        return;
    }
    let start_x = target_x - 60.0;
    let start_y = target_y - 50.0;
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

    // Phase 1: Move animation (~300ms)
    let move_duration = 0.30;
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

    // Phase 2: Click pulse (~160ms)
    if click_pulse {
        let pulse_duration = 0.16;
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

    // Phase 3: Fade out (~200ms)
    let fade_duration = 0.20;
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
    panel.setLevel(3); // NSFloatingWindowLevel
    panel.setBackgroundColor(None);
    panel.setOpaque(false);
    panel.setHasShadow(false);
    panel.setIgnoresMouseEvents(true);
    panel.setCollectionBehavior(
        NSWindowCollectionBehavior::CanJoinAllSpaces
            | NSWindowCollectionBehavior::FullScreenAuxiliary
            | NSWindowCollectionBehavior::Stationary
            | NSWindowCollectionBehavior::IgnoresCycle,
    );
    panel.setAnimationBehavior(NSWindowAnimationBehavior(1)); // None

    let view = NSView::initWithFrame(mtm.alloc::<NSView>(), rect);
    view.setWantsLayer(true);
    panel.setContentView(Some(&view));

    panel
}

fn place_panel(panel: &NSPanel, tip_x: f64, tip_y: f64, mtm: MainThreadMarker) {
    let origin = tip_to_origin(tip_x, tip_y, mtm);
    panel.setFrameOrigin(origin);
}

fn tip_to_origin(tip_x: f64, tip_y: f64, mtm: MainThreadMarker) -> CGPoint {
    let screen_height = NSScreen::screens(mtm)
        .firstObject()
        .map(|s| s.frame().size.height)
        .unwrap_or(1080.0);
    CGPoint::new(
        tip_x - TIP_ANCHOR_X,
        screen_height - tip_y - (WINDOW_SIZE - TIP_ANCHOR_Y),
    )
}

fn redraw_panel(panel: &NSPanel, click_progress: f64) {
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
    draw_fog(&cg, bounds, click_progress);
    draw_pointer(&cg, bounds, click_progress);

    let () = unsafe { msg_send![&view, unlockFocus] };
    unsafe {
        let _: () = msg_send![panel, flushWindow];
    }
}

fn draw_fog(ctx: &CGContext, bounds: CGRect, pulse: f64) {
    let center = CGPoint::new(bounds.size.width / 2.0, bounds.size.height / 2.0);
    let radius = (FOG_DIAMETER / 2.0) + pulse * 1.2;

    let Some(color_space) = CGColorSpace::new_device_rgb() else {
        return;
    };

    let components: [CGFloat; 16] = [
        0.20, 0.48, 0.95, 0.30 + pulse * 0.02,
        0.25, 0.52, 0.95, 0.18 + pulse * 0.015,
        0.30, 0.55, 0.95, 0.07,
        0.35, 0.60, 1.00, 0.0,
    ];
    let locations: [CGFloat; 4] = [0.0, 0.50, 0.82, 1.0];

    let gradient = unsafe {
        CGGradient::with_color_components(
            Some(&color_space),
            components.as_ptr(),
            locations.as_ptr(),
            4,
        )
    };
    let Some(gradient) = gradient else {
        return;
    };

    CGContext::save_g_state(Some(ctx));
    CGContext::draw_radial_gradient(
        Some(ctx),
        Some(&gradient),
        center,
        0.0,
        center,
        radius,
        CGGradientDrawingOptions::DrawsBeforeStartLocation
            | CGGradientDrawingOptions::DrawsAfterEndLocation,
    );
    CGContext::restore_g_state(Some(ctx));
}

fn draw_pointer(ctx: &CGContext, bounds: CGRect, pulse: f64) {
    // Anchor: the arrow tip sits at the center of the view (adjusted by offsets).
    // NSView coordinates: y=0 is bottom, y increases upward.
    let tip_x = bounds.size.width / 2.0 + POINTER_OFFSET_X;
    let tip_y = bounds.size.height / 2.0 - POINTER_OFFSET_Y;

    // Standard macOS-style arrow cursor, defined relative to tip (0,0).
    // In NSView coords: tip at top, arrow body extends downward (negative y).
    let scale = POINTER_SIZE / 19.0;
    let raw: [(f64, f64); 7] = [
        (0.0,   0.0),    // tip
        (0.0, -16.5),    // left edge down
        (3.8, -12.5),    // notch left
        (6.5, -18.5),    // tail left
        (8.8, -17.2),    // tail right
        (6.0, -11.2),    // notch right
        (11.0, -11.2),   // wing right
    ];

    let points: Vec<(f64, f64)> = raw
        .iter()
        .map(|&(x, y)| (tip_x + x * scale, tip_y + y * scale))
        .collect();

    let cx = tip_x + 3.5 * scale;
    let cy = tip_y - 9.0 * scale;

    CGContext::save_g_state(Some(ctx));

    let sx = 1.0 - pulse * 0.04;
    let sy = 1.0 + pulse * 0.02;
    CGContext::translate_ctm(Some(ctx), cx, cy);
    CGContext::scale_ctm(Some(ctx), sx, sy);
    CGContext::translate_ctm(Some(ctx), -cx, -cy);

    build_pointer_path(ctx, &points);
    CGContext::set_rgb_fill_color(
        Some(ctx),
        POINTER_FILL.0,
        POINTER_FILL.1,
        POINTER_FILL.2,
        POINTER_FILL.3,
    );
    CGContext::fill_path(Some(ctx));

    build_pointer_path(ctx, &points);
    CGContext::set_line_width(Some(ctx), 1.8);
    CGContext::set_line_join(Some(ctx), CGLineJoin::Round);
    CGContext::set_rgb_stroke_color(
        Some(ctx),
        POINTER_STROKE.0,
        POINTER_STROKE.1,
        POINTER_STROKE.2,
        POINTER_STROKE.3,
    );
    CGContext::stroke_path(Some(ctx));

    CGContext::restore_g_state(Some(ctx));
}

fn build_pointer_path(ctx: &CGContext, points: &[(f64, f64)]) {
    CGContext::begin_path(Some(ctx));
    if let Some(&(x, y)) = points.first() {
        CGContext::move_to_point(Some(ctx), x, y);
    }
    for &(x, y) in &points[1..] {
        CGContext::add_line_to_point(Some(ctx), x, y);
    }
    CGContext::close_path(Some(ctx));
}

fn pump_frame() {
    use objc2_core_foundation::{CFRunLoop, kCFRunLoopDefaultMode};
    let mode = unsafe { kCFRunLoopDefaultMode };
    if let Some(mode) = mode {
        let _ = CFRunLoop::run_in_mode(Some(mode), 1.0 / 120.0, true);
    }
}
