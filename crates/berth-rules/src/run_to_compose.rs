//! `docker run` to Compose.
//!
//! Handles `-p`, `-e`, `-v`, `--name`, `--restart` and `--network` (plus `-d`, which Compose has no use for).
//! Any other flag is a `RunParse` error that names the flag, so nothing is dropped silently.
//!
//! Environment values never reach the output: `-e KEY=value` becomes `KEY: ${KEY}`, and a comment at
//! the top lists the keys to set in `.env`. Error messages never repeat a value either.

use crate::RulesError;

#[derive(Default)]
struct Run {
    name: Option<String>,
    image: String,
    ports: Vec<String>,
    env_keys: Vec<String>,
    volumes: Vec<String>,
    restart: Option<String>,
    network: Option<String>,
    command: Vec<String>,
}

fn bad(message: impl Into<String>) -> RulesError {
    RulesError::RunParse(message.into())
}

/// Turns one `docker run ...` command line into the text of a `compose.yaml`.
pub fn convert(docker_run: &str) -> Result<String, RulesError> {
    let run = parse(docker_run)?;
    let out = emit(&run);
    if serde_norway::from_str::<serde_norway::Value>(&out).is_err() {
        return Err(bad(
            "the command produced a Compose file that is not valid YAML",
        ));
    }
    Ok(out)
}

fn parse(input: &str) -> Result<Run, RulesError> {
    let joined = input.replace("\\\r\n", " ").replace("\\\n", " ");
    let tokens = shell_words::split(joined.trim().trim_start_matches("$ "))
        .map_err(|_| bad("unbalanced quotes"))?;
    let mut it = tokens.into_iter().peekable();

    if it.next_if(|t| t == "docker").is_some() {
        it.next_if(|t| t == "container");
        if it.next_if(|t| t == "run").is_none() {
            return Err(bad("expected `docker run`"));
        }
    } else {
        it.next_if(|t| t == "run");
    }

    let mut run = Run::default();
    while let Some(tok) = it.next() {
        if !tok.starts_with('-') || tok == "-" {
            run.image = tok;
            run.command = it.collect();
            break;
        }
        let (flag, inline) = split_flag(&tok);
        let canonical = match flag.as_str() {
            "-d" | "--detach" if inline.is_none() => continue,
            "-p" | "--publish" => "p",
            "-e" | "--env" => "e",
            "-v" | "--volume" => "v",
            "--name" => "name",
            "--restart" => "restart",
            "--network" | "--net" => "network",
            _ => return Err(bad(format!("unsupported flag {flag}"))),
        };
        let value = match inline {
            Some(v) => v,
            None => it
                .next()
                .ok_or_else(|| bad(format!("{flag} needs a value")))?,
        };
        match canonical {
            "p" => run.ports.push(check_port(&value)?),
            "e" => {
                let key = env_key(&value)?;
                if !run.env_keys.contains(&key) {
                    run.env_keys.push(key);
                }
            }
            "v" => run.volumes.push(check_volume(&value)?),
            "name" => run.name = Some(check_name(&value)?),
            "restart" => run.restart = Some(check_restart(&value)?),
            _ => run.network = Some(check_network(&value)?),
        }
    }
    if run.image.is_empty() {
        return Err(bad("no image in the command"));
    }
    Ok(run)
}

/// `--name=db` gives (`--name`, Some(`db`)); `-p5432:5432` gives (`-p`, Some(`5432:5432`)).
fn split_flag(tok: &str) -> (String, Option<String>) {
    if let Some(long) = tok.strip_prefix("--") {
        return match long.split_once('=') {
            Some((n, v)) => (format!("--{n}"), Some(v.to_string())),
            None => (tok.to_string(), None),
        };
    }
    let mut chars = tok.chars();
    chars.next();
    match chars.next() {
        Some(c @ ('p' | 'e' | 'v')) if tok.len() > 2 => {
            (format!("-{c}"), Some(tok[2..].to_string()))
        }
        _ => (tok.to_string(), None),
    }
}

fn check_port(v: &str) -> Result<String, RulesError> {
    let ok = !v.is_empty()
        && v.chars()
            .all(|c| c.is_ascii_alphanumeric() || ".:/-[]".contains(c));
    if ok {
        Ok(v.to_string())
    } else {
        Err(bad(format!("cannot read the port {v:?}")))
    }
}

/// Only the key survives; the value is dropped here so nothing downstream can write it.
fn env_key(v: &str) -> Result<String, RulesError> {
    let key = v.split_once('=').map_or(v, |(k, _)| k);
    let mut chars = key.chars();
    let valid = chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_');
    if valid {
        Ok(key.to_string())
    } else {
        Err(bad(
            "an -e flag has a variable name that Compose cannot use",
        ))
    }
}

fn is_name(s: &str) -> bool {
    let mut chars = s.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphanumeric())
        && chars.all(|c| c.is_ascii_alphanumeric() || "_.-".contains(c))
}

fn check_name(v: &str) -> Result<String, RulesError> {
    if is_name(v) {
        Ok(v.to_string())
    } else {
        Err(bad(format!("cannot use {v:?} as a service name")))
    }
}

fn check_restart(v: &str) -> Result<String, RulesError> {
    let (kind, max) = v.split_once(':').map_or((v, None), |(k, m)| (k, Some(m)));
    let ok = match kind {
        "no" | "always" | "unless-stopped" => max.is_none(),
        "on-failure" => max.is_none_or(|m| !m.is_empty() && m.chars().all(|c| c.is_ascii_digit())),
        _ => false,
    };
    if ok {
        Ok(v.to_string())
    } else {
        Err(bad(format!("cannot use {v:?} as a restart policy")))
    }
}

fn check_network(v: &str) -> Result<String, RulesError> {
    if v.is_empty() || v.chars().any(char::is_whitespace) {
        Err(bad("cannot read the --network value"))
    } else {
        Ok(v.to_string())
    }
}

fn check_volume(v: &str) -> Result<String, RulesError> {
    if v.is_empty() || v.chars().any(char::is_whitespace) {
        return Err(bad(format!("cannot read the volume {v:?}")));
    }
    if let Some(name) = named_volume(v) {
        if !is_name(name) {
            return Err(bad(format!("cannot use {name:?} as a volume name")));
        }
    }
    Ok(v.to_string())
}

/// `pg:/var/lib/data` names a volume; `./data:/data`, `/abs:/x`, `~/x:/x` and `$PWD/x:/x` are bind mounts.
fn named_volume(v: &str) -> Option<&str> {
    let (source, _) = v.split_once(':')?;
    if source.is_empty() || source.starts_with(['/', '.', '~', '$']) {
        None
    } else {
        Some(source)
    }
}

fn service_name(run: &Run) -> String {
    if let Some(n) = &run.name {
        return n.clone();
    }
    let no_digest = run.image.split('@').next().unwrap_or("");
    let last = no_digest.rsplit('/').next().unwrap_or("");
    let base = last.split(':').next().unwrap_or("");
    let cleaned: String = base
        .to_ascii_lowercase()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || "_.-".contains(c) {
                c
            } else {
                '-'
            }
        })
        .collect();
    let trimmed = cleaned.trim_start_matches(['_', '.', '-']);
    if trimmed.is_empty() {
        "app".to_string()
    } else {
        trimmed.to_string()
    }
}

/// A YAML scalar: plain when that is safe and means a string, double-quoted otherwise.
fn scalar(s: &str) -> String {
    let mut chars = s.chars();
    let first_ok = chars
        .next()
        .is_some_and(|c| c.is_ascii_alphanumeric() || "./~_$".contains(c));
    let rest_ok = s
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || "_./:@%+=,~-$".contains(c));
    let lower = s.to_ascii_lowercase();
    let reserved = matches!(
        lower.as_str(),
        "true" | "false" | "yes" | "no" | "on" | "off" | "null" | "y" | "n" | "~"
    );
    if first_ok && rest_ok && !reserved && s.parse::<f64>().is_err() {
        s.to_string()
    } else {
        quote(s)
    }
}

fn quote(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => out.push_str(&format!("\\u{:04x}", u32::from(c))),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn emit(run: &Run) -> String {
    let mut out = String::from("# Converted from a docker run command by Berth.\n");
    if !run.env_keys.is_empty() {
        out.push_str(&format!(
            "# Values given with -e were not copied. Set these in .env: {}\n",
            run.env_keys.join(", ")
        ));
    }
    out.push_str("services:\n");
    out.push_str(&format!("  {}:\n", scalar(&service_name(run))));
    out.push_str(&format!("    image: {}\n", scalar(&run.image)));

    let mut network_mode = None;
    let mut network_name = None;
    if let Some(n) = &run.network {
        match n.as_str() {
            "default" => {}
            "host" | "none" | "bridge" => network_mode = Some(n.as_str()),
            _ if n.starts_with("container:") || n.starts_with("service:") => {
                network_mode = Some(n.as_str())
            }
            _ => network_name = Some(n.as_str()),
        }
    }
    if let Some(r) = &run.restart {
        out.push_str(&format!("    restart: {}\n", scalar(r)));
    }
    if let Some(m) = network_mode {
        out.push_str(&format!("    network_mode: {}\n", scalar(m)));
    }
    if let Some(n) = network_name {
        out.push_str(&format!("    networks:\n      - {}\n", scalar(n)));
    }
    if !run.ports.is_empty() {
        out.push_str("    ports:\n");
        for p in &run.ports {
            out.push_str(&format!("      - {}\n", quote(p)));
        }
    }
    if !run.env_keys.is_empty() {
        out.push_str("    environment:\n");
        for k in &run.env_keys {
            out.push_str(&format!("      {k}: ${{{k}}}\n"));
        }
    }
    if !run.volumes.is_empty() {
        out.push_str("    volumes:\n");
        for v in &run.volumes {
            out.push_str(&format!("      - {}\n", scalar(v)));
        }
    }
    if !run.command.is_empty() {
        out.push_str("    command:\n");
        for c in &run.command {
            out.push_str(&format!("      - {}\n", scalar(c)));
        }
    }

    let mut named: Vec<&str> = Vec::new();
    for v in &run.volumes {
        if let Some(n) = named_volume(v) {
            if !named.contains(&n) {
                named.push(n);
            }
        }
    }
    if !named.is_empty() {
        out.push_str("volumes:\n");
        for n in named {
            out.push_str(&format!("  {}:\n", scalar(n)));
        }
    }
    if let Some(n) = network_name {
        out.push_str(&format!(
            "networks:\n  {}:\n    external: true\n",
            scalar(n)
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use serde_norway::Value;

    use super::*;

    fn yaml(s: &str) -> Value {
        serde_norway::from_str(s).expect("valid yaml")
    }

    fn err_text(cmd: &str) -> String {
        match convert(cmd) {
            Err(RulesError::RunParse(m)) => m,
            other => panic!("expected RunParse, got {other:?}"),
        }
    }

    #[test]
    fn the_postgres_example_from_the_task() {
        let out = convert(
            "docker run -d --name db -p 5432:5432 -e POSTGRES_PASSWORD='x' -v pg:/var/lib/postgresql/data postgres:17",
        )
        .expect("convert");
        assert_eq!(
            out,
            "# Converted from a docker run command by Berth.\n\
             # Values given with -e were not copied. Set these in .env: POSTGRES_PASSWORD\n\
             services:\n  db:\n    image: postgres:17\n    ports:\n      - \"5432:5432\"\n    environment:\n      POSTGRES_PASSWORD: ${POSTGRES_PASSWORD}\n    volumes:\n      - pg:/var/lib/postgresql/data\nvolumes:\n  pg:\n"
        );
        let v = yaml(&out);
        assert_eq!(v["services"]["db"]["image"], Value::from("postgres:17"));
        assert_eq!(v["services"]["db"]["ports"][0], Value::from("5432:5432"));
    }

    #[test]
    fn env_values_never_reach_the_output_in_any_flag_spelling() {
        let secrets = ["hunter2-s3cr3t", "tok_abc123XYZ", "pa ss;word$1", "k=v=w"];
        let cmd = format!(
            "docker run -e A={} --env B={} -eC='{}' --env=D='{}' -e E app:1",
            secrets[0], secrets[1], secrets[2], secrets[3]
        );
        let out = convert(&cmd).expect("convert");
        for s in secrets {
            assert!(!out.contains(s), "{s} leaked into:\n{out}");
        }
        for k in ["A", "B", "C", "D", "E"] {
            assert!(
                out.contains(&format!("{k}: ${{{k}}}")),
                "{k} missing:\n{out}"
            );
        }
        assert!(out.contains("Set these in .env: A, B, C, D, E"));
        assert!(!out.contains("=v=w"));
    }

    #[test]
    fn error_messages_do_not_repeat_env_values() {
        let m = err_text("docker run -e BAD-NAME=topsecretvalue app:1");
        assert!(!m.contains("topsecretvalue"), "{m}");
        let m = err_text("docker run -e GOOD=topsecretvalue --rm app:1");
        assert!(!m.contains("topsecretvalue"), "{m}");
    }

    #[test]
    fn repeated_env_keys_are_listed_once() {
        let out = convert("docker run -e A=1 -e A=2 app").expect("convert");
        assert_eq!(out.matches("A: ${A}").count(), 1);
    }

    #[test]
    fn long_and_attached_spellings_match_the_short_ones() {
        let a = convert("docker run --publish 8080:80 --volume ./site:/usr/share/nginx/html:ro --name=web --restart=unless-stopped nginx").expect("a");
        let b = convert("docker run -p8080:80 -v./site:/usr/share/nginx/html:ro --name web --restart unless-stopped nginx").expect("b");
        assert_eq!(a, b);
        let v = yaml(&a);
        assert_eq!(
            v["services"]["web"]["restart"],
            Value::from("unless-stopped")
        );
        assert_eq!(
            v["services"]["web"]["volumes"][0],
            Value::from("./site:/usr/share/nginx/html:ro")
        );
        assert!(
            v.get("volumes").is_none(),
            "a bind mount is not a named volume"
        );
    }

    #[test]
    fn restart_policy_with_a_retry_count_is_kept() {
        let out = convert("docker run --restart on-failure:3 app").expect("convert");
        assert_eq!(
            yaml(&out)["services"]["app"]["restart"],
            Value::from("on-failure:3")
        );
    }

    #[test]
    fn host_network_becomes_network_mode() {
        let out = convert("docker run --network host app:1").expect("convert");
        let v = yaml(&out);
        assert_eq!(v["services"]["app"]["network_mode"], Value::from("host"));
        assert!(v.get("networks").is_none());
    }

    #[test]
    fn a_named_network_is_declared_external() {
        let out = convert("docker run --network backend app:1").expect("convert");
        let v = yaml(&out);
        assert_eq!(v["services"]["app"]["networks"][0], Value::from("backend"));
        assert_eq!(v["networks"]["backend"]["external"], Value::from(true));
    }

    #[test]
    fn ip_and_protocol_ports_and_anonymous_volumes_survive() {
        let out =
            convert("docker run -p 127.0.0.1:5353:53/udp -p 80 -v /data app").expect("convert");
        let v = yaml(&out);
        assert_eq!(
            v["services"]["app"]["ports"][0],
            Value::from("127.0.0.1:5353:53/udp")
        );
        assert_eq!(v["services"]["app"]["ports"][1], Value::from("80"));
        assert_eq!(v["services"]["app"]["volumes"][0], Value::from("/data"));
        assert!(v.get("volumes").is_none());
    }

    #[test]
    fn arguments_after_the_image_are_the_command() {
        let out = convert("docker run -p 6379:6379 redis:7 redis-server --appendonly yes")
            .expect("convert");
        let v = yaml(&out);
        let cmd: Vec<&str> = v["services"]["redis"]["command"]
            .as_sequence()
            .expect("seq")
            .iter()
            .filter_map(Value::as_str)
            .collect();
        assert_eq!(cmd, ["redis-server", "--appendonly", "yes"]);
    }

    #[test]
    fn a_command_with_spaces_and_quotes_stays_one_item() {
        let out =
            convert(r#"docker run app:1 sh -c "echo \"hi there\"; sleep 1""#).expect("convert");
        let v = yaml(&out);
        assert_eq!(
            v["services"]["app"]["command"][2],
            Value::from("echo \"hi there\"; sleep 1")
        );
    }

    #[test]
    fn service_name_comes_from_the_image_when_there_is_no_name_flag() {
        let n = |cmd: &str| {
            let v = yaml(&convert(cmd).expect("convert"));
            v["services"]
                .as_mapping()
                .expect("map")
                .keys()
                .next()
                .expect("key")
                .as_str()
                .expect("str")
                .to_string()
        };
        assert_eq!(n("docker run postgres:17"), "postgres");
        assert_eq!(n("docker run ghcr.io/Org/My_App:1.2"), "my_app");
        assert_eq!(n("docker run redis@sha256:abc"), "redis");
        assert_eq!(n("docker run localhost:5000/team/api"), "api");
    }

    #[test]
    fn accepts_docker_container_run_a_prompt_and_line_continuations() {
        let one = convert("docker run -d -p 80:80 nginx").expect("one");
        assert_eq!(
            convert("docker container run -d -p 80:80 nginx").expect("two"),
            one
        );
        assert_eq!(
            convert("$ docker run -d -p 80:80 nginx").expect("three"),
            one
        );
        assert_eq!(
            convert("docker run -d \\\n  -p 80:80 \\\n  nginx").expect("four"),
            one
        );
        assert_eq!(convert("run -d -p 80:80 nginx").expect("five"), one);
    }

    #[test]
    fn unsupported_flags_are_refused_by_name() {
        for (cmd, flag) in [
            ("docker run --rm app", "--rm"),
            ("docker run -it app", "-it"),
            ("docker run --privileged app", "--privileged"),
            ("docker run --env-file .env app", "--env-file"),
            ("docker run -u 1000 app", "-u"),
            ("docker run --memory=1g app", "--memory"),
            ("docker run -dp 80:80 app", "-dp"),
        ] {
            let m = err_text(cmd);
            assert!(m.contains(flag), "{cmd}: {m}");
        }
    }

    #[test]
    fn flags_after_the_image_belong_to_the_command() {
        let out = convert("docker run app:1 --rm").expect("convert");
        assert_eq!(
            yaml(&out)["services"]["app"]["command"][0],
            Value::from("--rm")
        );
    }

    #[test]
    fn broken_commands_are_refused() {
        assert!(err_text("").contains("no image"));
        assert!(err_text("docker run -p 80:80").contains("no image"));
        assert!(err_text("docker run -p").contains("needs a value"));
        assert!(err_text("docker ps").contains("docker run"));
        assert!(err_text("docker run \"app").contains("quotes"));
        assert!(err_text("docker run --restart sometimes app").contains("restart"));
        assert!(err_text("docker run -p 80:8$0 app").contains("port"));
        assert!(err_text("docker run -v bad/name:/x app").contains("volume name"));
        assert!(err_text("docker run --name 'a b' app").contains("service name"));
    }
}
