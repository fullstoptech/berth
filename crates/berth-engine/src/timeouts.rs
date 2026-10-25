//! Per-call request timeouts for the Docker API (AD29).
//!
//! bollard has one timeout per client, applied from sending the request to
//! receiving the response head. A single 2 s value broke two kinds of
//! call: `system df` (seconds on a busy engine) and every call that holds a
//! connection open (logs, events, stats, pull, archive transfers, build,
//! attach), where the head can arrive late and the rest of the exchange has
//! no deadline at all. Each call site names its [`CallKind`]; this table is
//! the only place a number lives.
//!
//! Stop and restart keep their polling in `recreate` (a container whose
//! PID 1 ignores SIGTERM outlives any request timeout, so the caller polls
//! for the exit); they use [`CallKind::ListInspect`] like every other short
//! request.

use std::time::Duration;

/// List and inspect calls, and the short mutations that behave like them
/// (create, start, remove, rename, connect).
pub const LIST_INSPECT: Duration = Duration::from_secs(5);

/// `system df`, which `disk_usage` and `volume_sizes` read. It walks every
/// volume: 4 s measured on a small engine.
pub const DISK_USAGE: Duration = Duration::from_secs(60);

/// What a call needs from the request timeout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallKind {
    ListInspect,
    DiskUsage,
    Logs,
    Events,
    Stats,
    Pull,
    /// Archive download: file browser, volume backup.
    Export,
    /// Archive upload: copy into a container, volume restore.
    Upload,
    Build,
    /// Exec attach: the terminal.
    Attach,
}

impl CallKind {
    /// The request timeout of this kind of call. `None` means no timeout:
    /// the caller ends the call through its `CancellationToken` (or by
    /// dropping the future).
    #[must_use]
    pub const fn timeout(self) -> Option<Duration> {
        match self {
            Self::ListInspect => Some(LIST_INSPECT),
            Self::DiskUsage => Some(DISK_USAGE),
            Self::Logs
            | Self::Events
            | Self::Stats
            | Self::Pull
            | Self::Export
            | Self::Upload
            | Self::Build
            | Self::Attach => None,
        }
    }

    /// What to hand to bollard's `with_timeout`, which wants a duration:
    /// "no timeout" is the longest one it can hold. tokio treats a deadline
    /// that far out as never.
    #[must_use]
    pub(crate) fn for_bollard(self) -> Duration {
        self.timeout().unwrap_or(NO_TIMEOUT)
    }
}

const NO_TIMEOUT: Duration = Duration::from_secs(u64::MAX);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timeouts_each_kind_of_call_has_its_budget() {
        let table = [
            (CallKind::ListInspect, Some(Duration::from_secs(5))),
            (CallKind::DiskUsage, Some(Duration::from_secs(60))),
            (CallKind::Logs, None),
            (CallKind::Events, None),
            (CallKind::Stats, None),
            (CallKind::Pull, None),
            (CallKind::Export, None),
            (CallKind::Upload, None),
            (CallKind::Build, None),
            (CallKind::Attach, None),
        ];
        for (kind, expected) in table {
            assert_eq!(kind.timeout(), expected, "{kind:?}");
        }
    }

    #[tokio::test]
    async fn timeouts_no_timeout_does_not_overflow_tokios_clock() {
        let done = tokio::time::timeout(CallKind::Pull.for_bollard(), async { 7 }).await;
        assert_eq!(done.ok(), Some(7));
        assert_eq!(CallKind::ListInspect.for_bollard(), LIST_INSPECT);
    }
}
