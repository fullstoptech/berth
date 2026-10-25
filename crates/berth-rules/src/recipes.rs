//! Generation recipes per stack (PLAN section 6.7, D31, D32).
//!
//! A recipe turns a [`Detection`] into a [`GenerationPlan`]: a multi-stage `Dockerfile` and a `.dockerignore`
//! when the folder lacks them, a `compose.yaml` with the services the project needs, and `.env` additions
//! (a generated password per service). Nothing is written here. A file the folder already has is never a
//! Create: the only file a plan changes is `.env`, as a Modify that records the hash of what was read.

use std::collections::BTreeMap;
use std::path::Path;

use berth_core::{ComposeCommand, Detection, GenerationPlan, PlanSource, ServiceKind, StackGuess};

use crate::{compose_edit, diff, ports, templates, RulesError};

const COMPOSE_FILES: [&str; 4] = [
    "compose.yaml",
    "compose.yml",
    "docker-compose.yml",
    "docker-compose.yaml",
];

const DEFAULT_NODE: &str = "22";
const PYTHON_IMAGE: &str = "python:3.12-slim";
const GO_IMAGE: &str = "golang:1.25-alpine";
const GO_RUNTIME: &str = "alpine:3.22";
const RUST_IMAGE: &str = "rust:1-slim-bookworm";
const RUST_RUNTIME: &str = "debian:bookworm-slim";
const COMPOSER_IMAGE: &str = "composer:2";
const PHP_IMAGE: &str = "php:8.4-cli-alpine";
const RUBY_IMAGE: &str = "ruby:3.4-slim";
const MAVEN_IMAGE: &str = "maven:3-eclipse-temurin-21";
const GRADLE_IMAGE: &str = "gradle:8-jdk21";
const JRE_IMAGE: &str = "eclipse-temurin:21-jre";
const NGINX_IMAGE: &str = "nginx:1.29-alpine";
const BUN_IMAGE: &str = "oven/bun:1-alpine";

/// What the Generate sheet lets the user change.
///
/// `ports` maps a compose service name to its host port: `app`, `db`, `redis` and so on. A service with a
/// second published port uses `<service>.<input>` for it (`rabbitmq.management_port`, `minio.console_port`,
/// `mailpit.web_port`). A missing entry means the default.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecipeOptions {
    pub ports: std::collections::BTreeMap<String, u16>,
    pub node_version: Option<String>,
    pub services: Vec<berth_core::ServiceKind>,
}

/// Options for `d`: its detected services, a free host port for each published port (`taken_ports` are the
/// host ports in use, the next free port above the wanted one is offered) and the Node major version.
#[must_use]
pub fn options_for(d: &berth_core::Detection, taken_ports: &[u16]) -> RecipeOptions {
    let mut taken = taken_ports.to_vec();
    let mut ports_map = BTreeMap::new();

    let container = app_container_port(d);
    let app = ports::suggest(default_host_port(container), &taken);
    taken.push(app);
    ports_map.insert("app".to_string(), app);

    let services = kinds_of(d);
    for (kind, name) in service_names(&services) {
        let Some(t) = template_for(kind) else {
            continue;
        };
        for (key, default) in port_inputs(&t) {
            let host = ports::suggest(default, &taken);
            taken.push(host);
            ports_map.insert(port_key(&name, &key), host);
        }
    }

    let node_version = d.stack.iter().find_map(|s| match s {
        StackGuess::Node { version, .. } => Some(
            version
                .as_deref()
                .and_then(major)
                .unwrap_or_else(|| DEFAULT_NODE.to_string()),
        ),
        _ => None,
    });

    RecipeOptions {
        ports: ports_map,
        node_version,
        services,
    }
}

/// Dockerfile (when missing), .dockerignore (when missing), compose.yaml, .env additions. Never overwrites a file the folder has.
/// Reads and hashes .env for Modify diffs.
///
/// A folder that already has a compose file gets a plan without files and a note saying so: adding a service
/// to an existing file is `templates::add_service_plan`.
///
/// # Errors
/// `Read` when the folder has no recognised stack and no Dockerfile, or a file cannot be read.
pub fn plan(
    d: &berth_core::Detection,
    o: &RecipeOptions,
) -> Result<berth_core::GenerationPlan, RulesError> {
    generate(d, o, Mode::Full)
}

/// The compose.yaml (and `.env` additions) for a folder that has a Dockerfile: builds from it, publishes the
/// port its `EXPOSE` names, and never writes a Dockerfile or `.dockerignore`.
///
/// # Errors
/// `Read` when the folder has no Dockerfile, or a file cannot be read.
pub fn compose_for_dockerfile(
    d: &berth_core::Detection,
    o: &RecipeOptions,
) -> Result<berth_core::GenerationPlan, RulesError> {
    generate(d, o, Mode::ComposeOnly)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Full,
    ComposeOnly,
}

fn generate(d: &Detection, o: &RecipeOptions, mode: Mode) -> Result<GenerationPlan, RulesError> {
    let root = d.folder.as_path();
    let stack = d.stack.first();
    let dockerfile_text = read_opt(&root.join("Dockerfile"))?;
    let facts = dockerfile_text.as_deref().map(dockerfile_facts);
    let recipe = stack.map(|s| recipe_for(s, d, o, facts.as_ref()));
    let label = recipe.as_ref().map_or("Dockerfile", |r| r.label);
    let source = PlanSource::Recipe(label.to_string());

    if COMPOSE_FILES.iter().any(|f| present(&root.join(f))) {
        return Ok(GenerationPlan {
            root: root.to_path_buf(),
            files: Vec::new(),
            source,
            notes: vec![
                "This folder already has a compose file. Berth did not change it.".to_string(),
            ],
            after_write: None,
        });
    }
    if mode == Mode::ComposeOnly && dockerfile_text.is_none() {
        return Err(RulesError::Read(
            root.join("Dockerfile"),
            "there is no Dockerfile to build from".to_string(),
        ));
    }
    if dockerfile_text.is_none() && recipe.is_none() {
        return Err(RulesError::Read(
            root.to_path_buf(),
            "no stack was recognised and the folder has no Dockerfile".to_string(),
        ));
    }

    let container_port = facts
        .as_ref()
        .and_then(|f| f.expose)
        .or_else(|| recipe.as_ref().map(|r| r.container_port))
        .or_else(|| {
            facts
                .as_ref()
                .and_then(|f| f.from.as_deref())
                .filter(|f| f.contains("nginx") || f.contains("httpd"))
                .map(|_| 80)
        })
        .unwrap_or(8080);
    let host_port = o
        .ports
        .get("app")
        .copied()
        .unwrap_or_else(|| default_host_port(container_port));

    let env_text = read_opt(&root.join(".env"))?;
    let project = project_name(root);
    let kinds = unique(&o.services);
    let names = service_names(&kinds);
    let framework = recipe.as_ref().map_or(Framework::Other, |r| r.framework);

    // Service blocks, volumes and `.env` lines first: the app block needs to know whether `.env` exists.
    let mut env_lines: Vec<(String, String)> = Vec::new();
    let mut rendered: Vec<(String, templates::Rendered)> = Vec::new();
    for (kind, name) in &names {
        let Some(t) = template_for(*kind) else {
            continue;
        };
        let mut inputs = BTreeMap::new();
        for (key, default) in port_inputs(&t) {
            let host = o
                .ports
                .get(&port_key(name, &key))
                .copied()
                .unwrap_or(default);
            inputs.insert(key, host.to_string());
        }
        for i in &t.inputs {
            if let berth_core::TemplateInputKind::VolumeName { default } = &i.kind {
                inputs.insert(i.key.clone(), format!("{project}_{default}"));
            }
        }
        let r = templates::render_full(&t, &inputs);
        env_lines.extend(r.env.iter().cloned());
        rendered.push((name.clone(), r));
    }
    let new_env = compose_edit::add_env_lines(env_text.as_deref(), &env_lines);
    let has_env = env_text.is_some() || !new_env.is_empty();

    let primary_sql = kinds
        .iter()
        .find(|k| matches!(k, ServiceKind::Postgres | ServiceKind::Mysql))
        .copied();
    let mut app_env: Vec<(String, String)> = Vec::new();
    for (kind, name) in &names {
        app_env.extend(app_env_for(
            *kind,
            name,
            framework,
            primary_sql == Some(*kind),
        ));
    }

    let header = if dockerfile_text.is_some() {
        "# Generated by Berth from the Dockerfile and the key names in .env".to_string()
    } else {
        format!("# Generated by Berth from the {label} recipe and the key names in .env")
    };
    let mut compose = format!("{header}\nservices:\n");
    compose.push_str(&app_block(
        host_port,
        container_port,
        has_env,
        &app_env,
        &names,
        recipe.as_ref().map_or(&[][..], |r| &r.watch),
    ));
    for (name, r) in &rendered {
        compose = compose_edit::add_service(&compose, name, &r.service)?;
        for v in &r.volumes {
            compose = compose_edit::add_named_volume(&compose, v)?;
        }
    }

    let mut files = vec![diff::file_diff(
        Path::new("compose.yaml"),
        None,
        Some(&compose),
    )];
    let mut notes = Vec::new();
    match (&dockerfile_text, &recipe) {
        (Some(_), _) => notes.push("Reuses your Dockerfile for the app service.".to_string()),
        (None, Some(r)) => {
            notes.push(format!("Writes a multi-stage Dockerfile for {}.", r.label));
        }
        (None, None) => {}
    }
    if let Some(r) = &recipe {
        notes.extend(r.notes.iter().cloned());
    }

    if mode == Mode::Full {
        if dockerfile_text.is_none() {
            if let Some(r) = &recipe {
                files.push(diff::file_diff(
                    Path::new("Dockerfile"),
                    None,
                    Some(&r.dockerfile),
                ));
            }
        }
        if !present(&root.join(".dockerignore")) {
            let ignore = recipe
                .as_ref()
                .map_or_else(|| dockerignore(&[]), |r| r.dockerignore.clone());
            files.push(diff::file_diff(
                Path::new(".dockerignore"),
                None,
                Some(&ignore),
            ));
        }
    }

    if env_text.as_deref() != Some(new_env.as_str()) && !new_env.is_empty() {
        let have = env_text
            .as_deref()
            .map(templates::env_keys)
            .unwrap_or_default();
        for (k, _) in &env_lines {
            if !have.contains(k) {
                notes.push(format!("Adds {k} to .env."));
            }
        }
        files.push(diff::file_diff(
            Path::new(".env"),
            env_text.as_deref(),
            Some(&new_env),
        ));
    }

    Ok(GenerationPlan {
        root: root.to_path_buf(),
        files,
        source,
        notes,
        after_write: Some(ComposeCommand::Up {
            services: Vec::new(),
            build: true,
        }),
    })
}

// ---------- files and names ----------

/// A file or symlink at `p`, whatever it points to. A dangling link still counts: it is not ours to replace.
fn present(p: &Path) -> bool {
    p.symlink_metadata().is_ok()
}

fn read_opt(p: &Path) -> Result<Option<String>, RulesError> {
    match std::fs::read_to_string(p) {
        Ok(t) => Ok(Some(t)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(RulesError::Read(p.to_path_buf(), e.to_string())),
    }
}

/// The folder name as a compose-safe prefix for volume names.
fn project_name(root: &Path) -> String {
    let raw = root
        .file_name()
        .map(|n| n.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    let cleaned: String = raw
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect();
    let cleaned = cleaned.trim_matches(|c| c == '-' || c == '_').to_string();
    if cleaned.is_empty() {
        "app".to_string()
    } else {
        cleaned
    }
}

fn unique(kinds: &[ServiceKind]) -> Vec<ServiceKind> {
    let mut out: Vec<ServiceKind> = Vec::new();
    for k in kinds {
        if !out.contains(k) {
            out.push(*k);
        }
    }
    out
}

fn kinds_of(d: &Detection) -> Vec<ServiceKind> {
    unique(&d.services_needed.iter().map(|s| s.kind).collect::<Vec<_>>())
}

fn template_id(kind: ServiceKind) -> &'static str {
    match kind {
        ServiceKind::Postgres => "postgres",
        ServiceKind::Mysql => "mysql",
        ServiceKind::Mongo => "mongodb",
        ServiceKind::Redis => "redis",
        ServiceKind::Rabbitmq => "rabbitmq",
        ServiceKind::Meilisearch => "meilisearch",
        ServiceKind::Minio => "minio",
        ServiceKind::Mailpit => "mailpit",
    }
}

fn template_for(kind: ServiceKind) -> Option<berth_core::Template> {
    let id = template_id(kind);
    templates::built_in().into_iter().find(|t| t.id == id)
}

/// Compose service names. The one SQL database of a project is `db`; two of them keep their own names.
fn service_names(kinds: &[ServiceKind]) -> Vec<(ServiceKind, String)> {
    let sql = kinds
        .iter()
        .filter(|k| matches!(k, ServiceKind::Postgres | ServiceKind::Mysql))
        .count();
    kinds
        .iter()
        .map(|k| {
            let name = match k {
                ServiceKind::Postgres | ServiceKind::Mysql if sql == 1 => "db",
                ServiceKind::Mongo => "mongo",
                other => template_id(*other),
            };
            (*k, name.to_string())
        })
        .collect()
}

fn port_inputs(t: &berth_core::Template) -> Vec<(String, u16)> {
    t.inputs
        .iter()
        .filter_map(|i| match i.kind {
            berth_core::TemplateInputKind::Port { default } => Some((i.key.clone(), default)),
            _ => None,
        })
        .collect()
}

fn port_key(service: &str, input: &str) -> String {
    if input == "PORT" {
        service.to_string()
    } else {
        format!("{service}.{}", input.to_lowercase())
    }
}

fn major(version: &str) -> Option<String> {
    let m: String = version.chars().take_while(char::is_ascii_digit).collect();
    (!m.is_empty()).then_some(m)
}

fn default_host_port(container: u16) -> u16 {
    if container < 1024 {
        container + 8000
    } else {
        container
    }
}

fn app_container_port(d: &Detection) -> u16 {
    let facts = std::fs::read_to_string(d.folder.join("Dockerfile"))
        .ok()
        .map(|t| dockerfile_facts(&t));
    let from_dockerfile = facts.as_ref().and_then(|f| f.expose);
    from_dockerfile
        .or_else(|| d.stack.first().map(stack_port))
        .unwrap_or(8080)
}

// ---------- Dockerfile facts ----------

#[derive(Debug, Default)]
struct DockerfileFacts {
    expose: Option<u16>,
    workdir: Option<String>,
    from: Option<String>,
}

/// The last `EXPOSE` port, the last absolute `WORKDIR` and the last `FROM` image of a Dockerfile.
fn dockerfile_facts(text: &str) -> DockerfileFacts {
    let mut f = DockerfileFacts::default();
    for line in text.lines() {
        let line = line.trim();
        let (word, rest) = line.split_once(char::is_whitespace).unwrap_or((line, ""));
        let rest = rest.trim();
        match word.to_ascii_uppercase().as_str() {
            "EXPOSE" => {
                let first = rest.split_whitespace().next().unwrap_or("");
                if let Ok(p) = first.split('/').next().unwrap_or("").parse::<u16>() {
                    f.expose = Some(p);
                }
            }
            "WORKDIR" if rest.starts_with('/') && !rest.contains('$') => {
                f.workdir = Some(rest.trim_end_matches('/').to_string());
            }
            "FROM" => {
                f.from = rest
                    .split_whitespace()
                    .find(|t| !t.starts_with("--"))
                    .map(str::to_string);
            }
            _ => {}
        }
    }
    f
}

// ---------- stack recipes ----------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Framework {
    Laravel,
    Spring,
    Other,
}

struct Watch {
    action: &'static str,
    path: String,
    target: Option<String>,
    ignore: Vec<&'static str>,
}

struct Recipe {
    label: &'static str,
    container_port: u16,
    dockerfile: String,
    dockerignore: String,
    watch: Vec<Watch>,
    framework: Framework,
    notes: Vec<String>,
}

fn stack_port(s: &StackGuess) -> u16 {
    match s {
        StackGuess::Node { framework, .. } if framework.as_deref() == Some("Vite") => 80,
        StackGuess::Node { .. } | StackGuess::Ruby { .. } => 3000,
        StackGuess::Python { .. } | StackGuess::Php { .. } => 8000,
        StackGuess::Go | StackGuess::Rust | StackGuess::Java { .. } => 8080,
        StackGuess::Static => 80,
    }
}

fn recipe_for(
    stack: &StackGuess,
    d: &Detection,
    o: &RecipeOptions,
    facts: Option<&DockerfileFacts>,
) -> Recipe {
    let workdir = facts
        .and_then(|f| f.workdir.clone())
        .unwrap_or_else(|| "/app".to_string());
    let kinds = unique(&o.services);
    let container_port = stack_port(stack);
    let base = |label, dockerfile, ignore: &[&str], watch, framework, notes| Recipe {
        label,
        container_port,
        dockerfile,
        dockerignore: dockerignore(ignore),
        watch,
        framework,
        notes,
    };
    match stack {
        StackGuess::Node {
            pm,
            framework,
            version,
        } => {
            let v = o
                .node_version
                .as_deref()
                .and_then(major)
                .or_else(|| version.as_deref().and_then(major))
                .unwrap_or_else(|| DEFAULT_NODE.to_string());
            let fw = framework.as_deref();
            let src = d.folder.join("src").is_dir();
            let (path, target) = if src {
                ("./src".to_string(), Some(format!("{workdir}/src")))
            } else {
                (".".to_string(), Some(workdir.clone()))
            };
            let mut watch = Vec::new();
            if fw == Some("Vite") {
                // nginx serves the built files: a change needs a new build, not a file sync.
                watch.push(Watch {
                    action: "rebuild",
                    path,
                    target: None,
                    ignore: if src {
                        Vec::new()
                    } else {
                        vec!["node_modules/", "dist/"]
                    },
                });
            } else {
                watch.push(Watch {
                    action: "sync",
                    path,
                    target,
                    ignore: if src {
                        Vec::new()
                    } else {
                        vec!["node_modules/", ".next/", "dist/"]
                    },
                });
            }
            watch.push(Watch {
                action: "rebuild",
                path: "package.json".to_string(),
                target: None,
                ignore: Vec::new(),
            });
            base(
                "Node",
                node_dockerfile(pm, fw, &v),
                &["node_modules", ".next", "dist", "npm-debug.log"],
                watch,
                Framework::Other,
                Vec::new(),
            )
        }
        StackGuess::Python { tool, framework } => {
            let fw = framework.as_deref();
            let manifest = if tool == "pip" {
                "requirements.txt"
            } else {
                "pyproject.toml"
            };
            let watch = vec![
                Watch {
                    action: "sync",
                    path: ".".to_string(),
                    target: Some(workdir),
                    ignore: vec![".venv/", "__pycache__/", ".git/"],
                },
                Watch {
                    action: "rebuild",
                    path: manifest.to_string(),
                    target: None,
                    ignore: Vec::new(),
                },
            ];
            let mut notes = Vec::new();
            if fw == Some("FastAPI") {
                notes.push(
                    "The Dockerfile starts main:app with uvicorn. Change CMD if your app lives elsewhere."
                        .to_string(),
                );
            }
            base(
                "Python",
                python_dockerfile(tool, fw),
                &[".venv", "__pycache__", "*.pyc", ".pytest_cache"],
                watch,
                Framework::Other,
                notes,
            )
        }
        StackGuess::Go => base(
            "Go",
            go_dockerfile(),
            &["bin"],
            Vec::new(),
            Framework::Other,
            Vec::new(),
        ),
        StackGuess::Rust => base(
            "Rust",
            rust_dockerfile(&rust_bin_name(&d.folder)),
            &["target"],
            Vec::new(),
            Framework::Other,
            Vec::new(),
        ),
        StackGuess::Php { framework } => {
            let laravel = framework.as_deref() == Some("Laravel");
            base(
                "PHP",
                php_dockerfile(&kinds),
                &["vendor", "node_modules", "storage/logs"],
                Vec::new(),
                if laravel {
                    Framework::Laravel
                } else {
                    Framework::Other
                },
                Vec::new(),
            )
        }
        StackGuess::Ruby { .. } => base(
            "Ruby",
            ruby_dockerfile(&kinds),
            &["log", "tmp", ".bundle", "node_modules"],
            Vec::new(),
            Framework::Other,
            Vec::new(),
        ),
        StackGuess::Java { build, framework } => base(
            "Java",
            java_dockerfile(build),
            &["target", "build", ".gradle"],
            Vec::new(),
            if framework.as_deref() == Some("Spring Boot") {
                Framework::Spring
            } else {
                Framework::Other
            },
            Vec::new(),
        ),
        StackGuess::Static => base(
            "Static site",
            static_dockerfile(),
            &[],
            Vec::new(),
            Framework::Other,
            Vec::new(),
        ),
    }
}

fn json_list(items: &[&str]) -> String {
    let inner: Vec<String> = items
        .iter()
        .map(|i| format!("\"{}\"", i.replace('\\', "\\\\").replace('"', "\\\"")))
        .collect();
    format!("[{}]", inner.join(", "))
}

fn dockerignore(extra: &[&str]) -> String {
    let mut out = String::from(
        "# Keeps secrets and build output out of the image.\n.git\n.gitignore\n.env\n.env.*\n!.env.example\nDockerfile\n.dockerignore\ncompose.yaml\ncompose.yml\ndocker-compose.yml\ndocker-compose.yaml\n*.md\n",
    );
    for e in extra {
        out.push_str(e);
        out.push('\n');
    }
    out
}

fn node_dockerfile(pm: &str, framework: Option<&str>, version: &str) -> String {
    let (image, setup, lock, install, build, start): (String, &str, &str, &str, &str, &[&str]) =
        match pm {
            "pnpm" => (
                format!("node:{version}-alpine"),
                "corepack enable && ",
                "pnpm-lock.yaml*",
                "pnpm install --frozen-lockfile",
                "pnpm build",
                &["pnpm", "start"],
            ),
            "yarn" => (
                format!("node:{version}-alpine"),
                "corepack enable && ",
                "yarn.lock*",
                "yarn install --frozen-lockfile",
                "yarn build",
                &["yarn", "start"],
            ),
            "bun" => (
                BUN_IMAGE.to_string(),
                "",
                "bun.lock*",
                "bun install --frozen-lockfile",
                "bun run build",
                &["bun", "run", "start"],
            ),
            _ => (
                format!("node:{version}-alpine"),
                "",
                "package-lock.json*",
                "npm ci",
                "npm run build",
                &["npm", "start"],
            ),
        };
    let deps = format!(
        "FROM {image} AS deps\nWORKDIR /app\nCOPY package.json {lock} ./\nRUN {setup}{install}\n"
    );
    let cmd = json_list(start);
    match framework {
        Some("Next.js") => format!(
            "{deps}\nFROM {image} AS build\nWORKDIR /app\nCOPY --from=deps /app/node_modules ./node_modules\nCOPY . .\nRUN {setup}{build}\n\nFROM {image} AS runtime\nWORKDIR /app\nENV NODE_ENV=production\nCOPY --from=build /app ./\nEXPOSE 3000\nCMD {cmd}\n"
        ),
        Some("Vite") => format!(
            "{deps}\nFROM {image} AS build\nWORKDIR /app\nCOPY --from=deps /app/node_modules ./node_modules\nCOPY . .\nRUN {setup}{build}\n\nFROM {NGINX_IMAGE} AS runtime\nCOPY --from=build /app/dist /usr/share/nginx/html\nEXPOSE 80\n"
        ),
        _ => format!(
            "{deps}\nFROM {image} AS runtime\nWORKDIR /app\nENV NODE_ENV=production\nCOPY --from=deps /app/node_modules ./node_modules\nCOPY . .\nEXPOSE 3000\nCMD {cmd}\n"
        ),
    }
}

fn python_dockerfile(tool: &str, framework: Option<&str>) -> String {
    let install = match tool {
        "uv" => "RUN pip install --no-cache-dir uv\nCOPY pyproject.toml uv.lock* ./\nRUN uv sync --frozen --no-dev --no-install-project",
        "poetry" => "RUN pip install --no-cache-dir poetry\nENV POETRY_VIRTUALENVS_IN_PROJECT=true\nCOPY pyproject.toml poetry.lock* ./\nRUN poetry install --only main --no-root",
        _ => "RUN python -m venv /app/.venv\nCOPY requirements.txt ./\nRUN /app/.venv/bin/pip install --no-cache-dir -r requirements.txt",
    };
    let cmd = match framework {
        Some("Django") => json_list(&["python", "manage.py", "runserver", "0.0.0.0:8000"]),
        Some("FastAPI") => {
            json_list(&["uvicorn", "main:app", "--host", "0.0.0.0", "--port", "8000"])
        }
        Some("Flask") => json_list(&[
            "flask", "--app", "app", "run", "--host", "0.0.0.0", "--port", "8000",
        ]),
        _ => json_list(&["python", "main.py"]),
    };
    format!(
        "FROM {PYTHON_IMAGE} AS builder\nWORKDIR /app\n{install}\n\nFROM {PYTHON_IMAGE} AS runtime\nWORKDIR /app\nENV PATH=\"/app/.venv/bin:$PATH\" PYTHONUNBUFFERED=1\nCOPY --from=builder /app/.venv /app/.venv\nCOPY . .\nEXPOSE 8000\nCMD {cmd}\n"
    )
}

fn go_dockerfile() -> String {
    format!(
        "FROM {GO_IMAGE} AS build\nWORKDIR /src\nCOPY go.mod go.sum* ./\nRUN go mod download\nCOPY . .\nRUN CGO_ENABLED=0 go build -o /out/app .\n\nFROM {GO_RUNTIME} AS runtime\nWORKDIR /app\nCOPY --from=build /out/app ./app\nEXPOSE 8080\nCMD [\"./app\"]\n"
    )
}

/// The binary Cargo builds: `[package] name`, else the folder name.
fn rust_bin_name(folder: &Path) -> String {
    std::fs::read_to_string(folder.join("Cargo.toml"))
        .ok()
        .and_then(|t| t.parse::<toml::Table>().ok())
        .and_then(|doc| {
            doc.get("package")
                .and_then(|p| p.get("name"))
                .and_then(|n| n.as_str())
                .map(str::to_string)
        })
        .filter(|n| templates::valid_name(n))
        .unwrap_or_else(|| project_name(folder))
}

fn rust_dockerfile(bin: &str) -> String {
    format!(
        "FROM {RUST_IMAGE} AS build\nWORKDIR /src\nCOPY . .\nRUN cargo build --release\n\nFROM {RUST_RUNTIME} AS runtime\nCOPY --from=build /src/target/release/{bin} /usr/local/bin/{bin}\nEXPOSE 8080\nCMD [\"{bin}\"]\n"
    )
}

fn php_dockerfile(kinds: &[ServiceKind]) -> String {
    let mut ext = String::new();
    if kinds.contains(&ServiceKind::Postgres) {
        ext.push_str("RUN apk add --no-cache postgresql-dev && docker-php-ext-install pdo_pgsql\n");
    }
    if kinds.contains(&ServiceKind::Mysql) {
        ext.push_str("RUN docker-php-ext-install pdo_mysql\n");
    }
    format!(
        "FROM {COMPOSER_IMAGE} AS vendor\nWORKDIR /app\nCOPY composer.json composer.lock* ./\nRUN composer install --no-dev --no-scripts --no-autoloader --prefer-dist\nCOPY . .\nRUN composer dump-autoload --optimize --no-dev\n\nFROM {PHP_IMAGE} AS runtime\nWORKDIR /app\n{ext}COPY --from=vendor /app ./\nEXPOSE 8000\nCMD [\"php\", \"artisan\", \"serve\", \"--host=0.0.0.0\", \"--port=8000\"]\n"
    )
}

fn ruby_dockerfile(kinds: &[ServiceKind]) -> String {
    let (build_pkgs, run_pkgs) = if kinds.contains(&ServiceKind::Mysql) {
        (
            "build-essential default-libmysqlclient-dev git",
            "libmariadb3",
        )
    } else {
        ("build-essential libpq-dev git", "libpq5")
    };
    format!(
        "FROM {RUBY_IMAGE} AS build\nWORKDIR /app\nRUN apt-get update && apt-get install -y --no-install-recommends {build_pkgs} && rm -rf /var/lib/apt/lists/*\nCOPY Gemfile Gemfile.lock* ./\nRUN bundle config set without 'development test' && bundle install\nCOPY . .\n\nFROM {RUBY_IMAGE} AS runtime\nWORKDIR /app\nRUN apt-get update && apt-get install -y --no-install-recommends {run_pkgs} && rm -rf /var/lib/apt/lists/*\nCOPY --from=build /usr/local/bundle /usr/local/bundle\nCOPY --from=build /app ./\nENV RAILS_ENV=production\nEXPOSE 3000\nCMD [\"bin/rails\", \"server\", \"-b\", \"0.0.0.0\", \"-p\", \"3000\"]\n"
    )
}

fn java_dockerfile(build: &str) -> String {
    if build == "Gradle" {
        format!(
            "FROM {GRADLE_IMAGE} AS build\nWORKDIR /src\nCOPY . .\nRUN gradle bootJar --no-daemon -x test\n\nFROM {JRE_IMAGE} AS runtime\nWORKDIR /app\nCOPY --from=build /src/build/libs /tmp/libs\nRUN cp \"$(ls /tmp/libs/*.jar | grep -v plain | head -n 1)\" app.jar && rm -rf /tmp/libs\nEXPOSE 8080\nCMD [\"java\", \"-jar\", \"app.jar\"]\n"
        )
    } else {
        format!(
            "FROM {MAVEN_IMAGE} AS build\nWORKDIR /src\nCOPY pom.xml ./\nRUN mvn -q -B dependency:go-offline\nCOPY . .\nRUN mvn -q -B package -DskipTests\n\nFROM {JRE_IMAGE} AS runtime\nWORKDIR /app\nCOPY --from=build /src/target/*.jar app.jar\nEXPOSE 8080\nCMD [\"java\", \"-jar\", \"app.jar\"]\n"
        )
    }
}

fn static_dockerfile() -> String {
    format!("FROM {NGINX_IMAGE}\nCOPY . /usr/share/nginx/html\nEXPOSE 80\n")
}

// ---------- compose ----------

/// A value as a YAML scalar: bare when plain, double-quoted when YAML would read it as a number, a boolean
/// or something with structure.
fn scalar(v: &str) -> String {
    let lower = v.to_ascii_lowercase();
    let quote = v.is_empty()
        || v.chars().all(|c| c.is_ascii_digit() || c == '.')
        || matches!(
            lower.as_str(),
            "true" | "false" | "yes" | "no" | "on" | "off" | "null" | "~"
        )
        || v.contains(": ")
        || v.contains(" #")
        || v.starts_with(|c: char| "!&*-?[]{}|>'\"%@`#,".contains(c));
    if quote {
        format!("\"{}\"", v.replace('\\', "\\\\").replace('"', "\\\""))
    } else {
        v.to_string()
    }
}

/// The environment the app service gets: URLs with the containers' hostnames, and the passwords as
/// `${VAR}` references that compose reads from `.env`.
fn app_env_for(
    kind: ServiceKind,
    host: &str,
    framework: Framework,
    primary_sql: bool,
) -> Vec<(String, String)> {
    let kv = |k: &str, v: String| (k.to_string(), v);
    let url_key = if primary_sql {
        "DATABASE_URL"
    } else {
        "MYSQL_URL"
    };
    match (framework, kind) {
        (Framework::Laravel, ServiceKind::Postgres) => vec![
            kv("DB_CONNECTION", "pgsql".into()),
            kv("DB_HOST", host.into()),
            kv("DB_PORT", "5432".into()),
            kv("DB_DATABASE", "app".into()),
            kv("DB_USERNAME", "app".into()),
            kv("DB_PASSWORD", "${POSTGRES_PASSWORD}".into()),
        ],
        (Framework::Laravel, ServiceKind::Mysql) => vec![
            kv("DB_CONNECTION", "mysql".into()),
            kv("DB_HOST", host.into()),
            kv("DB_PORT", "3306".into()),
            kv("DB_DATABASE", "app".into()),
            kv("DB_USERNAME", "app".into()),
            kv("DB_PASSWORD", "${MYSQL_PASSWORD}".into()),
        ],
        (Framework::Laravel, ServiceKind::Redis) => vec![kv("REDIS_HOST", host.into())],
        (Framework::Laravel, ServiceKind::Mailpit) => {
            vec![kv("MAIL_HOST", host.into()), kv("MAIL_PORT", "1025".into())]
        }
        (Framework::Spring, ServiceKind::Postgres) => vec![
            kv(
                "SPRING_DATASOURCE_URL",
                format!("jdbc:postgresql://{host}:5432/app"),
            ),
            kv("SPRING_DATASOURCE_USERNAME", "app".into()),
            kv("SPRING_DATASOURCE_PASSWORD", "${POSTGRES_PASSWORD}".into()),
        ],
        (Framework::Spring, ServiceKind::Mysql) => vec![
            kv(
                "SPRING_DATASOURCE_URL",
                format!("jdbc:mysql://{host}:3306/app"),
            ),
            kv("SPRING_DATASOURCE_USERNAME", "app".into()),
            kv("SPRING_DATASOURCE_PASSWORD", "${MYSQL_PASSWORD}".into()),
        ],
        (Framework::Spring, ServiceKind::Redis) => {
            vec![kv("SPRING_DATA_REDIS_HOST", host.into())]
        }
        (Framework::Spring, ServiceKind::Rabbitmq) => vec![
            kv("SPRING_RABBITMQ_HOST", host.into()),
            kv("SPRING_RABBITMQ_USERNAME", "app".into()),
            kv("SPRING_RABBITMQ_PASSWORD", "${RABBITMQ_PASSWORD}".into()),
        ],
        (Framework::Spring, ServiceKind::Mongo) => vec![kv(
            "SPRING_DATA_MONGODB_URI",
            format!("mongodb://app:${{MONGO_PASSWORD}}@{host}:27017/app?authSource=admin"),
        )],
        (_, ServiceKind::Postgres) => vec![kv(
            url_key,
            format!("postgres://app:${{POSTGRES_PASSWORD}}@{host}:5432/app"),
        )],
        (_, ServiceKind::Mysql) => vec![kv(
            url_key,
            format!("mysql://app:${{MYSQL_PASSWORD}}@{host}:3306/app"),
        )],
        (_, ServiceKind::Mongo) => vec![kv(
            "MONGODB_URI",
            format!("mongodb://app:${{MONGO_PASSWORD}}@{host}:27017/app?authSource=admin"),
        )],
        (_, ServiceKind::Redis) => vec![kv("REDIS_URL", format!("redis://{host}:6379"))],
        (_, ServiceKind::Rabbitmq) => vec![kv(
            "RABBITMQ_URL",
            format!("amqp://app:${{RABBITMQ_PASSWORD}}@{host}:5672"),
        )],
        (_, ServiceKind::Meilisearch) => vec![
            kv("MEILISEARCH_URL", format!("http://{host}:7700")),
            kv("MEILI_MASTER_KEY", "${MEILI_MASTER_KEY}".into()),
        ],
        (_, ServiceKind::Minio) => vec![
            kv("S3_ENDPOINT", format!("http://{host}:9000")),
            kv("AWS_ACCESS_KEY_ID", "app".into()),
            kv("AWS_SECRET_ACCESS_KEY", "${MINIO_PASSWORD}".into()),
        ],
        (_, ServiceKind::Mailpit) => {
            vec![kv("SMTP_HOST", host.into()), kv("SMTP_PORT", "1025".into())]
        }
    }
}

fn app_block(
    host_port: u16,
    container_port: u16,
    has_env: bool,
    env: &[(String, String)],
    services: &[(ServiceKind, String)],
    watch: &[Watch],
) -> String {
    let mut s = String::from("  app:\n    build: .\n");
    s.push_str(&format!(
        "    ports:\n      - \"{host_port}:{container_port}\"\n"
    ));
    if has_env {
        s.push_str("    env_file: .env\n");
    }
    if !env.is_empty() {
        s.push_str("    environment:\n");
        for (k, v) in env {
            s.push_str(&format!("      {k}: {}\n", scalar(v)));
        }
    }
    if !services.is_empty() {
        s.push_str("    depends_on:\n");
        for (_, name) in services {
            s.push_str(&format!(
                "      {name}:\n        condition: service_healthy\n"
            ));
        }
    }
    if !watch.is_empty() {
        s.push_str("    develop:\n      watch:\n");
        for w in watch {
            s.push_str(&format!("        - action: {}\n", w.action));
            s.push_str(&format!("          path: {}\n", w.path));
            if let Some(t) = &w.target {
                s.push_str(&format!("          target: {t}\n"));
            }
            if !w.ignore.is_empty() {
                s.push_str("          ignore:\n");
                for i in &w.ignore {
                    s.push_str(&format!("            - {i}\n"));
                }
            }
        }
    }
    s
}

/// Every image a generated Dockerfile starts from, over each stack variant the recipes know.
#[cfg(test)]
pub(crate) fn pinned_base_images() -> std::collections::BTreeSet<String> {
    let mut texts = Vec::new();
    for pm in ["npm", "pnpm", "yarn", "bun"] {
        for fw in [Some("Next.js"), Some("Vite"), Some("Express"), None] {
            texts.push(node_dockerfile(pm, fw, DEFAULT_NODE));
        }
    }
    for tool in ["uv", "poetry", "pip"] {
        texts.push(python_dockerfile(tool, Some("Django")));
    }
    texts.push(go_dockerfile());
    texts.push(rust_dockerfile("app"));
    texts.push(php_dockerfile(&[]));
    texts.push(ruby_dockerfile(&[]));
    texts.push(ruby_dockerfile(&[ServiceKind::Mysql]));
    texts.push(java_dockerfile("Maven"));
    texts.push(java_dockerfile("Gradle"));
    texts.push(static_dockerfile());

    let mut out = std::collections::BTreeSet::new();
    for text in texts {
        let mut stages: Vec<String> = Vec::new();
        for line in text.lines() {
            let mut words = line.split_whitespace();
            if words.next() != Some("FROM") {
                continue;
            }
            let Some(image) = words.next() else { continue };
            if !stages.iter().any(|s| s == image) {
                out.insert(image.to_string());
            }
            if let (Some("AS"), Some(alias)) = (words.next(), words.next()) {
                stages.push(alias.to_string());
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use berth_core::{FileChange, FileDiff, ServiceGuess};
    use pretty_assertions::assert_eq;

    use super::*;
    use crate::detect::detect;
    use crate::write;

    const SECRET_KEYS: [&str; 7] = [
        "POSTGRES_PASSWORD",
        "MYSQL_PASSWORD",
        "MARIADB_PASSWORD",
        "MONGO_PASSWORD",
        "RABBITMQ_PASSWORD",
        "MINIO_PASSWORD",
        "MEILI_MASTER_KEY",
    ];

    fn fixture(name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/folders")
            .join(name)
    }

    fn copy_dir(from: &Path, to: &Path) {
        std::fs::create_dir_all(to).expect("mkdir");
        for e in std::fs::read_dir(from).expect("read_dir") {
            let e = e.expect("entry");
            let target = to.join(e.file_name());
            if e.file_type().expect("type").is_dir() {
                copy_dir(&e.path(), &target);
            } else {
                std::fs::copy(e.path(), &target).expect("copy");
            }
        }
    }

    /// A temp copy of a fixture folder; the folder inside is named like the fixture.
    fn copy_fixture(name: &str) -> (tempfile::TempDir, PathBuf) {
        let tmp = tempfile::tempdir().expect("tempdir");
        let dir = tmp.path().join(name);
        copy_dir(&fixture(name), &dir);
        (tmp, dir)
    }

    fn plan_of(name: &str, taken: &[u16]) -> GenerationPlan {
        let d = detect(&fixture(name)).expect("detect");
        let o = options_for(&d, taken);
        plan(&d, &o).expect("plan")
    }

    fn file<'a>(p: &'a GenerationPlan, path: &str) -> Option<&'a FileDiff> {
        p.files.iter().find(|f| f.path == Path::new(path))
    }

    fn text(p: &GenerationPlan, path: &str) -> String {
        file(p, path)
            .unwrap_or_else(|| panic!("plan has no {path}"))
            .new_text
            .clone()
            .unwrap_or_default()
    }

    /// The generated secrets in the plan's `.env`: `(key, value)` for each secret key it sets.
    fn secrets(p: &GenerationPlan) -> Vec<(String, String)> {
        let Some(env) = file(p, ".env").and_then(|f| f.new_text.clone()) else {
            return Vec::new();
        };
        env.lines()
            .filter_map(|l| l.split_once('='))
            .filter(|(k, _)| SECRET_KEYS.contains(k))
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    fn yaml(text: &str) -> serde_norway::Value {
        serde_norway::from_str(text).expect("valid yaml")
    }

    /// The snapshot text: every file with generated secrets replaced by `<redacted>`. Panics when a secret
    /// shows up anywhere but `.env`, so a snapshot can never hold one.
    fn view(p: &GenerationPlan) -> String {
        let secrets = secrets(p);
        let mut out = String::new();
        out.push_str(&format!("source: {:?}\n", p.source));
        out.push_str(&format!("after_write: {:?}\n", p.after_write));
        for n in &p.notes {
            out.push_str(&format!("note: {n}\n"));
        }
        for f in &p.files {
            let mut body = f.new_text.clone().unwrap_or_default();
            for (_, v) in &secrets {
                assert!(
                    f.path == Path::new(".env") || !body.contains(v.as_str()),
                    "a generated secret is in {}",
                    f.path.display()
                );
            }
            if f.path == Path::new(".env") {
                for (k, v) in &secrets {
                    body = body.replace(&format!("{k}={v}"), &format!("{k}=<redacted>"));
                }
            }
            out.push_str(&format!(
                "\n=== {} ({:?}, +{} -{}, old hash {}) ===\n{body}",
                f.path.display(),
                f.change,
                f.added,
                f.removed,
                if f.old_sha256.is_some() {
                    "recorded"
                } else {
                    "none"
                },
            ));
        }
        for (_, v) in &secrets {
            assert!(
                !out.contains(v.as_str()),
                "a generated secret reached the view"
            );
        }
        out
    }

    macro_rules! snapshot_recipe {
        ($test:ident, $name:expr) => {
            #[test]
            fn $test() {
                insta::assert_snapshot!($name, view(&plan_of($name, &[5432])));
            }
        };
    }

    snapshot_recipe!(recipe_node_next_pnpm, "node-next-pnpm");
    snapshot_recipe!(recipe_node_vite_npm, "node-vite-npm");
    snapshot_recipe!(recipe_python_django_poetry, "python-django-poetry");
    snapshot_recipe!(recipe_python_fastapi_uv, "python-fastapi-uv");
    snapshot_recipe!(recipe_go_mod, "go-mod");
    snapshot_recipe!(recipe_rust_axum, "rust-axum");
    snapshot_recipe!(recipe_php_laravel, "php-laravel");
    snapshot_recipe!(recipe_ruby_rails, "ruby-rails");
    snapshot_recipe!(recipe_java_spring_maven, "java-spring-maven");
    snapshot_recipe!(recipe_static_site, "static-site");
    snapshot_recipe!(recipe_compose_present, "compose-present");

    #[test]
    fn recipe_dockerfile_only_builds_a_compose_from_the_dockerfile() {
        let d = detect(&fixture("dockerfile-only")).expect("detect");
        let p = compose_for_dockerfile(&d, &options_for(&d, &[])).expect("plan");
        insta::assert_snapshot!("dockerfile-only", view(&p));
    }

    #[test]
    fn plan_without_a_stack_or_a_dockerfile_is_an_error() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let d = detect(tmp.path()).expect("detect");
        let err = plan(&d, &options_for(&d, &[])).expect_err("nothing to build");
        assert!(matches!(err, RulesError::Read(..)), "{err:?}");
    }

    #[test]
    fn compose_for_dockerfile_needs_a_dockerfile() {
        let d = detect(&fixture("go-mod")).expect("detect");
        let err = compose_for_dockerfile(&d, &options_for(&d, &[])).expect_err("no Dockerfile");
        assert!(
            matches!(&err, RulesError::Read(p, _) if p.ends_with("Dockerfile")),
            "{err:?}"
        );
    }

    #[test]
    fn compose_for_dockerfile_writes_neither_a_dockerfile_nor_a_dockerignore() {
        let d = detect(&fixture("node-next-pnpm")).expect("detect");
        let p = compose_for_dockerfile(&d, &options_for(&d, &[])).expect("plan");
        let paths: Vec<_> = p
            .files
            .iter()
            .map(|f| f.path.display().to_string())
            .collect();
        assert_eq!(paths, ["compose.yaml", ".env"]);
        assert!(plan_of("node-next-pnpm", &[])
            .files
            .iter()
            .any(|f| f.path == Path::new(".dockerignore")));
    }

    #[test]
    fn a_plan_never_creates_a_file_the_folder_has() {
        // Every fixture folder: a path that exists on disk is only ever the .env Modify.
        for name in [
            "compose-present",
            "dockerfile-only",
            "go-mod",
            "java-spring-maven",
            "node-next-pnpm",
            "node-vite-npm",
            "php-laravel",
            "python-django-poetry",
            "python-fastapi-uv",
            "ruby-rails",
            "rust-axum",
            "static-site",
        ] {
            let d = detect(&fixture(name)).expect("detect");
            let Ok(p) = plan(&d, &options_for(&d, &[])) else {
                continue;
            };
            for f in &p.files {
                let exists = fixture(name).join(&f.path).symlink_metadata().is_ok();
                if exists {
                    assert_eq!(
                        f.path,
                        Path::new(".env"),
                        "{name}: {} exists",
                        f.path.display()
                    );
                    assert_eq!(f.change, FileChange::Modify, "{name}");
                    assert!(f.old_sha256.is_some(), "{name}");
                } else {
                    assert_eq!(f.change, FileChange::Create, "{name}: {}", f.path.display());
                    assert!(f.old_sha256.is_none(), "{name}");
                }
            }
        }
    }

    #[test]
    fn existing_files_survive_writing_the_plan() {
        let (_tmp, dir) = copy_fixture("go-mod");
        std::fs::write(dir.join("Dockerfile"), "# mine\nFROM scratch\n").expect("write");
        std::fs::write(dir.join(".dockerignore"), "# mine\n").expect("write");
        std::fs::write(dir.join(".env"), "KEEP=1\n").expect("write");
        let d = detect(&dir).expect("detect");
        let p = plan(&d, &options_for(&d, &[])).expect("plan");

        assert!(file(&p, "Dockerfile").is_none());
        assert!(file(&p, ".dockerignore").is_none());
        let env = file(&p, ".env").expect(".env change");
        assert_eq!(env.change, FileChange::Modify);
        assert_eq!(env.old_sha256, Some(diff::sha256("KEEP=1\n")));
        assert!(env
            .new_text
            .as_deref()
            .is_some_and(|t| t.starts_with("KEEP=1\n")));

        write::apply(&p).expect("apply");
        assert_eq!(
            std::fs::read_to_string(dir.join("Dockerfile")).expect("read"),
            "# mine\nFROM scratch\n"
        );
        assert_eq!(
            std::fs::read_to_string(dir.join(".dockerignore")).expect("read"),
            "# mine\n"
        );
        assert!(std::fs::read_to_string(dir.join(".env"))
            .expect("read")
            .starts_with("KEEP=1\n"));
        assert!(dir.join("compose.yaml").is_file());
    }

    #[test]
    fn a_folder_with_a_compose_file_gets_an_empty_plan() {
        let (_tmp, dir) = copy_fixture("go-mod");
        std::fs::write(dir.join("docker-compose.yml"), "services: {}\n").expect("write");
        let d = detect(&dir).expect("detect");
        let p = plan(&d, &options_for(&d, &[])).expect("plan");
        assert!(p.files.is_empty());
        assert!(p
            .notes
            .iter()
            .any(|n| n.contains("already has a compose file")));
        assert_eq!(p.after_write, None);
    }

    #[test]
    fn an_existing_password_is_kept() {
        let (_tmp, dir) = copy_fixture("go-mod");
        let env = format!("{}={}\n", "POSTGRES_PASSWORD", "chosen-by-the-user");
        std::fs::write(dir.join(".env"), &env).expect("write");
        let d = detect(&dir).expect("detect");
        let p = plan(&d, &options_for(&d, &[])).expect("plan");
        // go-mod needs Postgres and Redis; Redis has no secret, so .env stays as the user wrote it.
        assert!(
            file(&p, ".env").is_none(),
            "an unchanged .env is not a change"
        );
        assert!(p.notes.iter().all(|n| !n.contains("POSTGRES_PASSWORD")));
        let compose = text(&p, "compose.yaml");
        assert!(compose.contains("${POSTGRES_PASSWORD:?set in .env}"));

        // With a second secret needed, the user's password stays and only the new key is added.
        let mut d2 = d.clone();
        d2.services_needed.push(ServiceGuess {
            kind: ServiceKind::Rabbitmq,
            reason: "test".to_string(),
        });
        let p2 = plan(&d2, &options_for(&d2, &[])).expect("plan");
        let new_env = text(&p2, ".env");
        assert!(new_env.starts_with(&env), "{new_env}");
        assert_eq!(new_env.matches("POSTGRES_PASSWORD").count(), 1);
        assert!(new_env.contains("RABBITMQ_PASSWORD="));
    }

    #[test]
    fn generated_passwords_are_24_alphanumeric_characters_and_only_in_env() {
        let p = plan_of("python-fastapi-uv", &[]);
        let s = secrets(&p);
        assert_eq!(
            s.iter().map(|(k, _)| k.as_str()).collect::<Vec<_>>(),
            ["POSTGRES_PASSWORD", "MEILI_MASTER_KEY"]
        );
        for (k, v) in &s {
            assert_eq!(v.chars().count(), 24, "{k}");
            assert!(v.chars().all(|c| c.is_ascii_alphanumeric()), "{k}");
            for other in ["compose.yaml", "Dockerfile", ".dockerignore"] {
                assert!(!text(&p, other).contains(v.as_str()), "{k} in {other}");
            }
            assert!(p.notes.iter().all(|n| !n.contains(v.as_str())));
        }
        let again = secrets(&plan_of("python-fastapi-uv", &[]));
        assert_ne!(s, again, "a second plan reuses the passwords");
    }

    #[test]
    fn compose_reads_every_variable_it_uses_from_env() {
        for name in [
            "node-next-pnpm",
            "python-fastapi-uv",
            "php-laravel",
            "java-spring-maven",
            "ruby-rails",
        ] {
            let (_tmp, dir) = copy_fixture(name);
            let d = detect(&dir).expect("detect");
            let p = plan(&d, &options_for(&d, &[])).expect("plan");
            write::apply(&p).expect("apply");
            let compose = std::fs::read_to_string(dir.join("compose.yaml")).expect("compose");
            let env = std::fs::read_to_string(dir.join(".env")).unwrap_or_default();
            let keys = templates::env_keys(&env);
            let vars = compose_edit::referenced_vars(&compose);
            assert!(!vars.is_empty(), "{name}");
            for v in vars {
                assert!(
                    keys.contains(&v),
                    "{name}: compose uses ${{{v}}} but .env lacks it"
                );
            }
        }
    }

    fn all_kinds() -> Vec<ServiceKind> {
        vec![
            ServiceKind::Postgres,
            ServiceKind::Mysql,
            ServiceKind::Mongo,
            ServiceKind::Redis,
            ServiceKind::Rabbitmq,
            ServiceKind::Meilisearch,
            ServiceKind::Minio,
            ServiceKind::Mailpit,
        ]
    }

    fn with_all_services(d: &Detection) -> Detection {
        Detection {
            services_needed: all_kinds()
                .into_iter()
                .map(|kind| ServiceGuess {
                    kind,
                    reason: "test".to_string(),
                })
                .collect(),
            ..d.clone()
        }
    }

    #[test]
    fn every_generated_service_has_a_healthcheck_and_the_app_waits_for_it() {
        let (_tmp, dir) = copy_fixture("go-mod");
        let d = with_all_services(&detect(&dir).expect("detect"));
        let p = plan(&d, &options_for(&d, &[])).expect("plan");
        let doc = yaml(&text(&p, "compose.yaml"));
        let services = doc
            .get("services")
            .and_then(|s| s.as_mapping())
            .expect("services");
        assert_eq!(services.len(), 9, "app and eight services");
        let depends = services
            .get("app")
            .and_then(|a| a.get("depends_on"))
            .and_then(|d| d.as_mapping())
            .expect("depends_on");
        for (name, svc) in services {
            let name = name.as_str().expect("name");
            if name == "app" {
                continue;
            }
            let hc = svc
                .get("healthcheck")
                .unwrap_or_else(|| panic!("{name} has no healthcheck"));
            assert!(hc.get("test").is_some(), "{name}");
            assert_eq!(
                depends
                    .get(name)
                    .and_then(|d| d.get("condition"))
                    .and_then(|c| c.as_str()),
                Some("service_healthy"),
                "{name}"
            );
        }
    }

    #[test]
    fn two_sql_databases_keep_their_own_names_and_urls() {
        let (_tmp, dir) = copy_fixture("go-mod");
        let d = with_all_services(&detect(&dir).expect("detect"));
        let p = plan(&d, &options_for(&d, &[])).expect("plan");
        let doc = yaml(&text(&p, "compose.yaml"));
        let env = doc
            .get("services")
            .and_then(|s| s.get("app"))
            .and_then(|a| a.get("environment"))
            .expect("environment");
        let url = |k: &str| env.get(k).and_then(|v| v.as_str()).map(str::to_string);
        assert_eq!(
            url("DATABASE_URL").as_deref(),
            Some("postgres://app:${POSTGRES_PASSWORD}@postgres:5432/app")
        );
        assert_eq!(
            url("MYSQL_URL").as_deref(),
            Some("mysql://app:${MYSQL_PASSWORD}@mysql:3306/app")
        );
        assert_eq!(url("REDIS_URL").as_deref(), Some("redis://redis:6379"));
        assert_eq!(url("SMTP_HOST").as_deref(), Some("mailpit"));
    }

    #[test]
    fn urls_use_container_hostnames_not_localhost() {
        let p = plan_of("node-next-pnpm", &[]);
        let compose = text(&p, "compose.yaml");
        let doc = yaml(&compose);
        let env = doc
            .get("services")
            .and_then(|s| s.get("app"))
            .and_then(|a| a.get("environment"))
            .expect("environment");
        assert_eq!(
            env.get("DATABASE_URL").and_then(|v| v.as_str()),
            Some("postgres://app:${POSTGRES_PASSWORD}@db:5432/app")
        );
        assert_eq!(
            env.get("REDIS_URL").and_then(|v| v.as_str()),
            Some("redis://redis:6379")
        );
        assert!(!compose.contains("localhost"));
    }

    #[test]
    fn node_and_python_get_watch_and_the_others_do_not() {
        let watch = |name: &str| {
            let p = plan_of(name, &[]);
            let doc = yaml(&text(&p, "compose.yaml"));
            doc.get("services")
                .and_then(|s| s.get("app"))
                .and_then(|a| a.get("develop"))
                .and_then(|d| d.get("watch"))
                .and_then(|w| w.as_sequence())
                .map(|w| {
                    w.iter()
                        .filter_map(|e| {
                            e.get("action").and_then(|a| a.as_str()).map(str::to_string)
                        })
                        .collect::<Vec<_>>()
                })
        };
        let sync_rebuild = Some(vec!["sync".to_string(), "rebuild".to_string()]);
        assert_eq!(watch("node-next-pnpm"), sync_rebuild);
        assert_eq!(
            watch("node-vite-npm"),
            Some(vec!["rebuild".to_string(), "rebuild".to_string()])
        );
        assert_eq!(watch("python-django-poetry"), sync_rebuild);
        assert_eq!(watch("python-fastapi-uv"), sync_rebuild);
        for other in [
            "go-mod",
            "rust-axum",
            "php-laravel",
            "ruby-rails",
            "java-spring-maven",
            "static-site",
        ] {
            assert_eq!(watch(other), None, "{other}");
        }
    }

    #[test]
    fn node_watch_syncs_src_when_the_folder_has_one() {
        let (_tmp, dir) = copy_fixture("node-next-pnpm");
        std::fs::create_dir(dir.join("src")).expect("mkdir");
        let d = detect(&dir).expect("detect");
        let p = plan(&d, &options_for(&d, &[])).expect("plan");
        let compose = text(&p, "compose.yaml");
        assert!(
            compose.contains("path: ./src\n          target: /app/src\n"),
            "{compose}"
        );
        let (_tmp2, dir2) = copy_fixture("node-next-pnpm");
        let d2 = detect(&dir2).expect("detect");
        let p2 = plan(&d2, &options_for(&d2, &[])).expect("plan");
        assert!(text(&p2, "compose.yaml").contains("path: .\n          target: /app\n"));
    }

    #[test]
    fn every_dockerfile_but_the_static_one_has_several_stages() {
        for name in [
            "node-next-pnpm",
            "node-vite-npm",
            "python-django-poetry",
            "python-fastapi-uv",
            "go-mod",
            "rust-axum",
            "php-laravel",
            "ruby-rails",
            "java-spring-maven",
        ] {
            let (_tmp, dir) = copy_fixture(name);
            std::fs::remove_file(dir.join("Dockerfile")).ok();
            let d = detect(&dir).expect("detect");
            let p = plan(&d, &options_for(&d, &[])).expect("plan");
            let df = text(&p, "Dockerfile");
            let froms = df.lines().filter(|l| l.starts_with("FROM ")).count();
            assert!(froms >= 2, "{name}: {froms} stage(s)");
            assert!(df.contains("EXPOSE "), "{name}");
        }
    }

    #[test]
    fn the_dockerignore_keeps_env_files_out_of_the_image() {
        let p = plan_of("go-mod", &[]);
        let ignore = text(&p, ".dockerignore");
        assert!(ignore.lines().any(|l| l == ".env"));
        assert!(ignore.lines().any(|l| l == ".env.*"));
        assert!(ignore.lines().any(|l| l == "!.env.example"));
    }

    #[test]
    fn options_for_offers_free_ports_and_the_node_version() {
        let d = detect(&fixture("node-next-pnpm")).expect("detect");
        let o = options_for(&d, &[5432, 6379, 3000]);
        assert_eq!(o.ports.get("app"), Some(&3001));
        assert_eq!(o.ports.get("db"), Some(&5433));
        assert_eq!(o.ports.get("redis"), Some(&6380));
        assert_eq!(o.node_version.as_deref(), Some("22"));
        assert_eq!(o.services, [ServiceKind::Postgres, ServiceKind::Redis]);

        let d = detect(&fixture("go-mod")).expect("detect");
        assert_eq!(options_for(&d, &[]).node_version, None);
        let d = detect(&fixture("node-vite-npm")).expect("detect");
        let o = options_for(&d, &[]);
        assert_eq!(
            o.node_version.as_deref(),
            Some("22"),
            "no .nvmrc: the default"
        );
        assert_eq!(
            o.ports.get("app"),
            Some(&8080),
            "container port 80 maps to 8080"
        );
    }

    #[test]
    fn options_for_gives_every_published_port_its_own_host_port() {
        let (_tmp, dir) = copy_fixture("go-mod");
        let d = with_all_services(&detect(&dir).expect("detect"));
        let o = options_for(&d, &[]);
        let mut seen: Vec<u16> = o.ports.values().copied().collect();
        let all = seen.len();
        seen.sort_unstable();
        seen.dedup();
        assert_eq!(seen.len(), all, "{:?}", o.ports);
        assert_eq!(o.ports.get("rabbitmq.management_port"), Some(&15672));
        assert_eq!(o.ports.get("mailpit.web_port"), Some(&8025));
        assert_eq!(o.ports.get("minio.console_port"), Some(&9001));
    }

    #[test]
    fn chosen_ports_and_services_reach_the_compose_file() {
        let d = detect(&fixture("node-next-pnpm")).expect("detect");
        let mut o = options_for(&d, &[]);
        o.ports.insert("app".to_string(), 3100);
        o.ports.insert("db".to_string(), 5544);
        o.services = vec![ServiceKind::Postgres];
        o.node_version = Some("20".to_string());
        let p = plan(&d, &o).expect("plan");
        let compose = text(&p, "compose.yaml");
        assert!(compose.contains("\"3100:3000\""));
        assert!(compose.contains("\"5544:5432\""));
        assert!(
            !compose.contains("redis"),
            "an unchecked service is not generated"
        );
    }

    #[test]
    fn the_node_version_picks_the_base_image() {
        let (_tmp, dir) = copy_fixture("node-vite-npm");
        let d = detect(&dir).expect("detect");
        let mut o = options_for(&d, &[]);
        o.node_version = Some("20".to_string());
        let df = text(&plan(&d, &o).expect("plan"), "Dockerfile");
        assert!(df.contains("FROM node:20-alpine AS deps"), "{df}");
    }

    #[test]
    fn the_existing_dockerfile_decides_the_app_port_and_workdir() {
        let (_tmp, dir) = copy_fixture("node-next-pnpm");
        std::fs::write(
            dir.join("Dockerfile"),
            "FROM node:22-alpine\nWORKDIR /srv/web\nEXPOSE 4000/tcp\nCMD [\"node\", \"x.js\"]\n",
        )
        .expect("write");
        std::fs::create_dir(dir.join("src")).expect("mkdir");
        let d = detect(&dir).expect("detect");
        let p = plan(&d, &options_for(&d, &[])).expect("plan");
        let compose = text(&p, "compose.yaml");
        assert!(compose.contains("\"4000:4000\""), "{compose}");
        assert!(compose.contains("target: /srv/web/src"), "{compose}");
        assert!(p.notes.iter().any(|n| n.contains("Reuses your Dockerfile")));
    }

    #[test]
    fn plans_apply_and_the_written_files_are_the_planned_text() {
        for name in [
            "node-vite-npm",
            "go-mod",
            "static-site",
            "python-django-poetry",
        ] {
            let (_tmp, dir) = copy_fixture(name);
            let d = detect(&dir).expect("detect");
            let p = plan(&d, &options_for(&d, &[])).expect("plan");
            let report = write::apply(&p).expect("apply");
            assert_eq!(report.written.len(), p.files.len(), "{name}");
            for f in &p.files {
                assert_eq!(
                    std::fs::read_to_string(dir.join(&f.path)).expect("read"),
                    f.new_text.clone().unwrap_or_default(),
                    "{name}: {}",
                    f.path.display()
                );
            }
            assert!(yaml(&text(&p, "compose.yaml")).get("services").is_some());
        }
    }

    #[test]
    fn a_plan_conflicts_when_env_changes_before_it_is_written() {
        let (_tmp, dir) = copy_fixture("node-next-pnpm");
        let d = detect(&dir).expect("detect");
        let p = plan(&d, &options_for(&d, &[])).expect("plan");
        let mut env = std::fs::read_to_string(dir.join(".env")).expect("read");
        env.push_str("EDITED=1\n");
        std::fs::write(dir.join(".env"), &env).expect("write");
        let err = write::apply(&p).expect_err("conflict");
        assert!(matches!(err, RulesError::Conflict(_)), "{err:?}");
        assert!(
            !dir.join("compose.yaml").exists(),
            "nothing is written on a conflict"
        );
    }

    #[test]
    fn scalar_quotes_what_yaml_would_misread() {
        assert_eq!(scalar("3306"), "\"3306\"");
        assert_eq!(scalar("true"), "\"true\"");
        assert_eq!(scalar("a: b"), "\"a: b\"");
        assert_eq!(scalar("redis://redis:6379"), "redis://redis:6379");
        assert_eq!(scalar("${POSTGRES_PASSWORD}"), "${POSTGRES_PASSWORD}");
    }

    #[test]
    fn project_names_are_valid_volume_prefixes() {
        assert_eq!(project_name(Path::new("/x/New App!")), "new-app");
        assert_eq!(project_name(Path::new("/x/shop_api")), "shop_api");
        assert_eq!(project_name(Path::new("/")), "app");
    }

    #[test]
    fn dockerfile_facts_take_the_last_expose_and_workdir() {
        let f = dockerfile_facts(
            "FROM a AS b\nWORKDIR /one\nEXPOSE 1\nFROM nginx:1\nWORKDIR /two/\nexpose 8080/tcp 9\n",
        );
        assert_eq!(f.expose, Some(8080));
        assert_eq!(f.workdir.as_deref(), Some("/two"));
        assert_eq!(f.from.as_deref(), Some("nginx:1"));
        assert_eq!(dockerfile_facts("FROM x\nWORKDIR $HOME\n").workdir, None);
    }

    /// Runs `docker compose config -q` in `dir`; no containers and no engine are needed.
    fn compose_config(dir: &Path) -> Result<(), String> {
        let out = std::process::Command::new("docker")
            .args(["compose", "config", "-q"])
            .current_dir(dir)
            .output()
            .map_err(|e| e.to_string())?;
        if out.status.success() {
            Ok(())
        } else {
            Err(String::from_utf8_lossy(&out.stderr).into_owned())
        }
    }

    /// `BERTH_IT=1 cargo nextest run -p berth-rules --run-ignored only recipe_it`
    #[test]
    #[ignore = "needs the docker CLI: BERTH_IT=1"]
    fn recipe_it() {
        if std::env::var("BERTH_IT").as_deref() != Ok("1") {
            eprintln!("skipped: set BERTH_IT=1");
            return;
        }
        let (_tmp, dir) = copy_fixture("node-next-pnpm");
        let d = detect(&dir).expect("detect");
        let p = plan(&d, &options_for(&d, &[])).expect("plan");
        write::apply(&p).expect("apply");
        assert!(dir.join("compose.yaml").is_file());
        compose_config(&dir).unwrap_or_else(|e| panic!("docker compose config failed: {e}"));
    }

    /// The same check for every other stack, and for a project with all eight services.
    #[test]
    #[ignore = "needs the docker CLI: BERTH_IT=1"]
    fn recipe_it_every_stack() {
        if std::env::var("BERTH_IT").as_deref() != Ok("1") {
            eprintln!("skipped: set BERTH_IT=1");
            return;
        }
        for name in [
            "node-vite-npm",
            "python-django-poetry",
            "python-fastapi-uv",
            "go-mod",
            "rust-axum",
            "php-laravel",
            "ruby-rails",
            "java-spring-maven",
            "static-site",
        ] {
            let (_tmp, dir) = copy_fixture(name);
            let d = detect(&dir).expect("detect");
            let p = plan(&d, &options_for(&d, &[])).expect("plan");
            write::apply(&p).expect("apply");
            compose_config(&dir)
                .unwrap_or_else(|e| panic!("{name}: docker compose config failed: {e}"));
        }
        let (_tmp, dir) = copy_fixture("go-mod");
        let d = with_all_services(&detect(&dir).expect("detect"));
        let p = plan(&d, &options_for(&d, &[])).expect("plan");
        write::apply(&p).expect("apply");
        compose_config(&dir)
            .unwrap_or_else(|e| panic!("all services: docker compose config failed: {e}"));
    }
}
