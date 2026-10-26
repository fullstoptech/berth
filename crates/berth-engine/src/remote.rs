//! Remote engine transports (P8-T1, PLAN §6.5 and D47): the pure helpers
//! behind SSH and TCP+TLS engines. `BollardEngine::connect` does the actual
//! connecting; everything testable without a network lives here.

use std::path::{Path, PathBuf};

use crate::EngineError;

/// One concrete host from `~/.ssh/config`: a `Host` pattern without wildcards
/// plus the options of its block. Global options (before the first `Host`)
/// are defaults for every host and are not reported.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SshHostEntry {
    pub name: String,
    pub hostname: Option<String>,
    pub user: Option<String>,
    pub port: Option<u16>,
    pub identity_files: Vec<PathBuf>,
}

/// Parse `~/.ssh/config` text into its concrete hosts. `Host` patterns with
/// `*`, `?` or a leading `!` are skipped (they name no single host); a block
/// may name several hosts, and each gets the block's options. As in OpenSSH,
/// the first value of an option inside a block wins; `IdentityFile` collects
/// every value. A `Match` block ends the current host block.
pub fn parse_ssh_config(text: &str) -> Vec<SshHostEntry> {
    #[derive(Default, Clone)]
    struct Block {
        names: Vec<String>,
        hostname: Option<String>,
        user: Option<String>,
        port: Option<u16>,
        identity_files: Vec<PathBuf>,
    }

    fn flush(block: Block, out: &mut Vec<SshHostEntry>) {
        for name in block.names {
            out.push(SshHostEntry {
                name,
                hostname: block.hostname.clone(),
                user: block.user.clone(),
                port: block.port,
                identity_files: block.identity_files.clone(),
            });
        }
    }

    let mut out = Vec::new();
    let mut block: Option<Block> = None;
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        // ssh_config accepts `Keyword value` and `Keyword=value`.
        let (keyword, args) = match line.find(|c: char| c == '=' || c.is_whitespace()) {
            Some(i) => (&line[..i], line[i..].trim_start_matches(['=', ' ', '\t'])),
            None => (line, ""),
        };
        let mut words = args.split_whitespace();
        match keyword.to_ascii_lowercase().as_str() {
            "host" => {
                if let Some(done) = block.take() {
                    flush(done, &mut out);
                }
                block = Some(Block {
                    names: words
                        .filter(|p| !p.starts_with('!') && !p.contains(['*', '?']))
                        .map(str::to_string)
                        .collect(),
                    ..Block::default()
                });
            }
            "match" => {
                if let Some(done) = block.take() {
                    flush(done, &mut out);
                }
            }
            "hostname" => {
                if let (Some(b), Some(v)) = (&mut block, words.next()) {
                    if b.hostname.is_none() {
                        b.hostname = Some(v.to_string());
                    }
                }
            }
            "user" => {
                if let (Some(b), Some(v)) = (&mut block, words.next()) {
                    if b.user.is_none() {
                        b.user = Some(v.to_string());
                    }
                }
            }
            "port" => {
                if let (Some(b), Some(v)) = (&mut block, words.next()) {
                    if b.port.is_none() {
                        b.port = v.parse().ok();
                    }
                }
            }
            "identityfile" => {
                if let (Some(b), Some(v)) = (&mut block, words.next()) {
                    b.identity_files.push(PathBuf::from(v));
                }
            }
            _ => {}
        }
    }
    if let Some(done) = block.take() {
        flush(done, &mut out);
    }
    out
}

/// Parse an ssh destination as typed by a user: `[user@]host[:port]`.
/// Returns `None` for an empty destination or an empty host.
pub fn parse_ssh_target(target: &str) -> Option<berth_core::RemoteTransport> {
    let target = target.trim();
    if target.is_empty() {
        return None;
    }
    let (user, host_port) = match target.split_once('@') {
        Some((u, rest)) if !u.is_empty() => (Some(u.to_string()), rest),
        _ => (None, target),
    };
    let (host, port) = match host_port.rsplit_once(':') {
        Some((h, p)) => match p.parse::<u16>() {
            Ok(p) => (h, Some(p)),
            Err(_) => (host_port, None),
        },
        None => (host_port, None),
    };
    if host.is_empty() {
        return None;
    }
    Some(berth_core::RemoteTransport::Ssh {
        host: host.to_string(),
        user,
        port,
    })
}

/// The `DOCKER_HOST` string for a unix socket (D47).
pub fn docker_host_unix(path: &Path) -> String {
    format!("unix://{}", path.display())
}

/// The `DOCKER_HOST` string for a remote transport (D47): Compose and
/// "Open in Terminal app" get exactly this value for remote engines.
pub fn docker_host_for_transport(transport: &berth_core::RemoteTransport) -> String {
    match transport {
        berth_core::RemoteTransport::Ssh { host, user, port } => {
            let user = user.as_ref().map_or(String::new(), |u| format!("{u}@"));
            let port = port.map_or(String::new(), |p| format!(":{p}"));
            format!("ssh://{user}{host}{port}")
        }
        berth_core::RemoteTransport::Tcp { url, .. } => url.to_string(),
    }
}

/// The `DOCKER_HOST` string an engine kind implies, if it implies one.
/// Colima engines get theirs from the profile's socket at discovery time.
pub fn docker_host_for_kind(kind: &berth_core::EngineKind) -> Option<String> {
    match kind {
        berth_core::EngineKind::Socket { path, .. } => Some(docker_host_unix(path)),
        berth_core::EngineKind::Remote { transport } => Some(docker_host_for_transport(transport)),
        berth_core::EngineKind::Colima { .. } => None,
    }
}

/// Check that the TLS files of a TCP engine can be read before connecting, so
/// a bad path reports the file instead of a handshake failure.
pub fn validate_tls_files(tls: &berth_core::TlsFiles) -> Result<(), EngineError> {
    for (what, path) in [
        ("TLS CA file", &tls.ca),
        ("TLS certificate file", &tls.cert),
        ("TLS key file", &tls.key),
    ] {
        let describe = |message: String| EngineError::Parse {
            what: format!("{what} {}", path.display()),
            message,
        };
        match std::fs::metadata(path) {
            Ok(m) if m.is_file() => {
                std::fs::File::open(path).map_err(|e| describe(e.to_string()))?;
            }
            Ok(_) => return Err(describe("not a regular file".to_string())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(describe("file not found".to_string()));
            }
            Err(e) => return Err(describe(e.to_string())),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const CONFIG: &str = "\
# leading comment
User globaluser

Host *
    ServerAliveInterval 30

Host hetzner-1
    HostName 203.0.113.10
    User deploy
    Port 2222
    IdentityFile ~/.ssh/hetzner
    IdentityFile ~/.ssh/hetzner2

Host *.internal !excluded plain
    hostname=plain.example.com
    USER ops

Match user bob
    HostName should.not.appear

Host minimal
";

    #[test]
    fn remote_parse_ssh_config_reads_hosts_and_options() {
        let hosts = parse_ssh_config(CONFIG);
        let names: Vec<&str> = hosts.iter().map(|h| h.name.as_str()).collect();
        assert_eq!(names, ["hetzner-1", "plain", "minimal"]);
        let h = &hosts[0];
        assert_eq!(h.hostname.as_deref(), Some("203.0.113.10"));
        assert_eq!(h.user.as_deref(), Some("deploy"));
        assert_eq!(h.port, Some(2222));
        assert_eq!(
            h.identity_files,
            [
                PathBuf::from("~/.ssh/hetzner"),
                PathBuf::from("~/.ssh/hetzner2")
            ]
        );
        // `Key=value` and mixed-case keywords parse the same.
        let plain = &hosts[1];
        assert_eq!(plain.hostname.as_deref(), Some("plain.example.com"));
        assert_eq!(plain.user.as_deref(), Some("ops"));
        // Global defaults before the first Host do not leak into entries.
        assert_eq!(hosts[2].user, None);
        assert_eq!(hosts[2].hostname, None);
    }

    #[test]
    fn remote_parse_ssh_config_first_value_wins_and_bad_port_is_ignored() {
        let hosts = parse_ssh_config("Host a\n  HostName one\n  HostName two\n  Port nope\n");
        assert_eq!(hosts.len(), 1);
        assert_eq!(hosts[0].hostname.as_deref(), Some("one"));
        assert_eq!(hosts[0].port, None);
        assert!(parse_ssh_config("").is_empty());
        assert!(parse_ssh_config("Host *\n  User x\n").is_empty());
    }

    #[test]
    fn remote_parse_ssh_target_handles_user_host_and_port() {
        assert_eq!(
            parse_ssh_target("deploy@hetzner-1:2222"),
            Some(berth_core::RemoteTransport::Ssh {
                host: "hetzner-1".into(),
                user: Some("deploy".into()),
                port: Some(2222),
            })
        );
        assert_eq!(
            parse_ssh_target("hetzner-1"),
            Some(berth_core::RemoteTransport::Ssh {
                host: "hetzner-1".into(),
                user: None,
                port: None,
            })
        );
        assert_eq!(parse_ssh_target(""), None);
        assert_eq!(parse_ssh_target("ops@"), None);
        // A non-numeric suffix stays part of the host (e.g. a zone id), no port.
        match parse_ssh_target("host:zone") {
            Some(berth_core::RemoteTransport::Ssh { host, port, .. }) => {
                assert_eq!(host, "host:zone");
                assert_eq!(port, None);
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn remote_docker_host_strings_match_d47() {
        let ssh = berth_core::RemoteTransport::Ssh {
            host: "hetzner-1".into(),
            user: Some("deploy".into()),
            port: Some(2222),
        };
        assert_eq!(
            docker_host_for_transport(&ssh),
            "ssh://deploy@hetzner-1:2222"
        );
        let ssh_bare = berth_core::RemoteTransport::Ssh {
            host: "hetzner-1".into(),
            user: None,
            port: None,
        };
        assert_eq!(docker_host_for_transport(&ssh_bare), "ssh://hetzner-1");
        let tcp = berth_core::RemoteTransport::Tcp {
            url: "tcp://203.0.113.10:2376".parse().unwrap(),
            tls: None,
        };
        assert_eq!(docker_host_for_transport(&tcp), "tcp://203.0.113.10:2376");
        assert_eq!(
            docker_host_unix(Path::new("/run/docker.sock")),
            "unix:///run/docker.sock"
        );
        assert_eq!(
            docker_host_for_kind(&berth_core::EngineKind::Remote { transport: ssh }),
            Some("ssh://deploy@hetzner-1:2222".to_string())
        );
        assert_eq!(
            docker_host_for_kind(&berth_core::EngineKind::Colima {
                profile: "default".into()
            }),
            None
        );
    }

    #[test]
    fn remote_validate_tls_files_names_the_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        let cert = dir.path().join("cert.pem");
        let key = dir.path().join("key.pem");
        std::fs::write(&cert, "cert").unwrap();
        std::fs::write(&key, "key").unwrap();
        let tls = berth_core::TlsFiles {
            ca: dir.path().join("ca.pem"),
            cert: cert.clone(),
            key: key.clone(),
        };
        let err = validate_tls_files(&tls).unwrap_err();
        let text = err.to_string();
        assert!(text.contains("TLS CA file"), "{text}");
        assert!(text.contains("ca.pem"), "{text}");
        assert!(text.contains("not found"), "{text}");
        // A directory where a file belongs is its own clear error.
        std::fs::write(dir.path().join("ca.pem"), "ca").unwrap();
        let tls_dir = berth_core::TlsFiles {
            ca: dir.path().to_path_buf(),
            cert,
            key,
        };
        let err = validate_tls_files(&tls_dir).unwrap_err();
        assert!(err.to_string().contains("not a regular file"), "{err}");
        // All three present and readable passes.
        let tls_ok = berth_core::TlsFiles {
            ca: dir.path().join("ca.pem"),
            cert: tls_dir.cert.clone(),
            key: tls_dir.key.clone(),
        };
        validate_tls_files(&tls_ok).unwrap();
    }
}
