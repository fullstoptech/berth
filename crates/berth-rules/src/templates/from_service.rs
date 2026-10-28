//! Save as template (P8-T4): a custom [`Template`] from one service of a project's compose file.
//!
//! The template keeps the service as the file says it, with three kinds of change so that Add service can
//! put it into another project:
//!
//! * Host ports, and named volumes in the short `name:/path` form, become inputs (`${PORT}`, `${VOLUME}`)
//!   with the old values as defaults, so Add service can suggest a free port and declare the volume.
//! * A secret never reaches the store (CLAUDE.md, secrets): a literal value of a password-like
//!   `environment` key, or a URL that carries a password, becomes `${KEY:?set in .env}`. A password-like
//!   key also gets a generated password in the new project's `.env` (the Password input), the way the
//!   built-in templates do. Values that already read `${...}` stay as written.
//! * Keys that point at the old project's other parts (`depends_on`, `networks`, `container_name`, ...) are
//!   dropped, because compose would refuse a service that names something the new file lacks.
//!
//! Rendering the saved template with its defaults gives the service's own settings back.

use berth_core::{Template, TemplateInput, TemplateInputKind, TemplateKind};
use serde_norway::{Mapping, Value};

use super::valid_name;
use crate::RulesError;

/// The category the Templates screen files a saved template under (the screen groups by kind).
pub const CUSTOM_CATEGORY: &str = "Custom";

/// Keys that refer to other parts of the old project, or to files next to its compose file.
const PROJECT_BOUND_KEYS: [&str; 9] = [
    "container_name",
    "depends_on",
    "links",
    "external_links",
    "networks",
    "extends",
    "volumes_from",
    "env_file",
    "secrets",
];

const SECRET_WORDS: [&str; 8] = [
    "PASSWORD",
    "PASSWD",
    "SECRET",
    "TOKEN",
    "APIKEY",
    "API_KEY",
    "PRIVATE_KEY",
    "CREDENTIAL",
];

fn yaml_error(message: String) -> RulesError {
    RulesError::Yaml { line: 0, message }
}

fn is_secret_key(key: &str) -> bool {
    let upper = key.to_ascii_uppercase();
    SECRET_WORDS.iter().any(|w| upper.contains(w))
        || upper.ends_with("_PASS")
        || upper.ends_with("_PWD")
        || upper.ends_with("_KEY")
        || upper.contains("ACCESS_KEY")
}

/// `scheme://user:password@host`: a password in a URL.
fn url_carries_password(value: &str) -> bool {
    let Some((_, rest)) = value.split_once("://") else {
        return false;
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    authority
        .rsplit_once('@')
        .is_some_and(|(userinfo, _)| userinfo.contains(':'))
}

/// What replaces an environment value, if it must not be stored.
enum Scrub {
    Keep,
    /// A password-like key: the new `.env` gets a generated password.
    Password,
    /// A value with a password inside: the user fills it in.
    Blank,
}

fn scrub_of(key: &str, value: &str) -> Scrub {
    let plain = value.chars().all(|c| c.is_ascii_digit()) || matches!(value, "true" | "false");
    if value.is_empty() || value.contains('$') || plain {
        Scrub::Keep
    } else if is_secret_key(key) {
        Scrub::Password
    } else if url_carries_password(value) {
        Scrub::Blank
    } else {
        Scrub::Keep
    }
}

fn placeholder(key: &str) -> String {
    format!("${{{key}:?set in .env}}")
}

#[derive(Default)]
struct Builder {
    inputs: Vec<TemplateInput>,
    env: Vec<(String, String)>,
    password: bool,
}

impl Builder {
    fn next_key(&self, base: &str) -> String {
        let taken = |k: &str| self.inputs.iter().any(|i| i.key == k);
        if !taken(base) {
            return base.to_string();
        }
        (2..)
            .map(|n| format!("{base}_{n}"))
            .find(|k| !taken(k))
            .unwrap_or_else(|| base.to_string())
    }

    fn secret(&mut self, key: &str, scrub: &Scrub) -> String {
        match scrub {
            Scrub::Password => {
                if !self.password {
                    self.password = true;
                    self.inputs.push(TemplateInput {
                        key: "PASSWORD".to_string(),
                        label: "Password".to_string(),
                        kind: TemplateInputKind::Password,
                    });
                }
                self.env_line(key, "${PASSWORD}");
            }
            Scrub::Blank | Scrub::Keep => self.env_line(key, ""),
        }
        placeholder(key)
    }

    fn env_line(&mut self, key: &str, value: &str) {
        if !self.env.iter().any(|(k, _)| k == key) {
            self.env.push((key.to_string(), value.to_string()));
        }
    }

    fn environment(&mut self, env: &mut Value) {
        match env {
            Value::Mapping(map) => {
                let keys: Vec<Value> = map.keys().cloned().collect();
                for k in keys {
                    let Some(name) = k.as_str().map(str::to_string) else {
                        continue;
                    };
                    let Some(Value::String(value)) = map.get(&k).cloned() else {
                        continue;
                    };
                    let scrub = scrub_of(&name, &value);
                    if !matches!(scrub, Scrub::Keep) {
                        let replaced = self.secret(&name, &scrub);
                        map.insert(k, Value::String(replaced));
                    }
                }
            }
            Value::Sequence(items) => {
                for item in items {
                    let Value::String(line) = item else { continue };
                    let Some((name, value)) = line.split_once('=') else {
                        continue;
                    };
                    let scrub = scrub_of(name, value);
                    if !matches!(scrub, Scrub::Keep) {
                        let name = name.to_string();
                        let replaced = self.secret(&name, &scrub);
                        *line = format!("{name}={replaced}");
                    }
                }
            }
            _ => {}
        }
    }

    /// `8080:80`, `127.0.0.1:8080:80/tcp`: the host port becomes an input. Anything else stays.
    fn port(&mut self, spec: &str) -> Option<String> {
        let (mapping, proto) = match spec.split_once('/') {
            Some((m, p)) => (m, Some(p)),
            None => (spec, None),
        };
        let parts: Vec<&str> = mapping.split(':').collect();
        let (ip, host, container) = match parts.as_slice() {
            [host, container] => (None, *host, *container),
            [ip, host, container] => (Some(*ip), *host, *container),
            _ => return None,
        };
        let default: u16 = host.parse().ok().filter(|p| *p != 0)?;
        if container.is_empty() || container.contains('$') || ip.is_some_and(|i| i.contains('$')) {
            return None;
        }
        let first = !self.inputs.iter().any(|i| i.key == "PORT");
        let key = self.next_key("PORT");
        self.inputs.push(TemplateInput {
            label: if first {
                "Host port".to_string()
            } else {
                format!("Host port for {container}")
            },
            key: key.clone(),
            kind: TemplateInputKind::Port { default },
        });
        let mut out = String::new();
        if let Some(ip) = ip {
            out.push_str(ip);
            out.push(':');
        }
        out.push_str(&format!("${{{key}}}:{container}"));
        if let Some(proto) = proto {
            out.push('/');
            out.push_str(proto);
        }
        Some(out)
    }

    /// `pgdata:/var/lib/postgresql/data[:ro]`: the volume name becomes an input. Paths and anonymous
    /// volumes stay.
    fn volume(&mut self, spec: &str) -> Option<String> {
        let (source, rest) = spec.split_once(':')?;
        if !valid_name(source) {
            return None;
        }
        let key = self.next_key("VOLUME");
        self.inputs.push(TemplateInput {
            key: key.clone(),
            label: if key == "VOLUME" {
                "Data volume".to_string()
            } else {
                format!("Volume for {rest}")
            },
            kind: TemplateInputKind::VolumeName {
                default: source.to_string(),
            },
        });
        Some(format!("${{{key}}}:{rest}"))
    }
}

fn rewrite_each(list: &mut Value, mut rewrite: impl FnMut(&str) -> Option<String>) {
    if let Value::Sequence(items) = list {
        for item in items {
            if let Value::String(text) = item {
                if let Some(new) = rewrite(text) {
                    *text = new;
                }
            }
        }
    }
}

/// The custom template for `services.<service>` of `compose_text`. `id` is the template's key in the
/// store (saving again with the same id replaces it), `name` what the Templates screen shows, `source`
/// the project it was saved from.
///
/// # Errors
/// `Yaml` when the file does not parse, or the service has no `image` to pull (a service built from a
/// folder cannot be a template); `NoService` when the file has no such service.
pub fn from_service(
    compose_text: &str,
    service: &str,
    id: &str,
    name: &str,
    source: &str,
) -> Result<Template, RulesError> {
    let doc: Value = serde_norway::from_str(compose_text).map_err(|e| RulesError::Yaml {
        line: e.location().map_or(0, |l| l.line()),
        message: e.to_string(),
    })?;
    let mut body: Mapping = doc
        .get("services")
        .and_then(|s| s.get(service))
        .and_then(Value::as_mapping)
        .cloned()
        .ok_or_else(|| RulesError::NoService(service.to_string()))?;

    let image = match body.get("image") {
        Some(Value::String(image)) if !image.trim().is_empty() => image.trim().to_string(),
        _ => {
            return Err(yaml_error(format!(
                "{service} is built from a folder and has no image to pull, so it cannot be a template."
            )))
        }
    };
    if body.contains_key("build") {
        return Err(yaml_error(format!(
            "{service} is built from a folder, so it cannot be a template."
        )));
    }
    for key in PROJECT_BOUND_KEYS {
        body.remove(key);
    }

    let mut b = Builder::default();
    if let Some(ports) = body.get_mut("ports") {
        rewrite_each(ports, |p| b.port(p));
    }
    if let Some(volumes) = body.get_mut("volumes") {
        rewrite_each(volumes, |v| b.volume(v));
    }
    if let Some(env) = body.get_mut("environment") {
        b.environment(env);
    }

    let mut text = serde_norway::to_string(&Value::Mapping(body))
        .map_err(|e| yaml_error(format!("cannot write {service}: {e}")))?;
    if !text.ends_with('\n') {
        text.push('\n');
    }
    let mut description = format!("Saved from the {service} service of {source}.");
    if b.password {
        description.push_str(" Its password is generated when you add it.");
    }
    Ok(Template {
        id: id.to_string(),
        name: name.to_string(),
        kind: TemplateKind::Custom,
        service: text,
        env: b.env,
        inputs: b.inputs,
        category: CUSTOM_CATEGORY.to_string(),
        image,
        description,
    })
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::path::Path;

    use super::*;
    use crate::templates::{add_service_plan, render};

    const COMPOSE: &str = r#"services:
  cache:
    image: redis:7-alpine
    labels: [dev.berth.fixture=1]
    ports:
      - "127.0.0.1:56379:6379"
  db:
    image: postgres:17-alpine
    container_name: shop-db
    depends_on: [cache]
    networks: [backend]
    environment:
      POSTGRES_USER: app
      POSTGRES_PASSWORD: hunter2-literal
      DATABASE_URL: postgres://app:s3cr3t-in-url@db:5432/app
      LOG_LEVEL: debug
      TOKEN_TTL: "3600"
      FROM_ENV: ${SOMETHING_ELSE}
    ports:
      - "5432:5432/tcp"
      - 9187
    volumes:
      - pgdata:/var/lib/postgresql/data
      - ./init.sql:/docker-entrypoint-initdb.d/init.sql:ro
      - /var/tmp
      - logs:/var/log:ro
  api:
    build: ./api
    image: shop-api
  listy:
    image: alpine:3
    environment:
      - API_TOKEN=abc-literal
      - MODE=fast
      - PLAIN
volumes:
  pgdata:
  logs:
networks:
  backend:
"#;

    fn save(service: &str) -> Template {
        from_service(COMPOSE, service, "custom-shop-x", service, "shop").expect("template")
    }

    fn value(text: &str) -> Value {
        serde_norway::from_str(text).expect("yaml")
    }

    fn original(service: &str) -> Value {
        value(COMPOSE)["services"][service].clone()
    }

    #[test]
    fn custom_templates_a_service_saves_as_a_custom_template_and_renders_back_to_itself() {
        let t = save("cache");
        assert_eq!(t.kind, TemplateKind::Custom);
        assert_eq!(t.id, "custom-shop-x");
        assert_eq!(t.name, "cache");
        assert_eq!(t.image, "redis:7-alpine");
        assert_eq!(t.category, CUSTOM_CATEGORY);
        assert_eq!(t.description, "Saved from the cache service of shop.");
        assert_eq!(
            t.inputs,
            vec![TemplateInput {
                key: "PORT".to_string(),
                label: "Host port".to_string(),
                kind: TemplateInputKind::Port { default: 56379 },
            }]
        );
        assert!(t.env.is_empty());
        let (rendered, env) = render(&t, &BTreeMap::new());
        assert!(env.is_empty());
        assert_eq!(value(&rendered), original("cache"), "{rendered}");
        // The host port is a choice, the container side and the address are not.
        let mut chosen = BTreeMap::new();
        chosen.insert("PORT".to_string(), "56400".to_string());
        let (rendered, _) = render(&t, &chosen);
        assert!(rendered.contains("127.0.0.1:56400:6379"), "{rendered}");
    }

    #[test]
    fn custom_templates_add_service_writes_the_saved_block_into_another_project() {
        let t = save("cache");
        let other = "services:\n  web:\n    image: nginx\n";
        let plan = add_service_plan(
            Path::new("/p"),
            Path::new("compose.yaml"),
            other,
            None,
            &t,
            "cache",
            &BTreeMap::new(),
        )
        .expect("plan");
        let new_compose = plan.files[0].new_text.clone().expect("new text");
        let doc = value(&new_compose);
        assert_eq!(doc["services"]["cache"], original("cache"), "{new_compose}");
        assert_eq!(doc["services"]["web"]["image"], Value::from("nginx"));
        assert_eq!(
            plan.files.len(),
            1,
            "no .env change for a service without secrets"
        );
    }

    #[test]
    fn custom_templates_secrets_never_reach_the_template() {
        let t = save("db");
        let all = format!("{} {:?} {:?}", t.service, t.env, t.inputs);
        for secret in ["hunter2-literal", "s3cr3t-in-url"] {
            assert!(!all.contains(secret), "{secret} leaked: {all}");
        }
        let doc = value(&t.service);
        let env = &doc["environment"];
        assert_eq!(
            env["POSTGRES_PASSWORD"],
            Value::from("${POSTGRES_PASSWORD:?set in .env}")
        );
        assert_eq!(
            env["DATABASE_URL"],
            Value::from("${DATABASE_URL:?set in .env}")
        );
        // Plain settings, numbers and references stay as they were.
        assert_eq!(env["POSTGRES_USER"], Value::from("app"));
        assert_eq!(env["LOG_LEVEL"], Value::from("debug"));
        assert_eq!(env["TOKEN_TTL"], Value::from("3600"));
        assert_eq!(env["FROM_ENV"], Value::from("${SOMETHING_ELSE}"));
        // A password-like key gets a generated password in the new .env, the URL is left to the user.
        assert_eq!(
            t.env,
            vec![
                ("POSTGRES_PASSWORD".to_string(), "${PASSWORD}".to_string()),
                ("DATABASE_URL".to_string(), String::new()),
            ]
        );
        assert!(t
            .inputs
            .iter()
            .any(|i| i.key == "PASSWORD" && i.kind == TemplateInputKind::Password));
        assert!(t
            .description
            .ends_with("Its password is generated when you add it."));

        let (_, lines) = render(&t, &BTreeMap::new());
        let generated = &lines[0].1;
        assert_eq!(generated.len(), 24);
        assert_ne!(generated, "hunter2-literal");
    }

    #[test]
    fn custom_templates_list_form_environment_is_scrubbed_too() {
        let t = save("listy");
        let doc = value(&t.service);
        let items: Vec<&str> = doc["environment"]
            .as_sequence()
            .expect("list")
            .iter()
            .filter_map(Value::as_str)
            .collect();
        assert_eq!(
            items,
            ["API_TOKEN=${API_TOKEN:?set in .env}", "MODE=fast", "PLAIN"]
        );
        assert!(!t.service.contains("abc-literal"));
    }

    #[test]
    fn custom_templates_named_volumes_and_ports_become_inputs_and_paths_stay() {
        let t = save("db");
        let keys: Vec<(&str, &TemplateInputKind)> =
            t.inputs.iter().map(|i| (i.key.as_str(), &i.kind)).collect();
        assert!(keys.contains(&("PORT", &TemplateInputKind::Port { default: 5432 })));
        assert!(keys.contains(&(
            "VOLUME",
            &TemplateInputKind::VolumeName {
                default: "pgdata".to_string()
            }
        )));
        assert!(keys.contains(&(
            "VOLUME_2",
            &TemplateInputKind::VolumeName {
                default: "logs".to_string()
            }
        )));
        let doc = value(&t.service);
        let ports: Vec<&str> = doc["ports"]
            .as_sequence()
            .expect("ports")
            .iter()
            .filter_map(Value::as_str)
            .collect();
        assert_eq!(
            ports,
            ["${PORT}:5432/tcp"],
            "9187 is an int and stays: {doc:?}"
        );
        assert_eq!(doc["ports"][1], Value::from(9187));
        let volumes: Vec<&str> = doc["volumes"]
            .as_sequence()
            .expect("volumes")
            .iter()
            .filter_map(Value::as_str)
            .collect();
        assert_eq!(
            volumes,
            [
                "${VOLUME}:/var/lib/postgresql/data",
                "./init.sql:/docker-entrypoint-initdb.d/init.sql:ro",
                "/var/tmp",
                "${VOLUME_2}:/var/log:ro",
            ]
        );
        // Adding it declares the volumes the new project lacks.
        let plan = add_service_plan(
            Path::new("/p"),
            Path::new("compose.yaml"),
            "services:\n  web:\n    image: nginx\n",
            None,
            &t,
            "db",
            &BTreeMap::new(),
        )
        .expect("plan");
        let new_compose = plan.files[0].new_text.clone().expect("text");
        let doc = value(&new_compose);
        assert!(doc["volumes"].get("pgdata").is_some(), "{new_compose}");
        assert!(doc["volumes"].get("logs").is_some(), "{new_compose}");
        let env_after = plan.files[1].new_text.clone().expect(".env");
        assert!(env_after.contains("POSTGRES_PASSWORD="), "{env_after}");
        assert!(env_after.contains("DATABASE_URL="), "{env_after}");
    }

    #[test]
    fn custom_templates_keys_that_point_at_the_old_project_are_dropped() {
        let t = save("db");
        let doc = value(&t.service);
        for key in ["container_name", "depends_on", "networks"] {
            assert!(doc.get(key).is_none(), "{key} kept: {doc:?}");
        }
        assert_eq!(doc["image"], Value::from("postgres:17-alpine"));
    }

    #[test]
    fn custom_templates_a_built_service_or_a_missing_one_is_refused() {
        let err = from_service(COMPOSE, "api", "i", "api", "shop").expect_err("build");
        assert!(matches!(err, RulesError::Yaml { .. }), "{err:?}");
        assert!(err.to_string().contains("built from a folder"), "{err}");
        let err = from_service(COMPOSE, "nope", "i", "nope", "shop").expect_err("missing");
        assert!(matches!(err, RulesError::NoService(n) if n == "nope"));
        let err = from_service("services: [", "cache", "i", "c", "shop").expect_err("bad yaml");
        assert!(matches!(err, RulesError::Yaml { .. }));
    }
}
