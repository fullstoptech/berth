use bytesize::ByteSize;
use chrono::{DateTime, Utc};

use crate::container::{PortMapping, Proto};

#[must_use]
#[allow(clippy::cast_precision_loss)]
pub fn bytes(b: ByteSize) -> String {
    let v = b.as_u64() as f64;
    let units = ["B", "KB", "MB", "GB", "TB", "PB"];
    let mut scaled = v;
    let mut unit = 0_usize;
    while scaled >= 1000.0 && unit < units.len() - 1 {
        scaled /= 1000.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{} B", b.as_u64())
    } else if scaled >= 100.0 {
        format!("{scaled:.0} {}", units[unit])
    } else {
        let s = format!("{scaled:.1}");
        let s = s.strip_suffix(".0").unwrap_or(&s);
        format!("{s} {}", units[unit])
    }
}

fn duration_parts(from: DateTime<Utc>, now: DateTime<Utc>) -> (i64, &'static str) {
    let secs = now.signed_duration_since(from).num_seconds().max(0);
    if secs < 60 {
        (secs, "s")
    } else if secs < 3600 {
        (secs / 60, "min")
    } else if secs < 86_400 {
        (secs / 3600, "h")
    } else {
        (secs / 86_400, "d")
    }
}

#[must_use]
pub fn uptime(since: DateTime<Utc>, now: DateTime<Utc>) -> String {
    let (n, unit) = duration_parts(since, now);
    format!("{n} {unit}")
}

/// A memory limit in binary units labelled "MB" (D19): `256m` shows as
/// "256 MB", 1.5 GiB as "1.5 GB". Image and disk sizes use [`bytes`].
#[must_use]
#[allow(clippy::cast_precision_loss)]
pub fn memory(b: ByteSize) -> String {
    let units = ["B", "KB", "MB", "GB", "TB"];
    let mut scaled = b.as_u64() as f64;
    let mut unit = 0_usize;
    while scaled >= 1024.0 && unit < units.len() - 1 {
        scaled /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        return format!("{} B", b.as_u64());
    }
    let text = format!("{scaled:.1}");
    let text = text.strip_suffix(".0").unwrap_or(&text);
    format!("{text} {}", units[unit])
}

#[must_use]
pub fn age(at: DateTime<Utc>, now: DateTime<Utc>) -> String {
    let secs = now.signed_duration_since(at).num_seconds().max(0);
    if secs < 60 {
        if secs == 1 {
            "1 second ago".to_string()
        } else {
            format!("{secs} seconds ago")
        }
    } else {
        let (n, unit) = duration_parts(at, now);
        format!("{n} {unit} ago")
    }
}

#[must_use]
pub fn port(p: &PortMapping) -> String {
    let proto = match p.proto {
        Proto::Tcp => "tcp",
        Proto::Udp => "udp",
        Proto::Sctp => "sctp",
    };
    match p.host_port {
        Some(host) => {
            let ip = p.host_ip.map_or_else(String::new, |ip| ip.to_string());
            format!("{ip}:{host}→{}/{proto}", p.container_port)
        }
        None => format!("{}/{proto}", p.container_port),
    }
}

#[must_use]
pub fn middle_truncate(s: &str, max_chars: usize) -> String {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= max_chars {
        return s.to_string();
    }
    if max_chars == 0 {
        return String::new();
    }
    if max_chars == 1 {
        return "…".to_string();
    }
    let keep = max_chars - 1;
    let head_len = keep / 2;
    let tail_len = keep - head_len;
    let head: String = chars[..head_len].iter().collect();
    let tail: String = chars[chars.len() - tail_len..].iter().collect();
    format!("{head}…{tail}")
}

#[must_use]
pub fn tilde(path: &std::path::Path, home: &std::path::Path) -> String {
    match path.strip_prefix(home) {
        Ok(rest) if rest.as_os_str().is_empty() => "~".to_string(),
        Ok(rest) => format!("~/{}", rest.display()),
        Err(_) => path.display().to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn at(secs: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(secs, 0).expect("timestamp")
    }

    #[test]
    fn bytes_decimal_units() {
        assert_eq!(bytes(ByteSize(512)), "512 B");
        assert_eq!(bytes(ByteSize(14_200_000_000)), "14.2 GB");
        assert_eq!(bytes(ByteSize::mb(184)), "184 MB");
        assert_eq!(bytes(ByteSize(9_800_000)), "9.8 MB");
        assert_eq!(bytes(ByteSize(1_500)), "1.5 KB");
    }

    #[test]
    fn uptime_largest_unit() {
        assert_eq!(uptime(at(0), at(45)), "45 s");
        assert_eq!(uptime(at(0), at(600)), "10 min");
        assert_eq!(uptime(at(0), at(2 * 3600)), "2 h");
        assert_eq!(uptime(at(0), at(3 * 86_400)), "3 d");
        assert_eq!(uptime(at(100), at(50)), "0 s");
    }

    #[test]
    fn age_seconds_then_units() {
        assert_eq!(age(at(0), at(1)), "1 second ago");
        assert_eq!(age(at(0), at(3)), "3 seconds ago");
        assert_eq!(age(at(0), at(120)), "2 min ago");
    }

    #[test]
    fn port_with_and_without_host() {
        let bound = PortMapping {
            host_ip: None,
            host_port: Some(5432),
            container_port: 5432,
            proto: Proto::Tcp,
        };
        assert_eq!(port(&bound), ":5432→5432/tcp");
        let exposed = PortMapping {
            host_port: None,
            ..bound
        };
        assert_eq!(port(&exposed), "5432/tcp");
    }

    #[test]
    fn middle_truncate_keeps_both_ends() {
        let out = middle_truncate("~/Dev/clients/acme/shop-api", 16);
        assert_eq!(out.chars().count(), 16);
        assert!(out.starts_with("~/Dev"), "{out}");
        assert!(out.ends_with("shop-api"), "{out}");
        assert!(out.contains('…'));
        assert_eq!(middle_truncate("short", 16), "short");
        assert_eq!(middle_truncate("anything", 0), "");
        assert_eq!(middle_truncate("anything", 1), "…");
    }

    #[test]
    fn memory_is_binary_units_labelled_mb() {
        assert_eq!(memory(ByteSize(256 << 20)), "256 MB");
        assert_eq!(memory(ByteSize(1_610_612_736)), "1.5 GB");
        assert_eq!(memory(ByteSize(512 << 10)), "512 KB");
        assert_eq!(memory(ByteSize(100)), "100 B");
        // Image and disk sizes stay decimal.
        assert_eq!(bytes(ByteSize(268_435_456)), "268 MB");
    }

    #[test]
    fn tilde_replaces_home_prefix() {
        let home = Path::new("/Users/me");
        assert_eq!(tilde(Path::new("/Users/me/Dev/app"), home), "~/Dev/app");
        assert_eq!(tilde(home, home), "~");
        assert_eq!(tilde(Path::new("/etc/hosts"), home), "/etc/hosts");
    }
}
