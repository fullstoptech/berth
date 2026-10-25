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

/// The action of a gap marker: the engine's event stream ended, so events
/// between this entry and the reconnect are missing (D65, P2-T10).
pub const GAP_ACTION: &str = "engine.disconnected";

/// Build the gap marker for an engine whose event stream ended at `at`.
pub fn entry_for_gap(
    engine: berth_core::EngineId,
    at: chrono::DateTime<chrono::Utc>,
) -> berth_core::ActivityEntry {
    berth_core::ActivityEntry {
        id: uuid::Uuid::now_v7(),
        ts: at,
        subject: berth_core::Subject::Engine(engine.clone()),
        subject_name: engine.0.clone(),
        engine,
        actor: berth_core::Actor::Local,
        origin: berth_core::ActivityOrigin::Engine,
        action: GAP_ACTION.to_string(),
        project: None,
        outcome: berth_core::Outcome::Ok,
    }
}

/// Every entry that reached the store, for the live Activity feed. A
/// process-wide tap keeps `record` (the single write path) unchanged for
/// its callers; a slow receiver gets `Lagged` and reloads from the store.
fn tap() -> &'static tokio::sync::broadcast::Sender<berth_core::ActivityEntry> {
    static TAP: std::sync::OnceLock<tokio::sync::broadcast::Sender<berth_core::ActivityEntry>> =
        std::sync::OnceLock::new();
    TAP.get_or_init(|| tokio::sync::broadcast::channel(1024).0)
}

/// Follow entries as they are recorded. Subscribe before loading history
/// so nothing falls between the two (the feed de-duplicates by id).
pub fn subscribe() -> tokio::sync::broadcast::Receiver<berth_core::ActivityEntry> {
    tap().subscribe()
}

/// Best-effort write: activity must never fail the action it describes.
pub fn record(store: &berth_store::Store, entry: &berth_core::ActivityEntry) {
    match store.activity_push(entry) {
        // No receiver is the normal case (Activity screen closed).
        Ok(()) => drop(tap().send(entry.clone())),
        Err(err) => tracing::warn!(%err, action = %entry.action, "activity write failed"),
    }
}

/// The newest `limit` entries, newest first. Best-effort: a failed read
/// is an empty feed, not a crash. Blocking SQLite call: run it off the UI
/// thread.
pub fn load_recent(store: &berth_store::Store, limit: usize) -> Vec<berth_core::ActivityEntry> {
    match store.activity_query(&berth_store::ActivityFilter::default(), None, limit) {
        Ok(entries) => entries,
        Err(err) => {
            tracing::warn!(%err, "activity read failed");
            Vec::new()
        }
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
    fn services_activity_recorded_entries_reach_subscribers_and_the_store() {
        let store = berth_store::Store::open_in_memory().expect("store");
        let mut rx = subscribe();
        let entry = entry_for_event(&container_event(), "tap-web-1".to_string(), None);
        record(&store, &entry);
        let mut seen = None;
        while let Ok(got) = rx.try_recv() {
            if got.id == entry.id {
                seen = Some(got);
            }
        }
        assert_eq!(seen.as_ref(), Some(&entry), "tap delivered the entry");
        assert_eq!(load_recent(&store, 10), vec![entry]);
    }

    #[test]
    fn services_activity_gap_entry_marks_the_engine_without_failing() {
        let engine = berth_core::EngineId("ssh:hetzner-1".to_string());
        let at = chrono::DateTime::from_timestamp(500, 0).expect("ts");
        let gap = entry_for_gap(engine.clone(), at);
        assert_eq!(gap.action, GAP_ACTION);
        assert_eq!(gap.origin, berth_core::ActivityOrigin::Engine);
        assert_eq!(gap.subject, berth_core::Subject::Engine(engine.clone()));
        assert_eq!(gap.engine, engine);
        assert_eq!(gap.ts, at);
        assert_eq!(gap.outcome, berth_core::Outcome::Ok);
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
