use chrono::{DateTime, Utc};
use rusqlite::OptionalExtension;
use serde::{Deserialize, Serialize};

use crate::settings::Settings;
use crate::StoreError;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UserEngine {
    pub id: berth_core::EngineId,
    pub name: String,
    pub kind: berth_core::EngineKind,
    pub added: DateTime<Utc>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EngineSnapshot {
    pub engine: berth_core::EngineId,
    pub taken: DateTime<Utc>,
    pub containers: Vec<berth_core::Container>,
    pub projects: Vec<berth_core::Project>,
}
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ActivityFilter {
    pub engine: Option<berth_core::EngineId>,
    pub project: Option<berth_core::ProjectId>,
    pub origin: Option<String>,
    pub failed_only: bool,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionRecord {
    pub session: berth_core::AssistantSession,
    pub updated: DateTime<Utc>,
}

/// One folder cloned from Git: where it is, the engine it deploys to and its
/// `ProjectSource::Git`.
pub type GitSourceRow = (
    std::path::PathBuf,
    berth_core::EngineId,
    berth_core::ProjectSource,
);

const MIGRATIONS: &[(u32, &str)] = &[
    (1, include_str!("../migrations/0001_init.sql")),
    (2, include_str!("../migrations/0002_git_sources.sql")),
];

pub struct Store {
    conn: std::sync::Mutex<rusqlite::Connection>,
}

fn sql_err(err: rusqlite::Error) -> StoreError {
    StoreError::Sql(err.to_string())
}

fn now_text() -> String {
    Utc::now().to_rfc3339()
}

fn origin_key(origin: &berth_core::ActivityOrigin) -> String {
    match origin {
        berth_core::ActivityOrigin::Engine => "engine".to_string(),
        berth_core::ActivityOrigin::Berth => "berth".to_string(),
        berth_core::ActivityOrigin::Plugin(id) => format!("plugin:{id}"),
    }
}

impl Store {
    pub fn open(path: &std::path::Path) -> Result<Self, StoreError> {
        Self::from_conn(rusqlite::Connection::open(path).map_err(sql_err)?)
    }

    pub fn open_in_memory() -> Result<Self, StoreError> {
        Self::from_conn(rusqlite::Connection::open_in_memory().map_err(sql_err)?)
    }

    fn from_conn(mut conn: rusqlite::Connection) -> Result<Self, StoreError> {
        conn.execute_batch("CREATE TABLE IF NOT EXISTS schema_version (version INTEGER NOT NULL);")
            .map_err(sql_err)?;
        let current: u32 = conn
            .query_row(
                "SELECT COALESCE(MAX(version), 0) FROM schema_version",
                [],
                |row| row.get(0),
            )
            .map_err(sql_err)?;
        for (version, sql) in MIGRATIONS {
            if *version <= current {
                continue;
            }
            let tx = conn.transaction().map_err(sql_err)?;
            tx.execute_batch(sql)
                .map_err(|e| StoreError::Migration(*version, e.to_string()))?;
            tx.execute(
                "INSERT INTO schema_version (version) VALUES (?1)",
                [version],
            )
            .map_err(|e| StoreError::Migration(*version, e.to_string()))?;
            tx.commit()
                .map_err(|e| StoreError::Migration(*version, e.to_string()))?;
        }
        Ok(Self {
            conn: std::sync::Mutex::new(conn),
        })
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, rusqlite::Connection>, StoreError> {
        self.conn
            .lock()
            .map_err(|_| StoreError::Sql("store lock poisoned".to_string()))
    }

    fn get_json<T: serde::de::DeserializeOwned>(
        &self,
        sql: &str,
        params: impl rusqlite::Params,
    ) -> Result<Option<T>, StoreError> {
        let conn = self.lock()?;
        let json: Option<String> = conn
            .query_row(sql, params, |row| row.get(0))
            .optional()
            .map_err(sql_err)?;
        match json {
            Some(json) => Ok(Some(serde_json::from_str(&json)?)),
            None => Ok(None),
        }
    }

    fn list_json<T: serde::de::DeserializeOwned>(
        &self,
        sql: &str,
        params: impl rusqlite::Params,
    ) -> Result<Vec<T>, StoreError> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare(sql).map_err(sql_err)?;
        let rows = stmt
            .query_map(params, |row| row.get::<_, String>(0))
            .map_err(sql_err)?;
        let mut out = Vec::new();
        for row in rows {
            out.push(serde_json::from_str(&row.map_err(sql_err)?)?);
        }
        Ok(out)
    }

    pub fn settings(&self) -> Result<Settings, StoreError> {
        Ok(self
            .get_json("SELECT json FROM settings WHERE key = 'settings'", [])?
            .unwrap_or_default())
    }

    pub fn set_settings(&self, settings: &Settings) -> Result<(), StoreError> {
        let conn = self.lock()?;
        conn.execute(
            "INSERT INTO settings (key, json) VALUES ('settings', ?1)
             ON CONFLICT(key) DO UPDATE SET json = excluded.json",
            [serde_json::to_string(settings)?],
        )
        .map_err(sql_err)?;
        Ok(())
    }

    pub fn pins(&self) -> Result<Vec<berth_core::PinnedFolder>, StoreError> {
        let conn = self.lock()?;
        let mut stmt = conn
            .prepare("SELECT folder, engine, added FROM pins ORDER BY added, folder")
            .map_err(sql_err)?;
        let rows = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })
            .map_err(sql_err)?;
        let mut out = Vec::new();
        for row in rows {
            let (folder, engine, added) = row.map_err(sql_err)?;
            let added = DateTime::parse_from_rfc3339(&added)
                .map(|t| t.with_timezone(&Utc))
                .map_err(|e| StoreError::Sql(format!("bad pin timestamp: {e}")))?;
            out.push(berth_core::PinnedFolder {
                folder: std::path::PathBuf::from(folder),
                engine: berth_core::EngineId(engine),
                added,
            });
        }
        Ok(out)
    }

    pub fn pin(&self, pin: &berth_core::PinnedFolder) -> Result<(), StoreError> {
        let conn = self.lock()?;
        conn.execute(
            "INSERT INTO pins (folder, engine, added) VALUES (?1, ?2, ?3)
             ON CONFLICT(folder) DO UPDATE SET engine = excluded.engine, added = excluded.added",
            rusqlite::params![
                pin.folder.to_string_lossy().into_owned(),
                pin.engine.0,
                pin.added.to_rfc3339()
            ],
        )
        .map_err(sql_err)?;
        Ok(())
    }

    pub fn unpin(&self, folder: &std::path::Path) -> Result<(), StoreError> {
        let conn = self.lock()?;
        conn.execute(
            "DELETE FROM pins WHERE folder = ?1",
            [folder.to_string_lossy()],
        )
        .map_err(sql_err)?;
        Ok(())
    }

    pub fn recent_folders(&self, limit: usize) -> Result<Vec<std::path::PathBuf>, StoreError> {
        let conn = self.lock()?;
        let mut stmt = conn
            .prepare("SELECT folder FROM recent ORDER BY at DESC, rowid DESC LIMIT ?1")
            .map_err(sql_err)?;
        let rows = stmt
            .query_map([limit as i64], |row| row.get::<_, String>(0))
            .map_err(sql_err)?;
        let mut out = Vec::new();
        for row in rows {
            out.push(std::path::PathBuf::from(row.map_err(sql_err)?));
        }
        Ok(out)
    }

    pub fn push_recent(&self, folder: &std::path::Path) -> Result<(), StoreError> {
        let conn = self.lock()?;
        conn.execute(
            "INSERT INTO recent (folder, at) VALUES (?1, ?2)
             ON CONFLICT(folder) DO UPDATE SET at = excluded.at",
            rusqlite::params![folder.to_string_lossy().into_owned(), now_text()],
        )
        .map_err(sql_err)?;
        Ok(())
    }

    pub fn snapshot(
        &self,
        engine: &berth_core::EngineId,
    ) -> Result<Option<EngineSnapshot>, StoreError> {
        self.get_json("SELECT json FROM snapshots WHERE engine = ?1", [&engine.0])
    }

    /// Every stored engine snapshot, ordered by engine id.
    pub fn snapshots(&self) -> Result<Vec<EngineSnapshot>, StoreError> {
        let conn = self.lock()?;
        let mut stmt = conn
            .prepare("SELECT json FROM snapshots ORDER BY engine")
            .map_err(sql_err)?;
        let rows = stmt
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(sql_err)?;
        let mut out = Vec::new();
        for row in rows {
            out.push(serde_json::from_str(&row.map_err(sql_err)?)?);
        }
        Ok(out)
    }

    pub fn save_snapshot(&self, snapshot: &EngineSnapshot) -> Result<(), StoreError> {
        let conn = self.lock()?;
        conn.execute(
            "INSERT INTO snapshots (engine, taken, json) VALUES (?1, ?2, ?3)
             ON CONFLICT(engine) DO UPDATE SET taken = excluded.taken, json = excluded.json",
            rusqlite::params![
                snapshot.engine.0,
                snapshot.taken.to_rfc3339(),
                serde_json::to_string(snapshot)?
            ],
        )
        .map_err(sql_err)?;
        Ok(())
    }

    pub fn activity_push(&self, entry: &berth_core::ActivityEntry) -> Result<(), StoreError> {
        let outcome = match &entry.outcome {
            berth_core::Outcome::Ok => "ok",
            berth_core::Outcome::Failed { .. } => "failed",
        };
        let conn = self.lock()?;
        conn.execute(
            "INSERT INTO activity (id, ts, engine, project, origin, action, outcome, json)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            rusqlite::params![
                entry.id.to_string(),
                entry.ts.to_rfc3339(),
                entry.engine.0,
                entry.project.as_ref().map(|p| &p.0),
                origin_key(&entry.origin),
                entry.action,
                outcome,
                serde_json::to_string(entry)?
            ],
        )
        .map_err(sql_err)?;
        Ok(())
    }

    pub fn activity_query(
        &self,
        filter: &ActivityFilter,
        before: Option<DateTime<Utc>>,
        limit: usize,
    ) -> Result<Vec<berth_core::ActivityEntry>, StoreError> {
        let mut sql = "SELECT json FROM activity WHERE 1 = 1".to_string();
        let mut params: Vec<rusqlite::types::Value> = Vec::new();
        if let Some(engine) = &filter.engine {
            params.push(engine.0.clone().into());
            sql += &format!(" AND engine = ?{}", params.len());
        }
        if let Some(project) = &filter.project {
            params.push(project.0.clone().into());
            sql += &format!(" AND project = ?{}", params.len());
        }
        if let Some(origin) = &filter.origin {
            params.push(origin.clone().into());
            sql += &format!(" AND origin = ?{}", params.len());
        }
        if filter.failed_only {
            sql += " AND outcome = 'failed'";
        }
        if let Some(before) = before {
            params.push(before.to_rfc3339().into());
            sql += &format!(" AND ts < ?{}", params.len());
        }
        params.push((limit as i64).into());
        sql += &format!(" ORDER BY ts DESC, rowid DESC LIMIT ?{}", params.len());
        self.list_json(&sql, rusqlite::params_from_iter(params))
    }

    pub fn activity_prune(&self, older_than: DateTime<Utc>) -> Result<usize, StoreError> {
        let conn = self.lock()?;
        let removed = conn
            .execute(
                "DELETE FROM activity WHERE ts < ?1",
                [older_than.to_rfc3339()],
            )
            .map_err(sql_err)?;
        Ok(removed)
    }

    pub fn user_engines(&self) -> Result<Vec<UserEngine>, StoreError> {
        self.list_json("SELECT json FROM engines ORDER BY id", [])
    }

    pub fn upsert_engine(&self, engine: &UserEngine) -> Result<(), StoreError> {
        let conn = self.lock()?;
        conn.execute(
            "INSERT INTO engines (id, json) VALUES (?1, ?2)
             ON CONFLICT(id) DO UPDATE SET json = excluded.json",
            rusqlite::params![engine.id.0, serde_json::to_string(engine)?],
        )
        .map_err(sql_err)?;
        Ok(())
    }

    pub fn remove_engine(&self, id: &berth_core::EngineId) -> Result<(), StoreError> {
        let conn = self.lock()?;
        conn.execute("DELETE FROM engines WHERE id = ?1", [&id.0])
            .map_err(sql_err)?;
        Ok(())
    }

    pub fn custom_templates(&self) -> Result<Vec<berth_core::Template>, StoreError> {
        self.list_json("SELECT json FROM templates ORDER BY id", [])
    }

    pub fn save_template(&self, template: &berth_core::Template) -> Result<(), StoreError> {
        let conn = self.lock()?;
        conn.execute(
            "INSERT INTO templates (id, json) VALUES (?1, ?2)
             ON CONFLICT(id) DO UPDATE SET json = excluded.json",
            rusqlite::params![template.id, serde_json::to_string(template)?],
        )
        .map_err(sql_err)?;
        Ok(())
    }

    pub fn delete_template(&self, id: &str) -> Result<(), StoreError> {
        let conn = self.lock()?;
        conn.execute("DELETE FROM templates WHERE id = ?1", [id])
            .map_err(sql_err)?;
        Ok(())
    }

    /// The folders cloned from Git (P8-T5): `(folder, engine, source)` with
    /// `source` always a `ProjectSource::Git`, ordered by folder.
    pub fn git_sources(&self) -> Result<Vec<GitSourceRow>, StoreError> {
        let conn = self.lock()?;
        let mut stmt = conn
            .prepare("SELECT folder, engine, json FROM git_sources ORDER BY folder")
            .map_err(sql_err)?;
        let rows = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })
            .map_err(sql_err)?;
        let mut out = Vec::new();
        for row in rows {
            let (folder, engine, json) = row.map_err(sql_err)?;
            out.push((
                std::path::PathBuf::from(folder),
                berth_core::EngineId(engine),
                serde_json::from_str(&json)?,
            ));
        }
        Ok(out)
    }

    /// Record (or update) a cloned folder's Git source.
    pub fn set_git_source(
        &self,
        folder: &std::path::Path,
        engine: &berth_core::EngineId,
        source: &berth_core::ProjectSource,
    ) -> Result<(), StoreError> {
        let conn = self.lock()?;
        conn.execute(
            "INSERT INTO git_sources (folder, engine, json) VALUES (?1, ?2, ?3)
             ON CONFLICT(folder) DO UPDATE SET engine = excluded.engine, json = excluded.json",
            rusqlite::params![
                folder.to_string_lossy().into_owned(),
                engine.0,
                serde_json::to_string(source)?
            ],
        )
        .map_err(sql_err)?;
        Ok(())
    }

    /// Forget a folder's Git source (the folder itself is not touched).
    pub fn remove_git_source(&self, folder: &std::path::Path) -> Result<(), StoreError> {
        let conn = self.lock()?;
        conn.execute(
            "DELETE FROM git_sources WHERE folder = ?1",
            [folder.to_string_lossy()],
        )
        .map_err(sql_err)?;
        Ok(())
    }

    pub fn registries(&self) -> Result<Vec<berth_core::Registry>, StoreError> {
        self.list_json("SELECT json FROM registries ORDER BY id", [])
    }

    pub fn upsert_registry(&self, registry: &berth_core::Registry) -> Result<(), StoreError> {
        let conn = self.lock()?;
        conn.execute(
            "INSERT INTO registries (id, json) VALUES (?1, ?2)
             ON CONFLICT(id) DO UPDATE SET json = excluded.json",
            rusqlite::params![registry.id, serde_json::to_string(registry)?],
        )
        .map_err(sql_err)?;
        Ok(())
    }

    pub fn remove_registry(&self, id: &str) -> Result<(), StoreError> {
        let conn = self.lock()?;
        conn.execute("DELETE FROM registries WHERE id = ?1", [id])
            .map_err(sql_err)?;
        Ok(())
    }

    pub fn sessions(
        &self,
        project: Option<&berth_core::ProjectId>,
    ) -> Result<Vec<SessionRecord>, StoreError> {
        match project {
            Some(project) => self.list_json(
                "SELECT json FROM sessions WHERE project = ?1 ORDER BY updated DESC",
                [&project.0],
            ),
            None => self.list_json("SELECT json FROM sessions ORDER BY updated DESC", []),
        }
    }

    pub fn save_session(&self, session: &berth_core::AssistantSession) -> Result<(), StoreError> {
        let record = SessionRecord {
            session: session.clone(),
            updated: Utc::now(),
        };
        let conn = self.lock()?;
        conn.execute(
            "INSERT INTO sessions (id, project, updated, json) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(id) DO UPDATE SET project = excluded.project,
                updated = excluded.updated, json = excluded.json",
            rusqlite::params![
                session.id.0.to_string(),
                session.project.as_ref().map(|p| &p.0),
                record.updated.to_rfc3339(),
                serde_json::to_string(&record)?
            ],
        )
        .map_err(sql_err)?;
        Ok(())
    }

    pub fn plugin_config(&self, plugin: &str) -> Result<Option<serde_json::Value>, StoreError> {
        self.get_json("SELECT json FROM plugins WHERE id = ?1", [plugin])
    }

    pub fn set_plugin_config(
        &self,
        plugin: &str,
        value: &serde_json::Value,
    ) -> Result<(), StoreError> {
        let conn = self.lock()?;
        conn.execute(
            "INSERT INTO plugins (id, json) VALUES (?1, ?2)
             ON CONFLICT(id) DO UPDATE SET json = excluded.json",
            rusqlite::params![plugin, serde_json::to_string(value)?],
        )
        .map_err(sql_err)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(secs: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(secs, 0).expect("ts")
    }
    fn engine(id: &str) -> berth_core::EngineId {
        berth_core::EngineId(id.to_string())
    }
    fn activity(id: u128, ts: i64, failed: bool) -> berth_core::ActivityEntry {
        berth_core::ActivityEntry {
            id: uuid::Uuid::from_u128(id),
            ts: at(ts),
            engine: engine("colima:default"),
            actor: berth_core::Actor::Local,
            origin: berth_core::ActivityOrigin::Berth,
            action: "container.restart".to_string(),
            subject: berth_core::Subject::Volume("data".to_string()),
            subject_name: "data".to_string(),
            project: Some(berth_core::ProjectId("p1".to_string())),
            outcome: if failed {
                berth_core::Outcome::Failed {
                    reason: "no".to_string(),
                }
            } else {
                berth_core::Outcome::Ok
            },
        }
    }

    #[test]
    fn settings_default_then_round_trip_and_update() {
        let store = Store::open_in_memory().expect("open");
        assert_eq!(store.settings().expect("default"), Settings::default());
        let changed = Settings {
            theme: crate::settings::Theme::Dark,
            ..Settings::default()
        };
        store.set_settings(&changed).expect("set");
        assert_eq!(store.settings().expect("get"), changed);
    }

    #[test]
    fn settings_text_scale_survives_a_save_and_load() {
        let store = Store::open_in_memory().expect("open");
        let changed = Settings {
            text_scale: 1.3,
            ..Settings::default()
        };
        store.set_settings(&changed).expect("set");
        assert!((store.settings().expect("get").text_scale - 1.3).abs() < f32::EPSILON);
    }

    #[test]
    fn opening_twice_runs_migrations_once_and_keeps_data() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("berth.db");
        {
            let store = Store::open(&path).expect("first open");
            let changed = Settings {
                update_check: false,
                ..Settings::default()
            };
            store.set_settings(&changed).expect("set");
        }
        let store = Store::open(&path).expect("second open");
        assert!(!store.settings().expect("get").update_check);
        let store = Store::open(&path).expect("third open still fine");
        assert!(!store.settings().expect("get").update_check);
    }

    #[test]
    fn pins_crud_and_order() {
        let store = Store::open_in_memory().expect("open");
        let pin_b = berth_core::PinnedFolder {
            folder: "/x/b".into(),
            engine: engine("colima:default"),
            added: at(200),
        };
        let pin_a = berth_core::PinnedFolder {
            folder: "/x/a".into(),
            engine: engine("colima:work"),
            added: at(100),
        };
        store.pin(&pin_b).expect("pin b");
        store.pin(&pin_a).expect("pin a");
        assert_eq!(store.pins().expect("pins"), vec![pin_a.clone(), pin_b]);
        store.unpin(std::path::Path::new("/x/b")).expect("unpin");
        assert_eq!(store.pins().expect("pins"), vec![pin_a]);
    }

    #[test]
    fn recent_folders_most_recent_first_with_limit() {
        let store = Store::open_in_memory().expect("open");
        for name in ["a", "b", "c", "a"] {
            store
                .push_recent(std::path::Path::new(&format!("/x/{name}")))
                .expect("push");
        }
        let got: Vec<String> = store
            .recent_folders(2)
            .expect("recent")
            .iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect();
        assert_eq!(got, ["/x/a", "/x/c"]);
    }

    #[test]
    fn snapshot_round_trip_and_overwrite() {
        let store = Store::open_in_memory().expect("open");
        assert_eq!(
            store.snapshot(&engine("colima:default")).expect("none"),
            None
        );
        let snap = EngineSnapshot {
            engine: engine("colima:default"),
            taken: at(500),
            containers: vec![],
            projects: vec![],
        };
        store.save_snapshot(&snap).expect("save");
        assert_eq!(
            store.snapshot(&engine("colima:default")).expect("get"),
            Some(snap.clone())
        );
        let newer = EngineSnapshot {
            taken: at(600),
            ..snap
        };
        store.save_snapshot(&newer).expect("overwrite");
        assert_eq!(
            store
                .snapshot(&engine("colima:default"))
                .expect("get")
                .map(|s| s.taken),
            Some(at(600))
        );
    }

    #[test]
    fn snapshots_returns_every_engine_and_the_latest_save_of_each() {
        let store = Store::open_in_memory().expect("open");
        assert_eq!(store.snapshots().expect("empty"), vec![]);
        let snap = |id: &str, secs: i64| EngineSnapshot {
            engine: engine(id),
            taken: at(secs),
            containers: vec![],
            projects: vec![],
        };
        store.save_snapshot(&snap("remote:b", 100)).expect("save");
        store
            .save_snapshot(&snap("colima:default", 200))
            .expect("save");
        store
            .save_snapshot(&snap("colima:default", 300))
            .expect("save");
        assert_eq!(
            store.snapshots().expect("all"),
            vec![snap("colima:default", 300), snap("remote:b", 100)]
        );
    }

    #[test]
    fn activity_query_orders_newest_first_and_filters() {
        let store = Store::open_in_memory().expect("open");
        store.activity_push(&activity(1, 100, false)).expect("push");
        store.activity_push(&activity(2, 300, true)).expect("push");
        store.activity_push(&activity(3, 200, false)).expect("push");
        let all = store
            .activity_query(&ActivityFilter::default(), None, 10)
            .expect("query");
        let stamps: Vec<i64> = all.iter().map(|e| e.ts.timestamp()).collect();
        assert_eq!(stamps, [300, 200, 100]);
        let failed = store
            .activity_query(
                &ActivityFilter {
                    failed_only: true,
                    ..Default::default()
                },
                None,
                10,
            )
            .expect("failed only");
        assert_eq!(failed.len(), 1);
        assert_eq!(failed[0].ts.timestamp(), 300);
        let wrong_engine = store
            .activity_query(
                &ActivityFilter {
                    engine: Some(engine("colima:work")),
                    ..Default::default()
                },
                None,
                10,
            )
            .expect("engine filter");
        assert!(wrong_engine.is_empty());
        let before = store
            .activity_query(&ActivityFilter::default(), Some(at(250)), 10)
            .expect("before");
        assert_eq!(before.len(), 2);
        let limited = store
            .activity_query(&ActivityFilter::default(), None, 1)
            .expect("limit");
        assert_eq!(limited.len(), 1);
    }

    #[test]
    fn activity_prune_removes_old_rows_only() {
        let store = Store::open_in_memory().expect("open");
        store.activity_push(&activity(1, 100, false)).expect("push");
        store.activity_push(&activity(2, 300, false)).expect("push");
        assert_eq!(store.activity_prune(at(200)).expect("prune"), 1);
        let left = store
            .activity_query(&ActivityFilter::default(), None, 10)
            .expect("query");
        assert_eq!(left.len(), 1);
        assert_eq!(left[0].ts.timestamp(), 300);
        assert_eq!(store.activity_prune(at(200)).expect("prune again"), 0);
    }

    #[test]
    fn user_engines_crud() {
        let store = Store::open_in_memory().expect("open");
        let e = UserEngine {
            id: engine("ssh:prod"),
            name: "prod".to_string(),
            kind: berth_core::EngineKind::Remote {
                transport: berth_core::RemoteTransport::Ssh {
                    host: "prod".to_string(),
                    user: None,
                    port: None,
                },
            },
            added: at(10),
        };
        store.upsert_engine(&e).expect("upsert");
        assert_eq!(store.user_engines().expect("list"), vec![e.clone()]);
        let renamed = UserEngine {
            name: "prod-2".to_string(),
            ..e
        };
        store.upsert_engine(&renamed).expect("upsert again");
        assert_eq!(store.user_engines().expect("list"), vec![renamed]);
        store.remove_engine(&engine("ssh:prod")).expect("remove");
        assert!(store.user_engines().expect("empty").is_empty());
    }

    #[test]
    fn templates_and_registries_crud() {
        let store = Store::open_in_memory().expect("open");
        let template = berth_core::Template {
            id: "mine".to_string(),
            name: "Mine".to_string(),
            kind: berth_core::TemplateKind::Custom,
            service: "image: x".to_string(),
            env: vec![],
            inputs: vec![],
            category: "Custom".to_string(),
            image: "x".to_string(),
            description: String::new(),
        };
        store.save_template(&template).expect("save template");
        assert_eq!(store.custom_templates().expect("list"), vec![template]);
        store.delete_template("mine").expect("delete template");
        assert!(store.custom_templates().expect("empty").is_empty());

        let registry = berth_core::Registry {
            id: "local".to_string(),
            name: "Local".to_string(),
            url: url::Url::parse("http://127.0.0.1:56500").expect("url"),
            username: Some("berthfx".to_string()),
            status: berth_core::RegistryStatus::LoggedIn,
        };
        store.upsert_registry(&registry).expect("upsert registry");
        assert_eq!(store.registries().expect("list"), vec![registry]);
        store.remove_registry("local").expect("remove registry");
        assert!(store.registries().expect("empty").is_empty());
    }

    #[test]
    fn sessions_save_list_and_project_filter() {
        let store = Store::open_in_memory().expect("open");
        let project = berth_core::ProjectId("p1".to_string());
        let session = berth_core::AssistantSession {
            id: berth_core::SessionId(uuid::Uuid::nil()),
            harness: berth_core::HarnessId("muse".to_string()),
            harness_session: None,
            project: Some(project.clone()),
            context: vec![],
            turns: vec![],
            state: berth_core::SessionState::Idle,
            watch: false,
        };
        store.save_session(&session).expect("save");
        let records = store.sessions(Some(&project)).expect("by project");
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].session, session);
        assert!(store
            .sessions(Some(&berth_core::ProjectId("other".to_string())))
            .expect("none")
            .is_empty());
        assert_eq!(store.sessions(None).expect("all").len(), 1);
    }

    #[test]
    fn git_sources_round_trip_update_and_remove() {
        let dir = tempfile::tempdir().expect("dir");
        let path = dir.path().join("berth.db");
        let engine = berth_core::EngineId("colima:default".to_string());
        let folder = std::path::Path::new("/Users/me/Dev/app");
        let source = |commit: &str, auto: bool| berth_core::ProjectSource::Git {
            repo: url::Url::parse("ssh://git@example.com/me/app.git").expect("url"),
            branch: "main".to_string(),
            last_commit: Some(commit.to_string()),
            poll: Some(std::time::Duration::from_secs(300)),
            auto_redeploy: auto,
        };
        {
            let store = Store::open(&path).expect("open");
            store
                .set_git_source(folder, &engine, &source("abc1234", false))
                .expect("set");
            // The same folder again updates the row.
            store
                .set_git_source(folder, &engine, &source("def5678", true))
                .expect("update");
        }
        // It survives a reopen (the migration is idempotent on an existing file).
        let store = Store::open(&path).expect("reopen");
        assert_eq!(
            store.git_sources().expect("list"),
            vec![(folder.to_path_buf(), engine, source("def5678", true))]
        );
        store.remove_git_source(folder).expect("remove");
        assert!(store.git_sources().expect("empty").is_empty());
    }

    #[test]
    fn empty_store_lists_are_empty() {
        let store = Store::open_in_memory().expect("open");
        assert!(store.pins().expect("pins").is_empty());
        assert!(store.recent_folders(5).expect("recent").is_empty());
        assert!(store.user_engines().expect("engines").is_empty());
        assert!(store.custom_templates().expect("templates").is_empty());
        assert!(store.registries().expect("registries").is_empty());
        assert!(store.git_sources().expect("git sources").is_empty());
        assert!(store.sessions(None).expect("sessions").is_empty());
        assert!(store
            .activity_query(&ActivityFilter::default(), None, 5)
            .expect("activity")
            .is_empty());
    }

    #[test]
    fn activity_origin_and_project_filters() {
        let store = Store::open_in_memory().expect("open");
        store.activity_push(&activity(1, 100, false)).expect("push");
        let mut plugin_entry = activity(2, 200, false);
        plugin_entry.origin = berth_core::ActivityOrigin::Plugin("coolify".to_string());
        plugin_entry.project = None;
        store.activity_push(&plugin_entry).expect("push plugin");
        let by_origin = store
            .activity_query(
                &ActivityFilter {
                    origin: Some("plugin:coolify".to_string()),
                    ..Default::default()
                },
                None,
                10,
            )
            .expect("origin filter");
        assert_eq!(by_origin.len(), 1);
        assert_eq!(by_origin[0].ts.timestamp(), 200);
        let by_project = store
            .activity_query(
                &ActivityFilter {
                    project: Some(berth_core::ProjectId("p1".to_string())),
                    ..Default::default()
                },
                None,
                10,
            )
            .expect("project filter");
        assert_eq!(by_project.len(), 1);
        assert_eq!(by_project[0].ts.timestamp(), 100);
    }

    #[test]
    fn plugin_config_round_trip() {
        let store = Store::open_in_memory().expect("open");
        assert_eq!(store.plugin_config("coolify").expect("none"), None);
        let value = serde_json::json!({"url": "https://example.test", "team": 1});
        store.set_plugin_config("coolify", &value).expect("set");
        assert_eq!(store.plugin_config("coolify").expect("get"), Some(value));
    }
}
