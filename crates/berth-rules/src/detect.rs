//! Folder detection (PLAN section 6.7, D36, D83).
//!
//! `detect` opens only the named manifest files in the folder itself plus `.git/HEAD`. It never lists a
//! directory, never walks into a subfolder and never opens a file outside that list, so unreadable or
//! private neighbours cannot cause an error. `.env` values are inspected here (for a scheme such as
//! `postgresql://`) but never leave this module: reasons name keys and dependencies only.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use berth_core::{
    Confidence, Detection, FoundFile, FoundKind, ServiceGuess, ServiceKind, StackGuess,
};
use regex::Regex;

use crate::RulesError;

const COMPOSE_FILES: [&str; 4] = [
    "compose.yaml",
    "compose.yml",
    "docker-compose.yml",
    "docker-compose.yaml",
];

/// Service kinds in the order `Detection::services_needed` lists them.
const SERVICE_ORDER: [ServiceKind; 8] = [
    ServiceKind::Postgres,
    ServiceKind::Mysql,
    ServiceKind::Mongo,
    ServiceKind::Redis,
    ServiceKind::Rabbitmq,
    ServiceKind::Meilisearch,
    ServiceKind::Minio,
    ServiceKind::Mailpit,
];

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FolderMarkers {
    pub compose: bool,
    pub dockerfile: bool,
}

/// Reads `folder` (one level) and only these files when present: compose.yaml, compose.yml, docker-compose.yml,
/// docker-compose.yaml, Dockerfile, .dockerignore, .env, .env.example, package.json, pnpm-lock.yaml, yarn.lock,
/// bun.lockb, package-lock.json, .nvmrc, pyproject.toml, requirements.txt, poetry.lock, uv.lock, manage.py, go.mod,
/// Cargo.toml, composer.json, artisan, Gemfile, pom.xml, build.gradle, build.gradle.kts, index.html, .git/HEAD.
///
/// Confidence: `High` when exactly one stack is found and either its framework is known (or it is Go or Rust)
/// and, for Node, the package manager comes from a lockfile; `Medium` for any other stack, or for no stack but a
/// Dockerfile or Compose file; `Low` otherwise.
///
/// # Errors
/// `RulesError::Read` when `folder` is not a readable folder or a listed file exists but cannot be read.
pub fn detect(folder: &Path) -> Result<Detection, RulesError> {
    let files = Files { folder };
    files.check_folder()?;

    let found = found_files(&files)?;
    let mut stacks = Vec::new();
    let mut sure = Vec::new();
    let mut deps: Vec<Dep> = Vec::new();

    if let Some((stack, certain, d)) = node(&files)? {
        stacks.push(stack);
        sure.push(certain);
        deps.extend(d);
    }
    if let Some((stack, certain, d)) = python(&files)? {
        stacks.push(stack);
        sure.push(certain);
        deps.extend(d);
    }
    if let Some(d) = go(&files)? {
        stacks.push(StackGuess::Go);
        sure.push(true);
        deps.extend(d);
    }
    if let Some(d) = rust(&files)? {
        stacks.push(StackGuess::Rust);
        sure.push(true);
        deps.extend(d);
    }
    if let Some((stack, certain, d)) = php(&files)? {
        stacks.push(stack);
        sure.push(certain);
        deps.extend(d);
    }
    if let Some((stack, certain, d)) = ruby(&files)? {
        stacks.push(stack);
        sure.push(certain);
        deps.extend(d);
    }
    if let Some((stack, certain, d)) = java(&files)? {
        stacks.push(stack);
        sure.push(certain);
        deps.extend(d);
    }
    if stacks.is_empty() && files.exists("index.html") {
        stacks.push(StackGuess::Static);
        sure.push(false);
    }

    let services_needed = services(&files, &deps)?;
    let confidence = if stacks.len() == 1 && sure.first() == Some(&true) {
        Confidence::High
    } else if !stacks.is_empty()
        || found
            .iter()
            .any(|f| matches!(f.kind, FoundKind::Compose | FoundKind::Dockerfile { .. }))
    {
        Confidence::Medium
    } else {
        Confidence::Low
    };

    Ok(Detection {
        folder: folder.to_path_buf(),
        found,
        stack: stacks,
        services_needed,
        confidence,
        git_branch: git_branch(&files)?,
    })
}

/// For the folder browser: compose / dockerfile flags only.
#[must_use]
pub fn markers(folder: &Path) -> FolderMarkers {
    let files = Files { folder };
    FolderMarkers {
        compose: COMPOSE_FILES.iter().any(|n| files.exists(n)),
        dockerfile: files.exists("Dockerfile"),
    }
}

// ---------------------------------------------------------------------------------------------
// File access: the only place that touches the disk.

struct Files<'a> {
    folder: &'a Path,
}

impl Files<'_> {
    fn path(&self, name: &str) -> PathBuf {
        self.folder.join(name)
    }

    fn check_folder(&self) -> Result<(), RulesError> {
        match std::fs::metadata(self.folder) {
            Ok(m) if m.is_dir() => Ok(()),
            Ok(_) => Err(RulesError::Read(
                self.folder.to_path_buf(),
                "not a folder".to_string(),
            )),
            Err(e) => Err(RulesError::Read(self.folder.to_path_buf(), e.to_string())),
        }
    }

    /// A regular file exists. Needs no read permission on the file.
    fn exists(&self, name: &str) -> bool {
        std::fs::metadata(self.path(name)).is_ok_and(|m| m.is_file())
    }

    /// `None` when the file is absent; an error when it exists and cannot be read.
    fn read(&self, name: &str) -> Result<Option<String>, RulesError> {
        let path = self.path(name);
        match std::fs::read(&path) {
            Ok(bytes) => Ok(Some(String::from_utf8_lossy(&bytes).into_owned())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) if e.kind() == std::io::ErrorKind::NotADirectory => Ok(None),
            Err(e) => Err(RulesError::Read(path, e.to_string())),
        }
    }
}

fn found_files(files: &Files<'_>) -> Result<Vec<FoundFile>, RulesError> {
    let mut found = Vec::new();
    for name in COMPOSE_FILES {
        if files.exists(name) {
            found.push(FoundFile {
                kind: FoundKind::Compose,
                path: files.path(name),
            });
        }
    }
    if let Some(text) = files.read("Dockerfile")? {
        found.push(FoundFile {
            kind: FoundKind::Dockerfile {
                from: dockerfile_from(&text),
            },
            path: files.path("Dockerfile"),
        });
    }
    if let Some(text) = files.read(".env")? {
        let keys = u32::try_from(env_pairs(&text).len()).unwrap_or(u32::MAX);
        found.push(FoundFile {
            kind: FoundKind::Env { keys },
            path: files.path(".env"),
        });
    }
    if files.exists(".env.example") {
        found.push(FoundFile {
            kind: FoundKind::EnvExample,
            path: files.path(".env.example"),
        });
    }
    if files.exists(".dockerignore") {
        found.push(FoundFile {
            kind: FoundKind::DockerIgnore,
            path: files.path(".dockerignore"),
        });
    }
    Ok(found)
}

fn dockerfile_from(text: &str) -> Option<String> {
    let re = Regex::new(r"(?im)^\s*FROM\s+(?:--platform=\S+\s+)?(\S+)").ok()?;
    re.captures(text)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().to_string())
}

fn git_branch(files: &Files<'_>) -> Result<Option<String>, RulesError> {
    Ok(files.read(".git/HEAD")?.and_then(|text| {
        text.trim()
            .strip_prefix("ref: refs/heads/")
            .map(str::to_string)
    }))
}

// ---------------------------------------------------------------------------------------------
// Dependencies and the services they imply.

/// A dependency name (lower case) and the file it was found in.
#[derive(Debug, Clone)]
struct Dep {
    name: String,
    file: &'static str,
}

fn deps_of(names: impl IntoIterator<Item = String>, file: &'static str) -> Vec<Dep> {
    names
        .into_iter()
        .map(|n| Dep {
            name: n.to_lowercase().replace('_', "-"),
            file,
        })
        .collect()
}

/// Dependency names (exact, or a module path prefix such as `github.com/jackc/pgx`) per service.
const DEP_TABLE: [(ServiceKind, &[&str]); 8] = [
    (
        ServiceKind::Postgres,
        &[
            "pg",
            "postgres",
            "pg-promise",
            "psycopg",
            "psycopg2",
            "psycopg2-binary",
            "psycopg-binary",
            "asyncpg",
            "github.com/lib/pq",
            "github.com/jackc/pgx",
            "tokio-postgres",
            "sqlx[postgres]",
            "diesel[postgres]",
            "postgresql",
        ],
    ),
    (
        ServiceKind::Mysql,
        &[
            "mysql",
            "mysql2",
            "mysqlclient",
            "pymysql",
            "mysql-connector-python",
            "github.com/go-sql-driver/mysql",
            "mysql-async",
            "sqlx[mysql]",
            "diesel[mysql]",
            "mysql-connector-j",
            "mysql-connector-java",
        ],
    ),
    (
        ServiceKind::Mongo,
        &[
            "mongodb",
            "mongoose",
            "pymongo",
            "motor",
            "mongoengine",
            "go.mongodb.org/mongo-driver",
            "mongoid",
            "mongo",
            "mongodb/mongodb",
            "mongodb-driver-sync",
            "spring-boot-starter-data-mongodb",
        ],
    ),
    (
        ServiceKind::Redis,
        &[
            "redis",
            "ioredis",
            "bullmq",
            "bull",
            "github.com/redis/go-redis",
            "github.com/go-redis/redis",
            "github.com/redis/rueidis",
            "predis/predis",
            "jedis",
            "lettuce-core",
            "spring-boot-starter-data-redis",
        ],
    ),
    (
        ServiceKind::Rabbitmq,
        &[
            "amqplib",
            "pika",
            "aio-pika",
            "github.com/rabbitmq/amqp091-go",
            "github.com/streadway/amqp",
            "lapin",
            "php-amqplib/php-amqplib",
            "bunny",
            "spring-boot-starter-amqp",
            "amqp-client",
        ],
    ),
    (
        ServiceKind::Meilisearch,
        &[
            "meilisearch",
            "github.com/meilisearch/meilisearch-go",
            "meilisearch-sdk",
            "meilisearch/meilisearch-php",
            "meilisearch-ruby",
        ],
    ),
    (
        ServiceKind::Minio,
        &["minio", "github.com/minio/minio-go", "minio-java"],
    ),
    (ServiceKind::Mailpit, &["nodemailer"]),
];

fn service_for_dep(name: &str) -> Option<ServiceKind> {
    DEP_TABLE.iter().find_map(|(kind, names)| {
        names
            .iter()
            .any(|n| name == *n || name.strip_prefix(n).is_some_and(|r| r.starts_with('/')))
            .then_some(*kind)
    })
}

/// `.env` key and value rules. Returns the kind and the key that names it.
fn service_for_env(key: &str, value: &str) -> Option<ServiceKind> {
    let k = key.to_uppercase();
    let v = value.to_lowercase();
    let scheme = |s: &[&str]| s.iter().any(|p| v.starts_with(p));
    let pre = |p: &[&str]| p.iter().any(|p| k.starts_with(p));
    if scheme(&["postgres://", "postgresql://", "postgres+", "postgresql+"])
        || pre(&["POSTGRES_", "PGHOST", "PGUSER", "PGDATABASE"])
        || (k == "DB_CONNECTION" && matches!(v.as_str(), "pgsql" | "postgres" | "postgresql"))
    {
        Some(ServiceKind::Postgres)
    } else if scheme(&["mysql://", "mysql2://", "mysql+", "mariadb://"])
        || pre(&["MYSQL_"])
        || (k == "DB_CONNECTION" && matches!(v.as_str(), "mysql" | "mariadb"))
    {
        Some(ServiceKind::Mysql)
    } else if scheme(&["mongodb://", "mongodb+srv://"]) || pre(&["MONGO_", "MONGODB_"]) {
        Some(ServiceKind::Mongo)
    } else if scheme(&["redis://", "rediss://"]) || pre(&["REDIS_"]) {
        Some(ServiceKind::Redis)
    } else if scheme(&["amqp://", "amqps://"]) || pre(&["RABBITMQ_", "AMQP_"]) {
        Some(ServiceKind::Rabbitmq)
    } else if pre(&["MEILI_", "MEILISEARCH_"]) {
        Some(ServiceKind::Meilisearch)
    } else if pre(&["MINIO_"]) {
        Some(ServiceKind::Minio)
    } else if pre(&["MAIL_HOST", "SMTP_HOST", "EMAIL_HOST"]) {
        Some(ServiceKind::Mailpit)
    } else {
        None
    }
}

/// Services from `.env`, then `.env.example`, then dependencies. The first reason per kind wins.
fn services(files: &Files<'_>, deps: &[Dep]) -> Result<Vec<ServiceGuess>, RulesError> {
    let mut reasons: Vec<(ServiceKind, String)> = Vec::new();
    let mut note = |kind: ServiceKind, reason: String| {
        if !reasons.iter().any(|(k, _)| *k == kind) {
            reasons.push((kind, reason));
        }
    };
    for name in [".env", ".env.example"] {
        if let Some(text) = files.read(name)? {
            for (key, value) in env_pairs(&text) {
                if let Some(kind) = service_for_env(&key, &value) {
                    note(kind, format!("from {key}"));
                }
            }
        }
    }
    for dep in deps {
        if let Some(kind) = service_for_dep(&dep.name) {
            note(kind, format!("from {} in {}", dep.name, dep.file));
        }
    }
    Ok(SERVICE_ORDER
        .iter()
        .filter_map(|kind| {
            reasons
                .iter()
                .find(|(k, _)| k == kind)
                .map(|(_, reason)| ServiceGuess {
                    kind: *kind,
                    reason: reason.clone(),
                })
        })
        .collect())
}

/// Distinct `KEY=value` pairs in file order. Comments and blank lines are skipped, `export ` is allowed.
fn env_pairs(text: &str) -> Vec<(String, String)> {
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let line = line.strip_prefix("export ").unwrap_or(line).trim_start();
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        if key.is_empty() || key.contains(char::is_whitespace) || !seen.insert(key.to_string()) {
            continue;
        }
        let value = value.trim().trim_matches(|c| c == '"' || c == '\'');
        out.push((key.to_string(), value.to_string()));
    }
    out
}

// ---------------------------------------------------------------------------------------------
// Stacks. Each returns the guess, whether it is certain enough for High confidence, and its dependencies.

type Found<T> = Result<Option<(T, bool, Vec<Dep>)>, RulesError>;

/// Keys of the named top-level objects of a JSON document. JSON is read as YAML flow syntax (no extra
/// dependency); when that fails the document's quoted keys are used instead.
fn json_sections(text: &str, sections: &[&str]) -> (Vec<String>, Option<serde_norway::Value>) {
    match serde_norway::from_str::<serde_norway::Value>(text) {
        Ok(doc) => {
            let mut names = Vec::new();
            for section in sections {
                if let Some(map) = doc.get(*section).and_then(|v| v.as_mapping()) {
                    names.extend(map.keys().filter_map(|k| k.as_str().map(str::to_string)));
                }
            }
            (names, Some(doc))
        }
        Err(_) => {
            let names = Regex::new(r#""([^"\s]+)"\s*:"#).map_or_else(
                |_| Vec::new(),
                |re| {
                    re.captures_iter(text)
                        .filter_map(|c| c.get(1).map(|m| m.as_str().to_string()))
                        .collect()
                },
            );
            (names, None)
        }
    }
}

fn node(files: &Files<'_>) -> Found<StackGuess> {
    let Some(text) = files.read("package.json")? else {
        return Ok(None);
    };
    let (names, doc) = json_sections(&text, &["dependencies", "devDependencies"]);
    let has = |n: &str| names.iter().any(|d| d == n);

    let from_lock = if files.exists("pnpm-lock.yaml") {
        Some("pnpm")
    } else if files.exists("yarn.lock") {
        Some("yarn")
    } else if files.exists("bun.lockb") {
        Some("bun")
    } else if files.exists("package-lock.json") {
        Some("npm")
    } else {
        None
    };
    let from_field = doc
        .as_ref()
        .and_then(|d| d.get("packageManager"))
        .and_then(|v| v.as_str())
        .and_then(|s| s.split('@').next())
        .filter(|s| matches!(*s, "pnpm" | "yarn" | "bun" | "npm"))
        .map(str::to_string);
    let known_pm = from_lock.is_some() || from_field.is_some();
    let pm = from_lock
        .map(str::to_string)
        .or(from_field)
        .unwrap_or_else(|| "npm".to_string());

    let framework = if has("next") {
        Some("Next.js")
    } else if has("vite") {
        Some("Vite")
    } else if has("express") {
        Some("Express")
    } else {
        None
    }
    .map(str::to_string);

    let version = files
        .read(".nvmrc")?
        .map(|v| v.trim().trim_start_matches('v').to_string())
        .filter(|v| v.starts_with(|c: char| c.is_ascii_digit()));

    let certain = framework.is_some() && known_pm;
    Ok(Some((
        StackGuess::Node {
            pm,
            framework,
            version,
        },
        certain,
        deps_of(names, "package.json"),
    )))
}

/// Name before a version specifier or extras: `Django[argon2]>=5; python_version>"3"` gives `django`.
fn requirement_name(spec: &str) -> Option<String> {
    let end = spec
        .find(|c: char| !(c.is_alphanumeric() || matches!(c, '-' | '_' | '.')))
        .unwrap_or(spec.len());
    let name = spec[..end].trim().replace(['_', '.'], "-").to_lowercase();
    (!name.is_empty()).then_some(name)
}

fn python(files: &Files<'_>) -> Found<StackGuess> {
    let pyproject = files.read("pyproject.toml")?;
    let requirements = files.read("requirements.txt")?;
    let manage = files.exists("manage.py");
    if pyproject.is_none()
        && requirements.is_none()
        && !manage
        && !files.exists("poetry.lock")
        && !files.exists("uv.lock")
    {
        return Ok(None);
    }

    let mut deps: Vec<Dep> = Vec::new();
    let mut poetry = files.exists("poetry.lock");
    let mut uv = files.exists("uv.lock");
    if let Some(text) = &pyproject {
        if let Ok(doc) = text.parse::<toml::Table>() {
            poetry |= doc.get("tool").and_then(|t| t.get("poetry")).is_some();
            uv |= doc.get("tool").and_then(|t| t.get("uv")).is_some();
            deps.extend(deps_of(pyproject_names(&doc), "pyproject.toml"));
        }
    }
    if let Some(text) = &requirements {
        let names = text
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with(['#', '-']))
            .filter_map(requirement_name);
        deps.extend(deps_of(names, "requirements.txt"));
    }
    let has = |n: &str| deps.iter().any(|d| d.name == n);

    let tool = if poetry {
        "poetry"
    } else if uv {
        "uv"
    } else {
        "pip"
    };
    let framework = if manage || has("django") {
        Some("Django")
    } else if has("fastapi") {
        Some("FastAPI")
    } else if has("flask") {
        Some("Flask")
    } else {
        None
    }
    .map(str::to_string);

    let certain = framework.is_some();
    Ok(Some((
        StackGuess::Python {
            tool: tool.to_string(),
            framework,
        },
        certain,
        deps,
    )))
}

fn pyproject_names(doc: &toml::Table) -> Vec<String> {
    let mut names = Vec::new();
    let mut strings = |v: Option<&toml::Value>| {
        if let Some(arr) = v.and_then(|v| v.as_array()) {
            names.extend(
                arr.iter()
                    .filter_map(|s| s.as_str())
                    .filter_map(requirement_name),
            );
        }
    };
    strings(doc.get("project").and_then(|p| p.get("dependencies")));
    if let Some(groups) = doc.get("dependency-groups").and_then(|g| g.as_table()) {
        for group in groups.values() {
            strings(Some(group));
        }
    }
    if let Some(opt) = doc
        .get("project")
        .and_then(|p| p.get("optional-dependencies"))
        .and_then(|g| g.as_table())
    {
        for group in opt.values() {
            strings(Some(group));
        }
    }
    let poetry = doc.get("tool").and_then(|t| t.get("poetry"));
    let mut tables = vec![poetry.and_then(|p| p.get("dependencies"))];
    if let Some(groups) = poetry
        .and_then(|p| p.get("group"))
        .and_then(|g| g.as_table())
    {
        tables.extend(groups.values().map(|g| g.get("dependencies")));
    }
    for t in tables.into_iter().flatten() {
        if let Some(t) = t.as_table() {
            names.extend(
                t.keys()
                    .filter(|k| k.as_str() != "python")
                    .filter_map(|k| requirement_name(k)),
            );
        }
    }
    names
}

fn go(files: &Files<'_>) -> Result<Option<Vec<Dep>>, RulesError> {
    let Some(text) = files.read("go.mod")? else {
        return Ok(None);
    };
    let names = Regex::new(
        r"(?m)^\s*(?:require\s+)?([A-Za-z0-9.\-]+\.[A-Za-z0-9.\-]+/[A-Za-z0-9._\-/]+)\s+v\d",
    )
    .map_or_else(
        |_| Vec::new(),
        |re| {
            re.captures_iter(&text)
                .filter_map(|c| c.get(1).map(|m| m.as_str().to_string()))
                .collect()
        },
    );
    Ok(Some(deps_of(names, "go.mod")))
}

fn rust(files: &Files<'_>) -> Result<Option<Vec<Dep>>, RulesError> {
    let Some(text) = files.read("Cargo.toml")? else {
        return Ok(None);
    };
    let mut names = Vec::new();
    if let Ok(doc) = text.parse::<toml::Table>() {
        for section in ["dependencies", "dev-dependencies"] {
            let Some(table) = doc.get(section).and_then(|t| t.as_table()) else {
                continue;
            };
            for (name, spec) in table {
                names.push(name.clone());
                let features = spec.get("features").and_then(|f| f.as_array());
                for feature in features.into_iter().flatten().filter_map(|f| f.as_str()) {
                    names.push(format!("{name}[{feature}]"));
                }
            }
        }
    }
    Ok(Some(deps_of(names, "Cargo.toml")))
}

fn php(files: &Files<'_>) -> Found<StackGuess> {
    let composer = files.read("composer.json")?;
    let artisan = files.exists("artisan");
    if composer.is_none() && !artisan {
        return Ok(None);
    }
    let names = composer
        .as_deref()
        .map(|t| json_sections(t, &["require", "require-dev"]).0)
        .unwrap_or_default();
    let framework =
        (artisan || names.iter().any(|n| n == "laravel/framework")).then(|| "Laravel".to_string());
    let certain = framework.is_some();
    Ok(Some((
        StackGuess::Php { framework },
        certain,
        deps_of(names, "composer.json"),
    )))
}

fn ruby(files: &Files<'_>) -> Found<StackGuess> {
    let Some(text) = files.read("Gemfile")? else {
        return Ok(None);
    };
    let names: Vec<String> = Regex::new(r#"(?m)^\s*gem\s+['"]([^'"]+)['"]"#).map_or_else(
        |_| Vec::new(),
        |re| {
            re.captures_iter(&text)
                .filter_map(|c| c.get(1).map(|m| m.as_str().to_string()))
                .collect()
        },
    );
    let framework = names
        .iter()
        .any(|n| n == "rails")
        .then(|| "Rails".to_string());
    let certain = framework.is_some();
    Ok(Some((
        StackGuess::Ruby { framework },
        certain,
        deps_of(names, "Gemfile"),
    )))
}

fn java(files: &Files<'_>) -> Found<StackGuess> {
    let candidates = [
        ("pom.xml", "Maven"),
        ("build.gradle", "Gradle"),
        ("build.gradle.kts", "Gradle"),
    ];
    for (file, build) in candidates {
        let Some(text) = files.read(file)? else {
            continue;
        };
        let pattern = if file == "pom.xml" {
            r"<artifactId>\s*([^<\s]+)\s*</artifactId>"
        } else {
            r#"['"][\w.\-]+:([\w.\-]+)(?::[^'"]*)?['"]"#
        };
        let names: Vec<String> = Regex::new(pattern).map_or_else(
            |_| Vec::new(),
            |re| {
                re.captures_iter(&text)
                    .filter_map(|c| c.get(1).map(|m| m.as_str().to_string()))
                    .collect()
            },
        );
        let framework = (text.contains("spring-boot")).then(|| "Spring Boot".to_string());
        let certain = framework.is_some();
        return Ok(Some((
            StackGuess::Java {
                build: build.to_string(),
                framework,
            },
            certain,
            deps_of(names, file),
        )));
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;

    use super::*;

    fn fixture(name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/folders")
            .join(name)
    }

    /// The snapshot view: paths relative to the folder so the snapshot does not hold this Mac's paths.
    fn view(folder: &Path) -> Detection {
        let mut d = detect(folder).expect("detect");
        d.folder = PathBuf::from("<folder>");
        for f in &mut d.found {
            f.path = f.path.strip_prefix(folder).expect("inside").to_path_buf();
        }
        d
    }

    macro_rules! snapshot_fixture {
        ($test:ident, $name:literal) => {
            #[test]
            fn $test() {
                insta::assert_yaml_snapshot!($name, view(&fixture($name)));
            }
        };
    }

    snapshot_fixture!(detect_node_next_pnpm, "node-next-pnpm");
    snapshot_fixture!(detect_node_vite_npm, "node-vite-npm");
    snapshot_fixture!(detect_python_django_poetry, "python-django-poetry");
    snapshot_fixture!(detect_python_fastapi_uv, "python-fastapi-uv");
    snapshot_fixture!(detect_go_mod, "go-mod");
    snapshot_fixture!(detect_rust_axum, "rust-axum");
    snapshot_fixture!(detect_php_laravel, "php-laravel");
    snapshot_fixture!(detect_ruby_rails, "ruby-rails");
    snapshot_fixture!(detect_java_spring_maven, "java-spring-maven");
    snapshot_fixture!(detect_static_site, "static-site");
    snapshot_fixture!(detect_dockerfile_only, "dockerfile-only");
    snapshot_fixture!(detect_compose_present, "compose-present");
    snapshot_fixture!(detect_empty, "empty");

    fn write(dir: &Path, rel: &str, text: &str) {
        let path = dir.join(rel);
        std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        std::fs::write(path, text).expect("write");
    }

    fn deny(path: &Path) {
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o000)).expect("chmod");
    }

    fn can_read(path: &Path) -> bool {
        std::fs::read(path).is_ok()
    }

    #[test]
    fn detect_reads_no_file_outside_the_list() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path();
        write(root, "package.json", r#"{"dependencies":{"express":"4"}}"#);
        write(root, "package-lock.json", "{}");
        let before = detect(root).expect("detect");

        // Sentinels nobody may open: a loose file, files in subfolders, and a whole closed folder.
        let sentinels = [
            "notes.txt",
            "secrets/api.key",
            "sub/.env",
            "sub/package.json",
            "node_modules/pg/index.js",
            ".git/config",
            ".git/refs/heads/main",
        ];
        for s in sentinels {
            write(root, s, "sentinel");
            deny(&root.join(s));
        }
        deny(&root.join("secrets"));
        deny(&root.join("node_modules"));
        let probe = root.join("notes.txt");
        let enforced = !can_read(&probe);

        let after = detect(root);
        let marks = markers(root);

        // Restore so the temp dir can be cleaned.
        for dir_name in ["secrets", "node_modules"] {
            std::fs::set_permissions(root.join(dir_name), std::fs::Permissions::from_mode(0o755))
                .expect("chmod");
        }
        let after = after.expect("detect must not fail on unreadable neighbours");
        assert_eq!(after, before, "unlisted files must not change the result");
        assert_eq!(marks, FolderMarkers::default());
        // The sentinel only proves something when this process cannot read it (not when run as root).
        assert!(enforced, "sentinel files must be unreadable for this test");
    }

    #[test]
    fn detect_does_not_look_in_subfolders() {
        let dir = tempfile::tempdir().expect("tempdir");
        write(
            dir.path(),
            "app/package.json",
            r#"{"dependencies":{"next":"15"}}"#,
        );
        write(dir.path(), "app/Dockerfile", "FROM node:22\n");
        let d = detect(dir.path()).expect("detect");
        assert!(d.stack.is_empty());
        assert!(d.found.is_empty());
        assert_eq!(d.confidence, Confidence::Low);
    }

    #[test]
    fn detect_reports_an_unreadable_listed_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        write(dir.path(), ".env", "A=1\n");
        deny(&dir.path().join(".env"));
        let err = detect(dir.path()).expect_err("listed file unreadable");
        std::fs::set_permissions(
            dir.path().join(".env"),
            std::fs::Permissions::from_mode(0o600),
        )
        .expect("chmod");
        assert!(matches!(err, RulesError::Read(p, _) if p.ends_with(".env")));
    }

    #[test]
    fn detect_errors_when_the_folder_is_missing_or_a_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        assert!(matches!(
            detect(&dir.path().join("nope")),
            Err(RulesError::Read(..))
        ));
        write(dir.path(), "file.txt", "x");
        assert!(matches!(
            detect(&dir.path().join("file.txt")),
            Err(RulesError::Read(..))
        ));
    }

    #[test]
    fn detect_markers_flag_compose_and_dockerfile_only() {
        let dir = tempfile::tempdir().expect("tempdir");
        assert_eq!(markers(dir.path()), FolderMarkers::default());
        write(dir.path(), "docker-compose.yaml", "services: {}\n");
        assert_eq!(
            markers(dir.path()),
            FolderMarkers {
                compose: true,
                dockerfile: false
            }
        );
        write(dir.path(), "Dockerfile", "FROM alpine\n");
        assert_eq!(
            markers(dir.path()),
            FolderMarkers {
                compose: true,
                dockerfile: true
            }
        );
        assert_eq!(
            markers(&dir.path().join("missing")),
            FolderMarkers::default()
        );
    }

    #[test]
    fn detect_reasons_name_keys_and_never_values() {
        let dir = tempfile::tempdir().expect("tempdir");
        write(
            dir.path(),
            ".env",
            "DATABASE_URL=postgresql://app:hunter2-fake@db:5432/app\nREDIS_URL=redis://:topsecret-fake@cache:6379\n",
        );
        let d = detect(dir.path()).expect("detect");
        let reasons: Vec<_> = d
            .services_needed
            .iter()
            .map(|s| s.reason.as_str())
            .collect();
        assert_eq!(reasons, ["from DATABASE_URL", "from REDIS_URL"]);
        let all = format!("{d:?}");
        assert!(!all.contains("hunter2") && !all.contains("topsecret"));
        assert!(matches!(d.found[0].kind, FoundKind::Env { keys: 2 }));
    }

    #[test]
    fn detect_env_wins_over_dependency_for_the_reason() {
        let dir = tempfile::tempdir().expect("tempdir");
        write(dir.path(), "package.json", r#"{"dependencies":{"pg":"8"}}"#);
        write(
            dir.path(),
            ".env.example",
            "DATABASE_URL=postgres://u@h/db\n",
        );
        let d = detect(dir.path()).expect("detect");
        assert_eq!(d.services_needed.len(), 1);
        assert_eq!(d.services_needed[0].reason, "from DATABASE_URL");
    }

    #[test]
    fn detect_node_package_manager_and_version_sources() {
        let dir = tempfile::tempdir().expect("tempdir");
        // Tab-indented JSON, a packageManager field and no lockfile.
        write(
            dir.path(),
            "package.json",
            "{\n\t\"packageManager\": \"yarn@4.1.0\",\n\t\"devDependencies\": {\n\t\t\"vite\": \"^5\"\n\t}\n}\n",
        );
        write(dir.path(), ".nvmrc", "v20.11.0\n");
        let d = detect(dir.path()).expect("detect");
        assert_eq!(
            d.stack,
            [StackGuess::Node {
                pm: "yarn".into(),
                framework: Some("Vite".into()),
                version: Some("20.11.0".into())
            }]
        );
        assert_eq!(d.confidence, Confidence::High);

        // No lockfile and no field: npm, but only Medium confidence.
        let bare = tempfile::tempdir().expect("tempdir");
        write(
            bare.path(),
            "package.json",
            r#"{"dependencies":{"next":"15"}}"#,
        );
        let d = detect(bare.path()).expect("detect");
        assert!(matches!(&d.stack[0], StackGuess::Node { pm, .. } if pm == "npm"));
        assert_eq!(d.confidence, Confidence::Medium);
    }

    #[test]
    fn detect_gradle_and_git_branch() {
        let dir = tempfile::tempdir().expect("tempdir");
        write(
            dir.path(),
            "build.gradle.kts",
            "plugins { id(\"org.springframework.boot\") version \"3.3.0\" }\ndependencies {\n  implementation(\"org.springframework.boot:spring-boot-starter-data-redis\")\n  runtimeOnly(\"org.postgresql:postgresql\")\n}\n",
        );
        write(dir.path(), ".git/HEAD", "ref: refs/heads/feature/x\n");
        let d = detect(dir.path()).expect("detect");
        assert_eq!(
            d.stack,
            [StackGuess::Java {
                build: "Gradle".into(),
                framework: Some("Spring Boot".into())
            }]
        );
        let kinds: Vec<_> = d.services_needed.iter().map(|s| s.kind).collect();
        assert_eq!(kinds, [ServiceKind::Postgres, ServiceKind::Redis]);
        assert_eq!(d.git_branch.as_deref(), Some("feature/x"));

        write(dir.path(), ".git/HEAD", "3f2a9c0d1e\n");
        assert_eq!(detect(dir.path()).expect("detect").git_branch, None);
    }

    #[test]
    fn detect_dockerfile_from_skips_platform_flag() {
        let text = "# syntax=docker/dockerfile:1\nARG V=1\nFROM --platform=linux/amd64 node:22-alpine AS build\nFROM nginx\n";
        assert_eq!(dockerfile_from(text).as_deref(), Some("node:22-alpine"));
        assert_eq!(dockerfile_from("RUN echo FROM x\n"), None);
    }
}
