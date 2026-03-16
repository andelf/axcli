//! Screenshot capture and image saving utilities.

use objc2_core_foundation::{CFRetained, CFString, CFURL, CFURLPathStyle, CGPoint, CGRect, CGSize};
#[allow(deprecated)]
use objc2_core_graphics::{CGImage, CGWindowImageOption, CGWindowListCreateImage, CGWindowListOption};
use objc2_image_io::CGImageDestination;

extern "C" {
    fn CGMainDisplayID() -> u32;
}

/// Ensure the CoreGraphics window server connection is initialized.
/// Must be called before any CGImage/ImageIO operations in a CLI context.
pub fn ensure_cg_init() {
    unsafe { CGMainDisplayID(); }
}

/// Capture a screenshot of the given screen rectangle.
#[allow(deprecated)]
pub fn capture(rect: CGRect) -> Option<CFRetained<CGImage>> {
    CGWindowListCreateImage(rect, CGWindowListOption(1), 0, CGWindowImageOption(0))
}

/// Capture a full-screen screenshot.
pub fn capture_full() -> Option<CFRetained<CGImage>> {
    capture(CGRect::new(CGPoint::new(0.0, 0.0), CGSize::new(0.0, 0.0)))
}

/// Save a CGImage as PNG to the given file path. Returns true on success.
pub fn save_png(image: &CGImage, path: &str) -> bool {
    let cf_path = CFString::from_str(path);
    let url = CFURL::with_file_system_path(None, Some(&cf_path), CFURLPathStyle::CFURLPOSIXPathStyle, false);
    let Some(url) = url else { return false };
    let png_type = CFString::from_str("public.png");
    let dest = unsafe { CGImageDestination::with_url(&url, &png_type, 1, None) };
    let Some(dest) = dest else { return false };
    unsafe {
        dest.add_image(image, None);
        dest.finalize()
    }
}
