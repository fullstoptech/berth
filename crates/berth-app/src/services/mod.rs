//! Services own every I/O task and write signals; the UI never calls an
//! engine (D23). One file per service (PLAN §6.11).

pub mod actions;
pub mod activity;
pub mod colima;
pub mod compose;
pub mod engines;

/// Everything a later service or screen needs from the substrate P2-T1
/// wires up: the store, the live client map and the root cancellation
/// token. Provided as Dioxus context by the App root.
#[derive(Clone)]
pub struct Services {
    pub store: std::sync::Arc<berth_store::Store>,
    pub clients: engines::ClientMap,
    pub cancel: tokio_util::sync::CancellationToken,
}

impl Services {
    /// The client for one engine, once discovery has connected it.
    pub fn client(
        &self,
        engine: &berth_core::EngineId,
    ) -> Option<std::sync::Arc<dyn berth_engine::EngineClient>> {
        self.clients
            .lock()
            .ok()
            .and_then(|map| map.get(engine).cloned())
    }
}
