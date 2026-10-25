//! Confirms for the volume browser's writes and the helper container they
//! need: upload, restore, start the helper.

use super::*;

/// What uploading one file into a container folder does (Files tab,
/// P3-T8). `replaces` says a file of that name is already there; the
/// upload would overwrite it, which is why the dialog asks at all. The
/// size is in the text because `bytes` means "removes" in the footer.
#[must_use]
pub fn upload_effect(
    dir: &str,
    name: &str,
    size: bytesize::ByteSize,
    replaces: bool,
) -> EffectSummary {
    let mut lines = vec![line(
        EffectKind::Writes,
        format!("{name} ({}) to {dir}", berth_core::fmt::bytes(size)),
    )];
    if replaces {
        lines.push(line(
            EffectKind::Changes,
            format!("The {name} already in {dir} is overwritten"),
        ));
    }
    summary("Upload", "file", "files", 1, lines)
}

/// The upload dialog's title: names the file and where it goes.
#[must_use]
pub fn upload_title(dir: &str, name: &str) -> String {
    format!("Upload {name} to {dir}?")
}

/// What restoring a backup into a volume does (P4-T4): the files of the
/// backup are written over the volume's, and the name must be typed. The
/// volume is not emptied first: files the backup does not have stay.
#[must_use]
pub fn restore_effect(
    volume: &str,
    file: &str,
    files: u64,
    size: bytesize::ByteSize,
    users: &[String],
) -> EffectSummary {
    let count = usize::try_from(files).unwrap_or(usize::MAX);
    let mut lines = vec![
        line(
            EffectKind::Writes,
            format!(
                "{} {} ({}) from {file} into {volume}",
                crate::ui::file_browser::thousands(count),
                if count == 1 { "file" } else { "files" },
                berth_core::fmt::bytes(size)
            ),
        ),
        line(
            EffectKind::Changes,
            format!("Files in {volume} with the same names are overwritten"),
        ),
        line(
            EffectKind::Stays,
            format!("Other files in {volume} are not touched"),
        ),
    ];
    if !users.is_empty() {
        lines.push(line(
            EffectKind::Changes,
            format!(
                "{} use {volume} and see the files change; stop them first for a clean restore",
                users.join(", ")
            ),
        ));
    }
    let mut effect = summary("Restore", "backup", "backups", 1, lines);
    effect.typed_name = Some(volume.to_string());
    effect
}

/// The restore dialog's title.
#[must_use]
pub fn restore_title(volume: &str) -> String {
    format!("Restore {volume} from a backup?")
}

/// The first-use dialog of the volume helper image (D51): what it is, what
/// it weighs, and that it stays.
#[must_use]
pub fn helper_effect() -> EffectSummary {
    summary(
        "Download",
        "image",
        "images",
        1,
        vec![
            line(
                EffectKind::Writes,
                format!(
                    "{} (about {}) is downloaded once and stays on this engine",
                    crate::services::volumes::HELPER_IMAGE,
                    berth_core::fmt::bytes(crate::services::volumes::HELPER_SIZE)
                ),
            ),
            line(
                EffectKind::Changes,
                "A short-lived helper container mounts the volume to read it, then is removed"
                    .to_string(),
            ),
        ],
    )
}

/// The helper dialog's title.
#[must_use]
pub fn helper_title() -> String {
    format!(
        "Berth uses a small helper image {} to read volumes",
        crate::services::volumes::HELPER_IMAGE
    )
}
