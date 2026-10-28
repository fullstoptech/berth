//! What this Coolify instance supports, learned while Berth uses it.
//!
//! The version is read when a view loads and the last read is older than
//! the plugin's `version_ttl`, and again at once after a run of 404/422
//! answers from endpoints that worked. Each endpoint or action is a
//! [`Feature`]: one that answers 404 before it ever worked here, or answers
//! without the fields Berth reads, is marked unsupported on this instance and
//! its buttons and tabs are pruned from every view; the rest keeps working.
//! A different version forgets everything learned, since an upgrade may
//! bring an endpoint back or take one away.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::time::{Duration, Instant};

use berth_plugin_protocol::{ActionSpec, View};

use crate::actions::{
    ACTION_CANCEL_DEPLOYMENT, ACTION_DEPLOY, ACTION_RESTART, ACTION_START, ACTION_STOP,
};
use crate::client::ClientError;
use crate::types::{ResourceKind, VersionInfo};

/// 404/422 answers from endpoints that worked here before the version is
/// read again without waiting for the interval.
pub const ERROR_RUN: u32 = 2;

/// One endpoint or action of the Coolify API that Berth uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Feature {
    Resources,
    Servers,
    RunningDeployments,
    Applications,
    Application,
    History,
    Envs,
    AppLogs,
    Deploy,
    Start(ResourceKind),
    Stop(ResourceKind),
    Restart(ResourceKind),
    CancelDeployment,
}

impl Feature {
    /// How the banner and messages name it.
    #[must_use]
    pub fn label(self) -> String {
        let lifecycle =
            |verb: &str, kind: ResourceKind| format!("{verb} ({})", kind.path_segment());
        match self {
            Feature::Resources => "the resource list".to_owned(),
            Feature::Servers => "the server list".to_owned(),
            Feature::RunningDeployments => "the running deployments".to_owned(),
            Feature::Applications => "the application list".to_owned(),
            Feature::Application => "application details".to_owned(),
            Feature::History => "deployment history".to_owned(),
            Feature::Envs => "environment variables".to_owned(),
            Feature::AppLogs => "application logs".to_owned(),
            Feature::Deploy => "Deploy".to_owned(),
            Feature::Start(kind) => lifecycle("Start", kind),
            Feature::Stop(kind) => lifecycle("Stop", kind),
            Feature::Restart(kind) => lifecycle("Restart", kind),
            Feature::CancelDeployment => "Cancel deployment".to_owned(),
        }
    }

    /// The request behind it, as the spec writes it.
    #[must_use]
    pub fn endpoint(self) -> String {
        let lifecycle = |verb: &str, kind: ResourceKind| {
            format!("POST /{}/{{uuid}}/{verb}", kind.path_segment())
        };
        match self {
            Feature::Resources => "GET /resources".to_owned(),
            Feature::Servers => "GET /servers".to_owned(),
            Feature::RunningDeployments => "GET /deployments".to_owned(),
            Feature::Applications => "GET /applications".to_owned(),
            Feature::Application => "GET /applications/{uuid}".to_owned(),
            Feature::History => "GET /deployments/applications/{uuid}".to_owned(),
            Feature::Envs => "GET /applications/{uuid}/envs".to_owned(),
            Feature::AppLogs => "GET /applications/{uuid}/logs".to_owned(),
            Feature::Deploy => "POST /deploy".to_owned(),
            Feature::Start(kind) => lifecycle("start", kind),
            Feature::Stop(kind) => lifecycle("stop", kind),
            Feature::Restart(kind) => lifecycle("restart", kind),
            Feature::CancelDeployment => "POST /deployments/{uuid}/cancel".to_owned(),
        }
    }

    /// The feature an action button runs (its id and, for start, stop and
    /// restart, the `kind` its params carry). `None` for a button that
    /// only opens a view.
    #[must_use]
    pub fn of_action(id: &str, params: &serde_json::Value) -> Option<Feature> {
        let kind = || {
            params
                .get("kind")
                .and_then(serde_json::Value::as_str)
                .and_then(ResourceKind::parse)
        };
        match id {
            ACTION_DEPLOY => Some(Feature::Deploy),
            ACTION_START => kind().map(Feature::Start),
            ACTION_STOP => kind().map(Feature::Stop),
            ACTION_RESTART => kind().map(Feature::Restart),
            ACTION_CANCEL_DEPLOYMENT => Some(Feature::CancelDeployment),
            _ => None,
        }
    }

    /// The feature a detail tab's view reads.
    #[must_use]
    pub fn of_view(view: &str) -> Option<Feature> {
        match view.split('/').collect::<Vec<_>>().as_slice() {
            ["deployments", _] => Some(Feature::History),
            ["application", _, "envs"] => Some(Feature::Envs),
            ["application", _, "logs"] => Some(Feature::AppLogs),
            _ => None,
        }
    }
}

/// What a version read changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VersionChange {
    /// The first version known for this instance.
    First,
    Same,
    /// A different version than before: what was learned is forgotten.
    Changed,
}

/// What one failed call says about its feature.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Learned {
    Nothing,
    /// The answer lacks what Berth reads: unsupported here.
    Unsupported,
    /// A 404 from a feature that has not worked here: unsupported when the
    /// resource it was asked about exists (a list endpoint always does).
    Missing,
    /// The run of errors from features that worked reached [`ERROR_RUN`]:
    /// read the version now, then [`Probe::classify`] the error again.
    Recheck,
}

#[derive(Debug, Default)]
pub struct Probe {
    version: Option<VersionInfo>,
    checked: Option<Instant>,
    worked: BTreeSet<Feature>,
    unsupported: BTreeMap<Feature, String>,
    error_run: u32,
}

/// A 404 or 422 from an endpoint that worked: the API may have moved.
fn is_drift(error: &ClientError) -> bool {
    matches!(
        error,
        ClientError::NotFound | ClientError::Http { status: 422, .. }
    )
}

impl Probe {
    #[must_use]
    pub fn version(&self) -> Option<&VersionInfo> {
        self.version.as_ref()
    }

    /// Whether the version should be read before the next view loads.
    #[must_use]
    pub fn version_due(&self, now: Instant, ttl: Duration) -> bool {
        self.error_run >= ERROR_RUN
            || self
                .checked
                .is_none_or(|at| now.saturating_duration_since(at) >= ttl)
    }

    /// Records a version read (`None` when it failed: the last one known
    /// stays). A different version forgets every feature learned.
    pub fn set_version(&mut self, read: Option<VersionInfo>, now: Instant) -> VersionChange {
        self.checked = Some(now);
        self.error_run = 0;
        let Some(read) = read else {
            return VersionChange::Same;
        };
        let change = match &self.version {
            None => VersionChange::First,
            Some(known) if known.raw == read.raw => VersionChange::Same,
            Some(_) => VersionChange::Changed,
        };
        if change == VersionChange::Changed {
            self.worked.clear();
            self.unsupported.clear();
        }
        self.version = Some(read);
        change
    }

    pub fn succeeded(&mut self, feature: Feature) {
        self.worked.insert(feature);
    }

    /// Counts the error toward the run when the feature worked here, then
    /// classifies it.
    pub fn failed(&mut self, feature: Feature, error: &ClientError) -> Learned {
        if self.worked.contains(&feature) && is_drift(error) {
            self.error_run += 1;
            if self.error_run >= ERROR_RUN {
                return Learned::Recheck;
            }
        }
        self.classify(feature, error)
    }

    /// What the error says about the feature, without counting it.
    #[must_use]
    pub fn classify(&self, feature: Feature, error: &ClientError) -> Learned {
        match error {
            ClientError::Decode(_) => Learned::Unsupported,
            ClientError::NotFound if !self.worked.contains(&feature) => Learned::Missing,
            _ => Learned::Nothing,
        }
    }

    /// Marks the feature unsupported; `true` when it was not already.
    pub fn mark(&mut self, feature: Feature, error: &ClientError) -> bool {
        let why = match error {
            ClientError::NotFound => "answered 404".to_owned(),
            ClientError::Decode(detail) => {
                format!("answered without what Berth reads: {detail}")
            }
            other => format!("failed: {other}"),
        };
        self.worked.remove(&feature);
        self.unsupported.insert(feature, why).is_none()
    }

    /// Why the feature is unavailable, as a sentence, when it is.
    #[must_use]
    pub fn unsupported_text(&self, feature: Feature) -> Option<String> {
        self.unsupported.get(&feature).map(|why| {
            format!(
                "This Coolify instance does not offer {} ({} {why}).",
                feature.label(),
                feature.endpoint()
            )
        })
    }

    /// The banner line naming everything unsupported, if anything is.
    #[must_use]
    pub fn summary(&self) -> Option<String> {
        if self.unsupported.is_empty() {
            return None;
        }
        let names: Vec<String> = self.unsupported.keys().map(|f| f.label()).collect();
        let mut line = String::from("Not available on this Coolify instance: ");
        let _ = write!(line, "{}.", names.join(", "));
        Some(line)
    }

    fn allowed_action(&self, action: &ActionSpec) -> bool {
        Feature::of_action(&action.id, &action.params)
            .is_none_or(|feature| !self.unsupported.contains_key(&feature))
    }

    /// Drops the buttons and tabs of unsupported features from a view.
    pub fn prune(&self, view: &mut View) {
        match view {
            View::Table { rows, .. } => {
                for row in rows {
                    row.actions.retain(|a| self.allowed_action(a));
                }
            }
            View::Detail { tabs, actions, .. } => {
                actions.retain(|a| self.allowed_action(a));
                tabs.retain(|tab| {
                    Feature::of_view(&tab.view)
                        .is_none_or(|feature| !self.unsupported.contains_key(&feature))
                });
            }
            View::Message { action, .. } => {
                if action.as_ref().is_some_and(|a| !self.allowed_action(a)) {
                    *action = None;
                }
            }
            View::LogStream { .. } | View::Form { .. } => {}
        }
    }
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;
    use serde_json::json;

    fn version(raw: &str) -> VersionInfo {
        VersionInfo {
            raw: raw.to_owned(),
            major: None,
            warning: None,
        }
    }

    #[test]
    fn probe_version_is_due_first_after_the_interval_and_after_an_error_run() {
        let ttl = Duration::from_secs(5);
        let start = Instant::now();
        let mut probe = Probe::default();
        assert!(probe.version_due(start, ttl), "never read");
        probe.set_version(Some(version("v4.5.0")), start);
        assert!(!probe.version_due(start + Duration::from_secs(4), ttl));
        assert!(probe.version_due(start + ttl, ttl));

        probe.succeeded(Feature::Servers);
        assert_eq!(
            probe.failed(Feature::Servers, &ClientError::NotFound),
            Learned::Nothing,
            "one 404 from an endpoint that worked is not yet a run"
        );
        assert_eq!(
            probe.failed(
                Feature::Servers,
                &ClientError::Http {
                    status: 422,
                    message: String::new()
                }
            ),
            Learned::Recheck
        );
        assert!(probe.version_due(start, ttl), "the run makes a read due");
    }

    #[test]
    fn probe_a_new_version_forgets_what_was_learned() {
        let now = Instant::now();
        let mut probe = Probe::default();
        assert_eq!(
            probe.set_version(Some(version("v4.5.0")), now),
            VersionChange::First
        );
        probe.mark(Feature::Deploy, &ClientError::NotFound);
        probe.succeeded(Feature::Servers);
        assert_eq!(
            probe.set_version(Some(version("v4.5.0")), now),
            VersionChange::Same
        );
        assert!(probe.summary().is_some(), "same version keeps the marks");
        assert_eq!(probe.set_version(None, now), VersionChange::Same);
        assert_eq!(probe.version().map(|v| v.raw.as_str()), Some("v4.5.0"));
        assert_eq!(
            probe.set_version(Some(version("5.0.0")), now),
            VersionChange::Changed
        );
        assert_eq!(probe.summary(), None);
        assert_eq!(
            probe.classify(Feature::Servers, &ClientError::NotFound),
            Learned::Missing,
            "after an upgrade nothing has worked yet"
        );
    }

    #[test]
    fn probe_classifies_404_by_whether_the_feature_worked_and_a_bad_shape_always() {
        let mut probe = Probe::default();
        assert_eq!(
            probe.classify(Feature::Envs, &ClientError::NotFound),
            Learned::Missing
        );
        probe.succeeded(Feature::Envs);
        assert_eq!(
            probe.classify(Feature::Envs, &ClientError::NotFound),
            Learned::Nothing,
            "a 404 from an endpoint that worked is a missing resource"
        );
        assert_eq!(
            probe.classify(
                Feature::Envs,
                &ClientError::Decode("missing field `key`".into())
            ),
            Learned::Unsupported
        );
        assert_eq!(
            probe.classify(Feature::Envs, &ClientError::TokenInvalid),
            Learned::Nothing
        );
    }

    #[test]
    fn probe_prunes_only_the_unsupported_actions_and_tabs() {
        let application: crate::types::Application =
            serde_json::from_value(json!({ "uuid": "app-1", "name": "landing" }))
                .expect("an application decodes");
        let mut view = crate::views::application_detail(&application, None);
        let mut probe = Probe::default();
        assert!(probe.mark(Feature::Deploy, &ClientError::NotFound));
        assert!(!probe.mark(Feature::Deploy, &ClientError::NotFound), "once");
        probe.mark(
            Feature::Envs,
            &ClientError::Decode("missing field `key`".into()),
        );
        probe.mark(Feature::Stop(ResourceKind::Service), &ClientError::NotFound);
        probe.prune(&mut view);
        let View::Detail { actions, tabs, .. } = view else {
            panic!("a detail");
        };
        let ids: Vec<&str> = actions.iter().map(|a| a.id.as_str()).collect();
        assert_eq!(
            ids,
            ["start", "stop", "restart"],
            "an application's Stop stays"
        );
        let tab_ids: Vec<&str> = tabs.iter().map(|t| t.id.as_str()).collect();
        assert_eq!(tab_ids, ["deployments", "logs"]);
        assert_eq!(
            probe.summary().as_deref(),
            Some("Not available on this Coolify instance: environment variables, Deploy, Stop (services).")
        );
        assert_eq!(
            probe.unsupported_text(Feature::Deploy).as_deref(),
            Some("This Coolify instance does not offer Deploy (POST /deploy answered 404).")
        );
    }

    #[test]
    fn probe_maps_actions_and_tab_views_to_features() {
        assert_eq!(
            Feature::of_action(
                "stop",
                &json!({ "uuid": "u", "kind": "standalone-postgresql" })
            ),
            Some(Feature::Stop(ResourceKind::Database))
        );
        assert_eq!(Feature::of_action("stop", &json!({ "uuid": "u" })), None);
        assert_eq!(
            Feature::of_action("open-logs", &json!({ "open": "x" })),
            None
        );
        assert_eq!(
            Feature::of_view("deployments/app-1"),
            Some(Feature::History)
        );
        assert_eq!(Feature::of_view("deployments"), None);
        assert_eq!(
            Feature::of_view("application/app-1/envs"),
            Some(Feature::Envs)
        );
    }
}
