use super::*;
use serde_json::json;
use std::path::Path;

/// Values that must never survive any redaction below.
const SECRETS: [&str; 6] = [
    "s3cr3t-token-xyz",
    "hunter2hunter2",
    "p4ss/with=equals",
    "AKIAFAKEFAKEFAKE",
    "line-two-of-a-key",
    "quoted secret value",
];

fn assert_clean(output: &str) {
    for secret in SECRETS {
        assert!(
            !output.contains(secret),
            "`{secret}` leaked into:\n{output}"
        );
    }
}

// ----------------------------------------------------------------- is_env_file

#[test]
fn redact_env_files_are_detected_by_name() {
    for yes in [
        ".env",
        ".env.local",
        ".env.production",
        "app/.env",
        "/Users/me/app/.env.staging",
        "prod.env",
        "config/dev.env",
        ".ENV",
        ".Env.Local",
    ] {
        assert!(is_env_file(Path::new(yes)), "{yes} is an env file");
    }
    for no in [
        "",
        "/",
        "env",
        ".envrc",
        ".environment",
        "environment.yaml",
        "compose.yaml",
        "env.txt",
        "src/env/mod.rs",
        ".env/readme.md",
        "dotenv",
        "myenv",
    ] {
        assert!(!is_env_file(Path::new(no)), "{no} is not an env file");
    }
}

// -------------------------------------------------------------------- env_file

#[test]
fn redact_env_file_masks_every_value_and_keeps_comments_and_blanks() {
    let text = "# database\nDB_URL=postgres://u:hunter2hunter2@db/app\n\nPORT=8080\n# KEY=commented\nEMPTY=\n";
    assert_eq!(
        env_file(text),
        "# database\nDB_URL=••••\n\nPORT=••••\n# KEY=commented\nEMPTY=••••\n"
    );
}

#[test]
fn redact_env_file_handles_export_quotes_spaces_and_inline_comments() {
    let text = "export TOKEN=s3cr3t-token-xyz\nA = \"quoted secret value\"\nB='single' # note s3cr3t-token-xyz\n  C=indented\nD=a=b=hunter2hunter2\n";
    let out = env_file(text);
    assert_eq!(out, "TOKEN=••••\nA=••••\nB=••••\nC=••••\nD=••••\n");
    assert_clean(&out);
}

#[test]
fn redact_env_file_swallows_a_quoted_value_that_spans_lines() {
    let text =
        "BEFORE=1\nKEY=\"-----BEGIN-----\nline-two-of-a-key\nAAAA==\n-----END-----\"\nAFTER=2\n";
    let out = env_file(text);
    assert_eq!(out, "BEFORE=••••\nKEY=••••\nAFTER=••••\n");
    assert_clean(&out);
}

#[test]
fn redact_env_file_masks_a_line_that_is_not_an_assignment() {
    // A stray secret on its own line must not pass for a comment or a blank.
    let out = env_file("s3cr3t-token-xyz\nA=1\n");
    assert_eq!(out, "••••\nA=••••\n");
}

#[test]
fn redact_env_file_keeps_crlf_line_endings() {
    assert_eq!(
        env_file("A=hunter2hunter2\r\n# c\r\nB=2\r\n"),
        "A=••••\r\n# c\r\nB=••••\r\n"
    );
}

#[test]
fn redact_env_file_of_nothing_is_nothing() {
    assert_eq!(env_file(""), "");
    assert_eq!(env_file("\n"), "\n");
}

// --------------------------------------------------------------------- inspect

#[test]
fn redact_inspect_masks_config_env_values_and_keeps_names() {
    let v = json!({
        "Id": "abc",
        "Config": {
            "Image": "postgres:17",
            "Cmd": ["postgres"],
            "Env": ["POSTGRES_PW=s3cr3t-token-xyz", "PATH=/usr/bin", "NOVALUE"],
            "Labels": {"com.docker.compose.project": "app"}
        },
        "State": {"Status": "running"}
    });
    let out = inspect(&v);
    assert_eq!(
        out["Config"]["Env"],
        json!(["POSTGRES_PW=••••", "PATH=••••", "NOVALUE"])
    );
    assert_eq!(out["Config"]["Image"], "postgres:17");
    assert_eq!(out["Config"]["Cmd"], json!(["postgres"]));
    assert_eq!(out["Config"]["Labels"], v["Config"]["Labels"]);
    assert_eq!(out["State"], v["State"]);
    assert_clean(&out.to_string());
}

#[test]
fn redact_inspect_reaches_env_inside_a_list_of_containers() {
    let v = json!([
        {"Config": {"Env": ["A=hunter2hunter2"]}},
        {"Config": {"Env": ["B=s3cr3t-token-xyz"]}}
    ]);
    let out = inspect(&v);
    assert_eq!(out[0]["Config"]["Env"], json!(["A=••••"]));
    assert_eq!(out[1]["Config"]["Env"], json!(["B=••••"]));
    assert_clean(&out.to_string());
}

#[test]
fn redact_inspect_masks_key_value_pairs_and_maps_too() {
    let pairs =
        json!({"env": [{"key": "A", "value": "hunter2hunter2"}, {"key": "B", "value": ""}]});
    assert_eq!(
        inspect(&pairs),
        json!({"env": [{"key": "A", "value": "••••"}, {"key": "B", "value": "••••"}]})
    );
    let map = json!({"Env": {"A": "hunter2hunter2", "B": null}});
    assert_eq!(inspect(&map), json!({"Env": {"A": "••••", "B": null}}));
}

#[test]
fn redact_inspect_leaves_a_document_without_env_alone() {
    let v = json!({"Id": "x", "Config": {"Image": "nginx"}});
    assert_eq!(inspect(&v), v);
    assert_eq!(inspect(&json!(null)), json!(null));
}

// -------------------------------------------------------------- compose_config

#[test]
fn redact_compose_config_masks_map_and_list_environment() {
    let v = json!({
        "name": "app",
        "services": {
            "web": {
                "image": "nginx",
                "ports": ["80:80"],
                "environment": {"API_KEY": "s3cr3t-token-xyz", "MODE": "prod", "PASSTHROUGH": null}
            },
            "db": {
                "image": "postgres",
                "environment": ["POSTGRES_PW=hunter2hunter2", "BARE", "URL=p4ss/with=equals"]
            }
        }
    });
    let out = compose_config(&v);
    assert_eq!(
        out["services"]["web"]["environment"],
        json!({"API_KEY": "••••", "MODE": "••••", "PASSTHROUGH": null})
    );
    assert_eq!(
        out["services"]["db"]["environment"],
        json!(["POSTGRES_PW=••••", "BARE", "URL=••••"])
    );
    assert_eq!(out["services"]["web"]["image"], "nginx");
    assert_eq!(out["services"]["web"]["ports"], json!(["80:80"]));
    assert_eq!(out["name"], "app");
    assert_clean(&out.to_string());
}

#[test]
fn redact_compose_config_masks_build_args_and_shared_env_extensions() {
    let v = json!({
        "x-common-env": {"TOKEN": "s3cr3t-token-xyz"},
        "x-logging": {"driver": "json-file"},
        "services": {"api": {"build": {"context": ".", "args": {"NPM_TOKEN": "hunter2hunter2"}}}}
    });
    let out = compose_config(&v);
    assert_eq!(out["x-common-env"], json!({"TOKEN": "••••"}));
    assert_eq!(out["x-logging"], json!({"driver": "json-file"}));
    assert_eq!(
        out["services"]["api"]["build"]["args"],
        json!({"NPM_TOKEN": "••••"})
    );
    assert_eq!(out["services"]["api"]["build"]["context"], ".");
    assert_clean(&out.to_string());
}

#[test]
fn redact_compose_config_masks_environment_nested_deeper_than_services() {
    let v = json!({"services": {"a": {"deploy": {"x": {"environment": ["K=s3cr3t-token-xyz"]}}}}});
    assert_clean(&compose_config(&v).to_string());
}

// ---------------------------------------------------------------- compose_text

#[test]
fn redact_compose_text_masks_a_block_map_and_changes_nothing_else() {
    let yaml = "services:\n  web:\n    image: nginx # the image\n    environment:\n      API_KEY: s3cr3t-token-xyz\n      # keep this comment\n      MODE: \"quoted secret value\"\n      PASSTHROUGH:\n      EMPTY: ''\n    ports:\n      - \"80:80\"\n";
    let expected = "services:\n  web:\n    image: nginx # the image\n    environment:\n      API_KEY: ••••\n      # keep this comment\n      MODE: ••••\n      PASSTHROUGH:\n      EMPTY: ••••\n    ports:\n      - \"80:80\"\n";
    let out = compose_text(yaml);
    assert_eq!(out, expected);
    assert_clean(&out);
}

#[test]
fn redact_compose_text_masks_list_items_indented_or_level_with_the_key() {
    let yaml = "services:\n  db:\n    environment:\n      - POSTGRES_PW=hunter2hunter2\n      - \"TOKEN=s3cr3t-token-xyz\"\n      - 'URL=p4ss/with=equals'\n      - BARE\n    image: postgres\n  cache:\n    environment:\n    - KEY=hunter2hunter2\n    image: redis\n";
    let expected = "services:\n  db:\n    environment:\n      - POSTGRES_PW=••••\n      - \"TOKEN=••••\"\n      - 'URL=••••'\n      - BARE\n    image: postgres\n  cache:\n    environment:\n    - KEY=••••\n    image: redis\n";
    let out = compose_text(yaml);
    assert_eq!(out, expected);
    assert_clean(&out);
}

#[test]
fn redact_compose_text_masks_flow_collections_on_one_line_and_many() {
    let yaml = "services:\n  a:\n    environment: {A: s3cr3t-token-xyz, B: \"hunter2hunter2\", C: }\n    image: x\n  b:\n    environment: [A=s3cr3t-token-xyz, \"B=hunter2hunter2\", C]\n    image: y\n  c:\n    environment: {\n      A: p4ss/with=equals,\n      B: quoted secret value\n    }\n    image: z\n";
    let expected = "services:\n  a:\n    environment: {A: ••••, B: ••••, C:}\n    image: x\n  b:\n    environment: [A=••••, \"B=••••\", C]\n    image: y\n  c:\n    environment: {A: ••••, B: ••••}\n    image: z\n";
    let out = compose_text(yaml);
    assert_eq!(out, expected);
    assert_clean(&out);
}

#[test]
fn redact_compose_text_masks_block_scalars_and_continuation_lines() {
    let yaml = "services:\n  a:\n    environment:\n      KEY: |\n        line-two-of-a-key\n        hunter2hunter2\n\n      NEXT: s3cr3t-token-xyz\n      LONG: \"quoted\n        secret value\"\n    image: x\n";
    let out = compose_text(yaml);
    assert_eq!(
        out,
        "services:\n  a:\n    environment:\n      KEY: ••••\n\n      NEXT: ••••\n      LONG: ••••\n    image: x\n"
    );
    assert_clean(&out);
}

#[test]
fn redact_compose_text_masks_build_args_and_shared_env_anchors() {
    let yaml = "x-common-env: &common-env\n  TOKEN: s3cr3t-token-xyz\nx-logging: &logging\n  driver: json-file\nservices:\n  api:\n    build:\n      context: .\n      args:\n        NPM_TOKEN: hunter2hunter2\n    environment:\n      <<: *common-env\n      EXTRA: p4ss/with=equals\n";
    let expected = "x-common-env: &common-env\n  TOKEN: ••••\nx-logging: &logging\n  driver: json-file\nservices:\n  api:\n    build:\n      context: .\n      args:\n        NPM_TOKEN: ••••\n    environment:\n      <<: *common-env\n      EXTRA: ••••\n";
    let out = compose_text(yaml);
    assert_eq!(out, expected);
    assert_clean(&out);
}

#[test]
fn redact_compose_text_masks_a_merge_key_that_holds_values_but_not_one_that_points() {
    let yaml = "services:\n  a:\n    environment:\n      <<: {K: s3cr3t-token-xyz}\n      <<: *common\n  b:\n    environment:\n      <<:\n        K: hunter2hunter2\n      X: p4ss/with=equals\n";
    let out = compose_text(yaml);
    assert_eq!(
        out,
        "services:\n  a:\n    environment:\n      <<: ••••\n      <<: *common\n  b:\n    environment:\n      <<: ••••\n      X: ••••\n"
    );
    assert_clean(&out);
}

#[test]
fn redact_compose_text_masks_an_anchored_environment_and_leaves_an_alias() {
    let yaml = "services:\n  a:\n    environment: &env\n      KEY: s3cr3t-token-xyz\n  b:\n    environment: *env\n";
    assert_eq!(
        compose_text(yaml),
        "services:\n  a:\n    environment: &env\n      KEY: ••••\n  b:\n    environment: *env\n"
    );
}

#[test]
fn redact_compose_text_does_not_cut_a_value_at_an_equals_after_the_map_colon() {
    let yaml =
        "services:\n  a:\n    environment:\n      - URL: https://h/?token=s3cr3t-token-xyz\n";
    let out = compose_text(yaml);
    assert_eq!(
        out,
        "services:\n  a:\n    environment:\n      - URL: ••••\n"
    );
    assert_clean(&out);
}

#[test]
fn redact_compose_text_keeps_crlf_and_text_without_environment_byte_for_byte() {
    let plain = "services:\r\n  a:\r\n    image: nginx\r\n    command: [\"echo\", \"environment: x\"]\r\n# environment: hunter2hunter2\r\n";
    assert_eq!(compose_text(plain), plain);
    let with_env =
        "services:\r\n  a:\r\n    environment:\r\n      K: hunter2hunter2\r\n    image: x\r\n";
    assert_eq!(
        compose_text(with_env),
        "services:\r\n  a:\r\n    environment:\r\n      K: ••••\r\n    image: x\r\n"
    );
}

#[test]
fn redact_compose_text_ends_the_block_at_the_next_key() {
    let yaml = "services:\n  a:\n    environment:\n      K: hunter2hunter2\n    labels:\n      com.example.note: visible\n";
    assert_eq!(
        compose_text(yaml),
        "services:\n  a:\n    environment:\n      K: ••••\n    labels:\n      com.example.note: visible\n"
    );
}

#[test]
fn redact_compose_text_masks_a_scalar_where_a_map_was_expected() {
    let out = compose_text("services:\n  a:\n    environment: s3cr3t-token-xyz\n    image: x\n");
    assert_eq!(
        out,
        "services:\n  a:\n    environment: ••••\n    image: x\n"
    );
}

#[test]
fn redact_compose_text_hides_an_unterminated_flow_rather_than_showing_it() {
    let out =
        compose_text("services:\n  a:\n    environment: {A: s3cr3t-token-xyz,\n    image: x\n");
    assert_clean(&out);
}

// ----------------------------------------------------------------------- scrub

fn owned(secrets: &[&str]) -> Vec<String> {
    secrets.iter().map(ToString::to_string).collect()
}

#[test]
fn redact_scrub_removes_every_listed_secret_inside_longer_text() {
    let text = "2026-10-26 connecting to postgres://app:hunter2hunter2@db:5432/app with key AKIAFAKEFAKEFAKE (retry)\nauth failed for s3cr3t-token-xyz";
    let out = scrub(
        text,
        &owned(&["hunter2hunter2", "AKIAFAKEFAKEFAKE", "s3cr3t-token-xyz"]),
    );
    assert_eq!(
        out,
        "2026-10-26 connecting to postgres://app:••••@db:5432/app with key •••• (retry)\nauth failed for ••••"
    );
    assert_clean(&out);
}

#[test]
fn redact_scrub_replaces_every_occurrence_and_treats_secrets_as_plain_text() {
    let out = scrub("a.b a.b axb a.b", &owned(&["a.b"]));
    assert_eq!(out, "•••• •••• axb ••••");
    // Regex metacharacters match only themselves.
    let out = scrub(
        "x p4ss/with=equals y abc [a-z]+ .*",
        &owned(&["p4ss/with=equals", "[a-z]+", ".*"]),
    );
    assert_eq!(out, "x •••• y abc •••• ••••");
}

#[test]
fn redact_scrub_prefers_the_longest_secret_where_they_overlap() {
    let out = scrub(
        "pass=hunter2hunter2!",
        &owned(&["hunter2", "hunter2hunter2"]),
    );
    assert_eq!(out, "pass=••••!");
    // Order of the list does not matter.
    let out = scrub(
        "pass=hunter2hunter2!",
        &owned(&["hunter2hunter2", "hunter2"]),
    );
    assert_eq!(out, "pass=••••!");
}

#[test]
fn redact_scrub_ignores_empty_secrets_and_changes_nothing_without_a_match() {
    let text = "plain log line, naïve café ✓";
    assert_eq!(scrub(text, &owned(&["", "absent"])), text);
    assert_eq!(scrub(text, &[]), text);
    assert_eq!(scrub("", &owned(&["x"])), "");
}

#[test]
fn redact_scrub_handles_multibyte_text_around_and_inside_a_secret() {
    assert_eq!(scrub("ü→pässwörd←ü", &owned(&["pässwörd"])), "ü→••••←ü");
}

// ------------------------------------------------------------------ unmask_env

#[test]
fn redact_unmask_env_keeps_the_disk_value_for_a_masked_line() {
    let disk = "# db\nDB_PW=hunter2hunter2\nAPI_KEY=\"s3cr3t-token-xyz\"\nPORT=8080\n";
    let proposed = "# db\nDB_PW=••••\nAPI_KEY=••••\nPORT=9090\n";
    assert_eq!(
        unmask_env(proposed, disk),
        "# db\nDB_PW=hunter2hunter2\nAPI_KEY=\"s3cr3t-token-xyz\"\nPORT=9090\n"
    );
}

#[test]
fn redact_unmask_env_takes_new_keys_as_proposed() {
    let disk = "A=hunter2hunter2\n";
    let proposed = "A=••••\nNEW_PORT=3000\nNEW_SECRET=s3cr3t-token-xyz\n";
    assert_eq!(
        unmask_env(proposed, disk),
        "A=hunter2hunter2\nNEW_PORT=3000\nNEW_SECRET=s3cr3t-token-xyz\n"
    );
}

#[test]
fn redact_unmask_env_drops_a_key_the_proposal_removes() {
    let disk = "A=1\nB=hunter2hunter2\n";
    assert_eq!(unmask_env("A=••••\n", disk), "A=1\n");
}

#[test]
fn redact_unmask_env_never_writes_the_mask_as_a_value() {
    // The model invented a key and masked it: there is nothing to keep.
    let out = unmask_env("GHOST=••••\nA=••••\n", "A=1\n");
    assert_eq!(out, "GHOST=\nA=1\n");
    assert!(!out.contains(MASK));
}

#[test]
fn redact_unmask_env_restores_multiline_export_and_last_duplicate() {
    let disk = "export TOKEN=old\nTOKEN=new-value\nKEY=\"-----BEGIN-----\nline-two-of-a-key\n-----END-----\"\n";
    let proposed = "TOKEN=••••\nexport KEY=••••\n";
    assert_eq!(
        unmask_env(proposed, disk),
        "TOKEN=new-value\nKEY=\"-----BEGIN-----\nline-two-of-a-key\n-----END-----\"\n"
    );
}

#[test]
fn redact_unmask_env_accepts_a_quoted_mask_and_keeps_comments_and_crlf() {
    let disk = "A=hunter2hunter2\r\nB=2\r\n";
    let proposed = "# note\r\nA=\"••••\"\r\nB=3\r\n";
    assert_eq!(
        unmask_env(proposed, disk),
        "# note\r\nA=hunter2hunter2\r\nB=3\r\n"
    );
}

#[test]
fn redact_unmask_env_round_trips_what_env_file_showed_the_harness() {
    let disk = "# c\nA=hunter2hunter2\nB=\"quoted secret value\"\n\nC=p4ss/with=equals\n";
    let shown = env_file(disk);
    assert_clean(&shown);
    // The harness echoes the masked file back unchanged: nothing is lost.
    assert_eq!(unmask_env(&shown, disk), disk);
}
