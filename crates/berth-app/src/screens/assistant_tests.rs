//! SSR checks of the Assistant scenes (P7-T5): the frame scene word
//! for word where the frame fixes the words, and one render per state.

use super::*;
use crate::scenes::render_scene_html;

fn scene(frame: &str) -> String {
    render_scene_html(frame).unwrap_or_else(|| panic!("scene {frame} does not render"))
}

#[test]
fn assistant_frame_scene_matches_the_frames_words() {
    let html = scene("Project / assistant / 1440");
    for wanted in [
        "From Logs \u{b7} Ask about 2 lines",
        "Why does the worker keep restarting? Fix it.",
        "Read 200 lines of shop-api-worker-1 logs",
        "140 ms",
        "Inspected shop-api-db-1",
        "38 ms",
        "Read compose.yaml",
        "12 ms",
        "The worker restarts because Postgres never started",
        "shop-api-worker-1 \u{b7} 2 lines",
        "Port 5432 is already in use",
        "compose.yaml",
        "Ask about shop-api\u{2026}",
        "Watch and fix",
        ".env values stay on this Mac. Claude Code sees key names and ports only.",
        "Step 1 of 2",
        "Move db to 5433 and update DATABASE_URL",
        "2 files \u{b7} +10 \u{2212}3",
        "Review changes",
        "Change",
        "Then: recreate db and worker",
        "docker compose up -d db worker",
        "Volume shop-api_pgdata is kept.",
        "Approve",
        "Deny",
        "Claude Code 2.4.1",
        "Assistant",
    ] {
        assert!(
            html.contains(wanted),
            "missing {wanted:?} in the frame scene"
        );
    }
    // The Then: step waits for the write: its Approve is disabled with
    // the reason attached (the frame greys both buttons; our Deny stays
    // live so the step can still be refused).
    assert!(
        html.contains("disabled"),
        "the Then: approval is not disabled"
    );
    assert!(
        html.contains("Review the proposed change first"),
        "the blocked reason is missing"
    );
    // The risk tokens of the frame.
    assert!(html.matches(">Read<").count() >= 3, "read badges missing");
    assert!(!html.contains("Thinking\u{2026}"), "frame turn is done");
}

#[test]
fn assistant_state_scenes_render_their_state() {
    let thinking = scene("Assistant / thinking");
    assert!(thinking.contains("Thinking\u{2026}"), "{thinking}");
    assert!(thinking.contains("running\u{2026}"), "running row missing");
    assert!(thinking.contains("Stop"), "stop button missing");

    let waiting = scene("Assistant / waiting for approval");
    assert!(waiting.contains("waiting for approval"));
    assert!(waiting.contains("docker restart shop-api-worker-1"));
    assert!(waiting.contains("Restart container shop-api-worker-1"));
    assert!(waiting.contains("Approve"));

    let errored = scene("Assistant / error");
    assert!(errored.contains("The assistant stopped."));
    assert!(errored.contains("Retry"));
    assert!(errored.contains("stream closed before the turn finished"));

    let no_harness = scene("Assistant / no harness");
    assert!(no_harness.contains("No assistant installed"));
    assert!(no_harness.contains("npm install -g @anthropic-ai/claude-code"));
    assert!(no_harness.contains("muse --version"));
    assert!(no_harness.contains("keep working without an assistant"));
    assert!(no_harness.contains("No assistant found"));

    let watch = scene("Assistant / watch");
    assert!(watch.contains("Watching shop-api"));
    assert!(watch.contains("Nothing is applied without your click."));

    let empty = scene("Assistant / empty");
    assert!(empty.contains("Ask about shop-api \u{2014}"));

    let no_project = scene("Assistant / no project");
    assert!(no_project.contains("No project selected"));

    let maximized = scene("Assistant / maximized");
    assert!(maximized.contains("bottom-panel maximized"));
    assert!(maximized.contains("Restore the panel size"));
}

#[test]
fn assistant_helpers_name_chips_steps_and_hints() {
    let problems = vec![berth_core::Problem {
        id: berth_core::ProblemId("p1".to_string()),
        kind: berth_core::ProblemKind::PortAllocated { port: 5432 },
        severity: berth_core::Severity::Error,
        subject: berth_core::Subject::Container(berth_core::ContainerId("c1".to_string())),
        title: "Port 5432 is already in use".to_string(),
        explanation: String::new(),
        evidence: Vec::new(),
        fixes: Vec::new(),
        first_seen: chrono::DateTime::from_timestamp(0, 0).expect("ts"),
    }];
    let (icon, label) = context_chip_model(
        &ContextItem::Problem(berth_core::ProblemId("p1".to_string())),
        &problems,
        &[],
    );
    assert_eq!(icon, "alert-triangle");
    assert_eq!(label, "Port 5432 is already in use");
    let (icon, label) = context_chip_model(
        &ContextItem::Logs {
            container: berth_core::ContainerId("c1".to_string()),
            name: "shop-api-worker-1".to_string(),
            lines: vec!["a".to_string(), "b".to_string()],
        },
        &[],
        &[],
    );
    assert_eq!(
        (icon, label.as_str()),
        ("logs", "shop-api-worker-1 \u{b7} 2 lines")
    );

    assert_eq!(
        install_hint("claude-code").1,
        "npm install -g @anthropic-ai/claude-code"
    );
    assert_eq!(install_hint("muse").1, "muse --version");
    assert_eq!(
        footnote(None),
        ".env values stay on this Mac. the assistant sees key names and ports only."
    );
    assert_eq!(
        crate::ui::proposal_totals(&[
            (PathBuf::from("compose.yaml"), 9, 2),
            (PathBuf::from(".env"), 1, 1),
        ]),
        "2 files \u{b7} +10 \u{2212}3"
    );
}
