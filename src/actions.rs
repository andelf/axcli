//! High-level, Result-based operations on the accessibility tree.
//!
//! `ExecutionContext` wraps a target application and provides convenient
//! methods for common automation tasks (click, input, wait, etc.).

use std::time::{Duration, Instant};

use crate::accessibility::{self, AXNode};
use crate::error::AxError;
use crate::input;

/// Execution context bound to a specific application.
pub struct ExecutionContext {
    pub pid: i32,
    pub app: AXNode,
    pub activate_delay: Duration,
}

impl ExecutionContext {
    /// Create a new context for the given application.
    pub fn new(pid: i32, app: AXNode) -> Self {
        Self {
            pid,
            app,
            activate_delay: Duration::from_millis(200),
        }
    }

    /// Activate (bring to front) the target application and wait.
    pub fn activate(&self) {
        input::activate_app(self.pid);
        std::thread::sleep(self.activate_delay);
    }

    /// Resolve a locator to exactly one element.
    pub fn resolve_one(&self, locator: &str) -> Result<AXNode, AxError> {
        let nodes = self.app.locate_all(locator);
        match nodes.len() {
            0 => Err(AxError::LocatorNotFound(locator.to_string())),
            1 => Ok(nodes.into_iter().next().unwrap()),
            n => Err(AxError::LocatorAmbiguous {
                locator: locator.to_string(),
                count: n,
            }),
        }
    }

    /// Get the center screen coordinates of an element.
    /// Returns `Err(ElementZeroSize)` if the element has zero size and `allow_zero` is false.
    pub fn element_center(&self, node: &AXNode, allow_zero: bool) -> Result<(f64, f64), AxError> {
        let (w, h) = node.size().unwrap_or((0.0, 0.0));
        if !allow_zero && w == 0.0 && h == 0.0 {
            return Err(AxError::ElementZeroSize);
        }
        let (x, y) = node.position().unwrap_or((0.0, 0.0));
        Ok((x + w / 2.0, y + h / 2.0))
    }

    /// Click an element resolved by locator.
    pub fn click(&self, locator: &str) -> Result<(), AxError> {
        let node = self.resolve_one(locator)?;
        let role = node.role().unwrap_or_default();
        self.activate();

        if is_menu_role(&role) {
            if !accessibility::perform_action(&node.0, "AXPress") {
                return Err(AxError::ActionFailed("AXPress".to_string()));
            }
        } else {
            let (cx, cy) = self.element_center(&node, false)?;
            input::mouse_move(cx, cy);
            std::thread::sleep(Duration::from_millis(50));
            input::mouse_click(cx, cy);
        }
        Ok(())
    }

    /// Focus an element and type text.
    pub fn input_text(&self, locator: &str, text: &str) -> Result<(), AxError> {
        let node = self.resolve_one(locator)?;
        self.activate();
        self.focus_node(&node)?;
        std::thread::sleep(Duration::from_millis(200));
        input::type_text(text);
        Ok(())
    }

    /// Wait for a locator to appear, polling until timeout.
    pub fn wait_for(&self, locator: &str, timeout: Duration) -> Result<AXNode, AxError> {
        let start = Instant::now();
        loop {
            let nodes = self.app.locate_all(locator);
            if let Some(node) = nodes.into_iter().next() {
                return Ok(node);
            }
            if start.elapsed() > timeout {
                return Err(AxError::Timeout {
                    locator: locator.to_string(),
                    duration: timeout,
                });
            }
            std::thread::sleep(Duration::from_millis(200));
        }
    }

    /// Try to focus an element via AXFocused, falling back to click.
    fn focus_node(&self, node: &AXNode) -> Result<(), AxError> {
        if !node.set_focused(true) {
            let (cx, cy) = self.element_center(node, false)?;
            input::mouse_move(cx, cy);
            std::thread::sleep(Duration::from_millis(50));
            input::mouse_click(cx, cy);
        }
        Ok(())
    }
}

fn is_menu_role(role: &str) -> bool {
    role == "AXMenuItem" || role == "AXMenuBarItem"
}
