//! Activity service (D65): every engine event and every Berth action
//! becomes an `ActivityEntry` in the store, kept 30 days (pruned at
//! start by the App root via [`prune_old`]).

/// How long activity is kept (D65).
pub const KEEP: chrono::Duration = chrono::Duration::days(30);

/// Build the entry for one engine event. `subject_name`/`project` come
/// from whatever the caller already knows (event attributes carry the
/// container name on the bollard path).
pub fn entry_for_event(
    event: &berth_core::EngineEvent,
    subject_name: String,
    project: Option<berth_core::ProjectId>,
) -> berth_core::ActivityEntry {
    let (action, subject) = match &event.kind {
        berth_core::EventKind::Container { id, action } => (
            format!("container.{action}"),
            berth_core::Subject::Container(id.clone()),
        ),
        berth_core::EventKind::Image { id, action } => (
            format!("image.{action}"),
            berth_core::Subject::Image(id.clone()),
        ),
        berth_core::EventKind::Volume { name, action } => (
            format!("volume.{action}"),
            berth_core::Subject::Volume(name.clone()),
        ),
        berth_core::EventKind::Network { id, action } => (
            format!("network.{action}"),
            berth_core::Subject::Network(id.clone()),
        ),
        berth_core::EventKind::Other { ty, action } => (
            format!("{ty}.{action}"),
            berth_core::Subject::Engine(event.engine.clone()),
        ),
    };
    berth_core::ActivityEntry {
        id: uuid::Uuid::now_v7(),
        ts: event.at,
        engine: event.engine.clone(),
        actor: berth_core::Actor::Local,
        origin: berth_core::ActivityOrigin::Engine,
        action,
        subject,
        subject_name,
        project,
        outcome: berth_core::Outcome::Ok,
    }
}

/// Build the entry for an action Berth itself took (row buttons, palette,
/// bulk bar — the actions service of P2-T5 calls this per action).
pub fn entry_for_action(
    engine: berth_core::EngineId,
    action: &str,
    subject: berth_core::Subject,
    subject_name: String,
    project: Option<berth_core::ProjectId>,
    outcome: berth_core::Outcome,
) -> berth_core::ActivityEntry {
    berth_core::ActivityEntry {
        id: uuid::Uuid::now_v7(),
        ts: chrono::Utc::now(),
        engine,
        actor: berth_core::Actor::Local,
        origin: berth_core::ActivityOrigin::Berth,
        action: action.to_string(),
        subject,
        subject_name,
        project,
        outcome,
    }
}

/// Best-effort write: activity must never fail the action it describes.
pub fn record(store: &berth_store::Store, entry: &berth_core::ActivityEntry) {
    if let Err(err) = store.activity_push(entry) {
        tracing::warn!(%err, action = %entry.action, "activity write failed");
    }
}

/// Drop entries older than [`KEEP`]. Best-effort; returns rows removed.
pub fn prune_old(store: &berth_store::Store, now: chrono::DateTime<chrono::Utc>) -> usize {
    match store.activity_prune(now - KEEP) {
        Ok(removed) => removed,
        Err(err) => {
            tracing::warn!(%err, "activity prune failed");
            0
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn container_event() -> berth_core::EngineEvent {
        berth_core::EngineEvent {
            engine: berth_core::EngineId("colima:default".to_string()),
            at: chrono::DateTime::from_timestamp(100, 0).expect("ts"),
            kind: berth_core::EventKind::Container {
                id: berth_core::ContainerId("abc123".to_string()),
                action: "die".to_string(),
            },
            attributes: std::collections::BTreeMap::new(),
        }
    }

    #[test]
    fn services_activity_event_maps_to_an_engine_entry() {
        let entry = entry_for_event(&container_event(), "web-1".to_string(), None);
        assert_eq!(entry.action, "container.die");
        assert_eq!(entry.origin, berth_core::ActivityOrigin::Engine);
        assert_eq!(entry.actor, berth_core::Actor::Local);
        assert_eq!(
            entry.subject,
            berth_core::Subject::Container(berth_core::ContainerId("abc123".to_string()))
        );
        assert_eq!(entry.subject_name, "web-1");
        assert_eq!(entry.outcome, berth_core::Outcome::Ok);
        assert_eq!(
            entry.ts,
            chrono::DateTime::from_timestamp(100, 0).expect("ts")
        );
    }

    #[test]
    fn services_activity_records_and_prunes_by_age() {
        let store = berth_store::Store::open_in_memory().expect("store");
        let now = chrono::Utc::now();
        let mut old = entry_for_event(&container_event(), "old-1".to_string(), None);
        old.ts = now - chrono::Duration::days(31);
        let mut fresh = entry_for_action(
            berth_core::EngineId("colima:default".to_string()),
            "container.restart",
            berth_core::Subject::Container(berth_core::ContainerId("abc123".to_string())),
            "web-1".to_string(),
            None,
            berth_core::Outcome::Ok,
        );
        fresh.ts = now;
        record(&store, &old);
        record(&store, &fresh);
        let all = store
            .activity_query(&berth_store::ActivityFilter::default(), None, 10)
            .expect("query");
        assert_eq!(all.len(), 2);
        assert_eq!(prune_old(&store, now), 1);
        let left = store
            .activity_query(&berth_store::ActivityFilter::default(), None, 10)
            .expect("query after prune");
        assert_eq!(left.len(), 1);
        assert_eq!(left[0].action, "container.restart");
        assert_eq!(left[0].origin, berth_core::ActivityOrigin::Berth);
    }
}
