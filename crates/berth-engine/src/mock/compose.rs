//! [`MockCompose`]: records every command it is asked to run and streams
//! scripted progress ending in `Done { code: 0 }`. `config` returns the
//! world's services object for the target project when the world knows it,
//! else an empty services object.

use futures::StreamExt;

use super::MockWorld;
use crate::compose::ComposeRunner;
use crate::EngineError;

#[derive(Default)]
pub struct MockCompose {
    recorded: std::sync::Mutex<Vec<(berth_core::ComposeTarget, berth_core::ComposeCommand)>>,
    compose_services: std::collections::BTreeMap<String, serde_json::Value>,
}

impl MockCompose {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn from_fixture(json: &str) -> Result<Self, EngineError> {
        Ok(Self::from_world(&MockWorld::from_json(json)?))
    }

    #[must_use]
    pub fn from_world(world: &MockWorld) -> Self {
        Self {
            recorded: std::sync::Mutex::new(Vec::new()),
            compose_services: world.compose_services.clone(),
        }
    }

    #[must_use]
    pub fn bundled() -> Self {
        Self::from_world(&MockWorld::bundled())
    }

    /// Every `(target, command)` this mock was asked to run, in order.
    #[must_use]
    pub fn recorded(&self) -> Vec<(berth_core::ComposeTarget, berth_core::ComposeCommand)> {
        self.recorded.lock().map(|r| r.clone()).unwrap_or_default()
    }
}

fn command_services(cmd: &berth_core::ComposeCommand) -> Vec<String> {
    match cmd {
        berth_core::ComposeCommand::Up { services, .. }
        | berth_core::ComposeCommand::Start { services }
        | berth_core::ComposeCommand::Stop { services }
        | berth_core::ComposeCommand::Restart { services }
        | berth_core::ComposeCommand::Build { services, .. }
        | berth_core::ComposeCommand::Pull { services } => services.clone(),
        berth_core::ComposeCommand::Down { .. } => Vec::new(),
    }
}

#[async_trait::async_trait]
impl ComposeRunner for MockCompose {
    fn run(
        &self,
        target: &berth_core::ComposeTarget,
        cmd: berth_core::ComposeCommand,
        _cancel: tokio_util::sync::CancellationToken,
    ) -> futures::stream::BoxStream<'static, Result<berth_core::ComposeEvent, EngineError>> {
        if let Ok(mut recorded) = self.recorded.lock() {
            recorded.push((target.clone(), cmd.clone()));
        }
        let mut services = command_services(&cmd);
        if services.is_empty() {
            if let Some(known) = self
                .compose_services
                .get(&target.project_name)
                .and_then(serde_json::Value::as_object)
            {
                services = known.keys().cloned().collect();
            }
        }
        let mut events: Vec<Result<berth_core::ComposeEvent, EngineError>> = services
            .into_iter()
            .map(|service| {
                Ok(berth_core::ComposeEvent::Progress {
                    id: format!("Container {}-{service}-1", target.project_name),
                    status: "Working".to_string(),
                    text: None,
                })
            })
            .collect();
        if events.is_empty() {
            events.push(Ok(berth_core::ComposeEvent::Progress {
                id: target.project_name.clone(),
                status: "Working".to_string(),
                text: None,
            }));
        }
        events.push(Ok(berth_core::ComposeEvent::Done { code: 0 }));
        futures::stream::iter(events).boxed()
    }

    async fn config(
        &self,
        target: &berth_core::ComposeTarget,
    ) -> Result<serde_json::Value, EngineError> {
        let services = self
            .compose_services
            .get(&target.project_name)
            .cloned()
            .unwrap_or_else(|| serde_json::json!({}));
        Ok(serde_json::json!({
            "name": target.project_name,
            "services": services,
        }))
    }

    async fn validate(
        &self,
        _folder: &std::path::Path,
        _files: &[std::path::PathBuf],
    ) -> Result<(), EngineError> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target() -> berth_core::ComposeTarget {
        berth_core::ComposeTarget {
            engine: berth_core::EngineId("colima:default".to_string()),
            docker_host: "unix:///mock/colima/default/docker.sock".to_string(),
            project_name: "shop-api".to_string(),
            folder: std::path::PathBuf::from("/Users/berth/Dev/clients/acme/shop-api"),
            files: vec![std::path::PathBuf::from(
                "/Users/berth/Dev/clients/acme/shop-api/compose.yaml",
            )],
        }
    }

    #[tokio::test]
    async fn mock_compose_records_commands_and_ends_done_zero() {
        let mock = MockCompose::bundled();
        let cmd = berth_core::ComposeCommand::Restart {
            services: vec!["web".to_string()],
        };
        let events: Vec<_> = mock
            .run(
                &target(),
                cmd.clone(),
                tokio_util::sync::CancellationToken::new(),
            )
            .collect::<Vec<_>>()
            .await
            .into_iter()
            .collect::<Result<Vec<_>, _>>()
            .expect("mock run succeeds");
        assert!(events
            .iter()
            .any(|e| matches!(e, berth_core::ComposeEvent::Progress { .. })));
        assert_eq!(
            events.last(),
            Some(&berth_core::ComposeEvent::Done { code: 0 })
        );
        assert_eq!(mock.recorded(), [(target(), cmd)]);
    }

    #[tokio::test]
    async fn mock_compose_config_returns_world_services() {
        let mock = MockCompose::bundled();
        let config = mock.config(&target()).await.expect("config");
        let services = config
            .get("services")
            .and_then(serde_json::Value::as_object)
            .expect("services object");
        assert_eq!(services.len(), 5, "{config}");
        assert!(services.contains_key("api") && services.contains_key("db"));
        mock.validate(&target().folder, &target().files)
            .await
            .expect("validate");
        // A bare mock with no world still answers with an empty services object.
        let bare = MockCompose::new();
        let config = bare.config(&target()).await.expect("bare config");
        assert!(
            config
                .get("services")
                .and_then(serde_json::Value::as_object)
                .is_some_and(|o| o.is_empty()),
            "{config}"
        );
    }
}
