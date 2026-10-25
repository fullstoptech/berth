//! P5-T3 tests for "Write to compose.yaml instead": a changed host port, restart policy and limits
//! become edits of the service block; anything else is refused with its name.

use std::path::Path;

use berth_core::{
    ComposeCommand, ContainerSpec, ImageRef, Limits, PortMapping, Proto, RestartPolicy,
};
use bytesize::ByteSize;

use super::compose_write_plan;

const COMPOSE: &str = "\
# the shop
services:
  api:
    image: shop/api:1
    ports:
      - \"3000:3000\"
    restart: unless-stopped
  db:
    image: postgres:17-alpine
    ports:
      - \"5432:5432\"
";

fn port(host: u16, container: u16) -> PortMapping {
    PortMapping {
        host_ip: None,
        host_port: Some(host),
        container_port: container,
        proto: Proto::Tcp,
    }
}

fn spec() -> ContainerSpec {
    ContainerSpec {
        name: "shop-api-1".to_string(),
        image: ImageRef {
            reference: "shop/api:1".to_string(),
            id: None,
        },
        command: None,
        entrypoint: None,
        env: vec![("A".to_string(), "1".to_string())],
        ports: vec![port(3000, 3000)],
        mounts: Vec::new(),
        networks: vec!["shop_default".to_string()],
        labels: std::collections::BTreeMap::new(),
        restart: RestartPolicy::UnlessStopped,
        limits: Limits::default(),
        platform: None,
        working_dir: None,
        user: None,
        healthcheck: None,
        cap_add: Vec::new(),
        cap_drop: Vec::new(),
        network_aliases: std::collections::BTreeMap::new(),
    }
}

fn write(old: &ContainerSpec, new: &ContainerSpec) -> Result<berth_core::GenerationPlan, String> {
    compose_write_plan(
        Path::new("/x/shop"),
        Path::new("compose.yaml"),
        COMPOSE,
        "api",
        old,
        new,
    )
}

fn after(plan: &berth_core::GenerationPlan) -> String {
    plan.files[0].new_text.clone().expect("new text")
}

#[test]
fn review_changes_compose_write_moves_one_host_port_and_keeps_the_rest_of_the_file() {
    let mut new = spec();
    new.ports = vec![port(3100, 3000)];
    let plan = write(&spec(), &new).expect("plan");
    let text = after(&plan);
    assert!(text.contains("3100:3000"), "{text}");
    assert!(!text.contains("\"3000:3000\""), "{text}");
    assert!(
        text.starts_with("# the shop\n"),
        "the comment stays: {text}"
    );
    assert!(
        text.contains("5432:5432"),
        "the other service is untouched: {text}"
    );
    assert_eq!(plan.files.len(), 1);
    assert_eq!(plan.files[0].path, Path::new("compose.yaml"));
    assert_eq!(plan.root, Path::new("/x/shop"));
    assert!(
        plan.files[0].old_sha256.is_some(),
        "the write is guarded by the file's hash"
    );
}

#[test]
fn review_changes_compose_write_recreates_only_the_service_afterwards() {
    let mut new = spec();
    new.restart = RestartPolicy::Always;
    let plan = write(&spec(), &new).expect("plan");
    assert_eq!(
        plan.after_write,
        Some(ComposeCommand::Up {
            services: vec!["api".to_string()],
            build: false
        })
    );
    assert!(after(&plan).contains("restart: always"), "{}", after(&plan));
}

#[test]
fn review_changes_compose_write_restart_no_is_quoted_so_yaml_does_not_read_false() {
    let mut new = spec();
    new.restart = RestartPolicy::No;
    let text = after(&write(&spec(), &new).expect("plan"));
    assert!(text.contains("restart: \"no\""), "{text}");
    let mut new = spec();
    new.restart = RestartPolicy::OnFailure { max_retries: 3 };
    assert!(after(&write(&spec(), &new).expect("plan")).contains("restart: on-failure:3"));
}

#[test]
fn review_changes_compose_write_limits_become_service_keys() {
    let mut new = spec();
    new.limits = Limits {
        cpus: Some(0.5),
        memory: Some(ByteSize::mib(512)),
        memory_swap: Some(ByteSize::gib(1)),
    };
    let text = after(&write(&spec(), &new).expect("plan"));
    assert!(text.contains("cpus: 0.5"), "{text}");
    assert!(text.contains("mem_limit: 512m"), "{text}");
    assert!(text.contains("memswap_limit: 1g"), "{text}");
}

#[test]
fn review_changes_compose_write_refuses_what_a_scalar_cannot_say_and_names_it() {
    let mut new = spec();
    new.env.push(("B".to_string(), "2".to_string()));
    let message = write(&spec(), &new).expect_err("env is not written here");
    assert!(message.contains("environment variables"), "{message}");

    let mut new = spec();
    new.ports.push(port(3001, 3001));
    let message = write(&spec(), &new).expect_err("a new port");
    assert!(message.contains("published port"), "{message}");

    let mut old = spec();
    old.limits.memory = Some(ByteSize::mib(256));
    let message = write(&old, &spec()).expect_err("a removed limit");
    assert!(message.contains("Removing a limit"), "{message}");
}

#[test]
fn review_changes_compose_write_with_no_difference_has_nothing_to_write() {
    let message = write(&spec(), &spec()).expect_err("nothing differs");
    assert!(message.contains("Nothing differs"), "{message}");
}

#[test]
fn review_changes_compose_write_a_service_the_file_lacks_is_an_error_not_a_plan() {
    let mut new = spec();
    new.restart = RestartPolicy::Always;
    let result = compose_write_plan(
        Path::new("/x/shop"),
        Path::new("compose.yaml"),
        COMPOSE,
        "worker",
        &spec(),
        &new,
    );
    assert!(result.expect_err("no such service").contains("worker"));
}
