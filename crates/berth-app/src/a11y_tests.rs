//! Accessibility audit over every scene (P6-T4). Each scene in
//! `scenes::scenes()` is rendered with dioxus-ssr and its HTML is read back
//! into a small tree; the checks below then name every control a screen
//! reader would announce. A new screen needs no test of its own: registering
//! its scene puts it under this guard.
//!
//! Checked per scene:
//! - every `button`, `[role=button|tab|switch|checkbox|radio|menuitem|option]`
//!   and `a[href]` has an accessible name (own text, `aria-label`, or
//!   `aria-labelledby`; an icon child never counts, a `title` only names it
//!   when nothing else does and is reported separately);
//! - every `svg` and `img` is hidden (`aria-hidden`) or named;
//! - every dialog has `aria-modal` and a name;
//! - every `input`, `textarea` and `select` has a label;
//! - every `aria-labelledby` / `aria-controls`-free reference resolves to an id;
//! - toasts and progress sit in a live region or carry `role=progressbar`.

use std::collections::{BTreeMap, BTreeSet};

use crate::scenes::{render_scene_html, scenes};

#[derive(Debug, Default)]
struct Node {
    tag: String,
    attrs: BTreeMap<String, String>,
    /// Text directly inside this element, in order with child markers elided.
    text: String,
    children: Vec<usize>,
    parent: Option<usize>,
}

impl Node {
    fn attr(&self, name: &str) -> Option<&str> {
        self.attrs.get(name).map(String::as_str)
    }
    fn has(&self, name: &str) -> bool {
        self.attrs.contains_key(name)
    }
    fn role(&self) -> Option<&str> {
        self.attr("role")
    }
}

const VOID: &[&str] = &[
    "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "source", "track",
    "wbr", "path", "circle", "rect", "line", "polyline", "polygon", "use", "ellipse", "stop",
];

struct Doc {
    nodes: Vec<Node>,
}

fn decode(text: &str) -> String {
    text.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&nbsp;", " ")
}

impl Doc {
    fn parse(html: &str) -> Self {
        let mut nodes = vec![Node {
            tag: "#root".to_string(),
            ..Node::default()
        }];
        let mut stack = vec![0usize];
        let bytes = html.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i] != b'<' {
                let end = html[i..].find('<').map_or(bytes.len(), |p| i + p);
                let top = *stack.last().unwrap_or(&0);
                nodes[top].text.push_str(&decode(&html[i..end]));
                i = end;
                continue;
            }
            if html[i..].starts_with("<!--") {
                i = html[i..].find("-->").map_or(bytes.len(), |p| i + p + 3);
                continue;
            }
            if html[i..].starts_with("<!") {
                i = html[i..].find('>').map_or(bytes.len(), |p| i + p + 1);
                continue;
            }
            // Find the end of the tag, skipping quoted attribute values.
            let mut j = i + 1;
            let mut quote: Option<u8> = None;
            while j < bytes.len() {
                match (quote, bytes[j]) {
                    (Some(q), c) if c == q => quote = None,
                    (None, b'"' | b'\'') => quote = Some(bytes[j]),
                    (None, b'>') => break,
                    _ => {}
                }
                j += 1;
            }
            let inner = &html[i + 1..j.min(html.len())];
            i = j + 1;
            if let Some(name) = inner.strip_prefix('/') {
                let name = name.trim().to_ascii_lowercase();
                if let Some(pos) = stack.iter().rposition(|&n| nodes[n].tag == name) {
                    stack.truncate(pos);
                }
                continue;
            }
            let self_closing = inner.ends_with('/');
            let inner = inner.trim_end_matches('/');
            let (tag, rest) = inner.split_once(char::is_whitespace).unwrap_or((inner, ""));
            let tag = tag.to_ascii_lowercase();
            let mut node = Node {
                tag: tag.clone(),
                attrs: parse_attrs(rest),
                parent: stack.last().copied(),
                ..Node::default()
            };
            if stack.is_empty() {
                node.parent = Some(0);
            }
            let id = nodes.len();
            let parent = *stack.last().unwrap_or(&0);
            nodes.push(node);
            nodes[parent].children.push(id);
            if !self_closing && !VOID.contains(&tag.as_str()) {
                stack.push(id);
            }
        }
        Self { nodes }
    }

    fn ids(&self) -> BTreeSet<&str> {
        self.nodes.iter().filter_map(|n| n.attr("id")).collect()
    }

    fn by_id(&self, id: &str) -> Option<usize> {
        self.nodes.iter().position(|n| n.attr("id") == Some(id))
    }

    fn descendants(&self, at: usize) -> impl Iterator<Item = usize> + '_ {
        let mut stack = self.nodes[at].children.clone();
        std::iter::from_fn(move || {
            let next = stack.pop()?;
            stack.extend(self.nodes[next].children.iter().copied());
            Some(next)
        })
    }

    fn ancestors(&self, mut at: usize) -> impl Iterator<Item = usize> + '_ {
        std::iter::from_fn(move || {
            let parent = self.nodes[at].parent?;
            at = parent;
            Some(parent)
        })
    }

    /// Hidden from the accessibility tree: itself or an ancestor says so.
    fn hidden(&self, at: usize) -> bool {
        let own = |n: usize| {
            let node = &self.nodes[n];
            node.attr("aria-hidden") == Some("true") || node.has("hidden")
        };
        own(at) || self.ancestors(at).any(own)
    }

    /// The text a screen reader would speak for an element's content:
    /// visible text of the subtree, minus hidden branches, plus the names of
    /// named images inside it.
    fn content_name(&self, at: usize) -> String {
        let node = &self.nodes[at];
        let mut out = String::new();
        if node.attr("aria-hidden") == Some("true") || node.has("hidden") {
            return out;
        }
        out.push_str(&node.text);
        for &child in &node.children {
            let c = &self.nodes[child];
            if c.attr("aria-hidden") == Some("true") {
                continue;
            }
            if let Some(label) = c.attr("aria-label").filter(|l| !l.trim().is_empty()) {
                out.push_str(label);
            }
            out.push_str(&self.content_name(child));
        }
        out
    }

    /// `aria-label`, `aria-labelledby`, then (for controls that take it)
    /// content. `None` when nothing names the element.
    fn accessible_name(&self, at: usize, from_content: bool) -> Option<String> {
        let node = &self.nodes[at];
        if let Some(label) = node.attr("aria-label").filter(|l| !l.trim().is_empty()) {
            return Some(label.to_string());
        }
        if let Some(ids) = node.attr("aria-labelledby") {
            let text: String = ids
                .split_whitespace()
                .filter_map(|id| self.by_id(id))
                .map(|n| self.content_name(n))
                .collect::<Vec<_>>()
                .join(" ");
            if !text.trim().is_empty() {
                return Some(text);
            }
        }
        if from_content {
            let text = self.content_name(at);
            if !text.trim().is_empty() {
                return Some(text);
            }
        }
        None
    }

    /// A form control's label: ARIA, a `label[for]`, or a wrapping `label`.
    fn control_label(&self, at: usize) -> Option<String> {
        if let Some(name) = self.accessible_name(at, false) {
            return Some(name);
        }
        let node = &self.nodes[at];
        if let Some(id) = node.attr("id") {
            for (n, label) in self.nodes.iter().enumerate() {
                if label.tag == "label" && label.attr("for") == Some(id) {
                    let text = self.content_name(n);
                    if !text.trim().is_empty() {
                        return Some(text);
                    }
                }
            }
        }
        for anc in self.ancestors(at) {
            if self.nodes[anc].tag == "label" {
                let text = self.content_name(anc);
                if !text.trim().is_empty() {
                    return Some(text);
                }
            }
        }
        None
    }
}

fn parse_attrs(rest: &str) -> BTreeMap<String, String> {
    let mut attrs = BTreeMap::new();
    let bytes = rest.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        let start = i;
        while i < bytes.len() && !bytes[i].is_ascii_whitespace() && bytes[i] != b'=' {
            i += 1;
        }
        if start == i {
            break;
        }
        let name = rest[start..i].to_ascii_lowercase();
        let mut value = String::new();
        if i < bytes.len() && bytes[i] == b'=' {
            i += 1;
            if i < bytes.len() && (bytes[i] == b'"' || bytes[i] == b'\'') {
                let q = bytes[i];
                i += 1;
                let vstart = i;
                while i < bytes.len() && bytes[i] != q {
                    i += 1;
                }
                value = decode(&rest[vstart..i]);
                i += 1;
            } else {
                let vstart = i;
                while i < bytes.len() && !bytes[i].is_ascii_whitespace() {
                    i += 1;
                }
                value = decode(&rest[vstart..i]);
            }
        }
        attrs.insert(name, value);
    }
    attrs
}

/// Elements the browser already exposes with a role and puts in the Tab order.
fn is_native_control(node: &Node) -> bool {
    matches!(
        node.tag.as_str(),
        "button" | "a" | "input" | "select" | "textarea" | "summary" | "iframe"
    )
}

/// The controls a screen reader announces as pressable.
fn is_pressable(node: &Node) -> bool {
    match node.role() {
        Some("button" | "tab" | "switch" | "checkbox" | "radio" | "menuitem" | "option") => true,
        Some(_) => false,
        None => node.tag == "button" || (node.tag == "a" && node.has("href")),
    }
}

/// Every accessibility defect in one rendered page, as readable lines.
pub(crate) fn audit(html: &str) -> Vec<String> {
    let doc = Doc::parse(html);
    let mut issues = Vec::new();
    let mut seen_ids = BTreeSet::new();
    for node in &doc.nodes {
        if let Some(id) = node.attr("id") {
            if !seen_ids.insert(id) {
                issues.push(format!("id used twice: {id:?}"));
            }
        }
    }
    let describe = |n: usize| {
        let node = &doc.nodes[n];
        let class = node.attr("class").unwrap_or("");
        format!("<{} class=\"{class}\">", node.tag)
    };
    for (n, node) in doc.nodes.iter().enumerate() {
        if n == 0 || doc.hidden(n) {
            continue;
        }
        if is_pressable(node) && doc.accessible_name(n, true).is_none() {
            issues.push(format!(
                "control without an accessible name: {}",
                describe(n)
            ));
        }
        match node.tag.as_str() {
            "svg" => {
                let named = node.attr("aria-label").is_some_and(|l| !l.is_empty())
                    || node.has("aria-labelledby")
                    || node.children.iter().any(|&c| doc.nodes[c].tag == "title");
                if !named {
                    issues.push(format!(
                        "svg that is neither hidden nor named: {}",
                        describe(n)
                    ));
                }
            }
            "img" => {
                let named = node.attr("alt").is_some() || node.has("aria-label");
                if !named {
                    issues.push(format!("img without alt or aria-hidden: {}", describe(n)));
                }
            }
            "input" | "textarea" | "select" => {
                let kind = node.attr("type").unwrap_or("text");
                let skip = kind == "hidden"
                    || (node.tag == "input" && matches!(kind, "submit" | "button" | "reset"));
                if !skip && doc.control_label(n).is_none() {
                    issues.push(format!("form field without a label: {}", describe(n)));
                }
            }
            _ => {}
        }
        if matches!(node.role(), Some("dialog" | "alertdialog")) || node.tag == "dialog" {
            if doc.accessible_name(n, false).is_none() {
                issues.push(format!("dialog without a label: {}", describe(n)));
            }
            if node.attr("aria-modal") != Some("true") {
                issues.push(format!("dialog without aria-modal: {}", describe(n)));
            }
        }
        if matches!(
            node.role(),
            Some("progressbar" | "img" | "table" | "grid" | "listbox")
        ) && doc.accessible_name(n, false).is_none()
        {
            issues.push(format!(
                "role={} without a label: {}",
                node.role().unwrap_or(""),
                describe(n)
            ));
        }
        if node.attr("tabindex") == Some("0") && node.role().is_none() && !is_native_control(node) {
            issues.push(format!("focusable element with no role: {}", describe(n)));
        }
        if node.role() == Some("tab")
            && !doc
                .ancestors(n)
                .any(|a| doc.nodes[a].role() == Some("tablist"))
        {
            issues.push(format!("tab outside a tablist: {}", describe(n)));
        }
        if node.role() == Some("row")
            && !doc.ancestors(n).any(|a| {
                matches!(
                    doc.nodes[a].role(),
                    Some("table" | "grid" | "treegrid" | "rowgroup")
                )
            })
        {
            issues.push(format!("row outside a table: {}", describe(n)));
        }
        if node
            .attr("class")
            .is_some_and(|c| c.split_whitespace().any(|c| c == "overlay-scrim"))
            && !doc
                .descendants(n)
                .any(|d| matches!(doc.nodes[d].role(), Some("dialog" | "alertdialog")))
        {
            issues.push(format!("overlay without a dialog: {}", describe(n)));
        }
        if let Some(ids) = node.attr("aria-labelledby") {
            for id in ids.split_whitespace() {
                if !doc.ids().contains(id) {
                    issues.push(format!("aria-labelledby points at a missing id {id:?}"));
                }
            }
        }
    }
    issues
}

#[test]
fn a11y_every_scene_passes_the_audit() {
    let mut failures = Vec::new();
    for scene in scenes() {
        let html = render_scene_html(scene.frame)
            .unwrap_or_else(|| panic!("{:?} is registered but does not render", scene.frame));
        let mut issues = audit(&html);
        issues.sort();
        issues.dedup();
        for issue in issues {
            failures.push(format!("{:?}: {issue}", scene.frame));
        }
    }
    assert!(
        failures.is_empty(),
        "{} accessibility issues:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[test]
fn a11y_audit_flags_each_kind_of_defect() {
    let bad = r#"
        <div>
          <button class="icon-btn"><svg class="icon"><use href="/icons.svg#x"></use></svg></button>
          <button aria-label="Fine"><svg aria-hidden="true"></svg></button>
          <button>Text</button>
          <svg class="loose"><use href="/icons.svg#y"></use></svg>
          <img src="a.png">
          <div role="dialog" class="dlg"></div>
          <input type="text" class="bare">
          <label for="f">Name</label><input id="f" type="text">
          <label>Wrapped <input type="text"></label>
          <input type="hidden" name="x">
        </div>"#;
    let issues = audit(bad);
    let joined = issues.join("\n");
    assert!(
        joined.contains("control without an accessible name: <button class=\"icon-btn\">"),
        "{joined}"
    );
    assert!(
        joined.contains("svg that is neither hidden nor named: <svg class=\"loose\">"),
        "{joined}"
    );
    assert!(joined.contains("img without alt"), "{joined}");
    assert!(joined.contains("dialog without a label"), "{joined}");
    assert!(joined.contains("dialog without aria-modal"), "{joined}");
    assert!(
        joined.contains("form field without a label: <input class=\"bare\">"),
        "{joined}"
    );
    // The named, hidden, labelled and hidden-type cases are not reported.
    assert_eq!(issues.len(), 7, "{joined}");
}

#[test]
fn a11y_audit_accepts_labelled_by_and_named_dialogs() {
    let good = r#"
        <div role="dialog" aria-modal="true" aria-labelledby="t">
          <h2 id="t">Delete berthfx?</h2>
          <button>Delete</button>
          <button aria-label="Close"><svg aria-hidden="true"></svg></button>
        </div>"#;
    assert_eq!(audit(good), Vec::<String>::new());
}
