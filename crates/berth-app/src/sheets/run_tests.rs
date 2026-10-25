//! P4-T1 tests for the Run sheet: what it prefills from an image, and how
//! the typed text becomes a `ContainerSpec` or a field error.

use super::*;

use berth_core::{Arch, MountKind, Proto};

fn image(tag: &str) -> Image {
    Image {
        id: "sha256:0123456789abcdef".to_string(),
        repo_tags: if tag.is_empty() {
            vec![]
        } else {
            vec![tag.to_string()]
        },
        digests: Vec::new(),
        size: bytesize::ByteSize::mb(74),
        created: chrono::Utc::now(),
        arch: Arch::Aarch64,
        os: "linux".to_string(),
        used_by: Vec::new(),
        dangling: tag.is_empty(),
        engine: berth_core::EngineId("colima:default".to_string()),
    }
}

fn detail(image: &Image) -> ImageDetail {
    ImageDetail {
        image: image.clone(),
        layers: Vec::new(),
        env_keys: vec!["PATH".to_string(), "NGINX_VERSION".to_string()],
        exposed_ports: vec![80, 443],
    }
}

fn draft_for(tag: &str, taken: &[&str]) -> (RunDraft, ContainerSpec, Image) {
    let image = image(tag);
    let draft = RunDraft::prefill(&image, Some(&detail(&image)), taken);
    let base = base_spec(&image, &draft.name);
    (draft, base, image)
}

fn errors(result: Result<ContainerSpec, Vec<RunError>>) -> Vec<RunError> {
    result.expect_err("errors")
}

#[test]
fn run_sheet_prefills_name_ports_and_environment_from_the_image() {
    let (draft, _base, _image) = draft_for("nginx:alpine", &[]);
    assert_eq!(draft.name, "nginx");
    let ports: Vec<&str> = draft.ports.iter().map(|(_, p)| p.as_str()).collect();
    assert_eq!(ports, ["80", "443"]);
    let keys: Vec<(&str, bool)> = draft
        .env
        .iter()
        .map(|e| (e.key.as_str(), e.from_image))
        .collect();
    assert_eq!(keys, [("PATH", true), ("NGINX_VERSION", true)]);
    assert!(
        draft.env.iter().all(|e| e.value.is_empty()),
        "no value is read"
    );
    assert!(draft.mounts.is_empty() && draft.labels.is_empty());
}

#[test]
fn run_sheet_name_steps_past_a_container_that_has_it() {
    let (draft, ..) = draft_for("nginx:alpine", &["nginx", "nginx-2"]);
    assert_eq!(draft.name, "nginx-3");
}

#[test]
fn run_sheet_the_done_when_input_becomes_the_spec_that_serves_curl() {
    // Name berthfx-run1, port 56081 to 80, the fixture label typed in.
    let (mut draft, base, _image) = draft_for("nginx:alpine", &["server-db-1"]);
    draft.name = "berthfx-run1".to_string();
    let port_id = draft.ports[0].0;
    draft.set_port(port_id, "56081:80");
    let extra = draft.ports[1].0;
    draft.remove_port(extra);
    draft.add_label();
    let label_id = draft.labels[0].0;
    draft.set_label(label_id, "dev.berth.fixture=1");

    let spec = draft.to_spec(&base, &["server-db-1"]).expect("a spec");
    assert_eq!(spec.name, "berthfx-run1");
    assert_eq!(spec.image.reference, "nginx:alpine");
    assert_eq!(spec.ports.len(), 1);
    assert_eq!(
        (
            spec.ports[0].host_port,
            spec.ports[0].container_port,
            spec.ports[0].proto
        ),
        (Some(56081), 80, Proto::Tcp)
    );
    assert_eq!(
        spec.labels.get("dev.berth.fixture").map(String::as_str),
        Some("1")
    );
    // The image's own variables stay the image's: none was typed.
    assert!(spec.env.is_empty(), "{:?}", spec.env);
}

#[test]
fn run_sheet_a_typed_value_overrides_an_image_variable_and_a_new_one_is_added() {
    let (mut draft, base, _image) = draft_for("nginx:alpine", &[]);
    let version = draft.env[1].id;
    draft.set_env_value(version, "9.9");
    draft.add_env();
    let new_id = draft.env[2].id;
    draft.set_env_key(new_id, "MODE");
    draft.set_env_value(new_id, "test");
    let spec = draft.to_spec(&base, &[]).expect("a spec");
    assert_eq!(
        spec.env,
        [
            ("NGINX_VERSION".to_string(), "9.9".to_string()),
            ("MODE".to_string(), "test".to_string())
        ]
    );
}

#[test]
fn run_sheet_volumes_become_mounts() {
    let (mut draft, base, _image) = draft_for("nginx:alpine", &[]);
    draft.add_mount();
    draft.add_mount();
    let (a, b) = (draft.mounts[0].0, draft.mounts[1].0);
    draft.set_mount(a, "berthfx_data:/usr/share/nginx/html");
    draft.set_mount(b, "/tmp/site:/site:ro");
    let spec = draft.to_spec(&base, &[]).expect("a spec");
    assert_eq!(spec.mounts.len(), 2);
    assert_eq!(spec.mounts[0].kind, MountKind::Volume);
    assert_eq!(spec.mounts[0].volume_name.as_deref(), Some("berthfx_data"));
    assert_eq!(spec.mounts[1].kind, MountKind::Bind);
    assert!(spec.mounts[1].read_only);
}

#[test]
fn run_sheet_blank_rows_are_skipped_not_errors() {
    let (mut draft, base, _image) = draft_for("nginx:alpine", &[]);
    draft.add_port();
    draft.add_mount();
    draft.add_label();
    draft.add_env();
    let spec = draft.to_spec(&base, &[]).expect("a spec");
    assert_eq!(spec.ports.len(), 2, "the blank port is not a port");
    assert!(spec.mounts.is_empty() && spec.labels.is_empty() && spec.env.is_empty());
}

#[test]
fn run_sheet_names_each_wrong_field() {
    let (mut draft, base, _image) = draft_for("nginx:alpine", &["web"]);
    draft.name = "web".to_string();
    let port = draft.ports[0].0;
    draft.set_port(port, "eighty");
    draft.add_mount();
    let mount = draft.mounts[0].0;
    draft.set_mount(mount, "relative/path:/x");
    draft.add_label();
    let label = draft.labels[0].0;
    draft.set_label(label, "no-equals-sign");
    draft.add_env();
    let env = draft.env[2].id;
    draft.set_env_value(env, "orphan value");

    let found = errors(draft.to_spec(&base, &["web"]));
    let fields: Vec<RunField> = found.iter().map(|e| e.field).collect();
    assert!(fields.contains(&RunField::Name));
    assert!(fields.contains(&RunField::Port(port)));
    assert!(fields.contains(&RunField::Mount(mount)));
    assert!(fields.contains(&RunField::Label(label)));
    assert!(fields.contains(&RunField::EnvKey(env)));
    let name = found
        .iter()
        .find(|e| e.field == RunField::Name)
        .expect("name");
    assert_eq!(name.message, "web is already a container on this engine");
    assert!(found.iter().all(|e| !e.message.is_empty()));
}

#[test]
fn run_sheet_refuses_an_empty_or_malformed_name_and_a_repeated_variable() {
    let (mut draft, base, _image) = draft_for("nginx:alpine", &[]);
    draft.name = "  ".to_string();
    assert_eq!(
        errors(draft.to_spec(&base, &[]))[0].message,
        "Give the container a name"
    );
    draft.name = "-bad".to_string();
    assert!(errors(draft.to_spec(&base, &[]))[0]
        .message
        .starts_with("Use letters"));

    draft.name = "ok".to_string();
    draft.add_env();
    draft.add_env();
    let (a, b) = (draft.env[2].id, draft.env[3].id);
    draft.set_env_key(a, "MODE");
    draft.set_env_key(b, "MODE");
    draft.set_env_value(a, "1");
    draft.set_env_value(b, "2");
    let found = errors(draft.to_spec(&base, &[]));
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].field, RunField::EnvKey(b));
    assert_eq!(found[0].message, "MODE is set twice");
}

#[test]
fn run_sheet_an_image_variable_row_cannot_be_renamed() {
    let (mut draft, ..) = draft_for("nginx:alpine", &[]);
    let id = draft.env[0].id;
    draft.set_env_key(id, "HACKED");
    assert_eq!(draft.env[0].key, "PATH");
}

#[test]
fn run_sheet_a_typed_value_never_reaches_debug_output() {
    let (mut draft, ..) = draft_for("nginx:alpine", &[]);
    let id = draft.env[0].id;
    draft.set_env_value(id, "hunter2-example");
    let printed = format!("{draft:?}");
    assert!(!printed.contains("hunter2-example"), "{printed}");
}

#[test]
fn run_sheet_labels_parse_key_equals_value() {
    assert_eq!(
        parse_label("dev.berth.fixture=1"),
        Ok(("dev.berth.fixture".to_string(), "1".to_string()))
    );
    assert_eq!(
        parse_label(" a = b c "),
        Ok(("a".to_string(), "b c".to_string()))
    );
    assert_eq!(
        parse_label("empty="),
        Ok(("empty".to_string(), String::new()))
    );
    assert!(parse_label("novalue").unwrap_err().contains("key=value"));
    assert!(parse_label("=x").unwrap_err().contains("key"));
    assert!(parse_label("a b=c").unwrap_err().contains("spaces"));
}

#[test]
fn run_sheet_a_dangling_image_runs_by_id_and_is_named_container() {
    let image = image("");
    let draft = RunDraft::prefill(&image, None, &[]);
    assert_eq!(draft.name, "container");
    let base = base_spec(&image, &draft.name);
    let spec = draft.to_spec(&base, &[]).expect("a spec");
    assert_eq!(spec.image.reference, "sha256:0123456789abcdef");
    assert!(spec.ports.is_empty());
}
