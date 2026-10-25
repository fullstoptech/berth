//! Volume confirms: delete a volume (typed name).

use super::*;

/// Docker names an anonymous volume with 64 hex digits; a named volume
/// survives `docker rm -v`.
#[must_use]
pub fn is_anonymous_volume(name: &str) -> bool {
    name.len() == 64 && name.bytes().all(|b| b.is_ascii_hexdigit())
}

/// What deleting a volume does (P4-T4): everything stored in it goes, and
/// the name must be typed (D33). The size is the footer ("Removes 1.2 GB"),
/// kept off the button label.
#[must_use]
pub fn delete_volume_effect(name: &str, size: Option<bytesize::ByteSize>) -> EffectSummary {
    let mut effect = summary(
        "Delete",
        "volume",
        "volumes",
        1,
        vec![line(
            EffectKind::Goes,
            format!("{name}, with every file stored in it"),
        )],
    );
    effect.bytes = size;
    effect.typed_name = Some(name.to_string());
    effect
}

/// The delete dialog for a volume: typed name, and `EngineAction::Remove`
/// of the volume on the engine that has it (D25a). `None` when the volume
/// is gone or in use.
#[must_use]
pub fn delete_volume_model(state: &AppState, name: &str) -> Option<ConfirmModel> {
    let volumes = state.volumes.peek();
    let volume = volumes.iter().find(|v| v.name == name)?;
    if !volume.used_by.is_empty() {
        return None;
    }
    let engine = volume.engine.clone();
    Some(ConfirmModel {
        title: format!("Delete volume {name}?"),
        effect: delete_volume_effect(name, volume.size),
        action: EngineAction::Remove {
            engine,
            target: ResourceRef::Volume(name.to_string()),
            force: false,
        },
        checkbox: None,
        targets: vec![Subject::Volume(name.to_string())],
    })
}

/// Open the delete dialog for one volume on the overlay stack. False when
/// nothing can be deleted (see [`delete_volume_model`]).
pub fn open_delete_volume(state: &mut AppState, name: &str) -> bool {
    let Some(model) = delete_volume_model(state, name) else {
        return false;
    };
    state
        .overlays
        .write()
        .push(crate::state::Overlay::Confirm(model));
    true
}
