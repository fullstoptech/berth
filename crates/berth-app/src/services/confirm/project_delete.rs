//! Project confirms: Delete project (P5-T6). The sheet has two checkboxes,
//! so this builder takes the options and returns the one `EffectSummary` the
//! sheet's rows, button and result message all read.

use berth_core::Project;

use super::*;

/// What the Delete project sheet's checkboxes ask for. Both off is the safe
/// default: `compose down` removes containers and networks only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DeleteOptions {
    /// Also delete the project's volumes (`down -v`): their data goes.
    pub volumes: bool,
    /// Also remove the images its services use (`down --rmi all`).
    pub remove_images: bool,
}

/// What a project owns on the engine, read from the state when the sheet
/// opens.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ProjectHoldings {
    pub containers: Vec<String>,
    pub volumes: Vec<(String, Option<bytesize::ByteSize>)>,
    pub images: Vec<String>,
}

/// "api, worker and 3 more": up to `max` names, then a count.
fn name_list(names: &[String], max: usize) -> String {
    if names.len() <= max {
        return names.join(", ");
    }
    format!("{} and {} more", names[..max].join(", "), names.len() - max)
}

/// Compose labels every volume it creates with its project's name. The
/// engine's volume list leaves `Volume::project` empty, so the label is what
/// ties a live volume to its project; the field answers for the mock.
fn owns_volume(project: &Project, volume: &Volume) -> bool {
    volume.project.as_ref() == Some(&project.id)
        || volume
            .labels
            .get("com.docker.compose.project")
            .is_some_and(|name| *name == project.name)
}

/// The containers, volumes and images of `project`, by name, in a stable
/// order (containers and volumes by name, images in service order).
#[must_use]
pub fn project_holdings(
    project: &Project,
    containers: &[Container],
    volumes: &[Volume],
) -> ProjectHoldings {
    let ids: Vec<&ContainerId> = project
        .services
        .iter()
        .flat_map(|s| s.containers.iter())
        .collect();
    let mut names: Vec<String> = ids
        .iter()
        .map(|id| {
            containers
                .iter()
                .find(|c| c.id == **id)
                .map_or_else(|| id.0.clone(), |c| c.name.clone())
        })
        .collect();
    names.sort();
    let mut owned: Vec<(String, Option<bytesize::ByteSize>)> = volumes
        .iter()
        .filter(|v| v.engine == project.engine && owns_volume(project, v))
        .map(|v| (v.name.clone(), v.size))
        .collect();
    owned.sort_by(|a, b| a.0.cmp(&b.0));
    let mut images: Vec<String> = Vec::new();
    for service in &project.services {
        if !service.image.is_empty() && !images.contains(&service.image) {
            images.push(service.image.clone());
        }
    }
    ProjectHoldings {
        containers: names,
        volumes: owned,
        images,
    }
}

/// What deleting a project does under `options`. Containers and networks
/// always go; volumes and images only when ticked; the folder never. With
/// volumes ticked and some to delete, the project's name must be typed.
#[must_use]
pub fn project_delete_effect(
    project: &Project,
    holdings: &ProjectHoldings,
    folder: Option<&str>,
    options: DeleteOptions,
) -> EffectSummary {
    let mut lines = Vec::new();
    lines.push(line(
        EffectKind::Goes,
        if holdings.containers.is_empty() {
            "The project's networks. It has no containers right now.".to_string()
        } else {
            format!(
                "{} ({}) and the project's networks.",
                plural(holdings.containers.len(), "container", "containers"),
                name_list(&holdings.containers, 4)
            )
        },
    ));
    if !holdings.volumes.is_empty() {
        let names: Vec<String> = holdings.volumes.iter().map(|(n, _)| n.clone()).collect();
        let text = format!(
            "{} with their data: {}.",
            plural(names.len(), "volume", "volumes"),
            name_list(&names, 4)
        );
        lines.push(line(
            if options.volumes {
                EffectKind::Goes
            } else {
                EffectKind::Stays
            },
            text,
        ));
    }
    if !holdings.images.is_empty() {
        let count = plural(holdings.images.len(), "image", "images");
        let names = name_list(&holdings.images, 3);
        lines.push(if options.remove_images {
            line(
                EffectKind::Goes,
                format!("{count} its services use: {names}. They are pulled or built again when needed."),
            )
        } else {
            line(
                EffectKind::Stays,
                format!("{count}: {names}, so the next start needs no download."),
            )
        });
    }
    lines.push(line(
        EffectKind::Stays,
        match folder {
            Some(folder) => format!("Your project folder and every file in it: {folder}."),
            None => "Your files: nothing on this Mac is touched.".to_string(),
        },
    ));
    let mut effect = summary("Delete", "project", "projects", 1, lines);
    if options.volumes && !holdings.volumes.is_empty() {
        effect.typed_name = Some(project.name.clone());
        let sizes: Vec<bytesize::ByteSize> =
            holdings.volumes.iter().filter_map(|(_, s)| *s).collect();
        if sizes.len() == holdings.volumes.len() {
            effect.bytes = Some(
                sizes
                    .into_iter()
                    .fold(bytesize::ByteSize::b(0), |total, next| total + next),
            );
        }
    }
    effect
}

/// The checkbox labels of the sheet: [volumes, images].
#[must_use]
pub fn delete_option_labels(holdings: &ProjectHoldings) -> [String; 2] {
    [
        match holdings.volumes.len() {
            0 => "Also delete volumes (this project has none)".to_string(),
            1 => "Also delete its volume and the data in it".to_string(),
            n => format!("Also delete its {n} volumes and the data in them"),
        },
        "Also remove the images its services use".to_string(),
    ]
}
