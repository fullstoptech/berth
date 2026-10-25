//! Image confirms: delete an image.

use super::*;

/// What deleting one image does: the image and every tag of it go. The
/// containers that used it would have blocked the delete, so none is named.
/// The size is the image's own.
#[must_use]
pub fn delete_image_effect(tags: &[String], size: bytesize::ByteSize) -> EffectSummary {
    let name = tags.first().map_or("This untagged image", String::as_str);
    let mut lines = vec![line(
        EffectKind::Goes,
        format!(
            "Image {name} ({}) is deleted from this engine.",
            berth_core::fmt::bytes(size)
        ),
    )];
    if tags.len() > 1 {
        lines.push(line(
            EffectKind::Goes,
            format!("All {} of its tags go with it.", tags.len()),
        ));
    }
    lines.push(line(
        EffectKind::Stays,
        "Pulling it again brings it back.".to_string(),
    ));
    let mut effect = summary("Delete", "image", "images", 1, lines);
    effect.bytes = Some(size);
    effect
}

/// The dialog of Delete on one image (D25a: `EngineAction::Remove`). An
/// image with several tags needs `force`, which here means "all its names".
#[must_use]
pub fn delete_image_model(engine: &EngineId, image: &berth_core::Image) -> ConfirmModel {
    let name = image.repo_tags.first().cloned().unwrap_or_else(|| {
        image
            .id
            .trim_start_matches("sha256:")
            .chars()
            .take(12)
            .collect()
    });
    ConfirmModel {
        title: format!("Delete image {name}?"),
        effect: delete_image_effect(&image.repo_tags, image.size),
        action: EngineAction::Remove {
            engine: engine.clone(),
            target: berth_core::ResourceRef::Image(image.id.clone()),
            force: image.repo_tags.len() > 1,
        },
        checkbox: None,
        targets: vec![Subject::Image(image.id.clone())],
    }
}
