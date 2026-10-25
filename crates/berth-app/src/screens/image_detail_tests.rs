//! P4-T1 tests for the image panel's view model: the Overview rows, the
//! history lines and what the Tag bar accepts.

use super::*;

use berth_core::{Arch, ImageLayer};

fn detail() -> ImageDetail {
    ImageDetail {
        image: Image {
            id: "sha256:0123456789abcdef0123456789abcdef".to_string(),
            repo_tags: vec!["nginx:alpine".to_string(), "nginx:1.27-alpine".to_string()],
            digests: vec!["nginx@sha256:aaaa".to_string()],
            size: bytesize::ByteSize::mb(74),
            created: chrono::DateTime::from_timestamp(1_790_000_000, 0).unwrap_or_default(),
            arch: Arch::Aarch64,
            os: "linux".to_string(),
            used_by: vec![ContainerId("c1".to_string())],
            dangling: false,
            engine: berth_core::EngineId("colima:default".to_string()),
        },
        layers: vec![
            ImageLayer {
                created_by: "/bin/sh -c #(nop)  CMD [\"nginx\" \"-g\" \"daemon off;\"]".to_string(),
                size: bytesize::ByteSize(0),
                created: chrono::DateTime::from_timestamp(1_790_000_000, 0),
            },
            ImageLayer {
                created_by: "/bin/sh -c apk add --no-cache nginx # buildkit".to_string(),
                size: bytesize::ByteSize::mb(12),
                created: None,
            },
        ],
        env_keys: vec!["PATH".to_string(), "NGINX_VERSION".to_string()],
        exposed_ports: vec![80, 443],
    }
}

fn now() -> chrono::DateTime<chrono::Utc> {
    chrono::DateTime::from_timestamp(1_790_000_000 + 2 * 86_400, 0).unwrap_or_default()
}

fn row<'a>(rows: &'a [KeyValue], label: &str) -> &'a KeyValue {
    rows.iter()
        .find(|r| r.label == label)
        .unwrap_or_else(|| panic!("a `{label}` row"))
}

#[test]
fn image_detail_overview_rows_say_where_the_image_runs_and_what_it_exposes() {
    let rows = overview_rows(&detail(), "default", &[], now());
    assert_eq!(row(&rows, "ID").value, "0123456789ab");
    assert_eq!(row(&rows, "Size").value, "74 MB");
    assert_eq!(row(&rows, "Platform").value, "linux/arm64");
    assert_eq!(row(&rows, "Engine").value, "default");
    assert_eq!(row(&rows, "Digest").value, "nginx@sha256:aaaa");
    assert_eq!(row(&rows, "Exposed ports").value, "80, 443");
    // Keys only: the panel never reads an image's variable values.
    assert_eq!(row(&rows, "Environment").value, "PATH, NGINX_VERSION");
    assert!(
        row(&rows, "Created").value.ends_with("(2 d ago)"),
        "{:?}",
        row(&rows, "Created")
    );
    assert_eq!(row(&rows, "Used by").value, "1 container");
}

#[test]
fn image_detail_overview_rows_say_so_when_there_is_nothing_to_show() {
    let mut bare = detail();
    bare.image.used_by.clear();
    bare.image.digests.clear();
    bare.image.arch = Arch::Other("unknown".to_string());
    bare.env_keys.clear();
    bare.exposed_ports.clear();
    let rows = overview_rows(&bare, "default", &[], now());
    for (label, empty) in [
        ("Used by", "Nothing"),
        ("Digest", "Not set"),
        ("Platform", "\u{2014}"),
        ("Exposed ports", "Not set"),
        ("Environment", "No variables"),
    ] {
        let found = row(&rows, label);
        assert!(found.value.is_empty(), "{label}: {found:?}");
        assert_eq!(found.empty_text, empty, "{label}");
    }
}

#[test]
fn image_detail_layer_commands_read_as_the_dockerfile_line() {
    assert_eq!(
        layer_command("/bin/sh -c #(nop)  CMD [\"nginx\" \"-g\" \"daemon off;\"]"),
        "CMD [\"nginx\" \"-g\" \"daemon off;\"]"
    );
    assert_eq!(
        layer_command("/bin/sh -c apk add curl # buildkit"),
        "RUN apk add curl"
    );
    assert_eq!(layer_command("COPY . /app # buildkit"), "COPY . /app");
    assert_eq!(layer_command("   "), "(no command)");
}

#[test]
fn image_detail_layer_rows_keep_the_order_sizes_and_ages() {
    let rows = layer_rows(&detail(), now());
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].size, "0 B");
    assert_eq!(rows[0].age, "2 d ago");
    assert_eq!(rows[1].size, "12 MB");
    assert_eq!(rows[1].command, "RUN apk add --no-cache nginx");
    assert_eq!(rows[1].age, "");
}

#[test]
fn image_detail_split_tag_takes_the_last_colon_unless_it_is_a_registry_port() {
    assert_eq!(
        split_tag("nginx:alpine"),
        ("nginx".to_string(), "alpine".to_string())
    );
    assert_eq!(
        split_tag("nginx"),
        ("nginx".to_string(), "latest".to_string())
    );
    assert_eq!(
        split_tag("localhost:5000/app"),
        ("localhost:5000/app".to_string(), "latest".to_string())
    );
    assert_eq!(
        split_tag("localhost:5000/app:dev"),
        ("localhost:5000/app".to_string(), "dev".to_string())
    );
    assert_eq!(
        split_tag("a/b@sha256:abcdef0123456789"),
        (
            "a/b@sha256:abcdef0123456789".to_string(),
            "latest".to_string()
        )
    );
}

#[test]
fn image_detail_tag_problem_names_the_field_to_fix() {
    assert_eq!(tag_problem("berthfx/web", "v1"), None);
    assert_eq!(tag_problem("localhost:5000/web", "v1"), None);
    assert!(tag_problem("", "v1").unwrap().contains("repository"));
    assert!(tag_problem("web", "").unwrap().contains("tag"));
    assert!(tag_problem("web:v1", "v2").unwrap().contains("Tag field"));
    assert!(tag_problem("Web", "v1").unwrap().contains("Web"));
    assert!(tag_problem("web", "bad tag")
        .unwrap()
        .contains("not a valid"));
}

#[test]
fn image_detail_platform_is_empty_when_the_engine_did_not_say() {
    let mut image = detail().image;
    assert_eq!(platform_label(&image), "linux/arm64");
    image.arch = Arch::X86_64;
    assert_eq!(platform_label(&image), "linux/amd64");
    image.os.clear();
    assert_eq!(platform_label(&image), "amd64");
    image.arch = Arch::Other("unknown".to_string());
    assert_eq!(platform_label(&image), "");
}

#[test]
fn image_detail_tab_ids_round_trip() {
    for tab in [ImageTab::Overview, ImageTab::Layers, ImageTab::Containers] {
        assert_eq!(ImageTab::from_id(tab.id()), Some(tab));
    }
    assert_eq!(ImageTab::from_id("env"), None);
}
