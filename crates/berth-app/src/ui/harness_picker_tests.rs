use super::*;

fn choice(id: &str) -> berth_assistant::ModelChoice {
    berth_assistant::ModelChoice {
        id: id.to_string(),
        label: id.to_string(),
        is_default: false,
    }
}

fn values(options: &[SelectOption]) -> Vec<&str> {
    options.iter().map(|o| o.value.as_str()).collect()
}

#[test]
fn harness_picker_model_options_offer_default_catalog_and_custom() {
    let catalog = [choice("claude-opus-5-5"), choice("claude-sonnet-5-5")];
    let options = model_options(&catalog, None);
    assert_eq!(
        values(&options),
        ["", "claude-opus-5-5", "claude-sonnet-5-5", CUSTOM_MODEL]
    );
    assert_eq!(options[0].label, "Default model");
    // The original id is the label (AD40).
    assert_eq!(options[1].label, "claude-opus-5-5");
}

#[test]
fn harness_picker_model_options_keep_a_saved_pick_the_catalog_dropped() {
    let catalog = [choice("claude-opus-5-5")];
    let options = model_options(&catalog, Some("claude-older-1"));
    assert_eq!(
        values(&options),
        ["", "claude-opus-5-5", "claude-older-1", CUSTOM_MODEL]
    );
    // A catalog pick is not duplicated.
    let options = model_options(&catalog, Some("claude-opus-5-5"));
    assert_eq!(values(&options), ["", "claude-opus-5-5", CUSTOM_MODEL]);
}

#[test]
fn harness_picker_model_options_without_a_catalog_still_work() {
    assert_eq!(values(&model_options(&[], None)), ["", CUSTOM_MODEL]);
    assert_eq!(
        values(&model_options(&[], Some("typed-model"))),
        ["", "typed-model", CUSTOM_MODEL]
    );
}
