//! One positive and one negative test per rule, against `fixtures/problems/<kind>/`, plus the
//! cross-rule checks (engine filter, duplicates, masking, past tense).

use std::path::Path;

use berth_core::{
    Arch, ColimaOp, ComposeCommand, ContainerAction, ContainerState, EffectSummary, EngineAction,
    EngineId, EngineKind, EngineState, Fix, FixEffect, GenerationPlan, Health, Mount, MountKind,
    PortMapping, Problem, ProblemKind, Proto, Severity, Subject, VmType,
};

use super::evaluate;
use super::testkit::{
    container, engine, facts, fixture, input, lines, project_facts, project_id, with_inspect,
};
use super::{ContainerFacts, ProblemInput, ProjectFacts};
use crate::diff;

// ---------------------------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------------------------

fn plan_of(fix: &Fix) -> &GenerationPlan {
    match &fix.effect {
        FixEffect::EditFiles(plan) => plan,
        other => panic!("expected a file fix, got {other:?}"),
    }
}

fn action_of(fix: &Fix) -> (&EngineAction, &EffectSummary) {
    match &fix.effect {
        FixEffect::RunAction { action, effect } => (action, effect),
        other => panic!("expected an action fix, got {other:?}"),
    }
}

fn new_text(fix: &Fix) -> String {
    plan_of(fix).files[0]
        .new_text
        .clone()
        .expect("the plan writes a text")
}

fn only(problems: Vec<Problem>) -> Problem {
    assert_eq!(problems.len(), 1, "expected one problem: {problems:#?}");
    problems.into_iter().next().expect("one problem")
}

fn broken_facts(service: &str, state: ContainerState) -> ContainerFacts {
    facts(container(
        &format!("berthfx-broken-{service}-1"),
        Some(service),
        "berthfx-broken",
        state,
    ))
}

fn broken_project(kind: &str, failure: &[String]) -> ProjectFacts {
    project_facts(
        "berthfx-broken",
        Some(&fixture(kind, "compose.yaml")),
        failure,
    )
}

// ---------------------------------------------------------------------------------------------
// PortAllocated
// ---------------------------------------------------------------------------------------------

fn portclash() -> ContainerFacts {
    with_inspect(
        broken_facts("portclash", ContainerState::Created),
        &fixture("port_allocated", "inspect.json"),
    )
}

#[test]
fn problems_port_allocated_container_error_maps_the_service_to_a_free_port() {
    let mut i = input();
    let cf = portclash();
    let id = cf.container.id.clone();
    i.containers.push(cf);
    i.projects.push(broken_project("port_allocated", &[]));
    i.taken_ports = vec![56080];

    let p = only(evaluate(&i));

    assert_eq!(p.kind, ProblemKind::PortAllocated { port: 56080 });
    assert_eq!(p.subject, Subject::Container(id));
    assert_eq!(p.severity, Severity::Error);
    assert_eq!(p.first_seen, i.now);
    // 56081 belongs to `web` in the same file, so the next free port is 56082.
    assert_eq!(p.fixes[0].label, "Map portclash to port 56082");
    let plan = plan_of(&p.fixes[0]);
    let text = new_text(&p.fixes[0]);
    assert!(text.contains("\"127.0.0.1:56082:80\""), "{text}");
    assert!(
        text.contains("\"127.0.0.1:56081:80\""),
        "web keeps its port: {text}"
    );
    assert!(
        text.contains("# The host port the clash is about."),
        "comments stay: {text}"
    );
    assert_eq!(plan.files[0].path, Path::new("compose.yaml"));
    assert_eq!(
        plan.files[0].old_sha256,
        Some(diff::sha256(&fixture("port_allocated", "compose.yaml")))
    );
    assert_eq!(
        plan.after_write,
        Some(ComposeCommand::Up {
            services: vec!["portclash".to_string()],
            build: false
        })
    );
    assert!(p.evidence[0].contains("port is already allocated"));
}

#[test]
fn problems_port_allocated_compose_output_alone_finds_the_service_by_its_port() {
    let mut i = input();
    let failure = lines("port_allocated", "compose_failure.txt");
    i.projects.push(broken_project("port_allocated", &failure));

    let p = only(evaluate(&i));

    assert_eq!(p.kind, ProblemKind::PortAllocated { port: 56080 });
    assert_eq!(p.subject, Subject::Project(project_id("berthfx-broken")));
    assert_eq!(p.fixes[0].label, "Map portclash to port 56082");
}

#[test]
fn problems_port_allocated_the_container_and_its_project_raise_it_once() {
    let mut i = input();
    i.containers.push(portclash());
    i.projects.push(broken_project(
        "port_allocated",
        &lines("port_allocated", "compose_failure.txt"),
    ));

    let p = only(evaluate(&i));

    assert!(matches!(p.subject, Subject::Container(_)));
}

#[test]
fn problems_port_allocated_loose_container_is_recreated_on_another_port() {
    let mut i = input();
    i.taken_ports = vec![56080];
    i.containers.push(with_inspect(
        facts(container("clash-loose", None, "", ContainerState::Created)),
        &fixture("port_allocated", "inspect.json"),
    ));

    let p = only(evaluate(&i));

    let (action, effect) = action_of(&p.fixes[0]);
    let EngineAction::Recreate { spec, pull, .. } = action else {
        panic!("expected a recreate, got {action:?}");
    };
    assert!(!pull);
    assert_eq!(spec.ports.len(), 1);
    assert_eq!(spec.ports[0].host_port, Some(56081));
    assert_eq!(spec.ports[0].container_port, 80);
    assert_eq!(effect.button_label(), "Recreate 1 container");
    assert_eq!(effect.result_label(), "Recreated 1 container");
}

#[test]
fn problems_port_allocated_names_the_container_that_holds_the_port() {
    let mut i = input();
    let mut holder = container("server-db-1", Some("db"), "server", ContainerState::Running);
    holder.ports.push(PortMapping {
        host_ip: None,
        host_port: Some(5432),
        container_port: 5432,
        proto: Proto::Tcp,
    });
    i.containers.push(facts(holder));
    let mut asking = facts(container(
        "shop-db-1",
        Some("db"),
        "shop",
        ContainerState::Created,
    ));
    asking.error_lines = lines("port_allocated", "listen_in_use.txt");
    i.containers.push(asking);

    let p = only(evaluate(&i));

    assert_eq!(p.kind, ProblemKind::PortAllocated { port: 5432 });
    assert!(p.explanation.contains("server-db-1"), "{}", p.explanation);
    // No compose file and no detail: nothing to rewrite, so the fix explains.
    assert_eq!(p.fixes[0].effect, FixEffect::Explain);
}

#[test]
fn problems_port_allocated_ignores_a_normal_start() {
    let mut i = input();
    let mut cf = broken_facts("portclash", ContainerState::Running);
    cf.last_lines = lines("port_allocated", "healthy_start.txt");
    i.containers.push(cf);
    i.projects.push(broken_project("port_allocated", &[]));

    assert!(evaluate(&i).is_empty());
}

#[test]
fn problems_a_container_of_another_engine_is_not_evaluated() {
    let mut i = input();
    let mut cf = portclash();
    cf.container.engine = EngineId("colima:work".to_string());
    i.containers.push(cf);

    assert!(evaluate(&i).is_empty());
}

// ---------------------------------------------------------------------------------------------
// ImageNotFound
// ---------------------------------------------------------------------------------------------

#[test]
fn problems_image_not_found_pull_denied_names_the_service_and_the_full_reference() {
    let mut i = input();
    i.projects.push(broken_project(
        "image_not_found",
        &lines("image_not_found", "compose_failure.txt"),
    ));

    let p = only(evaluate(&i));

    assert_eq!(
        p.kind,
        ProblemKind::ImageNotFound {
            reference: "berthfx/does-not-exist:1".to_string()
        }
    );
    assert_eq!(p.subject, Subject::Project(project_id("berthfx-broken")));
    assert!(p.title.contains("badimage"), "{}", p.title);
    assert!(p.explanation.contains("login"), "{}", p.explanation);
}

#[test]
fn problems_image_not_found_unknown_tag_says_so() {
    let mut i = input();
    let mut cf = facts(container(
        "shop-db-1",
        Some("db"),
        "shop",
        ContainerState::Created,
    ));
    cf.container.image.reference = "postgres:17.99-alpine".to_string();
    cf.error_lines = lines("image_not_found", "manifest_unknown.txt");
    i.containers.push(cf);

    let p = only(evaluate(&i));

    assert_eq!(
        p.kind,
        ProblemKind::ImageNotFound {
            reference: "postgres:17.99-alpine".to_string()
        }
    );
    assert!(matches!(p.subject, Subject::Container(_)));
    assert!(p.explanation.contains("tag"), "{}", p.explanation);
}

#[test]
fn problems_image_not_found_ignores_a_good_pull() {
    let mut i = input();
    i.projects.push(broken_project(
        "image_not_found",
        &lines("image_not_found", "pull_ok.txt"),
    ));

    assert!(evaluate(&i).is_empty());
}

// ---------------------------------------------------------------------------------------------
// ExecFormat
// ---------------------------------------------------------------------------------------------

fn legacy(arch: Option<Arch>, text: &[String]) -> ProblemInput {
    let mut i = input();
    let mut cf = with_inspect(
        facts(container(
            "shop-api-legacy-1",
            Some("legacy"),
            "shop-api",
            ContainerState::Exited,
        )),
        &fixture("exec_format", "inspect.json"),
    );
    cf.container.platform = arch;
    cf.last_lines = text.to_vec();
    i.containers.push(cf);
    i.projects.push(project_facts(
        "shop-api",
        Some(&fixture("exec_format", "compose.yaml")),
        &[],
    ));
    i
}

#[test]
fn problems_exec_format_on_a_vz_machine_offers_rosetta() {
    let i = legacy(Some(Arch::X86_64), &lines("exec_format", "logs.txt"));

    let p = only(evaluate(&i));

    assert_eq!(
        p.kind,
        ProblemKind::ExecFormat {
            image_arch: Some(Arch::X86_64)
        }
    );
    assert_eq!(p.fixes.len(), 1);
    assert_eq!(p.fixes[0].label, "Restart Colima with Rosetta");
    let (action, effect) = action_of(&p.fixes[0]);
    let EngineAction::Colima { profile, op } = action else {
        panic!("expected a Colima action, got {action:?}");
    };
    assert_eq!(profile, "default");
    let ColimaOp::Apply(options) = op else {
        panic!("expected Apply, got {op:?}");
    };
    assert_eq!(options.rosetta, Some(true));
    assert_eq!(options.vm_type, Some(VmType::Vz));
    assert_eq!(effect.button_label(), "Restart 1 engine");
    assert_eq!(effect.result_label(), "Restarted 1 engine");
}

#[test]
fn problems_exec_format_without_rosetta_support_sets_the_service_platform() {
    for tweak in ["qemu", "rosetta-on", "not-colima"] {
        let mut i = legacy(Some(Arch::X86_64), &lines("exec_format", "logs.txt"));
        match tweak {
            "qemu" => {
                i.engine.resources.as_mut().expect("resources").vm_type = VmType::Qemu;
            }
            "rosetta-on" => {
                i.engine.resources.as_mut().expect("resources").rosetta = true;
            }
            _ => {
                i.engine.kind = EngineKind::Socket {
                    path: "/var/run/docker.sock".into(),
                    context: None,
                };
            }
        }

        let p = only(evaluate(&i));

        assert_eq!(p.fixes[0].label, "Run legacy as linux/arm64", "{tweak}");
        let text = new_text(&p.fixes[0]);
        assert!(text.contains("platform: linux/arm64"), "{tweak}: {text}");
        assert!(text.contains("image: postgres:17-alpine"), "{tweak}");
    }
}

#[test]
fn problems_exec_format_reads_the_image_arch_from_the_platform_warning() {
    let i = legacy(None, &lines("exec_format", "platform_warning.txt"));

    let p = only(evaluate(&i));

    assert_eq!(
        p.kind,
        ProblemKind::ExecFormat {
            image_arch: Some(Arch::X86_64)
        }
    );
}

#[test]
fn problems_exec_format_on_the_hosts_own_arch_is_not_an_arch_problem() {
    let i = legacy(Some(Arch::Aarch64), &lines("exec_format", "same_arch.txt"));

    assert!(evaluate(&i).is_empty());
}

#[test]
fn problems_exec_format_needs_the_error_text() {
    let i = legacy(Some(Arch::X86_64), &[]);

    assert!(evaluate(&i).is_empty());
}

// ---------------------------------------------------------------------------------------------
// OutOfMemory
// ---------------------------------------------------------------------------------------------

const MIB: u64 = 1024 * 1024;

fn oom_input(inspect: &str, compose: &str, service: Option<&str>) -> ProblemInput {
    let mut i = input();
    let project = if service.is_some() {
        "berthfx-broken"
    } else {
        ""
    };
    i.containers.push(with_inspect(
        facts(container(
            "berthfx-broken-oom-1",
            service,
            project,
            ContainerState::Exited,
        )),
        inspect,
    ));
    if service.is_some() {
        i.projects
            .push(project_facts("berthfx-broken", Some(compose), &[]));
    }
    i
}

#[test]
fn problems_oom_raises_mem_limit_in_the_compose_file() {
    let i = oom_input(
        &fixture("oom", "inspect.json"),
        &fixture("oom", "compose.yaml"),
        Some("oom"),
    );

    let p = only(evaluate(&i));

    assert_eq!(p.kind, ProblemKind::OutOfMemory);
    assert_eq!(
        p.fixes[0].label,
        "Raise the memory limit of berthfx-broken-oom-1 to 64 MiB"
    );
    let text = new_text(&p.fixes[0]);
    assert!(text.contains("mem_limit: 64m"), "{text}");
    assert!(!text.contains("16m"), "{text}");
    assert!(
        text.contains("image: nginx:alpine"),
        "web is untouched: {text}"
    );
    assert!(
        p.evidence.iter().any(|e| e.contains("16 MiB")),
        "{:?}",
        p.evidence
    );
}

#[test]
fn problems_oom_doubles_a_limit_that_is_already_large() {
    let inspect = fixture("oom", "inspect.json").replace("16777216", "314572800");
    let i = oom_input(&inspect, &fixture("oom", "compose.yaml"), Some("oom"));

    let p = only(evaluate(&i));

    assert!(
        p.fixes[0].label.ends_with("to 600 MiB"),
        "{}",
        p.fixes[0].label
    );
    assert!(new_text(&p.fixes[0]).contains("mem_limit: 600m"));
}

#[test]
fn problems_oom_recreates_a_loose_container_with_more_memory() {
    let i = oom_input(&fixture("oom", "inspect.json"), "", None);

    let p = only(evaluate(&i));

    let (action, effect) = action_of(&p.fixes[0]);
    let EngineAction::Recreate { spec, .. } = action else {
        panic!("expected a recreate, got {action:?}");
    };
    assert_eq!(
        spec.limits.memory.map(|m| m.as_u64()),
        Some(64 * MIB),
        "{spec:?}"
    );
    assert_eq!(effect.result_label(), "Recreated 1 container");
}

#[test]
fn problems_oom_with_a_deploy_limit_explains_instead_of_adding_a_second_limit() {
    let i = oom_input(
        &fixture("oom", "inspect.json"),
        &fixture("oom", "compose_deploy_limit.yaml"),
        Some("oom"),
    );

    let p = only(evaluate(&i));

    assert_eq!(p.fixes[0].effect, FixEffect::Explain);
}

#[test]
fn problems_oom_without_any_limit_blames_the_engine() {
    let i = oom_input(&fixture("oom", "inspect_no_limit.json"), "", None);

    let p = only(evaluate(&i));

    assert_eq!(p.fixes[0].effect, FixEffect::Explain);
    assert!(p.explanation.contains("engine"), "{}", p.explanation);
}

#[test]
fn problems_oom_ignores_a_container_stopped_by_hand() {
    // Exit code 137 without the OOMKilled flag is `docker stop` running out of patience.
    let i = oom_input(
        &fixture("oom", "inspect_stopped_by_hand.json"),
        &fixture("oom", "compose.yaml"),
        Some("oom"),
    );

    assert!(evaluate(&i).is_empty());
}

// ---------------------------------------------------------------------------------------------
// ExitedNoLogs
// ---------------------------------------------------------------------------------------------

fn crashed(code: Option<i32>, state: ContainerState) -> ContainerFacts {
    let mut cf = with_inspect(
        broken_facts("crash", state),
        &fixture("exited_no_logs", "inspect.json"),
    );
    cf.container.exit_code = code;
    cf
}

#[test]
fn problems_exited_no_logs_reports_a_silent_failure() {
    let mut i = input();
    i.containers.push(crashed(Some(1), ContainerState::Exited));

    let p = only(evaluate(&i));

    assert_eq!(p.kind, ProblemKind::ExitedNoLogs { code: 1 });
    assert!(p.title.contains("code 1"), "{}", p.title);
    assert!(p.evidence.contains(&"The log is empty".to_string()));
}

#[test]
fn problems_exited_no_logs_stays_quiet_when_the_log_explains() {
    let mut i = input();
    let mut cf = crashed(Some(1), ContainerState::Exited);
    cf.last_lines = lines("exited_no_logs", "log_with_output.txt");
    i.containers.push(cf);

    assert!(evaluate(&i).is_empty());
}

#[test]
fn problems_exited_no_logs_ignores_clean_exits_stops_and_running_containers() {
    for (code, state) in [
        (Some(0), ContainerState::Exited),
        (Some(137), ContainerState::Exited),
        (Some(143), ContainerState::Exited),
        (Some(130), ContainerState::Exited),
        (None, ContainerState::Exited),
        (Some(1), ContainerState::Running),
    ] {
        let mut i = input();
        i.containers.push(crashed(code, state));
        assert!(evaluate(&i).is_empty(), "{code:?} {state:?}");
    }
}

#[test]
fn problems_exited_no_logs_leaves_a_memory_kill_to_the_oom_rule() {
    let mut i = input();
    let mut cf = with_inspect(
        broken_facts("oom", ContainerState::Exited),
        &fixture("oom", "inspect.json"),
    );
    cf.container.exit_code = Some(1);
    i.containers.push(cf);

    let found = evaluate(&i);

    assert!(found
        .iter()
        .all(|p| !matches!(p.kind, ProblemKind::ExitedNoLogs { .. })));
    assert_eq!(found.len(), 1);
}

// ---------------------------------------------------------------------------------------------
// Unhealthy
// ---------------------------------------------------------------------------------------------

fn unhealthy_facts(health: Option<Health>, state: ContainerState) -> ContainerFacts {
    let mut cf = with_inspect(
        broken_facts("unhealthy", state),
        &fixture("unhealthy", "inspect.json"),
    );
    cf.container.health = health;
    cf
}

#[test]
fn problems_unhealthy_shows_the_check_and_offers_a_restart() {
    let mut i = input();
    let cf = unhealthy_facts(Some(Health::Unhealthy), ContainerState::Running);
    let id = cf.container.id.clone();
    i.containers.push(cf);

    let p = only(evaluate(&i));

    assert_eq!(p.kind, ProblemKind::Unhealthy);
    assert_eq!(p.severity, Severity::Warning);
    assert!(
        p.evidence.contains(&"Check: false".to_string()),
        "{:?}",
        p.evidence
    );
    assert!(
        p.evidence
            .contains(&"Exit code 1: connection refused".to_string()),
        "{:?}",
        p.evidence
    );
    let (action, effect) = action_of(&p.fixes[0]);
    assert_eq!(
        action,
        &EngineAction::Container {
            engine: EngineId("colima:default".to_string()),
            id,
            action: ContainerAction::Restart { timeout_s: None },
        }
    );
    assert_eq!(effect.result_label(), "Restarted 1 container");
}

#[test]
fn problems_unhealthy_ignores_healthy_starting_and_stopped_containers() {
    for (health, state) in [
        (Some(Health::Healthy), ContainerState::Running),
        (Some(Health::Starting), ContainerState::Running),
        (None, ContainerState::Running),
        (Some(Health::Unhealthy), ContainerState::Exited),
    ] {
        let mut i = input();
        i.containers.push(unhealthy_facts(health, state));
        assert!(evaluate(&i).is_empty(), "{health:?} {state:?}");
    }
}

#[test]
fn problems_evidence_never_shows_an_env_value() {
    let secret = "fake-token-4f9a2c71";
    let inspect = fixture("unhealthy", "inspect.json")
        .replace("PATH=••••", &format!("API_TOKEN={secret}"))
        .replace("connection refused", &format!("401 for token {secret}"));
    let mut i = input();
    let mut cf = with_inspect(broken_facts("unhealthy", ContainerState::Running), &inspect);
    cf.container.health = Some(Health::Unhealthy);
    i.containers.push(cf);

    let p = only(evaluate(&i));

    let shown = format!("{p:?}");
    assert!(!shown.contains(secret), "{shown}");
    assert!(
        p.evidence.iter().any(|e| e.contains("401 for token ••••")),
        "{:?}",
        p.evidence
    );
}

// ---------------------------------------------------------------------------------------------
// BindPathMissing
// ---------------------------------------------------------------------------------------------

#[test]
fn problems_bind_missing_from_the_engines_start_error() {
    let mut i = input();
    i.containers.push(with_inspect(
        broken_facts("badmount", ContainerState::Created),
        &fixture("bind_missing", "inspect.json"),
    ));

    let p = only(evaluate(&i));

    assert_eq!(
        p.kind,
        ProblemKind::BindPathMissing {
            path: "/nonexistent-berthfx-path".to_string()
        }
    );
    assert!(matches!(p.subject, Subject::Container(_)));
    assert!(p.title.contains("/nonexistent-berthfx-path"));
}

#[test]
fn problems_bind_missing_from_compose_output_belongs_to_the_project() {
    let mut i = input();
    i.projects.push(broken_project(
        "bind_missing",
        &lines("bind_missing", "compose_failure.txt"),
    ));

    let p = only(evaluate(&i));

    assert_eq!(p.subject, Subject::Project(project_id("berthfx-broken")));
}

#[test]
fn problems_bind_missing_from_compose_output_names_the_service() {
    let mut i = input();
    i.projects.push(broken_project(
        "bind_missing",
        &[
            " Container berthfx-broken-badmount-1 Creating".to_string(),
            " service:badmount:1 Error response from daemon: invalid mount config for type \"bind\": bind source path does not exist: /nonexistent-berthfx-path ".to_string(),
        ],
    ));

    let p = only(evaluate(&i));

    assert!(
        p.explanation.contains("badmount"),
        "the card does not say which service: {}",
        p.explanation
    );
}

#[test]
fn problems_bind_missing_is_raised_once_when_both_sources_say_it() {
    let mut i = input();
    i.containers.push(with_inspect(
        broken_facts("badmount", ContainerState::Created),
        &fixture("bind_missing", "inspect.json"),
    ));
    i.projects.push(broken_project(
        "bind_missing",
        &lines("bind_missing", "compose_failure.txt"),
    ));

    assert_eq!(evaluate(&i).len(), 1);
}

#[test]
fn problems_bind_missing_ignores_a_started_mount() {
    let mut i = input();
    i.projects.push(broken_project(
        "bind_missing",
        &lines("bind_missing", "mount_ok.txt"),
    ));

    assert!(evaluate(&i).is_empty());
}

#[test]
fn problems_the_compose_fixture_does_not_let_docker_create_the_missing_folder() {
    // Without `create_host_path: false` the engine makes the folder and the problem never occurs.
    assert!(fixture("bind_missing", "compose.yaml").contains("create_host_path: false"));
    assert!(std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/compose/berthfx-broken/compose.yaml")
    )
    .expect("broken fixture")
    .contains("create_host_path: false"));
}

// ---------------------------------------------------------------------------------------------
// BindPermission
// ---------------------------------------------------------------------------------------------

fn uploads_mount() -> Mount {
    Mount {
        kind: MountKind::Bind,
        source: "/Users/me/Dev/shop/uploads".to_string(),
        destination: "/srv/uploads".to_string(),
        read_only: false,
        volume_name: None,
    }
}

#[test]
fn problems_bind_permission_when_the_engine_cannot_create_the_source() {
    let mut i = input();
    i.projects.push(project_facts(
        "shop",
        None,
        &lines("bind_permission", "compose_failure.txt"),
    ));

    let p = only(evaluate(&i));

    assert_eq!(
        p.kind,
        ProblemKind::BindPermission {
            path: "/host_mnt/Users/me/Documents/shop/uploads".to_string()
        }
    );
}

#[test]
fn problems_bind_permission_when_the_program_is_denied_inside_a_bind_mount() {
    let mut i = input();
    let mut cf = facts(container(
        "shop-web-1",
        Some("web"),
        "shop",
        ContainerState::Running,
    ));
    cf.container.mounts.push(uploads_mount());
    cf.error_lines = lines("bind_permission", "logs.txt");
    i.containers.push(cf);

    let p = only(evaluate(&i));

    // The fix is about the folder on the Mac, not the path inside the container.
    assert_eq!(
        p.kind,
        ProblemKind::BindPermission {
            path: "/Users/me/Dev/shop/uploads".to_string()
        }
    );
}

#[test]
fn problems_bind_permission_ignores_a_denial_outside_any_bind_mount() {
    let mut i = input();
    let mut cf = facts(container(
        "shop-web-1",
        Some("web"),
        "shop",
        ContainerState::Running,
    ));
    cf.container.mounts.push(uploads_mount());
    cf.error_lines = lines("bind_permission", "unrelated_denied.txt");
    i.containers.push(cf);
    let mut loose = facts(container("tool", None, "", ContainerState::Running));
    loose.error_lines = lines("bind_permission", "logs.txt");
    i.containers.push(loose);

    assert!(evaluate(&i).is_empty());
}

#[test]
fn problems_bind_permission_ignores_a_denial_inside_a_named_volume() {
    // A volume lives inside the engine; there is no folder on the Mac to grant access to.
    let mut i = input();
    let mut cf = facts(container(
        "shop-web-1",
        Some("web"),
        "shop",
        ContainerState::Running,
    ));
    cf.container.mounts.push(Mount {
        kind: MountKind::Volume,
        source: "/var/lib/docker/volumes/shop_uploads/_data".to_string(),
        destination: "/srv/uploads".to_string(),
        read_only: false,
        volume_name: Some("shop_uploads".to_string()),
    });
    cf.error_lines = lines("bind_permission", "logs.txt");
    i.containers.push(cf);

    assert!(evaluate(&i).is_empty());
}

// ---------------------------------------------------------------------------------------------
// DiskFull
// ---------------------------------------------------------------------------------------------

fn disk(file: &str) -> berth_core::DiskUsage {
    serde_json::from_str(&fixture("disk_full", file)).expect("df fixture")
}

#[test]
fn problems_disk_full_from_the_error_text_offers_to_remove_stopped_containers() {
    let mut i = input();
    let mut db = facts(container(
        "shop-db-1",
        Some("db"),
        "shop",
        ContainerState::Running,
    ));
    db.error_lines = lines("disk_full", "logs.txt");
    i.containers.push(db);
    i.containers.push(facts(container(
        "old-job",
        None,
        "",
        ContainerState::Exited,
    )));

    let p = only(evaluate(&i));

    assert_eq!(p.kind, ProblemKind::DiskFull);
    assert_eq!(p.severity, Severity::Error);
    assert_eq!(
        p.subject,
        Subject::Engine(EngineId("colima:default".to_string()))
    );
    assert_eq!(p.fixes[0].label, "Remove 1 stopped container");
    let (action, effect) = action_of(&p.fixes[0]);
    let EngineAction::Prune { plan } = action else {
        panic!("expected a prune, got {action:?}");
    };
    assert_eq!(plan.items.len(), 1);
    assert_eq!(plan.items[0].name, "old-job");
    assert_eq!(effect.result_label(), "Removed 1 item (0 B)");
}

#[test]
fn problems_disk_full_warns_before_the_disk_is_full() {
    let mut i = input();
    i.disk = Some(disk("df.json"));

    let p = only(evaluate(&i));

    assert_eq!(p.kind, ProblemKind::DiskFull);
    assert_eq!(p.severity, Severity::Warning);
    assert!(p.evidence[0].contains("98%"), "{:?}", p.evidence);
    // Nothing stopped to remove: only the pointer to Clean up.
    assert_eq!(p.fixes.len(), 1);
    assert_eq!(p.fixes[0].effect, FixEffect::Explain);
}

#[test]
fn problems_disk_full_ignores_a_roomy_disk() {
    let mut i = input();
    i.disk = Some(disk("df_roomy.json"));

    assert!(evaluate(&i).is_empty());
}

// ---------------------------------------------------------------------------------------------
// EngineSocketMissing
// ---------------------------------------------------------------------------------------------

fn with_state(state: EngineState) -> ProblemInput {
    let mut i = input();
    i.engine.state = state;
    i
}

#[test]
fn problems_socket_missing_stopped_colima_offers_start() {
    let p = only(evaluate(&with_state(EngineState::Stopped)));

    assert_eq!(p.kind, ProblemKind::EngineSocketMissing);
    assert_eq!(p.severity, Severity::Warning);
    assert_eq!(
        p.subject,
        Subject::Engine(EngineId("colima:default".to_string()))
    );
    assert_eq!(p.fixes[0].label, "Start Colima");
    let (action, effect) = action_of(&p.fixes[0]);
    assert_eq!(
        action,
        &EngineAction::Colima {
            profile: "default".to_string(),
            op: ColimaOp::Start
        }
    );
    assert_eq!(effect.result_label(), "Started 1 engine");
}

#[test]
fn problems_socket_missing_unreachable_colima_offers_restart_and_shows_why() {
    let reason = fixture("socket_missing", "unreachable_reason.txt");
    let p = only(evaluate(&with_state(EngineState::Unreachable {
        reason: reason.trim().to_string(),
        last_seen: None,
    })));

    assert_eq!(p.severity, Severity::Error);
    let (action, effect) = action_of(&p.fixes[0]);
    assert_eq!(
        action,
        &EngineAction::Colima {
            profile: "default".to_string(),
            op: ColimaOp::Restart
        }
    );
    assert_eq!(effect.result_label(), "Restarted 1 engine");
    assert!(p.evidence[0].contains("Cannot connect to the Docker daemon"));
}

#[test]
fn problems_socket_missing_leaves_running_and_remote_engines_alone() {
    assert!(evaluate(&with_state(EngineState::Running)).is_empty());
    let mut remote = with_state(EngineState::Unreachable {
        reason: "timed out".to_string(),
        last_seen: None,
    });
    remote.engine.kind = EngineKind::Remote {
        transport: berth_core::engine::RemoteTransport::Ssh {
            host: "prod".to_string(),
            user: None,
            port: None,
        },
    };
    assert!(evaluate(&remote).is_empty());
}

#[test]
fn problems_socket_missing_on_a_plain_socket_only_explains() {
    let mut i = with_state(EngineState::Stopped);
    i.engine.kind = EngineKind::Socket {
        path: "/var/run/docker.sock".into(),
        context: None,
    };

    let p = only(evaluate(&i));

    assert_eq!(p.fixes[0].effect, FixEffect::Explain);
}

// ---------------------------------------------------------------------------------------------
// NetworkNotFound
// ---------------------------------------------------------------------------------------------

#[test]
fn problems_network_not_found_external_network_in_compose_output() {
    let mut i = input();
    i.projects.push(project_facts(
        "edge",
        Some(&fixture("network_not_found", "compose.yaml")),
        &lines("network_not_found", "compose_failure.txt"),
    ));

    let p = only(evaluate(&i));

    assert_eq!(
        p.kind,
        ProblemKind::NetworkNotFound {
            name: "berthfx_shared".to_string()
        }
    );
    assert!(p
        .explanation
        .contains("docker network create berthfx_shared"));
}

#[test]
fn problems_network_not_found_from_a_container_start_error() {
    let mut i = input();
    let mut cf = facts(container(
        "edge-web-1",
        Some("web"),
        "edge",
        ContainerState::Created,
    ));
    cf.error_lines = lines("network_not_found", "run_failure.txt");
    i.containers.push(cf);

    let p = only(evaluate(&i));

    assert_eq!(
        p.kind,
        ProblemKind::NetworkNotFound {
            name: "berthfx-edge".to_string()
        }
    );
}

#[test]
fn problems_network_not_found_ignores_a_good_pull() {
    let mut i = input();
    i.projects.push(project_facts(
        "edge",
        None,
        &lines("image_not_found", "pull_ok.txt"),
    ));

    assert!(evaluate(&i).is_empty());
}

// ---------------------------------------------------------------------------------------------
// NameInUse
// ---------------------------------------------------------------------------------------------

const HOLDER_ID: &str = "d3b07384d113edec49eaa6238ad5ff00c1b2a3d4e5f60718293a4b5c6d7e8f90";

fn name_clash(state: ContainerState, extra: &[&str]) -> ProblemInput {
    let mut i = input();
    let mut holder = facts(container("berthfx-db-1", Some("db"), "other", state));
    holder.container.id = berth_core::ContainerId(HOLDER_ID.to_string());
    i.containers.push(holder);
    for name in extra {
        i.containers
            .push(facts(container(name, None, "", ContainerState::Exited)));
    }
    i.projects.push(project_facts(
        "berthfx",
        None,
        &lines("name_in_use", "compose_failure.txt"),
    ));
    i
}

#[test]
fn problems_name_in_use_offers_to_remove_or_rename_the_holder() {
    let i = name_clash(ContainerState::Exited, &[]);

    let p = only(evaluate(&i));

    assert_eq!(
        p.kind,
        ProblemKind::NameInUse {
            name: "berthfx-db-1".to_string()
        }
    );
    let id = berth_core::ContainerId(HOLDER_ID.to_string());
    let (remove, remove_effect) = action_of(&p.fixes[0]);
    assert_eq!(
        remove,
        &EngineAction::Container {
            engine: EngineId("colima:default".to_string()),
            id: id.clone(),
            action: ContainerAction::Remove {
                force: false,
                volumes: false
            },
        }
    );
    assert_eq!(remove_effect.result_label(), "Removed 1 container");
    let (rename, rename_effect) = action_of(&p.fixes[1]);
    assert_eq!(
        rename,
        &EngineAction::Container {
            engine: EngineId("colima:default".to_string()),
            id,
            action: ContainerAction::Rename {
                name: "berthfx-db-1-old".to_string()
            },
        }
    );
    assert_eq!(rename_effect.result_label(), "Renamed 1 container");
}

#[test]
fn problems_name_in_use_removing_a_running_holder_is_forced_and_says_so() {
    let i = name_clash(ContainerState::Running, &[]);

    let p = only(evaluate(&i));

    let (remove, effect) = action_of(&p.fixes[0]);
    assert!(matches!(
        remove,
        EngineAction::Container {
            action: ContainerAction::Remove { force: true, .. },
            ..
        }
    ));
    assert!(
        effect.lines[0].text.contains("running"),
        "{:?}",
        effect.lines
    );
}

#[test]
fn problems_name_in_use_picks_a_new_name_nobody_has() {
    let i = name_clash(ContainerState::Exited, &["berthfx-db-1-old"]);

    let p = only(evaluate(&i));

    let (rename, _) = action_of(&p.fixes[1]);
    assert!(matches!(
        rename,
        EngineAction::Container {
            action: ContainerAction::Rename { name },
            ..
        } if name == "berthfx-db-1-old-2"
    ));
}

#[test]
fn problems_name_in_use_is_resolved_once_the_holder_is_gone() {
    let mut i = input();
    i.projects.push(project_facts(
        "berthfx",
        None,
        &lines("name_in_use", "compose_failure.txt"),
    ));

    assert!(evaluate(&i).is_empty());
}

// ---------------------------------------------------------------------------------------------
// EnvVarMissing
// ---------------------------------------------------------------------------------------------

fn needs_var(env: Option<&str>) -> ProblemInput {
    let mut i = input();
    let mut pf = project_facts(
        "berthfx",
        Some(&fixture("env_missing", "compose.yaml")),
        &[],
    );
    pf.env_text = env.map(ToString::to_string);
    i.projects.push(pf);
    i
}

#[test]
fn problems_env_missing_adds_the_variable_to_dot_env() {
    let env = fixture("env_missing", "env_without_var.env");
    let i = needs_var(Some(&env));

    let p = only(evaluate(&i));

    // GREETING has a default, HOME is the shell's, COMMENTED_OUT is a comment.
    assert_eq!(
        p.kind,
        ProblemKind::EnvVarMissing {
            var: "BERTHFX_MISSING".to_string()
        }
    );
    assert_eq!(p.subject, Subject::Project(project_id("berthfx")));
    assert_eq!(p.fixes[0].label, "Add BERTHFX_MISSING to .env");
    let plan = plan_of(&p.fixes[0]);
    assert_eq!(plan.files[0].path, Path::new(".env"));
    assert_eq!(plan.files[0].old_sha256, Some(diff::sha256(&env)));
    let text = new_text(&p.fixes[0]);
    assert!(text.starts_with(&env), "existing lines stay: {text}");
    assert!(text.ends_with("BERTHFX_MISSING=\n"), "{text}");
    assert_eq!(p.fixes[0].label, "Add BERTHFX_MISSING to .env");
    assert_eq!(plan.effect().button_label(), "Write 1 file");
    assert!(p.evidence[0].contains("line 6"), "{:?}", p.evidence);
}

#[test]
fn problems_env_missing_creates_dot_env_when_there_is_none() {
    let p = only(evaluate(&needs_var(None)));

    let plan = plan_of(&p.fixes[0]);
    assert_eq!(plan.files[0].change, berth_core::FileChange::Create);
    assert_eq!(plan.files[0].old_sha256, None);
    assert_eq!(new_text(&p.fixes[0]), "BERTHFX_MISSING=\n");
}

#[test]
fn problems_env_missing_ignores_a_variable_dot_env_sets() {
    let env = fixture("env_missing", "env_with_var.env");

    assert!(evaluate(&needs_var(Some(&env))).is_empty());
}

#[test]
fn problems_env_missing_counts_required_forms_and_skips_escapes_and_defaults() {
    let compose = "services:\n  a:\n    image: x\n    environment:\n      ONE: ${NEEDS_MESSAGE:?set it}\n      TWO: $${LITERAL}\n      THREE: ${WITH_DEFAULT-x}\n      FOUR: ${ALT:+yes}\n      FIVE: $BARE\n";
    let mut i = input();
    i.projects.push(project_facts("demo", Some(compose), &[]));

    let vars: Vec<String> = evaluate(&i)
        .into_iter()
        .filter_map(|p| match p.kind {
            ProblemKind::EnvVarMissing { var } => Some(var),
            _ => None,
        })
        .collect();

    assert_eq!(vars, vec!["NEEDS_MESSAGE".to_string(), "BARE".to_string()]);
}

// ---------------------------------------------------------------------------------------------
// DependencyFailed
// ---------------------------------------------------------------------------------------------

fn shop_project(failure: &[String]) -> ProjectFacts {
    project_facts("shop-api", None, failure)
}

#[test]
fn problems_dependency_failed_names_the_service_and_offers_to_restart_it() {
    let mut i = input();
    i.projects.push(shop_project(&lines(
        "dependency_failed",
        "compose_failure.txt",
    )));

    let p = only(evaluate(&i));

    assert_eq!(
        p.kind,
        ProblemKind::DependencyFailed {
            service: "db".to_string()
        }
    );
    assert_eq!(p.subject, Subject::Project(project_id("shop-api")));
    let (action, effect) = action_of(&p.fixes[0]);
    let EngineAction::Compose { target, command } = action else {
        panic!("expected a Compose action, got {action:?}");
    };
    assert_eq!(
        command,
        &ComposeCommand::Restart {
            services: vec!["db".to_string()]
        }
    );
    assert_eq!(target.project_name, "shop-api");
    assert_eq!(target.folder, Path::new("/work/shop-api"));
    assert_eq!(effect.result_label(), "Restarted 1 service");
}

#[test]
fn problems_dependency_failed_prefers_the_services_own_label_over_the_container_name() {
    let mut i = input();
    i.containers.push(facts(container(
        "shop-api-db-1",
        Some("database"),
        "shop-api",
        ContainerState::Running,
    )));
    i.projects.push(shop_project(&lines(
        "dependency_failed",
        "compose_failure.txt",
    )));

    let p = only(evaluate(&i));

    assert_eq!(
        p.kind,
        ProblemKind::DependencyFailed {
            service: "database".to_string()
        }
    );
}

#[test]
fn problems_dependency_failed_one_shot_service_that_did_not_complete() {
    let mut i = input();
    i.projects.push(shop_project(&lines(
        "dependency_failed",
        "migrate_failure.txt",
    )));

    let p = only(evaluate(&i));

    assert_eq!(
        p.kind,
        ProblemKind::DependencyFailed {
            service: "migrate".to_string()
        }
    );
}

#[test]
fn problems_dependency_failed_without_a_docker_host_can_only_explain() {
    let mut i = input();
    i.engine.docker_host = None;
    i.projects.push(shop_project(&lines(
        "dependency_failed",
        "compose_failure.txt",
    )));

    let p = only(evaluate(&i));

    assert_eq!(p.fixes[0].effect, FixEffect::Explain);
}

#[test]
fn problems_dependency_failed_ignores_a_clean_run() {
    let mut i = input();
    i.projects
        .push(shop_project(&lines("build_failed", "build_ok.txt")));

    assert!(evaluate(&i).is_empty());
}

// ---------------------------------------------------------------------------------------------
// BuildFailed
// ---------------------------------------------------------------------------------------------

#[test]
fn problems_build_failed_names_the_failing_step() {
    let mut i = input();
    i.projects
        .push(shop_project(&lines("build_failed", "compose_failure.txt")));

    let p = only(evaluate(&i));

    assert_eq!(
        p.kind,
        ProblemKind::BuildFailed {
            step: Some("RUN npm ci".to_string())
        }
    );
    assert!(p.title.contains("RUN npm ci"), "{}", p.title);
    assert!(
        p.evidence.iter().any(|e| e.starts_with("failed to solve")),
        "{:?}",
        p.evidence
    );
}

#[test]
fn problems_build_failed_without_a_step_still_reports_the_failure() {
    let mut i = input();
    i.projects.push(shop_project(&[
        "failed to solve: failed to read dockerfile: open Dockerfile: no such file or directory"
            .to_string(),
    ]));

    let p = only(evaluate(&i));

    assert_eq!(p.kind, ProblemKind::BuildFailed { step: None });
}

#[test]
fn problems_build_failed_ignores_a_good_build_and_a_failed_dependency() {
    let mut i = input();
    i.projects
        .push(shop_project(&lines("build_failed", "build_ok.txt")));
    assert!(evaluate(&i).is_empty());

    let mut j = input();
    j.projects.push(shop_project(&lines(
        "dependency_failed",
        "migrate_failure.txt",
    )));
    assert!(evaluate(&j)
        .iter()
        .all(|p| !matches!(p.kind, ProblemKind::BuildFailed { .. })));
}

// ---------------------------------------------------------------------------------------------
// Across rules
// ---------------------------------------------------------------------------------------------

#[test]
fn problems_nothing_to_report_for_an_empty_input() {
    assert!(evaluate(&input()).is_empty());
}

#[test]
fn problems_every_action_fix_reads_in_the_past_tense_in_its_result() {
    // Name clash (remove, rename), unhealthy (restart), loose OOM (recreate) and an engine to start.
    let mut i = name_clash(ContainerState::Running, &[]);
    i.containers.push(unhealthy_facts(
        Some(Health::Unhealthy),
        ContainerState::Running,
    ));
    i.containers.push(with_inspect(
        facts(container("loose-oom", None, "", ContainerState::Exited)),
        &fixture("oom", "inspect.json"),
    ));
    let mut stopped = input();
    stopped.engine.state = EngineState::Stopped;
    let mut all = evaluate(&i);
    all.extend(evaluate(&stopped));

    let mut seen = Vec::new();
    for fix in all.iter().flat_map(|p| &p.fixes) {
        if let FixEffect::RunAction { effect, .. } = &fix.effect {
            let past = match effect.verb.as_str() {
                "Remove" => "Removed",
                "Rename" => "Renamed",
                "Restart" => "Restarted",
                "Recreate" => "Recreated",
                "Start" => "Started",
                other => panic!("verb {other} is not one core knows"),
            };
            assert!(
                effect.result_label().starts_with(past),
                "{} -> {}",
                effect.verb,
                effect.result_label()
            );
            seen.push(effect.verb.clone());
        }
    }
    seen.sort();
    seen.dedup();
    assert_eq!(seen, ["Recreate", "Remove", "Rename", "Restart", "Start"]);
}

#[test]
fn problems_the_same_failure_gets_the_same_id_each_time() {
    let build = || {
        let mut i = input();
        i.containers.push(portclash());
        i.projects.push(broken_project("port_allocated", &[]));
        evaluate(&i)
    };

    assert_eq!(build()[0].id, build()[0].id);
}

#[test]
fn problems_a_stable_engine_for_the_fixtures() {
    // The tests above lean on these facts about the Colima fixture engine.
    let e = engine();
    assert!(matches!(e.kind, EngineKind::Colima { .. }));
    assert_eq!(
        e.resources.as_ref().map(|r| r.vm_type.clone()),
        Some(VmType::Vz)
    );
}
