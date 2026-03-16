//! ScreenCaptureKit-based window capture (macOS 14+).
//!
//! Captures windows by PID without requiring the target app to be in the foreground.

use std::sync::mpsc;
use std::time::Duration;

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::AnyThread;
use objc2_core_foundation::CFRetained;
use objc2_core_graphics::CGImage;
use objc2_foundation::NSError;
use objc2_core_foundation::CGRect;
use objc2_screen_capture_kit::{
    SCContentFilter, SCRunningApplication, SCScreenshotManager, SCShareableContent,
    SCStreamConfiguration, SCWindow,
};

extern "C" {
    fn CGDisplayBounds(display: u32) -> CGRect;
    fn CGGetActiveDisplayList(max: u32, displays: *mut u32, count: *mut u32) -> i32;
    fn CGDisplayCopyDisplayMode(display: u32) -> *const std::ffi::c_void;
    fn CGDisplayModeGetPixelWidth(mode: *const std::ffi::c_void) -> usize;
    fn CGDisplayModeGetWidth(mode: *const std::ffi::c_void) -> usize;
    fn CGDisplayModeRelease(mode: *const std::ffi::c_void);
}

/// Get all shareable content (windows, displays, applications).
/// Bridges the async ObjC callback to a synchronous call.
fn get_shareable_content() -> Option<Retained<SCShareableContent>> {
    let (tx, rx) = mpsc::channel();
    let block = RcBlock::new(move |content: *mut SCShareableContent, error: *mut NSError| {
        if !error.is_null() {
            let err = unsafe { &*error };
            eprintln!("SCShareableContent error: {}", err.localizedDescription());
            let _ = tx.send(None);
            return;
        }
        if content.is_null() {
            let _ = tx.send(None);
            return;
        }
        let retained = unsafe { Retained::retain(content).unwrap() };
        let _ = tx.send(Some(retained));
    });
    unsafe {
        SCShareableContent::getShareableContentExcludingDesktopWindows_onScreenWindowsOnly_completionHandler(
            true,
            false, // include off-screen windows too
            &block,
        );
    }
    rx.recv_timeout(Duration::from_secs(5)).ok().flatten()
}

/// Filter windows belonging to the given PID (layer 0, non-tiny).
fn windows_for_pid(content: &SCShareableContent, pid: i32) -> Vec<Retained<SCWindow>> {
    let windows = unsafe { content.windows() };
    let mut result = Vec::new();
    for window in windows.iter() {
        let app: Option<Retained<SCRunningApplication>> = unsafe { window.owningApplication() };
        let Some(app) = app else { continue };
        if unsafe { app.processID() } != pid {
            continue;
        }
        if unsafe { window.windowLayer() } != 0 {
            continue;
        }
        let frame = unsafe { window.frame() };
        if frame.size.width < 10.0 || frame.size.height < 10.0 {
            continue;
        }
        result.push(window.clone());
    }
    result
}

/// Find the SCK window that best matches an AX window frame (position + size).
/// Tolerance is used because AX and SCK may report slightly different coordinates.
fn find_window_by_frame(
    windows: &[Retained<SCWindow>],
    x: f64,
    y: f64,
    w: f64,
    h: f64,
) -> Option<Retained<SCWindow>> {
    let tolerance = 5.0;
    // SCK uses flipped Y (origin at top-left), same as AX, so direct comparison works.
    // But SCK's frame.origin.y may differ from AX position.y by the title bar height.
    // Use size match + x match as primary, allow y to be approximate.
    let mut best: Option<(Retained<SCWindow>, f64)> = None;
    for win in windows {
        let frame = unsafe { win.frame() };
        let dw = (frame.size.width - w).abs();
        let dh = (frame.size.height - h).abs();
        let dx = (frame.origin.x - x).abs();
        // y is less reliable: AX reports client area, SCK may include title bar
        let dy = (frame.origin.y - y).abs();
        if dw < tolerance && dh < tolerance && dx < tolerance {
            let score = dx + dy + dw + dh;
            if best.as_ref().map_or(true, |(_, bs)| score < *bs) {
                best = Some((win.clone(), score));
            }
        }
    }
    best.map(|(w, _)| w)
}

/// Find the largest window from a list.
fn find_largest_window(windows: &[Retained<SCWindow>]) -> Option<Retained<SCWindow>> {
    windows
        .iter()
        .max_by(|a, b| {
            let fa = unsafe { a.frame() };
            let fb = unsafe { b.frame() };
            let aa = fa.size.width * fa.size.height;
            let ab = fb.size.width * fb.size.height;
            aa.partial_cmp(&ab).unwrap_or(std::cmp::Ordering::Equal)
        })
        .cloned()
}

/// Determine the backing scale factor for the display containing a window.
/// Uses CGDisplayMode to compare physical pixel width vs logical width.
fn display_scale_for_frame(frame: &CGRect) -> usize {
    let win_cx = frame.origin.x + frame.size.width / 2.0;
    let win_cy = frame.origin.y + frame.size.height / 2.0;

    let mut display_ids = [0u32; 16];
    let mut count = 0u32;
    let ret = unsafe { CGGetActiveDisplayList(16, display_ids.as_mut_ptr(), &mut count) };
    if ret != 0 {
        return 2;
    }

    for &did in &display_ids[..count as usize] {
        let bounds = unsafe { CGDisplayBounds(did) };
        if win_cx >= bounds.origin.x
            && win_cx < bounds.origin.x + bounds.size.width
            && win_cy >= bounds.origin.y
            && win_cy < bounds.origin.y + bounds.size.height
        {
            let mode = unsafe { CGDisplayCopyDisplayMode(did) };
            if mode.is_null() {
                return 2;
            }
            let pixel_w = unsafe { CGDisplayModeGetPixelWidth(mode) };
            let logical_w = unsafe { CGDisplayModeGetWidth(mode) };
            unsafe { CGDisplayModeRelease(mode) };
            if logical_w > 0 {
                let scale = (pixel_w as f64 / logical_w as f64).round() as usize;
                return scale.max(1);
            }
        }
    }
    2
}

/// Capture a window image using ScreenCaptureKit's `SCScreenshotManager`.
/// The window does NOT need to be in the foreground.
fn capture_window(window: &SCWindow) -> Option<CFRetained<CGImage>> {
    let filter = unsafe {
        SCContentFilter::initWithDesktopIndependentWindow(SCContentFilter::alloc(), window)
    };

    let config = unsafe { SCStreamConfiguration::new() };
    let frame = unsafe { window.frame() };
    // Determine the actual backing scale factor for the display this window is on.
    let scale = display_scale_for_frame(&frame);
    unsafe {
        config.setWidth(frame.size.width as usize * scale);
        config.setHeight(frame.size.height as usize * scale);
        config.setShowsCursor(false);
        config.setScalesToFit(false);
    }

    let (tx, rx) = mpsc::channel();
    let block = RcBlock::new(move |image: *mut CGImage, error: *mut NSError| {
        if !error.is_null() {
            let err = unsafe { &*error };
            eprintln!("SCScreenshotManager error: {}", err.localizedDescription());
            let _ = tx.send(None);
            return;
        }
        if image.is_null() {
            let _ = tx.send(None);
            return;
        }
        let retained = unsafe { CFRetained::retain(std::ptr::NonNull::new_unchecked(image)) };
        let _ = tx.send(Some(retained));
    });

    unsafe {
        SCScreenshotManager::captureImageWithFilter_configuration_completionHandler(
            &filter, &config, Some(&block),
        );
    }
    rx.recv_timeout(Duration::from_secs(5)).ok().flatten()
}

/// Capture a specific window identified by PID + AX frame (position and size).
/// This ensures we capture the correct window when an app has multiple windows
/// across different screens.
pub fn capture_window_by_frame(
    pid: i32,
    win_x: f64,
    win_y: f64,
    win_w: f64,
    win_h: f64,
) -> Option<CFRetained<CGImage>> {
    let content = get_shareable_content()?;
    let windows = windows_for_pid(&content, pid);
    let window = find_window_by_frame(&windows, win_x, win_y, win_w, win_h)?;
    capture_window(window.as_ref())
}

/// Capture the largest window of the given PID.
/// Fallback when no specific window frame is known.
pub fn capture_window_by_pid(pid: i32) -> Option<CFRetained<CGImage>> {
    let content = get_shareable_content()?;
    let windows = windows_for_pid(&content, pid);
    let window = find_largest_window(&windows)?;
    capture_window(window.as_ref())
}
