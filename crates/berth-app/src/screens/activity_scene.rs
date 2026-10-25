//! Mock data for the Activity scenes (D58): the story of a restart that
//! failed to settle, a project stopped from the sidebar, a plugin call and
//! an engine that dropped its event stream. Times are relative to "now" so
//! the table always reads as recent.

use chrono::{DateTime, Duration, Utc};

use super::activity::{seed_feed, FeedSeed};
use berth_core::{ActivityEntry, ActivityOrigin, Actor, EngineId, Outcome, Subject};

const DEFAULT: &str = "colima:default";
const WORK: &str = "colima:work";
const HETZNER: &str = "ssh:hetzner-1";

struct Maker<'a> {
    now: DateTime<Utc>,
    world: &'a berth_engine::MockWorld,
}

impl Maker<'_> {
    fn container(&self, name: &str) -> (Subject, Option<berth_core::ProjectId>) {
        self.world
            .containers
            .iter()
            .find(|c| c.name == name)
            .map_or(
                (
                    Subject::Container(berth_core::ContainerId(name.to_string())),
                    None,
                ),
                |c| (Subject::Container(c.id.clone()), c.project.clone()),
            )
    }

    #[allow(clippy::too_many_arguments)]
    fn make(
        &self,
        ago_s: i64,
        engine: &str,
        origin: ActivityOrigin,
        action: &str,
        (subject, project): (Subject, Option<berth_core::ProjectId>),
        name: &str,
        outcome: Outcome,
    ) -> ActivityEntry {
        ActivityEntry {
            id: uuid::Uuid::now_v7(),
            ts: self.now - Duration::seconds(ago_s),
            engine: EngineId(engine.to_string()),
            actor: Actor::Local,
            origin,
            action: action.to_string(),
            subject,
            subject_name: name.to_string(),
            project,
            outcome,
        }
    }

    /// An engine event; engine events carry no project of their own.
    fn event(&self, ago_s: i64, engine: &str, action: &str, container: &str) -> ActivityEntry {
        let (subject, _) = self.container(container);
        self.make(
            ago_s,
            engine,
            ActivityOrigin::Engine,
            action,
            (subject, None),
            container,
            Outcome::Ok,
        )
    }

    fn berth(&self, ago_s: i64, action: &str, container: &str, outcome: Outcome) -> ActivityEntry {
        let subject = self.container(container);
        self.make(
            ago_s,
            DEFAULT,
            ActivityOrigin::Berth,
            action,
            subject,
            container,
            outcome,
        )
    }

    fn project(&self, ago_s: i64, action: &str, project: &str) -> ActivityEntry {
        let id = self
            .world
            .projects
            .iter()
            .find(|p| p.name == project)
            .map_or_else(
                || berth_core::ProjectId(project.to_string()),
                |p| p.id.clone(),
            );
        self.make(
            ago_s,
            DEFAULT,
            ActivityOrigin::Berth,
            action,
            (Subject::Project(id.clone()), Some(id)),
            project,
            Outcome::Ok,
        )
    }
}

fn history(m: &Maker) -> Vec<ActivityEntry> {
    let gap = crate::services::activity::entry_for_gap(
        EngineId(HETZNER.to_string()),
        m.now - Duration::seconds(7 * 60),
    );
    vec![
        m.event(2, DEFAULT, "container.start", "shop-api-worker-1"),
        m.event(3, DEFAULT, "container.die", "shop-api-worker-1"),
        m.event(3, DEFAULT, "container.kill", "shop-api-worker-1"),
        m.berth(3, "container.restart", "shop-api-worker-1", Outcome::Ok),
        m.event(
            41,
            DEFAULT,
            "container.health_status: unhealthy",
            "analytics-ingest-1",
        ),
        m.berth(
            63,
            "container.stop",
            "analytics-grafana-1",
            Outcome::Failed {
                reason: "The engine did not finish within 30 s".to_string(),
            },
        ),
        m.event(95, DEFAULT, "container.start", "shop-api-redis-1"),
        m.event(95, DEFAULT, "container.create", "shop-api-redis-1"),
        m.berth(96, "container.start", "shop-api-redis-1", Outcome::Ok),
        m.project(131, "project.stop", "berth-site"),
        m.event(132, DEFAULT, "container.stop", "berth-site-proxy-1"),
        m.event(132, DEFAULT, "container.die", "berth-site-proxy-1"),
        m.event(133, DEFAULT, "container.stop", "berth-site-web-1"),
        m.event(133, DEFAULT, "container.die", "berth-site-web-1"),
        m.make(
            188,
            DEFAULT,
            ActivityOrigin::Plugin("coolify".to_string()),
            "deploy.trigger",
            (Subject::Plugin("coolify".to_string()), None),
            "shop-api",
            Outcome::Ok,
        ),
        m.event(240, WORK, "container.start", "crm-api-1"),
        m.event(241, WORK, "container.start", "crm-db-1"),
        m.event(310, DEFAULT, "container.die", "pg-scratch"),
        m.berth(
            372,
            "container.delete",
            "old-builder-1",
            Outcome::Failed {
                reason: "Container is paused; unpause it first".to_string(),
            },
        ),
        m.event(388, DEFAULT, "container.oom", "analytics-clickhouse-1"),
        gap,
        m.event(455, HETZNER, "container.die", "landing-app-1"),
        m.event(512, DEFAULT, "container.start", "shop-api-api-1"),
        m.berth(513, "container.start", "shop-api-api-1", Outcome::Ok),
        m.event(640, DEFAULT, "container.stop", "pg-old"),
        m.berth(641, "container.stop", "pg-old", Outcome::Ok),
        m.event(902, DEFAULT, "container.start", "whoami-1"),
        m.event(1_200, DEFAULT, "container.die", "test-runner"),
        m.event(1_742, DEFAULT, "container.start", "metrics-exporter-1"),
    ]
}

/// The 23 events that arrived after the pause.
fn held(m: &Maker) -> Vec<ActivityEntry> {
    let names = [
        "shop-api-api-1",
        "shop-api-db-1",
        "analytics-ingest-1",
        "analytics-clickhouse-1",
    ];
    (0..23)
        .map(|i| {
            m.event(
                0,
                DEFAULT,
                "container.health_status: healthy",
                names[i % names.len()],
            )
        })
        .collect()
}

/// Seed the next Activity screen with the mock story; `paused` shows the
/// pause banner with 23 new events waiting behind it.
pub fn seed(world: &berth_engine::MockWorld, paused: bool) {
    let m = Maker {
        now: Utc::now(),
        world,
    };
    seed_feed(FeedSeed {
        entries: history(&m),
        held: if paused { held(&m) } else { Vec::new() },
        paused,
    });
}
