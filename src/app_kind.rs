//! Detect the runtime kind of a target application: native AppKit vs.
//! Chromium/Electron-based.  Used by the `auto` click strategy to pick a
//! delivery path that actually reaches the target.

use std::path::PathBuf;

use objc2_app_kit::NSRunningApplication;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AppKind {
    /// Plain AppKit / Cocoa application — `CGEventPostToPid` mouse events
    /// reach `-[NSWindow sendEvent:]` and route normally.
    Native,
    /// Chromium-based: Electron, CEF, Chrome itself, Edge, Brave, Arc, etc.
    /// Mouse events posted via `CGEventPostToPid` are filtered at the
    /// renderer IPC boundary and don't reach Blink content.
    Chromium,
    /// Could not determine; treat as native.
    Unknown,
}

/// Resolve the bundle path for a running pid, e.g.
/// `/Applications/LarkSuite.app`.
fn bundle_path_for_pid(pid: i32) -> Option<PathBuf> {
    let app = NSRunningApplication::runningApplicationWithProcessIdentifier(pid)?;
    let url = app.bundleURL()?;
    let path = url.path()?;
    Some(PathBuf::from(path.to_string()))
}

/// Walk a bundle's `Contents/Frameworks/` looking for any nested directory
/// that ends in `Helper*.app`.  Chromium / Electron apps always ship one or
/// more helper bundles (Renderer, GPU, Plugin, ...), so the presence of any
/// such directory is a strong signal.
fn has_helper_app(frameworks_dir: &PathBuf) -> bool {
    fn walk(dir: &PathBuf, depth: u32) -> bool {
        if depth == 0 {
            return false;
        }
        let Ok(entries) = std::fs::read_dir(dir) else { return false };
        for entry in entries.flatten() {
            let path = entry.path();
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else { continue };
            // Match patterns like "Lark Helper.app", "Code Helper (Renderer).app",
            // "Google Chrome Helper.app", "Visual Studio Code Helper (GPU).app".
            if name.ends_with(".app") && name.contains("Helper") {
                return true;
            }
            if path.is_dir() {
                if walk(&path, depth - 1) {
                    return true;
                }
            }
        }
        false
    }
    walk(frameworks_dir, 5)
}

/// Heuristically classify the running app at `pid`.
pub fn detect(pid: i32) -> AppKind {
    let Some(bundle) = bundle_path_for_pid(pid) else { return AppKind::Unknown };
    let frameworks = bundle.join("Contents").join("Frameworks");
    if !frameworks.exists() {
        return AppKind::Native;
    }
    if has_helper_app(&frameworks) {
        AppKind::Chromium
    } else {
        AppKind::Native
    }
}
