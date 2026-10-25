//! `EngineAction::Remove` (D25a): delete one image, volume or network
//! through whichever client call fits the kind.
//!
//! `force` means what it means to Docker: for an image, remove it although
//! containers (or other tags) use it; for a volume, treat "already gone" as
//! done (`docker volume rm -f`); a network has no force, so it is ignored.

use berth_core::ResourceRef;

use crate::{EngineClient, EngineError};

/// Remove `target` from the engine behind `client`.
///
/// # Errors
/// The engine's error; a volume that is not there is an error unless `force`.
pub async fn remove_resource(
    client: &dyn EngineClient,
    target: &ResourceRef,
    force: bool,
) -> Result<(), EngineError> {
    match target {
        ResourceRef::Image(id) => client.remove_image(id, force).await,
        ResourceRef::Volume(name) => match client.remove_volume(name).await {
            Err(EngineError::NotFound(_)) if force => Ok(()),
            other => other,
        },
        ResourceRef::Network(id) => client.remove_network(id).await,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MockEngine;

    fn engine() -> MockEngine {
        MockEngine::bundled(&berth_core::EngineId("colima:default".to_string()))
    }

    #[tokio::test]
    async fn remove_network_deletes_a_network_and_only_that() {
        let engine = engine();
        let before = engine.networks().await.expect("networks");
        let volumes = engine.volumes().await.expect("volumes").len();
        let target = before
            .iter()
            .find(|n| n.name == "docs-site_default")
            .expect("network")
            .clone();
        remove_resource(&engine, &ResourceRef::Network(target.id.clone()), false)
            .await
            .expect("removed");
        let after = engine.networks().await.expect("networks");
        assert_eq!(after.len(), before.len() - 1);
        assert!(after.iter().all(|n| n.id != target.id));
        assert_eq!(engine.volumes().await.expect("volumes").len(), volumes);
    }

    #[tokio::test]
    async fn remove_volume_deletes_a_volume_not_a_network_of_that_name() {
        let engine = engine();
        let volumes = engine.volumes().await.expect("volumes");
        let networks = engine.networks().await.expect("networks");
        let volume = volumes.first().expect("a volume").name.clone();
        remove_resource(&engine, &ResourceRef::Volume(volume.clone()), false)
            .await
            .expect("removed");
        assert!(engine
            .volumes()
            .await
            .expect("volumes")
            .iter()
            .all(|v| v.name != volume));
        assert_eq!(
            engine.networks().await.expect("networks").len(),
            networks.len()
        );
    }

    #[tokio::test]
    async fn remove_image_deletes_an_unused_image_and_force_deletes_a_used_one() {
        let engine = engine();
        let images = engine.images().await.expect("images");
        let used = images
            .iter()
            .find(|i| !i.used_by.is_empty())
            .expect("a used image")
            .id
            .clone();
        let unused = images
            .iter()
            .find(|i| i.used_by.is_empty())
            .expect("an unused image")
            .id
            .clone();
        remove_resource(&engine, &ResourceRef::Image(unused.clone()), false)
            .await
            .expect("removed");
        assert!(engine
            .images()
            .await
            .expect("images")
            .iter()
            .all(|i| i.id != unused));
        // Not forced, a used image stays; forced, it goes.
        let refused = remove_resource(&engine, &ResourceRef::Image(used.clone()), false).await;
        assert!(refused.is_err(), "{refused:?}");
        assert!(engine
            .images()
            .await
            .expect("images")
            .iter()
            .any(|i| i.id == used));
        remove_resource(&engine, &ResourceRef::Image(used.clone()), true)
            .await
            .expect("forced");
        assert!(engine
            .images()
            .await
            .expect("images")
            .iter()
            .all(|i| i.id != used));
    }

    #[tokio::test]
    async fn remove_a_missing_volume_is_an_error_unless_forced() {
        let engine = engine();
        let missing = ResourceRef::Volume("berthfx-no-such-volume".to_string());
        let refused = remove_resource(&engine, &missing, false).await;
        assert!(
            matches!(refused, Err(EngineError::NotFound(_))),
            "{refused:?}"
        );
        remove_resource(&engine, &missing, true)
            .await
            .expect("force treats already gone as done");
        // A missing network or image is still an error with force.
        let network =
            remove_resource(&engine, &ResourceRef::Network("nope".to_string()), true).await;
        assert!(
            matches!(network, Err(EngineError::NotFound(_))),
            "{network:?}"
        );
    }
}
