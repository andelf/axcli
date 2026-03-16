//! Accessibility tree formatting and display utilities.

use crate::accessibility::{self, AXNode};

const TEXT_ROLES: &[&str] = &["AXStaticText", "AXTextArea", "AXTextField"];

/// Sort class names: meaningful ones first, noisy ones (numeric, `-` or `_` prefixed) last.
pub fn sort_classes(classes: &[String]) -> Vec<&str> {
    let mut visible: Vec<&str> = classes
        .iter()
        .filter(|c| !c.starts_with('_'))
        .map(|s| s.as_str())
        .collect();
    visible.sort_by_key(|c| {
        if c.starts_with('-') || c.chars().next().map_or(true, |ch| ch.is_ascii_digit()) {
            1
        } else {
            0
        }
    });
    visible
}

/// Truncate a string to `max` chars (0 = no limit), replacing newlines with `\n`.
pub fn truncate(s: &str, max: usize) -> String {
    if max == 0 || s.chars().count() <= max {
        s.replace('\n', "\\n")
    } else {
        let truncated: String = s.chars().take(max).collect();
        format!("{}...", truncated.replace('\n', "\\n"))
    }
}

/// Configuration and state for tree printing.
pub struct TreePrinter {
    /// Max text/title/description display length. 0 = no truncation.
    pub max_text_len: usize,
    /// Simplify output: hide dom_id/dom_classes and prune empty subtrees.
    pub simplify: bool,
    /// Count of interactive elements seen during printing.
    interactive: usize,
}

impl TreePrinter {
    pub fn new() -> Self {
        Self {
            max_text_len: 80,
            simplify: false,
            interactive: 0,
        }
    }

    /// Number of interactive elements counted during the last print.
    pub fn interactive_count(&self) -> usize {
        self.interactive
    }

    /// Format a one-line summary of a node (no trailing newline).
    pub fn format_node_line(&self, node: &AXNode) -> String {
        let role = node.role().unwrap_or_default();
        let title = node.title().unwrap_or_default();
        let desc = node.description().unwrap_or_default();
        let value = node.value().unwrap_or_default();
        let is_text = TEXT_ROLES.iter().any(|r| role == *r);
        let max = self.max_text_len;

        let short_role = role.strip_prefix("AX").unwrap_or(&role).to_lowercase();

        if is_text && !value.is_empty() {
            return format!("text: \"{}\"", truncate(&value, max));
        }

        let mut s = short_role;
        if !self.simplify {
            let dom_id = accessibility::attr_string(&node.0, "AXDOMIdentifier").unwrap_or_default();
            let dom_classes = node.dom_classes();
            if !dom_id.is_empty() {
                s.push_str(&format!("#{dom_id}"));
            }
            for cls in sort_classes(&dom_classes) {
                s.push_str(&format!(".{cls}"));
            }
        }
        let title_max = if max == 0 { 0 } else { max.min(60) };
        if !title.is_empty() {
            s.push_str(&format!(" \"{}\"", truncate(&title, title_max)));
        } else if !desc.is_empty() {
            s.push_str(&format!(" \"{}\"", truncate(&desc, title_max)));
        }
        s
    }

    /// Print a matched node with up to 3 ancestor levels for context.
    pub fn print_with_ancestors(&mut self, node: &AXNode, max_depth: usize) {
        let mut ancestors = Vec::new();
        let mut cur = node.parent();
        for _ in 0..3 {
            match cur {
                Some(p) => {
                    ancestors.push(p);
                    cur = ancestors.last().unwrap().parent();
                }
                None => break,
            }
        }
        ancestors.reverse();

        for (i, anc) in ancestors.iter().enumerate() {
            let indent = "  ".repeat(i);
            println!("{indent}- {}:", self.format_node_line(anc));
        }

        let base_depth = ancestors.len();
        let indent = "  ".repeat(base_depth);
        let line = self.format_node_line(node);
        println!("{indent}- {line}  ← matched");
        for child in node.children() {
            self.print_tree_inner(&child, base_depth + 1, base_depth + max_depth, false);
        }
    }

    /// Print the accessibility tree starting from `node`.
    pub fn print_tree(&mut self, node: &AXNode, depth: usize, max_depth: usize) {
        self.print_tree_inner(node, depth, max_depth, depth == 0);
    }

    fn print_tree_inner(
        &mut self,
        node: &AXNode,
        depth: usize,
        max_depth: usize,
        is_root: bool,
    ) {
        if depth > max_depth {
            return;
        }

        if !is_root {
            if let Some((w, h)) = node.size() {
                if w == 0.0 && h == 0.0 {
                    return;
                }
            }
        }

        // In simplify mode, prune subtrees with no visible content
        if self.simplify && !is_root && !has_content(node, max_depth.saturating_sub(depth)) {
            return;
        }

        let role = node.role().unwrap_or_default();
        let title = node.title().unwrap_or_default();
        let desc = node.description().unwrap_or_default();
        let value = node.value().unwrap_or_default();
        let actions = node.actions();
        let is_text = TEXT_ROLES.iter().any(|r| role == *r);
        let max = self.max_text_len;

        let (dom_id, dom_classes) = if self.simplify {
            (String::new(), Vec::new())
        } else {
            (
                accessibility::attr_string(&node.0, "AXDOMIdentifier").unwrap_or_default(),
                node.dom_classes(),
            )
        };

        let has_visible_class = dom_classes.iter().any(|c| !c.starts_with('_'));
        let has_identity = !title.is_empty()
            || !desc.is_empty()
            || !value.is_empty()
            || !dom_id.is_empty()
            || has_visible_class;

        let has_meaningful_actions = actions
            .iter()
            .any(|a| a != "AXScrollToVisible" && a != "AXShowMenu");

        let is_text_node = is_text && !value.is_empty();
        let is_interactive = !is_text_node && has_identity && has_meaningful_actions;
        let is_structural = !is_text_node && !is_interactive && has_identity;
        let keep = is_root || is_text_node || is_interactive || is_structural;

        if keep {
            let short_role = role
                .strip_prefix("AX")
                .unwrap_or(&role)
                .to_lowercase();

            let indent = "  ".repeat(depth);
            let mut line = String::new();

            if is_text_node {
                let display_value = truncate(&value, max);
                line = format!("{indent}- text: \"{display_value}\"");
            } else {
                line.push_str(&format!("{indent}- {short_role}"));

                if !dom_id.is_empty() {
                    line.push_str(&format!("#{dom_id}"));
                }
                for cls in sort_classes(&dom_classes) {
                    line.push_str(&format!(".{cls}"));
                }

                let title_max = if max == 0 { 0 } else { max.min(60) };
                if !title.is_empty() {
                    line.push_str(&format!(" \"{}\"", truncate(&title, title_max)));
                } else if !desc.is_empty() {
                    line.push_str(&format!(" \"{}\"", truncate(&desc, title_max)));
                }

                if is_interactive {
                    self.interactive += 1;
                } else {
                    line.push(':');
                }
            }

            println!("{line}");

            if depth == max_depth {
                let remaining = count_descendants(node, 5);
                if remaining > 0 {
                    let child_indent = "  ".repeat(depth + 1);
                    let suffix = if remaining >= 100 { "+" } else { "" };
                    println!("{child_indent}... {remaining}{suffix} more");
                }
            } else {
                for child in node.children() {
                    self.print_tree_inner(&child, depth + 1, max_depth, false);
                }
            }
        } else {
            for child in node.children() {
                self.print_tree_inner(&child, depth, max_depth, false);
            }
        }
    }
}

/// Check whether a node or its subtree has any visible content (title, desc, value, or text).
fn has_content(node: &AXNode, max_depth: usize) -> bool {
    if let Some((w, h)) = node.size() {
        if w == 0.0 && h == 0.0 {
            return false;
        }
    }

    let role = node.role().unwrap_or_default();
    let is_text = TEXT_ROLES.iter().any(|r| role == *r);
    if is_text && node.value().map_or(false, |v| !v.is_empty()) {
        return true;
    }
    if !node.title().unwrap_or_default().is_empty()
        || !node.description().unwrap_or_default().is_empty()
        || !node.value().unwrap_or_default().is_empty()
    {
        return true;
    }

    if max_depth == 0 {
        return false;
    }
    node.children().iter().any(|c| has_content(c, max_depth - 1))
}

/// Count descendants up to a shallow depth, capped at 100 for speed.
fn count_descendants(node: &AXNode, max_depth: usize) -> usize {
    if max_depth == 0 {
        return 0;
    }
    let mut total = 0;
    for child in node.children() {
        total += 1;
        if total >= 100 {
            return total;
        }
        total += count_descendants(&child, max_depth - 1);
        if total >= 100 {
            return total;
        }
    }
    total
}
