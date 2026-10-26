use super::*;
use berth_core::HarnessId;

fn claude() -> Harness {
    Harness {
        id: HarnessId("claude-code".to_string()),
        name: "Claude Code".to_string(),
        binary: "/bin/true".into(),
        version: None,
        transport: HarnessTransport::ClaudeStreamJson,
        available: true,
    }
}

fn values(options: &[SelectOption]) -> Vec<&str> {
    options.iter().map(|o| o.value.as_str()).collect()
}

#[test]
fn harness_picker_model_options_offer_default_curated_and_custom() {
    let options = model_options(&claude(), None);
    assert_eq!(
        values(&options),
        ["", "opus", "sonnet", "haiku", CUSTOM_MODEL]
    );
    assert_eq!(options[0].label, "Default model");
    assert_eq!(options[1].label, "Opus");
}

#[test]
fn harness_picker_model_options_keep_a_custom_saved_pick_selectable() {
    let options = model_options(&claude(), Some("claude-custom-1"));
    assert_eq!(
        values(&options),
        [
            "",
            "opus",
            "sonnet",
            "haiku",
            "claude-custom-1",
            CUSTOM_MODEL
        ]
    );
    // A curated pick is not duplicated.
    let options = model_options(&claude(), Some("opus"));
    assert_eq!(
        values(&options),
        ["", "opus", "sonnet", "haiku", CUSTOM_MODEL]
    );
}

#[test]
fn harness_picker_model_options_for_muse_list_only_the_verified_id() {
    let muse = Harness {
        id: HarnessId("muse".to_string()),
        name: "muse".to_string(),
        binary: "/bin/true".into(),
        version: None,
        transport: HarnessTransport::MuseMsp,
        available: true,
    };
    assert_eq!(
        values(&model_options(&muse, None)),
        ["", "muse-spark-1.2", CUSTOM_MODEL]
    );
}
