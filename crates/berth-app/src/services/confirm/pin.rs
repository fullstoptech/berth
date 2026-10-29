//! Remove from Berth (P10-T3): unpinning a folder nothing runs from. The dialog's rows, its
//! button and the result message all read this one `EffectSummary`. It states what the action
//! does and does not do: Berth forgets the folder; the folder, its files and every Docker
//! resource stay as they are.

use berth_core::EffectKind;

use super::*;

/// What "Remove from Berth…" does to the pinned folder `name` at `folder` (the `~/…` path).
#[must_use]
pub fn remove_pin_effect(name: &str, folder: &str) -> EffectSummary {
    summary(
        "Unpin",
        "folder",
        "folders",
        1,
        vec![
            line(
                EffectKind::Goes,
                format!("{name} from Berth's Projects list (the pin of {folder})"),
            ),
            line(
                EffectKind::Stays,
                format!("The folder and every file in {folder}"),
            ),
            line(
                EffectKind::Stays,
                "Docker containers, images and volumes: nothing was created for this project",
            ),
        ],
    )
}

/// The dialog's title.
#[must_use]
pub fn remove_pin_title(name: &str) -> String {
    format!("Remove {name} from Berth?")
}
