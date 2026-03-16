//! Playwright-style Locator API for macOS Accessibility tree queries.
//!
//! `Locator` is a lazy, chainable query builder. Each action method
//! (e.g. `resolve`, `click`) re-traverses the AX tree from the root,
//! applying filter steps in order.
//!
//! # Examples
//!
//! ```ignore
//! let app = AXNode::app(pid);
//!
//! // Semantic factories
//! let btn = app.get_by_role("AXButton", "Send");
//! let field = app.get_by_title("Username");
//!
//! // Chaining
//! let row = app
//!     .get_by_role("AXGroup", "")
//!     .filter(|f| f.has_text("Mary"))
//!     .get_by_role("AXButton", "");
//!
//! // Actions (resolve on demand)
//! let node = btn.resolve();
//! let ok = btn.click();
//! ```

use crate::accessibility::{attr_string, children, find_all, AXNode, AXQuery};
use objc2_application_services::AXUIElement;
use objc2_core_foundation::CFRetained;

// ---------------------------------------------------------------------------
// Core types
// ---------------------------------------------------------------------------

/// A lazy, composable element locator.
///
/// Stores a root element and a chain of filter steps. The AX tree is only
/// traversed when an action method (`resolve`, `click`, …) is called.
pub struct Locator {
    root: CFRetained<AXUIElement>,
    steps: Vec<LocatorStep>,
}

enum LocatorStep {
    // Semantic factories — search descendants
    Role(String),
    RoleWithName(String, String),
    Text(String),
    Title(String),
    Description(String),
    DomId(String),
    DomClass(String),
    Query(AXQuery),
    // Filtering
    Filter(FilterCriteria),
    // Selection
    Nth(usize),
    First,
    Last,
}

/// Criteria for filtering candidates.
pub struct FilterCriteria {
    has_text: Option<String>,
    has_not_text: Option<String>,
    has: Option<Box<Locator>>,
    has_not: Option<Box<Locator>>,
    predicate: Option<fn(&AXNode) -> bool>,
}

impl FilterCriteria {
    fn new() -> Self {
        Self {
            has_text: None,
            has_not_text: None,
            has: None,
            has_not: None,
            predicate: None,
        }
    }

    /// Require that the element's subtree contains the given text.
    pub fn has_text(&mut self, text: &str) -> &mut Self {
        self.has_text = Some(text.to_string());
        self
    }

    /// Require that the element's subtree does NOT contain the given text.
    pub fn has_not_text(&mut self, text: &str) -> &mut Self {
        self.has_not_text = Some(text.to_string());
        self
    }

    /// Require that a sub-locator matches at least one descendant.
    pub fn has(&mut self, locator: Locator) -> &mut Self {
        self.has = Some(Box::new(locator));
        self
    }

    /// Require that a sub-locator does NOT match any descendant.
    pub fn has_not(&mut self, locator: Locator) -> &mut Self {
        self.has_not = Some(Box::new(locator));
        self
    }

    /// Require a custom predicate to return true.
    pub fn predicate(&mut self, f: fn(&AXNode) -> bool) -> &mut Self {
        self.predicate = Some(f);
        self
    }
}

// ---------------------------------------------------------------------------
// AXNode factory methods
// ---------------------------------------------------------------------------

impl AXNode {
    /// Create a locator that matches descendants with the given role and name.
    ///
    /// If `name` is empty, only the role is matched.
    pub fn get_by_role(&self, role: &str, name: &str) -> Locator {
        let step = if name.is_empty() {
            LocatorStep::Role(role.to_string())
        } else {
            LocatorStep::RoleWithName(role.to_string(), name.to_string())
        };
        Locator {
            root: self.0.clone(),
            steps: vec![step],
        }
    }

    /// Create a locator matching descendants whose subtree text contains `text`.
    pub fn get_by_text(&self, text: &str) -> Locator {
        Locator {
            root: self.0.clone(),
            steps: vec![LocatorStep::Text(text.to_string())],
        }
    }

    /// Create a locator matching descendants with the exact AXTitle.
    pub fn get_by_title(&self, title: &str) -> Locator {
        Locator {
            root: self.0.clone(),
            steps: vec![LocatorStep::Title(title.to_string())],
        }
    }

    /// Create a locator matching descendants with the exact AXDescription.
    pub fn get_by_description(&self, desc: &str) -> Locator {
        Locator {
            root: self.0.clone(),
            steps: vec![LocatorStep::Description(desc.to_string())],
        }
    }

    /// Create a locator matching descendants by AXDOMIdentifier.
    ///
    /// Note: DOM IDs are NOT guaranteed to be unique across an app — Electron
    /// apps like Lark may have multiple `AXWebArea` subtrees, each containing
    /// its own `#root`. Chain with a prior step to narrow scope first:
    ///
    /// ```ignore
    /// app.get_by_role("AXWindow", "Lark")
    ///     .query(role("AXWebArea"))
    ///     .first()
    ///     .get_by_dom_id("root")
    /// ```
    pub fn get_by_dom_id(&self, id: &str) -> Locator {
        Locator {
            root: self.0.clone(),
            steps: vec![LocatorStep::DomId(id.to_string())],
        }
    }

    /// Create a locator matching descendants by DOM class.
    pub fn get_by_dom_class(&self, class: &str) -> Locator {
        Locator {
            root: self.0.clone(),
            steps: vec![LocatorStep::DomClass(class.to_string())],
        }
    }

    /// Create a locator from an existing `AXQuery`.
    pub fn query(&self, q: AXQuery) -> Locator {
        Locator {
            root: self.0.clone(),
            steps: vec![LocatorStep::Query(q)],
        }
    }
}

// ---------------------------------------------------------------------------
// Locator — chaining methods
// ---------------------------------------------------------------------------

impl Locator {
    /// Narrow results: search within each candidate's subtree for the given role+name.
    pub fn get_by_role(mut self, role: &str, name: &str) -> Self {
        let step = if name.is_empty() {
            LocatorStep::Role(role.to_string())
        } else {
            LocatorStep::RoleWithName(role.to_string(), name.to_string())
        };
        self.steps.push(step);
        self
    }

    /// Narrow results: search within each candidate's subtree for text.
    pub fn get_by_text(mut self, text: &str) -> Self {
        self.steps.push(LocatorStep::Text(text.to_string()));
        self
    }

    /// Narrow results: search within each candidate's subtree for exact AXTitle.
    pub fn get_by_title(mut self, title: &str) -> Self {
        self.steps.push(LocatorStep::Title(title.to_string()));
        self
    }

    /// Narrow results: search within each candidate's subtree for exact AXDescription.
    pub fn get_by_description(mut self, desc: &str) -> Self {
        self.steps
            .push(LocatorStep::Description(desc.to_string()));
        self
    }

    /// Narrow results: search within each candidate's subtree by DOM ID.
    ///
    /// See [`AXNode::get_by_dom_id`] for a note on DOM ID uniqueness.
    pub fn get_by_dom_id(mut self, id: &str) -> Self {
        self.steps.push(LocatorStep::DomId(id.to_string()));
        self
    }

    /// Narrow results: search within each candidate's subtree by DOM class.
    pub fn get_by_dom_class(mut self, class: &str) -> Self {
        self.steps.push(LocatorStep::DomClass(class.to_string()));
        self
    }

    /// Narrow results using an `AXQuery`.
    pub fn query(mut self, q: AXQuery) -> Self {
        self.steps.push(LocatorStep::Query(q));
        self
    }

    /// Filter candidates using `FilterCriteria`.
    ///
    /// ```ignore
    /// app.get_by_role("AXGroup", "")
    ///     .filter(|f| f.has_text("Mary"))
    /// ```
    pub fn filter(mut self, f: impl FnOnce(&mut FilterCriteria)) -> Self {
        let mut criteria = FilterCriteria::new();
        f(&mut criteria);
        self.steps.push(LocatorStep::Filter(criteria));
        self
    }

    /// Select the nth candidate (0-indexed).
    pub fn nth(mut self, n: usize) -> Self {
        self.steps.push(LocatorStep::Nth(n));
        self
    }

    /// Select the first candidate.
    pub fn first(mut self) -> Self {
        self.steps.push(LocatorStep::First);
        self
    }

    /// Select the last candidate.
    pub fn last(mut self) -> Self {
        self.steps.push(LocatorStep::Last);
        self
    }
}

// ---------------------------------------------------------------------------
// Locator — resolve engine
// ---------------------------------------------------------------------------

const MAX_DEPTH: usize = 30;

impl Locator {
    /// Resolve all matching elements.
    pub fn resolve_all(&self) -> Vec<AXNode> {
        let root_node = AXNode::new(self.root.clone());
        let mut candidates = vec![root_node];

        for step in &self.steps {
            candidates = apply_step(step, candidates);
            if candidates.is_empty() {
                return Vec::new();
            }
        }

        candidates
    }

    /// Resolve the first matching element.
    pub fn resolve(&self) -> Option<AXNode> {
        self.resolve_all().into_iter().next()
    }

    /// Resolve exactly one element. Panics if count != 1.
    pub fn resolve_one(&self) -> AXNode {
        let results = self.resolve_all();
        assert!(
            results.len() == 1,
            "resolve_one: expected 1 match, got {}",
            results.len()
        );
        results.into_iter().next().unwrap()
    }

    /// Alias for `resolve_all`.
    pub fn all(&self) -> Vec<AXNode> {
        self.resolve_all()
    }

    /// Count matching elements.
    pub fn count(&self) -> usize {
        self.resolve_all().len()
    }
}

// ---------------------------------------------------------------------------
// Locator — action methods
// ---------------------------------------------------------------------------

impl Locator {
    /// Get concatenated text content of the first match's subtree.
    pub fn text_content(&self) -> Option<String> {
        self.resolve().map(|n| n.text(15))
    }

    /// Get concatenated text content for each matching element.
    pub fn all_text_contents(&self) -> Vec<String> {
        self.resolve_all().iter().map(|n| n.text(15)).collect()
    }

    /// Get AXTitle of the first match.
    pub fn title(&self) -> Option<String> {
        self.resolve().and_then(|n| n.title())
    }

    /// Get AXValue of the first match.
    pub fn value(&self) -> Option<String> {
        self.resolve().and_then(|n| n.value())
    }

    /// Get bounding box (x, y, width, height) of the first match.
    pub fn bounding_box(&self) -> Option<(f64, f64, f64, f64)> {
        let node = self.resolve()?;
        let (x, y) = node.position()?;
        let (w, h) = node.size()?;
        Some((x, y, w, h))
    }

    /// Click the center of the first match by performing AXPress.
    pub fn click(&self) -> bool {
        self.resolve()
            .map(|n| n.perform_action("AXPress"))
            .unwrap_or(false)
    }

    /// Set focus on the first match.
    pub fn focus(&self) -> bool {
        self.resolve()
            .map(|n| n.set_focused(true))
            .unwrap_or(false)
    }

    /// Perform an arbitrary AX action on the first match.
    pub fn perform(&self, action: &str) -> bool {
        self.resolve()
            .map(|n| n.perform_action(action))
            .unwrap_or(false)
    }

    /// Set AXValue on the first match.
    pub fn set_value(&self, text: &str) -> bool {
        self.resolve()
            .map(|n| n.set_value(text))
            .unwrap_or(false)
    }
}

// ---------------------------------------------------------------------------
// Step evaluation
// ---------------------------------------------------------------------------

fn apply_step(step: &LocatorStep, candidates: Vec<AXNode>) -> Vec<AXNode> {
    match step {
        // --- Factory steps: search within each candidate's subtree ---
        LocatorStep::Role(role) => {
            let q = AXQuery::new().role(role);
            search_descendants(candidates, &q)
        }
        LocatorStep::RoleWithName(role, name) => {
            let mut results = Vec::new();
            for c in &candidates {
                let q = AXQuery::new().role(role);
                let matches = find_all(&c.0, &q, MAX_DEPTH);
                for m in matches {
                    let t = attr_string(&m, "AXTitle").unwrap_or_default();
                    let d = attr_string(&m, "AXDescription").unwrap_or_default();
                    if t == *name || d == *name {
                        results.push(AXNode::new(m));
                    }
                }
            }
            results
        }
        LocatorStep::Text(text) => {
            let q = AXQuery::new().has_text(text);
            search_descendants(candidates, &q)
        }
        LocatorStep::Title(title) => {
            let q = AXQuery::new().title(title);
            search_descendants(candidates, &q)
        }
        LocatorStep::Description(desc) => {
            let mut results = Vec::new();
            for c in &candidates {
                find_by_description(&c.0, desc, MAX_DEPTH, &mut results);
            }
            results
        }
        LocatorStep::DomId(id) => {
            let mut results = Vec::new();
            for c in &candidates {
                find_by_dom_id(&c.0, id, MAX_DEPTH, &mut results);
            }
            results
        }
        LocatorStep::DomClass(class) => {
            let q = AXQuery::new().dom_class(class);
            search_descendants(candidates, &q)
        }
        LocatorStep::Query(q) => search_descendants(candidates, q),

        // --- Filter step ---
        LocatorStep::Filter(criteria) => candidates
            .into_iter()
            .filter(|node| matches_filter(node, criteria))
            .collect(),

        // --- Selection steps ---
        LocatorStep::First => candidates.into_iter().take(1).collect(),
        LocatorStep::Last => candidates.into_iter().last().into_iter().collect(),
        LocatorStep::Nth(n) => candidates.into_iter().nth(*n).into_iter().collect(),
    }
}

fn search_descendants(candidates: Vec<AXNode>, q: &AXQuery) -> Vec<AXNode> {
    let mut results = Vec::new();
    for c in &candidates {
        let matches = find_all(&c.0, q, MAX_DEPTH);
        results.extend(matches.into_iter().map(AXNode::new));
    }
    results
}

fn find_by_description(
    root: &AXUIElement,
    desc: &str,
    max_depth: usize,
    results: &mut Vec<AXNode>,
) {
    if max_depth == 0 {
        return;
    }
    for child in children(root) {
        if attr_string(&child, "AXDescription").as_deref() == Some(desc) {
            results.push(AXNode::new(child.clone()));
        }
        find_by_description(&child, desc, max_depth - 1, results);
    }
}

fn find_by_dom_id(root: &AXUIElement, id: &str, max_depth: usize, results: &mut Vec<AXNode>) {
    if max_depth == 0 {
        return;
    }
    for child in children(root) {
        if attr_string(&child, "AXDOMIdentifier").as_deref() == Some(id) {
            results.push(AXNode::new(child.clone()));
        }
        find_by_dom_id(&child, id, max_depth - 1, results);
    }
}

fn matches_filter(node: &AXNode, criteria: &FilterCriteria) -> bool {
    if let Some(ref text) = criteria.has_text {
        let texts = node.text(15);
        if !texts.contains(text.as_str()) {
            return false;
        }
    }
    if let Some(ref text) = criteria.has_not_text {
        let texts = node.text(15);
        if texts.contains(text.as_str()) {
            return false;
        }
    }
    if let Some(ref sub_locator) = criteria.has {
        // Re-root the sub-locator at this node and check if it has matches
        let sub = Locator {
            root: node.0.clone(),
            steps: sub_locator
                .steps
                .iter()
                .map(|s| clone_step(s))
                .collect(),
        };
        if sub.resolve_all().is_empty() {
            return false;
        }
    }
    if let Some(ref sub_locator) = criteria.has_not {
        let sub = Locator {
            root: node.0.clone(),
            steps: sub_locator
                .steps
                .iter()
                .map(|s| clone_step(s))
                .collect(),
        };
        if !sub.resolve_all().is_empty() {
            return false;
        }
    }
    if let Some(pred) = criteria.predicate {
        if !pred(node) {
            return false;
        }
    }
    true
}

/// Clone a LocatorStep (needed because AXQuery doesn't derive Clone for predicate fn ptrs,
/// but we've already derived Clone on AXQuery).
fn clone_step(step: &LocatorStep) -> LocatorStep {
    match step {
        LocatorStep::Role(r) => LocatorStep::Role(r.clone()),
        LocatorStep::RoleWithName(r, n) => LocatorStep::RoleWithName(r.clone(), n.clone()),
        LocatorStep::Text(t) => LocatorStep::Text(t.clone()),
        LocatorStep::Title(t) => LocatorStep::Title(t.clone()),
        LocatorStep::Description(d) => LocatorStep::Description(d.clone()),
        LocatorStep::DomId(id) => LocatorStep::DomId(id.clone()),
        LocatorStep::DomClass(c) => LocatorStep::DomClass(c.clone()),
        LocatorStep::Query(q) => LocatorStep::Query(q.clone()),
        LocatorStep::Filter(criteria) => {
            LocatorStep::Filter(FilterCriteria {
                has_text: criteria.has_text.clone(),
                has_not_text: criteria.has_not_text.clone(),
                has: criteria.has.as_ref().map(|loc| Box::new(Locator {
                    root: loc.root.clone(),
                    steps: loc.steps.iter().map(clone_step).collect(),
                })),
                has_not: criteria.has_not.as_ref().map(|loc| Box::new(Locator {
                    root: loc.root.clone(),
                    steps: loc.steps.iter().map(clone_step).collect(),
                })),
                predicate: criteria.predicate,
            })
        }
        LocatorStep::Nth(n) => LocatorStep::Nth(*n),
        LocatorStep::First => LocatorStep::First,
        LocatorStep::Last => LocatorStep::Last,
    }
}

// ---------------------------------------------------------------------------
// Debug
// ---------------------------------------------------------------------------

impl std::fmt::Debug for Locator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Locator")
            .field("steps", &self.steps.len())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use objc2_application_services::AXUIElement;
    use objc2_core_foundation::CFRetained;

    /// Helper: create a dummy AXUIElement (system-wide) for constructing Locators.
    fn dummy_element() -> CFRetained<AXUIElement> {
        unsafe { AXUIElement::new_system_wide() }
    }

    #[test]
    fn clone_step_preserves_filter_has_text() {
        let mut criteria = FilterCriteria::new();
        criteria.has_text("hello");

        let step = LocatorStep::Filter(criteria);
        let cloned = clone_step(&step);

        if let LocatorStep::Filter(c) = cloned {
            assert_eq!(c.has_text.as_deref(), Some("hello"));
        } else {
            panic!("expected Filter step");
        }
    }

    #[test]
    fn clone_step_preserves_filter_has_not_text() {
        let mut criteria = FilterCriteria::new();
        criteria.has_not_text("world");

        let step = LocatorStep::Filter(criteria);
        let cloned = clone_step(&step);

        if let LocatorStep::Filter(c) = cloned {
            assert_eq!(c.has_not_text.as_deref(), Some("world"));
        } else {
            panic!("expected Filter step");
        }
    }

    #[test]
    fn clone_step_preserves_nested_has_locator() {
        let inner_locator = Locator {
            root: dummy_element(),
            steps: vec![LocatorStep::Role("AXButton".to_string())],
        };
        let mut criteria = FilterCriteria::new();
        criteria.has(inner_locator);

        let step = LocatorStep::Filter(criteria);
        let cloned = clone_step(&step);

        if let LocatorStep::Filter(c) = cloned {
            assert!(c.has.is_some());
            let inner = c.has.unwrap();
            assert_eq!(inner.steps.len(), 1);
            if let LocatorStep::Role(r) = &inner.steps[0] {
                assert_eq!(r, "AXButton");
            } else {
                panic!("expected Role step inside has locator");
            }
        } else {
            panic!("expected Filter step");
        }
    }

    #[test]
    fn clone_step_preserves_nested_has_not_locator() {
        let inner_locator = Locator {
            root: dummy_element(),
            steps: vec![LocatorStep::Text("foo".to_string())],
        };
        let mut criteria = FilterCriteria::new();
        criteria.has_not(inner_locator);

        let step = LocatorStep::Filter(criteria);
        let cloned = clone_step(&step);

        if let LocatorStep::Filter(c) = cloned {
            assert!(c.has_not.is_some());
        } else {
            panic!("expected Filter step");
        }
    }

    #[test]
    fn clone_step_preserves_predicate() {
        fn my_pred(_: &AXNode) -> bool { true }

        let mut criteria = FilterCriteria::new();
        criteria.predicate(my_pred);

        let step = LocatorStep::Filter(criteria);
        let cloned = clone_step(&step);

        if let LocatorStep::Filter(c) = cloned {
            assert!(c.predicate.is_some());
        } else {
            panic!("expected Filter step");
        }
    }

    #[test]
    fn clone_step_simple_variants() {
        assert!(matches!(clone_step(&LocatorStep::Nth(5)), LocatorStep::Nth(5)));
        assert!(matches!(clone_step(&LocatorStep::First), LocatorStep::First));
        assert!(matches!(clone_step(&LocatorStep::Last), LocatorStep::Last));

        if let LocatorStep::Role(r) = clone_step(&LocatorStep::Role("AXButton".into())) {
            assert_eq!(r, "AXButton");
        } else {
            panic!("expected Role");
        }

        if let LocatorStep::Text(t) = clone_step(&LocatorStep::Text("hello".into())) {
            assert_eq!(t, "hello");
        } else {
            panic!("expected Text");
        }
    }

    #[test]
    fn clone_step_empty_filter() {
        let criteria = FilterCriteria::new();
        let step = LocatorStep::Filter(criteria);
        let cloned = clone_step(&step);

        if let LocatorStep::Filter(c) = cloned {
            assert!(c.has_text.is_none());
            assert!(c.has_not_text.is_none());
            assert!(c.has.is_none());
            assert!(c.has_not.is_none());
            assert!(c.predicate.is_none());
        } else {
            panic!("expected Filter step");
        }
    }
}
