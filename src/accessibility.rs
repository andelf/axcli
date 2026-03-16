//! macOS Accessibility API wrapper with selector-like query support.
//!
//! Wraps the AXUIElement C API via `objc2-application-services` for fast,
//! direct access to the accessibility tree.

use std::ptr::NonNull;

use objc2_application_services::{AXError, AXUIElement};
use objc2_core_foundation::{CFArray, CFIndex, CFRetained, CFString, CFType};

pub use objc2_application_services::AXIsProcessTrusted;

// ---------------------------------------------------------------------------
// Low-level convenience functions
// ---------------------------------------------------------------------------

/// Check if the current process is a trusted accessibility client.
pub fn is_trusted() -> bool {
    unsafe { AXIsProcessTrusted() }
}

/// Get a raw attribute value from an element.
pub fn attr_value(element: &AXUIElement, attribute: &str) -> Option<CFRetained<CFType>> {
    let attr = CFString::from_str(attribute);
    let mut value_ptr: *const CFType = std::ptr::null();
    let err = unsafe {
        element.copy_attribute_value(
            &attr,
            NonNull::new(&mut value_ptr as *mut *const CFType as *mut _).unwrap(),
        )
    };
    if err != AXError(0) || value_ptr.is_null() {
        return None;
    }
    // copy_attribute_value follows the Create rule (+1 retained)
    Some(unsafe { CFRetained::from_raw(NonNull::new_unchecked(value_ptr as *mut CFType)) })
}

/// Get a string attribute value.
pub fn attr_string(element: &AXUIElement, attribute: &str) -> Option<String> {
    let value = attr_value(element, attribute)?;
    value.downcast_ref::<CFString>().map(|s| s.to_string())
}

/// Read a CFArray-of-CFString attribute (e.g. AXDOMClassList) as Vec<String>.
pub fn attr_string_list(element: &AXUIElement, attribute: &str) -> Vec<String> {
    let Some(value) = attr_value(element, attribute) else {
        return Vec::new();
    };
    // The attribute is a CFArray containing CFString elements.
    let arr_ptr = value.as_ref() as *const CFType as *const CFArray;
    let arr = unsafe { &*arr_ptr };
    let count = arr.len();
    let mut result = Vec::with_capacity(count);
    for i in 0..count {
        let ptr = unsafe { arr.as_opaque().value_at_index(i as CFIndex) };
        if !ptr.is_null() {
            let s = unsafe { &*(ptr as *const CFString) };
            result.push(s.to_string());
        }
    }
    result
}

/// List all attribute names on an element.
pub fn attr_names(element: &AXUIElement) -> Vec<String> {
    let mut names_ptr: *const CFArray = std::ptr::null();
    let err = unsafe {
        element.copy_attribute_names(
            NonNull::new(&mut names_ptr as *mut *const CFArray as *mut _).unwrap(),
        )
    };
    if err != AXError(0) || names_ptr.is_null() {
        return Vec::new();
    }
    let names: CFRetained<CFArray> =
        unsafe { CFRetained::from_raw(NonNull::new_unchecked(names_ptr as *mut CFArray)) };
    let count = names.len();
    let mut result = Vec::with_capacity(count);
    for i in 0..count {
        let ptr = unsafe { names.as_opaque().value_at_index(i as CFIndex) };
        if !ptr.is_null() {
            let cf_str = unsafe { &*(ptr as *const CFString) };
            result.push(cf_str.to_string());
        }
    }
    result
}

/// List available action names on an element.
pub fn action_names(element: &AXUIElement) -> Vec<String> {
    let mut names_ptr: *const CFArray = std::ptr::null();
    let err = unsafe {
        element.copy_action_names(
            NonNull::new(&mut names_ptr as *mut *const CFArray as *mut _).unwrap(),
        )
    };
    if err != AXError(0) || names_ptr.is_null() {
        return Vec::new();
    }
    let names: CFRetained<CFArray> =
        unsafe { CFRetained::from_raw(NonNull::new_unchecked(names_ptr as *mut CFArray)) };
    let count = names.len();
    let mut result = Vec::with_capacity(count);
    for i in 0..count {
        let ptr = unsafe { names.as_opaque().value_at_index(i as CFIndex) };
        if !ptr.is_null() {
            let cf_str = unsafe { &*(ptr as *const CFString) };
            result.push(cf_str.to_string());
        }
    }
    result
}

/// Perform an action on an element.
pub fn perform_action(element: &AXUIElement, action: &str) -> bool {
    let action_name = CFString::from_str(action);
    let err = unsafe { element.perform_action(&action_name) };
    err == AXError(0)
}

/// Set an attribute value on an element. Returns true on success.
pub fn set_attr_value(element: &AXUIElement, attribute: &str, value: &CFType) -> bool {
    let attr = CFString::from_str(attribute);
    let err = unsafe { element.set_attribute_value(&attr, value) };
    err == AXError(0)
}

/// Get child elements.
pub fn children(element: &AXUIElement) -> Vec<CFRetained<AXUIElement>> {
    let value = match attr_value(element, "AXChildren") {
        Some(v) => v,
        None => return Vec::new(),
    };
    cf_type_as_ax_elements(&value)
}

// ---------------------------------------------------------------------------
// CF type conversion helpers (private)
// ---------------------------------------------------------------------------

fn cf_type_as_ax_elements(value: &CFType) -> Vec<CFRetained<AXUIElement>> {
    let array = match value.downcast_ref::<CFArray>() {
        Some(a) => a,
        None => return Vec::new(),
    };
    let count = array.len();
    let mut result = Vec::with_capacity(count);
    for i in 0..count {
        let ptr = unsafe { array.as_opaque().value_at_index(i as CFIndex) };
        if !ptr.is_null() {
            let el = unsafe {
                CFRetained::retain(NonNull::new_unchecked(ptr as *mut AXUIElement))
            };
            result.push(el);
        }
    }
    result
}

// ---------------------------------------------------------------------------
// AXQuery — selector-like query builder
// ---------------------------------------------------------------------------

/// A parsed CSS-class selector for matching AXDOMClassList.
///
/// Supports:
///   - `.class`                — has class
///   - `.class1.class2`        — has ALL classes (AND)
///   - `.class1, .class2`      — has ANY group (OR between comma-separated groups)
///   - `.class1:not(.class2)`  — has class1 AND does NOT have class2
///
/// Examples:
///   - `".message-item.message-self"` → self messages
///   - `".text-message, .text-card-message"` → text or card messages
///   - `".message-item:not(.message-self)"` → other people's messages
#[derive(Debug, Clone)]
pub struct DOMSelector {
    /// OR groups: at least one group must match.
    groups: Vec<DOMSelectorGroup>,
}

#[derive(Debug, Clone)]
struct DOMSelectorGroup {
    /// Classes that must be present.
    required: Vec<String>,
    /// Classes that must NOT be present.
    excluded: Vec<String>,
}

impl DOMSelector {
    /// Parse a CSS-class selector string.
    pub fn parse(selector: &str) -> Self {
        let groups = selector
            .split(',')
            .map(|group| {
                let group = group.trim();
                let mut required = Vec::new();
                let mut excluded = Vec::new();

                // Split by :not( to find excluded classes
                let parts: Vec<&str> = group.split(":not(").collect();
                // First part: required classes (split by '.')
                for class in parts[0].split('.') {
                    let class = class.trim();
                    if !class.is_empty() {
                        required.push(class.to_string());
                    }
                }
                // Remaining parts: excluded classes inside :not(...)
                for not_part in &parts[1..] {
                    let inner = not_part.trim_end_matches(')');
                    for class in inner.split('.') {
                        let class = class.trim();
                        if !class.is_empty() {
                            excluded.push(class.to_string());
                        }
                    }
                }
                DOMSelectorGroup { required, excluded }
            })
            .collect();
        DOMSelector { groups }
    }

    /// Check if a list of DOM classes matches this selector.
    fn matches(&self, classes: &[String]) -> bool {
        self.groups.iter().any(|g| {
            g.required.iter().all(|r| classes.iter().any(|c| c == r))
                && g.excluded.iter().all(|e| !classes.iter().any(|c| c == e))
        })
    }
}

/// A query for matching AX elements. All set fields must match (AND logic).
#[derive(Debug, Default, Clone)]
pub struct AXQuery {
    role: Option<String>,
    title: Option<String>,
    title_contains: Option<String>,
    value_contains: Option<String>,
    subrole: Option<String>,
    min_children: Option<usize>,
    has_descendant_text: Option<String>,
    dom_class: Option<String>,
    dom_sel: Option<DOMSelector>,
    predicate: Option<fn(&AXNode) -> bool>,
}

impl AXQuery {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn role(mut self, r: &str) -> Self {
        self.role = Some(r.to_string());
        self
    }

    pub fn title(mut self, t: &str) -> Self {
        self.title = Some(t.to_string());
        self
    }

    pub fn title_contains(mut self, t: &str) -> Self {
        self.title_contains = Some(t.to_string());
        self
    }

    pub fn value_contains(mut self, v: &str) -> Self {
        self.value_contains = Some(v.to_string());
        self
    }

    pub fn subrole(mut self, s: &str) -> Self {
        self.subrole = Some(s.to_string());
        self
    }

    /// Match only elements with at least `n` direct children.
    pub fn min_children(mut self, n: usize) -> Self {
        self.min_children = Some(n);
        self
    }

    /// Match only elements that contain the given text somewhere in their subtree.
    pub fn has_text(mut self, text: &str) -> Self {
        self.has_descendant_text = Some(text.to_string());
        self
    }

    /// Match elements whose AXDOMClassList contains the given class.
    pub fn dom_class(mut self, class: &str) -> Self {
        self.dom_class = Some(class.to_string());
        self
    }

    /// Match elements using a CSS-class selector string.
    ///
    /// Examples:
    /// - `".message-item.message-self"` — self messages
    /// - `".text-message, .text-card-message"` — text or card messages
    /// - `".message-item:not(.message-self)"` — others' messages
    pub fn dom_selector(mut self, selector: &str) -> Self {
        self.dom_sel = Some(DOMSelector::parse(selector));
        self
    }

    /// Match with a custom predicate function.
    pub fn filter(mut self, f: fn(&AXNode) -> bool) -> Self {
        self.predicate = Some(f);
        self
    }

    fn matches(&self, element: &AXUIElement) -> bool {
        if let Some(ref r) = self.role {
            match attr_string(element, "AXRole") {
                Some(ref v) if v == r => {}
                _ => return false,
            }
        }
        if let Some(ref t) = self.title {
            match attr_string(element, "AXTitle") {
                Some(ref v) if v == t => {}
                _ => return false,
            }
        }
        if let Some(ref t) = self.title_contains {
            match attr_string(element, "AXTitle") {
                Some(ref v) if v.contains(t.as_str()) => {}
                _ => return false,
            }
        }
        if let Some(ref vc) = self.value_contains {
            match attr_string(element, "AXValue") {
                Some(ref v) if v.contains(vc.as_str()) => {}
                _ => return false,
            }
        }
        if let Some(ref s) = self.subrole {
            match attr_string(element, "AXSubrole") {
                Some(ref v) if v == s => {}
                _ => return false,
            }
        }
        if let Some(min) = self.min_children {
            if children(element).len() < min {
                return false;
            }
        }
        if let Some(ref text) = self.has_descendant_text {
            let node = AXNode(unsafe {
                CFRetained::retain(NonNull::new_unchecked(
                    element as *const AXUIElement as *mut AXUIElement,
                ))
            });
            let found = node.texts(15).iter().any(|t| t.contains(text.as_str()));
            if !found {
                return false;
            }
        }
        // DOM class checks — lazy-read the class list only once if needed
        let need_dom = self.dom_class.is_some() || self.dom_sel.is_some();
        if need_dom {
            let classes = attr_string_list(element, "AXDOMClassList");
            if let Some(ref class) = self.dom_class {
                if !classes.iter().any(|c| c == class) {
                    return false;
                }
            }
            if let Some(ref sel) = self.dom_sel {
                if !sel.matches(&classes) {
                    return false;
                }
            }
        }
        if let Some(pred) = self.predicate {
            let node = AXNode(unsafe {
                CFRetained::retain(NonNull::new_unchecked(
                    element as *const AXUIElement as *mut AXUIElement,
                ))
            });
            if !pred(&node) {
                return false;
            }
        }
        true
    }
}

/// Convenience: build a query matching a role.
pub fn role(r: &str) -> AXQuery {
    AXQuery::new().role(r)
}

/// Convenience: build a query matching a title.
pub fn title(t: &str) -> AXQuery {
    AXQuery::new().title(t)
}

// ---------------------------------------------------------------------------
// DFS search functions
// ---------------------------------------------------------------------------

/// Find the first element matching `query`, searching up to `max_depth` levels.
pub fn find_first(
    root: &AXUIElement,
    query: &AXQuery,
    max_depth: usize,
) -> Option<CFRetained<AXUIElement>> {
    if max_depth == 0 {
        return None;
    }
    for child in children(root) {
        if query.matches(&child) {
            return Some(child);
        }
        if let Some(found) = find_first(&child, query, max_depth - 1) {
            return Some(found);
        }
    }
    None
}

/// Find all elements matching `query`, searching up to `max_depth` levels.
pub fn find_all(
    root: &AXUIElement,
    query: &AXQuery,
    max_depth: usize,
) -> Vec<CFRetained<AXUIElement>> {
    let mut results = Vec::new();
    find_all_inner(root, query, max_depth, &mut results);
    results
}

fn find_all_inner(
    root: &AXUIElement,
    query: &AXQuery,
    max_depth: usize,
    results: &mut Vec<CFRetained<AXUIElement>>,
) {
    if max_depth == 0 {
        return;
    }
    for child in children(root) {
        if query.matches(&child) {
            results.push(child.clone());
        }
        find_all_inner(&child, query, max_depth - 1, results);
    }
}

// ---------------------------------------------------------------------------
// AXNode — ergonomic wrapper
// ---------------------------------------------------------------------------

/// Lightweight wrapper around `AXUIElement` for convenient chained access.
pub struct AXNode(pub CFRetained<AXUIElement>);

impl AXNode {
    /// Create a node for an application by PID.
    pub fn app(pid: i32) -> Self {
        Self(unsafe { AXUIElement::new_application(pid) })
    }

    /// Create a node from a retained element.
    pub fn new(element: CFRetained<AXUIElement>) -> Self {
        Self(element)
    }

    pub fn role(&self) -> Option<String> {
        attr_string(&self.0, "AXRole")
    }

    pub fn title(&self) -> Option<String> {
        attr_string(&self.0, "AXTitle")
    }

    pub fn value(&self) -> Option<String> {
        attr_string(&self.0, "AXValue")
    }

    pub fn description(&self) -> Option<String> {
        attr_string(&self.0, "AXDescription")
    }

    /// Read AXDOMClassList (Electron/web apps).
    pub fn dom_classes(&self) -> Vec<String> {
        attr_string_list(&self.0, "AXDOMClassList")
    }

    /// Check if element has a specific DOM class.
    pub fn has_dom_class(&self, class: &str) -> bool {
        self.dom_classes().iter().any(|c| c == class)
    }

    pub fn subrole(&self) -> Option<String> {
        attr_string(&self.0, "AXSubrole")
    }

    pub fn attr_names(&self) -> Vec<String> {
        attr_names(&self.0)
    }

    pub fn children(&self) -> Vec<AXNode> {
        children(&self.0).into_iter().map(AXNode::new).collect()
    }

    /// Get the nth direct child (0-indexed).
    pub fn child(&self, index: usize) -> Option<AXNode> {
        children(&self.0).into_iter().nth(index).map(AXNode::new)
    }

    /// Number of direct children.
    pub fn child_count(&self) -> usize {
        children(&self.0).len()
    }

    /// Find the first direct child matching `query`.
    pub fn child_matching(&self, q: &AXQuery) -> Option<AXNode> {
        children(&self.0).into_iter().find(|c| q.matches(c)).map(AXNode::new)
    }

    /// Find all direct children matching `query`.
    pub fn children_matching(&self, q: &AXQuery) -> Vec<AXNode> {
        children(&self.0)
            .into_iter()
            .filter(|c| q.matches(c))
            .map(AXNode::new)
            .collect()
    }

    /// Find the first descendant matching `query` (max depth 20).
    pub fn find(&self, q: AXQuery) -> Option<AXNode> {
        find_first(&self.0, &q, 20).map(AXNode::new)
    }

    /// Find all descendants matching `query` (max depth 20).
    pub fn find_all(&self, q: AXQuery) -> Vec<AXNode> {
        find_all(&self.0, &q, 20)
            .into_iter()
            .map(AXNode::new)
            .collect()
    }

    /// Find an element using a locator string (as produced by `generate_locator`).
    ///
    /// ```ignore
    /// let btn = app.locate(r#"AXButton[title="Send"]"#);
    /// let el  = app.locate("#messenger-chat >> AXStaticText[text=\"Hello\"]");
    /// let nth = app.locate("AXGroup:nth(2)");
    /// ```
    pub fn locate(&self, locator: &str) -> Option<AXNode> {
        resolve_locator(&self.0, locator).map(AXNode::new)
    }

    /// Find all elements matching a locator string (supports `>>` chains).
    pub fn locate_all(&self, locator: &str) -> Vec<AXNode> {
        resolve_locator_all(&self.0, locator)
            .into_iter()
            .map(AXNode::new)
            .collect()
    }

    /// Generate a locator string that uniquely identifies `target` within this node's subtree.
    pub fn locator(&self, target: &AXNode) -> String {
        generate_locator(&self.0, &target.0)
    }

    /// Navigate a path of queries: at each step, find the first descendant
    /// matching the query (DFS), then continue from that node.
    ///
    /// Like XPath: `//step1//step2//step3`
    ///
    /// ```ignore
    /// node.select(&[
    ///     role("AXWebArea").title_contains("messenger-chat"),
    ///     AXQuery::new().min_children(5),
    /// ])
    /// ```
    pub fn select(&self, steps: &[AXQuery]) -> Option<AXNode> {
        let mut current = AXNode::new(self.0.clone());
        for step in steps {
            current = find_first(&current.0, step, 30).map(AXNode::new)?;
        }
        Some(current)
    }

    /// Navigate a path of queries using only direct children at each step.
    ///
    /// Like XPath: `/step1/step2/step3`
    pub fn select_direct(&self, steps: &[AXQuery]) -> Option<AXNode> {
        let mut current = AXNode::new(self.0.clone());
        for step in steps {
            current = current.child_matching(step)?;
        }
        Some(current)
    }

    /// Collect all AXStaticText values from descendants up to `max_depth`.
    pub fn texts(&self, max_depth: usize) -> Vec<String> {
        let elements = find_all(&self.0, &role("AXStaticText"), max_depth);
        elements
            .iter()
            .filter_map(|el| attr_string(el, "AXValue"))
            .collect()
    }

    /// Concatenate all descendant text into one string.
    pub fn text(&self, max_depth: usize) -> String {
        self.texts(max_depth).join("")
    }

    /// Check if subtree contains an element with the given role.
    pub fn has_role(&self, r: &str, max_depth: usize) -> bool {
        find_first(&self.0, &role(r), max_depth).is_some()
    }

    /// Get the parent element.
    pub fn parent(&self) -> Option<AXNode> {
        let value = attr_value(&self.0, "AXParent")?;
        // AXParent returns an AXUIElement
        let el = unsafe {
            CFRetained::retain(NonNull::new_unchecked(
                value.as_ref() as *const CFType as *mut AXUIElement,
            ))
        };
        Some(AXNode::new(el))
    }

    /// List available actions on this element.
    pub fn actions(&self) -> Vec<String> {
        action_names(&self.0)
    }

    /// Perform an action on this element (e.g. "AXPress", "AXScrollToVisible").
    pub fn perform_action(&self, action: &str) -> bool {
        perform_action(&self.0, action)
    }

    /// Get the position (x, y) of this element on screen.
    pub fn position(&self) -> Option<(f64, f64)> {
        use objc2_application_services::{AXValue, AXValueType};
        use objc2_core_foundation::CGPoint;
        let value = attr_value(&self.0, "AXPosition")?;
        let ax_val = unsafe { &*(value.as_ref() as *const CFType as *const AXValue) };
        let mut point = CGPoint { x: 0.0, y: 0.0 };
        let ok = unsafe {
            ax_val.value(
                AXValueType(1), // kAXValueCGPointType
                NonNull::new_unchecked(&mut point as *mut CGPoint as *mut _),
            )
        };
        if ok { Some((point.x, point.y)) } else { None }
    }

    /// Get the size (width, height) of this element.
    pub fn size(&self) -> Option<(f64, f64)> {
        use objc2_application_services::{AXValue, AXValueType};
        use objc2_core_foundation::CGSize;
        let value = attr_value(&self.0, "AXSize")?;
        let ax_val = unsafe { &*(value.as_ref() as *const CFType as *const AXValue) };
        let mut size = CGSize { width: 0.0, height: 0.0 };
        let ok = unsafe {
            ax_val.value(
                AXValueType(2), // kAXValueCGSizeType
                NonNull::new_unchecked(&mut size as *mut CGSize as *mut _),
            )
        };
        if ok { Some((size.width, size.height)) } else { None }
    }

    /// Set an attribute value on the underlying element.
    pub fn set_attr_value(&self, attribute: &str, value: &CFType) -> bool {
        set_attr_value(&self.0, attribute, value)
    }

    /// Set AXValue (text) on this element.
    pub fn set_value(&self, text: &str) -> bool {
        let cf_str = CFString::from_str(text);
        let cf_type: &CFType = cf_str.as_ref();
        set_attr_value(&self.0, "AXValue", cf_type)
    }

    /// Set AXFocused on this element.
    pub fn set_focused(&self, focused: bool) -> bool {
        let val: &CFType = if focused {
            unsafe { objc2_core_foundation::kCFBooleanTrue.unwrap() }.as_ref()
        } else {
            unsafe { objc2_core_foundation::kCFBooleanFalse.unwrap() }.as_ref()
        };
        set_attr_value(&self.0, "AXFocused", val)
    }
}

// ---------------------------------------------------------------------------
// App discovery
// ---------------------------------------------------------------------------

/// Find a running application by name (partial match on bundle ID or localized name).
/// Returns `(pid, localized_name)` if found.
pub fn find_app_by_name(_mtm: objc2::MainThreadMarker, name: &str) -> Option<(i32, String)> {
    use objc2_app_kit::NSRunningApplication;

    let workspace_cls = objc2::runtime::AnyClass::get(c"NSWorkspace").unwrap();
    let workspace: objc2::rc::Retained<objc2::runtime::NSObject> =
        unsafe { objc2::msg_send![workspace_cls, sharedWorkspace] };
    let apps: objc2::rc::Retained<objc2_foundation::NSArray<NSRunningApplication>> =
        unsafe { objc2::msg_send![&workspace, runningApplications] };

    let name_lower = name.to_lowercase();
    for app in apps.iter() {
        let bundle = app
            .bundleIdentifier()
            .map(|b| b.to_string())
            .unwrap_or_default();
        let localized = app
            .localizedName()
            .map(|n| n.to_string())
            .unwrap_or_default();
        if bundle.to_lowercase().contains(&name_lower)
            || localized.to_lowercase().contains(&name_lower)
        {
            return Some((app.processIdentifier(), localized));
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Locator generation — Playwright-style simplified selectors
// ---------------------------------------------------------------------------

/// Compare two AXUIElement references for identity using CFEqual.
fn is_same_element(a: &AXUIElement, b: &AXUIElement) -> bool {
    use objc2_core_foundation::CFEqual;
    CFEqual(Some(a.as_ref()), Some(b.as_ref()))
}

/// A candidate token with a score (lower = better) and the selector string.
struct Token {
    score: u32,
    selector: String,
}

/// Generate candidate tokens for an element, sorted by score (ascending).
fn candidate_tokens(el: &AXUIElement) -> Vec<Token> {
    let mut tokens = Vec::new();

    // 1. AXDOMIdentifier → #id (score 10)
    if let Some(dom_id) = attr_string(el, "AXDOMIdentifier") {
        if !dom_id.is_empty() {
            tokens.push(Token {
                score: 10,
                selector: format!("#{dom_id}"),
            });
        }
    }

    let role = attr_string(el, "AXRole").unwrap_or_default();

    // 2. Role + AXTitle (score 100)
    if let Some(title) = attr_string(el, "AXTitle") {
        if !title.is_empty() && title.len() <= 80 {
            tokens.push(Token {
                score: 100,
                selector: format!("{role}[title={:?}]", title),
            });
        }
    }

    // 3. Role + AXDescription (score 120)
    if let Some(desc) = attr_string(el, "AXDescription") {
        if !desc.is_empty() && desc.len() <= 80 {
            tokens.push(Token {
                score: 120,
                selector: format!("{role}[desc={:?}]", desc),
            });
        }
    }

    // 4. Role + AXValue ≤80 chars (score 140)
    if let Some(val) = attr_string(el, "AXValue") {
        let val = val.replace('\u{200b}', "");
        if !val.is_empty() && val.len() <= 80 {
            tokens.push(Token {
                score: 140,
                selector: format!("{role}[text={:?}]", val),
            });
        }
    }

    // 5. Role + DOMClassList (score 200)
    let classes = attr_string_list(el, "AXDOMClassList");
    if !classes.is_empty() {
        let class_str = classes.iter().map(|c| format!(".{c}")).collect::<String>();
        tokens.push(Token {
            score: 200,
            selector: format!("{role}{class_str}"),
        });
    }

    // 6. Role only (score 510)
    if !role.is_empty() {
        tokens.push(Token {
            score: 510,
            selector: role,
        });
    }

    tokens.sort_by_key(|t| t.score);
    tokens
}

/// Result of counting matches in the tree.
enum MatchResult {
    /// Found exactly the target, no other matches.
    Unique,
    /// Found 0 or more-than-1 matches.
    NotUnique,
}

/// DFS match count. A token "matches" an element if its generated candidate_tokens
/// contain a selector equal to `selector`. Early-returns when >1 match found.
fn count_matches(
    root: &AXUIElement,
    selector: &str,
    target: &AXUIElement,
    max_depth: usize,
) -> MatchResult {
    let mut found_target = false;
    let mut found_other = false;
    count_matches_inner(
        root,
        selector,
        target,
        max_depth,
        &mut found_target,
        &mut found_other,
    );
    if found_target && !found_other {
        MatchResult::Unique
    } else {
        MatchResult::NotUnique
    }
}

fn count_matches_inner(
    root: &AXUIElement,
    selector: &str,
    target: &AXUIElement,
    depth: usize,
    found_target: &mut bool,
    found_other: &mut bool,
) {
    if depth == 0 || *found_other {
        return;
    }
    for child in children(root) {
        if *found_other {
            return;
        }
        // Check if this child matches the selector
        if element_matches_selector(&child, selector) {
            if is_same_element(&child, target) {
                *found_target = true;
            } else {
                *found_other = true;
                return;
            }
        }
        count_matches_inner(&child, selector, target, depth - 1, found_target, found_other);
    }
}

/// Check if `el_role` (e.g. "AXButton") matches `selector_role` (e.g. "button", "AXButton", "text").
/// "text" is a special alias matching AXStaticText, AXTextArea, AXTextField.
fn role_matches(el_role: &str, selector_role: &str) -> bool {
    if el_role == selector_role {
        return true;
    }
    let short = el_role.strip_prefix("AX").unwrap_or(el_role).to_lowercase();
    let sel = selector_role.to_lowercase();
    if short == sel {
        return true;
    }
    if sel == "text" {
        return short == "statictext" || short == "textarea" || short == "textfield";
    }
    false
}

/// Parsed pseudo-class conditions extracted from a selector.
struct PseudoClasses<'a> {
    base: &'a str,
    has_text: Option<String>,
    has_selector: Option<String>,
    visible: bool,
    /// `:nth-child(N)` — 0-based index among all siblings (regardless of role).
    nth_child: Option<usize>,
}

/// Extract pseudo-classes from the end of a selector string.
///
/// Supported pseudo-classes:
///   - `:has-text("text")` — subtree contains text
///   - `:has(selector)` — subtree contains element matching selector
///   - `:visible` — element has non-zero size
fn parse_pseudo_classes(selector: &str) -> PseudoClasses<'_> {
    let mut base = selector;
    let mut has_text = None;
    let mut has_selector = None;
    let mut visible = false;
    let mut nth_child = None;

    // Parse from the end, peeling off pseudo-classes
    loop {
        if let Some(stripped) = base.strip_suffix(":visible") {
            base = stripped;
            visible = true;
            continue;
        }
        // :nth-child(N)
        if let Some(pos) = base.rfind(":nth-child(") {
            let after = &base[pos + ":nth-child(".len()..];
            if let Some(end) = after.strip_suffix(')') {
                if let Ok(n) = end.trim().parse::<usize>() {
                    nth_child = Some(n);
                    base = &base[..pos];
                    continue;
                }
            }
        }
        // :has-text("...") — must check before :has(...)
        if let Some(pos) = base.rfind(":has-text(") {
            let after = &base[pos + ":has-text(".len()..];
            if let Some(end) = after.strip_suffix(')') {
                let text = end.trim_matches('"').trim_matches('\'');
                has_text = Some(text.to_string());
                base = &base[..pos];
                continue;
            }
        }
        // :has(...)
        if let Some(pos) = base.rfind(":has(") {
            let after = &base[pos + ":has(".len()..];
            if let Some(end) = after.strip_suffix(')') {
                has_selector = Some(end.to_string());
                base = &base[..pos];
                continue;
            }
        }
        break;
    }

    PseudoClasses { base, has_text, has_selector, visible, nth_child }
}

/// Check if an element's subtree contains the given text.
fn subtree_has_text(el: &AXUIElement, text: &str) -> bool {
    // Check this element's own text attributes
    for attr in &["AXValue", "AXTitle", "AXDescription"] {
        if let Some(val) = attr_string(el, attr) {
            if val.contains(text) {
                return true;
            }
        }
    }
    // DFS into children
    for child in children(el) {
        if subtree_has_text(&child, text) {
            return true;
        }
    }
    false
}

/// Check if an element has non-zero size (visible).
fn element_is_visible(el: &AXUIElement) -> bool {
    use objc2_application_services::{AXValue, AXValueType};
    use objc2_core_foundation::CGSize;
    let Some(value) = attr_value(el, "AXSize") else { return false };
    let ax_val = unsafe { &*(value.as_ref() as *const CFType as *const AXValue) };
    let mut size = CGSize { width: 0.0, height: 0.0 };
    let ok = unsafe {
        ax_val.value(
            AXValueType(2),
            NonNull::new_unchecked(&mut size as *mut CGSize as *mut _),
        )
    };
    ok && size.width > 0.0 && size.height > 0.0
}

/// Check if an element is the Nth child (0-based) among all its parent's children.
fn is_nth_child(el: &AXUIElement, n: usize) -> bool {
    let Some(parent_val) = attr_value(el, "AXParent") else { return false };
    let parent = unsafe {
        &*(parent_val.as_ref() as *const CFType as *const AXUIElement)
    };
    let siblings = children(parent);
    siblings.get(n).is_some_and(|sib| is_same_element(sib, el))
}

/// Check if an element matches a selector string.
///
/// Supports pseudo-classes: `:has-text("text")`, `:has(selector)`, `:visible`, `:nth-child(N)`.
/// Supports `text=/regex/flags` for regex matching.
fn element_matches_selector(el: &AXUIElement, selector: &str) -> bool {
    // Parse pseudo-classes from the selector
    let pseudo = parse_pseudo_classes(selector);
    let base = pseudo.base;

    // If base is empty (e.g. `:has-text("Hello")`), match any element
    let base_matches = if base.is_empty() {
        true
    } else {
        element_matches_base_selector(el, base)
    };

    if !base_matches {
        return false;
    }

    // Check :visible
    if pseudo.visible && !element_is_visible(el) {
        return false;
    }

    // Check :has-text("...")
    if let Some(ref text) = pseudo.has_text {
        if !subtree_has_text(el, text) {
            return false;
        }
    }

    // Check :has(selector)
    if let Some(ref inner_sel) = pseudo.has_selector {
        let matches = collect_matching(el, inner_sel, 20);
        if matches.is_empty() {
            return false;
        }
    }

    // Check :nth-child(N) — 0-based index among ALL siblings
    if let Some(n) = pseudo.nth_child {
        if !is_nth_child(el, n) {
            return false;
        }
    }

    true
}

/// Check if an element matches the base part of a selector (no pseudo-classes).
/// CSS-style attribute match operator for bracket selectors.
#[derive(Debug, Clone, Copy, PartialEq)]
enum AttrOp {
    Exact,      // =
    Contains,   // *=
    StartsWith, // ^=
    EndsWith,   // $=
}

impl AttrOp {
    fn matches(self, haystack: &str, needle: &str) -> bool {
        match self {
            Self::Exact => haystack == needle,
            Self::Contains => haystack.contains(needle),
            Self::StartsWith => haystack.starts_with(needle),
            Self::EndsWith => haystack.ends_with(needle),
        }
    }
}

/// Parsed `[attr="val"]` / `[attr*="val"]` bracket expression.
struct AttrBracket<'a> {
    attr: &'a str,
    op: AttrOp,
    val: &'a str,
}

/// Parse the content between `[` and `]` into attr, operator, value.
///
/// Finds the first unquoted `=` to split attr from value, then checks the
/// preceding char for a 2-char operator (`*=`, `^=`, `$=`).  This avoids
/// false matches when operator chars appear inside the quoted value.
fn parse_attr_bracket(inner: &str) -> Option<AttrBracket<'_>> {
    // Find the first `=` that is NOT inside quotes.
    let mut in_quote = false;
    let mut eq_pos = None;
    for (i, ch) in inner.char_indices() {
        if ch == '"' {
            in_quote = !in_quote;
        } else if ch == '=' && !in_quote {
            eq_pos = Some(i);
            break;
        }
    }
    let eq_pos = eq_pos?;

    // Determine operator by inspecting the char before `=`.
    let (attr, op, val_start) = if eq_pos > 0 {
        match inner.as_bytes()[eq_pos - 1] {
            b'*' => (&inner[..eq_pos - 1], AttrOp::Contains, eq_pos + 1),
            b'^' => (&inner[..eq_pos - 1], AttrOp::StartsWith, eq_pos + 1),
            b'$' => (&inner[..eq_pos - 1], AttrOp::EndsWith, eq_pos + 1),
            _ => (&inner[..eq_pos], AttrOp::Exact, eq_pos + 1),
        }
    } else {
        // `=` is the first char → empty attr name
        return None;
    };

    let attr = attr.trim();
    if attr.is_empty() {
        return None;
    }

    let val = inner[val_start..].trim_matches('"');
    Some(AttrBracket { attr, op, val })
}

fn element_matches_base_selector(el: &AXUIElement, selector: &str) -> bool {
    if selector.starts_with('#') {
        // DOM ID selector
        let id = &selector[1..];
        return attr_string(el, "AXDOMIdentifier").as_deref() == Some(id);
    }

    // text=/regex/flags — regex text matching
    if let Some(rest) = selector.strip_prefix("text=") {
        if rest.starts_with('/') {
            // Parse /pattern/flags
            if let Some(last_slash) = rest[1..].rfind('/') {
                let pattern = &rest[1..1 + last_slash];
                let flags = &rest[1 + last_slash + 1..];
                let case_insensitive = flags.contains('i');
                let regex_pattern = if case_insensitive {
                    format!("(?i){pattern}")
                } else {
                    pattern.to_string()
                };
                if let Ok(re) = regex::Regex::new(&regex_pattern) {
                    return [attr_string(el, "AXValue"), attr_string(el, "AXTitle"), attr_string(el, "AXDescription")]
                        .iter()
                        .any(|v| v.as_ref().is_some_and(|s| re.is_match(s)));
                }
            }
            return false;
        }
        // text=VALUE — exact text matching
        let val = rest.trim_matches('"');
        return [attr_string(el, "AXValue"), attr_string(el, "AXTitle"), attr_string(el, "AXDescription")]
            .iter()
            .any(|v| v.as_deref() == Some(val));
    }
    if let Some(rest) = selector.strip_prefix("text~=") {
        let val = rest.trim_matches('"');
        return [attr_string(el, "AXValue"), attr_string(el, "AXTitle"), attr_string(el, "AXDescription")]
            .iter()
            .any(|v| v.as_ref().is_some_and(|s| s.contains(val)));
    }

    // Parse role[attr="value"] or role[attr*="value"] etc.
    if let Some(bracket_start) = selector.find('[') {
        let role = &selector[..bracket_start];
        let el_role = attr_string(el, "AXRole").unwrap_or_default();
        if !role_matches(&el_role, role) {
            return false;
        }
        let remainder = &selector[bracket_start + 1..];
        let Some(inner) = remainder.strip_suffix(']') else {
            return false;
        };
        if let Some(ab) = parse_attr_bracket(inner) {
            let field = match ab.attr {
                "title" | "name" => attr_string(el, "AXTitle"),
                "desc" => attr_string(el, "AXDescription"),
                "text" => attr_string(el, "AXValue").map(|v| v.replace('\u{200b}', "")),
                _ => None,
            };
            field.as_deref().is_some_and(|f| ab.op.matches(f, ab.val))
        } else {
            false
        }
    } else if selector.contains('#') {
        // role#id — match role + DOM ID (e.g. "AXGroup#root", "group#sidebar")
        let (role_part, id) = selector.split_once('#').unwrap();
        if !id.is_empty() {
            if attr_string(el, "AXDOMIdentifier").as_deref() != Some(id) {
                return false;
            }
        }
        if !role_part.is_empty() {
            let el_role = attr_string(el, "AXRole").unwrap_or_default();
            if !role_matches(&el_role, role_part) {
                return false;
            }
        }
        true
    } else if selector.contains('.') {
        // Role.class1.class2 or .class1.class2 (role optional)
        // Handle :not() within DOM class selectors
        let sel = if selector.contains(":not(") {
            selector.to_string()
        } else {
            selector.to_string()
        };
        let dom_sel = DOMSelector::parse(&sel);
        // Extract role part (before first dot)
        let role_part = sel.split('.').next().unwrap_or("");
        // Also handle :not() stripping for role check
        let role_clean = role_part.split(":not(").next().unwrap_or(role_part);
        if !role_clean.is_empty() {
            let el_role = attr_string(el, "AXRole").unwrap_or_default();
            if !role_matches(&el_role, role_clean) {
                return false;
            }
        }
        let classes = attr_string_list(el, "AXDOMClassList");
        dom_sel.matches(&classes)
    } else if selector.contains(":nth(") {
        let Some((role_part, rest)) = selector.split_once(":nth(") else { return false };
        let Some(n_str) = rest.strip_suffix(')') else { return false };
        let Ok(n) = n_str.parse::<usize>() else { return false };
        let el_role = attr_string(el, "AXRole").unwrap_or_default();
        if !role_matches(&el_role, role_part) { return false }
        nth_among_siblings(el).is_some_and(|(_, idx)| idx == n)
    } else {
        // Plain role — support both "AXButton" and "button" (case-insensitive without AX prefix)
        let el_role = attr_string(el, "AXRole").unwrap_or_default();
        role_matches(&el_role, selector)
    }
}

/// Find the nth-among-siblings index (0-based) for the element among siblings with the same role.
fn nth_among_siblings(el: &AXUIElement) -> Option<(String, usize)> {
    let role = attr_string(el, "AXRole")?;
    let parent_val = attr_value(el, "AXParent")?;
    let parent = unsafe {
        &*(parent_val.as_ref() as *const CFType as *const AXUIElement)
    };
    let siblings = children(parent);
    let mut idx = 0;
    for sib in &siblings {
        if is_same_element(sib, el) {
            return Some((role, idx));
        }
        if attr_string(sib, "AXRole").as_deref() == Some(&role) {
            idx += 1;
        }
    }
    None
}

/// Generate a Playwright-style locator that uniquely identifies `target` within `root`.
///
/// Returns a string like `#id`, `AXButton[title="Send"]`, or
/// `AXWindow[title="Main"] >> AXButton[title="Send"]`.
pub fn generate_locator(root: &AXUIElement, target: &AXUIElement) -> String {
    let target_tokens = candidate_tokens(target);

    // Step 1: Try single token
    for token in &target_tokens {
        if token.score >= 10000 {
            break; // skip index-based
        }
        if let MatchResult::Unique = count_matches(root, &token.selector, target, 50) {
            return token.selector.clone();
        }
    }

    // Step 2: Try ancestor >> target combinations (up to 5 ancestors)
    let mut best: Option<(u32, String)> = None;
    let mut current = unsafe {
        CFRetained::retain(NonNull::new_unchecked(
            target as *const AXUIElement as *mut AXUIElement,
        ))
    };

    for _ in 0..5 {
        let parent_val = match attr_value(&current, "AXParent") {
            Some(v) => v,
            None => break,
        };
        let parent = unsafe {
            CFRetained::retain(NonNull::new_unchecked(
                parent_val.as_ref() as *const CFType as *mut AXUIElement,
            ))
        };

        // Don't go above root
        if is_same_element(&parent, root) {
            break;
        }

        let ancestor_tokens = candidate_tokens(&parent);

        for at in &ancestor_tokens {
            if at.score >= 10000 {
                break;
            }
            for tt in &target_tokens {
                if tt.score >= 10000 {
                    break;
                }
                let combined_score = at.score + tt.score;
                if best.as_ref().is_some_and(|(s, _)| combined_score >= *s) {
                    continue;
                }
                let combined = format!("{} >> {}", at.selector, tt.selector);
                // For chain selectors, we need a custom check:
                // find elements matching ancestor, then within each, find target
                if chain_is_unique(root, &at.selector, &tt.selector, target) {
                    best = Some((combined_score, combined));
                }
            }
        }

        current = parent;
    }

    if let Some((_, locator)) = best {
        return locator;
    }

    // Step 3: Fallback — nth among siblings
    if let Some((role, idx)) = nth_among_siblings(target) {
        return format!("{role}:nth({idx})");
    }

    // Last resort
    "?".to_string()
}

/// Check if `ancestor_sel >> target_sel` uniquely identifies the target.
fn chain_is_unique(
    root: &AXUIElement,
    ancestor_sel: &str,
    target_sel: &str,
    target: &AXUIElement,
) -> bool {
    let mut found_target = false;
    let mut found_other = false;
    chain_check_inner(
        root,
        ancestor_sel,
        target_sel,
        target,
        50,
        &mut found_target,
        &mut found_other,
    );
    found_target && !found_other
}

fn chain_check_inner(
    root: &AXUIElement,
    ancestor_sel: &str,
    target_sel: &str,
    target: &AXUIElement,
    depth: usize,
    found_target: &mut bool,
    found_other: &mut bool,
) {
    if depth == 0 || *found_other {
        return;
    }
    for child in children(root) {
        if *found_other {
            return;
        }
        if element_matches_selector(&child, ancestor_sel) {
            // Search within this ancestor for target_sel matches
            count_matches_inner(
                &child,
                target_sel,
                target,
                50,
                found_target,
                found_other,
            );
        } else {
            chain_check_inner(
                &child,
                ancestor_sel,
                target_sel,
                target,
                depth - 1,
                found_target,
                found_other,
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Locator resolution — find element by locator string
// ---------------------------------------------------------------------------

/// Resolve a locator string to the first matching element under `root`.
///
/// Supports selector chains separated by ` >> `:
/// - `#id`
/// - `AXButton[title="Send"]`
/// - `AXGroup.class1.class2` or `.class`
/// - `text=Value` or `text~=Substring`
/// - `AXRole`
/// - `AXGroup:nth(2)` — nth among siblings with same role
/// - `nth=N` — pick Nth result from previous step
/// - `sel1 >> sel2 >> nth=0` — pipeline chain
pub fn resolve_locator(
    root: &AXUIElement,
    locator: &str,
) -> Option<CFRetained<AXUIElement>> {
    resolve_locator_all(root, locator).into_iter().next()
}

/// A parsed locator step: either a descendant search (`>>`) or direct child (`>`).
enum LocatorStep<'a> {
    /// `>>` — search all descendants
    Descendant(&'a str),
    /// `>` — search only direct children
    DirectChild(&'a str),
}

/// Parse a locator string into steps, respecting both `>>` and `>` operators.
///
/// ` >> ` splits as descendant, ` > ` splits as direct child.
/// We first split by ` >> `, then within each part split by ` > `.
fn parse_locator_steps(locator: &str) -> Vec<LocatorStep<'_>> {
    let mut steps = Vec::new();
    let desc_parts: Vec<&str> = locator.split(" >> ").collect();
    for (i, desc_part) in desc_parts.iter().enumerate() {
        // Within each >> segment, split by " > " for direct child
        let child_parts: Vec<&str> = desc_part.split(" > ").collect();
        for (j, part) in child_parts.iter().enumerate() {
            let trimmed = part.trim();
            if trimmed.is_empty() {
                continue;
            }
            if i == 0 && j == 0 {
                // First step is always descendant (from root)
                steps.push(LocatorStep::Descendant(trimmed));
            } else if j == 0 {
                // First part after >> is descendant
                steps.push(LocatorStep::Descendant(trimmed));
            } else {
                // Parts after > within a >> segment are direct child
                steps.push(LocatorStep::DirectChild(trimmed));
            }
        }
    }
    steps
}

/// Resolve a locator string to all matching elements under `root`.
///
/// Supports arbitrary-length ` >> ` and ` > ` chains. Each `>>` step
/// searches descendants, each `>` step searches only direct children.
/// Special selectors:
/// - `nth=N` — pick the Nth element (0-based) from current results
/// - `first` / `last` — pick first/last element from current results
pub fn resolve_locator_all(
    root: &AXUIElement,
    locator: &str,
) -> Vec<CFRetained<AXUIElement>> {
    let steps = parse_locator_steps(locator);

    if steps.is_empty() {
        return Vec::new();
    }

    // First step
    let mut current = match &steps[0] {
        LocatorStep::Descendant(sel) => collect_matching(root, sel, 50),
        LocatorStep::DirectChild(sel) => collect_direct_children_matching(root, sel),
    };

    // Pipeline: each subsequent step searches within current results
    for step in &steps[1..] {
        current = apply_step_typed(&current, step);
        if current.is_empty() {
            break;
        }
    }
    current
}

/// Apply a typed pipeline step (descendant or direct child).
fn apply_step_typed(elements: &[CFRetained<AXUIElement>], step: &LocatorStep<'_>) -> Vec<CFRetained<AXUIElement>> {
    match step {
        LocatorStep::Descendant(sel) => apply_step_inner(elements, sel, false),
        LocatorStep::DirectChild(sel) => apply_step_inner(elements, sel, true),
    }
}

/// Apply a single pipeline step to a set of elements.
/// If `direct_only` is true, only search direct children (not descendants).
fn apply_step_inner(elements: &[CFRetained<AXUIElement>], step: &str, direct_only: bool) -> Vec<CFRetained<AXUIElement>> {
    // nth=N — pick Nth element from current set (supports negative index)
    if let Some(n_str) = step.strip_prefix("nth=") {
        if let Ok(n) = n_str.parse::<isize>() {
            let idx = if n < 0 {
                elements.len().checked_sub((-n) as usize)
            } else {
                Some(n as usize)
            };
            return idx.and_then(|i| elements.get(i)).cloned().into_iter().collect();
        }
        return Vec::new();
    }
    // first / last — convenience aliases
    if step == "first" {
        return elements.first().cloned().into_iter().collect();
    }
    if step == "last" {
        return elements.last().cloned().into_iter().collect();
    }

    // Normal selector: search within each element
    let mut results = Vec::new();
    for el in elements {
        if direct_only {
            for child in children(el) {
                if element_matches_selector(&child, step) {
                    results.push(child);
                }
            }
        } else {
            collect_matching_inner(el, step, 50, &mut results);
        }
    }
    results
}

/// Collect direct children matching a selector (no recursion).
fn collect_direct_children_matching(
    root: &AXUIElement,
    selector: &str,
) -> Vec<CFRetained<AXUIElement>> {
    children(root)
        .into_iter()
        .filter(|child| element_matches_selector(child, selector))
        .collect()
}

/// Collect all elements matching a selector string (DFS).
pub fn collect_matching(
    root: &AXUIElement,
    selector: &str,
    max_depth: usize,
) -> Vec<CFRetained<AXUIElement>> {
    let mut results = Vec::new();
    collect_matching_inner(root, selector, max_depth, &mut results);
    results
}

fn collect_matching_inner(
    root: &AXUIElement,
    selector: &str,
    depth: usize,
    results: &mut Vec<CFRetained<AXUIElement>>,
) {
    if depth == 0 {
        return;
    }
    for child in children(root) {
        if element_matches_selector(&child, selector) {
            results.push(child.clone());
        }
        collect_matching_inner(&child, selector, depth - 1, results);
    }
}

impl std::fmt::Debug for AXNode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AXNode")
            .field("role", &self.role())
            .field("title", &self.title())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_exact_match() {
        let ab = parse_attr_bracket(r#"title="Send""#).unwrap();
        assert_eq!(ab.attr, "title");
        assert_eq!(ab.op, AttrOp::Exact);
        assert_eq!(ab.val, "Send");
    }

    #[test]
    fn parse_contains() {
        let ab = parse_attr_bracket(r#"name*="Tab Title""#).unwrap();
        assert_eq!(ab.attr, "name");
        assert_eq!(ab.op, AttrOp::Contains);
        assert_eq!(ab.val, "Tab Title");
    }

    #[test]
    fn parse_starts_with() {
        let ab = parse_attr_bracket(r#"title^="Chat""#).unwrap();
        assert_eq!(ab.attr, "title");
        assert_eq!(ab.op, AttrOp::StartsWith);
        assert_eq!(ab.val, "Chat");
    }

    #[test]
    fn parse_ends_with() {
        let ab = parse_attr_bracket(r#"desc$="ago""#).unwrap();
        assert_eq!(ab.attr, "desc");
        assert_eq!(ab.op, AttrOp::EndsWith);
        assert_eq!(ab.val, "ago");
    }

    #[test]
    fn parse_no_equals_returns_none() {
        assert!(parse_attr_bracket("title").is_none());
    }

    #[test]
    fn parse_unquoted_value() {
        let ab = parse_attr_bracket("title=Send").unwrap();
        assert_eq!(ab.val, "Send");
        assert_eq!(ab.op, AttrOp::Exact);
    }

    #[test]
    fn attr_op_exact() {
        assert!(AttrOp::Exact.matches("hello", "hello"));
        assert!(!AttrOp::Exact.matches("hello world", "hello"));
    }

    #[test]
    fn attr_op_contains() {
        assert!(AttrOp::Contains.matches("hello world", "lo wo"));
        assert!(!AttrOp::Contains.matches("hello", "xyz"));
    }

    #[test]
    fn attr_op_starts_with() {
        assert!(AttrOp::StartsWith.matches("hello world", "hello"));
        assert!(!AttrOp::StartsWith.matches("hello world", "world"));
    }

    #[test]
    fn attr_op_ends_with() {
        assert!(AttrOp::EndsWith.matches("hello world", "world"));
        assert!(!AttrOp::EndsWith.matches("hello world", "hello"));
    }

    // --- Edge case tests for bracket parser fixes ---

    #[test]
    fn parse_unclosed_bracket_returns_none() {
        // Missing closing quote — still parseable as attr content
        let ab = parse_attr_bracket(r#"title="Send"#);
        assert!(ab.is_some()); // just strips quotes
        // But truly missing `=` is None
        assert!(parse_attr_bracket("title").is_none());
    }

    #[test]
    fn parse_empty_attr_returns_none() {
        // `="val"` has no attribute name
        assert!(parse_attr_bracket(r#"="val""#).is_none());
    }

    #[test]
    fn parse_value_containing_operator_chars() {
        // Value `a*=b` should be parsed as exact match with val `a*=b`,
        // NOT as contains-operator with attr `title` and val `b`.
        let ab = parse_attr_bracket(r#"title="a*=b""#).unwrap();
        assert_eq!(ab.attr, "title");
        assert_eq!(ab.op, AttrOp::Exact);
        assert_eq!(ab.val, "a*=b");
    }

    #[test]
    fn parse_value_containing_caret_equals() {
        let ab = parse_attr_bracket(r#"name="x^=y""#).unwrap();
        assert_eq!(ab.attr, "name");
        assert_eq!(ab.op, AttrOp::Exact);
        assert_eq!(ab.val, "x^=y");
    }

    #[test]
    fn parse_value_containing_dollar_equals() {
        let ab = parse_attr_bracket(r#"desc="a$=b""#).unwrap();
        assert_eq!(ab.attr, "desc");
        assert_eq!(ab.op, AttrOp::Exact);
        assert_eq!(ab.val, "a$=b");
    }

    #[test]
    fn parse_empty_value() {
        let ab = parse_attr_bracket(r#"title="""#).unwrap();
        assert_eq!(ab.attr, "title");
        assert_eq!(ab.op, AttrOp::Exact);
        assert_eq!(ab.val, "");
    }

    #[test]
    fn parse_whitespace_around_attr() {
        let ab = parse_attr_bracket(r#" title ="Send""#).unwrap();
        assert_eq!(ab.attr, "title");
        assert_eq!(ab.op, AttrOp::Exact);
        assert_eq!(ab.val, "Send");
    }

    #[test]
    fn parse_contains_with_operator_in_value() {
        // `name*="a*=b"` — real contains operator, value happens to have `*=`
        let ab = parse_attr_bracket(r#"name*="a*=b""#).unwrap();
        assert_eq!(ab.attr, "name");
        assert_eq!(ab.op, AttrOp::Contains);
        assert_eq!(ab.val, "a*=b");
    }

    #[test]
    fn no_panic_on_various_malformed_inputs() {
        // None of these should panic
        let cases = ["", "=", "*=", "^=", "$=", "[", "]", "[]", r#""""#, "a*"];
        for case in cases {
            let _ = parse_attr_bracket(case);
        }
    }

    // --- role_matches tests ---

    #[test]
    fn role_matches_exact() {
        assert!(role_matches("AXButton", "AXButton"));
        assert!(!role_matches("AXButton", "AXGroup"));
    }

    #[test]
    fn role_matches_short_name() {
        assert!(role_matches("AXButton", "button"));
        assert!(role_matches("AXButton", "Button"));
        assert!(role_matches("AXStaticText", "statictext"));
    }

    #[test]
    fn role_matches_text_alias() {
        assert!(role_matches("AXStaticText", "text"));
        assert!(role_matches("AXTextArea", "text"));
        assert!(role_matches("AXTextField", "text"));
        assert!(!role_matches("AXButton", "text"));
    }

    // --- role#id selector parsing ---

    #[test]
    fn selector_role_hash_id_split() {
        // Verify the split logic used in element_matches_base_selector
        let selector = "AXGroup#root";
        let (role_part, id) = selector.split_once('#').unwrap();
        assert_eq!(role_part, "AXGroup");
        assert_eq!(id, "root");
    }

    #[test]
    fn selector_role_hash_id_short_role() {
        let selector = "group#sidebar";
        let (role_part, id) = selector.split_once('#').unwrap();
        assert_eq!(role_part, "group");
        assert_eq!(id, "sidebar");
        assert!(role_matches("AXGroup", role_part));
    }

    #[test]
    fn selector_hash_id_only_handled_early() {
        // Pure #id starts with '#', handled by the starts_with('#') branch,
        // not the contains('#') branch.
        let selector = "#root";
        assert!(selector.starts_with('#'));
        // so split_once would give ("", "root") — but this path isn't reached
        let (role_part, id) = selector.split_once('#').unwrap();
        assert_eq!(role_part, "");
        assert_eq!(id, "root");
    }
}
