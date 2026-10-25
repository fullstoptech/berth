//! Compile test: every public fn of PLAN section 6.7 is callable with the signature as written.
//! It asserts nothing about results, so it stays valid while P5-T1, P5-T2 and P5-T7 fill the bodies.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use berth_core::{
    Arch, Confidence, Detection, Engine, EngineId, EngineKind, EngineState, FileChange,
    GenerationPlan, OwnerId, PlanSource, ResourceCounts, ServiceKind, Template, TemplateKind,
};
use berth_rules::problems::{evaluate, ProblemInput};
use berth_rules::recipes::RecipeOptions;
use berth_rules::{
    compose_edit, detect, diff, ports, recipes, run_to_compose, templates, write, RulesError,
};

fn nowhere() -> PathBuf {
    PathBuf::from("/nonexistent/berth-rules-signatures")
}

fn detection() -> Detection {
    Detection {
        folder: nowhere(),
        found: Vec::new(),
        stack: Vec::new(),
        services_needed: Vec::new(),
        confidence: Confidence::Low,
        git_branch: None,
    }
}

fn template() -> Template {
    Template {
        id: "x".to_string(),
        name: "X".to_string(),
        kind: TemplateKind::BuiltIn,
        service: String::new(),
        env: Vec::new(),
        inputs: Vec::new(),
        category: "Test".to_string(),
        image: "x:1".to_string(),
        description: String::new(),
    }
}

fn plan() -> GenerationPlan {
    GenerationPlan {
        root: nowhere(),
        files: Vec::new(),
        source: PlanSource::Editor,
        notes: Vec::new(),
        after_write: None,
    }
}

#[test]
fn every_public_fn_is_callable_as_written() {
    // detect
    let _: Result<Detection, RulesError> = detect::detect(&nowhere());
    let _: detect::FolderMarkers = detect::markers(&nowhere());

    // recipes
    let o: RecipeOptions = recipes::options_for(&detection(), &[3000]);
    let _: Result<GenerationPlan, RulesError> = recipes::plan(&detection(), &o);
    let _: Result<GenerationPlan, RulesError> = recipes::compose_for_dockerfile(&detection(), &o);
    let _ = RecipeOptions {
        ports: BTreeMap::new(),
        node_version: None,
        services: vec![ServiceKind::Postgres],
    };

    // templates
    let inputs: BTreeMap<String, String> = BTreeMap::new();
    let _: Vec<Template> = templates::built_in();
    let _: (String, Vec<(String, String)>) = templates::render(&template(), &inputs);
    let _: String = templates::generate_password();
    let _: Result<GenerationPlan, RulesError> = templates::add_service_plan(
        &nowhere(),
        Path::new("docker-compose.yml"),
        "services: {}\n",
        None,
        &template(),
        "cache",
        &inputs,
    );

    // compose_edit
    let text = "services:\n  web:\n    image: x\n    ports:\n      - \"3000:3000\"\n";
    let _: Result<String, RulesError> = compose_edit::add_service(text, "db", "image: postgres\n");
    let _: Result<String, RulesError> = compose_edit::set_host_port(text, "web", 3000, 3001);
    let _: String = compose_edit::add_env_lines(None, &[("A".to_string(), "b".to_string())]);
    let _: Result<Vec<String>, RulesError> = compose_edit::service_names(text);
    let _: Vec<String> = compose_edit::referenced_vars("${A} $B");
    let _: Result<String, RulesError> = compose_edit::add_named_volume(text, "data");
    let _: Result<String, RulesError> =
        compose_edit::set_service_key(text, "web", "mem_limit", "512m");

    // diff
    let _: berth_core::FileDiff =
        diff::file_diff(Path::new("compose.yaml"), None, Some("services: {}\n"));
    let _: String = diff::sha256("x");
    let _ = FileChange::Create;

    // write (an empty plan under a path that does not exist)
    let _: Result<write::WriteReport, RulesError> = write::apply(&plan());

    // ports
    let _: u16 = ports::suggest(3000, &[3000]);

    // run_to_compose
    let _: Result<String, RulesError> =
        run_to_compose::convert("docker run -d --name db postgres:17");

    // problems
    let input = ProblemInput {
        now: chrono::Utc::now(),
        engine: Engine {
            id: EngineId("colima:default".to_string()),
            name: "default".to_string(),
            kind: EngineKind::Colima {
                profile: "default".to_string(),
            },
            state: EngineState::Running,
            docker_host: None,
            docker_version: None,
            resources: None,
            counts: ResourceCounts::default(),
            owner: OwnerId::Local,
        },
        host_arch: Arch::Aarch64,
        containers: Vec::new(),
        projects: Vec::new(),
        disk: None,
        taken_ports: Vec::new(),
    };
    let _: Vec<berth_core::Problem> = evaluate(&input);

    // RulesError variants
    let _ = [
        RulesError::OutsideRoot(nowhere()),
        RulesError::Conflict(nowhere()),
        RulesError::Yaml {
            line: 1,
            message: String::new(),
        },
        RulesError::Read(nowhere(), String::new()),
        RulesError::from(std::io::Error::other("x")),
    ];
}
