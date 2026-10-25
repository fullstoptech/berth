//! D16 to D19 tests: the run settings a spec carries, what it cannot carry,
//! the diff of both, memory in binary units and `EngineAction::Recreate`.

use std::collections::BTreeMap;

use bytesize::ByteSize;
use chrono::{DateTime, Utc};

use crate::container::{
    needs_recreate, spec_diff, Container, ContainerDetail, ContainerSpec, ContainerState, EnvVar,
    HealthCheck, ImageRef, Limits, RestartPolicy,
};
use crate::ids::{ContainerId, EngineId};
use crate::problem::EngineAction;

type Change = fn(&mut ContainerSpec);

fn detail_of(inspect: serde_json::Value) -> ContainerDetail {
    ContainerDetail {
        container: Container {
            id: ContainerId("0123456789abcdef0123".to_string()),
            engine: EngineId("e".to_string()),
            name: "web".to_string(),
            project: None,
            compose_project: None,
            service: None,
            image: ImageRef {
                reference: "nginx:alpine".to_string(),
                id: None,
            },
            command: None,
            state: ContainerState::Running,
            health: None,
            exit_code: None,
            restart_count: 0,
            created: DateTime::<Utc>::UNIX_EPOCH,
            started: None,
            ports: vec![],
            mounts: vec![],
            networks: vec!["shop_default".to_string()],
            labels: BTreeMap::new(),
            stats: None,
            platform: None,
        },
        entrypoint: None,
        cmd: Some(vec!["nginx".to_string()]),
        env: vec![EnvVar {
            key: "A".to_string(),
            value: "1".to_string(),
        }],
        working_dir: Some("/app".to_string()),
        user: Some("1000:1000".to_string()),
        restart: RestartPolicy::No,
        limits: Limits::default(),
        health_log: vec![],
        inspect,
    }
}

fn rich_inspect() -> serde_json::Value {
    serde_json::json!({
        "Config": {
            "Hostname": "0123456789ab",
            "Healthcheck": {
                "Test": ["CMD", "curl", "-f", "http://localhost"],
                "Interval": 30_000_000_000_u64,
                "Timeout": 3_000_000_000_u64,
                "Retries": 5,
                "StartPeriod": 10_000_000_000_u64
            }
        },
        "HostConfig": {
            "CapAdd": ["NET_ADMIN"],
            "CapDrop": ["MKNOD"],
            "IpcMode": "private",
            "Runtime": "runc",
            "NetworkMode": "shop_default",
            "LogConfig": { "Type": "json-file" },
            "PortBindings": { "80/tcp": [{ "HostIp": "127.0.0.1", "HostPort": "56080" }] }
        },
        "NetworkSettings": { "Networks": {
            "shop_default": { "Aliases": ["web", "0123456789ab"] },
            "bridge": { "Aliases": null }
        } }
    })
}

#[test]
fn from_detail_carries_the_run_settings() {
    let spec = ContainerSpec::from_detail(&detail_of(rich_inspect()));
    assert_eq!(spec.working_dir.as_deref(), Some("/app"));
    assert_eq!(spec.user.as_deref(), Some("1000:1000"));
    assert_eq!(spec.cap_add, ["NET_ADMIN"]);
    assert_eq!(spec.cap_drop, ["MKNOD"]);
    assert_eq!(
        spec.healthcheck,
        Some(HealthCheck {
            test: ["CMD", "curl", "-f", "http://localhost"]
                .into_iter()
                .map(String::from)
                .collect(),
            interval_s: Some(30),
            timeout_s: Some(3),
            retries: Some(5),
            start_period_s: Some(10),
        })
    );
    // Docker's own short-id alias is not carried; the service name is.
    assert_eq!(
        spec.network_aliases,
        BTreeMap::from([("shop_default".to_string(), vec!["web".to_string()])])
    );
    assert_eq!(spec.ports.len(), 1);
    assert_eq!(spec.ports[0].host_port, Some(56080));
    assert_eq!(spec.env, [("A".to_string(), "1".to_string())]);
}

#[test]
fn uncarried_is_empty_for_a_plain_container_and_names_the_rest() {
    assert_eq!(
        ContainerSpec::uncarried(&detail_of(rich_inspect())),
        Vec::<String>::new()
    );
    let mut inspect = rich_inspect();
    inspect["HostConfig"]["Privileged"] = serde_json::json!(true);
    inspect["HostConfig"]["Devices"] = serde_json::json!([{ "PathOnHost": "/dev/fuse" }]);
    inspect["HostConfig"]["Sysctls"] = serde_json::json!({ "net.core.somaxconn": "1024" });
    inspect["HostConfig"]["ExtraHosts"] = serde_json::json!(["db:10.0.0.2"]);
    inspect["HostConfig"]["Init"] = serde_json::json!(true);
    inspect["HostConfig"]["NetworkMode"] = serde_json::json!("host");
    inspect["Config"]["Tty"] = serde_json::json!(true);
    inspect["Config"]["Hostname"] = serde_json::json!("custom-host");
    let names = ContainerSpec::uncarried(&detail_of(inspect));
    for expected in [
        "privileged mode",
        "devices",
        "sysctls",
        "extra hosts",
        "init process",
        "network mode",
        "terminal (tty)",
        "hostname",
    ] {
        assert!(
            names.contains(&expected.to_string()),
            "{expected} in {names:?}"
        );
    }
}

#[test]
fn spec_diff_shows_changes_to_the_run_settings_and_they_need_a_recreate() {
    let old = ContainerSpec::from_detail(&detail_of(rich_inspect()));
    let changes: [(&str, Change); 6] = [
        ("working_dir", |s| s.working_dir = Some("/srv".into())),
        ("user", |s| s.user = None),
        ("healthcheck", |s| s.healthcheck = None),
        ("cap_add", |s| s.cap_add.push("SYS_TIME".into())),
        ("cap_drop", |s| s.cap_drop.clear()),
        ("network_aliases", |s| s.network_aliases.clear()),
    ];
    for (field, change) in changes {
        let mut new = old.clone();
        change(&mut new);
        let diff = spec_diff(&old, &new);
        assert_eq!(diff.len(), 1, "{field}: {diff:?}");
        assert_eq!(diff[0].field, field);
        assert!(needs_recreate(&old, &new), "{field}");
    }
}

#[test]
fn spec_diff_shows_memory_in_binary_units() {
    let old = ContainerSpec::from_detail(&detail_of(rich_inspect()));
    let mut new = old.clone();
    new.limits.memory = Some(ByteSize(268_435_456));
    let diff = spec_diff(&old, &new);
    assert!(diff[0].after.contains("memory=256 MB"), "{diff:?}");
    assert!(!needs_recreate(&old, &new));
}

#[test]
fn engine_action_recreate_round_trips_through_serde() {
    let action = EngineAction::Recreate {
        engine: EngineId("e".to_string()),
        id: ContainerId("c".to_string()),
        spec: ContainerSpec::from_detail(&detail_of(rich_inspect())),
        pull: true,
    };
    let json = serde_json::to_string(&action).expect("serialize");
    let back: EngineAction = serde_json::from_str(&json).expect("parse");
    assert_eq!(back, action);
}
