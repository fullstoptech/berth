//! Log polling (P9-T4). Coolify has no log stream endpoint, so the
//! plugin polls: a deployment until it reaches a final status, and an
//! application's log tail for appended lines.
//!
//! Both pollers take their interval (and, for application logs, their
//! poll count) as parameters and honour the [`CancellationToken`], so
//! tests run with millisecond intervals and still stop deterministically.

use std::time::Duration;

use tokio_util::sync::CancellationToken;

use crate::client::{ClientError, CoolifyClient};
use crate::types::Deployment;

/// Statuses a deployment never leaves once reached.
pub const FINAL_STATUSES: [&str; 3] = ["finished", "failed", "cancelled"];

/// Whether a deployment status is final (missing counts as not final:
/// the poller keeps asking until Coolify says so or the caller cancels).
#[must_use]
pub fn is_final_status(status: Option<&str>) -> bool {
    match status {
        Some(status) => FINAL_STATUSES.contains(&status),
        None => false,
    }
}

/// The lines of a deployment's `logs` field. Coolify stores a JSON array
/// of entries with an `output` field as a string; anything else is split
/// into lines as-is.
#[must_use]
pub fn deployment_log_lines(deployment: &Deployment) -> Vec<String> {
    let Some(raw) = deployment.logs.as_deref() else {
        return Vec::new();
    };
    if let Ok(entries) = serde_json::from_str::<serde_json::Value>(raw) {
        if let Some(entries) = entries.as_array() {
            return entries
                .iter()
                .filter_map(|entry| entry.get("output").and_then(|o| o.as_str()))
                .flat_map(|output| output.lines().map(str::to_owned))
                .collect();
        }
    }
    raw.lines().map(str::to_owned).collect()
}

async fn sleep_or_cancel(
    interval: Duration,
    cancel: &CancellationToken,
) -> Result<(), ClientError> {
    tokio::select! {
        biased;
        () = cancel.cancelled() => Err(ClientError::Cancelled),
        () = tokio::time::sleep(interval) => Ok(()),
    }
}

/// Polls `GET /deployments/{uuid}` until the deployment reaches a final
/// status, then returns it. `on_poll` sees every response, including the
/// final one — the plugin forwards fresh log lines from it as
/// `logs/lines` notifications.
pub async fn poll_deployment(
    client: &CoolifyClient,
    uuid: &str,
    interval: Duration,
    cancel: &CancellationToken,
    mut on_poll: impl FnMut(&Deployment),
) -> Result<Deployment, ClientError> {
    loop {
        let deployment = client.deployment(uuid, cancel).await?;
        let done = is_final_status(deployment.status.as_deref());
        on_poll(&deployment);
        if done {
            return Ok(deployment);
        }
        sleep_or_cancel(interval, cancel).await?;
    }
}

/// Polls an application's log tail `polls` times (the first fetch counts),
/// returning every line seen with re-sent tails de-duplicated: when the
/// new text extends the old, only the appended part is new.
pub async fn poll_application_logs(
    client: &CoolifyClient,
    uuid: &str,
    lines: u32,
    interval: Duration,
    polls: usize,
    cancel: &CancellationToken,
) -> Result<Vec<String>, ClientError> {
    let mut seen_text = String::new();
    let mut out: Vec<String> = Vec::new();
    for poll in 0..polls {
        if poll > 0 {
            sleep_or_cancel(interval, cancel).await?;
        }
        let text = client.application_logs(uuid, lines, cancel).await?.logs;
        let fresh = match text.strip_prefix(seen_text.as_str()) {
            Some(appended) => appended,
            None if seen_text.starts_with(text.as_str()) => "",
            None => text.as_str(),
        };
        out.extend(fresh.lines().map(str::to_owned));
        if text.len() >= seen_text.len() || !seen_text.starts_with(text.as_str()) {
            seen_text = text;
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    use secrecy::SecretString;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

    use super::*;

    const TIMEOUT: Duration = Duration::from_secs(5);
    const FAST: Duration = Duration::from_millis(1);

    fn client(server: &MockServer) -> CoolifyClient {
        CoolifyClient::new(
            &server.uri(),
            SecretString::from("test-token-not-real".to_owned()),
        )
        .expect("base builds")
    }

    /// Answers with the given bodies in order, repeating the last one.
    struct Sequence {
        bodies: Vec<String>,
        next: Arc<AtomicUsize>,
    }

    impl Sequence {
        fn new(bodies: Vec<String>) -> Self {
            Self {
                bodies,
                next: Arc::new(AtomicUsize::new(0)),
            }
        }
    }

    impl Respond for Sequence {
        fn respond(&self, _request: &Request) -> ResponseTemplate {
            let index = self.next.fetch_add(1, Ordering::SeqCst);
            let body = &self.bodies[index.min(self.bodies.len() - 1)];
            ResponseTemplate::new(200)
                .insert_header("content-type", "application/json")
                .set_body_string(body.clone())
        }
    }

    fn deployment_body(status: &str) -> String {
        format!(r#"{{"deployment_uuid":"dep-1","status":"{status}","application_name":"landing"}}"#)
    }

    #[test]
    fn logs_final_statuses_are_finished_failed_cancelled() {
        assert!(is_final_status(Some("finished")));
        assert!(is_final_status(Some("failed")));
        assert!(is_final_status(Some("cancelled")));
        assert!(!is_final_status(Some("in_progress")));
        assert!(!is_final_status(Some("queued")));
        assert!(!is_final_status(None));
    }

    #[test]
    fn logs_deployment_lines_parse_json_entries() {
        let deployment: Deployment = serde_json::from_value(serde_json::json!({
            "deployment_uuid": "dep-1",
            "logs": "[{\"output\":\"step 1\\nstep 2\",\"type\":\"stdout\"},{\"output\":\"done\"}]",
        }))
        .expect("decodes");
        assert_eq!(
            deployment_log_lines(&deployment),
            ["step 1", "step 2", "done"]
        );
        let plain: Deployment = serde_json::from_value(serde_json::json!({
            "deployment_uuid": "dep-2",
            "logs": "plain one\nplain two",
        }))
        .expect("decodes");
        assert_eq!(deployment_log_lines(&plain), ["plain one", "plain two"]);
    }

    #[tokio::test]
    async fn logs_poll_deployment_stops_at_finished() {
        let server = MockServer::start().await;
        let sequence = Sequence::new(vec![
            deployment_body("in_progress"),
            deployment_body("in_progress"),
            deployment_body("finished"),
        ]);
        let probe = Arc::clone(&sequence.next);
        Mock::given(method("GET"))
            .and(path("/api/v1/deployments/dep-1"))
            .respond_with(sequence)
            .mount(&server)
            .await;
        let mut polls = 0usize;
        let done = tokio::time::timeout(
            TIMEOUT,
            poll_deployment(
                &client(&server),
                "dep-1",
                FAST,
                &CancellationToken::new(),
                |_| polls += 1,
            ),
        )
        .await
        .expect("within timeout")
        .expect("poll ok");
        assert_eq!(done.status.as_deref(), Some("finished"));
        assert_eq!(polls, 3);
        assert_eq!(probe.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn logs_poll_deployment_stops_at_failed_and_cancelled() {
        for final_status in ["failed", "cancelled"] {
            let server = MockServer::start().await;
            Mock::given(method("GET"))
                .and(path("/api/v1/deployments/dep-1"))
                .respond_with(Sequence::new(vec![
                    deployment_body("queued"),
                    deployment_body(final_status),
                ]))
                .mount(&server)
                .await;
            let done = tokio::time::timeout(
                TIMEOUT,
                poll_deployment(
                    &client(&server),
                    "dep-1",
                    FAST,
                    &CancellationToken::new(),
                    |_| {},
                ),
            )
            .await
            .expect("within timeout")
            .expect("poll ok");
            assert_eq!(done.status.as_deref(), Some(final_status));
        }
    }

    #[tokio::test]
    async fn logs_poll_deployment_honours_cancellation() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/deployments/dep-1"))
            .respond_with(Sequence::new(vec![deployment_body("in_progress")]))
            .mount(&server)
            .await;
        let cancel = CancellationToken::new();
        let canceller = cancel.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(20)).await;
            canceller.cancel();
        });
        let err = tokio::time::timeout(
            TIMEOUT,
            poll_deployment(&client(&server), "dep-1", FAST, &cancel, |_| {}),
        )
        .await
        .expect("within timeout")
        .expect_err("cancelled poll fails");
        assert!(matches!(err, ClientError::Cancelled), "{err:?}");
    }

    #[tokio::test]
    async fn logs_poll_application_logs_dedupes_the_tail() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/applications/app-1/logs"))
            .respond_with(Sequence::new(vec![
                r#"{"logs":"one\ntwo\n"}"#.to_owned(),
                r#"{"logs":"one\ntwo\n"}"#.to_owned(),
                r#"{"logs":"one\ntwo\nthree\n"}"#.to_owned(),
            ]))
            .mount(&server)
            .await;
        let lines = tokio::time::timeout(
            TIMEOUT,
            poll_application_logs(
                &client(&server),
                "app-1",
                100,
                FAST,
                3,
                &CancellationToken::new(),
            ),
        )
        .await
        .expect("within timeout")
        .expect("poll ok");
        assert_eq!(lines, ["one", "two", "three"]);
    }
}
