//! The data behind the frames "Logs / errors / 1440" and "Logs / paused / 1440": five `shop-api`
//! containers, a worker that cannot reach its database, and a view that has already searched for
//! ECONNREFUSED. Used by the scenes (`scenes.rs`) and the tests.

use std::collections::{BTreeMap, BTreeSet};

use berth_core::{ContainerId, LogLine, Stream};
use berth_logs::{LogView, Query};
use chrono::{DateTime, Duration, TimeZone, Utc};

use crate::ui::log_toolbar::{build_filter, StreamChoice, TimeRange};
use crate::ui::log_view::{Span, ViewPrefs};

/// The five containers in picker order; their ids sort in this order, so the series colours run
/// 1..=5 as the frames show them.
pub const NAMES: [&str; 5] = [
    "shop-api-api-1",
    "shop-api-worker-1",
    "shop-api-db-1",
    "shop-api-redis-1",
    "shop-api-mailpit-1",
];

const API: usize = 0;
const WORKER: usize = 1;
const DB: usize = 2;
const REDIS: usize = 3;
const MAILPIT: usize = 4;

/// Frames of the trace under each `ECONNREFUSED` line.
const TRACE: [&str; 11] = [
    "    at TCPConnectWrap.afterConnect [as oncomplete] (node:net:1611:16)",
    "    at Object.connect (node:net:1139:12)",
    "    at Pool.connect (/app/worker/db.js:41:19)",
    "    at Pool.acquire (/app/worker/db.js:58:12)",
    "    at Queue.next (/app/worker/queue.js:77:9)",
    "    at Queue.poll (/app/worker/queue.js:52:14)",
    "    at Worker.tick (/app/worker/index.js:31:7)",
    "    at Timeout._onTimeout (/app/worker/index.js:18:5)",
    "    at listOnTimeout (node:internal/timers:573:17)",
    "    at process.processTimers (node:internal/timers:514:7)",
    "    at process.processTicksAndRejections (node:internal/process/task_queues:95:5)",
];

const CONNECTING: &str = "worker: connecting to postgres://shop@db:5432/shop";
const ECONNREFUSED: &str = "Error: connect ECONNREFUSED 172.19.0.3:5432";

/// `(ms after 14:02:10.000, container index, stderr?, text, trace lines)`.
fn script() -> Vec<(i64, usize, bool, String, usize)> {
    let json = |msg: &str, status: u16, ms: &str| {
        format!(r#"{{"level":"info","msg":"{msg}","status":{status},"ms":{ms}}}"#)
    };
    let plain = |at: i64, who: usize, text: &str| (at, who, false, text.to_string(), 0);
    vec![
        (
            402,
            DB,
            true,
            "Error response from daemon: Bind for 0.0.0.0:5432 failed: port is already allocated"
                .to_string(),
            0,
        ),
        plain(1_204, API, "Server listening on http://0.0.0.0:3000"),
        plain(
            1_388,
            REDIS,
            "1:M 24 Oct 2026 14:02:11.388 * Ready to accept connections tcp",
        ),
        plain(2_017, WORKER, CONNECTING),
        // The open trace of the frames: three frames shown (the frames add a fourth row, "8 more
        // lines", that a trace folded whole does not have).
        (2_031, WORKER, true, ECONNREFUSED.to_string(), 3),
        (2_540, API, false, json("GET /health", 200, "2.1"), 0),
        plain(
            3_002,
            MAILPIT,
            "[http] accessible via http://localhost:8025/",
        ),
        plain(
            4_118,
            API,
            r#"{"level":"warn","msg":"cache miss storm","keys":412}"#,
        ),
        plain(5_090, WORKER, CONNECTING),
        (5_104, WORKER, true, ECONNREFUSED.to_string(), TRACE.len()),
        (5_640, API, false, json("POST /cart/items", 201, "14.8"), 0),
        plain(
            6_877,
            REDIS,
            "1:M 24 Oct 2026 14:02:16.877 * 1 changes in 3600 seconds. Saving...",
        ),
        (7_512, API, false, json("GET /health", 200, "1.9"), 0),
        plain(8_088, WORKER, CONNECTING),
        (8_101, WORKER, true, ECONNREFUSED.to_string(), TRACE.len()),
        (8_640, API, false, json("GET /cart", 200, "6.3"), 0),
        plain(9_201, MAILPIT, "[http] GET /api/v1/messages 200"),
        (10_015, API, false, json("GET /health", 200, "2.0"), 0),
        plain(11_074, WORKER, CONNECTING),
        (11_087, WORKER, true, ECONNREFUSED.to_string(), TRACE.len()),
        (11_530, API, false, json("GET /health", 200, "2.2"), 0),
        plain(
            12_119,
            REDIS,
            "1:M 24 Oct 2026 14:02:22.119 * Background saving started by pid 29",
        ),
        (12_402, API, false, json("POST /cart/items", 201, "11.7"), 0),
    ]
}

/// 14:02:10 on the day of the frames.
fn base() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 24, 14, 2, 10)
        .single()
        .unwrap_or_default()
}

/// The ids the scene gives the five containers: the mock world's own id behind an index, so the
/// ids sort in [`NAMES`] order.
#[must_use]
pub fn scene_id(index: usize, original: &str) -> ContainerId {
    ContainerId(format!("{}-{original}", index + 1))
}

/// The frame's lines, container by index into `ids`, in time order.
#[must_use]
pub fn lines(ids: &[ContainerId]) -> Vec<LogLine> {
    let mut out = Vec::new();
    for (at, who, stderr, text, trace) in script() {
        let Some(container) = ids.get(who) else {
            continue;
        };
        let ts = base() + Duration::milliseconds(at);
        let stream = if stderr {
            Stream::Stderr
        } else {
            Stream::Stdout
        };
        out.push(LogLine {
            container: container.clone(),
            ts,
            stream,
            text,
        });
        for frame in TRACE.iter().take(trace) {
            out.push(LogLine {
                container: container.clone(),
                ts,
                stream,
                text: (*frame).to_string(),
            });
        }
    }
    out
}

/// What the scene's view is made of.
pub struct SceneView {
    pub view: LogView,
    pub prefs: ViewPrefs,
}

/// The view of the frames: the search for ECONNREFUSED already stepped to its last match
/// ("4 of 4"), "Last 15 min" chosen, the first trace open. `paused` turns Follow off (the
/// "142 new lines" pill is the caller's: it lives in the session, not the view); otherwise the
/// first and second errors of the worker are selected.
#[must_use]
pub fn scene_view(ids: &[ContainerId], names: &[String], paused: bool) -> SceneView {
    let name_map: BTreeMap<ContainerId, String> =
        ids.iter().cloned().zip(names.iter().cloned()).collect();
    let mut view = LogView::new(1_000, name_map);
    view.push_batch(lines(ids));
    let last = base() + Duration::milliseconds(12_402 + 1_000);
    view.set_filter(build_filter(
        &BTreeSet::new(),
        StreamChoice::All,
        TimeRange::Min15,
        last,
    ));
    // Open the first trace: the row of the first ECONNREFUSED.
    let first_head = view
        .rows(0..view.len())
        .iter()
        .find(|row| row.text.starts_with("Error: connect"))
        .map(|row| row.seq);
    if let Some(seq) = first_head {
        view.toggle_fold(seq);
    }
    let _ = view.set_search(Some(Query {
        text: "ECONNREFUSED".to_string(),
        regex: false,
        case_sensitive: false,
    }));
    // From nothing, "previous" lands on the last match: "4 of 4".
    let _ = view.prev_match();
    view.set_follow(!paused);
    let selection: Vec<Span> = if paused {
        Vec::new()
    } else {
        let heads: Vec<u64> = view
            .rows(0..view.len())
            .iter()
            .filter(|row| row.text.starts_with("Error: connect"))
            .map(|row| row.seq)
            .take(2)
            .collect();
        heads.into_iter().map(|seq| (seq, seq)).collect()
    };
    SceneView {
        view,
        prefs: ViewPrefs {
            query: "ECONNREFUSED".to_string(),
            range: TimeRange::Min15,
            selection,
            ..ViewPrefs::default()
        },
    }
}
