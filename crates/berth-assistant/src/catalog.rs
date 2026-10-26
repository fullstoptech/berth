//! The 25 tools Berth gives a harness over MCP (brief §6.12, PLAN §6.9,
//! ADR 0006): name, description, risk and a JSON schema for the arguments.
//!
//! Risk decides the gate in the app: Read runs at once, Change waits for one
//! click, Destructive waits for the item's name to be typed. Every Destructive
//! tool therefore takes a `name` argument: the exact name of the item it would
//! delete, which the approval card asks the user to type.

use berth_core::Risk;
use schemars::JsonSchema;
use serde_json::Value;

/// The prefix a harness puts in front of a tool of the `berth` MCP server.
const MCP_PREFIX: &str = "mcp__berth__";

pub struct ToolDef {
    /// Dotted name, as in the brief: `containers.logs`.
    pub name: &'static str,
    pub description: &'static str,
    pub risk: Risk,
    /// JSON Schema (an object schema) for the tool's arguments.
    pub input_schema: fn() -> Value,
}

impl ToolDef {
    /// The name on the wire, `containers_logs`. Claude API tool names forbid
    /// `.`, so Claude Code rewrites the dotted name; both forms must resolve.
    #[must_use]
    pub fn wire_name(&self) -> String {
        self.name.replace('.', "_")
    }
}

/// All tools, Read first, then Change, then Destructive.
#[must_use]
pub fn catalog() -> &'static [ToolDef] {
    &CATALOG
}

/// Looks a tool up by its dotted name (`containers.logs`), its wire name
/// (`containers_logs`) or the name a harness shows (`mcp__berth__containers_logs`).
#[must_use]
pub fn tool(name: &str) -> Option<&'static ToolDef> {
    let bare = name.strip_prefix(MCP_PREFIX).unwrap_or(name);
    CATALOG
        .iter()
        .find(|t| t.name == bare || t.wire_name() == bare)
}

fn schema_of<T: JsonSchema>() -> Value {
    serde_json::to_value(schemars::schema_for!(T))
        .unwrap_or_else(|_| serde_json::json!({ "type": "object" }))
}

// ---------------------------------------------------------------------------
// Argument shapes. They exist to produce the schemas; the app reads the JSON
// arguments itself. Doc comments become the property descriptions the model
// sees, so they are written for the model.
// ---------------------------------------------------------------------------
#[allow(dead_code)]
mod args {
    use schemars::JsonSchema;

    /// No arguments.
    #[derive(JsonSchema)]
    pub struct Empty {}

    #[derive(JsonSchema)]
    pub struct Engine {
        /// Engine id such as `colima:default`. Omit for the engine the project runs on.
        pub engine: Option<String>,
    }

    #[derive(JsonSchema)]
    pub struct ContainersList {
        /// Engine id such as `colima:default`. Omit for the engine the project runs on.
        pub engine: Option<String>,
        /// Only containers of this project (its folder name).
        pub project: Option<String>,
        /// Include stopped containers. Default true.
        pub all: Option<bool>,
    }

    #[derive(JsonSchema)]
    pub struct Container {
        /// Container name or id.
        pub container: String,
        /// Engine id such as `colima:default`. Omit for the engine the project runs on.
        pub engine: Option<String>,
    }

    #[derive(JsonSchema)]
    pub struct ContainerLogs {
        /// Container name or id.
        pub container: String,
        /// Engine id such as `colima:default`. Omit for the engine the project runs on.
        pub engine: Option<String>,
        /// Only lines since this time: a duration such as `10m` or an RFC 3339 timestamp.
        pub since: Option<String>,
        /// How many of the last lines to return. Default 200.
        pub tail: Option<u32>,
        /// Only lines that contain this text.
        pub grep: Option<String>,
    }

    #[derive(JsonSchema)]
    pub struct Project {
        /// Project name (its folder name) or id.
        pub project: String,
    }

    #[derive(JsonSchema)]
    pub struct ProblemsList {
        /// Only problems of this project (its folder name).
        pub project: Option<String>,
    }

    #[derive(JsonSchema)]
    pub struct FilesRead {
        /// Path relative to the project folder. Files outside the folder are refused.
        /// Values in `.env` files and in Compose `environment` are masked as `••••`.
        pub path: String,
        /// Project name (its folder name). Omit for the session's project.
        pub project: Option<String>,
    }

    #[derive(JsonSchema)]
    pub struct ActivityList {
        /// How many of the latest entries to return. Default 50.
        pub limit: Option<u32>,
        /// Only entries about this project (its folder name).
        pub project: Option<String>,
    }

    #[derive(JsonSchema)]
    pub struct ProposeEdit {
        /// Path relative to the project folder; the file may not exist yet.
        pub path: String,
        /// The complete new content of the file. In a `.env` file write `KEY=••••`
        /// to keep the value already on disk; give a real value only for a new key.
        pub content: String,
        /// One line saying why, shown above the change in Review changes.
        pub summary: Option<String>,
        /// Project name (its folder name). Omit for the session's project.
        pub project: Option<String>,
    }

    #[derive(JsonSchema)]
    pub struct ComposeRun {
        /// Project name (its folder name) or id.
        pub project: String,
        /// Only these services. Omit for the whole project.
        pub services: Option<Vec<String>>,
    }

    #[derive(JsonSchema)]
    pub struct ContainerTimeout {
        /// Container name or id.
        pub container: String,
        /// Engine id such as `colima:default`. Omit for the engine the project runs on.
        pub engine: Option<String>,
        /// Seconds to wait before the container is killed. Default 10.
        pub timeout_s: Option<u32>,
    }

    #[derive(JsonSchema)]
    pub struct PullImage {
        /// Image reference such as `postgres:17`.
        pub reference: String,
        /// Engine id such as `colima:default`. Omit for the engine the project runs on.
        pub engine: Option<String>,
        /// Platform such as `linux/amd64`. Omit for the engine's own.
        pub platform: Option<String>,
    }

    #[derive(JsonSchema)]
    pub struct BuildImage {
        /// Build context, relative to the project folder. Default `.`.
        pub context: Option<String>,
        /// Tag for the new image, such as `myapp:dev`.
        pub tag: String,
        /// Dockerfile path relative to the context. Default `Dockerfile`.
        pub dockerfile: Option<String>,
        /// Engine id such as `colima:default`. Omit for the engine the project runs on.
        pub engine: Option<String>,
        /// Project name (its folder name). Omit for the session's project.
        pub project: Option<String>,
    }

    #[derive(JsonSchema)]
    pub struct RemoveContainer {
        /// The container's exact name. The user must type it to confirm.
        pub name: String,
        /// Engine id such as `colima:default`. Omit for the engine the project runs on.
        pub engine: Option<String>,
        /// Remove a running container. Default false.
        pub force: Option<bool>,
        /// Also remove its anonymous volumes. Default false.
        pub volumes: Option<bool>,
    }

    #[derive(JsonSchema)]
    pub struct RemoveVolume {
        /// The volume's exact name. The user must type it to confirm.
        pub name: String,
        /// Engine id such as `colima:default`. Omit for the engine the project runs on.
        pub engine: Option<String>,
    }

    #[derive(JsonSchema)]
    pub struct RemoveImage {
        /// The image's reference (`postgres:17`) or id. The user must type it to confirm.
        pub name: String,
        /// Engine id such as `colima:default`. Omit for the engine the project runs on.
        pub engine: Option<String>,
        /// Remove even if a stopped container uses it. Default false.
        pub force: Option<bool>,
    }

    #[derive(JsonSchema, Clone, Copy)]
    #[serde(rename_all = "snake_case")]
    pub enum PruneKind {
        Container,
        Image,
        Volume,
        Network,
        BuildCache,
    }

    #[derive(JsonSchema)]
    pub struct Prune {
        /// The id of the engine to prune, such as `colima:default`. The user must type it to confirm.
        pub name: String,
        /// What to prune. Omit for stopped containers, dangling images, unused networks and build cache;
        /// volumes are pruned only when listed.
        pub kinds: Option<Vec<PruneKind>>,
    }
}

use args as a;

macro_rules! tool_def {
    ($name:literal, $risk:ident, $args:ty, $description:literal) => {
        ToolDef {
            name: $name,
            description: $description,
            risk: Risk::$risk,
            input_schema: schema_of::<$args>,
        }
    };
}

static CATALOG: [ToolDef; 25] = [
    // Read: run at once.
    tool_def!("engines.list", Read, a::Empty, "List the Docker engines Berth knows (Colima profiles, remote engines) with their state."),
    tool_def!("projects.list", Read, a::Empty, "List the projects Berth knows: folder, Compose file, state of each service."),
    tool_def!("containers.list", Read, a::ContainersList, "List containers with state, image, ports and project."),
    tool_def!("containers.inspect", Read, a::Container, "Inspect one container: image, command, mounts, ports, networks, health, restart policy. Environment values are masked as ••••."),
    tool_def!("containers.logs", Read, a::ContainerLogs, "Read a container's log lines, optionally since a time or filtered by text. Values the container holds in its environment are masked as ••••."),
    tool_def!("containers.stats", Read, a::Container, "Current CPU, memory, network and disk use of one container."),
    tool_def!("images.list", Read, a::Engine, "List images with tags, size and the containers that use them."),
    tool_def!("volumes.list", Read, a::Engine, "List volumes with size and the containers that use them."),
    tool_def!("networks.list", Read, a::Engine, "List networks with their containers."),
    tool_def!("compose.config", Read, a::Project, "The resolved Compose file of a project (docker compose config, without interpolation). Environment values are masked as ••••."),
    tool_def!("problems.list", Read, a::ProblemsList, "List the problems Berth found: port clashes, missing files, crash loops, with the fix it would offer."),
    tool_def!("files.read", Read, a::FilesRead, "Read a text file inside the project folder. Values in .env files and Compose environment are masked as ••••."),
    tool_def!("activity.list", Read, a::ActivityList, "List recent actions Berth, you or the user took, newest first."),
    // Change: one click.
    tool_def!("files.propose_edit", Change, a::ProposeEdit, "Propose a new content for a file in the project folder. Nothing is written: the user sees the change in Review changes and applies it there."),
    tool_def!("compose.up", Change, a::ComposeRun, "Start a project's services (docker compose up -d), building what is missing. The user approves first."),
    tool_def!("compose.restart", Change, a::ComposeRun, "Restart a project's services. The user approves first."),
    tool_def!("containers.restart", Change, a::ContainerTimeout, "Restart one container. The user approves first."),
    tool_def!("containers.start", Change, a::Container, "Start one stopped container. The user approves first."),
    tool_def!("containers.stop", Change, a::ContainerTimeout, "Stop one running container. The user approves first."),
    tool_def!("images.pull", Change, a::PullImage, "Pull an image from its registry. The user approves first."),
    tool_def!("images.build", Change, a::BuildImage, "Build an image from a Dockerfile in the project folder. The user approves first."),
    // Destructive: the user types the name.
    tool_def!("containers.remove", Destructive, a::RemoveContainer, "Remove one container. The user must type its name to confirm."),
    tool_def!("volumes.remove", Destructive, a::RemoveVolume, "Remove one volume and its data. The user must type its name to confirm."),
    tool_def!("images.remove", Destructive, a::RemoveImage, "Remove one image. The user must type its reference to confirm."),
    tool_def!("system.prune", Destructive, a::Prune, "Remove unused containers, images, networks, build cache and, if listed, volumes from one engine. The user sees the list and must type the engine id to confirm."),
];

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    fn by_name(name: &str) -> &'static ToolDef {
        tool(name).unwrap_or_else(|| panic!("{name} is in the catalog"))
    }

    fn names(risk: Risk) -> Vec<&'static str> {
        catalog()
            .iter()
            .filter(|t| t.risk == risk)
            .map(|t| t.name)
            .collect()
    }

    #[test]
    fn catalog_holds_the_25_tools_of_the_brief() {
        assert_eq!(catalog().len(), 25);
    }

    #[test]
    fn catalog_names_and_wire_names_are_unique() {
        let dotted: BTreeSet<_> = catalog().iter().map(|t| t.name).collect();
        let wire: BTreeSet<_> = catalog().iter().map(ToolDef::wire_name).collect();
        assert_eq!(dotted.len(), 25);
        assert_eq!(wire.len(), 25);
    }

    #[test]
    fn catalog_risk_table_matches_the_brief() {
        assert_eq!(
            names(Risk::Read),
            [
                "engines.list",
                "projects.list",
                "containers.list",
                "containers.inspect",
                "containers.logs",
                "containers.stats",
                "images.list",
                "volumes.list",
                "networks.list",
                "compose.config",
                "problems.list",
                "files.read",
                "activity.list",
            ]
        );
        assert_eq!(
            names(Risk::Change),
            [
                "files.propose_edit",
                "compose.up",
                "compose.restart",
                "containers.restart",
                "containers.start",
                "containers.stop",
                "images.pull",
                "images.build",
            ]
        );
        assert_eq!(
            names(Risk::Destructive),
            [
                "containers.remove",
                "volumes.remove",
                "images.remove",
                "system.prune"
            ]
        );
    }

    #[test]
    fn catalog_wire_names_have_no_dot_and_fit_the_claude_api_pattern() {
        for t in catalog() {
            let wire = t.wire_name();
            assert!(!wire.contains('.'), "{wire}");
            // Claude API: ^[a-zA-Z0-9_-]{1,64}$; the harness adds `mcp__berth__`.
            assert!(
                wire.chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-'),
                "{wire}"
            );
            assert!(MCP_PREFIX.len() + wire.len() <= 64, "{wire}");
        }
        assert_eq!(by_name("containers.logs").wire_name(), "containers_logs");
        assert_eq!(
            by_name("files.propose_edit").wire_name(),
            "files_propose_edit"
        );
    }

    #[test]
    fn catalog_every_name_round_trips_through_all_three_forms() {
        for t in catalog() {
            let wire = t.wire_name();
            for form in [
                t.name.to_string(),
                wire.clone(),
                format!("mcp__berth__{wire}"),
            ] {
                let found = tool(&form).unwrap_or_else(|| panic!("{form} resolves"));
                assert_eq!(found.name, t.name, "{form}");
            }
        }
    }

    #[test]
    fn catalog_lookup_refuses_unknown_and_foreign_names() {
        for name in [
            "",
            "containers",
            "containers.logs.extra",
            "Containers.Logs",
            "mcp__other__containers_logs",
            "mcp__berth__",
            "mcp__berth__mcp__berth__containers_logs",
            "shell.run",
        ] {
            assert!(tool(name).is_none(), "{name:?} must not resolve");
        }
    }

    #[test]
    fn catalog_every_destructive_tool_takes_a_required_name() {
        let destructive = names(Risk::Destructive);
        assert_eq!(destructive.len(), 4);
        for name in destructive {
            let schema = (by_name(name).input_schema)();
            assert!(
                schema["properties"]["name"].is_object(),
                "{name} has no `name` property"
            );
            assert!(
                schema["required"]
                    .as_array()
                    .is_some_and(|r| r.iter().any(|v| v == "name")),
                "{name} does not require `name`"
            );
        }
    }

    #[test]
    fn catalog_only_destructive_tools_require_a_name_to_confirm() {
        // Guards the reverse: a Read tool that wanted a typed name would never
        // be gated by one.
        for t in catalog().iter().filter(|t| t.risk != Risk::Destructive) {
            let schema = (t.input_schema)();
            assert!(
                schema["properties"]["name"].is_null(),
                "{} has a `name` property",
                t.name
            );
        }
    }

    /// A property must say what type it is, directly or through a `$ref`,
    /// `anyOf`/`oneOf` or an `enum`.
    fn describes_a_type(prop: &Value) -> bool {
        ["type", "$ref", "anyOf", "oneOf", "allOf", "enum", "const"]
            .iter()
            .any(|k| prop.get(k).is_some())
    }

    #[test]
    fn catalog_every_schema_is_a_valid_object_schema() {
        for t in catalog() {
            let schema = (t.input_schema)();
            let name = t.name;
            assert_eq!(
                schema["type"], "object",
                "{name}: MCP needs an object schema"
            );
            assert!(
                schema["$schema"]
                    .as_str()
                    .is_some_and(|s| s.starts_with("https://json-schema.org/")),
                "{name}: no $schema"
            );
            let props = schema["properties"]
                .as_object()
                .cloned()
                .unwrap_or_default();
            for (key, prop) in &props {
                assert!(prop.is_object(), "{name}.{key} is not a schema");
                assert!(describes_a_type(prop), "{name}.{key} has no type: {prop}");
                assert!(
                    prop["description"].as_str().is_some_and(|d| !d.is_empty()),
                    "{name}.{key} has no description for the model"
                );
            }
            for req in schema["required"].as_array().into_iter().flatten() {
                let key = req.as_str().unwrap_or_default();
                assert!(
                    props.contains_key(key),
                    "{name}: required {key} is not a property"
                );
            }
            // Every $ref points into the schema's own definitions.
            let text = schema.to_string();
            for (idx, _) in text.match_indices("\"$ref\":\"") {
                let rest = &text[idx + 8..];
                let target = rest.split('"').next().unwrap_or_default();
                let key = target.strip_prefix("#/$defs/").unwrap_or_else(|| {
                    panic!("{name}: $ref {target} is not local");
                });
                assert!(
                    schema["$defs"][key].is_object(),
                    "{name}: dangling {target}"
                );
            }
        }
    }

    #[test]
    fn catalog_schemas_accept_the_shape_a_model_would_send() {
        let logs = (by_name("containers.logs").input_schema)();
        assert_eq!(logs["required"], serde_json::json!(["container"]));
        for key in ["container", "engine", "since", "tail", "grep"] {
            assert!(logs["properties"][key].is_object(), "containers.logs.{key}");
        }
        let edit = (by_name("files.propose_edit").input_schema)();
        assert_eq!(edit["required"], serde_json::json!(["path", "content"]));
        let prune = (by_name("system.prune").input_schema)();
        assert!(prune["$defs"]["PruneKind"]["enum"]
            .as_array()
            .is_some_and(|e| e.iter().any(|v| v == "build_cache")));
    }

    #[test]
    fn catalog_has_a_description_for_every_tool_and_masks_are_promised_where_needed() {
        for t in catalog() {
            assert!(t.description.len() > 20, "{}", t.name);
        }
        for name in [
            "containers.inspect",
            "containers.logs",
            "compose.config",
            "files.read",
        ] {
            assert!(by_name(name).description.contains("••••"), "{name}");
        }
    }
}
