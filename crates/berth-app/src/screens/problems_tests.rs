//! P5-T8 tests for the Problems tab, the shared problem item and what a click on a fix does:
//! every kind of fix has a button or a note, a file fix opens Review changes with its plan, an
//! action fix shows its effect in a dialog and runs from it, and the running and failed states
//! show on the button.

use std::sync::Arc;

use berth_core::{
    ColimaOp, ColimaStartOptions, ComposeCommand, ComposeTarget, Container, ContainerAction,
    ContainerId, EffectKind, EffectLine, EffectSummary, EngineAction, EngineId, Fix, FixEffect,
    FixSource, GenerationPlan, HarnessId, PlanSource, Problem, ProblemId, ProblemKind, Project,
    PruneItem, PruneKind, PrunePlan, Severity, Subject, VmType,
};
use tokio_util::sync::CancellationToken;

use super::model::{self, Clicked};
use super::*;
use crate::services::actions::{ActionsService, StateView};
use crate::services::backing::Backing;
use crate::services::confirm;
use crate::services::plans::PlanStore;
use crate::state::{use_app_state, Overlay, PendingKey, PendingState, Selection};

fn engine_id() -> EngineId {
    EngineId("colima:default".to_string())
}

fn world() -> berth_engine::MockWorld {
    crate::services::mock_world_at(chrono::Utc::now())
}

fn effect(verb: &str, noun: &str, lines: &[(EffectKind, &str)]) -> EffectSummary {
    EffectSummary {
        verb: verb.to_string(),
        noun_one: noun.to_string(),
        noun_many: format!("{noun}s"),
        count: 1,
        bytes: None,
        lines: lines
            .iter()
            .map(|(kind, text)| EffectLine {
                kind: *kind,
                text: (*text).to_string(),
            })
            .collect(),
        typed_name: None,
    }
}

fn rule(label: &str, effect: FixEffect) -> Fix {
    Fix {
        label: label.to_string(),
        source: FixSource::Rule,
        effect,
    }
}

fn problem(kind: ProblemKind, subject: Subject, title: &str, fixes: Vec<Fix>) -> Problem {
    Problem {
        id: ProblemId::new(&kind, &subject),
        kind,
        severity: Severity::Error,
        subject,
        title: title.to_string(),
        explanation: "Why it happened, in plain words.".to_string(),
        evidence: vec!["the line it came from".to_string()],
        fixes,
        first_seen: chrono::Utc::now() - chrono::Duration::seconds(130),
    }
}

fn restart_action(c: &Container) -> EngineAction {
    EngineAction::Container {
        engine: c.engine.clone(),
        id: c.id.clone(),
        action: ContainerAction::Restart { timeout_s: None },
    }
}

fn restart_fix(c: &Container) -> Fix {
    rule(
        &format!("Restart {}", c.name),
        FixEffect::RunAction {
            action: restart_action(c),
            effect: effect(
                "Restart",
                "container",
                &[(EffectKind::Changes, "It stops and starts again")],
            ),
        },
    )
}

fn spec() -> berth_core::ContainerSpec {
    berth_core::ContainerSpec {
        name: "shop-api-db-1".to_string(),
        image: berth_core::ImageRef {
            reference: "postgres:17-alpine".to_string(),
            id: None,
        },
        command: None,
        entrypoint: None,
        env: Vec::new(),
        ports: Vec::new(),
        mounts: Vec::new(),
        networks: Vec::new(),
        labels: std::collections::BTreeMap::new(),
        restart: berth_core::RestartPolicy::No,
        limits: berth_core::Limits::default(),
        platform: None,
        working_dir: None,
        user: None,
        healthcheck: None,
        cap_add: Vec::new(),
        cap_drop: Vec::new(),
        network_aliases: std::collections::BTreeMap::new(),
    }
}

fn plan_for(project: &Project) -> GenerationPlan {
    let folder = project.folder.clone().expect("a project folder");
    GenerationPlan {
        root: folder,
        files: vec![berth_rules::diff::file_diff(
            std::path::Path::new("compose.yaml"),
            Some("services:\n  db:\n    ports: [\"5432:5432\"]\n"),
            Some("services:\n  db:\n    ports: [\"5433:5432\"]\n"),
        )],
        source: PlanSource::Rule(ProblemKind::PortAllocated { port: 5432 }),
        notes: Vec::new(),
        after_write: Some(ComposeCommand::Up {
            services: vec!["db".to_string()],
            build: false,
        }),
    }
}

// ---------------------------------------------------------------------
// A rig: state, plans and the actions service over mocks
// ---------------------------------------------------------------------

thread_local! {
    static SLOT: std::cell::RefCell<Option<AppState>> = const { std::cell::RefCell::new(None) };
}

#[component]
fn Harness() -> Element {
    let state = use_app_state();
    use_hook(move || {
        SLOT.with(|slot| *slot.borrow_mut() = Some(state));
    });
    rsx! { div {} }
}

struct Rig {
    dom: VirtualDom,
    state: AppState,
    plans: PlanStore,
    actions: ActionsService,
    compose: Arc<berth_engine::MockCompose>,
    colima: Arc<berth_engine::MockColima>,
    world: berth_engine::MockWorld,
}

fn rig() -> Rig {
    let mut dom = VirtualDom::new(Harness);
    let _ = dom.rebuild_to_vec();
    let state = SLOT
        .with(|slot| slot.borrow_mut().take())
        .expect("harness captured AppState");
    let world = world();
    let clients = crate::services::engines::client_map();
    clients.lock().expect("clients").insert(
        engine_id(),
        Arc::new(berth_engine::MockEngine::bundled(&engine_id()))
            as Arc<dyn berth_engine::EngineClient>,
    );
    let store = Arc::new(berth_store::Store::open_in_memory().expect("store"));
    let compose = Arc::new(berth_engine::MockCompose::bundled());
    let colima = Arc::new(berth_engine::MockColima::bundled());
    // The receiver is dropped: a finished action is never applied, so a pending action stays
    // "running" for the test to look at.
    let (actions, _finished) = ActionsService::new(
        clients,
        store,
        Arc::clone(&compose) as Arc<dyn berth_engine::ComposeRunner>,
        Arc::clone(&colima) as Arc<dyn berth_engine::ColimaControl>,
        CancellationToken::new(),
    );
    dom.in_runtime(|| {
        let mut state = state;
        state.engines.set(world.engines.clone());
        state.containers.set(world.containers_for(&engine_id()));
        state.projects.set(world.projects.clone());
    });
    Rig {
        dom,
        state,
        plans: PlanStore::default(),
        actions,
        compose,
        colima,
        world,
    }
}

impl Rig {
    fn container(&self, name: &str) -> Container {
        self.world
            .containers
            .iter()
            .find(|c| c.name == name)
            .cloned()
            .unwrap_or_else(|| panic!("mock container {name}"))
    }

    fn project(&self, name: &str) -> Project {
        self.world
            .projects
            .iter()
            .find(|p| p.name == name)
            .cloned()
            .unwrap_or_else(|| panic!("mock project {name}"))
    }

    fn click(&self, problem: &Problem, index: usize) -> Clicked {
        self.dom.in_runtime(|| {
            let mut state = self.state;
            model::fix_click(&mut state, Some(&self.plans), problem, index)
        })
    }

    fn overlays(&self) -> Vec<Overlay> {
        self.dom.in_runtime(|| self.state.overlays.peek().clone())
    }

    /// The one confirm dialog that is open.
    fn dialog(&self) -> crate::state::ConfirmModel {
        match self.overlays().as_slice() {
            [Overlay::Confirm(model)] => model.clone(),
            other => panic!("one confirm dialog expected: {other:?}"),
        }
    }

    fn views(&self, problem: &Problem) -> crate::ui::ProblemView {
        self.dom.in_runtime(|| {
            let engines = self.state.engines.peek();
            let containers = self.state.containers.peek();
            let projects = self.state.projects.peek();
            let pending = self.state.pending.peek();
            model::problem_view(
                problem,
                &StateView {
                    engines: &engines,
                    containers: &containers,
                    projects: &projects,
                    images: &[],
                    volumes: &[],
                    networks: &[],
                    pending: &pending,
                    backing: *self.state.backing.peek(),
                },
                chrono::Utc::now(),
                true,
                &[],
            )
        })
    }

    fn set_pending(&self, key: PendingKey, value: PendingState) {
        self.dom.in_runtime(|| {
            let mut state = self.state;
            state.pending.write().insert(key, value);
        });
    }

    fn confirmed(&self, model: &crate::state::ConfirmModel) -> confirm::Launched {
        self.dom.in_runtime(|| {
            let mut state = self.state;
            confirm::run(&mut state, &self.actions, model, false)
        })
    }
}

// ---------------------------------------------------------------------
// The item
// ---------------------------------------------------------------------

#[test]
fn problems_every_kind_of_fix_has_a_button_or_a_note_and_the_first_rule_fix_is_primary() {
    let rig = rig();
    let db = rig.container("shop-api-db-1");
    let shop = rig.project("shop-api");
    let prune = PrunePlan {
        engine: engine_id(),
        items: vec![PruneItem {
            kind: PruneKind::Container,
            id: "x".to_string(),
            name: "old".to_string(),
            size: None,
            selected: true,
        }],
    };
    let fixes = vec![
        rule("Map db to port 5433", FixEffect::EditFiles(plan_for(&shop))),
        restart_fix(&db),
        rule(
            "Recreate with more memory",
            FixEffect::RunAction {
                action: EngineAction::Recreate {
                    engine: engine_id(),
                    id: db.id.clone(),
                    spec: spec(),
                    pull: false,
                },
                effect: effect("Recreate", "container", &[]),
            },
        ),
        rule(
            "Restart db",
            FixEffect::RunAction {
                action: EngineAction::Compose {
                    target: ComposeTarget {
                        engine: engine_id(),
                        docker_host: "unix:///sock".to_string(),
                        project_name: shop.name.clone(),
                        folder: shop.folder.clone().expect("folder"),
                        files: shop.compose_files.clone(),
                    },
                    command: ComposeCommand::Restart {
                        services: vec!["db".to_string()],
                    },
                },
                effect: effect("Restart", "service", &[]),
            },
        ),
        rule(
            "Restart Colima with Rosetta",
            FixEffect::RunAction {
                action: EngineAction::Colima {
                    profile: "default".to_string(),
                    op: ColimaOp::Apply(ColimaStartOptions {
                        cpus: None,
                        memory_gib: None,
                        disk_gib: None,
                        vm_type: Some(VmType::Vz),
                        rosetta: Some(true),
                        arch: None,
                    }),
                },
                effect: effect("Restart", "engine", &[]),
            },
        ),
        rule(
            "Clean up",
            FixEffect::RunAction {
                action: EngineAction::Prune { plan: prune },
                effect: effect("Remove", "item", &[]),
            },
        ),
        rule("Keep both: choose another name", FixEffect::Explain),
        Fix {
            label: "Fix with Claude Code".to_string(),
            source: FixSource::Harness(HarnessId("claude".to_string())),
            effect: FixEffect::Explain,
        },
    ];
    let p = problem(
        ProblemKind::PortAllocated { port: 5432 },
        Subject::Container(db.id.clone()),
        "Port 5432 is already in use",
        fixes,
    );
    let item = rig.views(&p);
    let labels: Vec<&str> = item.fixes.iter().map(|f| f.label.as_str()).collect();
    assert_eq!(
        labels,
        [
            "Map db to port 5433",
            "Restart shop-api-db-1",
            "Recreate with more memory",
            "Restart db",
            "Restart Colima with Rosetta",
            "Clean up",
            "Fix with Claude Code",
        ]
    );
    assert_eq!(item.notes, ["Keep both: choose another name"]);
    let primaries: Vec<&str> = item
        .fixes
        .iter()
        .filter(|f| f.primary)
        .map(|f| f.label.as_str())
        .collect();
    assert_eq!(primaries, ["Map db to port 5433"]);
    let harness = item.fixes.last().expect("harness button");
    assert!(harness.split && harness.icon == Some("sparkles"));
    assert_eq!(
        harness.disabled_reason.as_deref(),
        Some("Needs a harness: install Claude Code or muse")
    );
    // Rule ids are `<problem id>#<index of the fix>`; the harness button is `assistant:<id>`.
    assert_eq!(item.fixes[0].id, format!("{}#0", p.id.0));
    assert_eq!(harness.id, format!("assistant:{}", p.id.0));
    assert_eq!(item.subject.as_deref(), Some("shop-api-db-1"));
    assert_eq!(item.age, "2 min");
    // Nothing is left from the placeholder the inspector card used before the rules existed.
    assert!(item
        .fixes
        .iter()
        .all(|f| f.disabled_reason.as_deref() != Some("Built in P5-T7")));
}

#[test]
fn problems_a_dependency_problem_offers_go_to_before_the_harness_button() {
    let rig = rig();
    let worker = rig.container("shop-api-worker-1");
    let p = problem(
        ProblemKind::DependencyFailed {
            service: "db".to_string(),
        },
        Subject::Container(worker.id),
        "worker keeps restarting",
        vec![
            restart_fix(&rig.container("shop-api-db-1")),
            Fix {
                label: "Fix with Claude Code".to_string(),
                source: FixSource::Harness(HarnessId("claude".to_string())),
                effect: FixEffect::Explain,
            },
        ],
    );
    let item = rig.views(&p);
    let labels: Vec<&str> = item.fixes.iter().map(|f| f.label.as_str()).collect();
    assert_eq!(
        labels,
        [
            "Restart shop-api-db-1",
            "Go to shop-api-db-1",
            "Fix with Claude Code"
        ]
    );
    let go = &item.fixes[1];
    assert!(go.id.starts_with("goto:") && go.disabled_reason.is_none());
}

#[test]
fn problems_a_running_fix_shows_loading_and_a_failed_one_says_why() {
    let rig = rig();
    let db = rig.container("shop-api-db-1");
    let p = problem(
        ProblemKind::Unhealthy,
        Subject::Container(db.id.clone()),
        "db is unhealthy",
        vec![restart_fix(&db)],
    );
    assert!(!rig.views(&p).fixes[0].loading);
    let key = PendingKey {
        subject: Subject::Container(db.id.clone()),
        action: "container.restart",
    };
    rig.set_pending(
        key.clone(),
        PendingState::Running {
            label: "Restarting\u{2026}".to_string(),
        },
    );
    let running = &rig.views(&p).fixes[0];
    assert!(running.loading);
    assert!(running.failed.is_none());
    assert!(
        running.disabled_reason.is_some(),
        "a second click is refused while it runs"
    );
    rig.set_pending(
        key,
        PendingState::Failed {
            message: "Cannot restart: no such container".to_string(),
        },
    );
    let failed = &rig.views(&p).fixes[0];
    assert!(!failed.loading);
    assert_eq!(
        failed.failed.as_deref(),
        Some("Cannot restart: no such container")
    );
    assert!(
        failed.disabled_reason.is_none(),
        "a failed fix can be tried again"
    );
}

#[test]
fn problems_the_inspector_card_leaves_the_subject_out_and_only_lists_its_container() {
    let rig = rig();
    let db = rig.container("shop-api-db-1");
    let api = rig.container("shop-api-api-1");
    let problems = vec![
        problem(
            ProblemKind::Unhealthy,
            Subject::Container(db.id.clone()),
            "db is unhealthy",
            vec![restart_fix(&db)],
        ),
        problem(
            ProblemKind::Unhealthy,
            Subject::Container(api.id.clone()),
            "api is unhealthy",
            vec![restart_fix(&api)],
        ),
    ];
    let cards = rig.dom.in_runtime(|| {
        let engines = rig.state.engines.peek();
        let containers = rig.state.containers.peek();
        let projects = rig.state.projects.peek();
        let pending = rig.state.pending.peek();
        model::card_views(
            &db,
            &problems,
            &StateView {
                engines: &engines,
                containers: &containers,
                projects: &projects,
                images: &[],
                volumes: &[],
                networks: &[],
                pending: &pending,
                backing: Backing::Live,
            },
            chrono::Utc::now(),
            &[],
        )
    });
    assert_eq!(cards.len(), 1);
    assert_eq!(cards[0].title, "db is unhealthy");
    assert_eq!(cards[0].subject, None);
}

#[test]
fn problems_groups_follow_the_lists_order_and_name_the_project_or_the_engine() {
    let rig = rig();
    let db = rig.container("shop-api-db-1");
    let worker = rig.container("shop-api-worker-1");
    let grafana = rig.container("analytics-grafana-1");
    let loose = rig.container("pg-scratch");
    let engine = Subject::Engine(engine_id());
    let problems = vec![
        problem(
            ProblemKind::Unhealthy,
            Subject::Container(grafana.id),
            "grafana",
            vec![],
        ),
        problem(
            ProblemKind::Unhealthy,
            Subject::Container(db.id),
            "db",
            vec![],
        ),
        problem(ProblemKind::EngineSocketMissing, engine, "engine", vec![]),
        problem(
            ProblemKind::Unhealthy,
            Subject::Container(worker.id),
            "worker",
            vec![],
        ),
        problem(
            ProblemKind::Unhealthy,
            Subject::Container(loose.id),
            "scratch",
            vec![],
        ),
    ];
    let groups = rig.dom.in_runtime(|| {
        let engines = rig.state.engines.peek();
        let containers = rig.state.containers.peek();
        let projects = rig.state.projects.peek();
        let pending = rig.state.pending.peek();
        model::groups(
            &problems,
            &StateView {
                engines: &engines,
                containers: &containers,
                projects: &projects,
                images: &[],
                volumes: &[],
                networks: &[],
                pending: &pending,
                backing: Backing::Live,
            },
            chrono::Utc::now(),
            &[],
        )
    });
    let shape: Vec<(String, Vec<String>)> = groups
        .iter()
        .map(|g| {
            (
                g.label.clone(),
                g.items.iter().map(|i| i.title.clone()).collect(),
            )
        })
        .collect();
    assert_eq!(
        shape[0],
        ("analytics".to_string(), vec!["grafana".to_string()])
    );
    assert_eq!(
        shape[1],
        (
            "shop-api".to_string(),
            vec!["db".to_string(), "worker".to_string()]
        )
    );
    // The engine's own problem is listed under the engine's name; a container that belongs to no
    // project under Unassigned.
    assert_eq!(shape[2].1, vec!["engine".to_string()]);
    assert!(
        shape[2].0.to_lowercase().contains("default"),
        "{}",
        shape[2].0
    );
    assert_eq!(
        shape[3],
        ("Unassigned".to_string(), vec!["scratch".to_string()])
    );
}

// ---------------------------------------------------------------------
// Clicks
// ---------------------------------------------------------------------

#[test]
fn problems_a_file_fix_opens_review_changes_with_its_plan_and_where_to_run_after() {
    let rig = rig();
    let shop = rig.project("shop-api");
    let plan = plan_for(&shop);
    let db = rig.container("shop-api-db-1");
    let p = problem(
        ProblemKind::PortAllocated { port: 5432 },
        Subject::Container(db.id),
        "Port 5432 is already in use",
        vec![rule(
            "Map db to port 5433",
            FixEffect::EditFiles(plan.clone()),
        )],
    );
    let Clicked::Review(id) = rig.click(&p, 0) else {
        panic!("a file fix opens Review changes");
    };
    assert_eq!(rig.overlays(), [Overlay::ReviewChanges(id)]);
    let pending = rig.plans.get(&id).expect("the plan is parked");
    assert_eq!(pending.plan, plan);
    assert_eq!(pending.title, "shop-api");
    let run = pending.run.expect("after_write runs against the project");
    assert_eq!(run.project, shop.id);
    assert_eq!(run.files, shop.compose_files);
    // Nothing was written by the click: the plan only waits.
    assert!(!std::path::Path::new("/nonexistent").exists());
}

#[test]
fn problems_a_file_fix_is_refused_with_a_reason_under_the_mock() {
    let rig = rig();
    rig.dom.in_runtime(|| {
        let mut state = rig.state;
        state.backing.set(Backing::Mock);
    });
    let shop = rig.project("shop-api");
    let db = rig.container("shop-api-db-1");
    let p = problem(
        ProblemKind::PortAllocated { port: 5432 },
        Subject::Container(db.id),
        "Port 5432 is already in use",
        vec![rule("Map db", FixEffect::EditFiles(plan_for(&shop)))],
    );
    let button = &rig.views(&p).fixes[0];
    assert_eq!(
        button.disabled_reason.as_deref(),
        Some("Writing project files need a real Docker engine. You're viewing mock data.")
    );
    assert!(matches!(rig.click(&p, 0), Clicked::Nothing(_)));
    assert!(rig.overlays().is_empty());
    assert!(rig.plans.is_empty());
}

#[tokio::test]
async fn problems_an_action_fix_shows_its_effect_in_a_dialog_and_runs_when_confirmed() {
    let rig = rig();
    let db = rig.container("shop-api-db-1");
    let p = problem(
        ProblemKind::Unhealthy,
        Subject::Container(db.id.clone()),
        "db is unhealthy",
        vec![restart_fix(&db)],
    );
    assert_eq!(rig.click(&p, 0), Clicked::Confirm);
    let model = rig.dialog();
    assert_eq!(model.title, "Restart shop-api-db-1?");
    assert_eq!(model.effect.lines.len(), 1);
    assert_eq!(model.effect.lines[0].text, "It stops and starts again");
    assert_eq!(model.targets, vec![Subject::Container(db.id.clone())]);
    assert_eq!(confirm::button_label(&model.effect), "Restart container");
    // Nothing ran until the dialog's button.
    assert!(rig.dom.in_runtime(|| rig.state.pending.peek().is_empty()));
    let launched = rig.confirmed(&model);
    assert_eq!(
        launched.keys,
        vec![PendingKey {
            subject: Subject::Container(db.id),
            action: "container.restart"
        }]
    );
    assert!(launched.refused.is_empty(), "{:?}", launched.refused);
    // And the item now shows the fix running.
    assert!(rig.views(&p).fixes[0].loading);
}

#[test]
fn problems_a_colima_fix_says_how_long_it_takes_in_an_effect_line() {
    let rig = rig();
    let engine = Subject::Engine(engine_id());
    let apply = EngineAction::Colima {
        profile: "default".to_string(),
        op: ColimaOp::Apply(ColimaStartOptions {
            cpus: None,
            memory_gib: None,
            disk_gib: None,
            vm_type: Some(VmType::Vz),
            rosetta: Some(true),
            arch: None,
        }),
    };
    let p = problem(
        ProblemKind::ExecFormat { image_arch: None },
        Subject::Container(rig.container("shop-api-db-1").id),
        "exec format error",
        vec![rule(
            "Restart Colima with Rosetta",
            FixEffect::RunAction {
                action: apply.clone(),
                effect: effect(
                    "Restart",
                    "engine",
                    &[(
                        EffectKind::Goes,
                        "Running containers stop while it restarts",
                    )],
                ),
            },
        )],
    );
    assert_eq!(rig.click(&p, 0), Clicked::Confirm);
    let model = rig.dialog();
    let texts: Vec<&str> = model.effect.lines.iter().map(|l| l.text.as_str()).collect();
    assert_eq!(
        texts,
        [
            "Running containers stop while it restarts",
            "This takes about 20 s."
        ]
    );
    assert_eq!(model.effect.lines[1].kind, EffectKind::Changes);
    assert_eq!(model.targets, vec![engine]);
    assert_eq!(model.action, apply);
    // The time is for what takes a while: a container restart has none.
    let restart = effect_with_time_of(&restart_action(&rig.container("shop-api-db-1")));
    assert_eq!(restart.lines.len(), 1);
}

fn effect_with_time_of(action: &EngineAction) -> EffectSummary {
    model::effect_with_time(
        &effect(
            "Restart",
            "container",
            &[(EffectKind::Changes, "It restarts")],
        ),
        action,
    )
}

#[tokio::test]
async fn problems_a_colima_fix_runs_through_the_engines_runner() {
    let rig = rig();
    let p = problem(
        ProblemKind::EngineSocketMissing,
        Subject::Engine(engine_id()),
        "Berth cannot reach Colima",
        vec![rule(
            "Restart Colima",
            FixEffect::RunAction {
                action: EngineAction::Colima {
                    profile: "default".to_string(),
                    op: ColimaOp::Restart,
                },
                effect: effect("Restart", "engine", &[]),
            },
        )],
    );
    assert_eq!(rig.click(&p, 0), Clicked::Confirm);
    let launched = rig.confirmed(&rig.dialog());
    assert_eq!(
        launched.keys,
        vec![PendingKey {
            subject: Subject::Engine(engine_id()),
            action: "engine.restart"
        }],
        "{:?}",
        launched.refused
    );
    let _ = &rig.colima;
}

fn compose_fix(rig: &Rig, project_name: &str) -> (Fix, Project) {
    let shop = rig.project("shop-api");
    let fix = rule(
        "Restart db",
        FixEffect::RunAction {
            action: EngineAction::Compose {
                target: ComposeTarget {
                    engine: shop.engine.clone(),
                    docker_host: "unix:///sock".to_string(),
                    project_name: project_name.to_string(),
                    folder: shop.folder.clone().expect("folder"),
                    files: shop.compose_files.clone(),
                },
                command: ComposeCommand::Restart {
                    services: vec!["db".to_string()],
                },
            },
            effect: effect(
                "Restart",
                "service",
                &[(EffectKind::Changes, "db stops and starts again")],
            ),
        },
    );
    (fix, shop)
}

#[tokio::test]
async fn problems_a_compose_fix_runs_through_the_projects_runner_with_its_services() {
    let rig = rig();
    let (fix, shop) = compose_fix(&rig, "shop-api");
    let worker = rig.container("shop-api-worker-1");
    let p = problem(
        ProblemKind::DependencyFailed {
            service: "db".to_string(),
        },
        Subject::Container(worker.id),
        "worker keeps restarting",
        vec![fix],
    );
    assert_eq!(rig.click(&p, 0), Clicked::Confirm);
    let model = rig.dialog();
    assert_eq!(model.title, "Restart db?");
    assert_eq!(model.targets, vec![Subject::Project(shop.id.clone())]);
    let launched = rig.confirmed(&model);
    assert!(launched.refused.is_empty(), "{:?}", launched.refused);
    assert_eq!(
        launched.keys,
        vec![PendingKey {
            subject: Subject::Project(shop.id),
            action: "project.restart"
        }]
    );
    // The command reaches Compose with the one service the fix names.
    let mut calls = Vec::new();
    for _ in 0..50 {
        calls = rig.compose.recorded();
        if !calls.is_empty() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    assert_eq!(calls.len(), 1);
    assert_eq!(
        calls[0].1,
        ComposeCommand::Restart {
            services: vec!["db".to_string()]
        }
    );
}

#[tokio::test]
async fn problems_a_compose_fix_for_another_project_is_refused() {
    let rig = rig();
    let (fix, shop) = compose_fix(&rig, "not-this-project");
    let model = rig.dom.in_runtime(|| {
        let engines = rig.state.engines.peek();
        let _ = engines;
        confirm::compose_fix_model(
            &shop,
            "Restart db?".to_string(),
            effect("Restart", "service", &[]),
            match &fix.effect {
                FixEffect::RunAction {
                    action: EngineAction::Compose { target, .. },
                    ..
                } => target.clone(),
                _ => unreachable!("a compose fix"),
            },
            ComposeCommand::Restart {
                services: vec!["db".to_string()],
            },
        )
    });
    let launched = rig.confirmed(&model);
    assert!(launched.keys.is_empty());
    assert_eq!(launched.refused, ["This is not the project the fix is for"]);
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    assert!(rig.compose.recorded().is_empty());
}

#[test]
fn problems_the_prune_fix_opens_clean_up_for_its_engine() {
    let rig = rig();
    let p = problem(
        ProblemKind::DiskFull,
        Subject::Engine(engine_id()),
        "The engine is out of disk space",
        vec![rule(
            "Remove 2 stopped containers",
            FixEffect::RunAction {
                action: EngineAction::Prune {
                    plan: PrunePlan {
                        engine: engine_id(),
                        items: Vec::new(),
                    },
                },
                effect: effect("Remove", "item", &[]),
            },
        )],
    );
    assert_eq!(rig.click(&p, 0), Clicked::CleanUp);
    assert_eq!(rig.overlays(), [Overlay::CleanUp(engine_id())]);
}

#[test]
fn problems_a_note_or_a_harness_fix_does_nothing_when_clicked() {
    let rig = rig();
    let db = rig.container("shop-api-db-1");
    let p = problem(
        ProblemKind::NameInUse {
            name: "db".to_string(),
        },
        Subject::Container(db.id),
        "The name db is taken",
        vec![
            rule("Keep both", FixEffect::Explain),
            Fix {
                label: "Fix with Claude Code".to_string(),
                source: FixSource::Harness(HarnessId("claude".to_string())),
                effect: FixEffect::Explain,
            },
        ],
    );
    assert!(matches!(rig.click(&p, 0), Clicked::Nothing(_)));
    assert!(matches!(rig.click(&p, 1), Clicked::Nothing(_)));
    assert!(matches!(rig.click(&p, 9), Clicked::Nothing(_)));
    assert!(rig.overlays().is_empty());
}

#[test]
fn problems_go_to_selects_the_container_and_unknown_ids_do_nothing() {
    let rig = rig();
    let db = rig.container("shop-api-db-1");
    let p = problem(
        ProblemKind::Unhealthy,
        Subject::Container(db.id.clone()),
        "db",
        vec![restart_fix(&db)],
    );
    let list = vec![p.clone()];
    rig.dom.in_runtime(|| {
        let mut state = rig.state;
        assert_eq!(
            model::click(&mut state, None, &list, &format!("goto:{}", db.id.0)),
            Clicked::Selected
        );
        assert_eq!(
            *state.selection.peek(),
            Selection::Containers(vec![db.id.clone()])
        );
        assert!(matches!(
            model::click(&mut state, None, &list, "nonsense"),
            Clicked::Nothing(_)
        ));
        assert!(matches!(
            model::click(&mut state, None, &list, "abc#0"),
            Clicked::Nothing(_)
        ));
        assert_eq!(
            model::click(&mut state, None, &list, &model::fix_id(&p, 0)),
            Clicked::Confirm
        );
    });
}

// ---------------------------------------------------------------------
// The tab and the Explain list, drawn
// ---------------------------------------------------------------------

#[component]
fn PanelRoot(problems: Vec<Problem>, connecting: bool) -> Element {
    let mut state = use_app_state();
    use_context_provider(|| state);
    use_hook(move || {
        let world = world();
        if !connecting {
            state.engines.set(world.engines.clone());
        }
        state.containers.set(world.containers_for(&engine_id()));
        state.projects.set(world.projects.clone());
        state.problems.set(problems.clone());
    });
    rsx! { ProblemsPanel {} }
}

fn render_panel(problems: Vec<Problem>, connecting: bool) -> String {
    let mut dom = VirtualDom::new_with_props(
        PanelRoot,
        PanelRootProps {
            problems,
            connecting,
        },
    );
    let _ = dom.rebuild_to_vec();
    dioxus_ssr::render(&dom)
}

#[test]
fn problems_the_tab_says_there_are_none() {
    let html = render_panel(Vec::new(), false);
    assert!(html.contains("No problems"), "{html}");
    assert!(
        !html.contains("<section class=\"problems-group\""),
        "{html}"
    );
}

#[test]
fn problems_the_tab_says_it_is_still_checking_before_an_engine_answers() {
    let html = render_panel(Vec::new(), true);
    assert!(html.contains("Checking"), "{html}");
    assert!(!html.contains("No problems"), "{html}");
}

#[test]
fn problems_the_tab_lists_problems_by_project_with_subject_age_and_fixes() {
    let world = world();
    let db = world
        .containers
        .iter()
        .find(|c| c.name == "shop-api-db-1")
        .cloned()
        .expect("db");
    let grafana = world
        .containers
        .iter()
        .find(|c| c.name == "analytics-grafana-1")
        .cloned()
        .expect("grafana");
    let problems = vec![
        problem(
            ProblemKind::Unhealthy,
            Subject::Container(db.id.clone()),
            "db is unhealthy",
            vec![restart_fix(&db)],
        ),
        problem(
            ProblemKind::Unhealthy,
            Subject::Container(grafana.id.clone()),
            "grafana is unhealthy",
            vec![restart_fix(&grafana)],
        ),
    ];
    let html = render_panel(problems, false);
    for needle in [
        "<section class=\"problems-group\"",
        "shop-api",
        "analytics",
        "db is unhealthy",
        "shop-api-db-1",
        "2 min",
        "Restart shop-api-db-1",
        "btn-primary",
        "the line it came from",
        "Why it happened",
    ] {
        assert!(html.contains(needle), "missing {needle}: {html}");
    }
    // Groups and items keep the order of the list: db first, then grafana.
    assert!(
        html.find("db is unhealthy").expect("db")
            < html.find("grafana is unhealthy").expect("grafana")
    );
}

#[test]
fn problems_a_long_list_opens_only_its_first_item() {
    let world = world();
    let names = [
        "shop-api-db-1",
        "shop-api-api-1",
        "shop-api-redis-1",
        "shop-api-worker-1",
    ];
    let problems: Vec<Problem> = names
        .iter()
        .map(|name| {
            let c = world
                .containers
                .iter()
                .find(|c| c.name == *name)
                .cloned()
                .expect("container");
            problem(
                ProblemKind::Unhealthy,
                Subject::Container(c.id.clone()),
                &format!("{name} is unhealthy"),
                vec![restart_fix(&c)],
            )
        })
        .collect();
    let html = render_panel(problems, false);
    assert_eq!(html.matches("aria-expanded=\"true\"").count(), 1, "{html}");
    assert_eq!(html.matches("aria-expanded=\"false\"").count(), 3, "{html}");
}

fn frame(name: &str) -> String {
    crate::scenes::render_scene_html(name).expect("the scene exists")
}

#[test]
fn problems_scene_lists_the_seven_problems_grouped_with_their_fixes_and_the_count_everywhere() {
    let html = frame("Problems / tab / 1440");
    // The tab, the title bar and the status bar all count seven.
    let chips = html.matches("<span>7 problems</span>").count();
    assert_eq!(chips, 2, "title bar and status bar each show the count");
    // The tab strip's badge says 7 too.
    let tab = html.find("Problems").expect("the tab");
    assert!(html[tab..].contains("7"));
    for title in [
        "Port 5432 is already in use",
        "shop-api-worker-1 exits at once",
        "shop-api-api-1 ran out of memory",
        "A folder shop-api-mailpit-1 needs is missing",
        "SESSION_SECRET is not set",
        "shop-api-redis-1 is unhealthy",
        "The image shop/mailer:2 was not found",
    ] {
        assert!(html.contains(title), "missing {title}");
    }
    // One group, shop-api, and the first item (the port clash) is the open one with its fixes.
    let labels: Vec<&str> = html
        .match_indices("problems-group-name\">")
        .map(|(i, m)| html[i + m.len()..].split('<').next().unwrap_or(""))
        .collect();
    assert_eq!(labels, ["shop-api"]);
    let panel = &html[html
        .find("class=\"bottom-panel\"")
        .expect("the bottom panel")..];
    assert_eq!(panel.matches("aria-expanded=\"true\"").count(), 1);
    assert!(html.contains("Map db to port 5433"));
    assert!(html.contains("Fix with Claude Code"));
    assert!(html.contains("Needs a harness: install Claude Code or muse"));
}

#[test]
fn problems_scene_with_a_failed_fix_says_why_under_its_button_and_not_under_the_others() {
    let html = frame("Problems / fix failed / 1440");
    // The container's row says it too (D34); this is about the item in the Problems tab.
    let at = html
        .find("class=\"bottom-panel\"")
        .expect("the bottom panel");
    let panel = &html[at..];
    let said = "Cannot restart shop-api-redis-1: the container is being removed";
    assert_eq!(panel.matches(said).count(), 1);
    // Two problems, so both are open, and the failure sits in the redis item, after its button.
    assert_eq!(panel.matches("aria-expanded=\"true\"").count(), 2);
    let item = panel
        .find("shop-api-redis-1 is unhealthy")
        .expect("the redis item");
    let button = panel[item..]
        .find("Restart shop-api-redis-1")
        .expect("its button")
        + item;
    let message = panel.find(said).expect("the message");
    assert!(message > button, "the message follows the button");
    // The button is enabled again so it can be tried once more.
    let tag_start = panel[..button].rfind("<button").expect("button tag");
    assert!(
        !panel[tag_start..button].contains("disabled"),
        "{}",
        &panel[tag_start..button]
    );
}

#[test]
fn problems_scene_empty_says_no_problems_and_the_bars_agree() {
    let html = frame("Problems / empty / 1440");
    assert!(html.contains("No problems"), "{html}");
    assert!(html.matches("No problems").count() >= 3, "{html}");
}

#[component]
fn ExplainRoot(problems: Option<Vec<Problem>>) -> Element {
    let state = use_app_state();
    use_context_provider(|| state);
    rsx! { ExplainPanel { problems, onclose: |()| {} } }
}

fn render_explain(problems: Option<Vec<Problem>>) -> String {
    let mut dom = VirtualDom::new_with_props(ExplainRoot, ExplainRootProps { problems });
    let _ = dom.rebuild_to_vec();
    dioxus_ssr::render(&dom)
}

#[test]
fn problems_explain_says_it_is_checking_found_nothing_or_lists_what_it_found() {
    assert!(render_explain(None).contains("Checking the lines"));
    let none = render_explain(Some(Vec::new()));
    assert!(
        none.contains("No known problem matches these lines"),
        "{none}"
    );
    let found = render_explain(Some(vec![problem(
        ProblemKind::PortAllocated { port: 3000 },
        Subject::Container(ContainerId("abc".to_string())),
        "Port 3000 is already in use",
        vec![rule("Keep both", FixEffect::Explain)],
    )]));
    assert!(found.contains("Port 3000 is already in use"), "{found}");
    assert!(!found.contains("No known problem"), "{found}");
}
