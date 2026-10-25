//! "Write to compose.yaml instead" (P5-T3): the edit sheet's changes to a Compose service, as a
//! plan for Review changes. Only what a compose file can say in one scalar is covered: a changed
//! host port, the restart policy and the CPU, memory and swap limits. Anything else is named, so
//! the person edits the file for it. The container is recreated by Compose afterwards, not by
//! Berth (AD17).

use std::path::Path;

use std::path::PathBuf;
use std::sync::Arc;

use berth_core::{ComposeCommand, ContainerSpec, GenerationPlan, PlanSource, RestartPolicy};
use berth_rules::{compose_edit, diff, RulesError};

use crate::services::plans::{PendingPlan, PlanRun};

/// The label Compose puts on a container with the name of its service.
pub const COMPOSE_SERVICE_LABEL: &str = "com.docker.compose.service";

const MIB: u64 = 1024 * 1024;
const GIB: u64 = 1024 * MIB;

/// `512m`, `2g`, or plain bytes when the size is not a whole MiB.
fn size_text(bytes: u64) -> String {
    if bytes > 0 && bytes.is_multiple_of(GIB) {
        format!("{}g", bytes / GIB)
    } else if bytes > 0 && bytes.is_multiple_of(MIB) {
        format!("{}m", bytes / MIB)
    } else {
        bytes.to_string()
    }
}

/// The YAML scalar for a restart policy. `no` is quoted: bare, YAML reads it as `false`.
fn restart_text(policy: RestartPolicy) -> String {
    match policy {
        RestartPolicy::No => "\"no\"".to_string(),
        RestartPolicy::Always => "always".to_string(),
        RestartPolicy::UnlessStopped => "unless-stopped".to_string(),
        RestartPolicy::OnFailure { max_retries: 0 } => "on-failure".to_string(),
        RestartPolicy::OnFailure { max_retries } => format!("on-failure:{max_retries}"),
    }
}

/// The settings that differ and that a compose file cannot take from here.
fn unsupported(old: &ContainerSpec, new: &ContainerSpec) -> Vec<&'static str> {
    let mut names = Vec::new();
    if old.image != new.image {
        names.push("the image");
    }
    if old.command != new.command || old.entrypoint != new.entrypoint {
        names.push("the command");
    }
    if old.env != new.env {
        names.push("environment variables");
    }
    if old.mounts != new.mounts {
        names.push("volumes and mounts");
    }
    if old.networks != new.networks {
        names.push("networks");
    }
    names
}

/// The plan that writes `old` to `new` into the compose file of `service`.
///
/// `compose_rel` is the file's path inside `root`, `compose_text` its text now. The plan's
/// `after_write` runs `up -d <service>`: Compose recreates the one service.
///
/// # Errors
/// A sentence naming what cannot be written (other settings, a port added or removed, a limit
/// removed) or what the file refuses (the service is not in it, the YAML is broken).
pub fn compose_write_plan(
    root: &Path,
    compose_rel: &Path,
    compose_text: &str,
    service: &str,
    old: &ContainerSpec,
    new: &ContainerSpec,
) -> Result<GenerationPlan, String> {
    let other = unsupported(old, new);
    if !other.is_empty() {
        return Err(format!(
            "compose.yaml can take port, restart and limit changes from here. Change {} in the file itself.",
            other.join(", ")
        ));
    }
    let mut text = compose_text.to_string();
    let mut changed = false;
    // One edit of `text`: the rules' function takes the text so far and returns the next.
    macro_rules! edit {
        ($f:path, $($arg:expr),+ $(,)?) => {{
            text = $f(&text, $($arg),+).map_err(|e| e.to_string())?;
            changed = true;
        }};
    }

    // Ports: a mapping keeps its container port and protocol, only the host port moves.
    if old.ports.len() != new.ports.len() {
        return Err(
            "Adding or removing a published port is not written from here. Edit the ports in compose.yaml."
                .to_string(),
        );
    }
    for wanted in &new.ports {
        let before = old
            .ports
            .iter()
            .find(|p| p.container_port == wanted.container_port && p.proto == wanted.proto)
            .ok_or_else(|| {
                "Adding or removing a published port is not written from here. Edit the ports in compose.yaml."
                    .to_string()
            })?;
        if before.host_port == wanted.host_port && before.host_ip == wanted.host_ip {
            continue;
        }
        let host = wanted
            .host_port
            .ok_or_else(|| "A port with no host port is not written from here.".to_string())?;
        if before.host_ip != wanted.host_ip {
            return Err(
                "Changing the address a port binds to is not written from here.".to_string(),
            );
        }
        edit!(
            compose_edit::set_host_port,
            service,
            wanted.container_port,
            host,
        );
    }

    if old.restart != new.restart {
        edit!(
            compose_edit::set_service_key,
            service,
            "restart",
            &restart_text(new.restart),
        );
    }

    let (ol, nl) = (&old.limits, &new.limits);
    if (ol.cpus.is_some() && nl.cpus.is_none())
        || (ol.memory.is_some() && nl.memory.is_none())
        || (ol.memory_swap.is_some() && nl.memory_swap.is_none())
    {
        return Err(
            "Removing a limit is not written from here. Delete the line in compose.yaml."
                .to_string(),
        );
    }
    if ol.cpus != nl.cpus {
        if let Some(cpus) = nl.cpus {
            edit!(
                compose_edit::set_service_key,
                service,
                "cpus",
                &format!("{cpus}")
            );
        }
    }
    if ol.memory != nl.memory {
        if let Some(memory) = nl.memory {
            edit!(
                compose_edit::set_service_key,
                service,
                "mem_limit",
                &size_text(memory.as_u64()),
            );
        }
    }
    if ol.memory_swap != nl.memory_swap {
        if let Some(swap) = nl.memory_swap {
            edit!(
                compose_edit::set_service_key,
                service,
                "memswap_limit",
                &size_text(swap.as_u64()),
            );
        }
    }

    if !changed {
        return Err("Nothing differs from the container yet.".to_string());
    }
    Ok(GenerationPlan {
        root: root.to_path_buf(),
        files: vec![diff::file_diff(
            compose_rel,
            Some(compose_text),
            Some(&text),
        )],
        source: PlanSource::Editor,
        notes: Vec::new(),
        after_write: Some(ComposeCommand::Up {
            services: vec![service.to_string()],
            build: false,
        }),
    })
}

/// The compose file of `project` that holds `service`: the first of its known files that exists,
/// as a path inside the folder and its text.
fn read_compose(project: &berth_core::Project) -> Result<(PathBuf, PathBuf, String), String> {
    let folder = project
        .folder
        .clone()
        .ok_or_else(|| "This project has no folder on this Mac.".to_string())?;
    let file = project
        .compose_files
        .iter()
        .find(|f| folder.join(f).is_file())
        .ok_or_else(|| "Berth cannot find this project's compose file on this Mac.".to_string())?;
    let absolute = folder.join(file);
    let text = std::fs::read_to_string(&absolute)
        .map_err(|e| format!("Cannot read {}: {e}", absolute.display()))?;
    let rel = absolute.strip_prefix(&folder).map_or_else(
        |_| PathBuf::from(file.file_name().unwrap_or_default()),
        PathBuf::from,
    );
    Ok((folder, rel, text))
}

/// The pending plan that writes the edit sheet's change into the compose file of `project`: read
/// from disk now, built again by Regenerate when the file changes meanwhile. Blocking.
///
/// # Errors
/// A sentence: no folder or file, something the file cannot take, or invalid YAML.
pub fn pending_for_project(
    project: &berth_core::Project,
    service: &str,
    old: &ContainerSpec,
    new: &ContainerSpec,
) -> Result<PendingPlan, String> {
    let (folder, rel, text) = read_compose(project)?;
    let plan = compose_write_plan(&folder, &rel, &text, service, old, new)?;
    let mut pending = PendingPlan::new(plan, project.name.clone());
    pending.run = PlanRun::for_project(project);
    pending.footer_note = Some(format!(
        "Compose recreates {service} afterwards. Volumes are kept."
    ));
    let (project, service, old, new) = (
        project.clone(),
        service.to_string(),
        old.clone(),
        new.clone(),
    );
    pending.regenerate = Some(Arc::new(move || {
        let (folder, rel, text) = read_compose(&project)
            .map_err(|m| RulesError::Read(project.folder.clone().unwrap_or_default(), m))?;
        compose_write_plan(&folder, &rel, &text, &service, &old, &new)
            .map_err(|m| RulesError::Read(folder, m))
    }));
    Ok(pending)
}

#[cfg(test)]
#[path = "compose_write_tests.rs"]
mod tests;
