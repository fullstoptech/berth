//! P3-T7 tests: the sheet's model (spec from a detail, the text forms, what
//! the sheet offers), the sheet in each state, the Limits tab, Duplicate's
//! name and the masked env in the diff.
//!
//! The env values below are made-up stand-ins; the fixture password lives
//! in `fixtures/` and `scripts/` only (CLAUDE.md).

use std::collections::BTreeMap;

use super::model::{
    diff_rows, duplicate_name, duplicate_spec, failure_text, format_memory, format_mount,
    format_port, join_command, not_carried, offer, parse_memory, parse_mount, parse_port,
    published_ports, removes_a_limit, rename_problem, spec_from_detail, split_command,
    takes_from_image, Draft, Field, Offer,
};
use super::*;
use crate::services::detail::{mask_inspect, provide_detail_store, Loaded};
use crate::state::{use_app_state, PendingKey, PendingState};
use berth_core::{
    Container, ContainerId, ContainerSpec, ImageRef, Limits, Mount, MountKind, PortMapping, Proto,
    RestartPolicy,
};
use berth_engine::EngineClient;
use bytesize::ByteSize;

const SECRET: &str = "ex-secret-zeta-4417";

fn world() -> berth_engine::MockWorld {
    berth_engine::MockWorld::bundled()
}

fn container(name: &str) -> Container {
    world()
        .containers
        .into_iter()
        .find(|c| c.name == name)
        .unwrap_or_else(|| panic!("{name} in the mock world"))
}

fn loaded_of(c: &Container) -> Loaded {
    let engine = berth_engine::MockEngine::bundled(&c.engine);
    let mut detail =
        futures::executor::block_on(engine.container(&c.id)).expect("the mock knows it");
    detail.container = c.clone();
    let inspect = mask_inspect(detail.inspect.clone());
    Loaded {
        detail,
        inspect,
        last_lines: Vec::new(),
    }
}

fn api() -> Container {
    container("shop-api-api-1")
}

fn api_spec() -> ContainerSpec {
    spec_from_detail(&loaded_of(&api()).detail)
}

// ---------------------------------------------------------------------
// Rendering the sheet
// ---------------------------------------------------------------------

/// A form edit. Compared as equal: the root component's props only need
/// to be comparable, and a test never swaps the function under it.
#[derive(Clone, Copy)]
struct Edits(fn(&mut Draft));

impl PartialEq for Edits {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}

#[derive(Clone, PartialEq)]
struct Sheet {
    container: Container,
    edits: Option<Edits>,
    pending: BTreeMap<PendingKey, PendingState>,
}

impl Sheet {
    fn of(container: &Container) -> Self {
        Self {
            container: container.clone(),
            edits: None,
            pending: BTreeMap::new(),
        }
    }

    fn edited(mut self, edits: fn(&mut Draft)) -> Self {
        self.edits = Some(Edits(edits));
        self
    }

    fn with_pending(mut self, action: &'static str, value: PendingState) -> Self {
        self.pending.insert(
            PendingKey {
                subject: berth_core::Subject::Container(self.container.id.clone()),
                action,
            },
            value,
        );
        self
    }
}

#[component]
fn SheetRoot(sheet: Sheet) -> Element {
    let state = use_app_state();
    use_context_provider(|| state);
    let store = provide_detail_store();
    if let Some(edits) = sheet.edits {
        use_context_provider(|| DevEditDraft(edits.0));
    }
    let seeded = sheet.clone();
    use_hook(move || {
        let mut state = state;
        state
            .containers
            .set(world().containers_for(&seeded.container.engine));
        state.engines.set(world().engines);
        state.pending.set(seeded.pending.clone());
        store.seed(&seeded.container, loaded_of(&seeded.container));
    });
    rsx! { EditRecreateSheet { id: sheet.container.id.clone() } }
}

fn strip_styles(html: &str) -> String {
    let mut out = html.to_string();
    while let (Some(a), Some(b)) = (out.find("<style>"), out.find("</style>")) {
        if b < a {
            break;
        }
        out.replace_range(a..b + "</style>".len(), "");
    }
    out
}

fn render(sheet: Sheet) -> String {
    let mut dom = VirtualDom::new_with_props(SheetRoot, SheetRootProps { sheet });
    let _ = dom.rebuild_to_vec();
    strip_styles(&dioxus_ssr::render(&dom))
}

fn primary_label(html: &str) -> &'static str {
    for label in ["Apply limits", "Recreate container"] {
        if html.contains(&format!("<span class=\"btn-label\">{label}</span>")) {
            return label;
        }
    }
    "none"
}

#[test]
fn edit_recreate_sheet_unchanged_offers_nothing_and_says_why() {
    let html = render(Sheet::of(&api()));
    assert!(
        html.contains("Nothing differs from the container yet."),
        "{html}"
    );
    assert!(
        html.contains("data-tooltip=\"Nothing has changed\""),
        "{html}"
    );
    assert!(!html.contains("What a recreate loses"), "{html}");
}

#[test]
fn edit_recreate_sheet_limits_only_change_offers_apply_limits() {
    let html = render(Sheet::of(&api()).edited(|d| {
        d.memory = "256m".to_string();
        d.restart = RestartKind::Always;
    }));
    assert_eq!(primary_label(&html), "Apply limits", "{html}");
    assert!(html.contains("Nothing is recreated."), "{html}");
    assert!(html.contains("limits"), "{html}");
    assert!(html.contains("restart"), "{html}");
    // No loss warning: nothing is lost by a live update.
    assert!(!html.contains("What a recreate loses"), "{html}");
}

#[test]
fn edit_recreate_sheet_port_change_offers_recreate_with_the_diff_and_the_loss_warning() {
    let html = render(Sheet::of(&api()).edited(|d| {
        let id = d.ports[0].0;
        d.set_port(id, "3001:3000");
    }));
    assert_eq!(primary_label(&html), "Recreate container", "{html}");
    assert!(html.contains("er-diff-field\">ports<"), "{html}");
    assert!(html.contains("3001"), "{html}");
    assert!(html.contains("What a recreate loses"), "{html}");
    assert!(
        html.contains("Files written inside the container, outside volumes, are lost."),
        "{html}"
    );
}

#[test]
fn edit_recreate_sheet_env_values_are_masked_in_the_form_and_the_diff() {
    let current = loaded_of(&api());
    let key = current.detail.env[0].key.clone();
    let html = render(Sheet::of(&api()).edited(|d| {
        let id = d.env[0].id;
        d.set_env_value(id, SECRET);
    }));
    assert_eq!(primary_label(&html), "Recreate container", "{html}");
    // Neither the typed value nor any value the container holds is in the page.
    assert!(!html.contains(SECRET), "{html}");
    for var in &current.detail.env {
        assert!(!html.contains(&var.value), "{} leaked: {html}", var.key);
    }
    // The diff says that a value changed and shows the key masked.
    assert!(html.contains(&format!("{key}=••••")), "{html}");
    assert!(html.contains("values changed (hidden)"), "{html}");
    assert!(html.contains("type=\"password\""), "{html}");
}

#[test]
fn edit_recreate_sheet_failed_recreate_says_the_old_container_was_kept() {
    let c = api();
    let html = render(
        Sheet::of(&c)
            .edited(|d| d.image = "berthfx/does-not-exist:1".to_string())
            .with_pending(
                "container.recreate",
                PendingState::Failed {
                    message: "No such image: berthfx/does-not-exist:1".to_string(),
                },
            ),
    );
    assert!(html.contains("The old container was kept."), "{html}");
    assert!(
        html.contains("No such image: berthfx/does-not-exist:1"),
        "{html}"
    );
    assert!(html.contains("aria-label=\"Dismiss message\""), "{html}");
}

#[test]
fn edit_recreate_sheet_running_recreate_shows_its_label_on_the_button() {
    let c = api();
    let html = render(
        Sheet::of(&c)
            .edited(|d| d.image = "nginx:alpine".to_string())
            .with_pending(
                "container.recreate",
                PendingState::Running {
                    label: "Recreating…".to_string(),
                },
            ),
    );
    assert!(html.contains("Recreating…"), "{html}");
    assert!(html.contains("aria-busy=\"true\""), "{html}");
}

#[test]
fn edit_recreate_sheet_compose_service_offers_to_write_compose_yaml_disabled_until_p5_t3() {
    let html = render(Sheet::of(&api()));
    assert!(html.contains("Write to compose.yaml instead"), "{html}");
    assert!(html.contains("data-tooltip=\"Built in P5-T3\""), "{html}");
    assert!(html.contains("shop-api"), "{html}");
    // A loose container has no such offer.
    let loose = world()
        .containers
        .into_iter()
        .find(|c| c.compose_project.is_none())
        .expect("a loose container in the mock world");
    assert!(!render(Sheet::of(&loose)).contains("Write to compose.yaml"));
}

#[test]
fn edit_recreate_sheet_a_bad_field_is_marked_and_blocks_the_button() {
    let html = render(Sheet::of(&api()).edited(|d| d.memory = "lots".to_string()));
    assert!(html.contains("Write a size such as 256m or 1.5g"), "{html}");
    assert!(
        html.contains("data-tooltip=\"Fix the fields marked in red first\""),
        "{html}"
    );
}

// ---------------------------------------------------------------------
// What the sheet offers
// ---------------------------------------------------------------------

#[test]
fn edit_recreate_offer_limits_and_restart_only_is_a_live_apply() {
    let old = api_spec();
    let mut new = old.clone();
    new.limits.memory = Some(ByteSize(256 << 20));
    new.restart = RestartPolicy::OnFailure { max_retries: 3 };
    assert_eq!(offer(&old, &new), Offer::ApplyLimits);
    assert_eq!(offer(&old, &old), Offer::Nothing);
}

#[test]
fn edit_recreate_offer_port_image_env_mount_and_network_changes_recreate() {
    let old = api_spec();
    let mut changed = Vec::new();
    let mut ports = old.clone();
    ports.ports[0].host_port = Some(3001);
    changed.push(ports);
    let mut image = old.clone();
    image.image = ImageRef {
        reference: "nginx:alpine".to_string(),
        id: None,
    };
    changed.push(image);
    let mut env = old.clone();
    env.env[0].1 = SECRET.to_string();
    changed.push(env);
    let mut mounts = old.clone();
    mounts.mounts.push(Mount {
        kind: MountKind::Volume,
        source: "data".to_string(),
        destination: "/data".to_string(),
        read_only: false,
        volume_name: Some("data".to_string()),
    });
    changed.push(mounts);
    let mut networks = old.clone();
    networks.networks.push("other".to_string());
    changed.push(networks);
    for new in changed {
        assert_eq!(
            offer(&old, &new),
            Offer::Recreate,
            "{:?}",
            diff_rows(&old, &new)
        );
    }
}

#[test]
fn edit_recreate_offer_removing_a_limit_needs_a_recreate() {
    let mut old = api_spec();
    old.limits.memory = Some(ByteSize(256 << 20));
    let mut new = old.clone();
    new.limits.memory = None;
    assert!(removes_a_limit(&old.limits, &new.limits));
    assert_eq!(offer(&old, &new), Offer::Recreate);
    // Raising a limit, or adding a cpu limit next to it, stays live.
    let mut more = old.clone();
    more.limits.cpus = Some(1.5);
    assert!(!removes_a_limit(&old.limits, &more.limits));
    assert_eq!(offer(&old, &more), Offer::ApplyLimits);
}

#[test]
fn edit_recreate_diff_never_shows_an_env_value() {
    let old = api_spec();
    let mut new = old.clone();
    new.env[0].1 = SECRET.to_string();
    new.env.push(("ADDED".to_string(), SECRET.to_string()));
    let rows = diff_rows(&old, &new);
    let env = rows.iter().find(|r| r.field == "env").expect("env row");
    assert!(
        env.before.contains(&format!("{}=••••", old.env[0].0)),
        "{env:?}"
    );
    assert!(env.after.contains("ADDED=••••"), "{env:?}");
    for row in &rows {
        for text in [&row.before, &row.after] {
            assert!(!text.contains(SECRET), "{row:?}");
        }
    }
    // A value-only change reads as one.
    let mut only = old.clone();
    only.env[0].1 = SECRET.to_string();
    let rows = diff_rows(&old, &only);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].after, "values changed (hidden)");
}

// ---------------------------------------------------------------------
// Spec from a detail
// ---------------------------------------------------------------------

#[test]
fn edit_recreate_spec_from_detail_carries_what_runs() {
    let loaded = loaded_of(&api());
    let spec = spec_from_detail(&loaded.detail);
    assert_eq!(spec.name, "shop-api-api-1");
    assert_eq!(spec.image.reference, "shop-api-api:latest");
    assert_eq!(
        spec.command,
        Some(vec!["node".to_string(), "dist/server.js".to_string()])
    );
    assert_eq!(spec.env.len(), loaded.detail.env.len());
    assert_eq!(
        spec.env[0].1, loaded.detail.env[0].value,
        "values stay in the spec"
    );
    assert_eq!(spec.restart, RestartPolicy::UnlessStopped);
    assert_eq!(spec.networks, ["shop-api_default"]);
    assert_eq!(spec.platform.as_deref(), Some("linux/arm64"));
    assert_eq!(spec.ports.len(), 1);
    assert_eq!(spec.ports[0].host_port, Some(3000));
}

#[test]
fn edit_recreate_published_ports_come_from_the_configured_bindings_and_skip_exposed_only() {
    let mut detail = loaded_of(&api()).detail;
    // A stopped container has no live ports; its configuration still has.
    detail.container.ports.clear();
    detail.inspect = serde_json::json!({
        "HostConfig": { "PortBindings": {
            "80/tcp": [{ "HostIp": "127.0.0.1", "HostPort": "56080" }],
            "53/udp": [{ "HostIp": "", "HostPort": "5353" }],
            "9000/tcp": [{ "HostIp": "", "HostPort": "" }],
            "8080/tcp": null,
        } }
    });
    let ports = published_ports(&detail);
    let text: Vec<String> = ports.iter().map(format_port).collect();
    assert!(text.contains(&"127.0.0.1:56080:80".to_string()), "{text:?}");
    assert!(text.contains(&"5353:53/udp".to_string()), "{text:?}");
    assert!(text.contains(&"9000".to_string()), "{text:?}");
    assert!(
        !text.iter().any(|t| t.contains("8080")),
        "exposed only: {text:?}"
    );

    // No configuration in the inspect: only live ports with a host side.
    let mut live = loaded_of(&api()).detail;
    live.container.ports.push(PortMapping {
        host_ip: None,
        host_port: None,
        container_port: 9999,
        proto: Proto::Tcp,
    });
    let ports = published_ports(&live);
    assert_eq!(ports.len(), 1);
    assert_eq!(ports[0].container_port, 3000);
}

#[test]
fn edit_recreate_not_carried_names_what_a_spec_cannot_hold() {
    let mut detail = loaded_of(&api()).detail;
    assert!(not_carried(&detail).is_empty());
    detail.inspect = serde_json::json!({
        "HostConfig": {
            "Privileged": true,
            "CapAdd": ["NET_ADMIN"],
            "Devices": [],
            "ExtraHosts": ["db:10.0.0.2"],
            "Tmpfs": { "/run": "" },
        }
    });
    let lost = not_carried(&detail);
    for expected in [
        "privileged mode",
        "added capabilities",
        "extra hosts",
        "tmpfs mounts",
    ] {
        assert!(lost.contains(&expected.to_string()), "{expected}: {lost:?}");
    }
    assert!(!lost.contains(&"devices".to_string()), "{lost:?}");
    assert!(
        takes_from_image(&detail),
        "the api container has a working dir"
    );
}

// ---------------------------------------------------------------------
// The draft and its text forms
// ---------------------------------------------------------------------

fn rich_spec() -> ContainerSpec {
    ContainerSpec {
        name: "web".to_string(),
        image: ImageRef {
            reference: "nginx:alpine".to_string(),
            id: Some("sha256:abc".to_string()),
        },
        command: Some(vec![
            "sh".to_string(),
            "-c".to_string(),
            "echo \"hi there\" && sleep 1".to_string(),
        ]),
        entrypoint: Some(vec!["/entry.sh".to_string()]),
        env: vec![
            ("A".to_string(), "1".to_string()),
            ("EMPTY".to_string(), String::new()),
        ],
        ports: vec![
            PortMapping {
                host_ip: Some("127.0.0.1".parse().expect("ip")),
                host_port: Some(56080),
                container_port: 80,
                proto: Proto::Tcp,
            },
            PortMapping {
                host_ip: None,
                host_port: Some(5353),
                container_port: 53,
                proto: Proto::Udp,
            },
            PortMapping {
                host_ip: None,
                host_port: None,
                container_port: 9000,
                proto: Proto::Tcp,
            },
        ],
        mounts: vec![
            Mount {
                kind: MountKind::Bind,
                source: "/Users/me/site".to_string(),
                destination: "/usr/share/nginx/html".to_string(),
                read_only: true,
                volume_name: None,
            },
            Mount {
                kind: MountKind::Volume,
                source: "/var/lib/docker/volumes/data/_data".to_string(),
                destination: "/data".to_string(),
                read_only: false,
                volume_name: Some("data".to_string()),
            },
        ],
        networks: vec!["bridge".to_string(), "shop_default".to_string()],
        labels: BTreeMap::from([("a".to_string(), "b".to_string())]),
        restart: RestartPolicy::OnFailure { max_retries: 5 },
        limits: Limits {
            cpus: Some(1.5),
            memory: Some(ByteSize(256 << 20)),
            memory_swap: Some(ByteSize(512 << 20)),
        },
        platform: Some("linux/arm64".to_string()),
    }
}

#[test]
fn edit_recreate_draft_round_trip_changes_nothing() {
    let spec = rich_spec();
    let draft = Draft::from_spec(&spec);
    assert_eq!(draft.to_spec(&spec), Ok(spec.clone()));
    assert_eq!(
        offer(&spec, &draft.to_spec(&spec).expect("spec")),
        Offer::Nothing
    );
}

#[test]
fn edit_recreate_draft_never_holds_an_env_value() {
    let spec = rich_spec();
    let draft = Draft::from_spec(&spec);
    assert!(draft
        .env
        .iter()
        .all(|row| row.value.is_none() && row.existing));
    // Typing a value replaces it; emptying the field keeps the old one.
    let mut typed = draft.clone();
    let id = typed.env[0].id;
    typed.set_env_value(id, SECRET);
    assert_eq!(typed.to_spec(&spec).expect("spec").env[0].1, SECRET);
    typed.set_env_value(id, "");
    assert_eq!(typed.to_spec(&spec).expect("spec").env[0].1, "1");
    assert!(!format!("{typed:?}").contains(SECRET));
}

#[test]
fn edit_recreate_draft_rows_add_and_remove() {
    let spec = rich_spec();
    let mut draft = Draft::from_spec(&spec);
    let first_port = draft.ports[0].0;
    draft.remove_port(first_port);
    let added = draft.add_env();
    draft.set_env_key(added, "NEW_ONE");
    draft.set_env_value(added, "v");
    let mount = draft.add_mount();
    draft.set_mount(mount, "cache:/cache");
    let built = draft.to_spec(&spec).expect("spec");
    assert_eq!(built.ports.len(), 2);
    assert_eq!(
        built.env.last(),
        Some(&("NEW_ONE".to_string(), "v".to_string()))
    );
    assert_eq!(
        built.mounts.last().expect("mount").volume_name.as_deref(),
        Some("cache")
    );
    // An existing variable that is removed is gone.
    let existing = draft.env[0].id;
    draft.remove_env(existing);
    assert!(!draft
        .to_spec(&spec)
        .expect("spec")
        .env
        .iter()
        .any(|(k, _)| k == "A"));
}

#[test]
fn edit_recreate_draft_reports_every_bad_field() {
    let spec = rich_spec();
    let mut draft = Draft::from_spec(&spec);
    draft.image = "has space".to_string();
    draft.command = "echo \"open".to_string();
    draft.cpus = "-1".to_string();
    draft.memory = "1x".to_string();
    draft.networks = "ok, bad name".to_string();
    let id = draft.ports[0].0;
    draft.set_port(id, "99999:80");
    let added = draft.add_env();
    draft.set_env_key(added, "A");
    let errors = draft.to_spec(&spec).expect_err("bad draft");
    let fields: Vec<Field> = errors.iter().map(|e| e.field).collect();
    for field in [
        Field::Image,
        Field::Command,
        Field::Cpus,
        Field::Memory,
        Field::Networks,
        Field::Port(id),
        Field::EnvKey(added),
    ] {
        assert!(fields.contains(&field), "{field:?} in {errors:?}");
    }
}

#[test]
fn edit_recreate_swap_below_memory_is_refused() {
    let spec = rich_spec();
    let mut draft = Draft::from_spec(&spec);
    draft.swap = "128m".to_string();
    let errors = draft.to_spec(&spec).expect_err("swap below memory");
    assert!(errors.iter().any(|e| e.field == Field::Swap), "{errors:?}");
}

#[test]
fn edit_recreate_changing_the_image_drops_its_old_id() {
    let spec = rich_spec();
    let mut draft = Draft::from_spec(&spec);
    draft.image = "nginx:1.27".to_string();
    let built = draft.to_spec(&spec).expect("spec");
    assert_eq!(built.image.reference, "nginx:1.27");
    assert_eq!(built.image.id, None);
}

#[test]
fn edit_recreate_command_text_round_trips_through_quotes() {
    for args in [
        vec!["node".to_string(), "dist/server.js".to_string()],
        vec![
            "sh".to_string(),
            "-c".to_string(),
            "echo \"a b\" && c".to_string(),
        ],
        vec!["a\\b".to_string(), String::new(), "it's".to_string()],
    ] {
        assert_eq!(
            split_command(&join_command(&args)),
            Ok(args.clone()),
            "{args:?}"
        );
    }
    assert_eq!(
        split_command("a  'b c' \"d e\" f\\ g"),
        Ok(vec![
            "a".to_string(),
            "b c".to_string(),
            "d e".to_string(),
            "f g".to_string()
        ])
    );
    assert!(split_command("echo 'open").is_err());
    assert_eq!(split_command("   "), Ok(Vec::new()));
}

#[test]
fn edit_recreate_ports_parse_docker_spelling_and_round_trip() {
    for text in [
        "80",
        "8080:80",
        "127.0.0.1:56080:80",
        "5353:53/udp",
        "127.0.0.1::80",
        "[::1]:8080:80",
    ] {
        let port = parse_port(text).unwrap_or_else(|e| panic!("{text}: {e}"));
        assert_eq!(format_port(&port), text);
    }
    for bad in [
        "",
        "0:80",
        "70000:80",
        "a:b",
        "1:2:3:4",
        "80/xyz",
        "[::1:80",
        "1.2.3.4.5:80:80",
    ] {
        assert!(parse_port(bad).is_err(), "{bad} should not parse");
    }
}

#[test]
fn edit_recreate_mounts_parse_binds_volumes_and_tmpfs() {
    let bind = parse_mount("/Users/me/app:/app:ro").expect("bind");
    assert_eq!((bind.kind, bind.read_only), (MountKind::Bind, true));
    let volume = parse_mount("pgdata:/var/lib/postgresql/data").expect("volume");
    assert_eq!(volume.kind, MountKind::Volume);
    assert_eq!(volume.volume_name.as_deref(), Some("pgdata"));
    let tmpfs = parse_mount("tmpfs:/run").expect("tmpfs");
    assert_eq!(tmpfs.kind, MountKind::Tmpfs);
    for mount in [&bind, &volume, &tmpfs] {
        assert_eq!(
            &parse_mount(&format_mount(mount)).expect("round trip"),
            mount
        );
    }
    for bad in [
        "",
        "/a",
        "./rel:/x",
        "~/x:/x",
        "vol:relative",
        "bad name:/x",
        "a:/b:rwx",
    ] {
        assert!(parse_mount(bad).is_err(), "{bad} should not parse");
    }
}

#[test]
fn edit_recreate_memory_follows_docker_units() {
    // 256 MB in `docker run -m` terms is 256 MiB.
    assert_eq!(parse_memory("256m"), Ok(Some(ByteSize(268_435_456))));
    assert_eq!(parse_memory("256 MB"), Ok(Some(ByteSize(268_435_456))));
    assert_eq!(parse_memory("1.5g"), Ok(Some(ByteSize(1_610_612_736))));
    assert_eq!(
        parse_memory("512k"),
        Err("Docker needs at least 6m".to_string())
    );
    assert_eq!(parse_memory(""), Ok(None));
    assert!(parse_memory("12q").is_err());
    assert!(parse_memory("m").is_err());
    for bytes in [268_435_456_u64, 1 << 30, 7 << 20, 6_291_457] {
        assert_eq!(
            parse_memory(&format_memory(Some(ByteSize(bytes)))),
            Ok(Some(ByteSize(bytes)))
        );
    }
    assert_eq!(format_memory(Some(ByteSize(268_435_456))), "256m");
}

// ---------------------------------------------------------------------
// Duplicate, rename, failure
// ---------------------------------------------------------------------

#[test]
fn edit_recreate_duplicate_names_the_copy_name_copy_and_finds_a_free_name() {
    assert_eq!(duplicate_name("web", &["web", "db"]), "web-copy");
    assert_eq!(duplicate_name("web", &["web", "web-copy"]), "web-copy-2");
    assert_eq!(
        duplicate_name("web", &["web", "web-copy", "web-copy-2"]),
        "web-copy-3"
    );
    let spec = rich_spec();
    assert_eq!(duplicate_spec(&spec, &["web"]).name, "web-copy");
}

#[test]
fn edit_recreate_duplicate_leaves_compose_labels_and_anonymous_volumes_behind() {
    let mut spec = rich_spec();
    spec.labels
        .insert("com.docker.compose.project".to_string(), "shop".to_string());
    spec.labels
        .insert("com.docker.compose.service".to_string(), "web".to_string());
    let anonymous = "a".repeat(64);
    spec.mounts.push(Mount {
        kind: MountKind::Volume,
        source: anonymous.clone(),
        destination: "/anon".to_string(),
        read_only: false,
        volume_name: Some(anonymous),
    });
    let copy = duplicate_spec(&spec, &[]);
    assert_eq!(
        copy.labels,
        BTreeMap::from([("a".to_string(), "b".to_string())])
    );
    assert_eq!(copy.mounts.len(), 2, "the anonymous volume is left behind");
    assert!(copy.mounts.iter().all(|m| m.destination != "/anon"));
    // Everything else is as the original has it.
    assert_eq!(copy.ports, spec.ports);
    assert_eq!(copy.env, spec.env);
    assert_eq!(copy.image, spec.image);
}

#[test]
fn edit_recreate_rename_checks_docker_names_and_the_engine() {
    assert_eq!(rename_problem("web", "web2", &["web", "db"]), None);
    assert!(rename_problem("web", "", &[]).is_some());
    assert!(rename_problem("web", "web", &[]).is_some());
    assert!(rename_problem("web", "db", &["web", "db"]).is_some());
    assert!(rename_problem("web", "bad name", &[]).is_some());
    assert!(rename_problem("web", "-lead", &[]).is_some());
    assert_eq!(rename_problem("web", " api_2.v1-x ", &[]), None);
}

#[test]
fn edit_recreate_failure_text_keeps_the_old_container_unless_restoring_failed_too() {
    let kept = failure_text("No such image: x:1");
    assert!(kept.starts_with("The old container was kept."), "{kept}");
    assert!(kept.contains("No such image: x:1"));
    let lost = failure_text(
        "recreate failed (port is already allocated); restoring the old container also failed: boom",
    );
    assert!(!lost.contains("was kept"), "{lost}");
    assert!(lost.contains("could not be restored"), "{lost}");
}

#[test]
fn edit_recreate_status_prefers_running_over_failed_and_ignores_other_containers() {
    let id = ContainerId("c1".to_string());
    let other = ContainerId("c2".to_string());
    let key = |id: &ContainerId, action: &'static str| PendingKey {
        subject: berth_core::Subject::Container(id.clone()),
        action,
    };
    let mut pending = BTreeMap::new();
    assert_eq!(edit_status(&pending, &id), EditStatus::Idle);
    pending.insert(
        key(&id, "container.recreate"),
        PendingState::Failed {
            message: "boom".to_string(),
        },
    );
    pending.insert(
        key(&other, "container.recreate"),
        PendingState::Running {
            label: "Recreating…".to_string(),
        },
    );
    assert!(matches!(
        edit_status(&pending, &id),
        EditStatus::Failed { .. }
    ));
    pending.insert(
        key(&id, "container.apply_limits"),
        PendingState::Running {
            label: "Applying…".to_string(),
        },
    );
    assert_eq!(
        edit_status(&pending, &id),
        EditStatus::Running("Applying…".to_string())
    );
    // A running restart is not the sheet's business.
    pending.clear();
    pending.insert(
        key(&id, "container.restart"),
        PendingState::Running {
            label: "Restarting…".to_string(),
        },
    );
    assert_eq!(edit_status(&pending, &id), EditStatus::Idle);
}

#[test]
fn edit_recreate_primary_button_says_what_it_does_and_why_not() {
    let idle = EditStatus::Idle;
    assert_eq!(
        primary_button(Offer::ApplyLimits, false, &idle),
        ("Apply limits".to_string(), None)
    );
    assert_eq!(
        primary_button(Offer::Recreate, false, &idle),
        ("Recreate container".to_string(), None)
    );
    assert_eq!(
        primary_button(Offer::Nothing, false, &idle).1.as_deref(),
        Some("Nothing has changed")
    );
    assert!(primary_button(Offer::Recreate, true, &idle).1.is_some());
    assert_eq!(
        primary_button(
            Offer::Recreate,
            false,
            &EditStatus::Running("Recreating…".to_string())
        ),
        ("Recreating…".to_string(), None)
    );
}

#[test]
fn edit_recreate_the_loss_warning_comes_from_the_one_effect_summary() {
    let c = api();
    let effect = confirm::recreate_effect(&c, &[], false, &["devices".to_string()], true);
    assert_eq!(confirm::button_label(&effect), "Recreate container");
    assert_eq!(confirm::result_label(&effect, 1), "Recreated 1 container");
    let texts: Vec<&str> = effect.lines.iter().map(|l| l.text.as_str()).collect();
    assert!(texts.iter().any(|t| t.contains("lost")), "{texts:?}");
    assert!(
        texts
            .iter()
            .any(|t| t.contains("Not carried over: devices")),
        "{texts:?}"
    );
    assert!(
        texts.iter().any(|t| t.contains("Compose service name")),
        "{texts:?}"
    );
    // The latest-image dialog adds the pull.
    let latest = confirm::recreate_effect(&c, &[], true, &[], false);
    assert!(
        latest.lines[0].text.contains("pulled again"),
        "{:?}",
        latest.lines
    );
}

// ---------------------------------------------------------------------
// The Limits tab
// ---------------------------------------------------------------------

#[component]
fn LimitsRoot(sheet: Sheet) -> Element {
    let state = use_app_state();
    use_context_provider(|| state);
    let store = provide_detail_store();
    let seeded = sheet.clone();
    use_hook(move || {
        let mut state = state;
        state
            .containers
            .set(world().containers_for(&seeded.container.engine));
        state.pending.set(seeded.pending.clone());
        store.seed(&seeded.container, loaded_of(&seeded.container));
    });
    rsx! { crate::screens::container_limits::LimitsPanel { container: sheet.container.clone() } }
}

fn render_limits(sheet: Sheet) -> String {
    let mut dom = VirtualDom::new_with_props(LimitsRoot, LimitsRootProps { sheet });
    let _ = dom.rebuild_to_vec();
    strip_styles(&dioxus_ssr::render(&dom))
}

#[test]
fn edit_recreate_limits_tab_shows_the_fields_and_waits_for_a_change() {
    let html = render_limits(Sheet::of(&api()));
    for needle in [
        "Restart policy",
        "CPUs",
        "Memory",
        "Memory plus swap",
        "Apply limits",
    ] {
        assert!(html.contains(needle), "{needle}: {html}");
    }
    assert!(
        html.contains("data-tooltip=\"Nothing has changed\""),
        "{html}"
    );
    assert!(html.contains("Edit and recreate…"), "{html}");
}

#[test]
fn edit_recreate_dev_scene_shows_the_sheet_with_a_diff() {
    let html = crate::scenes::render_scene_html("Dev / edit recreate").expect("scene renders");
    for needle in [
        "aria-label=\"Edit and recreate\"",
        "Edit shop-api-api-1",
        "Recreate container",
        "3001",
        "values changed (hidden)",
        "What a recreate loses",
        "Write to compose.yaml instead",
    ] {
        assert!(html.contains(needle), "{needle}");
    }
    assert!(
        !html.contains("a-new-value"),
        "the typed value is not in the page"
    );
}

#[test]
fn edit_recreate_limits_tab_button_states() {
    use crate::screens::container_limits::apply_button;
    let idle = EditStatus::Idle;
    assert_eq!(
        apply_button(Offer::ApplyLimits, false, false, &idle).1,
        None
    );
    assert!(apply_button(Offer::Recreate, false, true, &idle)
        .1
        .is_some_and(|r| r.contains("Edit and recreate")));
    assert!(apply_button(Offer::Nothing, true, false, &idle).1.is_some());
}
