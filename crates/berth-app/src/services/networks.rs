//! Networks (P4-T5, brief §7): everything the Networks screen, its create
//! sheet and the connect/disconnect lists decide without drawing.
//!
//! * [`parse_subnet`] and [`parse_gateway`] are pure: an invalid subnet is
//!   caught while the user types, never by Docker's error message.
//! * [`validate`] turns the sheet's [`Draft`] into a `NetworkSpec` or the
//!   [`FieldError`]s the sheet shows inline.
//! * [`NetOp`] and [`run_net`] are the engine work behind the actions
//!   service (create, delete, connect, disconnect).
//! * [`engine_of`], [`project_of`] and [`connectable`] answer the questions
//!   the data model leaves open: `Network` carries no engine id (PLAN §6.2),
//!   and Compose project names come from the containers on it.

use std::net::IpAddr;

use tokio_util::sync::CancellationToken;

use berth_core::{
    Container, ContainerId, Engine, EngineId, EngineState, Network, NetworkSpec, Project,
};

use crate::services::edit::{guard, Done};
use crate::state::EngineScope;

/// Why a built-in network has no Delete (`bridge`, `host`, `none`).
pub const BUILT_IN_REASON: &str = "Built-in networks cannot be removed";

// ---------------------------------------------------------------------
// Subnets
// ---------------------------------------------------------------------

/// A CIDR block: the network address and its prefix length.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Subnet {
    addr: IpAddr,
    prefix: u8,
}

/// Why a subnet is not usable. `Display` is the inline text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SubnetError {
    /// No `/prefix`: "172.31.250.0".
    NotCidr,
    /// The part before the slash is not an IP address.
    BadAddress,
    /// The prefix is not a number in the range a network needs.
    BadPrefix { max: u8 },
    /// Bits after the prefix are set: "172.31.250.5/24".
    HostBits { network: String },
}

impl std::fmt::Display for SubnetError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotCidr => write!(f, "Write the subnet with its prefix, like 172.31.250.0/24"),
            Self::BadAddress => write!(f, "The address before the / is not a valid IP address"),
            Self::BadPrefix { max } => {
                write!(f, "The prefix after the / must be a number from 1 to {max}")
            }
            Self::HostBits { network } => {
                write!(f, "Host bits are set. Did you mean {network}?")
            }
        }
    }
}

fn bits_of(addr: IpAddr) -> (u128, u32) {
    match addr {
        IpAddr::V4(a) => (u128::from(u32::from(a)), 32),
        IpAddr::V6(a) => (u128::from(a), 128),
    }
}

fn addr_from(raw: u128, bits: u32) -> IpAddr {
    if bits == 32 {
        IpAddr::V4(std::net::Ipv4Addr::from(u32::try_from(raw).unwrap_or(0)))
    } else {
        IpAddr::V6(std::net::Ipv6Addr::from(raw))
    }
}

impl Subnet {
    fn bits(self) -> u32 {
        bits_of(self.addr).1
    }

    /// The most-significant `prefix` bits set.
    fn mask(self) -> u128 {
        let bits = self.bits();
        let prefix = u32::from(self.prefix);
        let all = if bits == 128 {
            u128::MAX
        } else {
            (1u128 << bits) - 1
        };
        let host = if prefix >= bits {
            0
        } else {
            (1u128 << (bits - prefix)) - 1
        };
        all & !host
    }

    /// The first address of the block.
    #[must_use]
    pub fn network(self) -> IpAddr {
        addr_from(bits_of(self.addr).0 & self.mask(), self.bits())
    }

    /// The last address of the block (the IPv4 broadcast address).
    fn last(self) -> u128 {
        let bits = self.bits();
        let all = if bits == 128 {
            u128::MAX
        } else {
            (1u128 << bits) - 1
        };
        bits_of(self.addr).0 | (!self.mask() & all)
    }

    /// True when `ip` is inside the block (and of the same family).
    #[must_use]
    pub fn contains(self, ip: IpAddr) -> bool {
        let (raw, bits) = bits_of(ip);
        bits == self.bits() && raw & self.mask() == bits_of(self.addr).0 & self.mask()
    }

    /// True when the two blocks share an address.
    #[must_use]
    pub fn overlaps(self, other: Subnet) -> bool {
        // The shorter prefix has the smaller mask; it decides, because two
        // blocks either nest or are apart.
        let mask = self.mask().min(other.mask());
        self.bits() == other.bits() && bits_of(self.addr).0 & mask == bits_of(other.addr).0 & mask
    }
}

impl std::fmt::Display for Subnet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}/{}", self.network(), self.prefix)
    }
}

/// Read `172.31.250.0/24` (or an IPv6 block). IPv4 prefixes run 1 to 30 (a
/// bridge needs a gateway and one container), IPv6 prefixes 1 to 126. An
/// address with host bits set is refused with the corrected block named.
///
/// # Errors
/// [`SubnetError`], whose text is what the sheet shows.
pub fn parse_subnet(text: &str) -> Result<Subnet, SubnetError> {
    let text = text.trim();
    let (address, prefix) = text.split_once('/').ok_or(SubnetError::NotCidr)?;
    let addr: IpAddr = address
        .trim()
        .parse()
        .map_err(|_| SubnetError::BadAddress)?;
    let max: u8 = if addr.is_ipv4() { 30 } else { 126 };
    let prefix = prefix.trim();
    // `u8::from_str` accepts a leading "+": a prefix is plain digits.
    let prefix: u8 = if prefix.is_empty() || !prefix.bytes().all(|b| b.is_ascii_digit()) {
        return Err(SubnetError::BadPrefix { max });
    } else {
        prefix.parse().map_err(|_| SubnetError::BadPrefix { max })?
    };
    if prefix == 0 || prefix > max {
        return Err(SubnetError::BadPrefix { max });
    }
    let subnet = Subnet { addr, prefix };
    if subnet.network() != addr {
        return Err(SubnetError::HostBits {
            network: subnet.to_string(),
        });
    }
    Ok(subnet)
}

/// Why a gateway is not usable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GatewayError {
    /// The text is not an IP address.
    NotAddress,
    /// A gateway needs the subnet it belongs to.
    NeedsSubnet,
    /// An IPv4 address with an IPv6 subnet, or the reverse.
    WrongFamily,
    /// Not inside the subnet.
    Outside { subnet: String },
    /// The network address (or an IPv4 broadcast address) names no host.
    Reserved,
}

impl std::fmt::Display for GatewayError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotAddress => write!(f, "This is not a valid IP address"),
            Self::NeedsSubnet => write!(f, "Set the subnet first; the gateway lives inside it"),
            Self::WrongFamily => write!(f, "The gateway and the subnet must both be IPv4 or IPv6"),
            Self::Outside { subnet } => write!(f, "The gateway must be inside {subnet}"),
            Self::Reserved => write!(f, "This address is the network or broadcast address"),
        }
    }
}

/// Read a gateway and check it against its subnet. `subnet` is `None` when
/// the subnet field is empty or invalid.
///
/// # Errors
/// [`GatewayError`].
pub fn parse_gateway(text: &str, subnet: Option<Subnet>) -> Result<IpAddr, GatewayError> {
    let ip: IpAddr = text.trim().parse().map_err(|_| GatewayError::NotAddress)?;
    let subnet = subnet.ok_or(GatewayError::NeedsSubnet)?;
    if ip.is_ipv4() != subnet.addr.is_ipv4() {
        return Err(GatewayError::WrongFamily);
    }
    if !subnet.contains(ip) {
        return Err(GatewayError::Outside {
            subnet: subnet.to_string(),
        });
    }
    let (raw, _) = bits_of(ip);
    let is_network = ip == subnet.network();
    let is_broadcast = ip.is_ipv4() && raw == subnet.last();
    if is_network || is_broadcast {
        return Err(GatewayError::Reserved);
    }
    Ok(ip)
}

// ---------------------------------------------------------------------
// The create sheet's draft
// ---------------------------------------------------------------------

/// The drivers a single Docker engine offers (overlay needs Swarm).
pub const DRIVERS: [&str; 3] = ["bridge", "macvlan", "ipvlan"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Draft {
    pub name: String,
    pub driver: String,
    pub subnet: String,
    pub gateway: String,
    pub internal: bool,
    pub attach: Vec<ContainerId>,
}

impl Default for Draft {
    fn default() -> Self {
        Self {
            name: String::new(),
            driver: DRIVERS[0].to_string(),
            subnet: String::new(),
            gateway: String::new(),
            internal: false,
            attach: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    Name,
    Subnet,
    Gateway,
}

/// One thing wrong with the draft. A `blank` error (nothing typed yet) keeps
/// the button disabled but draws no red text on an untouched form.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldError {
    pub field: Field,
    pub message: String,
    pub blank: bool,
}

fn error(field: Field, message: impl Into<String>) -> FieldError {
    FieldError {
        field,
        message: message.into(),
        blank: false,
    }
}

fn name_error(name: &str, existing: &[Network]) -> Option<FieldError> {
    let name = name.trim();
    if name.is_empty() {
        return Some(FieldError {
            field: Field::Name,
            message: "Name the network".to_string(),
            blank: true,
        });
    }
    let mut chars = name.chars();
    let first_ok = chars.next().is_some_and(|c| c.is_ascii_alphanumeric());
    let rest_ok = name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'));
    if name.chars().count() < 2 || !first_ok || !rest_ok {
        return Some(error(
            Field::Name,
            "Use at least 2 letters, digits, _ . or -, starting with a letter or digit",
        ));
    }
    if existing.iter().any(|n| n.name == name) {
        return Some(error(
            Field::Name,
            format!("A network named {name} already exists"),
        ));
    }
    None
}

/// The draft as the spec to create, or every field that is wrong.
/// `existing` is what the engine already has: names and subnets must not
/// collide with it (Docker answers "Pool overlaps" only after the click).
///
/// # Errors
/// The [`FieldError`]s, in field order.
pub fn validate(draft: &Draft, existing: &[Network]) -> Result<NetworkSpec, Vec<FieldError>> {
    let mut errors = Vec::new();
    errors.extend(name_error(&draft.name, existing));

    let subnet_text = draft.subnet.trim();
    let mut subnet = None;
    if !subnet_text.is_empty() {
        match parse_subnet(subnet_text) {
            Ok(parsed) => {
                let clash = existing.iter().find(|n| {
                    n.subnet
                        .as_deref()
                        .and_then(|s| parse_subnet(s).ok())
                        .is_some_and(|other| other.overlaps(parsed))
                });
                match clash {
                    Some(n) => errors.push(error(
                        Field::Subnet,
                        format!(
                            "Overlaps {} on network {}",
                            n.subnet.clone().unwrap_or_default(),
                            n.name
                        ),
                    )),
                    None => subnet = Some(parsed),
                }
            }
            Err(err) => errors.push(error(Field::Subnet, err.to_string())),
        }
    }
    let gateway_text = draft.gateway.trim();
    let mut gateway = None;
    // A subnet that has its own error is the problem to show, not the gateway.
    let subnet_blamed = subnet.is_none() && !subnet_text.is_empty();
    if !gateway_text.is_empty() && !subnet_blamed {
        match parse_gateway(gateway_text, subnet) {
            Ok(ip) => gateway = Some(ip),
            Err(err) => errors.push(error(Field::Gateway, err.to_string())),
        }
    }
    if !errors.is_empty() {
        return Err(errors);
    }
    Ok(NetworkSpec {
        name: draft.name.trim().to_string(),
        driver: draft.driver.clone(),
        subnet: subnet.map(|s| s.to_string()),
        gateway: gateway.map(|g| g.to_string()),
        internal: draft.internal,
        attach: draft.attach.clone(),
    })
}

/// The create button's disabled reason: the first problem, blank or not.
#[must_use]
pub fn create_blocker(errors: &[FieldError]) -> Option<String> {
    errors.first().map(|e| e.message.clone())
}

// ---------------------------------------------------------------------
// What the data model leaves open
// ---------------------------------------------------------------------

/// The engine a network lives on. `Network` has no engine id (PLAN §6.2),
/// so: the engine of a container on it; else the selected engine; else the
/// only running engine; else `None` and the caller asks the user to pick.
#[must_use]
pub fn engine_of(
    network: &Network,
    containers: &[Container],
    engines: &[Engine],
    scope: &EngineScope,
) -> Option<EngineId> {
    if let Some(engine) = network
        .containers
        .iter()
        .find_map(|id| containers.iter().find(|c| c.id == *id))
        .map(|c| c.engine.clone())
    {
        return Some(engine);
    }
    if let EngineScope::One(engine) = scope {
        return Some(engine.clone());
    }
    let mut running = engines.iter().filter(|e| e.state == EngineState::Running);
    match (running.next(), running.next()) {
        (Some(only), None) => Some(only.id.clone()),
        _ => None,
    }
}

/// The project a network belongs to, by name: the engine's own answer, else
/// the one project every container on it belongs to (Compose creates the
/// default network of a project; the engine's list has no label for it).
#[must_use]
pub fn project_of(
    network: &Network,
    containers: &[Container],
    projects: &[Project],
) -> Option<String> {
    let id = network.project.clone().or_else(|| {
        let mut owners = network
            .containers
            .iter()
            .filter_map(|id| containers.iter().find(|c| c.id == *id))
            .map(|c| project_id_of(c, projects));
        let first = owners.next()??;
        owners.all(|p| p.as_ref() == Some(&first)).then_some(first)
    })?;
    projects.iter().find(|p| p.id == id).map(|p| p.name.clone())
}

/// The project of a container: the one the engines service assigned, else
/// the one named like its Compose project label.
fn project_id_of(container: &Container, projects: &[Project]) -> Option<berth_core::ProjectId> {
    container.project.clone().or_else(|| {
        let name = container.compose_project.as_deref()?;
        projects
            .iter()
            .find(|p| p.name == name)
            .map(|p| p.id.clone())
    })
}

/// The containers `network` could be connected to: on its engine and not
/// already on it, running ones first, then by name.
#[must_use]
pub fn connectable<'a>(
    network: &Network,
    engine: &EngineId,
    containers: &'a [Container],
) -> Vec<&'a Container> {
    let mut list: Vec<&Container> = containers
        .iter()
        .filter(|c| c.engine == *engine)
        .filter(|c| !network.containers.contains(&c.id) && !c.networks.contains(&network.name))
        .collect();
    list.sort_by(|a, b| {
        let running = |c: &Container| {
            matches!(
                c.state,
                berth_core::ContainerState::Running | berth_core::ContainerState::Restarting
            )
        };
        running(b)
            .cmp(&running(a))
            .then_with(|| a.name.cmp(&b.name))
    });
    list
}

/// The network a container's `networks` entry (a name) stands for: on the
/// container's own engine, so a `bridge` of another engine never matches.
#[must_use]
pub fn network_named<'a>(
    name: &str,
    container: &Container,
    networks: &'a [Network],
) -> Option<&'a Network> {
    let mut named = networks.iter().filter(|n| n.name == name);
    let first = named.next()?;
    if named.next().is_none() {
        return Some(first);
    }
    networks
        .iter()
        .filter(|n| n.name == name)
        .find(|n| n.containers.contains(&container.id))
        .or(Some(first))
}

/// The networks a container can still join: on its own engine, not already
/// on it, user networks first, then the built-in ones, by name. `host` and
/// `none` are never offered (Docker refuses either next to another network).
#[must_use]
pub fn joinable<'a>(
    container: &Container,
    networks: &'a [Network],
    containers: &[Container],
    engines: &[Engine],
    scope: &EngineScope,
) -> Vec<&'a Network> {
    let mut list: Vec<&Network> = networks
        .iter()
        .filter(|n| !container.networks.contains(&n.name))
        .filter(|n| !matches!(n.name.as_str(), "host" | "none"))
        .filter(|n| engine_of(n, containers, engines, scope).as_ref() == Some(&container.engine))
        .collect();
    list.sort_by(|a, b| {
        a.built_in
            .cmp(&b.built_in)
            .then_with(|| a.name.cmp(&b.name))
    });
    list
}

// ---------------------------------------------------------------------
// The engine work
// ---------------------------------------------------------------------

/// What one network action does.
#[derive(Debug, Clone, PartialEq)]
pub enum NetOp {
    Create(NetworkSpec),
    /// Delete by id.
    Remove {
        id: String,
    },
    Connect {
        network: String,
        container: ContainerId,
    },
    Disconnect {
        network: String,
        container: ContainerId,
    },
}

impl NetOp {
    /// The pending key and activity action of this op.
    #[must_use]
    pub fn action_id(&self) -> &'static str {
        match self {
            Self::Create(_) => "network.create",
            Self::Remove { .. } => "network.delete",
            Self::Connect { .. } => "network.connect_container",
            Self::Disconnect { .. } => "network.disconnect",
        }
    }
}

/// Run one network action to its outcome. `name` is the network's name,
/// for the toast.
///
/// # Errors
/// The engine's message.
pub async fn run_net(
    client: &dyn berth_engine::EngineClient,
    name: &str,
    op: NetOp,
    cancel: &CancellationToken,
) -> Result<Done, String> {
    let message = match op {
        NetOp::Create(spec) => {
            guard(cancel, client.create_network(&spec)).await?;
            Some(format!("Created network {}", spec.name))
        }
        // The confirm dialog says this result itself, from its summary.
        NetOp::Remove { id } => {
            guard(cancel, client.remove_network(&id)).await?;
            None
        }
        NetOp::Connect { network, container } => {
            guard(cancel, client.connect_network(&network, &container)).await?;
            Some(format!("Connected to {name}"))
        }
        NetOp::Disconnect { network, container } => {
            guard(cancel, client.disconnect_network(&network, &container)).await?;
            Some(format!("Disconnected from {name}"))
        }
    };
    Ok(Done {
        replaced_by: None,
        message,
    })
}

#[cfg(test)]
#[path = "networks_tests.rs"]
mod tests;
