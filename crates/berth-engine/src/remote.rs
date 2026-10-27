//! Remote engines (PLAN D47, P8-T1): the `DOCKER_HOST` string of a
//! transport, its engine id, and the hosts `~/.ssh/config` offers.
//! All pure except [`ssh_hosts`], which reads that one file.

use berth_core::{EngineId, RemoteTransport};

/// The `DOCKER_HOST` of a remote transport: `ssh://[user@]host[:port]`
/// or the TCP URL as given. Compose and the terminal app use the same
/// string (D47).
#[must_use]
pub fn docker_host(transport: &RemoteTransport) -> String {
    match transport {
        RemoteTransport::Ssh { host, user, port } => {
            let user = user.as_ref().map_or(String::new(), |u| format!("{u}@"));
            let port = port.map_or(String::new(), |p| format!(":{p}"));
            format!("ssh://{user}{host}{port}")
        }
        RemoteTransport::Tcp { url, .. } => url.to_string(),
    }
}

/// A user-added remote engine's id, following the context ids in
/// `discovery`: `ssh:<host>` or `tcp:<host:port>`.
#[must_use]
pub fn engine_id(transport: &RemoteTransport) -> EngineId {
    match transport {
        RemoteTransport::Ssh { host, .. } => EngineId(format!("ssh:{host}")),
        RemoteTransport::Tcp { url, .. } => {
            let host = url.host_str().unwrap_or_default();
            match url.port() {
                Some(port) => EngineId(format!("tcp:{host}:{port}")),
                None => EngineId(format!("tcp:{host}")),
            }
        }
    }
}

/// One host from `~/.ssh/config`: the alias to connect with, plus the
/// fields the file spells out for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SshHost {
    pub name: String,
    pub hostname: Option<String>,
    pub user: Option<String>,
    pub port: Option<u16>,
}

/// Parses the `Host` blocks of an ssh config. Wildcard and negated
/// patterns are not hosts a person picks, so they never appear; a
/// block applies its `HostName`, `User` and `Port` lines to every
/// name on its `Host` line. Keywords are case-insensitive and may be
/// separated by `=` (ssh_config(5)).
#[must_use]
pub fn parse_ssh_config(text: &str) -> Vec<SshHost> {
    let mut hosts: Vec<SshHost> = Vec::new();
    // Indexes into `hosts` for the block currently being read.
    let mut block: Vec<usize> = Vec::new();
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (keyword, args) = match line.split_once([' ', '\t', '=']) {
            Some((k, rest)) => (k, rest.trim_start_matches([' ', '\t', '='])),
            None => (line, ""),
        };
        match keyword.to_ascii_lowercase().as_str() {
            "host" => {
                block.clear();
                for name in args.split_whitespace() {
                    if name.contains('*') || name.contains('?') || name.starts_with('!') {
                        continue;
                    }
                    if let Some(at) = hosts.iter().position(|h| h.name == name) {
                        block.push(at);
                    } else {
                        block.push(hosts.len());
                        hosts.push(SshHost {
                            name: name.to_string(),
                            hostname: None,
                            user: None,
                            port: None,
                        });
                    }
                }
            }
            "hostname" => {
                for &at in &block {
                    if hosts[at].hostname.is_none() {
                        hosts[at].hostname = Some(first_word(args));
                    }
                }
            }
            "user" => {
                for &at in &block {
                    if hosts[at].user.is_none() {
                        hosts[at].user = Some(first_word(args));
                    }
                }
            }
            "port" => {
                if let Ok(port) = first_word(args).parse::<u16>() {
                    for &at in &block {
                        if hosts[at].port.is_none() {
                            hosts[at].port = Some(port);
                        }
                    }
                }
            }
            _ => {}
        }
    }
    hosts
}

fn first_word(args: &str) -> String {
    args.split_whitespace()
        .next()
        .unwrap_or_default()
        .to_string()
}

/// The hosts of `~/.ssh/config`; a missing or unreadable file offers
/// none.
#[must_use]
pub fn ssh_hosts() -> Vec<SshHost> {
    let path = berth_sys::paths::home().join(".ssh/config");
    std::fs::read_to_string(path).map_or_else(|_| Vec::new(), |text| parse_ssh_config(&text))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ssh(host: &str, user: Option<&str>, port: Option<u16>) -> RemoteTransport {
        RemoteTransport::Ssh {
            host: host.to_string(),
            user: user.map(str::to_string),
            port,
        }
    }

    #[test]
    fn remote_docker_host_strings_for_ssh_and_tcp() {
        assert_eq!(docker_host(&ssh("prod-1", None, None)), "ssh://prod-1");
        assert_eq!(
            docker_host(&ssh("10.0.0.5", Some("deploy"), Some(2222))),
            "ssh://deploy@10.0.0.5:2222"
        );
        let tcp = RemoteTransport::Tcp {
            url: url::Url::parse("tcp://10.0.0.5:2376").expect("url"),
            tls: None,
        };
        assert_eq!(docker_host(&tcp), "tcp://10.0.0.5:2376");
        assert_eq!(
            engine_id(&ssh("prod-1", Some("deploy"), Some(2222))).0,
            "ssh:prod-1"
        );
        assert_eq!(engine_id(&tcp).0, "tcp:10.0.0.5:2376");
    }

    #[test]
    fn remote_ssh_config_parses_hosts_and_skips_patterns() {
        let text = "\
# a comment
Host *
    User everyone

Host prod-1 web
    HostName 10.0.0.5
    User=deploy
    Port 2222

Host *.internal !secret
    User nope

Host db
    hostname db.example.com
";
        let hosts = parse_ssh_config(text);
        let names: Vec<&str> = hosts.iter().map(|h| h.name.as_str()).collect();
        assert_eq!(names, ["prod-1", "web", "db"]);
        assert_eq!(hosts[0].hostname.as_deref(), Some("10.0.0.5"));
        assert_eq!(hosts[0].user.as_deref(), Some("deploy"));
        assert_eq!(hosts[0].port, Some(2222));
        assert_eq!(hosts[1].user.as_deref(), Some("deploy"));
        assert_eq!(hosts[2].hostname.as_deref(), Some("db.example.com"));
        assert_eq!(hosts[2].user, None);
    }

    #[test]
    fn remote_ssh_config_first_value_wins_and_garbage_is_ignored() {
        let hosts = parse_ssh_config(
            "Host a\n User first\n User second\n Port nope\n Port 22\nweird line\n",
        );
        assert_eq!(hosts.len(), 1);
        assert_eq!(hosts[0].user.as_deref(), Some("first"));
        assert_eq!(hosts[0].port, Some(22));
        assert!(parse_ssh_config("").is_empty());
    }
}
