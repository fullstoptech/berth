//! Pure grouping of containers into projects (PLAN section 6.5).
//!
//! Compose containers group by project name plus working dir, so two
//! checkouts of the same compose project stay separate. Pins merge into a
//! compose project in the same folder or stand alone as `NotCreated`. A
//! plain container that bind-mounts a path inside a pinned folder is
//! `Inferred`. On a remote engine compose groups are `RemoteOnly` (their
//! working dir is a remote path, not a local folder). Everything else
//! lands in one `Loose` project named "Unassigned".

/// Compose label holding the project working directory.
pub const LABEL_WORKING_DIR: &str = "com.docker.compose.project.working_dir";
/// Compose label holding the comma-separated config file list.
pub const LABEL_CONFIG_FILES: &str = "com.docker.compose.project.config_files";

const LOOSE_NAME: &str = "Unassigned";
const LOOSE_ID_KEY: &str = "berth:unassigned";

fn state_rank(state: berth_core::ContainerState) -> u8 {
    use berth_core::ContainerState as S;
    match state {
        S::Running => 0,
        S::Paused => 1,
        S::Removing => 2,
        S::Created => 3,
        S::Exited => 4,
        S::Restarting => 5,
        S::Dead => 6,
    }
}

fn health_rank(health: berth_core::Health) -> u8 {
    use berth_core::Health as H;
    match health {
        H::Healthy => 0,
        H::Starting => 1,
        H::Unhealthy => 2,
    }
}

/// Worst-of merge: a service reads `Running` only when every replica runs,
/// and a failing replica (dead, restarting, unhealthy) stays visible.
fn merge_health(
    a: Option<berth_core::Health>,
    b: Option<berth_core::Health>,
) -> Option<berth_core::Health> {
    match (a, b) {
        (Some(x), Some(y)) => Some(if health_rank(x) >= health_rank(y) {
            x
        } else {
            y
        }),
        (x, None) => x,
        (None, y) => y,
    }
}

fn add_service(services: &mut Vec<berth_core::ServiceSummary>, c: &berth_core::Container) {
    let name = c
        .service
        .clone()
        .unwrap_or_else(|| c.name.trim_start_matches('/').to_string());
    if let Some(svc) = services.iter_mut().find(|s| s.name == name) {
        svc.replicas = svc.replicas.saturating_add(1);
        if state_rank(c.state) > state_rank(svc.state) {
            svc.state = c.state;
        }
        svc.health = merge_health(svc.health, c.health);
        for port in &c.ports {
            if !svc.ports.contains(port) {
                svc.ports.push(*port);
            }
        }
        if !svc.containers.contains(&c.id) {
            svc.containers.push(c.id.clone());
        }
        return;
    }
    services.push(berth_core::ServiceSummary {
        name,
        image: c.image.reference.clone(),
        replicas: 1,
        state: c.state,
        ports: c.ports.clone(),
        health: c.health,
        containers: vec![c.id.clone()],
    });
}

fn label_path(c: &berth_core::Container, label: &str) -> Option<std::path::PathBuf> {
    c.labels
        .get(label)
        .map(|v| v.trim())
        .filter(|v| !v.is_empty())
        .map(std::path::PathBuf::from)
}

fn compose_files_of(c: &berth_core::Container) -> Vec<std::path::PathBuf> {
    c.labels
        .get(LABEL_CONFIG_FILES)
        .map(|v| {
            v.split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(std::path::PathBuf::from)
                .collect()
        })
        .unwrap_or_default()
}

fn folder_name(folder: &std::path::Path) -> String {
    folder
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| folder.to_string_lossy().into_owned())
}

fn blank_project(
    engine: &berth_core::EngineId,
    id: berth_core::ProjectId,
    name: String,
    folder: Option<std::path::PathBuf>,
    source: berth_core::ProjectSource,
) -> berth_core::Project {
    berth_core::Project {
        id,
        name,
        folder,
        compose_files: Vec::new(),
        engine: engine.clone(),
        source,
        services: Vec::new(),
        git: None,
        status: berth_core::ProjectStatus::NotCreated,
        pinned: false,
    }
}

/// Group containers into projects for one engine (PLAN section 6.5).
///
/// Output is sorted by name (then folder) with the Loose project last, and
/// every project's `status` is `status(&services)`, so a pin with no
/// containers reads `NotCreated`.
#[must_use]
pub fn group(
    engine: &berth_core::EngineId,
    containers: &[berth_core::Container],
    pins: &[berth_core::PinnedFolder],
    remote: bool,
) -> Vec<berth_core::Project> {
    let mut projects: Vec<berth_core::Project> = Vec::new();

    // Compose groups, keyed by project name + working dir. Remote groups
    // key by name only: their id is for_remote(name) and the working dir
    // is a remote path that names no local folder.
    for c in containers {
        let Some(name) = c.compose_project.as_deref().filter(|n| !n.is_empty()) else {
            continue;
        };
        let folder = if remote {
            None
        } else {
            label_path(c, LABEL_WORKING_DIR)
        };
        let idx = projects.iter().position(|p| {
            matches!(
                p.source,
                berth_core::ProjectSource::Compose | berth_core::ProjectSource::RemoteOnly
            ) && p.name == name
                && p.folder == folder
        });
        let idx = match idx {
            Some(idx) => idx,
            None => {
                let id = match &folder {
                    Some(dir) => berth_core::ProjectId::for_folder(dir, engine),
                    None => berth_core::ProjectId::for_remote(name, engine),
                };
                let source = if remote {
                    berth_core::ProjectSource::RemoteOnly
                } else {
                    berth_core::ProjectSource::Compose
                };
                projects.push(blank_project(engine, id, name.to_string(), folder, source));
                projects.len() - 1
            }
        };
        if !remote {
            for file in compose_files_of(c) {
                if !projects[idx].compose_files.contains(&file) {
                    projects[idx].compose_files.push(file);
                }
            }
        }
        add_service(&mut projects[idx].services, c);
    }

    // Pins for this engine merge into a same-folder compose project, or
    // stand alone. Remote engines have no local folders, so pins and
    // bind-mount inference do not apply there.
    let local_pins: Vec<&berth_core::PinnedFolder> = if remote {
        Vec::new()
    } else {
        pins.iter().filter(|p| p.engine == *engine).collect()
    };
    for pin in &local_pins {
        if let Some(p) = projects
            .iter_mut()
            .find(|p| p.folder.as_deref() == Some(pin.folder.as_path()))
        {
            p.pinned = true;
        } else {
            let mut p = blank_project(
                engine,
                berth_core::ProjectId::for_folder(&pin.folder, engine),
                folder_name(&pin.folder),
                Some(pin.folder.clone()),
                berth_core::ProjectSource::Pinned,
            );
            p.pinned = true;
            projects.push(p);
        }
    }

    // Plain containers: Inferred when a bind mount lands inside a pinned
    // folder (longest matching folder wins), else Loose.
    let mut loose: Vec<&berth_core::Container> = Vec::new();
    for c in containers {
        if c.compose_project.as_deref().is_some_and(|n| !n.is_empty()) {
            continue;
        }
        let inferred = c
            .mounts
            .iter()
            .filter(|m| m.kind == berth_core::MountKind::Bind)
            .filter_map(|m| {
                let source = std::path::Path::new(&m.source);
                local_pins
                    .iter()
                    .filter(|pin| source.starts_with(&pin.folder))
                    .max_by_key(|pin| pin.folder.components().count())
                    .map(|pin| (pin.folder.clone(), source.to_path_buf()))
            })
            .max_by_key(|(folder, _)| folder.components().count());
        match inferred {
            Some((folder, mount)) => {
                // The pin (or the compose project it merged into) is
                // already in `projects` under this folder.
                if let Some(p) = projects
                    .iter_mut()
                    .find(|p| p.folder.as_deref() == Some(folder.as_path()))
                {
                    if matches!(p.source, berth_core::ProjectSource::Pinned) {
                        p.source = berth_core::ProjectSource::Inferred {
                            from_bind_mount: mount,
                        };
                    }
                    add_service(&mut p.services, c);
                } else {
                    loose.push(c);
                }
            }
            None => loose.push(c),
        }
    }

    if !loose.is_empty() {
        let mut p = blank_project(
            engine,
            berth_core::ProjectId::for_remote(LOOSE_ID_KEY, engine),
            LOOSE_NAME.to_string(),
            None,
            berth_core::ProjectSource::Loose,
        );
        for c in loose {
            add_service(&mut p.services, c);
        }
        projects.push(p);
    }

    for p in &mut projects {
        p.services.sort_by(|a, b| a.name.cmp(&b.name));
        p.status = status(&p.services);
    }
    projects.sort_by(|a, b| {
        let a_loose = matches!(a.source, berth_core::ProjectSource::Loose);
        let b_loose = matches!(b.source, berth_core::ProjectSource::Loose);
        a_loose
            .cmp(&b_loose)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
            .then_with(|| a.folder.cmp(&b.folder))
    });
    projects
}

/// Project status from its services (PLAN section 6.5).
///
/// `Failing` when any service is dead, restarting or unhealthy; counts in
/// `Partial` are services (a `ServiceSummary` carries no per-replica
/// state or exit code, so an exited service alone reads `Stopped`).
#[must_use]
pub fn status(services: &[berth_core::ServiceSummary]) -> berth_core::ProjectStatus {
    use berth_core::ProjectStatus as P;
    if services.is_empty() {
        return P::NotCreated;
    }
    let failing = services.iter().any(|s| {
        matches!(
            s.state,
            berth_core::ContainerState::Dead | berth_core::ContainerState::Restarting
        ) || s.health == Some(berth_core::Health::Unhealthy)
    });
    if failing {
        return P::Failing;
    }
    let running = services
        .iter()
        .filter(|s| s.state == berth_core::ContainerState::Running)
        .count();
    if running == services.len() {
        P::AllRunning
    } else if running == 0 {
        P::Stopped
    } else {
        P::Partial {
            running: u16::try_from(running).unwrap_or(u16::MAX),
            total: u16::try_from(services.len()).unwrap_or(u16::MAX),
        }
    }
}

/// Parse `git status --porcelain=v2 --branch` output (D49).
///
/// Reads `# branch.head` (a parenthesised value such as "(detached from
/// abc)" gives no branch) and `# branch.ab +N -M`. Any entry line other
/// than a header or an ignored (`!`) line marks the tree dirty, untracked
/// files included. `remote` is left `None`; the caller fills it from
/// `git remote get-url origin` (see `crate::git`).
#[must_use]
pub fn parse_git_status(porcelain_v2: &str) -> berth_core::GitInfo {
    let mut info = berth_core::GitInfo {
        branch: None,
        dirty: false,
        remote: None,
        ahead: 0,
        behind: 0,
    };
    for line in porcelain_v2.lines() {
        if let Some(rest) = line.strip_prefix("# branch.head ") {
            let head = rest.trim();
            if !head.is_empty() && !head.starts_with('(') {
                info.branch = Some(head.to_string());
            }
        } else if let Some(rest) = line.strip_prefix("# branch.ab ") {
            for tok in rest.split_whitespace() {
                if let Some(n) = tok.strip_prefix('+') {
                    info.ahead = n.parse().unwrap_or(0);
                } else if let Some(n) = tok.strip_prefix('-') {
                    info.behind = n.parse().unwrap_or(0);
                }
            }
        } else if line.starts_with('#') || line.starts_with('!') || line.trim().is_empty() {
            // Header or ignored entry: not a working-tree change.
        } else {
            info.dirty = true;
        }
    }
    info
}

#[cfg(test)]
mod tests {
    use super::*;
    use berth_core::ContainerState as S;

    fn engine() -> berth_core::EngineId {
        berth_core::EngineId("colima:default".to_string())
    }

    fn container(name: &str, state: S) -> berth_core::Container {
        berth_core::Container {
            id: berth_core::ContainerId(name.to_string()),
            engine: engine(),
            name: name.to_string(),
            project: None,
            compose_project: None,
            service: None,
            image: berth_core::ImageRef {
                reference: "alpine:3".to_string(),
                id: None,
            },
            command: None,
            state,
            health: None,
            exit_code: None,
            restart_count: 0,
            created: chrono::DateTime::from_timestamp(0, 0).expect("epoch"),
            started: None,
            ports: Vec::new(),
            mounts: Vec::new(),
            networks: Vec::new(),
            labels: std::collections::BTreeMap::new(),
            stats: None,
            platform: None,
        }
    }

    fn compose(
        name: &str,
        project: &str,
        service: &str,
        dir: &str,
        state: S,
    ) -> berth_core::Container {
        let mut c = container(name, state);
        c.compose_project = Some(project.to_string());
        c.service = Some(service.to_string());
        c.labels
            .insert(LABEL_WORKING_DIR.to_string(), dir.to_string());
        c.labels.insert(
            LABEL_CONFIG_FILES.to_string(),
            format!("{dir}/compose.yaml"),
        );
        c
    }

    fn pin(folder: &str) -> berth_core::PinnedFolder {
        berth_core::PinnedFolder {
            folder: std::path::PathBuf::from(folder),
            engine: engine(),
            added: chrono::DateTime::from_timestamp(0, 0).expect("epoch"),
        }
    }

    fn svc(state: S) -> berth_core::ServiceSummary {
        berth_core::ServiceSummary {
            name: "s".to_string(),
            image: "alpine:3".to_string(),
            replicas: 1,
            state,
            ports: Vec::new(),
            health: None,
            containers: Vec::new(),
        }
    }

    #[test]
    fn projects_same_name_different_folders_stay_separate() {
        let cs = vec![
            compose("a-web-1", "shop", "web", "/x/one/shop", S::Running),
            compose("b-web-1", "shop", "web", "/x/two/shop", S::Running),
        ];
        let ps = group(&engine(), &cs, &[], false);
        assert_eq!(ps.len(), 2, "{ps:?}");
        assert!(ps.iter().all(|p| p.name == "shop"));
        assert_ne!(ps[0].id, ps[1].id);
        assert_ne!(ps[0].folder, ps[1].folder);
    }

    #[test]
    fn projects_same_name_same_folder_merges_services_and_replicas() {
        let cs = vec![
            compose("shop-web-1", "shop", "web", "/x/shop", S::Running),
            compose("shop-web-2", "shop", "web", "/x/shop", S::Running),
            compose("shop-db-1", "shop", "db", "/x/shop", S::Running),
        ];
        let ps = group(&engine(), &cs, &[], false);
        assert_eq!(ps.len(), 1);
        assert_eq!(ps[0].services.len(), 2);
        let web = ps[0]
            .services
            .iter()
            .find(|s| s.name == "web")
            .expect("web");
        assert_eq!(web.replicas, 2);
        assert_eq!(web.containers.len(), 2);
        assert_eq!(ps[0].status, berth_core::ProjectStatus::AllRunning);
        assert_eq!(
            ps[0].id,
            berth_core::ProjectId::for_folder(std::path::Path::new("/x/shop"), &engine())
        );
    }

    #[test]
    fn projects_pin_without_containers_is_not_created() {
        let ps = group(&engine(), &[], &[pin("/x/empty")], false);
        assert_eq!(ps.len(), 1);
        assert_eq!(ps[0].name, "empty");
        assert_eq!(ps[0].status, berth_core::ProjectStatus::NotCreated);
        assert!(matches!(ps[0].source, berth_core::ProjectSource::Pinned));
        assert!(ps[0].pinned);
        assert!(ps[0].services.is_empty());
    }

    #[test]
    fn projects_pin_merges_with_compose_project_in_same_folder() {
        let cs = vec![compose("shop-web-1", "shop", "web", "/x/shop", S::Running)];
        let ps = group(&engine(), &cs, &[pin("/x/shop")], false);
        assert_eq!(ps.len(), 1, "{ps:?}");
        assert!(matches!(ps[0].source, berth_core::ProjectSource::Compose));
        assert!(ps[0].pinned);
        assert_eq!(ps[0].status, berth_core::ProjectStatus::AllRunning);
    }

    #[test]
    fn projects_pin_for_another_engine_is_ignored() {
        let mut other = pin("/x/elsewhere");
        other.engine = berth_core::EngineId("colima:work".to_string());
        assert!(group(&engine(), &[], &[other], false).is_empty());
    }

    #[test]
    fn projects_inferred_from_bind_mount_inside_pinned_folder() {
        let mut c = container("scratch", S::Running);
        c.mounts.push(berth_core::Mount {
            kind: berth_core::MountKind::Bind,
            source: "/x/shop/data".to_string(),
            destination: "/data".to_string(),
            read_only: false,
            volume_name: None,
        });
        let ps = group(&engine(), &[c], &[pin("/x/shop")], false);
        assert_eq!(ps.len(), 1, "{ps:?}");
        assert!(matches!(
            &ps[0].source,
            berth_core::ProjectSource::Inferred { from_bind_mount } if from_bind_mount == std::path::Path::new("/x/shop/data")
        ));
        assert!(ps[0].pinned);
        assert_eq!(ps[0].services.len(), 1);
    }

    #[test]
    fn projects_bind_mount_outside_any_pin_is_loose() {
        let mut c = container("scratch", S::Running);
        c.mounts.push(berth_core::Mount {
            kind: berth_core::MountKind::Bind,
            source: "/elsewhere/data".to_string(),
            destination: "/data".to_string(),
            read_only: false,
            volume_name: None,
        });
        let ps = group(&engine(), &[c], &[pin("/x/shop")], false);
        let loose = ps
            .iter()
            .find(|p| matches!(p.source, berth_core::ProjectSource::Loose))
            .expect("loose project");
        assert_eq!(loose.name, "Unassigned");
        assert_eq!(loose.folder, None);
        assert_eq!(loose.services.len(), 1);
    }

    #[test]
    fn projects_remote_compose_is_remote_only_without_folder() {
        let cs = vec![compose(
            "shop-web-1",
            "shop",
            "web",
            "/srv/shop",
            S::Running,
        )];
        let ps = group(&engine(), &cs, &[pin("/srv/shop")], true);
        assert_eq!(ps.len(), 1);
        assert!(matches!(
            ps[0].source,
            berth_core::ProjectSource::RemoteOnly
        ));
        assert_eq!(ps[0].folder, None);
        assert!(ps[0].compose_files.is_empty());
        assert!(!ps[0].pinned);
        assert_eq!(
            ps[0].id,
            berth_core::ProjectId::for_remote("shop", &engine())
        );
    }

    #[test]
    fn projects_compose_files_parsed_from_label_and_deduped() {
        let mut a = compose("shop-web-1", "shop", "web", "/x/shop", S::Running);
        a.labels.insert(
            LABEL_CONFIG_FILES.to_string(),
            "/x/shop/compose.yaml,/x/shop/compose.override.yaml".to_string(),
        );
        let b = compose("shop-db-1", "shop", "db", "/x/shop", S::Running);
        let ps = group(&engine(), &[a, b], &[], false);
        assert_eq!(
            ps[0].compose_files,
            vec![
                std::path::PathBuf::from("/x/shop/compose.yaml"),
                std::path::PathBuf::from("/x/shop/compose.override.yaml"),
            ]
        );
    }

    #[test]
    fn projects_mixed_replica_states_keep_the_worst() {
        let cs = vec![
            compose("shop-web-1", "shop", "web", "/x/shop", S::Running),
            compose("shop-web-2", "shop", "web", "/x/shop", S::Exited),
        ];
        let ps = group(&engine(), &cs, &[], false);
        assert_eq!(ps[0].services[0].state, S::Exited);
        assert_eq!(ps[0].status, berth_core::ProjectStatus::Stopped);
    }

    #[test]
    fn projects_empty_input_gives_no_projects() {
        assert!(group(&engine(), &[], &[], false).is_empty());
    }

    #[test]
    fn projects_status_all_running_partial_and_stopped() {
        assert_eq!(
            status(&[svc(S::Running), svc(S::Running)]),
            berth_core::ProjectStatus::AllRunning
        );
        assert_eq!(
            status(&[svc(S::Running), svc(S::Exited), svc(S::Created)]),
            berth_core::ProjectStatus::Partial {
                running: 1,
                total: 3
            }
        );
        assert_eq!(
            status(&[svc(S::Exited), svc(S::Paused)]),
            berth_core::ProjectStatus::Stopped
        );
        assert_eq!(status(&[]), berth_core::ProjectStatus::NotCreated);
    }

    #[test]
    fn projects_status_failing_on_restarting_dead_and_unhealthy() {
        assert_eq!(
            status(&[svc(S::Running), svc(S::Restarting)]),
            berth_core::ProjectStatus::Failing
        );
        assert_eq!(status(&[svc(S::Dead)]), berth_core::ProjectStatus::Failing);
        let mut unhealthy = svc(S::Running);
        unhealthy.health = Some(berth_core::Health::Unhealthy);
        assert_eq!(status(&[unhealthy]), berth_core::ProjectStatus::Failing);
    }

    const CLEAN: &str = "# branch.oid abc123\n# branch.head main\n# branch.upstream origin/main\n# branch.ab +0 -0\n";

    #[test]
    fn git_status_clean_branch() {
        let info = parse_git_status(CLEAN);
        assert_eq!(info.branch.as_deref(), Some("main"));
        assert!(!info.dirty);
        assert_eq!((info.ahead, info.behind), (0, 0));
        assert_eq!(info.remote, None);
    }

    #[test]
    fn git_status_ahead_behind_and_dirty_entries() {
        let text = "# branch.oid abc\n# branch.head feat/x\n# branch.ab +2 -3\n1 .M N... 100644 100644 100644 a b file.rs\n? untracked.txt\n";
        let info = parse_git_status(text);
        assert_eq!(info.branch.as_deref(), Some("feat/x"));
        assert_eq!((info.ahead, info.behind), (2, 3));
        assert!(info.dirty);
    }

    #[test]
    fn git_status_detached_head_has_no_branch() {
        let info = parse_git_status("# branch.oid abc\n# branch.head (detached from abc)\n");
        assert_eq!(info.branch, None);
        assert!(!info.dirty);
    }

    #[test]
    fn git_status_ignored_entries_do_not_mark_dirty() {
        let info = parse_git_status(&format!("{CLEAN}! target/\n"));
        assert!(!info.dirty);
    }

    #[test]
    fn git_status_empty_output_is_blank_info() {
        let info = parse_git_status("");
        assert_eq!(info.branch, None);
        assert!(!info.dirty);
        assert_eq!((info.ahead, info.behind), (0, 0));
    }
}
