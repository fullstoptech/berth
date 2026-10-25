//! Built-in service templates (D84) and Add service plans.
//!
//! A template is a compose service body with `${INPUT}` placeholders. `render` fills the placeholders that
//! name one of the template's inputs and leaves every other `${...}` alone, so `${POSTGRES_PASSWORD:?...}`
//! reaches compose untouched: the generated password lives in `.env` and never in `compose.yaml`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use berth_core::{
    ComposeCommand, GenerationPlan, PlanSource, Template, TemplateInput, TemplateInputKind,
    TemplateKind,
};

use crate::{compose_edit, diff, RulesError};

const PASSWORD_LEN: usize = 24;
const ALPHABET: &[u8; 62] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";

/// A template with every input resolved: the service text, the `.env` lines and the volumes it declares.
pub(crate) struct Rendered {
    pub service: String,
    pub env: Vec<(String, String)>,
    pub volumes: Vec<String>,
}

fn input(key: &str, label: &str, kind: TemplateInputKind) -> TemplateInput {
    TemplateInput {
        key: key.to_string(),
        label: label.to_string(),
        kind,
    }
}

fn port(key: &str, label: &str, default: u16) -> TemplateInput {
    input(key, label, TemplateInputKind::Port { default })
}

fn password() -> TemplateInput {
    input("PASSWORD", "Password", TemplateInputKind::Password)
}

fn volume(default: &str) -> TemplateInput {
    input(
        "VOLUME",
        "Data volume",
        TemplateInputKind::VolumeName {
            default: default.to_string(),
        },
    )
}

fn version(choices: &[&str]) -> TemplateInput {
    input(
        "VERSION",
        "Version",
        TemplateInputKind::Version {
            choices: choices.iter().map(|c| (*c).to_string()).collect(),
        },
    )
}

struct Def {
    id: &'static str,
    name: &'static str,
    category: &'static str,
    description: &'static str,
    image: &'static str,
    service: &'static str,
    env: &'static [(&'static str, &'static str)],
    inputs: Vec<TemplateInput>,
}

impl Def {
    fn build(self) -> Template {
        Template {
            id: self.id.to_string(),
            name: self.name.to_string(),
            kind: TemplateKind::BuiltIn,
            service: self.service.trim_start_matches('\n').to_string(),
            env: self
                .env
                .iter()
                .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
                .collect(),
            inputs: self.inputs,
            category: self.category.to_string(),
            image: self.image.to_string(),
            description: self.description.to_string(),
        }
    }
}

const POSTGRES: &str = r#"
image: postgres:${VERSION}-alpine
ports:
  - "${PORT}:5432"
environment:
  POSTGRES_USER: app
  POSTGRES_DB: app
  POSTGRES_PASSWORD: ${POSTGRES_PASSWORD:?set in .env}
healthcheck:
  test: ["CMD-SHELL", "pg_isready -U app -d app"]
  interval: 5s
  timeout: 3s
  retries: 10
volumes:
  - ${VOLUME}:/var/lib/postgresql/data
"#;

const MYSQL: &str = r#"
image: mysql:${VERSION}
ports:
  - "${PORT}:3306"
environment:
  MYSQL_USER: app
  MYSQL_PASSWORD: ${MYSQL_PASSWORD:?set in .env}
  MYSQL_ROOT_PASSWORD: ${MYSQL_PASSWORD:?set in .env}
  MYSQL_DATABASE: app
healthcheck:
  test: ["CMD-SHELL", "mysqladmin ping -h 127.0.0.1 -u app -p$$MYSQL_PASSWORD --silent"]
  interval: 5s
  timeout: 3s
  retries: 20
volumes:
  - ${VOLUME}:/var/lib/mysql
"#;

const MARIADB: &str = r#"
image: mariadb:${VERSION}
ports:
  - "${PORT}:3306"
environment:
  MARIADB_USER: app
  MARIADB_PASSWORD: ${MARIADB_PASSWORD:?set in .env}
  MARIADB_ROOT_PASSWORD: ${MARIADB_PASSWORD:?set in .env}
  MARIADB_DATABASE: app
healthcheck:
  test: ["CMD", "healthcheck.sh", "--connect", "--innodb_initialized"]
  interval: 5s
  timeout: 3s
  retries: 20
volumes:
  - ${VOLUME}:/var/lib/mysql
"#;

const MONGODB: &str = r#"
image: mongo:${VERSION}
ports:
  - "${PORT}:27017"
environment:
  MONGO_INITDB_ROOT_USERNAME: app
  MONGO_INITDB_ROOT_PASSWORD: ${MONGO_PASSWORD:?set in .env}
healthcheck:
  test: ["CMD", "mongosh", "--quiet", "--eval", "db.adminCommand('ping').ok"]
  interval: 5s
  timeout: 5s
  retries: 20
volumes:
  - ${VOLUME}:/data/db
"#;

const REDIS: &str = r#"
image: redis:${VERSION}-alpine
ports:
  - "${PORT}:6379"
healthcheck:
  test: ["CMD", "redis-cli", "ping"]
  interval: 5s
  timeout: 3s
  retries: 10
"#;

const VALKEY: &str = r#"
image: valkey/valkey:${VERSION}-alpine
ports:
  - "${PORT}:6379"
healthcheck:
  test: ["CMD", "valkey-cli", "ping"]
  interval: 5s
  timeout: 3s
  retries: 10
"#;

const RABBITMQ: &str = r#"
image: rabbitmq:${VERSION}-management-alpine
ports:
  - "${PORT}:5672"
  - "${MANAGEMENT_PORT}:15672"
environment:
  RABBITMQ_DEFAULT_USER: app
  RABBITMQ_DEFAULT_PASS: ${RABBITMQ_PASSWORD:?set in .env}
healthcheck:
  test: ["CMD", "rabbitmq-diagnostics", "-q", "ping"]
  interval: 10s
  timeout: 10s
  retries: 12
volumes:
  - ${VOLUME}:/var/lib/rabbitmq
"#;

const MEILISEARCH: &str = r#"
image: getmeili/meilisearch:${VERSION}
ports:
  - "${PORT}:7700"
environment:
  MEILI_MASTER_KEY: ${MEILI_MASTER_KEY:?set in .env}
healthcheck:
  test: ["CMD", "wget", "--no-verbose", "--spider", "http://127.0.0.1:7700/health"]
  interval: 5s
  timeout: 3s
  retries: 10
volumes:
  - ${VOLUME}:/meili_data
"#;

const S3: &str = r#"
image: chrislusf/seaweedfs:4.50
command: ["server", "-s3", "-dir=/data"]
ports:
  - "${PORT}:8333"
environment:
  AWS_ACCESS_KEY_ID: app
  AWS_SECRET_ACCESS_KEY: ${S3_SECRET_KEY:?set in .env}
healthcheck:
  test: ["CMD", "wget", "--no-verbose", "--spider", "http://127.0.0.1:8333/healthz"]
  interval: 5s
  timeout: 5s
  retries: 10
volumes:
  - ${VOLUME}:/data
"#;

const MAILPIT: &str = r#"
image: axllent/mailpit:v1.32
ports:
  - "${PORT}:1025"
  - "${WEB_PORT}:8025"
healthcheck:
  test: ["CMD", "/mailpit", "readyz"]
  interval: 5s
  timeout: 3s
  retries: 10
"#;

const ADMINER: &str = r#"
image: adminer:6
ports:
  - "${PORT}:8080"
"#;

/// The built-in templates of D84. Every image tag here is checked against Docker Hub by the
/// `BERTH_NET=1` test `template_tags_exist`.
#[must_use]
pub fn built_in() -> Vec<Template> {
    vec![
        Def {
            id: "postgres",
            name: "Postgres",
            category: "Database",
            description: "Postgres 17 with a generated password and a data volume.",
            image: "postgres:17-alpine",
            service: POSTGRES,
            env: &[("POSTGRES_PASSWORD", "${PASSWORD}")],
            inputs: vec![
                port("PORT", "Host port", 5432),
                password(),
                volume("pgdata"),
                version(&["17", "16"]),
            ],
        },
        Def {
            id: "mysql",
            name: "MySQL",
            category: "Database",
            description: "MySQL 8.4 with a generated password and a data volume.",
            image: "mysql:8.4",
            service: MYSQL,
            env: &[("MYSQL_PASSWORD", "${PASSWORD}")],
            inputs: vec![
                port("PORT", "Host port", 3306),
                password(),
                volume("mysqldata"),
                version(&["8.4", "8.0"]),
            ],
        },
        Def {
            id: "mariadb",
            name: "MariaDB",
            category: "Database",
            description: "MariaDB 11 with a generated password and a data volume.",
            image: "mariadb:11",
            service: MARIADB,
            env: &[("MARIADB_PASSWORD", "${PASSWORD}")],
            inputs: vec![
                port("PORT", "Host port", 3306),
                password(),
                volume("mariadbdata"),
                version(&["11", "10.11"]),
            ],
        },
        Def {
            id: "mongodb",
            name: "MongoDB",
            category: "Database",
            description: "MongoDB 8 with a generated password and a data volume.",
            image: "mongo:8",
            service: MONGODB,
            env: &[("MONGO_PASSWORD", "${PASSWORD}")],
            inputs: vec![
                port("PORT", "Host port", 27017),
                password(),
                volume("mongodata"),
                version(&["8", "7"]),
            ],
        },
        Def {
            id: "redis",
            name: "Redis",
            category: "Cache",
            description: "Redis 7 without a password, for a cache or a queue.",
            image: "redis:7.4-alpine",
            service: REDIS,
            env: &[],
            inputs: vec![port("PORT", "Host port", 6379), version(&["7.4", "7.2"])],
        },
        Def {
            id: "valkey",
            name: "Valkey",
            category: "Cache",
            description: "Valkey 8, a Redis-compatible store, without a password.",
            image: "valkey/valkey:8-alpine",
            service: VALKEY,
            env: &[],
            inputs: vec![port("PORT", "Host port", 6379), version(&["8", "7"])],
        },
        Def {
            id: "rabbitmq",
            name: "RabbitMQ",
            category: "Queue",
            description: "RabbitMQ 4 with the management page and a generated password.",
            image: "rabbitmq:4-management-alpine",
            service: RABBITMQ,
            env: &[("RABBITMQ_PASSWORD", "${PASSWORD}")],
            inputs: vec![
                port("PORT", "Host port", 5672),
                port("MANAGEMENT_PORT", "Management port", 15672),
                password(),
                volume("rabbitmqdata"),
                version(&["4", "3"]),
            ],
        },
        Def {
            id: "meilisearch",
            name: "Meilisearch",
            category: "Search",
            description: "Meilisearch 1 with a generated master key and a data volume.",
            image: "getmeili/meilisearch:v1.55",
            service: MEILISEARCH,
            env: &[("MEILI_MASTER_KEY", "${PASSWORD}")],
            inputs: vec![
                port("PORT", "Host port", 7700),
                password(),
                volume("meilidata"),
                version(&["v1.55", "v1.54"]),
            ],
        },
        Def {
            id: "s3",
            name: "S3 (SeaweedFS)",
            category: "Storage",
            description: "S3-compatible storage on SeaweedFS with a generated secret key.",
            image: "chrislusf/seaweedfs:4.50",
            service: S3,
            env: &[("S3_SECRET_KEY", "${PASSWORD}")],
            inputs: vec![port("PORT", "S3 port", 8333), password(), volume("s3data")],
        },
        Def {
            id: "mailpit",
            name: "Mailpit",
            category: "Mail",
            description: "Catches outgoing mail. SMTP on 1025, inbox on 8025.",
            image: "axllent/mailpit:v1.32",
            service: MAILPIT,
            env: &[],
            inputs: vec![
                port("PORT", "SMTP port", 1025),
                port("WEB_PORT", "Web port", 8025),
            ],
        },
        Def {
            id: "adminer",
            name: "Adminer",
            category: "Tools",
            description: "A web page for looking into the databases of this project.",
            image: "adminer:6",
            service: ADMINER,
            env: &[],
            inputs: vec![port("PORT", "Host port", 8080)],
        },
    ]
    .into_iter()
    .map(Def::build)
    .collect()
}

/// Fills `${KEY}` for each input of `t` with the value in `inputs`. A missing or invalid value falls back to
/// the input's default (first choice for a Version, a fresh password for a Password), so a hand-written
/// value can never change the shape of the YAML. Every other `${...}` stays as written.
/// Returns the service body and the `.env` lines.
#[must_use]
pub fn render(
    t: &Template,
    inputs: &std::collections::BTreeMap<String, String>,
) -> (String, Vec<(String, String)>) {
    let r = render_full(t, inputs);
    (r.service, r.env)
}

pub(crate) fn render_full(t: &Template, inputs: &BTreeMap<String, String>) -> Rendered {
    let mut values: Vec<(String, String)> = Vec::new();
    let mut volumes = Vec::new();
    for i in &t.inputs {
        let given = inputs.get(&i.key).map(String::as_str).unwrap_or_default();
        let value = match &i.kind {
            TemplateInputKind::Port { default } => given
                .trim()
                .parse::<u16>()
                .ok()
                .filter(|p| *p != 0)
                .unwrap_or(*default)
                .to_string(),
            TemplateInputKind::Password => {
                if given.is_empty() {
                    generate_password()
                } else {
                    given.to_string()
                }
            }
            TemplateInputKind::VolumeName { default } => {
                if valid_name(given) {
                    volumes.push(given.to_string());
                    given.to_string()
                } else {
                    volumes.push(default.clone());
                    default.clone()
                }
            }
            TemplateInputKind::Version { choices } => {
                if choices.iter().any(|c| c == given) {
                    given.to_string()
                } else {
                    choices.first().cloned().unwrap_or_default()
                }
            }
        };
        values.push((i.key.clone(), value));
    }
    Rendered {
        service: substitute(&t.service, &values),
        env: t
            .env
            .iter()
            .map(|(k, v)| (k.clone(), substitute(v, &values)))
            .collect(),
        volumes,
    }
}

/// One pass over `text`: `${KEY}` becomes the value of `KEY`, anything else is copied. Inserted values are
/// never scanned again.
fn substitute(text: &str, values: &[(String, String)]) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find("${") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        let hit = after.find('}').and_then(|end| {
            let key = &after[..end];
            values
                .iter()
                .find(|(k, _)| k == key)
                .map(|(_, v)| (v.as_str(), end))
        });
        if let Some((value, end)) = hit {
            out.push_str(value);
            rest = &after[end + 1..];
        } else {
            out.push_str("${");
            rest = after;
        }
    }
    out.push_str(rest);
    out
}

/// A compose volume or service name Berth is willing to write.
pub(crate) fn valid_name(s: &str) -> bool {
    let mut chars = s.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphanumeric())
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'))
}

/// 24 characters from `[A-Za-z0-9]`, drawn from the operating system's random source without modulo bias.
#[must_use]
pub fn generate_password() -> String {
    // 248 = 4 * 62: bytes at or above it are dropped so every character is equally likely.
    const LIMIT: u8 = 248;
    let mut out = String::with_capacity(PASSWORD_LEN);
    let mut buf = [0u8; 64];
    while out.len() < PASSWORD_LEN {
        fill_random(&mut buf);
        for b in buf {
            if b < LIMIT && out.len() < PASSWORD_LEN {
                out.push(char::from(ALPHABET[usize::from(b) % ALPHABET.len()]));
            }
        }
    }
    out
}

fn fill_random(buf: &mut [u8]) {
    if getrandom::fill(buf).is_ok() {
        return;
    }
    // The OS source failing is not something a password generator can report. `RandomState` is keyed from
    // the same source at process start, so the fallback stays unpredictable instead of panicking.
    use std::hash::{BuildHasher, Hasher};
    for chunk in buf.chunks_mut(8) {
        let word = std::collections::hash_map::RandomState::new()
            .build_hasher()
            .finish()
            .to_le_bytes();
        chunk.copy_from_slice(&word[..chunk.len()]);
    }
}

/// The compose service name for `t`: its id when that is a valid name, else its name made into one.
/// `add_service_plan` takes the name from its caller; this is the default the Add service sheet offers.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn service_name(t: &Template) -> String {
    if valid_name(&t.id) {
        return t.id.to_lowercase();
    }
    let cleaned: String = t
        .name
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    let cleaned = cleaned.trim_matches('-').to_string();
    if cleaned.is_empty() {
        "service".to_string()
    } else {
        cleaned
    }
}

/// The keys already set in `.env` text.
pub(crate) fn env_keys(text: &str) -> Vec<String> {
    text.lines()
        .filter_map(|l| {
            let l = l.trim_start();
            let l = l.strip_prefix("export ").unwrap_or(l);
            if l.starts_with('#') {
                return None;
            }
            l.split_once('=').map(|(k, _)| k.trim().to_string())
        })
        .collect()
}

/// A plan that adds `t` as the service `service` to the compose file at `root/compose_rel` (it may be
/// docker-compose.yml) and its lines to `.env`. The caller picks the name, so the user can rename the service
/// to get past `ServiceExists`. `compose_text` and `env_text` are what the files hold now; the plan records their hashes,
/// so a change on disk before Write shows as a conflict. An existing `.env` key keeps its value.
///
/// # Errors
/// `ServiceExists` when a service called `service` is already in the file (and only then: another template's
/// service does not count), `Yaml` when the compose file does not parse or `service` is not a usable key.
pub fn add_service_plan(
    root: &Path,
    compose_rel: &Path,
    compose_text: &str,
    env_text: Option<&str>,
    t: &Template,
    service: &str,
    inputs: &std::collections::BTreeMap<String, String>,
) -> Result<GenerationPlan, RulesError> {
    let name = service.to_string();
    let rendered = render_full(t, inputs);
    let mut new_compose = compose_edit::add_service(compose_text, &name, &rendered.service)?;
    for v in &rendered.volumes {
        new_compose = compose_edit::add_named_volume(&new_compose, v)?;
    }

    let mut files = vec![diff::file_diff(
        compose_rel,
        Some(compose_text),
        Some(&new_compose),
    )];
    let mut notes = vec![format!(
        "Adds the {name} service to {}.",
        compose_rel.display()
    )];

    let new_env = compose_edit::add_env_lines(env_text, &rendered.env);
    if env_text != Some(new_env.as_str()) && !(env_text.is_none() && new_env.is_empty()) {
        let have = env_text.map(env_keys).unwrap_or_default();
        for (k, _) in &rendered.env {
            if !have.contains(k) {
                notes.push(format!("Adds {k} to .env."));
            }
        }
        files.push(diff::file_diff(
            &PathBuf::from(".env"),
            env_text,
            Some(&new_env),
        ));
    }

    Ok(GenerationPlan {
        root: root.to_path_buf(),
        files,
        source: PlanSource::Template(t.id.clone()),
        notes,
        after_write: Some(ComposeCommand::Up {
            services: vec![name],
            build: false,
        }),
    })
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    use pretty_assertions::assert_eq;

    use super::*;

    fn by_id(id: &str) -> Template {
        built_in()
            .into_iter()
            .find(|t| t.id == id)
            .expect("template exists")
    }

    /// An obviously fake value for tests that pass a password in.
    fn fake_password() -> String {
        "q".repeat(24)
    }

    fn no_inputs() -> BTreeMap<String, String> {
        BTreeMap::new()
    }

    fn yaml(text: &str) -> serde_norway::Value {
        serde_norway::from_str(text).expect("valid yaml")
    }

    /// The `image:` of a rendered service.
    fn image_of(service: &str) -> String {
        yaml(service)
            .get("image")
            .and_then(|v| v.as_str())
            .expect("image")
            .to_string()
    }

    #[test]
    fn built_in_lists_the_templates_of_d84() {
        let ids: Vec<String> = built_in().into_iter().map(|t| t.id).collect();
        assert_eq!(
            ids,
            [
                "postgres",
                "mysql",
                "mariadb",
                "mongodb",
                "redis",
                "valkey",
                "rabbitmq",
                "meilisearch",
                "s3",
                "mailpit",
                "adminer"
            ]
        );
        assert!(built_in().iter().all(|t| t.kind == TemplateKind::BuiltIn));
    }

    #[test]
    fn every_template_renders_valid_yaml_that_matches_its_declared_image() {
        for t in built_in() {
            let (service, _) = render(&t, &no_inputs());
            assert!(!service.contains("${PORT}"), "{}: port left", t.id);
            assert!(!service.contains("${VOLUME}"), "{}: volume left", t.id);
            assert!(!service.contains("${VERSION}"), "{}: version left", t.id);
            assert_eq!(image_of(&service), t.image, "{}", t.id);
        }
    }

    #[test]
    fn every_version_choice_renders_valid_yaml() {
        for t in built_in() {
            for i in &t.inputs {
                if let TemplateInputKind::Version { choices } = &i.kind {
                    for c in choices {
                        let inputs = BTreeMap::from([(i.key.clone(), c.clone())]);
                        let (service, _) = render(&t, &inputs);
                        assert!(image_of(&service).contains(c.as_str()), "{} {c}", t.id);
                    }
                }
            }
        }
    }

    #[test]
    fn every_data_service_has_a_healthcheck() {
        for t in built_in() {
            let (service, _) = render(&t, &no_inputs());
            let has = yaml(&service).get("healthcheck").is_some();
            assert_eq!(has, t.id != "adminer", "{}", t.id);
        }
    }

    #[test]
    fn a_password_template_keeps_the_password_out_of_the_service_text() {
        for t in built_in() {
            if !t.inputs.iter().any(|i| i.key == "PASSWORD") {
                continue;
            }
            let inputs = BTreeMap::from([("PASSWORD".to_string(), fake_password())]);
            let (service, env) = render(&t, &inputs);
            assert!(!service.contains("qqqq"), "{}: password in service", t.id);
            assert_eq!(env.len(), 1, "{}", t.id);
            assert_eq!(env[0].1, fake_password());
            assert!(
                service.contains(&format!("${{{}:?set in .env}}", env[0].0)),
                "{}: service must read {} from .env",
                t.id,
                env[0].0
            );
        }
    }

    #[test]
    fn render_fills_inputs_and_leaves_other_placeholders() {
        let t = by_id("postgres");
        let inputs = BTreeMap::from([
            ("PORT".to_string(), "5544".to_string()),
            ("VOLUME".to_string(), "shop_pg".to_string()),
            ("VERSION".to_string(), "16".to_string()),
        ]);
        let (service, _) = render(&t, &inputs);
        assert!(service.contains("image: postgres:16-alpine"));
        assert!(service.contains("\"5544:5432\""));
        assert!(service.contains("- shop_pg:/var/lib/postgresql/data"));
        assert!(service.contains("${POSTGRES_PASSWORD:?set in .env}"));
    }

    #[test]
    fn render_replaces_a_bad_input_with_the_default() {
        let t = by_id("postgres");
        let inputs = BTreeMap::from([
            ("PORT".to_string(), "99999".to_string()),
            ("VOLUME".to_string(), "x\n  privileged: true".to_string()),
            (
                "VERSION".to_string(),
                "17\n    privileged: true".to_string(),
            ),
        ]);
        let (service, _) = render(&t, &inputs);
        assert!(!service.contains("privileged"));
        assert!(service.contains("\"5432:5432\""));
        assert!(service.contains("- pgdata:/var/lib"));
        assert!(service.contains("postgres:17-alpine"));
    }

    #[test]
    fn render_does_not_scan_inserted_values_again() {
        let mut t = by_id("redis");
        t.service = "image: x\nlabels:\n  a: ${PORT}\n  b: ${VERSION}".to_string();
        let inputs = BTreeMap::from([("VERSION".to_string(), "${PORT}".to_string())]);
        let (service, _) = render(&t, &inputs);
        // "${PORT}" is not a choice, so the default 7.4 is used; and nothing was substituted twice.
        assert!(service.contains("a: 6379"));
        assert!(service.contains("b: 7.4"));
    }

    #[test]
    fn generate_password_is_24_alphanumeric_characters() {
        let mut seen = BTreeSet::new();
        for _ in 0..200 {
            let p = generate_password();
            assert_eq!(p.chars().count(), 24, "{p}");
            assert!(p.chars().all(|c| c.is_ascii_alphanumeric()), "{p}");
            seen.insert(p);
        }
        assert_eq!(seen.len(), 200, "passwords repeat");
    }

    #[test]
    fn generate_password_uses_every_character_class() {
        let all: String = (0..300).map(|_| generate_password()).collect();
        assert!(all.chars().any(|c| c.is_ascii_uppercase()));
        assert!(all.chars().any(|c| c.is_ascii_lowercase()));
        assert!(all.chars().any(|c| c.is_ascii_digit()));
        let distinct: BTreeSet<char> = all.chars().collect();
        assert_eq!(distinct.len(), 62, "some characters never appear");
    }

    #[test]
    fn add_service_plan_keeps_comments_and_adds_the_volume_and_the_env_line() {
        let compose = "# my stack\nservices:\n  web:\n    image: nginx # front\n    ports:\n      - \"8080:80\"\n";
        let plan = add_service_plan(
            Path::new("/p"),
            Path::new("docker-compose.yml"),
            compose,
            Some("A=1\n"),
            &by_id("postgres"),
            "postgres",
            &BTreeMap::from([("PASSWORD".to_string(), fake_password())]),
        )
        .expect("plan");
        assert_eq!(plan.source, PlanSource::Template("postgres".to_string()));
        assert_eq!(plan.files.len(), 2);
        let c = &plan.files[0];
        assert_eq!(c.path, Path::new("docker-compose.yml"));
        assert_eq!(c.change, berth_core::FileChange::Modify);
        assert_eq!(
            c.old_sha256.as_deref(),
            Some(diff::sha256(compose).as_str())
        );
        let text = c.new_text.as_deref().expect("text");
        assert!(text.starts_with("# my stack\nservices:\n  web:\n    image: nginx # front\n"));
        assert!(text.contains("  postgres:\n"));
        assert!(text.contains("\nvolumes:\n  pgdata:"), "{text}");
        assert!(!text.contains("qqqq"));
        let e = &plan.files[1];
        assert_eq!(e.path, Path::new(".env"));
        assert_eq!(e.change, berth_core::FileChange::Modify);
        assert_eq!(
            e.new_text.as_deref(),
            Some(format!("A=1\nPOSTGRES_PASSWORD={}\n", fake_password()).as_str())
        );
        assert!(plan
            .notes
            .iter()
            .any(|n| n == "Adds POSTGRES_PASSWORD to .env."));
        assert!(plan.notes.iter().all(|n| !n.contains("qqqq")));
        assert_eq!(
            plan.after_write,
            Some(ComposeCommand::Up {
                services: vec!["postgres".to_string()],
                build: false
            })
        );
        serde_norway::from_str::<serde_norway::Value>(text).expect("valid yaml");
    }

    #[test]
    fn add_service_plan_creates_env_when_there_is_none() {
        let plan = add_service_plan(
            Path::new("/p"),
            Path::new("compose.yaml"),
            "services: {}\n",
            None,
            &by_id("postgres"),
            "postgres",
            &no_inputs(),
        )
        .expect("plan");
        let e = plan
            .files
            .iter()
            .find(|f| f.path == Path::new(".env"))
            .expect("env");
        assert_eq!(e.change, berth_core::FileChange::Create);
        assert_eq!(e.old_sha256, None);
    }

    #[test]
    fn add_service_plan_touches_no_env_for_a_template_without_secrets() {
        let plan = add_service_plan(
            Path::new("/p"),
            Path::new("compose.yaml"),
            "services:\n  web:\n    image: x\n",
            None,
            &by_id("redis"),
            "redis",
            &no_inputs(),
        )
        .expect("plan");
        assert_eq!(plan.files.len(), 1);
        assert_eq!(plan.files[0].path, Path::new("compose.yaml"));
    }

    #[test]
    fn add_service_plan_keeps_a_password_the_env_file_already_has() {
        let plan = add_service_plan(
            Path::new("/p"),
            Path::new("compose.yaml"),
            "services:\n  web:\n    image: x\n",
            Some(format!("{}={}\n", "POSTGRES_PASSWORD", "mine").as_str()),
            &by_id("postgres"),
            "postgres",
            &no_inputs(),
        )
        .expect("plan");
        assert_eq!(plan.files.len(), 1, "an unchanged .env is not a change");
        assert!(plan.notes.iter().all(|n| !n.contains(".env")));
    }

    #[test]
    fn add_service_plan_refuses_a_service_that_exists() {
        let err = add_service_plan(
            Path::new("/p"),
            Path::new("compose.yaml"),
            "services:\n  postgres:\n    image: x\n",
            None,
            &by_id("postgres"),
            "postgres",
            &no_inputs(),
        )
        .expect_err("exists");
        assert!(matches!(err, RulesError::ServiceExists(n) if n == "postgres"));
    }

    #[test]
    fn add_service_plan_uses_the_given_name_for_the_service_and_the_up_command() {
        let plan = add_service_plan(
            Path::new("/p"),
            Path::new("compose.yaml"),
            "services:\n  postgres:\n    image: x\n",
            None,
            &by_id("postgres"),
            "postgres-2",
            &no_inputs(),
        )
        .expect("a different name is not a clash");
        let text = plan.files[0].new_text.as_deref().expect("text");
        assert!(text.contains("\n  postgres-2:\n"), "{text}");
        assert!(text.contains("\n  postgres:\n    image: x\n"), "{text}");
        assert_eq!(
            plan.after_write,
            Some(ComposeCommand::Up {
                services: vec!["postgres-2".to_string()],
                build: false
            })
        );
        assert!(plan
            .notes
            .iter()
            .any(|n| n.starts_with("Adds the postgres-2 service")));
    }

    #[test]
    fn add_service_plan_clashes_only_on_the_name_it_was_given() {
        // `db` holds a Postgres, but the new service is called `pg`: no clash.
        let compose = "services:\n  db:\n    image: postgres:17\n";
        assert!(add_service_plan(
            Path::new("/p"),
            Path::new("compose.yaml"),
            compose,
            None,
            &by_id("postgres"),
            "pg",
            &no_inputs(),
        )
        .is_ok());
        let err = add_service_plan(
            Path::new("/p"),
            Path::new("compose.yaml"),
            compose,
            None,
            &by_id("postgres"),
            "db",
            &no_inputs(),
        )
        .expect_err("db exists");
        assert!(matches!(err, RulesError::ServiceExists(n) if n == "db"));
    }

    #[test]
    fn a_custom_template_gets_a_valid_service_name() {
        let mut t = by_id("redis");
        t.id = "7f3a-9c Cache!".to_string();
        t.name = "My Cache!".to_string();
        assert_eq!(service_name(&t), "my-cache");
        t.id = "queue".to_string();
        assert_eq!(service_name(&t), "queue");
    }

    /// Every image the templates can render, as `(registry repo, tag)` on Docker Hub.
    fn template_images() -> BTreeSet<String> {
        let mut out = BTreeSet::new();
        for t in built_in() {
            let versions: Vec<Option<(String, String)>> =
                match t.inputs.iter().find_map(|i| match &i.kind {
                    TemplateInputKind::Version { choices } => {
                        Some((i.key.clone(), choices.clone()))
                    }
                    _ => None,
                }) {
                    Some((key, choices)) => choices
                        .into_iter()
                        .map(|c| Some((key.clone(), c)))
                        .collect(),
                    None => vec![None],
                };
            for v in versions {
                let inputs: BTreeMap<String, String> = v.into_iter().collect();
                let (service, _) = render(&t, &inputs);
                out.insert(image_of(&service));
            }
        }
        out
    }

    /// `BERTH_NET=1 cargo nextest run -p berth-rules --run-ignored only template_tags_exist`
    ///
    /// Lists the tags of every template image and every Dockerfile base image on Docker Hub (registry-1 follows
    /// `Link` pagination; the Hub `tags` endpoint returns one page) and fails naming each image that is missing.
    #[tokio::test]
    #[ignore = "needs network: BERTH_NET=1"]
    async fn template_tags_exist() {
        if std::env::var("BERTH_NET").as_deref() != Ok("1") {
            eprintln!("skipped: set BERTH_NET=1");
            return;
        }
        let mut images = template_images();
        images.extend(crate::recipes::pinned_base_images());
        assert!(images.len() > 25, "only {} images collected", images.len());

        let registry = berth_registry::RegistryClient::new();
        let hub = url::Url::parse("https://registry-1.docker.io").expect("url");
        let mut by_repo: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for image in &images {
            let (repo, tag) = image.rsplit_once(':').expect("pinned tag");
            let repo = if repo.contains('/') {
                repo.to_string()
            } else {
                format!("library/{repo}")
            };
            by_repo.entry(repo).or_default().push(tag.to_string());
        }
        let mut missing = Vec::new();
        for (repo, wanted) in &by_repo {
            let have = registry
                .tags(&hub, repo, None)
                .await
                .unwrap_or_else(|e| panic!("tags of {repo}: {e}"));
            for tag in wanted {
                if !have.contains(tag) {
                    missing.push(format!("{repo}:{tag}"));
                }
            }
        }
        assert!(
            missing.is_empty(),
            "tags missing on Docker Hub: {missing:?}"
        );
    }
}
