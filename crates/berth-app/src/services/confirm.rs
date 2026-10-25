//! Confirm models (D33): the one place that builds the `EffectSummary` of
//! a container or engine action that asks first. The dialog's button, its
//! text and the result message all render from that one value
//! ([`button_label`], the lines, [`result_label`]); `search.rs` (palette),
//! the row, the bulk bar and ⌫ all come here.
//!
//! [`run`] is the other half: it sends a confirmed model to the actions
//! service and returns the pending keys, so the caller can watch for the
//! outcome ([`watch_result`]) and say it from the same summary.

use berth_core::{
    ColimaOp, Container, ContainerAction, ContainerId, ContainerState, EffectKind, EffectLine,
    EffectSummary, EngineAction, EngineId, Network, ResourceRef, Subject, Volume,
};
use dioxus::prelude::{ReadableExt, WritableExt};

use crate::services::actions::{container_action_id, ActionsService, ContainerTarget};
use crate::state::{AppState, ConfirmModel, PendingKey, PendingState, Toast, ToastTone};

/// "1 container" or "3 containers".
#[must_use]
pub fn plural(n: usize, one: &str, many: &str) -> String {
    if n == 1 {
        format!("{n} {one}")
    } else {
        format!("{n} {many}")
    }
}

fn line(kind: EffectKind, text: impl Into<String>) -> EffectLine {
    EffectLine {
        kind,
        text: text.into(),
    }
}

fn summary(
    verb: &str,
    noun_one: &str,
    noun_many: &str,
    count: usize,
    lines: Vec<EffectLine>,
) -> EffectSummary {
    EffectSummary {
        verb: verb.to_string(),
        noun_one: noun_one.to_string(),
        noun_many: noun_many.to_string(),
        count: u32::try_from(count).unwrap_or(u32::MAX),
        bytes: None,
        lines,
        typed_name: None,
    }
}

fn is_active(state: ContainerState) -> bool {
    matches!(
        state,
        ContainerState::Running | ContainerState::Restarting | ContainerState::Paused
    )
}

/// Docker names an anonymous volume with 64 hex digits; a named volume
/// survives `docker rm -v`.
#[must_use]
pub fn is_anonymous_volume(name: &str) -> bool {
    name.len() == 64 && name.bytes().all(|b| b.is_ascii_hexdigit())
}

/// The named volumes these containers mount, once each, in mount order.
fn kept_volumes<'a>(containers: &[&'a Container]) -> Vec<&'a str> {
    let mut names: Vec<&str> = Vec::new();
    for container in containers {
        for mount in &container.mounts {
            if let Some(name) = mount.volume_name.as_deref() {
                if !is_anonymous_volume(name) && !names.contains(&name) {
                    names.push(name);
                }
            }
        }
    }
    names
}

/// How many kept volumes get a line each before they are summed up.
const KEPT_LINES: usize = 3;

fn kept_lines(containers: &[&Container], volumes: &[Volume]) -> Vec<EffectLine> {
    let names = kept_volumes(containers);
    let size_of = |name: &str| {
        volumes
            .iter()
            .find(|v| v.name == name)
            .and_then(|v| v.size)
            .map(berth_core::fmt::bytes)
    };
    if names.len() <= KEPT_LINES {
        return names
            .into_iter()
            .map(|name| {
                let text = match size_of(name) {
                    Some(size) => format!("Volume {name} ({size}) is kept."),
                    None => format!("Volume {name} is kept."),
                };
                line(EffectKind::Stays, text)
            })
            .collect();
    }
    let total = names
        .iter()
        .filter_map(|n| volumes.iter().find(|v| v.name == *n).and_then(|v| v.size))
        .reduce(|a, b| a + b);
    let sum = total.map_or_else(String::new, |t| format!(" ({})", berth_core::fmt::bytes(t)));
    vec![line(
        EffectKind::Stays,
        format!(
            "{} kept{sum}.",
            plural(names.len(), "volume is", "volumes are")
        ),
    )]
}

/// What deleting these containers does. A running container is stopped
/// first, which is its own `Changes` line (it also turns the button into
/// "Stop and delete").
#[must_use]
pub fn delete_effect(containers: &[&Container], volumes: &[Volume]) -> EffectSummary {
    let one = containers.len() == 1;
    let mut lines = vec![line(
        EffectKind::Goes,
        if one {
            "Its writable layer is deleted."
        } else {
            "Their writable layers are deleted."
        },
    )];
    lines.extend(kept_lines(containers, volumes));
    let running: Vec<&&Container> = containers.iter().filter(|c| is_active(c.state)).collect();
    match running.as_slice() {
        [] => {}
        [only] => lines.push(line(
            EffectKind::Changes,
            format!("{} is stopped first.", only.name),
        )),
        many => lines.push(line(
            EffectKind::Changes,
            format!(
                "{} are stopped first.",
                plural(many.len(), "running container", "running containers")
            ),
        )),
    }
    summary("Delete", "container", "containers", containers.len(), lines)
}

/// What killing a container does.
#[must_use]
pub fn kill_effect(container: &Container) -> EffectSummary {
    summary(
        "Kill",
        "container",
        "containers",
        1,
        vec![line(
            EffectKind::Changes,
            format!("{} stops at once without a clean shutdown", container.name),
        )],
    )
}

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

/// What recreating a container does (P3-T7, D18): the loss warning of the
/// edit sheet, the dialog of "Recreate with latest image", the activity
/// entry and the result all read this one summary. Settings the spec
/// cannot carry never get here: Recreate is disabled for them (D16).
#[must_use]
pub fn recreate_effect(
    container: &Container,
    volumes: &[Volume],
    latest_image: bool,
) -> EffectSummary {
    let mut lines = Vec::new();
    if latest_image {
        lines.push(line(
            EffectKind::Changes,
            format!("{} is pulled again first.", container.image.reference),
        ));
    }
    if is_active(container.state) {
        lines.push(line(
            EffectKind::Changes,
            format!(
                "{} is stopped, then the new container is started.",
                container.name
            ),
        ));
    }
    lines.push(line(
        EffectKind::Goes,
        "Files written inside the container, outside volumes, are lost.",
    ));
    lines.extend(kept_lines(&[container], volumes));
    summary("Recreate", "container", "containers", 1, lines)
}

/// The result of one recreate, in words ("Recreated 1 container").
#[must_use]
pub fn recreate_result() -> String {
    result_label(
        &summary("Recreate", "container", "containers", 1, Vec::new()),
        1,
    )
}

/// What stopping or restarting a Colima engine does to its containers.
#[must_use]
pub fn engine_effect(stop: bool, running: usize) -> EffectSummary {
    let (verb, when) = if stop {
        ("Stop", "is stopped")
    } else {
        ("Restart", "restarts")
    };
    summary(
        verb,
        "engine",
        "engines",
        1,
        vec![line(
            EffectKind::Changes,
            format!(
                "{} stop while the engine {when}",
                plural(running, "running container", "running containers")
            ),
        )],
    )
}

/// What deleting a network does. Only an unused network can be deleted
/// (`disabled_reason`), so nothing running is touched; a Compose project's
/// network comes back with its next start.
#[must_use]
pub fn network_delete_effect(network: &Network, project: Option<&str>) -> EffectSummary {
    let what = match &network.subnet {
        Some(subnet) => format!("Network {} ({subnet}) is removed.", network.name),
        None => format!("Network {} is removed.", network.name),
    };
    let mut lines = vec![
        line(EffectKind::Goes, what),
        line(
            EffectKind::Stays,
            "No container is connected, so nothing running is affected.",
        ),
    ];
    if let Some(project) = project {
        lines.push(line(
            EffectKind::Changes,
            format!("Project {project} creates it again the next time it starts."),
        ));
    }
    ResourceRef::Network(network.id.clone()).delete_effect(lines)
}

/// The confirm button's text, from the summary alone: "Delete container",
/// "Delete 4 containers", "Stop and delete" when something runs.
#[must_use]
pub fn button_label(effect: &EffectSummary) -> String {
    let stops_first =
        effect.verb == "Delete" && effect.lines.iter().any(|l| l.kind == EffectKind::Changes);
    match (stops_first, effect.count) {
        (true, 1) => "Stop and delete".to_string(),
        (true, n) => format!("Stop and delete {n} {}", effect.noun_many),
        (false, 1) => format!("{} {}", effect.verb, effect.noun_one),
        (false, _) => effect.button_label(),
    }
}

/// The result message when `done` of the summary's items went through:
/// the same count and nouns as the button, in the past tense.
#[must_use]
pub fn result_label(effect: &EffectSummary, done: usize) -> String {
    let mut done_effect = effect.clone();
    done_effect.count = u32::try_from(done).unwrap_or(u32::MAX);
    done_effect.bytes = None;
    done_effect.result_label()
}

// ---------------------------------------------------------------------
// Models
// ---------------------------------------------------------------------

fn find_containers<'a>(all: &'a [Container], ids: &[ContainerId]) -> Vec<&'a Container> {
    ids.iter()
        .filter_map(|id| all.iter().find(|c| c.id == *id))
        .collect()
}

/// The delete dialog for one or more containers. `None` when none of the
/// ids is known any more.
#[must_use]
pub fn delete_model(state: &AppState, ids: &[ContainerId]) -> Option<ConfirmModel> {
    let containers = state.containers.peek();
    let volumes = state.volumes.peek();
    let found = find_containers(&containers, ids);
    let first = found.first()?;
    let effect = delete_effect(&found, &volumes);
    let checkbox = Some((delete_checkbox_label(&found), false));
    let targets: Vec<Subject> = found
        .iter()
        .map(|c| Subject::Container(c.id.clone()))
        .collect();
    if let [only] = found.as_slice() {
        return Some(ConfirmModel {
            title: format!("Delete container {}?", only.name),
            effect,
            action: EngineAction::Container {
                engine: only.engine.clone(),
                id: only.id.clone(),
                action: ContainerAction::Remove {
                    force: is_active(only.state),
                    volumes: false,
                },
            },
            checkbox,
            targets,
        });
    }
    // PLAN §6 has no multi-container `EngineAction`, so a bulk model's
    // action names the first container and `run` walks `targets`.
    Some(ConfirmModel {
        title: format!("Delete {}?", plural(found.len(), "container", "containers")),
        effect,
        action: EngineAction::Container {
            engine: first.engine.clone(),
            id: first.id.clone(),
            action: ContainerAction::Remove {
                force: is_active(first.state),
                volumes: false,
            },
        },
        checkbox,
        targets,
    })
}

/// The delete dialog's checkbox (frame "Containers / confirm delete"):
/// "Also delete anonymous volumes", with the named volumes that stay put
/// listed so the box cannot be read as "all volumes".
#[must_use]
pub fn delete_checkbox_label(containers: &[&Container]) -> String {
    let kept = kept_volumes(containers);
    match kept.as_slice() {
        [] => "Also delete anonymous volumes".to_string(),
        names if names.len() <= KEPT_LINES => {
            format!("Also delete anonymous volumes (not {})", names.join(", "))
        }
        names => format!(
            "Also delete anonymous volumes (not {} named volumes)",
            names.len()
        ),
    }
}

/// The kill dialog for one container.
#[must_use]
pub fn kill_model(state: &AppState, id: &ContainerId) -> Option<ConfirmModel> {
    let containers = state.containers.peek();
    let container = containers.iter().find(|c| c.id == *id)?;
    Some(ConfirmModel {
        title: format!("Kill {}?", container.name),
        effect: kill_effect(container),
        action: EngineAction::Container {
            engine: container.engine.clone(),
            id: container.id.clone(),
            action: ContainerAction::Kill { signal: None },
        },
        checkbox: None,
        targets: vec![Subject::Container(container.id.clone())],
    })
}

/// The dialog of "Recreate with latest image": one `EngineAction::Recreate`
/// with `pull: true` (D18). The spec is read when the dialog opens, like
/// the targets.
#[must_use]
pub fn recreate_latest_model(
    state: &AppState,
    id: &ContainerId,
    spec: berth_core::ContainerSpec,
) -> Option<ConfirmModel> {
    let containers = state.containers.peek();
    let volumes = state.volumes.peek();
    let container = containers.iter().find(|c| c.id == *id)?;
    Some(ConfirmModel {
        title: format!("Recreate {} with the latest image?", container.name),
        effect: recreate_effect(container, &volumes, true),
        action: EngineAction::Recreate {
            engine: container.engine.clone(),
            id: container.id.clone(),
            spec,
            pull: true,
        },
        checkbox: None,
        targets: vec![Subject::Container(container.id.clone())],
    })
}

/// The delete dialog for one network. `None` when the network is gone or
/// its engine cannot be told (the screen's Delete says why in that case).
#[must_use]
pub fn network_delete_model(state: &AppState, id: &str) -> Option<ConfirmModel> {
    let networks = state.networks.peek();
    // A built-in network has no delete dialog: it has no Delete.
    let network = networks.iter().find(|n| n.id == id && !n.built_in)?;
    let containers = state.containers.peek();
    let engine = crate::services::networks::engine_of(
        network,
        &containers,
        &state.engines.peek(),
        &state.engine_scope.peek(),
    )?;
    let project =
        crate::services::networks::project_of(network, &containers, &state.projects.peek());
    Some(ConfirmModel {
        title: format!("Delete network {}?", network.name),
        effect: network_delete_effect(network, project.as_deref()),
        action: EngineAction::Remove {
            engine,
            target: ResourceRef::Network(network.id.clone()),
            force: false,
        },
        checkbox: None,
        targets: vec![Subject::Network(network.id.clone())],
    })
}

/// Open the delete dialog for a network. Returns false when there is none
/// to delete.
pub fn open_delete_network(state: &mut AppState, id: &str) -> bool {
    let Some(model) = network_delete_model(state, id) else {
        return false;
    };
    state
        .overlays
        .write()
        .push(crate::state::Overlay::Confirm(model));
    true
}

/// The stop or restart dialog for a Colima engine.
#[must_use]
pub fn engine_model(
    engine: &EngineId,
    engine_name: &str,
    profile: String,
    op: ColimaOp,
    running: usize,
) -> ConfirmModel {
    let stop = matches!(op, ColimaOp::Stop);
    let verb = if stop { "Stop" } else { "Restart" };
    ConfirmModel {
        title: format!("{verb} engine {engine_name}?"),
        effect: engine_effect(stop, running),
        action: EngineAction::Colima { profile, op },
        checkbox: None,
        targets: vec![Subject::Engine(engine.clone())],
    }
}

/// The delete dialog for a volume: typed name, and `EngineAction::Remove`
/// of the volume on the engine that has it (D25a). `None` when the volume
/// is gone, in use, or its engine cannot be told.
#[must_use]
pub fn delete_volume_model(state: &AppState, name: &str) -> Option<ConfirmModel> {
    let volumes = state.volumes.peek();
    let containers = state.containers.peek();
    let engines = state.engines.peek();
    let volume = volumes.iter().find(|v| v.name == name)?;
    if !volume.used_by.is_empty() {
        return None;
    }
    let engine = crate::services::volumes::owner_engine(volume, &containers, &engines)?;
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

/// Open the delete dialog for these containers on the overlay stack.
/// Returns false when nothing is left to delete.
pub fn open_delete(state: &mut AppState, ids: &[ContainerId]) -> bool {
    let Some(model) = delete_model(state, ids) else {
        return false;
    };
    state
        .overlays
        .write()
        .push(crate::state::Overlay::Confirm(model));
    true
}

// ---------------------------------------------------------------------
// Running a confirmed model
// ---------------------------------------------------------------------

/// What [`run`] started: the pending keys to watch and the reasons the
/// actions service refused the rest.
#[derive(Debug, Default, PartialEq)]
pub struct Launched {
    pub keys: Vec<PendingKey>,
    pub refused: Vec<String>,
}

fn launch_container(
    state: &mut AppState,
    actions: &ActionsService,
    id: &ContainerId,
    action: ContainerAction,
    out: &mut Launched,
) {
    let target = state
        .containers
        .peek()
        .iter()
        .find(|c| c.id == *id)
        .map(|c| (ContainerTarget::from(c), is_active(c.state)));
    let Some((target, active)) = target else {
        out.refused
            .push("This container no longer exists".to_string());
        return;
    };
    // A running container needs `force` to be removed.
    let action = match action {
        ContainerAction::Remove { force, volumes } => ContainerAction::Remove {
            force: force || active,
            volumes,
        },
        other => other,
    };
    let key = PendingKey {
        subject: Subject::Container(id.clone()),
        action: container_action_id(&action),
    };
    match actions.container(state, &target, action) {
        Ok(()) => out.keys.push(key),
        Err(reason) => out.refused.push(reason),
    }
}

/// The model's action with the dialog's checkbox applied: on a delete it
/// decides whether anonymous volumes go too; any other action is as is.
#[must_use]
pub fn with_volumes(action: &ContainerAction, checked: bool) -> ContainerAction {
    match action {
        ContainerAction::Remove { force, .. } => ContainerAction::Remove {
            force: *force,
            volumes: checked,
        },
        other => other.clone(),
    }
}

/// Send a confirmed model to the actions service. `checked` is the state
/// of the dialog's checkbox (it means "also delete anonymous volumes" on a
/// delete).
pub fn run(
    state: &mut AppState,
    actions: &ActionsService,
    model: &ConfirmModel,
    checked: bool,
) -> Launched {
    let mut out = Launched::default();
    match &model.action {
        EngineAction::Container { id, action, .. } => {
            let action = with_volumes(action, checked);
            // One container action per target, each with its own pending
            // state and activity entry. The selection is not consulted. A
            // bulk model's action names its first target (PLAN §6 has no
            // multi-container action), so that one must be the container
            // the action names.
            if model.targets.first() != Some(&Subject::Container(id.clone())) {
                out.refused
                    .push("This is not the container to act on".to_string());
                return out;
            }
            for target in &model.targets {
                match target {
                    Subject::Container(id) => {
                        launch_container(state, actions, id, action.clone(), &mut out);
                    }
                    _ => out.refused.push("This is not a container".to_string()),
                }
            }
        }
        EngineAction::Colima { profile, op } => {
            let engine = state
                .engines
                .peek()
                .iter()
                .find(|e| crate::services::colima::profile_of(e) == Some(profile.as_str()))
                .cloned();
            // The one target is the engine the profile names.
            let engine = engine.filter(|e| model.targets == [Subject::Engine(e.id.clone())]);
            match engine {
                Some(engine) => match actions.engine(state, &engine, op.clone()) {
                    Ok(()) => out.keys.push(PendingKey {
                        subject: Subject::Engine(engine.id.clone()),
                        action: crate::services::colima::action_id(op),
                    }),
                    Err(reason) => out.refused.push(reason),
                },
                None => out.refused.push(format!(
                    "Engine {profile} is gone, or is not the engine to act on"
                )),
            }
        }
        // Edit and recreate, and recreate with the latest image (D18).
        EngineAction::Recreate { id, spec, pull, .. } => {
            for target in &model.targets {
                match target {
                    Subject::Container(target_id) if target_id == id => {
                        launch_recreate(state, actions, target_id, spec, *pull, &mut out);
                    }
                    _ => out
                        .refused
                        .push("This is not the container to recreate".to_string()),
                }
            }
        }
        EngineAction::Remove {
            engine,
            target: kind,
            force,
        } => {
            // Exactly the resource the action names: same kind, same name
            // or id. A dialog opened for one volume cannot delete another.
            for subject in &model.targets {
                match ResourceRef::from_subject(subject) {
                    Some(target) if target == *kind => {
                        launch_remove(state, actions, engine, &target, *force, &mut out);
                    }
                    _ => out
                        .refused
                        .push(format!("This is not the {} to delete", kind.noun_one())),
                }
            }
        }
        _ => out
            .refused
            .push("This action cannot run from here yet".to_string()),
    }
    out
}

/// What the activity entry and the refusals call a resource: an image's
/// first tag (else its id), a volume's name, a network's name.
fn resource_name(state: &AppState, target: &ResourceRef) -> Option<String> {
    match target {
        ResourceRef::Image(id) => state
            .images
            .peek()
            .iter()
            .find(|i| i.id == *id)
            .map(|i| i.repo_tags.first().cloned().unwrap_or_else(|| id.clone())),
        ResourceRef::Volume(name) => state
            .volumes
            .peek()
            .iter()
            .find(|v| v.name == *name)
            .map(|v| v.name.clone()),
        ResourceRef::Network(id) => state
            .networks
            .peek()
            .iter()
            .find(|n| n.id == *id)
            .map(|n| n.name.clone()),
    }
}

fn launch_remove(
    state: &mut AppState,
    actions: &ActionsService,
    engine: &EngineId,
    target: &ResourceRef,
    force: bool,
    out: &mut Launched,
) {
    let Some(name) = resource_name(state, target) else {
        out.refused
            .push(format!("This {} no longer exists", target.noun_one()));
        return;
    };
    let key = PendingKey {
        subject: target.subject(),
        action: target.action_id(),
    };
    match actions.remove(state, engine, target, &name, force) {
        Ok(()) => out.keys.push(key),
        Err(reason) => out.refused.push(reason),
    }
}

fn launch_recreate(
    state: &mut AppState,
    actions: &ActionsService,
    id: &ContainerId,
    spec: &berth_core::ContainerSpec,
    pull: bool,
    out: &mut Launched,
) {
    let target = state
        .containers
        .peek()
        .iter()
        .find(|c| c.id == *id)
        .map(ContainerTarget::from);
    let Some(target) = target else {
        out.refused
            .push("This container no longer exists".to_string());
        return;
    };
    let op = crate::services::edit::EditOp::Recreate {
        spec: Box::new(spec.clone()),
        pull,
    };
    let key = PendingKey {
        subject: Subject::Container(target.id.clone()),
        action: op.action_id(),
    };
    match actions.edit(state, &target, op) {
        Ok(()) => out.keys.push(key),
        Err(reason) => out.refused.push(reason),
    }
}

/// How many of `keys` finished without a failure, once none is running.
/// `None` while one still runs.
#[must_use]
pub fn finished_count(
    pending: &std::collections::BTreeMap<PendingKey, PendingState>,
    keys: &[PendingKey],
) -> Option<usize> {
    if keys
        .iter()
        .any(|k| matches!(pending.get(k), Some(PendingState::Running { .. })))
    {
        return None;
    }
    Some(
        keys.iter()
            .filter(|k| !matches!(pending.get(*k), Some(PendingState::Failed { .. })))
            .count(),
    )
}

fn push_toast(state: &mut AppState, text: String, tone: ToastTone) {
    state.toasts.write().push(Toast {
        id: uuid::Uuid::now_v7(),
        text,
        tone,
        action: None,
    });
}

/// After [`run`]: say what was refused, then wait for the started actions
/// and say what went through, from the same summary the dialog showed.
/// Failures stay on their rows (D34), so only successes are counted.
pub async fn watch_result(mut state: AppState, effect: EffectSummary, launched: Launched) {
    for reason in launched.refused {
        push_toast(&mut state, reason, ToastTone::Danger);
    }
    if launched.keys.is_empty() {
        return;
    }
    let done = loop {
        if let Some(done) = finished_count(&state.pending.peek(), &launched.keys) {
            break done;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    };
    if done > 0 {
        push_toast(&mut state, result_label(&effect, done), ToastTone::Success);
    }
}

#[cfg(test)]
#[path = "confirm_tests.rs"]
mod tests;
