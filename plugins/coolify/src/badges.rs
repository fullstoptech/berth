//! `project/badges` linking (P9-T4): a local project whose git remote
//! names the same repository as a Coolify application's `git_repository`
//! gets a "Deployed on Coolify: <status>" badge.

use berth_plugin_protocol::Tone;

use crate::types::Application;

/// What the host asked about: one local project and its git remote.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectRef {
    pub id: String,
    pub git_remote: Option<String>,
}

/// One `project/badges` answer entry (`{id, text, tone}` on the wire).
#[derive(Debug, Clone, PartialEq)]
pub struct ProjectBadge {
    pub id: String,
    pub text: String,
    pub tone: Tone,
}

/// Reduces a git remote or a Coolify `git_repository` to `owner/repo`,
/// lower-cased: handles `https://github.com/Owner/Repo.git`,
/// `git@github.com:owner/repo`, `ssh://git@host/owner/repo.git` and the
/// bare `owner/repo` Coolify itself stores. Anything without exactly an
/// owner and a repo part is `None`.
#[must_use]
pub fn normalize_repo(remote: &str) -> Option<String> {
    let trimmed = remote.trim().trim_end_matches('/').trim_end_matches(".git");
    let path = if let Some((_, after_scheme)) = trimmed.split_once("://") {
        // host/owner/repo — drop the host.
        after_scheme.split_once('/')?.1
    } else if let Some((_, after_colon)) = trimmed.split_once('@').and_then(|(user, rest)| {
        rest.split_once(':')
            .map(|(host, path)| (user, (host, path)))
    }) {
        after_colon.1
    } else {
        trimmed
    };
    let mut parts = path.split('/').filter(|part| !part.is_empty());
    let owner = parts.next()?;
    let repo = parts.next()?;
    if parts.next().is_some() {
        return None;
    }
    Some(format!(
        "{}/{}",
        owner.to_ascii_lowercase(),
        repo.to_ascii_lowercase()
    ))
}

/// The badge word for a Coolify status: the part before any `:`
/// qualifier (`running:healthy` → `running`).
fn status_word(status: Option<&str>) -> &str {
    match status {
        Some(status) => status.split(':').next().unwrap_or("unknown"),
        None => "unknown",
    }
}

fn status_tone(status: Option<&str>) -> Tone {
    match status_word(status) {
        "running" => Tone::Success,
        "exited" | "failed" => Tone::Danger,
        _ => Tone::Neutral,
    }
}

/// One badge per project whose remote matches an application's
/// repository; projects without a remote or a match get none.
#[must_use]
pub fn project_badges(projects: &[ProjectRef], applications: &[Application]) -> Vec<ProjectBadge> {
    let mut badges = Vec::new();
    for project in projects {
        let Some(remote) = project.git_remote.as_deref() else {
            continue;
        };
        let Some(wanted) = normalize_repo(remote) else {
            continue;
        };
        let Some(application) = applications.iter().find(|application| {
            application
                .git_repository
                .as_deref()
                .and_then(normalize_repo)
                .is_some_and(|repo| repo == wanted)
        }) else {
            continue;
        };
        badges.push(ProjectBadge {
            id: project.id.clone(),
            text: format!(
                "Deployed on Coolify: {}",
                status_word(application.status.as_deref())
            ),
            tone: status_tone(application.status.as_deref()),
        });
    }
    badges
}

#[cfg(test)]
mod tests {
    use super::*;

    fn landing() -> Application {
        serde_json::from_str(include_str!("../../../fixtures/coolify/application.json"))
            .expect("fixture decodes")
    }

    fn project(id: &str, remote: Option<&str>) -> ProjectRef {
        ProjectRef {
            id: id.to_owned(),
            git_remote: remote.map(str::to_owned),
        }
    }

    #[test]
    fn badges_https_remote_matches_the_application() {
        let badges = project_badges(
            &[project(
                "p1",
                Some("https://github.com/example/landing.git"),
            )],
            &[landing()],
        );
        assert_eq!(badges.len(), 1);
        assert_eq!(badges[0].id, "p1");
        assert_eq!(badges[0].text, "Deployed on Coolify: running");
        assert_eq!(badges[0].tone, Tone::Success);
    }

    #[test]
    fn badges_ssh_remote_matches_too() {
        let badges = project_badges(
            &[project("p1", Some("git@github.com:Example/Landing"))],
            &[landing()],
        );
        assert_eq!(badges.len(), 1);
        assert_eq!(badges[0].text, "Deployed on Coolify: running");
    }

    #[test]
    fn badges_unmatched_and_remoteless_projects_get_none() {
        let badges = project_badges(
            &[
                project("p1", Some("https://github.com/example/other")),
                project("p2", None),
            ],
            &[landing()],
        );
        assert!(badges.is_empty());
    }

    #[test]
    fn badges_exited_application_badges_as_exited() {
        let mut app = landing();
        app.status = Some("exited".to_owned());
        let badges = project_badges(&[project("p1", Some("example/landing"))], &[app]);
        assert_eq!(badges[0].text, "Deployed on Coolify: exited");
        assert_eq!(badges[0].tone, Tone::Danger);
    }

    #[test]
    fn badges_normalize_repo_shapes() {
        assert_eq!(
            normalize_repo("https://github.com/Owner/Repo.git").as_deref(),
            Some("owner/repo")
        );
        assert_eq!(
            normalize_repo("git@github.com:owner/repo").as_deref(),
            Some("owner/repo")
        );
        assert_eq!(
            normalize_repo("ssh://git@git.example.test/owner/repo.git").as_deref(),
            Some("owner/repo")
        );
        assert_eq!(normalize_repo("owner/repo").as_deref(), Some("owner/repo"));
        assert_eq!(normalize_repo("just-a-name"), None);
        assert_eq!(normalize_repo("https://github.com/onlyowner"), None);
    }
}
