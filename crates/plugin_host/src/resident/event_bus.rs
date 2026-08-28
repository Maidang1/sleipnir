//! Fan-out of [`HostEvent`] to subscribed plugins (ADR-0016 §2, §4).
//!
//! `SubscribeEvents` is continuous observation, not a snapshot. Delivery is
//! gated **here**, not at the call site: a plugin that was not granted that
//! capability never sees an event, no matter who called
//! [`Supervisor::broadcast`](super::Supervisor::broadcast). Narrowing uses
//! [`HostEvent::matches`]; this module does not reimplement the filter.
//!
//! A plugin that will not read must not grow host memory or delay a run
//! finishing. Events use the existing `try_send` write queue; overflow is
//! dropped and counted on the connection. Per-plugin order is enqueue order,
//! which is the caller's emission order — a plugin cannot observe
//! `RunFinished` before the `RunStarted` that was broadcast first.
//!
//! `RunStarted.command` is redacted with `run_ledger::redact` before enqueue
//! so a missed redact at the capture site cannot leak a secret onto the wire.

use super::session::Session;
use plugin_protocol::v2::HostEvent;
use std::sync::Arc;

/// Outcome of offering one event to one connection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Delivery {
    Delivered {
        id: plugin_protocol::v2::MessageId,
    },
    /// No `SubscribeEvents` grant, or the connection's [`EventFilter`] excluded
    /// this event. The plugin is not told that an event existed.
    Filtered,
    /// Write queue was full. Counted on the connection; not retried.
    Dropped,
    /// Connection is dead or shutting down. Broadcast continues to others.
    Skipped,
}

/// One broadcast's per-plugin outcomes, in `plugin_id` order (stable for
/// tests). Delivery to each plugin is still sequential in emission order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BroadcastReport {
    pub outcomes: Vec<(String, Delivery)>,
}

impl BroadcastReport {
    pub fn delivered(&self) -> usize {
        self.outcomes
            .iter()
            .filter(|(_, d)| matches!(d, Delivery::Delivered { .. }))
            .count()
    }

    pub fn dropped(&self) -> usize {
        self.outcomes
            .iter()
            .filter(|(_, d)| matches!(d, Delivery::Dropped))
            .count()
    }

    pub fn filtered(&self) -> usize {
        self.outcomes
            .iter()
            .filter(|(_, d)| matches!(d, Delivery::Filtered))
            .count()
    }

    pub fn skipped(&self) -> usize {
        self.outcomes
            .iter()
            .filter(|(_, d)| matches!(d, Delivery::Skipped))
            .count()
    }
}

/// Deliver `event` to every live session. Never blocks: a full queue is a
/// drop, a dead connection is a skip, a missing grant is a filter.
///
/// `SubscribeEvents` is checked per connection inside [`Session::receive_event`].
pub fn fan_out(
    sessions: impl IntoIterator<Item = Arc<Session>>,
    event: HostEvent,
) -> BroadcastReport {
    let event = redact_run_started(event);
    let mut sessions: Vec<Arc<Session>> = sessions.into_iter().collect();
    sessions.sort_by(|a, b| a.plugin_id.cmp(&b.plugin_id));
    let mut outcomes = Vec::with_capacity(sessions.len());
    for session in sessions {
        let outcome = session.receive_event(&event);
        outcomes.push((session.plugin_id.clone(), outcome));
    }
    BroadcastReport { outcomes }
}

fn redact_run_started(event: HostEvent) -> HostEvent {
    match event {
        HostEvent::RunStarted {
            run_id,
            pane,
            command,
            cwd,
        } => HostEvent::RunStarted {
            run_id,
            pane,
            command: run_ledger::redact_command(&command),
            cwd,
        },
        other => other,
    }
}

#[cfg(test)]
mod redact_tests {
    use super::redact_run_started;
    use plugin_protocol::v2::HostEvent;
    use uuid::Uuid;

    #[test]
    fn run_started_command_is_redacted_before_anyone_sees_it() {
        let event = HostEvent::RunStarted {
            run_id: Uuid::nil(),
            pane: Uuid::nil(),
            command: "AWS_SECRET_ACCESS_KEY=supersecret aws s3 ls".into(),
            cwd: None,
        };
        let HostEvent::RunStarted { command, .. } = redact_run_started(event) else {
            panic!("expected RunStarted");
        };
        assert!(
            !command.contains("supersecret"),
            "raw secret must not survive redact: {command}"
        );
    }
}
