//! Health monitor and types, adapted from shadow-terminal/shadow-core health API.
//! Provides GET /health endpoint with uptime and component status.

use std::{
    pin::Pin,
    task::{Context, Poll},
    time::Duration,
};

use either::Either;
use futures::{channel::mpsc, channel::oneshot, Future, Stream};
use log::{debug, trace, warn};
use pin_project::pin_project;
use serde::{Serialize, Serializer};
use time::OffsetDateTime;

/// Period for the health tick stream; the monitor is woken at least this often to refresh
/// uptime and run stale checks when all other inputs are idle.
const HEALTH_TICK_PERIOD: Duration = Duration::from_secs(30);

/// Returns a stream that yields `()` every [HEALTH_TICK_PERIOD] and the future that drives it.
/// Spawn the returned future and pass the stream into [HealthMonitor::new] so the monitor is
/// woken periodically even when engine/node/API streams are idle.
pub fn health_tick_stream() -> (
    mpsc::UnboundedReceiver<()>,
    impl Future<Output = ()> + Send + 'static,
) {
    let (tx, rx) = mpsc::unbounded();
    let period = HEALTH_TICK_PERIOD;
    let driver = async move {
        let mut interval = tokio::time::interval(period);
        loop {
            interval.tick().await;
            if tx.unbounded_send(()).is_err() {
                break;
            }
        }
    };
    (rx, driver)
}

/// Request to get current health state; sender receives the response.
pub struct GetHealth<EngineStatus, NodeStatus>(pub oneshot::Sender<Health<EngineStatus, NodeStatus>>);

/// HealthMonitor is a perpetual Future that monitors component health and serves health requests.
///
/// It processes four types of inputs:
/// 1. **Engine updates**: status updates from the execution engine
/// 2. **Node updates**: status updates from chain sync / node
/// 3. **Health requests from API**: GetHealth requests that need the current health state
/// 4. **Periodic tick**: wakes the task on an interval so uptime and stale checks run even when
///    all other streams are idle (use [health_tick_stream]).
///
/// If any input stream closes ([Poll::Ready(None)]), the future completes with `()` so the task
/// does not hang without a wake source.
///
/// Components that haven't updated within MAX_IDLE_DURATION are automatically marked as Stale.
///
/// Engine status is tracked per stream: once any stream sends NoFunding, health shows NoFunding
/// until that stream sends Ok. So the failed stream must "heal" before overall status becomes Ok.
#[pin_project]
pub struct HealthMonitor<FromEngine, FromNode, FromAPI, FromTick, EngineStatus, NodeStatus> {
    #[pin]
    from_engine: FromEngine,
    #[pin]
    from_node: FromNode,
    #[pin]
    from_api: FromAPI,
    #[pin]
    from_tick: FromTick,
    state: Health<EngineStatus, NodeStatus>,
    start_time: OffsetDateTime,
    /// Last status from each execution stream. Displayed engine status = worst over this vec.
    per_stream_status: Vec<EngineStatus>,
}

impl<FromEngine, FromNode, FromAPI, FromTick, EngineStatus, NodeStatus>
    HealthMonitor<FromEngine, FromNode, FromAPI, FromTick, EngineStatus, NodeStatus>
where
    EngineStatus: Clone + Default + AggregateWorst,
    NodeStatus: Clone + Default,
{
    /// Creates a health monitor. `num_engine_streams` is the number of concurrent execution
    /// streams; each must send `(StreamId, EngineStatus)` with `StreamId in 0..num_engine_streams`.
    pub fn new(
        from_engine: FromEngine,
        from_node: FromNode,
        from_api: FromAPI,
        from_tick: FromTick,
        num_engine_streams: usize,
    ) -> Self {
        let now = OffsetDateTime::now_utc();
        let per_stream_status = (0..num_engine_streams).map(|_| EngineStatus::default()).collect();
        Self {
            from_engine,
            from_node,
            from_api,
            from_tick,
            state: Health {
                uptime_secs: 0,
                engine: ComponentState {
                    status: Either::Left(EngineStatus::default()),
                    last_updated: now,
                },
                node: ComponentState {
                    status: Either::Left(NodeStatus::default()),
                    last_updated: now,
                },
            },
            start_time: now,
            per_stream_status,
        }
    }

    /// Update displayed engine status from per-stream state: worst across all streams.
    fn refresh_engine_display(
        per_stream_status: &[EngineStatus],
        state: &mut Health<EngineStatus, NodeStatus>,
        last_updated: OffsetDateTime,
    ) where
        EngineStatus: AggregateWorst,
    {
        let worst = per_stream_status
            .iter()
            .fold(EngineStatus::default(), |a, b| a.worst(b.clone()));
        state.engine.status = Either::Left(worst);
        state.engine.last_updated = last_updated;
    }
}

impl<FromEngine, FromNode, FromAPI, FromTick, EngineStatus, NodeStatus> Future
    for HealthMonitor<FromEngine, FromNode, FromAPI, FromTick, EngineStatus, NodeStatus>
where
    FromEngine: Stream<Item = (StreamId, EngineStatus)> + Unpin,
    FromNode: Stream<Item = NodeStatus> + Unpin,
    FromAPI: Stream<Item = GetHealth<EngineStatus, NodeStatus>> + Unpin,
    FromTick: Stream<Item = ()> + Unpin,
    EngineStatus: Clone + AggregateWorst,
    NodeStatus: Clone + Default,
{
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut this = self.project();

        loop {
            let mut made_progress = false;

            let now = OffsetDateTime::now_utc();
            let uptime_duration = now - *this.start_time;
            this.state.uptime_secs = uptime_duration.whole_seconds().try_into().unwrap_or(0_u64);

            if is_component_stale(this.state.engine.last_updated, now) && this.state.engine.status.is_left() {
                warn!("Engine component has been idle for too long, marking as stale");
                this.state.engine.status = Either::Right(Stale);
                made_progress = true;
            }

            if is_component_stale(this.state.node.last_updated, now) && this.state.node.status.is_left() {
                warn!("Node component has been idle for too long, marking as stale");
                this.state.node.status = Either::Right(Stale);
                made_progress = true;
            }

            match this.from_engine.as_mut().poll_next(cx) {
                Poll::Ready(Some((stream_id, status))) => {
                    made_progress = true;
                    let now = OffsetDateTime::now_utc();
                    debug!("Received engine status update for stream {}", stream_id);
                    let idx = stream_id as usize;
                    if idx < this.per_stream_status.len() {
                        this.per_stream_status[idx] = status;
                        Self::refresh_engine_display(this.per_stream_status, this.state, now);
                    } else {
                        warn!(
                            "Received engine status update for invalid stream id {} (max index {})",
                            stream_id,
                            this.per_stream_status.len().saturating_sub(1),
                        );
                    }
                }
                Poll::Ready(None) => {
                    trace!("Health monitor stopping: engine stream closed");
                    return Poll::Ready(());
                }
                Poll::Pending => {}
            }

            match this.from_node.as_mut().poll_next(cx) {
                Poll::Ready(Some(status)) => {
                    made_progress = true;
                    let now = OffsetDateTime::now_utc();
                    debug!("Received node status update");
                    this.state.node.status = Either::Left(status);
                    this.state.node.last_updated = now;
                }
                Poll::Ready(None) => {
                    trace!("Health monitor stopping: node stream closed");
                    return Poll::Ready(());
                }
                Poll::Pending => {}
            }

            match this.from_api.as_mut().poll_next(cx) {
                Poll::Ready(Some(request)) => {
                    made_progress = true;
                    debug!("Received health request from API");
                    let GetHealth(sender) = request;
                    let _ = sender.send(this.state.clone());
                }
                Poll::Ready(None) => {
                    trace!("Health monitor stopping: API stream closed");
                    return Poll::Ready(());
                }
                Poll::Pending => {}
            }

            match this.from_tick.as_mut().poll_next(cx) {
                Poll::Ready(Some(())) => {
                    made_progress = true;
                }
                Poll::Ready(None) => {
                    trace!("Health monitor stopping: tick stream closed (no guaranteed wakeups)");
                    return Poll::Ready(());
                }
                Poll::Pending => {}
            }

            if !made_progress {
                return Poll::Pending;
            }
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Stale;

/// Serialization-only wrappers so component status always has a stable JSON shape:
/// - `{ "state": "Stale" }` when the component is stale
/// - `{ "state": "Ok", "details": <Status> }` when the component has reported status
mod status_shape {
    use serde::Serialize;

    #[derive(Serialize)]
    pub struct StaleVariant {
        pub state: &'static str,
    }

    #[derive(Serialize)]
    pub struct OkVariant<D> {
        pub state: &'static str,
        pub details: D,
    }

    pub const STALE: StaleVariant = StaleVariant { state: "Stale" };
    pub const OK: &str = "Ok";
}

fn serialize_status<S, Status>(status: &Either<Status, Stale>, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
    Status: Serialize,
{
    match status {
        Either::Left(s) => status_shape::OkVariant {
            state: status_shape::OK,
            details: s,
        }
        .serialize(serializer),
        Either::Right(_) => status_shape::STALE.serialize(serializer),
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(bound(serialize = "Status: Serialize"))]
pub struct ComponentState<Status> {
    #[serde(serialize_with = "serialize_status")]
    pub status: Either<Status, Stale>,
    #[serde(with = "time::serde::timestamp")]
    pub last_updated: OffsetDateTime,
}

/// Maximum duration of idle time for a component before it is considered stale (10 minutes).
const MAX_IDLE_DURATION: time::Duration = time::Duration::minutes(10);

/// Returns true if a component should be considered stale (idle longer than [MAX_IDLE_DURATION]).
/// Parameterized by `now` so callers (including tests) can inject time.
pub(crate) fn is_component_stale(last_updated: OffsetDateTime, now: OffsetDateTime) -> bool {
    (now - last_updated) > MAX_IDLE_DURATION
}

/// Identifies a single execution stream. Health shows NoFunding until every stream that
/// reported NoFunding has since reported Ok.
pub type StreamId = u8;

/// Trait for engine status types that support "worst among streams" aggregation.
pub trait AggregateWorst: Clone + Default {
    fn worst(self, other: Self) -> Self;
}

#[derive(Debug, Clone, Serialize)]
#[serde(bound(serialize = "EngineStatus: Serialize, NodeStatus: Serialize"))]
pub struct Health<EngineStatus, NodeStatus> {
    /// Uptime in seconds (serialized as "uptime" for API compatibility).
    #[serde(rename = "uptime")]
    pub uptime_secs: u64,
    pub engine: ComponentState<EngineStatus>,
    pub node: ComponentState<NodeStatus>,
}

/// Engine status for health response and engine events (single type for Ok and NoFunding).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "PascalCase")]
pub enum EngineStatus {
    Ok,
    NoFunding,
}

impl EngineStatus {
    pub fn ok() -> Self {
        Self::Ok
    }
}

impl AggregateWorst for EngineStatus {
    fn worst(self, other: Self) -> Self {
        match (self, other) {
            (Self::NoFunding, _) | (_, Self::NoFunding) => Self::NoFunding,
            _ => Self::Ok,
        }
    }
}

impl Default for EngineStatus {
    fn default() -> Self {
        Self::Ok
    }
}

/// Agent node status for health response.
#[derive(Debug, Clone, Serialize)]
pub struct AgentNodeStatus {
    pub status: &'static str,
}

impl AgentNodeStatus {
    pub fn ok() -> Self {
        Self { status: "Ok" }
    }
}

impl Default for AgentNodeStatus {
    fn default() -> Self {
        Self::ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Replicates the aggregation used in [HealthMonitor::refresh_engine_display] so we can test it.
    fn aggregate_worst_across_streams(per_stream: &[EngineStatus]) -> EngineStatus {
        per_stream
            .iter()
            .fold(EngineStatus::default(), |a, b| a.worst(*b))
    }

    #[test]
    fn worst_status_aggregation_all_ok() {
        assert_eq!(
            aggregate_worst_across_streams(&[EngineStatus::Ok]),
            EngineStatus::Ok
        );
        assert_eq!(
            aggregate_worst_across_streams(&[EngineStatus::Ok, EngineStatus::Ok]),
            EngineStatus::Ok
        );
    }

    #[test]
    fn worst_status_aggregation_any_nofunding_is_worst() {
        assert_eq!(
            aggregate_worst_across_streams(&[EngineStatus::NoFunding]),
            EngineStatus::NoFunding
        );
        assert_eq!(
            aggregate_worst_across_streams(&[EngineStatus::Ok, EngineStatus::NoFunding]),
            EngineStatus::NoFunding
        );
        assert_eq!(
            aggregate_worst_across_streams(&[EngineStatus::NoFunding, EngineStatus::Ok]),
            EngineStatus::NoFunding
        );
        assert_eq!(
            aggregate_worst_across_streams(&[EngineStatus::Ok, EngineStatus::NoFunding, EngineStatus::Ok,]),
            EngineStatus::NoFunding
        );
    }

    #[test]
    fn worst_status_aggregation_empty_streams_defaults_ok() {
        assert_eq!(aggregate_worst_across_streams(&[]), EngineStatus::Ok);
    }

    #[test]
    fn stale_not_marked_when_within_max_idle() {
        let now = OffsetDateTime::now_utc();
        let last_updated = now - time::Duration::minutes(5);
        assert!(!is_component_stale(last_updated, now));
        // 9 min 59 sec is still within the 10 min window
        let last_updated = now - (time::Duration::minutes(9) + time::Duration::seconds(59));
        assert!(!is_component_stale(last_updated, now));
    }

    #[test]
    fn stale_marked_after_max_idle_duration() {
        let now = OffsetDateTime::now_utc();
        // Just over 10 minutes
        let last_updated = now - (time::Duration::minutes(10) + time::Duration::seconds(1));
        assert!(is_component_stale(last_updated, now));
        let last_updated = now - time::Duration::minutes(11);
        assert!(is_component_stale(last_updated, now));
    }

    #[test]
    fn stale_exactly_at_boundary() {
        let now = OffsetDateTime::now_utc();
        // Exactly MAX_IDLE_DURATION uses strict inequality (>), so not stale yet
        let last_updated = now - time::Duration::minutes(10);
        assert!(!is_component_stale(last_updated, now));
    }
}
