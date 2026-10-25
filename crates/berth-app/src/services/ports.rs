//! Which host ports the engines' containers hold (AD33), for the sheets that offer a free port (Add
//! service, Run an image, Edit and recreate) and for "5432 is used by pg-scratch" messages.
//!
//! Only containers that are up count: a running, paused or restarting container holds its published
//! ports, while an exited or created one binds nothing until it starts. A mapping with no host port
//! (`3000/tcp` exposed, not published) holds nothing. TCP and UDP are not told apart: a port is
//! reported as taken when either protocol publishes it.
//!
//! Pure functions over the container list. The caller passes the containers of the engine the new
//! service will run on; ports on another engine's host are not in conflict.

use berth_core::Container;

use super::group_ops::is_active;

/// The host ports the active containers publish, sorted and without repeats. Feed it to
/// `berth_rules::ports::suggest`.
#[must_use]
pub fn taken_ports(containers: &[Container]) -> Vec<u16> {
    let mut ports: Vec<u16> = containers
        .iter()
        .filter(|c| is_active(c.state))
        .flat_map(|c| c.ports.iter().filter_map(|p| p.host_port))
        .collect();
    ports.sort_unstable();
    ports.dedup();
    ports
}

/// The sentence for a taken port, `"5432 is used by pg-scratch"`, or `None` when no active container
/// publishes it. With several owners the first by name is named, so the message does not change
/// when the engine lists them in another order.
#[must_use]
pub fn port_owner(port: u16, containers: &[Container]) -> Option<String> {
    containers
        .iter()
        .filter(|c| is_active(c.state))
        .filter(|c| c.ports.iter().any(|p| p.host_port == Some(port)))
        .map(|c| c.name.as_str())
        .min()
        .map(|name| format!("{port} is used by {name}"))
}

#[cfg(test)]
mod tests {
    use berth_core::{ContainerState, PortMapping, Proto};

    use super::*;

    fn container(name: &str, state: ContainerState, host_ports: &[Option<u16>]) -> Container {
        let mut c = berth_engine::MockWorld::bundled()
            .containers
            .into_iter()
            .next()
            .expect("the mock world has containers");
        c.name = name.to_string();
        c.state = state;
        c.ports = host_ports
            .iter()
            .map(|host_port| PortMapping {
                host_ip: None,
                host_port: *host_port,
                container_port: 80,
                proto: Proto::Tcp,
            })
            .collect();
        c
    }

    #[test]
    fn taken_ports_lists_published_host_ports_sorted_and_once() {
        let containers = [
            container("web", ContainerState::Running, &[Some(8080), Some(443)]),
            container("db", ContainerState::Running, &[Some(5432)]),
            container("web-2", ContainerState::Paused, &[Some(8080)]),
        ];
        assert_eq!(taken_ports(&containers), [443, 5432, 8080]);
    }

    #[test]
    fn taken_ports_ignores_containers_that_hold_nothing() {
        let containers = [
            container("stopped", ContainerState::Exited, &[Some(5432)]),
            container("never-started", ContainerState::Created, &[Some(6379)]),
            container("dead", ContainerState::Dead, &[Some(3306)]),
            // Exposed but not published: no host port.
            container("internal", ContainerState::Running, &[None]),
            container("restarting", ContainerState::Restarting, &[Some(9000)]),
        ];
        assert_eq!(taken_ports(&containers), [9000]);
        assert_eq!(taken_ports(&[]), Vec::<u16>::new());
    }

    #[test]
    fn port_owner_names_the_container_in_a_sentence() {
        let containers = [
            container("pg-scratch", ContainerState::Running, &[Some(5432)]),
            container("web", ContainerState::Running, &[Some(8080)]),
        ];
        assert_eq!(
            port_owner(5432, &containers).as_deref(),
            Some("5432 is used by pg-scratch")
        );
        assert_eq!(port_owner(3000, &containers), None);
    }

    #[test]
    fn port_owner_skips_a_stopped_container_and_names_the_first_owner_by_name() {
        let containers = [
            container("zeta", ContainerState::Running, &[Some(5432)]),
            container("alpha", ContainerState::Running, &[Some(5432)]),
            container("old-pg", ContainerState::Exited, &[Some(5433)]),
        ];
        assert_eq!(
            port_owner(5432, &containers).as_deref(),
            Some("5432 is used by alpha")
        );
        assert_eq!(port_owner(5433, &containers), None, "exited holds nothing");
    }
}
