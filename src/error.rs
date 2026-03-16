//! Unified error type for accessibility operations.

use std::fmt;
use std::time::Duration;

/// Error type for accessibility CLI and automation operations.
#[derive(Debug)]
pub enum AxError {
    /// Accessibility API access not granted.
    AccessDenied,
    /// Application not found by name or PID.
    AppNotFound(String),
    /// Locator syntax is invalid.
    LocatorInvalid(String),
    /// Locator matched zero elements.
    LocatorNotFound(String),
    /// Locator matched multiple elements when exactly one was expected.
    LocatorAmbiguous { locator: String, count: usize },
    /// Element has zero size (not visible on screen).
    ElementZeroSize,
    /// An AX action (e.g. AXPress) failed.
    ActionFailed(String),
    /// An AX attribute was not available.
    AttributeNotFound(String),
    /// Timed out waiting for a locator to appear.
    Timeout { locator: String, duration: Duration },
    /// Invalid argument provided by the caller.
    InvalidArgument(String),
    /// Screenshot capture or save failed.
    ScreenshotFailed(String),
}

impl fmt::Display for AxError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AccessDenied => write!(f, "accessibility not granted"),
            Self::AppNotFound(name) => write!(f, "app not found: {name}"),
            Self::LocatorInvalid(msg) => write!(f, "invalid locator: {msg}"),
            Self::LocatorNotFound(loc) => write!(f, "locator not found: {loc}"),
            Self::LocatorAmbiguous { locator, count } => {
                write!(f, "locator matched {count} elements, must be unique for actions\nhint: use '{locator} >> nth=N' to select one")
            }
            Self::ElementZeroSize => write!(f, "element has zero size (not visible)"),
            Self::ActionFailed(action) => write!(f, "{action} failed"),
            Self::AttributeNotFound(attr) => write!(f, "no value for {attr}"),
            Self::Timeout { locator, duration } => {
                write!(f, "timeout after {:.1}s waiting for '{locator}'", duration.as_secs_f64())
            }
            Self::InvalidArgument(msg) => write!(f, "{msg}"),
            Self::ScreenshotFailed(msg) => write!(f, "screenshot failed: {msg}"),
        }
    }
}

impl std::error::Error for AxError {}

/// Map an AxError to a CLI exit code.
pub fn exit_code(e: &AxError) -> i32 {
    match e {
        AxError::AccessDenied => 1,
        AxError::AppNotFound(_) => 1,
        AxError::LocatorInvalid(_) => 1,
        AxError::LocatorNotFound(_) => 1,
        AxError::LocatorAmbiguous { .. } => 1,
        AxError::ElementZeroSize => 1,
        AxError::ActionFailed(_) => 1,
        AxError::AttributeNotFound(_) => 1,
        AxError::Timeout { .. } => 1,
        AxError::InvalidArgument(_) => 1,
        AxError::ScreenshotFailed(_) => 1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_access_denied() {
        let e = AxError::AccessDenied;
        assert_eq!(e.to_string(), "accessibility not granted");
    }

    #[test]
    fn display_app_not_found() {
        let e = AxError::AppNotFound("Lark".to_string());
        assert_eq!(e.to_string(), "app not found: Lark");
    }

    #[test]
    fn display_locator_not_found() {
        let e = AxError::LocatorNotFound(".missing".to_string());
        assert_eq!(e.to_string(), "locator not found: .missing");
    }

    #[test]
    fn display_locator_ambiguous() {
        let e = AxError::LocatorAmbiguous {
            locator: "AXButton".to_string(),
            count: 3,
        };
        let s = e.to_string();
        assert!(s.contains("3 elements"));
        assert!(s.contains("nth=N"));
    }

    #[test]
    fn display_timeout() {
        let e = AxError::Timeout {
            locator: ".loading".to_string(),
            duration: Duration::from_secs(10),
        };
        let s = e.to_string();
        assert!(s.contains("10.0s"));
        assert!(s.contains(".loading"));
    }

    #[test]
    fn display_element_zero_size() {
        assert_eq!(AxError::ElementZeroSize.to_string(), "element has zero size (not visible)");
    }

    #[test]
    fn display_screenshot_failed() {
        let e = AxError::ScreenshotFailed("capture failed".to_string());
        assert_eq!(e.to_string(), "screenshot failed: capture failed");
    }

    #[test]
    fn exit_codes_are_nonzero() {
        assert_eq!(exit_code(&AxError::AccessDenied), 1);
        assert_eq!(exit_code(&AxError::LocatorNotFound("x".into())), 1);
        assert_eq!(exit_code(&AxError::ElementZeroSize), 1);
    }

    #[test]
    fn implements_error_trait() {
        let e: Box<dyn std::error::Error> = Box::new(AxError::AccessDenied);
        let _ = e.to_string();
    }
}
