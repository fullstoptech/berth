//! Rendering the context chips into prompt text (PLAN §6.9).
//!
//! `render` is deterministic: one `## ` header per item, in the order the
//! items were given, followed by the body `resolve` returns for that item
//! (already redacted by the resolver). `Logs` items carry their own lines,
//! so an empty resolution falls back to them; every other kind renders its
//! header alone when the resolver has nothing, which still names what the
//! user attached and lets the harness fetch the body through its tools.

use berth_core::ContextItem;

fn header(item: &ContextItem) -> String {
    match item {
        ContextItem::Logs { name, lines, .. } => {
            format!("## Logs: {name} ({} lines)", lines.len())
        }
        ContextItem::Problem(id) => format!("## Problem {}", id.0),
        ContextItem::ComposeFile(path) => format!("## Compose file: {}", path.display()),
        ContextItem::Dockerfile(path) => format!("## Dockerfile: {}", path.display()),
        ContextItem::Inspect(id) => format!("## Inspect: container {}", id.short()),
        ContextItem::FolderTree(path) => format!("## Folder: {}", path.display()),
        ContextItem::File(path) => format!("## File: {}", path.display()),
    }
}

#[must_use]
pub fn render(items: &[ContextItem], resolve: &dyn Fn(&ContextItem) -> String) -> String {
    let mut out = String::new();
    for item in items {
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(&header(item));
        let mut body = resolve(item);
        if body.is_empty() {
            if let ContextItem::Logs { lines, .. } = item {
                body = lines.join("\n");
            }
        }
        if !body.is_empty() {
            out.push('\n');
            out.push_str(body.trim_end_matches('\n'));
        }
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use berth_core::{ContainerId, ProblemId};
    use std::path::PathBuf;

    fn echo(item: &ContextItem) -> String {
        format!("body for {item:?}").chars().take(24).collect()
    }

    #[test]
    fn claude_context_renders_every_variant_with_its_header() {
        let items = vec![
            ContextItem::Logs {
                container: ContainerId("c1".into()),
                name: "shop-api-api-1".into(),
                lines: vec!["one".into(), "two".into()],
            },
            ContextItem::Problem(ProblemId("p-1".into())),
            ContextItem::ComposeFile(PathBuf::from("/work/compose.yaml")),
            ContextItem::Dockerfile(PathBuf::from("/work/Dockerfile")),
            ContextItem::Inspect(ContainerId("abcdef0123456789".into())),
            ContextItem::FolderTree(PathBuf::from("/work")),
        ];
        let text = render(&items, &echo);
        for want in [
            "## Logs: shop-api-api-1 (2 lines)",
            "## Problem p-1",
            "## Compose file: /work/compose.yaml",
            "## Dockerfile: /work/Dockerfile",
            "## Inspect: container abcdef012345",
            "## Folder: /work",
        ] {
            assert!(text.contains(want), "missing {want} in:\n{text}");
        }
        // Order is the chip order, not sorted.
        assert!(
            text.find("## Logs").expect("logs") < text.find("## Folder").expect("folder"),
            "{text}"
        );
    }

    #[test]
    fn claude_context_logs_fall_back_to_their_own_lines() {
        let items = vec![ContextItem::Logs {
            container: ContainerId("c1".into()),
            name: "web".into(),
            lines: vec!["first".into(), "second".into()],
        }];
        let text = render(&items, &|_| String::new());
        assert_eq!(text, "## Logs: web (2 lines)\nfirst\nsecond\n");
    }

    #[test]
    fn claude_context_empty_items_render_nothing() {
        assert_eq!(render(&[], &echo), "");
        let items = vec![ContextItem::Problem(ProblemId("p".into()))];
        assert_eq!(render(&items, &|_| String::new()), "## Problem p\n");
    }
}
