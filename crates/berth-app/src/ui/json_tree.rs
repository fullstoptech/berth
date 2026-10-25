//! JsonTree: a searchable, collapsible view of one JSON value (the masked
//! inspect on the Inspect tab). The model (`visible_rows`) is pure; the
//! component draws rows and reports toggles. It shows what it is given:
//! the caller passes the masked inspect, never the raw one.

use std::collections::HashSet;
use std::sync::Arc;

use dioxus::prelude::*;
use serde_json::Value;

use super::icon::{Icon, IconSize};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LeafKind {
    Text,
    Number,
    Bool,
    Null,
}

impl LeafKind {
    fn class(self) -> &'static str {
        match self {
            Self::Text => "jt-text",
            Self::Number => "jt-number",
            Self::Bool => "jt-bool",
            Self::Null => "jt-null",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RowKind {
    /// An object or array: `{3}` or `[2]`, open or closed.
    Branch {
        summary: String,
        open: bool,
    },
    Leaf {
        text: String,
        kind: LeafKind,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeRow {
    /// A JSON pointer ("" is the root); also the toggle key.
    pub path: String,
    pub depth: usize,
    pub key: Option<String>,
    pub kind: RowKind,
    /// The row itself matches the query (its key or its value).
    pub matched: bool,
}

/// The paths that start open: the root.
#[must_use]
pub fn initially_open() -> HashSet<String> {
    HashSet::from([String::new()])
}

fn child_path(parent: &str, key: &str) -> String {
    format!("{parent}/{}", key.replace('~', "~0").replace('/', "~1"))
}

fn leaf_of(value: &Value) -> Option<(String, LeafKind)> {
    match value {
        Value::String(s) => Some((format!("\"{s}\""), LeafKind::Text)),
        Value::Number(n) => Some((n.to_string(), LeafKind::Number)),
        Value::Bool(b) => Some((b.to_string(), LeafKind::Bool)),
        Value::Null => Some(("null".to_string(), LeafKind::Null)),
        Value::Array(_) | Value::Object(_) => None,
    }
}

fn children(value: &Value) -> Vec<(String, &Value)> {
    match value {
        Value::Object(map) => map.iter().map(|(k, v)| (k.clone(), v)).collect(),
        Value::Array(items) => items
            .iter()
            .enumerate()
            .map(|(i, v)| (i.to_string(), v))
            .collect(),
        _ => Vec::new(),
    }
}

fn summary(value: &Value) -> String {
    match value {
        Value::Object(map) => format!("{{{}}}", map.len()),
        Value::Array(items) => format!("[{}]", items.len()),
        _ => String::new(),
    }
}

fn self_matches(key: Option<&str>, value: &Value, needle: &str) -> bool {
    key.is_some_and(|k| k.to_lowercase().contains(needle))
        || leaf_of(value).is_some_and(|(text, _)| text.to_lowercase().contains(needle))
}

fn subtree_matches(key: Option<&str>, value: &Value, needle: &str) -> bool {
    self_matches(key, value, needle)
        || children(value)
            .iter()
            .any(|(k, v)| subtree_matches(Some(k), v, needle))
}

fn walk(
    out: &mut Vec<TreeRow>,
    path: &str,
    depth: usize,
    key: Option<&str>,
    value: &Value,
    needle: &str,
    open_set: &HashSet<String>,
) {
    let searching = !needle.is_empty();
    if searching && !subtree_matches(key, value, needle) {
        return;
    }
    let matched = searching && self_matches(key, value, needle);
    if let Some((text, kind)) = leaf_of(value) {
        out.push(TreeRow {
            path: path.to_string(),
            depth,
            key: key.map(ToString::to_string),
            kind: RowKind::Leaf { text, kind },
            matched,
        });
        return;
    }
    // A search opens every branch that leads to a match.
    let open = searching || open_set.contains(path);
    out.push(TreeRow {
        path: path.to_string(),
        depth,
        key: key.map(ToString::to_string),
        kind: RowKind::Branch {
            summary: summary(value),
            open,
        },
        matched,
    });
    if open {
        for (k, v) in children(value) {
            walk(
                out,
                &child_path(path, &k),
                depth + 1,
                Some(&k),
                v,
                needle,
                open_set,
            );
        }
    }
}

/// The rows to draw, top to bottom. With a query only matching rows and the
/// branches above them show, all open; without one, `open_set` decides.
#[must_use]
pub fn visible_rows(value: &Value, query: &str, open_set: &HashSet<String>) -> Vec<TreeRow> {
    let needle = query.trim().to_lowercase();
    let mut out = Vec::new();
    walk(&mut out, "", 0, None, value, &needle, open_set);
    out
}

/// How many rows match `query` (key or value), anywhere in the tree.
#[must_use]
pub fn match_count(value: &Value, query: &str) -> usize {
    let needle = query.trim().to_lowercase();
    if needle.is_empty() {
        return 0;
    }
    visible_rows(value, query, &HashSet::new())
        .iter()
        .filter(|row| row.matched)
        .count()
}

/// How many rows the tree has with every branch open.
#[must_use]
pub fn node_count(value: &Value) -> usize {
    1 + children(value)
        .iter()
        .map(|(_, v)| node_count(v))
        .sum::<usize>()
}

#[component]
pub fn JsonTree(
    value: Arc<Value>,
    #[props(default)] query: String,
    open: HashSet<String>,
    ontoggle: EventHandler<String>,
) -> Element {
    let rows = visible_rows(&value, &query, &open);
    if rows.is_empty() {
        return rsx! { p { class: "jt-empty", "No match for \"{query}\"" } };
    }
    rsx! {
        div { class: "jt", role: "tree",
            for row in rows {
                {
                    let indent = format!("padding-left: calc({} * var(--space-12));", row.depth);
                    let path = row.path.clone();
                    let class = if row.matched { "jt-row jt-match" } else { "jt-row" };
                    rsx! {
                        div { key: "{row.path}", class, style: "{indent}", role: "treeitem",
                            match &row.kind {
                                RowKind::Branch { summary, open } => {
                                    let chevron = if *open { "chevron-down" } else { "chevron-right" };
                                    rsx! {
                                    button {
                                        class: "jt-toggle",
                                        r#type: "button",
                                        "aria-expanded": open.to_string(),
                                        onclick: move |_| ontoggle.call(path.clone()),
                                        Icon { name: chevron, size: IconSize::Sm }
                                        span { class: "jt-key", "{row.key.clone().unwrap_or_default()}" }
                                        span { class: "jt-summary", "{summary}" }
                                    }
                                    }
                                },
                                RowKind::Leaf { text, kind } => rsx! {
                                    span { class: "jt-spacer" }
                                    if let Some(key) = &row.key {
                                        span { class: "jt-key", "{key}" }
                                    }
                                    span { class: kind.class(), "{text}" }
                                },
                            }
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Value {
        serde_json::json!({
            "Config": { "Env": ["PATH=••••", "NODE_ENV=••••"], "User": "" },
            "State": { "Running": true, "ExitCode": 1 },
            "Name": "/shop-api-worker-1"
        })
    }

    #[component]
    fn Host(value: Arc<Value>, query: String, open: HashSet<String>) -> Element {
        rsx! { JsonTree { value, query, open, ontoggle: |_| {} } }
    }

    fn render(value: Value, query: &str, open: HashSet<String>) -> String {
        let props = HostProps {
            value: Arc::new(value),
            query: query.to_string(),
            open,
        };
        let mut dom = VirtualDom::new_with_props(Host, props);
        let _ = dom.rebuild_to_vec();
        dioxus_ssr::render(&dom)
    }

    fn keys(rows: &[TreeRow]) -> Vec<String> {
        rows.iter().filter_map(|r| r.key.clone()).collect()
    }

    #[test]
    fn json_tree_starts_with_only_the_root_open() {
        let rows = visible_rows(&sample(), "", &initially_open());
        assert_eq!(keys(&rows), ["Config", "State", "Name"]);
        assert!(matches!(rows[1].kind, RowKind::Branch { open: false, .. }));
    }

    #[test]
    fn json_tree_toggle_set_opens_a_branch_and_its_children_show() {
        let mut open = initially_open();
        open.insert("/Config".to_string());
        let rows = visible_rows(&sample(), "", &open);
        assert_eq!(keys(&rows), ["Config", "Env", "User", "State", "Name"]);
        assert_eq!(rows[2].depth, 2);
    }

    #[test]
    fn json_tree_search_keeps_matches_and_the_branches_above_them() {
        let rows = visible_rows(&sample(), "exitcode", &initially_open());
        assert_eq!(keys(&rows), ["State", "ExitCode"]);
        let row = |key: &str| {
            rows.iter()
                .find(|r| r.key.as_deref() == Some(key))
                .expect(key)
        };
        assert!(row("ExitCode").matched && !row("State").matched);
    }

    #[test]
    fn json_tree_search_matches_values_too_and_is_case_insensitive() {
        let rows = visible_rows(&sample(), "SHOP-API", &initially_open());
        assert_eq!(keys(&rows), ["Name"]);
    }

    #[test]
    fn json_tree_search_with_no_match_is_empty_and_the_component_says_so() {
        assert!(visible_rows(&sample(), "zzz", &initially_open()).is_empty());
        let html = render(sample(), "zzz", initially_open());
        assert!(html.contains("No match for"), "{html}");
    }

    #[test]
    fn json_tree_pointer_escapes_slashes_in_keys() {
        let value = serde_json::json!({ "a/b": { "c": 1 } });
        let mut open = initially_open();
        open.insert("/a~1b".to_string());
        let rows = visible_rows(&value, "", &open);
        assert_eq!(keys(&rows), ["a/b", "c"]);
    }

    #[test]
    fn json_tree_renders_rows_with_summaries_and_typed_leaves() {
        let mut open = initially_open();
        open.insert("/State".to_string());
        let html = render(sample(), "", open);
        assert!(html.contains("{2}"), "{html}");
        assert!(html.contains("jt-bool"), "{html}");
        assert!(html.contains("jt-number"), "{html}");
        assert!(html.contains("aria-expanded=\"true\""), "{html}");
    }
}
