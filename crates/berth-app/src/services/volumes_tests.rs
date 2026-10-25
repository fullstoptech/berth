//! P4-T4 tests for the volume list, the engine a volume is on, the create
//! and remove work, the helper image check and the pull progress.

use super::*;

fn world() -> berth_engine::MockWorld {
    berth_engine::MockWorld::bundled()
}

fn engine_id(s: &str) -> EngineId {
    EngineId(s.to_string())
}

fn all_volumes(world: &berth_engine::MockWorld) -> Vec<Volume> {
    world
        .volumes_by_engine
        .values()
        .flatten()
        .cloned()
        .collect()
}

fn list(
    world: &berth_engine::MockWorld,
    scope: &EngineScope,
    text: &str,
    filter: UsageFilter,
    sort: Option<(SortColumn, bool)>,
) -> VolumeList {
    let volumes = all_volumes(world);
    let sizes = BTreeMap::new();
    build_list(&ListInput {
        volumes: &volumes,
        containers: &world.containers,
        projects: &world.projects,
        engines: &world.engines,
        scope,
        sizes: &sizes,
        text,
        filter,
        sort,
    })
}

fn names(list: &VolumeList) -> Vec<&str> {
    list.rows.iter().map(|r| r.name.as_str()).collect()
}

fn scope_default() -> EngineScope {
    EngineScope::One(engine_id("colima:default"))
}

#[test]
fn volumes_list_shows_the_selected_engines_twelve_volumes() {
    let world = world();
    let all = list(&world, &scope_default(), "", UsageFilter::All, None);
    assert_eq!(all.total, 12, "the frames' badge: 12 volumes on default");
    assert_eq!(all.rows.len(), 12);
    // Another engine's volume is not in this engine's list.
    assert!(!names(&all).contains(&"crm_pgdata"));
    assert!(!names(&all).contains(&"blog_db-data"));
}

#[test]
fn volumes_list_all_engines_adds_the_others() {
    let world = world();
    let all = list(&world, &EngineScope::All, "", UsageFilter::All, None);
    assert_eq!(all.total, 15);
    assert!(names(&all).contains(&"crm_pgdata"));
}

#[test]
fn volumes_unused_filter_lists_only_volumes_no_container_uses() {
    let world = world();
    let unused = list(&world, &scope_default(), "", UsageFilter::Unused, None);
    assert_eq!(names(&unused), ["old-uploads", "stale-cache", "tmp-export"]);
    // The header counts do not move with the usage filter.
    assert_eq!(unused.total, 12);
    assert_eq!(unused.unused, 3);
}

#[test]
fn volumes_filter_text_matches_name_project_and_user_and_counts() {
    let world = world();
    let by_name = list(&world, &scope_default(), "grafana", UsageFilter::All, None);
    assert_eq!(names(&by_name), ["analytics_grafana-data"]);
    assert_eq!(by_name.matched, 1);
    assert_eq!(by_name.total, 12);
    // The project name and the name of a container that uses it count too.
    let by_project = list(
        &world,
        &scope_default(),
        "analytics",
        UsageFilter::All,
        None,
    );
    assert!(by_project.rows.len() >= 3, "{:?}", names(&by_project));
    assert!(by_project
        .rows
        .iter()
        .all(|r| r.project.as_deref() == Some("analytics") || r.name.contains("analytics")));
    let nothing = list(
        &world,
        &scope_default(),
        "no such volume",
        UsageFilter::All,
        None,
    );
    assert!(nothing.rows.is_empty());
    assert_eq!(nothing.matched, 0);
}

#[test]
fn volumes_filter_text_and_usage_filter_combine() {
    let world = world();
    let both = list(&world, &scope_default(), "stale", UsageFilter::Unused, None);
    assert_eq!(names(&both), ["stale-cache"]);
    // "matched" ignores the usage filter: it is the "N of M" number.
    assert_eq!(both.matched, 1);
}

#[test]
fn volumes_sort_by_size_runs_both_ways_with_unknown_sizes_last() {
    let mut world = world();
    // One volume whose size is not known yet.
    for rows in world.volumes_by_engine.values_mut() {
        for v in rows.iter_mut().filter(|v| v.name == "tmp-export") {
            v.size = None;
        }
    }
    let desc = list(
        &world,
        &scope_default(),
        "",
        UsageFilter::All,
        Some((SortColumn::Size, false)),
    );
    assert_eq!(desc.rows[0].name, "analytics_clickhouse-data");
    let asc = list(
        &world,
        &scope_default(),
        "",
        UsageFilter::All,
        Some((SortColumn::Size, true)),
    );
    assert_eq!(asc.rows[0].name, "berth-site_caddy-data");
    assert_eq!(asc.rows.last().map(|r| r.name.as_str()), Some("tmp-export"));
    assert_eq!(
        desc.rows.last().map(|r| r.name.as_str()),
        Some("tmp-export")
    );
}

#[test]
fn volumes_default_order_is_by_name() {
    let world = world();
    let rows = list(&world, &scope_default(), "", UsageFilter::All, None);
    let mut sorted: Vec<String> = rows.rows.iter().map(|r| r.name.to_lowercase()).collect();
    let before = sorted.clone();
    sorted.sort();
    assert_eq!(before, sorted);
}

#[test]
fn volumes_size_reads_calculating_until_the_fetch_answers() {
    let world = world();
    let mut volumes = all_volumes(&world);
    for v in &mut volumes {
        v.size = None;
    }
    let mut cache = BTreeMap::new();
    let make = |cache: &BTreeMap<String, ByteSize>| {
        build_list(&ListInput {
            volumes: &volumes,
            containers: &world.containers,
            projects: &world.projects,
            engines: &world.engines,
            scope: &scope_default(),
            sizes: cache,
            text: "pgdata",
            filter: UsageFilter::All,
            sort: None,
        })
    };
    let before = make(&cache);
    assert_eq!(before.rows[0].size_text(), "Calculating\u{2026}");
    assert!(needs_sizes(&volumes, &cache));
    cache.insert(before.rows[0].name.clone(), ByteSize::mb(1200));
    let after = make(&cache);
    assert_eq!(after.rows[0].size_text(), "1.2 GB");
    assert!(needs_sizes(&volumes, &cache), "other volumes still wait");
}

#[test]
fn volumes_used_by_cell_names_the_container_and_counts_the_rest() {
    let row = VolumeRow {
        name: "v".to_string(),
        owner: None,
        project: None,
        size: None,
        used_by: vec!["a-1".to_string(), "b-1".to_string(), "c-1".to_string()],
        driver: "local".to_string(),
        created: None,
    };
    assert_eq!(row.used_by_text(), "a-1 +2");
    let one = VolumeRow {
        used_by: vec!["a-1".to_string()],
        ..row.clone()
    };
    assert_eq!(one.used_by_text(), "a-1");
    let none = VolumeRow {
        used_by: Vec::new(),
        ..row
    };
    assert_eq!(none.used_by_text(), "\u{2014}");
    assert!(none.unused());
}

// --- which engine ---

#[test]
fn volumes_owner_is_the_engine_of_the_container_that_uses_it() {
    let world = world();
    let all = all_volumes(&world);
    let crm = all.iter().find(|v| v.name == "crm_pgdata").expect("crm");
    assert_eq!(
        owner_engine(crm, &world.containers, &world.engines),
        Some(engine_id("colima:work"))
    );
}

#[test]
fn volumes_owner_of_an_unused_volume_is_the_only_running_engine() {
    let world = world();
    let all = all_volumes(&world);
    let stale = all.iter().find(|v| v.name == "stale-cache").expect("stale");
    assert_eq!(
        owner_engine(stale, &world.containers, &world.engines),
        Some(engine_id("colima:default"))
    );
}

#[test]
fn volumes_owner_is_unknown_when_two_engines_run_and_nothing_uses_it() {
    let mut world = world();
    for engine in &mut world.engines {
        if engine.id == engine_id("colima:work") {
            engine.state = berth_core::EngineState::Running;
        }
    }
    let all = all_volumes(&world);
    let stale = all.iter().find(|v| v.name == "stale-cache").expect("stale");
    assert_eq!(owner_engine(stale, &world.containers, &world.engines), None);
    // A used volume is still placed by its user.
    let pg = all
        .iter()
        .find(|v| v.name == "shop-api_pgdata")
        .expect("pgdata");
    assert_eq!(
        owner_engine(pg, &world.containers, &world.engines),
        Some(engine_id("colima:default"))
    );
}

#[test]
fn volumes_unknown_owner_volume_stays_listed_in_every_scope() {
    let mut world = world();
    for engine in &mut world.engines {
        if engine.id == engine_id("colima:work") {
            engine.state = berth_core::EngineState::Running;
        }
    }
    let rows = list(&world, &scope_default(), "stale", UsageFilter::All, None);
    assert_eq!(names(&rows), ["stale-cache"]);
    assert_eq!(rows.rows[0].owner, None);
}

// --- names and options ---

#[test]
fn volumes_names_follow_dockers_rule() {
    for ok in ["berthfx-vol", "a1", "my_volume.2", "A-b"] {
        assert!(valid_name(ok), "{ok}");
    }
    for bad in ["", "a", "-lead", "has space", "slash/name", "café", "_x"] {
        assert!(!valid_name(bad), "{bad}");
    }
}

#[test]
fn volumes_name_problem_says_taken_before_it_says_invalid() {
    assert_eq!(name_problem("", false), None);
    assert_eq!(
        name_problem("stale-cache", true).as_deref(),
        Some("A volume named stale-cache already exists")
    );
    assert!(name_problem("bad name", false)
        .expect("invalid")
        .starts_with("Use letters"));
    assert_eq!(name_problem("fine-name", false), None);
}

#[test]
fn volumes_options_parse_key_value_words() {
    let parsed =
        parse_options("type=tmpfs  device=tmpfs\n\no=size=100m,uid=1000\n").expect("parses");
    assert_eq!(parsed["type"], "tmpfs");
    assert_eq!(parsed["device"], "tmpfs");
    assert_eq!(parsed["o"], "size=100m,uid=1000");
    assert!(parse_options("").expect("empty").is_empty());
    assert_eq!(
        parse_options("type=tmpfs\nnonsense").expect_err("bad line"),
        "\"nonsense\" is not key=value"
    );
    assert!(parse_options("=x").is_err());
    assert!(
        parse_options("device = tmpfs").is_err(),
        "spaces split a pair"
    );
}

#[test]
fn volumes_create_spec_carries_name_driver_and_options() {
    let spec = create_spec(" berthfx-vol ", "local", "type=tmpfs").expect("spec");
    assert_eq!(spec.name, "berthfx-vol");
    assert_eq!(spec.driver, "local");
    assert_eq!(spec.driver_opts["type"], "tmpfs");
    assert!(create_spec("", "local", "").is_err());
    assert!(create_spec("bad name", "local", "").is_err());
    assert!(create_spec("ok-name", "local", "junk").is_err());
}

// --- the work ---

fn mock_client() -> berth_engine::MockEngine {
    berth_engine::MockEngine::bundled(&engine_id("colima:default"))
}

#[tokio::test]
async fn volumes_run_create_makes_the_volume_and_says_so() {
    let client = mock_client();
    let spec = create_spec("berthfx-vol", "local", "").expect("spec");
    let done = run(&client, VolumeOp::Create(spec), &CancellationToken::new())
        .await
        .expect("created");
    assert_eq!(done.message.as_deref(), Some("Created 1 volume"));
    let names: Vec<String> = client
        .volumes()
        .await
        .expect("list")
        .into_iter()
        .map(|v| v.name)
        .collect();
    assert!(names.contains(&"berthfx-vol".to_string()));
}

#[tokio::test]
async fn volumes_run_create_twice_fails_with_the_engines_words() {
    let client = mock_client();
    let spec = create_spec("stale-cache", "local", "").expect("spec");
    let err = run(&client, VolumeOp::Create(spec), &CancellationToken::new())
        .await
        .expect_err("taken");
    assert!(err.contains("already exists"), "{err}");
}

#[test]
fn volumes_op_ids_match_the_action_defs() {
    assert_eq!(
        VolumeOp::Create(create_spec("ab", "local", "").expect("spec")).action_id(),
        "volume.create"
    );
    assert!(crate::actions::action_by_id(CREATE).is_some());
    assert!(crate::actions::action_by_id("volume.delete").is_some());
}

#[tokio::test]
async fn volumes_fetch_sizes_asks_each_client() {
    let client: Arc<dyn EngineClient> = Arc::new(mock_client());
    let sizes = fetch_sizes(vec![client]).await;
    assert_eq!(
        sizes.get("analytics_clickhouse-data"),
        Some(&ByteSize::mb(3400))
    );
    assert_eq!(sizes.len(), 12);
}

// --- the helper image ---

#[tokio::test]
async fn volumes_helper_is_missing_until_it_is_pulled() {
    let client = mock_client();
    assert!(!helper_present(&client).await.expect("checks"));
    let mut shown = Vec::new();
    pull_helper(&client, CancellationToken::new(), |p| shown.push(p))
        .await
        .expect("pulls");
    assert!(helper_present(&client).await.expect("checks"));
    assert_eq!(shown.last(), Some(&100), "{shown:?}");
}

#[test]
fn volumes_pull_percent_sums_bytes_over_all_layers() {
    let mut layers = BTreeMap::new();
    assert_eq!(percent(&layers), 0);
    layers.insert("a".to_string(), (50_u64, 100_u64));
    layers.insert("b".to_string(), (0, 300));
    assert_eq!(percent(&layers), 12);
    layers.insert("b".to_string(), (300, 300));
    assert_eq!(percent(&layers), 87);
    // A layer that reports more than its total is clamped.
    layers.insert("a".to_string(), (500, 100));
    assert_eq!(percent(&layers), 100);
}
