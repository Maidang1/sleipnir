//! Concurrency-safe in-memory session/task registry.
//!
//! Two queues:
//! - **Effects** — durable adapter intents with payloads and sequence ids.
//!   Peek without consuming; cleared only by adapter acknowledgement.
//! - **Facts** — coordinator-facing event ring with per-reader cursors and
//!   drop accounting.
//!
//! `apply` never drains facts. Requests acknowledge acceptance, never
//! execution.

use std::collections::{BTreeMap, VecDeque};
use std::path::Path;
use std::sync::{Arc, Mutex};

use uuid::Uuid;

use crate::protocol::{
    AdapterUpdate, AgentKind, AgentSessionId, CoordinationTaskId, Effect, EffectBody, Event, Fact,
    FactBatch, RegistryStats, Request, Response, SessionSnapshot, TaskSnapshot, TaskStatus,
    WireRequest, WireResponse, Writer,
};

pub const MAX_SESSIONS: usize = 32;
pub const MAX_CLOSED_SESSIONS: usize = 32;
pub const MAX_TASKS: usize = 256;
pub const MAX_PROMPT_CHARS: usize = 8 * 1024;
pub const MAX_NAME_CHARS: usize = 64;
pub const MAX_ARGS: usize = 16;
pub const MAX_ARG_CHARS: usize = 256;
pub const MAX_CWD_CHARS: usize = 1024;
pub const MAX_RESULT_CHARS: usize = 64 * 1024;
pub const MAX_DETAIL_CHARS: usize = 8 * 1024;
pub const MAX_FACTS: usize = 128;
pub const MAX_EFFECTS: usize = 128;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Limits {
    pub max_sessions: usize,
    pub max_closed_sessions: usize,
    pub max_tasks: usize,
    pub max_prompt_chars: usize,
    pub max_name_chars: usize,
    pub max_args: usize,
    pub max_arg_chars: usize,
    pub max_cwd_chars: usize,
    pub max_result_chars: usize,
    pub max_detail_chars: usize,
    pub max_facts: usize,
    pub max_effects: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_sessions: MAX_SESSIONS,
            max_closed_sessions: MAX_CLOSED_SESSIONS,
            max_tasks: MAX_TASKS,
            max_prompt_chars: MAX_PROMPT_CHARS,
            max_name_chars: MAX_NAME_CHARS,
            max_args: MAX_ARGS,
            max_arg_chars: MAX_ARG_CHARS,
            max_cwd_chars: MAX_CWD_CHARS,
            max_result_chars: MAX_RESULT_CHARS,
            max_detail_chars: MAX_DETAIL_CHARS,
            max_facts: MAX_FACTS,
            max_effects: MAX_EFFECTS,
        }
    }
}

#[derive(Clone)]
pub struct Registry {
    inner: Arc<Mutex<Inner>>,
}

struct Inner {
    limits: Limits,
    sessions: BTreeMap<AgentSessionId, Session>,
    tasks: BTreeMap<CoordinationTaskId, Task>,
    facts: VecDeque<Fact>,
    effects: VecDeque<Effect>,
    next_fact_seq: u64,
    next_effect_seq: u64,
    stats: RegistryStats,
}

struct Session {
    id: AgentSessionId,
    kind: AgentKind,
    cwd: String,
    name: Option<String>,
    writer: Writer,
    open: bool,
    pane: Option<Uuid>,
    tasks: Vec<CoordinationTaskId>,
    closed_at_ms: Option<u64>,
    /// Orders delivery against ownership/terminal mutations without holding `Inner` during I/O.
    delivery: Arc<Mutex<()>>,
}

struct Task {
    id: CoordinationTaskId,
    session: AgentSessionId,
    status: TaskStatus,
    accepted_at_ms: u64,
    result: Option<String>,
    detail: Option<String>,
}

impl Registry {
    pub fn new() -> Self {
        Self::with_limits(Limits::default())
    }

    pub fn with_limits(limits: Limits) -> Self {
        Self {
            inner: Arc::new(Mutex::new(Inner {
                limits,
                sessions: BTreeMap::new(),
                tasks: BTreeMap::new(),
                facts: VecDeque::new(),
                effects: VecDeque::new(),
                next_fact_seq: 1,
                next_effect_seq: 1,
                stats: RegistryStats::default(),
            })),
        }
    }

    pub fn handle(&self, req: WireRequest, now_ms: u64) -> WireResponse {
        let order = {
            let inner = self.lock();
            let session = match &req.body {
                Request::Prompt { session, .. }
                | Request::Interrupt { session }
                | Request::HumanTakeover { session }
                | Request::Close { session }
                | Request::ReportSessionClosed { session } => Some(*session),
                Request::ReportRunning { task }
                | Request::ReportAwaitingHuman { task, .. }
                | Request::ReportResult { task, .. } => {
                    inner.tasks.get(task).map(|task| task.session)
                }
                _ => None,
            };
            session.and_then(|id| inner.sessions.get(&id).map(|s| s.delivery.clone()))
        };
        let _ordered = order
            .as_ref()
            .map(|lock| lock.lock().unwrap_or_else(|p| p.into_inner()));
        let body = match self.lock().handle(req.body, now_ms) {
            Ok(body) => body,
            Err(message) => Response::Error { message },
        };
        WireResponse { id: req.id, body }
    }

    /// Apply an adapter/host update. Does **not** drain facts or effects.
    pub fn apply(&self, update: AdapterUpdate, now_ms: u64) -> Result<(), String> {
        let order = {
            let inner = self.lock();
            let session = match &update {
                AdapterUpdate::TaskRunning { task }
                | AdapterUpdate::TaskAwaitingHuman { task }
                | AdapterUpdate::TaskResult { task, .. } => {
                    inner.tasks.get(task).map(|t| t.session)
                }
                AdapterUpdate::SessionClosed { session }
                | AdapterUpdate::ReleaseToCoordinator { session }
                | AdapterUpdate::BindPane { session, .. } => Some(*session),
                AdapterUpdate::PromptDelivered { seq }
                | AdapterUpdate::InterruptDelivered { seq }
                | AdapterUpdate::FocusDelivered { seq }
                | AdapterUpdate::CloseDelivered { seq }
                | AdapterUpdate::DeliveryFailed { seq } => inner
                    .effects
                    .iter()
                    .find(|e| e.seq == *seq)
                    .map(|e| e.body.session()),
            };
            session.and_then(|id| inner.sessions.get(&id).map(|s| s.delivery.clone()))
        };
        let _ordered = order
            .as_ref()
            .map(|lock| lock.lock().unwrap_or_else(|p| p.into_inner()));
        self.lock().apply(update, now_ms)
    }

    /// Claim an exact queued intent under the session delivery lease. The callback
    /// may perform host I/O, but must not synchronously mutate this same session.
    /// Reads and other sessions remain available throughout the callback.
    pub fn deliver<T>(
        &self,
        effect: &Effect,
        deliver: impl FnOnce(&Delivery<'_>) -> T,
    ) -> Result<T, String> {
        let order = self.lock().session(effect.body.session())?.delivery.clone();
        let _ordered = order.lock().unwrap_or_else(|p| p.into_inner());
        self.lock().validate_effect(effect)?;
        Ok(deliver(&Delivery { registry: self }))
    }

    /// Pending adapter intents, oldest first. Does not consume them.
    pub fn peek_effects(&self) -> Vec<Effect> {
        self.lock().effects.iter().cloned().collect()
    }

    /// Coordinator-facing facts with `seq > cursor`. Independent of effects.
    pub fn facts_since(&self, cursor: u64) -> FactBatch {
        self.lock().facts_since(cursor)
    }

    pub fn stats(&self) -> RegistryStats {
        self.lock().stats
    }

    /// Host-local summary read. Unlike wire pages, this can return every session.
    pub fn session_summaries(&self) -> Vec<SessionSnapshot> {
        let inner = self.lock();
        inner
            .sessions
            .values()
            .filter_map(|session| {
                inner
                    .snapshot_page(session, session.tasks.len().saturating_sub(1), true)
                    .ok()
            })
            .collect()
    }

    /// Host-local bounded preview, never a second copy of a complete result.
    pub fn result_excerpt(&self, task: CoordinationTaskId, max_chars: usize) -> Option<String> {
        self.lock()
            .tasks
            .get(&task)?
            .result
            .as_deref()
            .map(|text| text.chars().take(max_chars).collect())
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|p| p.into_inner())
    }
}

/// Acknowledgements issued while the session delivery lease is still held.
/// Prevents a second consumer from replaying a successful-but-not-yet-acked effect.
pub struct Delivery<'a> {
    registry: &'a Registry,
}

impl Delivery<'_> {
    pub fn apply(&self, update: AdapterUpdate, now_ms: u64) -> Result<(), String> {
        self.registry.lock().apply(update, now_ms)
    }
}

impl Default for Registry {
    fn default() -> Self {
        Self::new()
    }
}

impl Inner {
    fn validate_effect(&self, effect: &Effect) -> Result<(), String> {
        if !self.effects.iter().any(|pending| pending == effect) {
            return Err("obsolete effect".into());
        }
        let session = self.session(effect.body.session())?;
        if !session.open {
            return Err("session is closed".into());
        }
        if !matches!(effect.body, EffectBody::FocusRequested { .. })
            && session.writer != Writer::Coordinator
        {
            return Err("human owns this session".into());
        }
        let task = match effect.body {
            EffectBody::LaunchRequested { task, .. } | EffectBody::PromptRequested { task, .. } => {
                Some(task)
            }
            EffectBody::InterruptRequested { task, .. } => task,
            _ => None,
        };
        if let Some(task) = task {
            let status = self.task(task)?.status;
            if !status.is_in_flight() {
                return Err("task is not in flight".into());
            }
            if matches!(effect.body, EffectBody::PromptRequested { .. })
                && status != TaskStatus::Dispatching
            {
                return Err("prompt is no longer dispatchable".into());
            }
        }
        Ok(())
    }

    fn handle(&mut self, req: Request, now_ms: u64) -> Result<Response, String> {
        match req {
            Request::List => self.list_page(0),
            Request::ListPage { offset } => self.list_page(offset),
            Request::InspectPage { session, offset } => Ok(Response::Inspect {
                session: self.snapshot_page(self.session(session)?, offset, false)?,
            }),
            Request::EffectsPage { cursor } => Ok(self.effects_page(cursor)),
            Request::Launch {
                kind,
                cwd,
                name,
                args,
            } => self.launch(kind, cwd, name, args, now_ms),
            Request::Prompt { session, text } => self.prompt(session, text, now_ms),
            Request::Wait { task } => self.wait_page(task, 0),
            Request::WaitPage { task, offset } => self.wait_page(task, offset),
            Request::Interrupt { session } => self.interrupt(session),
            Request::Focus { session } => self.focus(session),
            Request::Inspect { session } => self.inspect(session),
            Request::HumanTakeover { session } => self.human_takeover(session),
            Request::Close { session } => self.close_request(session),
            Request::ReportRunning { task } => self.report_running(task),
            Request::ReportAwaitingHuman { task, detail } => {
                self.report_awaiting_human(task, detail)
            }
            Request::ReportResult { task, text } => self.report_result(task, text),
            Request::ReportSessionClosed { session } => self.report_session_closed(session, now_ms),
            Request::Effects => Ok(self.effects_page(0)),
            Request::Facts { cursor } => {
                let batch = self.facts_since(cursor);
                Ok(Response::Facts {
                    facts: batch.facts,
                    next_cursor: batch.next_cursor,
                    missed: batch.missed,
                })
            }
        }
    }

    fn launch(
        &mut self,
        kind: AgentKind,
        cwd: String,
        name: Option<String>,
        args: Vec<String>,
        now_ms: u64,
    ) -> Result<Response, String> {
        self.check_cwd(&cwd)?;
        self.check_name(name.as_deref())?;
        self.check_args(&args)?;
        self.prune(now_ms);
        let open = self.sessions.values().filter(|s| s.open).count();
        if open >= self.limits.max_sessions {
            return Err(format!(
                "session limit {} reached",
                self.limits.max_sessions
            ));
        }
        self.ensure_task_capacity()?;
        self.ensure_effect_capacity()?;
        let session_id = AgentSessionId::new();
        let task_id = CoordinationTaskId::new();
        self.sessions.insert(
            session_id,
            Session {
                id: session_id,
                kind,
                cwd: cwd.clone(),
                name: name.clone(),
                writer: Writer::Coordinator,
                open: true,
                pane: None,
                tasks: vec![task_id],
                closed_at_ms: None,
                delivery: Arc::new(Mutex::new(())),
            },
        );
        self.tasks.insert(
            task_id,
            Task {
                id: task_id,
                session: session_id,
                status: TaskStatus::Dispatching,
                accepted_at_ms: now_ms,
                result: None,
                detail: None,
            },
        );
        self.push_fact(Event::SessionOpened {
            session: session_id,
        });
        self.push_fact(Event::TaskAccepted {
            session: session_id,
            task: task_id,
        });
        self.push_fact(Event::TaskDispatching { task: task_id });
        self.push_effect(EffectBody::LaunchRequested {
            session: session_id,
            task: task_id,
            kind,
            cwd,
            name,
            args,
        });
        Ok(Response::LaunchAccepted {
            session: session_id,
            task: task_id,
        })
    }

    fn prompt(
        &mut self,
        session_id: AgentSessionId,
        text: String,
        now_ms: u64,
    ) -> Result<Response, String> {
        self.check_prompt(&text)?;
        self.prune(now_ms);
        let session = self.session(session_id)?;
        if !session.open {
            return Err("session is closed".into());
        }
        if session.writer != Writer::Coordinator {
            return Err("human owns this session".into());
        }
        let existing = session.tasks.clone();
        for id in &existing {
            if let Some(task) = self.tasks.get(id) {
                if task.status == TaskStatus::AwaitingHuman {
                    return Err("coordinator cannot answer native approvals".into());
                }
                if task.status.is_in_flight() {
                    return Err("session already has an in-flight task".into());
                }
            }
        }
        self.ensure_task_capacity()?;
        let before = self.effects.len();
        self.effects.retain(|effect| {
            !matches!(effect.body,
            EffectBody::InterruptRequested { session, task: None } if session == session_id)
        });
        self.stats.effects_drained += (before - self.effects.len()) as u64;
        self.ensure_effect_capacity()?;
        let task_id = CoordinationTaskId::new();
        self.session_mut(session_id)?.tasks.push(task_id);
        self.tasks.insert(
            task_id,
            Task {
                id: task_id,
                session: session_id,
                status: TaskStatus::Dispatching,
                accepted_at_ms: now_ms,
                result: None,
                detail: None,
            },
        );
        self.push_fact(Event::TaskAccepted {
            session: session_id,
            task: task_id,
        });
        self.push_fact(Event::TaskDispatching { task: task_id });
        self.push_effect(EffectBody::PromptRequested {
            session: session_id,
            task: task_id,
            text,
        });
        Ok(Response::PromptAccepted { task: task_id })
    }

    fn wait_page(&self, task_id: CoordinationTaskId, offset: usize) -> Result<Response, String> {
        let task = self.task(task_id)?;
        let (result, next_result_offset) = self.wait_result_page(
            task_id,
            task.status,
            task.result.as_deref(),
            task.detail.as_deref(),
            offset,
        )?;
        Ok(Response::Wait {
            task: task_id,
            status: task.status,
            terminal: task.status.is_terminal(),
            result,
            detail: task.detail.clone(),
            next_result_offset,
        })
    }

    fn report_running(&mut self, task_id: CoordinationTaskId) -> Result<Response, String> {
        self.task_running(task_id)?;
        Ok(Response::ReportedRunning { task: task_id })
    }

    fn report_awaiting_human(
        &mut self,
        task_id: CoordinationTaskId,
        detail: Option<String>,
    ) -> Result<Response, String> {
        self.task_awaiting_human(task_id, detail)?;
        Ok(Response::ReportedAwaitingHuman { task: task_id })
    }

    fn report_result(
        &mut self,
        task_id: CoordinationTaskId,
        text: String,
    ) -> Result<Response, String> {
        self.record_result(task_id, text)?;
        Ok(Response::ReportedResult { task: task_id })
    }

    fn report_session_closed(
        &mut self,
        session_id: AgentSessionId,
        now_ms: u64,
    ) -> Result<Response, String> {
        self.close_session(session_id, now_ms)?;
        Ok(Response::ReportedSessionClosed {
            session: session_id,
        })
    }

    fn interrupt(&mut self, session_id: AgentSessionId) -> Result<Response, String> {
        let (open, writer, ids) = {
            let session = self.session(session_id)?;
            (session.open, session.writer, session.tasks.clone())
        };
        if !open {
            return Err("session is closed".into());
        }
        if writer != Writer::Coordinator {
            return Err("human owns this session".into());
        }
        if self
            .effects
            .iter()
            .any(|e| matches!(&e.body, EffectBody::InterruptRequested { session, .. } if *session == session_id))
        {
            return Ok(Response::InterruptAccepted {
                session: session_id,
            });
        }
        self.ensure_effect_capacity()?;
        let task = ids.into_iter().find(|id| {
            self.tasks
                .get(id)
                .is_some_and(|task| task.status.is_in_flight())
        });
        if let Some(id) = task {
            self.task_mut(id)?.status = TaskStatus::Interrupting;
            self.push_fact(Event::TaskInterrupting { task: id });
        }
        self.push_effect(EffectBody::InterruptRequested {
            session: session_id,
            task,
        });
        Ok(Response::InterruptAccepted {
            session: session_id,
        })
    }

    fn focus(&mut self, session_id: AgentSessionId) -> Result<Response, String> {
        let (open, pane) = {
            let session = self.session(session_id)?;
            (session.open, session.pane)
        };
        if !open {
            return Err("session is closed".into());
        }
        if pane.is_none() {
            return Err("pane is not bound".into());
        }
        if self.effects.iter().any(
            |e| matches!(&e.body, EffectBody::FocusRequested { session } if *session == session_id),
        ) {
            return Ok(Response::FocusAccepted {
                session: session_id,
            });
        }
        self.ensure_effect_capacity()?;
        self.push_effect(EffectBody::FocusRequested {
            session: session_id,
        });
        Ok(Response::FocusAccepted {
            session: session_id,
        })
    }

    fn inspect(&self, session_id: AgentSessionId) -> Result<Response, String> {
        Ok(Response::Inspect {
            session: self.snapshot(self.session(session_id)?)?,
        })
    }

    fn human_takeover(&mut self, session_id: AgentSessionId) -> Result<Response, String> {
        let session = self.session_mut(session_id)?;
        if !session.open {
            return Err("session is closed".into());
        }
        if session.writer == Writer::Human {
            return Err("human already owns this session".into());
        }
        session.writer = Writer::Human;
        self.push_fact(Event::OwnershipChanged {
            session: session_id,
            writer: Writer::Human,
        });
        Ok(Response::TakenOver {
            session: session_id,
        })
    }

    fn close_request(&mut self, session_id: AgentSessionId) -> Result<Response, String> {
        let (open, writer) = {
            let session = self.session(session_id)?;
            (session.open, session.writer)
        };
        if !open {
            return Err("session is closed".into());
        }
        if writer != Writer::Coordinator {
            return Err("human owns this session".into());
        }
        if self.effects.iter().any(
            |e| matches!(&e.body, EffectBody::CloseRequested { session } if *session == session_id),
        ) {
            return Ok(Response::CloseAccepted {
                session: session_id,
            });
        }
        self.ensure_effect_capacity()?;
        self.push_effect(EffectBody::CloseRequested {
            session: session_id,
        });
        Ok(Response::CloseAccepted {
            session: session_id,
        })
    }

    fn apply(&mut self, update: AdapterUpdate, now_ms: u64) -> Result<(), String> {
        match update {
            AdapterUpdate::BindPane { seq, session, pane } => self.bind_pane(seq, session, pane),
            AdapterUpdate::PromptDelivered { seq } => self.prompt_delivered(seq),
            AdapterUpdate::InterruptDelivered { seq } => self.interrupt_delivered(seq),
            AdapterUpdate::FocusDelivered { seq } => {
                self.ack_exact(
                    seq,
                    |b| matches!(b, EffectBody::FocusRequested { .. }),
                    "focus_requested",
                )?;
                Ok(())
            }
            AdapterUpdate::CloseDelivered { seq } => {
                self.ack_exact(
                    seq,
                    |b| matches!(b, EffectBody::CloseRequested { .. }),
                    "close_requested",
                )?;
                Ok(())
            }
            AdapterUpdate::DeliveryFailed { seq } => self.delivery_failed(seq, now_ms),
            AdapterUpdate::TaskRunning { task } => self.task_running(task),
            AdapterUpdate::TaskAwaitingHuman { task } => self.task_awaiting_human(task, None),
            AdapterUpdate::TaskResult { task, text } => self.record_result(task, text),
            AdapterUpdate::SessionClosed { session } => self.close_session(session, now_ms),
            AdapterUpdate::ReleaseToCoordinator { session } => self.release(session),
        }
    }

    fn bind_pane(
        &mut self,
        seq: u64,
        session_id: AgentSessionId,
        pane: Uuid,
    ) -> Result<(), String> {
        let (open, existing) = {
            let s = self.session(session_id)?;
            (s.open, s.pane)
        };
        if !open {
            return Err("session is closed".into());
        }
        match existing {
            Some(bound) if bound == pane => {
                // Same pane is idempotent. Ack the launch effect if `seq` still
                // names it; an already-acked seq is not an error.
                match self.effects.iter().position(|e| e.seq == seq) {
                    None => Ok(()),
                    Some(pos) => match &self.effects[pos].body {
                        EffectBody::LaunchRequested { session, .. } if *session == session_id => {
                            self.effects.remove(pos);
                            Ok(())
                        }
                        other => Err(format!(
                            "effect {seq} is {}, not launch_requested for this session",
                            other.kind_name()
                        )),
                    },
                }
            }
            Some(_) => Err("pane is already bound".into()),
            None => {
                let pos = self
                    .effects
                    .iter()
                    .position(|e| e.seq == seq)
                    .ok_or_else(|| "unknown effect".to_string())?;
                match &self.effects[pos].body {
                    EffectBody::LaunchRequested { session, .. } if *session == session_id => {}
                    EffectBody::LaunchRequested { .. } => {
                        return Err("effect belongs to a different session".into());
                    }
                    other => {
                        return Err(format!(
                            "effect {seq} is {}, not launch_requested",
                            other.kind_name()
                        ));
                    }
                }
                self.effects.remove(pos);
                self.session_mut(session_id)?.pane = Some(pane);
                Ok(())
            }
        }
    }

    fn interrupt_delivered(&mut self, seq: u64) -> Result<(), String> {
        let effect = self.ack_exact(
            seq,
            |b| matches!(b, EffectBody::InterruptRequested { .. }),
            "interrupt_requested",
        )?;
        let EffectBody::InterruptRequested { task, .. } = effect.body else {
            return Ok(());
        };
        if let Some(task) = task {
            // A successful key write is not evidence that the worker stopped.
            // Retire only the captured assignment, with an explicitly unknown outcome.
            self.finish_task(task, TaskStatus::Unknown)?;
        }
        Ok(())
    }

    fn prompt_delivered(&mut self, seq: u64) -> Result<(), String> {
        let pos = self
            .effects
            .iter()
            .position(|e| e.seq == seq)
            .ok_or_else(|| "unknown effect".to_string())?;
        let task_id = match &self.effects[pos].body {
            EffectBody::PromptRequested { task, .. } => *task,
            other => {
                return Err(format!(
                    "effect {seq} is {}, not prompt_requested",
                    other.kind_name()
                ));
            }
        };
        let task = self.task(task_id)?;
        if !task.status.is_in_flight() {
            return Err("task is not in flight".into());
        }
        self.effects.remove(pos);
        Ok(())
    }

    fn task_running(&mut self, task_id: CoordinationTaskId) -> Result<(), String> {
        let t = self.task_mut(task_id)?;
        match t.status {
            TaskStatus::Accepted
            | TaskStatus::Dispatching
            | TaskStatus::AwaitingHuman
            | TaskStatus::Interrupting => {
                t.status = TaskStatus::Running;
                self.push_fact(Event::TaskRunning { task: task_id });
                Ok(())
            }
            TaskStatus::Running => Ok(()),
            other => Err(format!("task is {other:?}, not runnable")),
        }
    }

    fn task_awaiting_human(
        &mut self,
        task_id: CoordinationTaskId,
        detail: Option<String>,
    ) -> Result<(), String> {
        let detail = match detail {
            None => None,
            Some(s) if s.trim().is_empty() => None,
            Some(s) => {
                self.check_payload("detail", &s, self.limits.max_detail_chars, false)?;
                Some(s)
            }
        };
        let t = self.task_mut(task_id)?;
        if !t.status.is_in_flight() {
            return Err("task is not in flight".into());
        }
        t.status = TaskStatus::AwaitingHuman;
        t.detail = detail.clone();
        self.push_fact(Event::TaskAwaitingHuman {
            task: task_id,
            detail,
        });
        Ok(())
    }

    fn delivery_failed(&mut self, seq: u64, now_ms: u64) -> Result<(), String> {
        let effect = self
            .remove_effect(seq)
            .ok_or_else(|| "unknown effect".to_string())?;
        match effect.body {
            EffectBody::LaunchRequested { task, session, .. } => {
                self.finish_task(task, TaskStatus::FailedDelivery)?;
                if self.session(session)?.pane.is_none() {
                    self.close_session(session, now_ms)?;
                }
            }
            EffectBody::PromptRequested { task, .. } => {
                // A newer native report or interrupt supersedes an undelivered
                // prompt; retiring that prompt must preserve the approval/interrupt gate.
                if self.task(task)?.status == TaskStatus::Dispatching {
                    self.finish_task(task, TaskStatus::FailedDelivery)?;
                }
            }
            EffectBody::InterruptRequested {
                task: Some(task), ..
            } => {
                self.finish_task(task, TaskStatus::Unknown)?;
            }
            EffectBody::InterruptRequested { task: None, .. }
            | EffectBody::FocusRequested { .. }
            | EffectBody::CloseRequested { .. } => {}
        }
        Ok(())
    }

    /// Terminal state and stale-intent retirement are one registry mutation.
    fn finish_task(&mut self, id: CoordinationTaskId, status: TaskStatus) -> Result<(), String> {
        let task = self.task_mut(id)?;
        if !task.status.is_in_flight() {
            return Ok(());
        }
        task.status = status;
        let event = match status {
            TaskStatus::Settled => Event::TaskSettled { task: id },
            TaskStatus::Unknown => Event::TaskUnknown { task: id },
            TaskStatus::FailedDelivery => Event::TaskFailedDelivery { task: id },
            _ => unreachable!("terminal transition required"),
        };
        let before = self.effects.len();
        self.effects.retain(|effect| match effect.body {
            EffectBody::LaunchRequested { task, .. }
            | EffectBody::PromptRequested { task, .. }
            | EffectBody::InterruptRequested {
                task: Some(task), ..
            } => task != id,
            _ => true,
        });
        self.stats.effects_drained += (before - self.effects.len()) as u64;
        self.push_fact(event);
        Ok(())
    }

    fn record_result(&mut self, task_id: CoordinationTaskId, text: String) -> Result<(), String> {
        self.check_payload("result", &text, self.limits.max_result_chars, true)?;
        let task = self.task(task_id)?;
        match task.status {
            TaskStatus::Unknown | TaskStatus::FailedDelivery => {
                self.push_fact(Event::LateResult { task: task_id });
                Ok(())
            }
            TaskStatus::Settled => {
                self.push_fact(Event::DuplicateResult { task: task_id });
                Ok(())
            }
            TaskStatus::Accepted
            | TaskStatus::Dispatching
            | TaskStatus::Running
            | TaskStatus::AwaitingHuman
            | TaskStatus::Interrupting => {
                self.task_mut(task_id)?.result = Some(text);
                self.finish_task(task_id, TaskStatus::Settled)?;
                self.prune(0);
                Ok(())
            }
        }
    }

    fn close_session(&mut self, session_id: AgentSessionId, now_ms: u64) -> Result<(), String> {
        let session = self.session_mut(session_id)?;
        if !session.open {
            return Ok(());
        }
        session.open = false;
        session.closed_at_ms = Some(now_ms);
        let ids = session.tasks.clone();
        for id in ids {
            self.finish_task(id, TaskStatus::Unknown)?;
        }
        self.drain_effects_for_session(session_id);
        self.push_fact(Event::SessionClosed {
            session: session_id,
        });
        self.prune(now_ms);
        Ok(())
    }

    fn release(&mut self, session_id: AgentSessionId) -> Result<(), String> {
        let session = self.session_mut(session_id)?;
        if !session.open {
            return Err("session is closed".into());
        }
        if session.writer == Writer::Coordinator {
            return Err("coordinator already owns this session".into());
        }
        session.writer = Writer::Coordinator;
        self.push_fact(Event::OwnershipChanged {
            session: session_id,
            writer: Writer::Coordinator,
        });
        Ok(())
    }

    fn list_page(&self, offset: usize) -> Result<Response, String> {
        let mut agents = Vec::new();
        let mut bytes = 256;
        for session in self.sessions.values().skip(offset) {
            // Lists expose the latest assignment; inspect owns task history.
            let snapshot =
                self.snapshot_page(session, session.tasks.len().saturating_sub(1), true)?;
            let size = serde_json::to_vec(&snapshot)
                .map_err(|e| e.to_string())?
                .len()
                + 1;
            if !agents.is_empty() && bytes + size >= crate::MAX_LINE_BYTES {
                break;
            }
            bytes += size;
            agents.push(snapshot);
        }
        let next = offset.saturating_add(agents.len());
        Ok(Response::Agents {
            agents,
            next_offset: (next < self.sessions.len()).then_some(next),
        })
    }

    fn effects_page(&self, cursor: u64) -> Response {
        let mut effects = Vec::new();
        let mut bytes = 256;
        let mut next_cursor = None;
        for effect in self.effects.iter().filter(|e| e.seq > cursor) {
            let size = serde_json::to_vec(effect)
                .map(|line| line.len())
                .unwrap_or(crate::MAX_LINE_BYTES)
                + 1;
            if !effects.is_empty() && bytes + size >= crate::MAX_LINE_BYTES {
                next_cursor = effects.last().map(|e: &Effect| e.seq);
                break;
            }
            bytes += size;
            effects.push(effect.clone());
        }
        Response::Effects {
            effects,
            next_cursor,
        }
    }

    fn snapshot(&self, session: &Session) -> Result<SessionSnapshot, String> {
        self.snapshot_page(session, 0, false)
    }

    fn snapshot_page(
        &self,
        session: &Session,
        offset: usize,
        latest_only: bool,
    ) -> Result<SessionSnapshot, String> {
        let mut tasks = Vec::new();
        // Reserve the full encoded session metadata and envelope before history.
        let mut bytes = 8192;
        for id in session.tasks.iter().skip(offset) {
            let Some(task) = self.tasks.get(id) else {
                continue;
            };
            let summary = TaskSnapshot {
                task: task.id,
                session: task.session,
                status: task.status,
                accepted_at_ms: task.accepted_at_ms,
                result: None,
                detail: task
                    .detail
                    .as_deref()
                    .map(|detail| detail.chars().take(80).collect()),
            };
            let size = serde_json::to_vec(&summary)
                .map_err(|e| e.to_string())?
                .len()
                + 1;
            if !tasks.is_empty() && bytes + size >= crate::MAX_LINE_BYTES {
                break;
            }
            bytes += size;
            tasks.push(summary);
        }
        Ok(SessionSnapshot {
            session: session.id,
            kind: session.kind,
            cwd: session.cwd.clone(),
            name: session.name.clone(),
            writer: session.writer,
            open: session.open,
            pane: session.pane,
            next_task_offset: (!latest_only && offset + tasks.len() < session.tasks.len())
                .then_some(offset + tasks.len()),
            tasks,
        })
    }

    fn wait_result_page(
        &self,
        task_id: CoordinationTaskId,
        status: TaskStatus,
        result: Option<&str>,
        detail: Option<&str>,
        offset: usize,
    ) -> Result<(Option<String>, Option<usize>), String> {
        let Some(result) = result else {
            if offset == 0 {
                return Ok((None, None));
            }
            return Err("result offset out of range".into());
        };

        let mut boundaries: Vec<usize> = result.char_indices().map(|(i, _)| i).collect();
        boundaries.push(result.len());
        let total_chars = boundaries.len().saturating_sub(1);
        if offset > total_chars {
            return Err("result offset out of range".into());
        }
        if offset == total_chars {
            return Ok((Some(String::new()), None));
        }

        if let Some(full) = self.wait_result_candidate(
            task_id,
            status,
            detail,
            result,
            &boundaries,
            offset,
            total_chars,
            None,
        )? {
            return Ok((Some(full), None));
        }

        if offset + 1 > total_chars {
            return Err("wait result page did not advance".into());
        }
        if self
            .wait_result_candidate(
                task_id,
                status,
                detail,
                result,
                &boundaries,
                offset,
                offset + 1,
                Some(offset + 1),
            )?
            .is_none()
        {
            return Err("wait response exceeds line length cap".into());
        }

        let mut low = offset + 1;
        let mut high = total_chars;
        let mut best = offset + 1;
        while low <= high {
            let mid = low + (high - low) / 2;
            let next = (mid < total_chars).then_some(mid);
            if self
                .wait_result_candidate(
                    task_id,
                    status,
                    detail,
                    result,
                    &boundaries,
                    offset,
                    mid,
                    next,
                )?
                .is_some()
            {
                best = mid;
                low = mid.saturating_add(1);
            } else {
                high = mid.saturating_sub(1);
            }
        }

        let next = (best < total_chars).then_some(best);
        let page = self
            .wait_result_candidate(
                task_id,
                status,
                detail,
                result,
                &boundaries,
                offset,
                best,
                next,
            )?
            .ok_or_else(|| "wait result page did not fit".to_string())?;
        Ok((Some(page), next))
    }

    fn wait_result_candidate(
        &self,
        task_id: CoordinationTaskId,
        status: TaskStatus,
        detail: Option<&str>,
        result: &str,
        boundaries: &[usize],
        start: usize,
        end: usize,
        next_result_offset: Option<usize>,
    ) -> Result<Option<String>, String> {
        let slice = result
            .get(boundaries[start]..boundaries[end])
            .ok_or_else(|| "result offset split a code point".to_string())?;
        let response = WireResponse {
            id: u64::MAX,
            body: Response::Wait {
                task: task_id,
                status,
                terminal: status.is_terminal(),
                result: Some(slice.to_string()),
                detail: detail.map(str::to_string),
                next_result_offset,
            },
        };
        let encoded = serde_json::to_vec(&response).map_err(|err| err.to_string())?;
        if encoded.len() <= crate::MAX_LINE_BYTES {
            Ok(Some(slice.to_string()))
        } else {
            Ok(None)
        }
    }

    fn session(&self, id: AgentSessionId) -> Result<&Session, String> {
        self.sessions
            .get(&id)
            .ok_or_else(|| "unknown session".into())
    }

    fn session_mut(&mut self, id: AgentSessionId) -> Result<&mut Session, String> {
        self.sessions
            .get_mut(&id)
            .ok_or_else(|| "unknown session".into())
    }

    fn task(&self, id: CoordinationTaskId) -> Result<&Task, String> {
        self.tasks.get(&id).ok_or_else(|| "unknown task".into())
    }

    fn task_mut(&mut self, id: CoordinationTaskId) -> Result<&mut Task, String> {
        self.tasks.get_mut(&id).ok_or_else(|| "unknown task".into())
    }

    fn check_cwd(&self, cwd: &str) -> Result<(), String> {
        if cwd.trim().is_empty() {
            return Err("cwd is empty".into());
        }
        if cwd.contains('\0') {
            return Err("cwd contains NUL".into());
        }
        if cwd.chars().count() > self.limits.max_cwd_chars {
            return Err("cwd exceeds length cap".into());
        }
        if !Path::new(cwd).is_absolute() {
            return Err("cwd must be absolute".into());
        }
        Ok(())
    }

    fn check_name(&self, name: Option<&str>) -> Result<(), String> {
        let Some(name) = name else {
            return Ok(());
        };
        if name.trim().is_empty() {
            return Err("name is empty".into());
        }
        if name.contains('\0') {
            return Err("name contains NUL".into());
        }
        if name.chars().count() > self.limits.max_name_chars {
            return Err("name exceeds length cap".into());
        }
        let mut chars = name.chars();
        let Some(first) = chars.next() else {
            return Err("name is empty".into());
        };
        if !first.is_ascii_alphabetic() {
            return Err("name must start with a letter".into());
        }
        if !chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') {
            return Err("name must be letters, digits, '_' or '-'".into());
        }
        Ok(())
    }

    fn check_args(&self, args: &[String]) -> Result<(), String> {
        if args.len() > self.limits.max_args {
            return Err(format!("too many args (max {})", self.limits.max_args));
        }
        for arg in args {
            if arg.chars().count() > self.limits.max_arg_chars {
                return Err("arg exceeds length cap".into());
            }
            if arg.contains('\0') {
                return Err("arg contains NUL".into());
            }
        }
        Ok(())
    }

    fn check_prompt(&self, text: &str) -> Result<(), String> {
        if text.trim().is_empty() {
            return Err("text is empty".into());
        }
        if text.chars().count() > self.limits.max_prompt_chars {
            return Err(format!(
                "text exceeds length cap of {} characters",
                self.limits.max_prompt_chars
            ));
        }
        if text.contains('\0') {
            return Err("text contains NUL".into());
        }
        if text
            .chars()
            .any(|c| c.is_control() && c != '\t' && c != '\n' && c != '\r')
        {
            return Err("text contains control characters".into());
        }
        Ok(())
    }

    fn check_payload(
        &self,
        label: &str,
        text: &str,
        max: usize,
        empty_ok: bool,
    ) -> Result<(), String> {
        if !empty_ok && text.trim().is_empty() {
            return Err(format!("{label} is empty"));
        }
        if text.chars().count() > max {
            return Err(format!("{label} exceeds length cap of {max} characters"));
        }
        if text.contains('\0') {
            return Err(format!("{label} contains NUL"));
        }
        if text
            .chars()
            .any(|c| c.is_control() && c != '\t' && c != '\n' && c != '\r')
        {
            return Err(format!("{label} contains control characters"));
        }
        Ok(())
    }

    fn ensure_task_capacity(&mut self) -> Result<(), String> {
        self.prune_terminal_tasks();
        if self.tasks.len() >= self.limits.max_tasks {
            return Err(format!("task limit {} reached", self.limits.max_tasks));
        }
        Ok(())
    }

    fn ensure_effect_capacity(&mut self) -> Result<(), String> {
        if self.effects.len() >= self.limits.max_effects {
            self.stats.effects_rejected += 1;
            return Err("effect queue full".into());
        }
        Ok(())
    }

    fn prune(&mut self, _now_ms: u64) {
        self.prune_terminal_tasks();
        self.prune_closed_sessions();
    }

    fn prune_terminal_tasks(&mut self) {
        while self.tasks.len() > self.limits.max_tasks {
            let victim = self
                .tasks
                .values()
                .filter(|t| t.status.is_terminal())
                .min_by_key(|t| t.accepted_at_ms)
                .map(|t| t.id);
            let Some(id) = victim else {
                break;
            };
            self.remove_task(id);
            self.stats.pruned_tasks += 1;
        }
        // Also drop terminal tasks when we are at the cap even if equal
        // (called before insert when len == max).
        if self.tasks.len() >= self.limits.max_tasks {
            if let Some(id) = self
                .tasks
                .values()
                .filter(|t| t.status.is_terminal())
                .min_by_key(|t| t.accepted_at_ms)
                .map(|t| t.id)
            {
                self.remove_task(id);
                self.stats.pruned_tasks += 1;
            }
        }
    }

    fn prune_closed_sessions(&mut self) {
        let mut closed: Vec<(u64, AgentSessionId)> = self
            .sessions
            .values()
            .filter(|s| !s.open)
            .map(|s| (s.closed_at_ms.unwrap_or(0), s.id))
            .collect();
        if closed.len() <= self.limits.max_closed_sessions {
            return;
        }
        closed.sort_by_key(|(at, _)| *at);
        let drop_n = closed.len() - self.limits.max_closed_sessions;
        let drop_ids: Vec<_> = closed.into_iter().take(drop_n).map(|(_, id)| id).collect();
        for id in drop_ids {
            if let Some(session) = self.sessions.remove(&id) {
                for task_id in session.tasks {
                    self.tasks.remove(&task_id);
                    self.stats.pruned_tasks += 1;
                }
                self.stats.pruned_sessions += 1;
            }
        }
    }

    fn remove_task(&mut self, id: CoordinationTaskId) {
        if let Some(task) = self.tasks.remove(&id) {
            if let Some(session) = self.sessions.get_mut(&task.session) {
                session.tasks.retain(|t| *t != id);
            }
        }
    }

    fn push_fact(&mut self, event: Event) {
        let seq = self.next_fact_seq;
        self.next_fact_seq = self.next_fact_seq.saturating_add(1);
        if self.facts.len() >= self.limits.max_facts {
            self.facts.pop_front();
            self.stats.facts_dropped += 1;
        }
        self.facts.push_back(Fact { seq, event });
    }

    fn push_effect(&mut self, body: EffectBody) {
        let seq = self.next_effect_seq;
        self.next_effect_seq = self.next_effect_seq.saturating_add(1);
        self.effects.push_back(Effect { seq, body });
    }

    fn drain_effects_for_session(&mut self, session_id: AgentSessionId) {
        let before = self.effects.len();
        self.effects.retain(|e| e.body.session() != session_id);
        let n = (before - self.effects.len()) as u64;
        self.stats.effects_drained = self.stats.effects_drained.saturating_add(n);
    }

    fn ack_exact(
        &mut self,
        seq: u64,
        want: impl Fn(&EffectBody) -> bool,
        want_name: &str,
    ) -> Result<Effect, String> {
        let pos = self
            .effects
            .iter()
            .position(|e| e.seq == seq)
            .ok_or_else(|| "unknown effect".to_string())?;
        if !want(&self.effects[pos].body) {
            return Err(format!(
                "effect {seq} is {}, not {want_name}",
                self.effects[pos].body.kind_name()
            ));
        }
        Ok(self.effects.remove(pos).expect("position is valid"))
    }

    fn remove_effect(&mut self, seq: u64) -> Option<Effect> {
        let pos = self.effects.iter().position(|e| e.seq == seq)?;
        self.effects.remove(pos)
    }

    fn facts_since(&self, cursor: u64) -> FactBatch {
        let oldest = self.facts.front().map(|f| f.seq);
        let missed = match oldest {
            Some(min) if cursor + 1 < min => min - cursor - 1,
            _ => 0,
        };
        let mut facts = Vec::new();
        let mut bytes = 256;
        for fact in self.facts.iter().filter(|fact| fact.seq > cursor) {
            let size = serde_json::to_vec(fact)
                .map(|line| line.len())
                .unwrap_or(crate::MAX_LINE_BYTES)
                + 1;
            if !facts.is_empty() && bytes + size >= crate::MAX_LINE_BYTES {
                break;
            }
            bytes += size;
            facts.push(fact.clone());
        }
        let next_cursor = facts.last().map(|f| f.seq).unwrap_or(cursor);
        FactBatch {
            facts,
            next_cursor,
            missed,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{WireRequest, decode_request_line};

    fn abs(path: &str) -> String {
        if cfg!(windows) {
            if let Some(rest) = path.strip_prefix('/') {
                format!(r"C:\{}", rest.replace('/', "\\"))
            } else {
                path.to_string()
            }
        } else {
            path.to_string()
        }
    }

    fn limits_one() -> Limits {
        Limits {
            max_sessions: 1,
            max_tasks: 2,
            ..Limits::default()
        }
    }

    fn req(id: u64, body: Request) -> WireRequest {
        WireRequest { id, body }
    }

    fn launch(reg: &Registry, now: u64) -> (AgentSessionId, CoordinationTaskId) {
        match reg
            .handle(
                req(
                    1,
                    Request::Launch {
                        kind: AgentKind::Codex,
                        cwd: abs("/work"),
                        name: Some("w".into()),
                        args: vec!["--bar".into()],
                    },
                ),
                now,
            )
            .body
        {
            Response::LaunchAccepted { session, task } => (session, task),
            other => panic!("expected LaunchAccepted, got {other:?}"),
        }
    }

    fn effect_seq(reg: &Registry, pred: impl Fn(&EffectBody) -> bool) -> u64 {
        reg.peek_effects()
            .into_iter()
            .find(|e| pred(&e.body))
            .map(|e| e.seq)
            .expect("expected matching effect")
    }

    fn bind(reg: &Registry, session: AgentSessionId, pane: u128, now: u64) {
        let seq = effect_seq(
            reg,
            |b| matches!(b, EffectBody::LaunchRequested { session: s, .. } if *s == session),
        );
        reg.apply(
            AdapterUpdate::BindPane {
                seq,
                session,
                pane: Uuid::from_u128(pane),
            },
            now,
        )
        .unwrap();
    }

    fn settle_launch(reg: &Registry, session: AgentSessionId, task: CoordinationTaskId, now: u64) {
        bind(reg, session, 1, now);
        reg.apply(AdapterUpdate::TaskRunning { task }, now).unwrap();
        reg.apply(
            AdapterUpdate::TaskResult {
                task,
                text: "up".into(),
            },
            now,
        )
        .unwrap();
    }

    fn ready(reg: &Registry) -> (AgentSessionId, CoordinationTaskId) {
        let (session, task) = launch(reg, 0);
        settle_launch(reg, session, task, 1);
        (session, task)
    }

    #[test]
    fn list_starts_empty() {
        let reg = Registry::new();
        match reg.handle(req(1, Request::List), 0).body {
            Response::Agents { agents, .. } => assert!(agents.is_empty()),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn launch_accepts_and_queues_a_payload_effect() {
        let reg = Registry::new();
        let (session, task) = launch(&reg, 10);
        match reg.handle(req(2, Request::List), 10).body {
            Response::Agents { agents, .. } => {
                assert_eq!(agents.len(), 1);
                assert_eq!(agents[0].session, session);
                assert_eq!(agents[0].writer, Writer::Coordinator);
                assert_eq!(agents[0].tasks[0].status, TaskStatus::Dispatching);
                assert_eq!(agents[0].tasks[0].task, task);
            }
            other => panic!("{other:?}"),
        }
        let effects = reg.peek_effects();
        assert_eq!(effects.len(), 1);
        match &effects[0].body {
            EffectBody::LaunchRequested {
                session: s,
                task: t,
                kind,
                cwd,
                name,
                args,
            } => {
                assert_eq!(*s, session);
                assert_eq!(*t, task);
                assert_eq!(*kind, AgentKind::Codex);
                assert_eq!(cwd, &abs("/work"));
                assert_eq!(name.as_deref(), Some("w"));
                assert_eq!(args, &["--bar"]);
            }
            other => panic!("{other:?}"),
        }
        let facts = reg.facts_since(0);
        assert!(
            facts
                .facts
                .iter()
                .any(|f| matches!(f.event, Event::SessionOpened { .. }))
        );
    }

    #[test]
    fn apply_does_not_steal_coordinator_facts() {
        let reg = Registry::new();
        let (session, task) = launch(&reg, 0);
        bind(&reg, session, 7, 1);
        // Adapter apply must not drain the fact log.
        let batch = reg.facts_since(0);
        assert!(
            batch
                .facts
                .iter()
                .any(|f| matches!(f.event, Event::SessionOpened { .. }))
        );
        assert!(
            batch
                .facts
                .iter()
                .any(|f| matches!(f.event, Event::TaskDispatching { .. }))
        );
        // Launch effect was acked by BindPane; peek no longer has it.
        assert!(
            !reg.peek_effects()
                .iter()
                .any(|e| matches!(e.body, EffectBody::LaunchRequested { .. }))
        );
        let _ = task;
    }

    #[test]
    fn prompt_effect_carries_text() {
        let reg = Registry::new();
        let (session, _) = ready(&reg);
        let task = match reg
            .handle(
                req(
                    3,
                    Request::Prompt {
                        session,
                        text: "implement the tests".into(),
                    },
                ),
                3,
            )
            .body
        {
            Response::PromptAccepted { task } => task,
            other => panic!("{other:?}"),
        };
        let prompt = reg
            .peek_effects()
            .into_iter()
            .find_map(|e| match e.body {
                EffectBody::PromptRequested { text, task: t, .. } if t == task => Some(text),
                _ => None,
            })
            .expect("prompt effect");
        assert_eq!(prompt, "implement the tests");
    }

    #[test]
    fn launch_rejects_relative_empty_and_nul_cwd() {
        let reg = Registry::new();
        for cwd in ["  ", "relative/path", "/tmp\0x"] {
            match reg
                .handle(
                    req(
                        1,
                        Request::Launch {
                            kind: AgentKind::Claude,
                            cwd: cwd.into(),
                            name: None,
                            args: vec![],
                        },
                    ),
                    0,
                )
                .body
            {
                Response::Error { message } => assert!(
                    message.contains("cwd")
                        || message.contains("NUL")
                        || message.contains("absolute"),
                    "{cwd:?} → {message}"
                ),
                other => panic!("{cwd:?} → {other:?}"),
            }
        }
        let long = format!("{}/{}", abs("/tmp"), "x".repeat(MAX_CWD_CHARS));
        match reg
            .handle(
                req(
                    2,
                    Request::Launch {
                        kind: AgentKind::Claude,
                        cwd: long,
                        name: None,
                        args: vec![],
                    },
                ),
                0,
            )
            .body
        {
            Response::Error { message } => assert!(message.contains("cwd")),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn launch_rejects_bad_names_and_args() {
        let reg = Registry::new();
        match reg
            .handle(
                req(
                    1,
                    Request::Launch {
                        kind: AgentKind::Gemini,
                        cwd: abs("/tmp"),
                        name: Some("1bad".into()),
                        args: vec![],
                    },
                ),
                0,
            )
            .body
        {
            Response::Error { message } => assert!(message.contains("name")),
            other => panic!("{other:?}"),
        }
        match reg
            .handle(
                req(
                    2,
                    Request::Launch {
                        kind: AgentKind::Gemini,
                        cwd: abs("/tmp"),
                        name: Some("ok\0".into()),
                        args: vec![],
                    },
                ),
                0,
            )
            .body
        {
            Response::Error { message } => assert!(message.contains("NUL")),
            other => panic!("{other:?}"),
        }
        let too_many: Vec<String> = (0..MAX_ARGS + 1).map(|i| i.to_string()).collect();
        match reg
            .handle(
                req(
                    3,
                    Request::Launch {
                        kind: AgentKind::Gemini,
                        cwd: abs("/tmp"),
                        name: None,
                        args: too_many,
                    },
                ),
                0,
            )
            .body
        {
            Response::Error { message } => assert!(message.contains("args")),
            other => panic!("{other:?}"),
        }
        match reg
            .handle(
                req(
                    4,
                    Request::Launch {
                        kind: AgentKind::Gemini,
                        cwd: abs("/tmp"),
                        name: None,
                        args: vec!["ok\0bad".into()],
                    },
                ),
                0,
            )
            .body
        {
            Response::Error { message } => assert!(message.contains("NUL")),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn session_cap_is_enforced() {
        let reg = Registry::with_limits(limits_one());
        launch(&reg, 0);
        match reg
            .handle(
                req(
                    2,
                    Request::Launch {
                        kind: AgentKind::Opencode,
                        cwd: abs("/tmp"),
                        name: None,
                        args: vec![],
                    },
                ),
                0,
            )
            .body
        {
            Response::Error { message } => assert!(message.contains("session limit")),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn prompt_requires_coordinator_owner_and_ready_session() {
        let reg = Registry::new();
        let (session, launch_task) = launch(&reg, 0);
        match reg
            .handle(
                req(
                    2,
                    Request::Prompt {
                        session,
                        text: "go".into(),
                    },
                ),
                1,
            )
            .body
        {
            Response::Error { message } => assert!(message.contains("in-flight")),
            other => panic!("{other:?}"),
        }
        settle_launch(&reg, session, launch_task, 2);
        match reg
            .handle(
                req(
                    3,
                    Request::Prompt {
                        session,
                        text: "go".into(),
                    },
                ),
                3,
            )
            .body
        {
            Response::PromptAccepted { task } => assert_ne!(task, launch_task),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn prompt_rejected_while_human_owns() {
        let reg = Registry::new();
        let (session, launch_task) = ready(&reg);
        let _ = launch_task;
        match reg
            .handle(req(2, Request::HumanTakeover { session }), 2)
            .body
        {
            Response::TakenOver { .. } => {}
            other => panic!("{other:?}"),
        }
        match reg
            .handle(
                req(
                    3,
                    Request::Prompt {
                        session,
                        text: "secret approve yes".into(),
                    },
                ),
                3,
            )
            .body
        {
            Response::Error { message } => assert!(message.contains("human owns")),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn prompt_rejected_while_awaiting_human() {
        let reg = Registry::new();
        let (session, _) = ready(&reg);
        let task = match reg
            .handle(
                req(
                    2,
                    Request::Prompt {
                        session,
                        text: "work".into(),
                    },
                ),
                2,
            )
            .body
        {
            Response::PromptAccepted { task } => task,
            other => panic!("{other:?}"),
        };
        let seq = effect_seq(
            &reg,
            |b| matches!(b, EffectBody::PromptRequested { task: t, .. } if *t == task),
        );
        reg.apply(AdapterUpdate::PromptDelivered { seq }, 3)
            .unwrap();
        reg.apply(AdapterUpdate::TaskRunning { task }, 3).unwrap();
        reg.apply(AdapterUpdate::TaskAwaitingHuman { task }, 4)
            .unwrap();
        match reg
            .handle(
                req(
                    5,
                    Request::Prompt {
                        session,
                        text: "yes, approved".into(),
                    },
                ),
                5,
            )
            .body
        {
            Response::Error { message } => {
                assert!(
                    message.contains("cannot answer native approvals"),
                    "{message}"
                );
            }
            other => panic!("coordinator must not approve: {other:?}"),
        }
    }

    #[test]
    fn empty_whitespace_control_and_oversize_prompt_are_errors() {
        let reg = Registry::new();
        let (session, _) = ready(&reg);
        for text in [
            String::new(),
            "   ".into(),
            "ok\0no".into(),
            "ok\u{07}bell".into(),
        ] {
            match reg
                .handle(req(2, Request::Prompt { session, text }), 2)
                .body
            {
                Response::Error { .. } => {}
                other => panic!("{other:?}"),
            }
        }
        let long: String = std::iter::repeat_n('x', MAX_PROMPT_CHARS + 1).collect();
        match reg
            .handle(
                req(
                    3,
                    Request::Prompt {
                        session,
                        text: long,
                    },
                ),
                3,
            )
            .body
        {
            Response::Error { message } => assert!(message.contains("exceeds")),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn wait_is_an_immediate_snapshot_with_terminal_flag() {
        let reg = Registry::new();
        let (_session, task) = launch(&reg, 100);
        match reg.handle(req(2, Request::Wait { task }), 999).body {
            Response::Wait {
                status, terminal, ..
            } => {
                assert_eq!(status, TaskStatus::Dispatching);
                assert!(!terminal);
            }
            other => panic!("{other:?}"),
        }
        settle_launch(&reg, _session, task, 200);
        match reg.handle(req(4, Request::Wait { task }), 300).body {
            Response::Wait {
                status, terminal, ..
            } => {
                assert_eq!(status, TaskStatus::Settled);
                assert!(terminal);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn summaries_and_diagnostic_pages_fit_the_wire_cap_at_capacity() {
        let reg = Registry::new();
        for i in 0..MAX_SESSIONS {
            let session = match reg
                .handle(
                    req(
                        i as u64,
                        Request::Launch {
                            kind: AgentKind::Codex,
                            cwd: format!("/{}", "界".repeat(1023)),
                            name: None,
                            args: vec![],
                        },
                    ),
                    i as u64,
                )
                .body
            {
                Response::LaunchAccepted { session, .. } => session,
                other => panic!("{other:?}"),
            };
            assert!(matches!(
                reg.handle(req(0, Request::Inspect { session }), 0).body,
                Response::Inspect { .. }
            ));
        }
        for body in [Request::List, Request::Effects] {
            let reply = reg.handle(req(0, body), 0);
            assert!(
                crate::encode_response_line(&reply).unwrap().len() < crate::MAX_LINE_BYTES,
                "summary/diagnostic page must be transportable at capacity"
            );
        }
    }

    #[test]
    fn result_retires_pending_interrupt_before_a_new_task() {
        let reg = Registry::new();
        let (session, first) = ready(&reg);
        let first = match reg
            .handle(
                req(
                    1,
                    Request::Prompt {
                        session,
                        text: "first".into(),
                    },
                ),
                1,
            )
            .body
        {
            Response::PromptAccepted { task } => task,
            other => panic!("{other:?}; launch was {first:?}"),
        };
        reg.handle(req(2, Request::Interrupt { session }), 2);
        let stale = effect_seq(&reg, |b| matches!(b, EffectBody::InterruptRequested { .. }));
        reg.apply(
            AdapterUpdate::TaskResult {
                task: first,
                text: "done".into(),
            },
            3,
        )
        .unwrap();
        let second = match reg
            .handle(
                req(
                    4,
                    Request::Prompt {
                        session,
                        text: "second".into(),
                    },
                ),
                4,
            )
            .body
        {
            Response::PromptAccepted { task } => task,
            other => panic!("{other:?}"),
        };
        assert!(
            reg.apply(AdapterUpdate::InterruptDelivered { seq: stale }, 5)
                .is_err()
        );
        assert!(matches!(
            reg.handle(req(6, Request::Wait { task: second }), 6).body,
            Response::Wait {
                status: TaskStatus::Dispatching,
                ..
            }
        ));
        assert_eq!(reg.peek_effects().len(), 1, "only the new prompt survives");
    }

    #[test]
    fn idle_interrupt_does_not_survive_acceptance_of_a_new_prompt() {
        let reg = Registry::new();
        let (session, _) = ready(&reg);
        reg.handle(req(1, Request::Interrupt { session }), 1);
        let interrupt = reg.peek_effects()[0].clone();
        reg.handle(
            req(
                2,
                Request::Prompt {
                    session,
                    text: "new work".into(),
                },
            ),
            2,
        );
        let mut delivered = false;
        let claim = reg.deliver(&interrupt, |_| {
            delivered = true;
        });
        assert!(claim.is_err());
        assert!(
            !delivered,
            "an idle interrupt must not hit the next assignment"
        );
    }

    #[test]
    fn failed_launch_closes_the_unbound_session_in_the_registry() {
        let reg = Registry::with_limits(limits_one());
        let (session, task) = launch(&reg, 0);
        let seq = reg.peek_effects()[0].seq;
        reg.apply(AdapterUpdate::DeliveryFailed { seq }, 1).unwrap();
        match reg.handle(req(2, Request::Inspect { session }), 2).body {
            Response::Inspect { session } => assert!(!session.open),
            other => panic!("{other:?}"),
        }
        assert!(matches!(
            reg.handle(req(3, Request::Wait { task }), 3).body,
            Response::Wait {
                status: TaskStatus::FailedDelivery,
                ..
            }
        ));
        launch(&reg, 4);
    }

    #[test]
    fn interrupt_delivery_retires_target_as_unknown_not_success() {
        let reg = Registry::new();
        let (session, task) = launch(&reg, 0);
        bind(&reg, session, 1, 1);
        reg.apply(AdapterUpdate::TaskRunning { task }, 1).unwrap();
        match reg.handle(req(2, Request::Interrupt { session }), 2).body {
            Response::InterruptAccepted { .. } => {}
            other => panic!("{other:?}"),
        }
        match reg.handle(req(3, Request::Inspect { session }), 3).body {
            Response::Inspect { session: snap } => {
                assert_eq!(snap.tasks[0].status, TaskStatus::Interrupting);
                assert!(snap.tasks[0].result.is_none());
            }
            other => panic!("{other:?}"),
        }
        assert!(
            reg.peek_effects()
                .iter()
                .any(|e| matches!(e.body, EffectBody::InterruptRequested { .. }))
        );
        match reg
            .handle(
                req(
                    4,
                    Request::Prompt {
                        session,
                        text: "next".into(),
                    },
                ),
                4,
            )
            .body
        {
            Response::Error { message } => assert!(message.contains("in-flight")),
            other => panic!("interrupt must keep the in-flight gate: {other:?}"),
        }
        let interrupt_seq = effect_seq(
            &reg,
            |b| matches!(b, EffectBody::InterruptRequested { session: s, .. } if *s == session),
        );
        reg.apply(AdapterUpdate::InterruptDelivered { seq: interrupt_seq }, 5)
            .unwrap();
        assert!(
            !reg.peek_effects()
                .iter()
                .any(|e| matches!(e.body, EffectBody::InterruptRequested { .. }))
        );
        match reg.handle(req(6, Request::Inspect { session }), 6).body {
            Response::Inspect { session: snap } => {
                assert_eq!(snap.tasks[0].status, TaskStatus::Unknown);
                assert!(snap.tasks[0].result.is_none());
            }
            other => panic!("{other:?}"),
        }
        match reg.handle(req(7, Request::Wait { task }), 7).body {
            Response::Wait {
                status, terminal, ..
            } => {
                assert_eq!(status, TaskStatus::Unknown);
                assert!(terminal);
            }
            other => panic!("{other:?}"),
        }
        assert!(
            reg.facts_since(0)
                .facts
                .iter()
                .any(|f| matches!(f.event, Event::TaskUnknown { task: t } if t == task))
        );
        match reg
            .handle(
                req(
                    8,
                    Request::Prompt {
                        session,
                        text: "after interrupt".into(),
                    },
                ),
                8,
            )
            .body
        {
            Response::PromptAccepted { .. } => {}
            other => panic!("settled interrupt must clear the in-flight gate: {other:?}"),
        }
    }

    #[test]
    fn interrupt_delivery_failure_marks_unknown_not_settled() {
        let reg = Registry::new();
        let (session, task) = launch(&reg, 0);
        bind(&reg, session, 1, 1);
        reg.apply(AdapterUpdate::TaskRunning { task }, 1).unwrap();
        assert!(matches!(
            reg.handle(req(2, Request::Interrupt { session }), 2).body,
            Response::InterruptAccepted { .. }
        ));
        let seq = effect_seq(
            &reg,
            |b| matches!(b, EffectBody::InterruptRequested { session: s, .. } if *s == session),
        );
        reg.apply(AdapterUpdate::DeliveryFailed { seq }, 3).unwrap();
        match reg.handle(req(4, Request::Wait { task }), 4).body {
            Response::Wait {
                status, terminal, ..
            } => {
                assert_eq!(status, TaskStatus::Unknown);
                assert!(terminal);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn stale_interrupt_delivered_does_not_settle_a_later_task() {
        let reg = Registry::new();
        let (session, first) = launch(&reg, 0);
        bind(&reg, session, 1, 1);
        reg.apply(AdapterUpdate::TaskRunning { task: first }, 1)
            .unwrap();
        assert!(matches!(
            reg.handle(req(2, Request::Interrupt { session }), 2).body,
            Response::InterruptAccepted { .. }
        ));
        let first_seq = effect_seq(
            &reg,
            |b| matches!(b, EffectBody::InterruptRequested { session: s, .. } if *s == session),
        );
        reg.apply(AdapterUpdate::InterruptDelivered { seq: first_seq }, 3)
            .unwrap();
        let second = match reg
            .handle(
                req(
                    4,
                    Request::Prompt {
                        session,
                        text: "next".into(),
                    },
                ),
                4,
            )
            .body
        {
            Response::PromptAccepted { task } => task,
            other => panic!("{other:?}"),
        };
        assert!(matches!(
            reg.handle(req(5, Request::Interrupt { session }), 5).body,
            Response::InterruptAccepted { .. }
        ));
        let second_seq = effect_seq(
            &reg,
            |b| matches!(b, EffectBody::InterruptRequested { session: s, .. } if *s == session),
        );
        assert_ne!(first_seq, second_seq);
        match reg.apply(AdapterUpdate::InterruptDelivered { seq: first_seq }, 6) {
            Err(message) => assert!(message.contains("unknown effect"), "{message}"),
            Ok(()) => panic!("stale seq must not settle the later task"),
        }
        match reg.handle(req(7, Request::Wait { task: second }), 7).body {
            Response::Wait { status, .. } => assert_eq!(status, TaskStatus::Interrupting),
            other => panic!("{other:?}"),
        }
        match reg.apply(AdapterUpdate::InterruptDelivered { seq: second_seq }, 8) {
            Ok(()) => {}
            Err(err) => panic!("{err}"),
        }
        match reg.handle(req(9, Request::Wait { task: second }), 9).body {
            Response::Wait {
                status, terminal, ..
            } => {
                assert_eq!(status, TaskStatus::Unknown);
                assert!(terminal);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn session_close_drains_all_effects_for_that_session() {
        let reg = Registry::new();
        let (session, _) = ready(&reg);
        assert!(matches!(
            reg.handle(
                req(
                    2,
                    Request::Prompt {
                        session,
                        text: "secret prompt".into(),
                    },
                ),
                2,
            )
            .body,
            Response::PromptAccepted { .. }
        ));
        assert!(matches!(
            reg.handle(req(3, Request::Focus { session }), 3).body,
            Response::FocusAccepted { .. }
        ));
        assert!(matches!(
            reg.handle(req(4, Request::Close { session }), 4).body,
            Response::CloseAccepted { .. }
        ));
        let (other, _) = launch(&reg, 5);
        assert!(
            reg.peek_effects()
                .iter()
                .any(|e| matches!(e.body, EffectBody::PromptRequested { .. }))
        );
        let before = reg.peek_effects().len();
        assert!(before >= 3, "prompt, focus, close, plus other launch");
        reg.apply(AdapterUpdate::SessionClosed { session }, 6)
            .unwrap();
        let leftover = reg.peek_effects();
        assert!(
            leftover.iter().all(|e| e.body.session() != session),
            "closed session must not leave prompt/interrupt/focus/close queued: {leftover:?}"
        );
        assert!(
            leftover.iter().any(
                |e| matches!(e.body, EffectBody::LaunchRequested { session: s, .. } if s == other)
            ),
            "other session's launch must survive"
        );
        assert!(reg.stats().effects_drained >= 3);
    }

    #[test]
    fn interrupt_rejected_while_human_owns() {
        let reg = Registry::new();
        let (session, _) = ready(&reg);
        reg.handle(req(2, Request::HumanTakeover { session }), 2);
        match reg.handle(req(3, Request::Interrupt { session }), 3).body {
            Response::Error { message } => assert!(message.contains("human owns")),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn delivery_failure_marks_failed_delivery_not_settled() {
        let reg = Registry::new();
        let (_session, task) = launch(&reg, 0);
        let seq = reg.peek_effects()[0].seq;
        reg.apply(AdapterUpdate::DeliveryFailed { seq }, 1).unwrap();
        match reg.handle(req(2, Request::Wait { task }), 2).body {
            Response::Wait {
                status, terminal, ..
            } => {
                assert_eq!(status, TaskStatus::FailedDelivery);
                assert!(terminal);
            }
            other => panic!("{other:?}"),
        }
        assert!(reg.peek_effects().is_empty());
    }

    #[test]
    fn takeover_is_explicit_and_release_is_host_only() {
        let reg = Registry::new();
        let (session, _) = ready(&reg);
        assert!(matches!(
            reg.handle(req(2, Request::HumanTakeover { session }), 2)
                .body,
            Response::TakenOver { .. }
        ));
        assert!(matches!(
            reg.handle(req(3, Request::HumanTakeover { session }), 3)
                .body,
            Response::Error { .. }
        ));
        match decode_request_line(&format!(
            r#"{{"id":4,"op":"release_to_coordinator","session":"{}"}}"#,
            session.as_uuid()
        )) {
            Err(_) => {}
            Ok(ok) => panic!("coordinator must not decode release: {ok:?}"),
        }
        reg.apply(AdapterUpdate::ReleaseToCoordinator { session }, 4)
            .unwrap();
        match reg
            .handle(
                req(
                    5,
                    Request::Prompt {
                        session,
                        text: "again".into(),
                    },
                ),
                5,
            )
            .body
        {
            Response::PromptAccepted { .. } => {}
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn focus_requires_bound_pane_and_queues_effect() {
        let reg = Registry::new();
        let (session, _) = launch(&reg, 0);
        match reg.handle(req(2, Request::Focus { session }), 2).body {
            Response::Error { message } => assert!(message.contains("not bound")),
            other => panic!("{other:?}"),
        }
        bind(&reg, session, 9, 3);
        match reg.handle(req(4, Request::Focus { session }), 4).body {
            Response::FocusAccepted { .. } => {}
            other => panic!("{other:?}"),
        }
        assert!(
            reg.peek_effects()
                .iter()
                .any(|e| matches!(e.body, EffectBody::FocusRequested { .. }))
        );
    }

    #[test]
    fn bind_pane_is_one_time_and_idempotent_for_same_pane() {
        let reg = Registry::new();
        let (session, _) = launch(&reg, 0);
        let pane = Uuid::from_u128(3);
        let seq = effect_seq(
            &reg,
            |b| matches!(b, EffectBody::LaunchRequested { session: s, .. } if *s == session),
        );
        reg.apply(AdapterUpdate::BindPane { seq, session, pane }, 1)
            .unwrap();
        reg.apply(AdapterUpdate::BindPane { seq, session, pane }, 2)
            .unwrap();
        match reg.apply(
            AdapterUpdate::BindPane {
                seq,
                session,
                pane: Uuid::from_u128(4),
            },
            3,
        ) {
            Err(message) => assert!(message.contains("already bound")),
            Ok(()) => panic!("rebind must fail"),
        }
    }

    #[test]
    fn close_request_queues_effect_without_closing() {
        let reg = Registry::new();
        let (session, task) = launch(&reg, 0);
        bind(&reg, session, 1, 1);
        reg.apply(AdapterUpdate::TaskRunning { task }, 1).unwrap();
        match reg.handle(req(2, Request::Close { session }), 2).body {
            Response::CloseAccepted { .. } => {}
            other => panic!("{other:?}"),
        }
        match reg.handle(req(3, Request::Inspect { session }), 3).body {
            Response::Inspect { session: snap } => {
                assert!(snap.open);
                assert_eq!(snap.tasks[0].status, TaskStatus::Running);
            }
            other => panic!("{other:?}"),
        }
        assert!(
            reg.peek_effects()
                .iter()
                .any(|e| matches!(e.body, EffectBody::CloseRequested { .. }))
        );
        reg.apply(AdapterUpdate::SessionClosed { session }, 4)
            .unwrap();
        match reg.handle(req(5, Request::Inspect { session }), 5).body {
            Response::Inspect { session: snap } => {
                assert!(!snap.open);
                assert_eq!(snap.tasks[0].status, TaskStatus::Unknown);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn focus_rejects_closed_and_unknown_sessions() {
        let reg = Registry::new();
        match reg
            .handle(
                req(
                    1,
                    Request::Focus {
                        session: AgentSessionId::new(),
                    },
                ),
                0,
            )
            .body
        {
            Response::Error { message } => assert!(message.contains("unknown session")),
            other => panic!("{other:?}"),
        }
        let (session, _) = ready(&reg);
        assert!(matches!(
            reg.handle(req(2, Request::Focus { session }), 2).body,
            Response::FocusAccepted { .. }
        ));
        reg.apply(AdapterUpdate::SessionClosed { session }, 3)
            .unwrap();
        match reg.handle(req(4, Request::Focus { session }), 4).body {
            Response::Error { message } => assert!(message.contains("closed")),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn pane_closure_marks_in_flight_tasks_unknown() {
        let reg = Registry::new();
        let (session, task) = launch(&reg, 0);
        bind(&reg, session, 1, 1);
        reg.apply(AdapterUpdate::TaskRunning { task }, 1).unwrap();
        reg.apply(AdapterUpdate::SessionClosed { session }, 2)
            .unwrap();
        let facts = reg.facts_since(0);
        assert!(
            facts
                .facts
                .iter()
                .any(|f| matches!(f.event, Event::TaskUnknown { .. }))
        );
        match reg.handle(req(3, Request::Inspect { session }), 3).body {
            Response::Inspect { session: snap } => {
                assert!(!snap.open);
                assert_eq!(snap.tasks[0].status, TaskStatus::Unknown);
            }
            other => panic!("{other:?}"),
        }
        match reg
            .handle(
                req(
                    4,
                    Request::Prompt {
                        session,
                        text: "nope".into(),
                    },
                ),
                4,
            )
            .body
        {
            Response::Error { message } => assert!(message.contains("closed")),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn late_and_duplicate_results_are_correlated_not_resurrected() {
        let reg = Registry::new();
        let (session, task) = ready(&reg);
        let _ = session;
        let dup = {
            reg.apply(
                AdapterUpdate::TaskResult {
                    task,
                    text: "again".into(),
                },
                2,
            )
            .unwrap();
            reg.facts_since(0)
        };
        assert!(
            dup.facts
                .iter()
                .any(|f| matches!(f.event, Event::DuplicateResult { .. }))
        );
        let (session2, open_task) = launch(&reg, 3);
        bind(&reg, session2, 2, 4);
        reg.apply(AdapterUpdate::TaskRunning { task: open_task }, 4)
            .unwrap();
        let cursor = reg.facts_since(0).next_cursor;
        reg.apply(AdapterUpdate::SessionClosed { session: session2 }, 5)
            .unwrap();
        reg.apply(
            AdapterUpdate::TaskResult {
                task: open_task,
                text: "late".into(),
            },
            6,
        )
        .unwrap();
        let late = reg.facts_since(cursor);
        assert!(
            late.facts
                .iter()
                .any(|f| matches!(f.event, Event::LateResult { .. }))
        );
        match reg
            .handle(req(7, Request::Inspect { session: session2 }), 7)
            .body
        {
            Response::Inspect { session: snap } => {
                assert_eq!(snap.tasks[0].status, TaskStatus::Unknown);
                assert!(snap.tasks[0].result.is_none());
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn bind_pane_is_visible_on_inspect() {
        let reg = Registry::new();
        let (session, _) = launch(&reg, 0);
        let pane = Uuid::from_u128(42);
        let seq = effect_seq(
            &reg,
            |b| matches!(b, EffectBody::LaunchRequested { session: s, .. } if *s == session),
        );
        reg.apply(AdapterUpdate::BindPane { seq, session, pane }, 1)
            .unwrap();
        match reg.handle(req(2, Request::Inspect { session }), 2).body {
            Response::Inspect { session: snap } => assert_eq!(snap.pane, Some(pane)),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn unknown_task_wait_is_an_error() {
        let reg = Registry::new();
        match reg
            .handle(
                req(
                    1,
                    Request::Wait {
                        task: CoordinationTaskId::new(),
                    },
                ),
                0,
            )
            .body
        {
            Response::Error { message } => assert!(message.contains("unknown task")),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn awaiting_human_can_return_to_running_after_human_handles() {
        let reg = Registry::new();
        let (session, _) = ready(&reg);
        let task = match reg
            .handle(
                req(
                    2,
                    Request::Prompt {
                        session,
                        text: "work".into(),
                    },
                ),
                2,
            )
            .body
        {
            Response::PromptAccepted { task } => task,
            other => panic!("{other:?}"),
        };
        let seq = effect_seq(
            &reg,
            |b| matches!(b, EffectBody::PromptRequested { task: t, .. } if *t == task),
        );
        reg.apply(AdapterUpdate::PromptDelivered { seq }, 3)
            .unwrap();
        reg.apply(AdapterUpdate::TaskRunning { task }, 3).unwrap();
        reg.apply(AdapterUpdate::TaskAwaitingHuman { task }, 4)
            .unwrap();
        reg.handle(req(5, Request::HumanTakeover { session }), 5);
        reg.apply(AdapterUpdate::TaskRunning { task }, 6).unwrap();
        match reg.handle(req(7, Request::Wait { task }), 7).body {
            Response::Wait {
                status, terminal, ..
            } => {
                assert_eq!(status, TaskStatus::Running);
                assert!(!terminal);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn fact_cursors_are_independent_of_effects_and_count_overflow() {
        let mut limits = Limits::default();
        limits.max_facts = 2;
        let reg = Registry::with_limits(limits);
        launch(&reg, 0);
        // launch pushes 3 facts; cap 2 → at least one drop
        assert!(reg.stats().facts_dropped >= 1);
        let a = reg.facts_since(0);
        let b = reg.facts_since(0);
        assert_eq!(a.facts, b.facts);
        assert!(a.missed >= 1 || a.facts.len() == 2);
        let effects = reg.peek_effects();
        assert_eq!(effects.len(), 1);
        let again = reg.peek_effects();
        assert_eq!(again, effects);
    }

    #[test]
    fn closed_sessions_and_terminal_tasks_are_pruned() {
        let limits = Limits {
            max_sessions: 2,
            max_closed_sessions: 1,
            max_tasks: 2,
            ..Limits::default()
        };
        let reg = Registry::with_limits(limits);
        let (s1, t1) = ready(&reg);
        let _ = t1;
        let (s2, t2) = launch(&reg, 10);
        bind(&reg, s2, 2, 11);
        reg.apply(AdapterUpdate::TaskRunning { task: t2 }, 12)
            .unwrap();
        reg.apply(
            AdapterUpdate::TaskResult {
                task: t2,
                text: "up".into(),
            },
            12,
        )
        .unwrap();
        reg.apply(AdapterUpdate::SessionClosed { session: s1 }, 13)
            .unwrap();
        reg.apply(AdapterUpdate::SessionClosed { session: s2 }, 14)
            .unwrap();
        assert!(reg.stats().pruned_sessions >= 1);
        match reg
            .handle(req(9, Request::Inspect { session: s1 }), 15)
            .body
        {
            Response::Error { message } => assert!(message.contains("unknown session")),
            other => panic!("expected eviction of s1, got {other:?}"),
        }
    }

    #[test]
    fn effect_queue_full_rejects_without_dropping_payloads() {
        let limits = Limits {
            max_effects: 1,
            ..Limits::default()
        };
        let reg = Registry::with_limits(limits);
        launch(&reg, 0);
        match reg
            .handle(
                req(
                    2,
                    Request::Launch {
                        kind: AgentKind::Claude,
                        cwd: abs("/tmp"),
                        name: None,
                        args: vec![],
                    },
                ),
                1,
            )
            .body
        {
            Response::Error { message } => assert!(message.contains("effect queue full")),
            other => panic!("{other:?}"),
        }
        assert_eq!(reg.stats().effects_rejected, 1);
        assert_eq!(reg.peek_effects().len(), 1);
    }

    #[test]
    fn registry_is_usable_from_two_threads() {
        let reg = Registry::new();
        let a = reg.clone();
        let b = reg.clone();
        let t1 = std::thread::spawn(move || launch(&a, 0));
        let t2 = std::thread::spawn(move || {
            b.handle(req(9, Request::List), 0);
        });
        let (session, _) = t1.join().unwrap();
        t2.join().unwrap();
        match reg.handle(req(10, Request::List), 1).body {
            Response::Agents { agents, .. } => {
                assert!(agents.iter().any(|s| s.session == session));
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn worker_reports_running_awaiting_and_result_on_wait() {
        let reg = Registry::new();
        let (session, task) = launch(&reg, 0);
        assert!(matches!(
            reg.handle(req(2, Request::ReportRunning { task }), 1).body,
            Response::ReportedRunning { .. }
        ));
        match reg.handle(req(3, Request::Wait { task }), 1).body {
            Response::Wait {
                status,
                terminal,
                result,
                detail,
                ..
            } => {
                assert_eq!(status, TaskStatus::Running);
                assert!(!terminal);
                assert!(result.is_none());
                assert!(detail.is_none());
            }
            other => panic!("{other:?}"),
        }
        assert!(matches!(
            reg.handle(
                req(
                    4,
                    Request::ReportAwaitingHuman {
                        task,
                        detail: Some("native dialog".into()),
                    },
                ),
                2,
            )
            .body,
            Response::ReportedAwaitingHuman { .. }
        ));
        match reg.handle(req(5, Request::Wait { task }), 2).body {
            Response::Wait { status, detail, .. } => {
                assert_eq!(status, TaskStatus::AwaitingHuman);
                assert_eq!(detail.as_deref(), Some("native dialog"));
            }
            other => panic!("{other:?}"),
        }
        match reg
            .handle(
                req(
                    6,
                    Request::Prompt {
                        session,
                        text: "yes".into(),
                    },
                ),
                3,
            )
            .body
        {
            Response::Error { message } => {
                assert!(
                    message.contains("cannot answer native approvals"),
                    "{message}"
                );
            }
            other => panic!("{other:?}"),
        }
        assert!(matches!(
            reg.handle(
                req(
                    7,
                    Request::ReportResult {
                        task,
                        text: "files written".into(),
                    },
                ),
                4,
            )
            .body,
            Response::ReportedResult { .. }
        ));
        match reg.handle(req(8, Request::Wait { task }), 4).body {
            Response::Wait {
                status,
                terminal,
                result,
                detail,
                ..
            } => {
                assert_eq!(status, TaskStatus::Settled);
                assert!(terminal);
                assert_eq!(result.as_deref(), Some("files written"));
                assert_eq!(detail.as_deref(), Some("native dialog"));
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn worker_report_late_and_duplicate_do_not_resurrect() {
        let reg = Registry::new();
        let (session, task) = launch(&reg, 0);
        reg.handle(
            req(
                2,
                Request::ReportResult {
                    task,
                    text: "first".into(),
                },
            ),
            1,
        );
        let cursor = reg.facts_since(0).next_cursor;
        assert!(matches!(
            reg.handle(
                req(
                    3,
                    Request::ReportResult {
                        task,
                        text: "second".into(),
                    },
                ),
                2,
            )
            .body,
            Response::ReportedResult { .. }
        ));
        assert!(
            reg.facts_since(cursor)
                .facts
                .iter()
                .any(|f| matches!(f.event, Event::DuplicateResult { .. }))
        );
        match reg.handle(req(4, Request::Wait { task }), 2).body {
            Response::Wait { result, status, .. } => {
                assert_eq!(status, TaskStatus::Settled);
                assert_eq!(result.as_deref(), Some("first"));
            }
            other => panic!("{other:?}"),
        }
        match reg.handle(req(5, Request::ReportRunning { task }), 3).body {
            Response::Error { message } => assert!(message.contains("not runnable"), "{message}"),
            other => panic!("{other:?}"),
        }
        let (open, open_task) = launch(&reg, 4);
        bind(&reg, open, 2, 4);
        reg.apply(AdapterUpdate::SessionClosed { session: open }, 5)
            .unwrap();
        assert_eq!(task_wait(&reg, open_task), TaskStatus::Unknown);
        let cursor = reg.facts_since(0).next_cursor;
        assert!(matches!(
            reg.handle(
                req(
                    6,
                    Request::ReportResult {
                        task: open_task,
                        text: "late".into(),
                    },
                ),
                6,
            )
            .body,
            Response::ReportedResult { .. }
        ));
        assert!(
            reg.facts_since(cursor)
                .facts
                .iter()
                .any(|f| matches!(f.event, Event::LateResult { .. }))
        );
        assert_eq!(task_wait(&reg, open_task), TaskStatus::Unknown);
        match reg
            .handle(req(7, Request::Wait { task: open_task }), 6)
            .body
        {
            Response::Wait { result, .. } => assert!(result.is_none()),
            other => panic!("{other:?}"),
        }
        let _ = session;
    }

    fn task_wait(reg: &Registry, task: CoordinationTaskId) -> TaskStatus {
        match reg.handle(req(0, Request::Wait { task }), 0).body {
            Response::Wait { status, .. } => status,
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn worker_report_rejects_unknown_and_oversize_control_payloads() {
        let reg = Registry::new();
        let (session, task) = launch(&reg, 0);
        match reg
            .handle(
                req(
                    1,
                    Request::ReportRunning {
                        task: CoordinationTaskId::new(),
                    },
                ),
                0,
            )
            .body
        {
            Response::Error { message } => assert!(message.contains("unknown task")),
            other => panic!("{other:?}"),
        }
        match reg
            .handle(
                req(
                    2,
                    Request::ReportSessionClosed {
                        session: AgentSessionId::new(),
                    },
                ),
                0,
            )
            .body
        {
            Response::Error { message } => assert!(message.contains("unknown session")),
            other => panic!("{other:?}"),
        }
        match reg
            .handle(
                req(
                    3,
                    Request::ReportAwaitingHuman {
                        task,
                        detail: Some("ok\0no".into()),
                    },
                ),
                0,
            )
            .body
        {
            Response::Error { message } => assert!(message.contains("NUL"), "{message}"),
            other => panic!("{other:?}"),
        }
        let long: String = std::iter::repeat_n('x', MAX_DETAIL_CHARS + 1).collect();
        match reg
            .handle(
                req(
                    4,
                    Request::ReportAwaitingHuman {
                        task,
                        detail: Some(long),
                    },
                ),
                0,
            )
            .body
        {
            Response::Error { message } => assert!(message.contains("detail"), "{message}"),
            other => panic!("{other:?}"),
        }
        match reg
            .handle(
                req(
                    5,
                    Request::ReportResult {
                        task,
                        text: "ok\u{07}".into(),
                    },
                ),
                0,
            )
            .body
        {
            Response::Error { message } => {
                assert!(message.contains("control"), "{message}");
            }
            other => panic!("{other:?}"),
        }
        assert!(matches!(
            reg.handle(req(6, Request::ReportSessionClosed { session }), 1)
                .body,
            Response::ReportedSessionClosed { .. }
        ));
        match reg.handle(req(7, Request::Inspect { session }), 1).body {
            Response::Inspect { session: snap } => assert!(!snap.open),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn correlation_id_is_echoed_on_error_and_ok() {
        let reg = Registry::new();
        let err = reg.handle(req(42, Request::List), 0);
        assert_eq!(err.id, 42);
        let (session, _) = launch(&reg, 0);
        let ok = reg.handle(req(99, Request::Inspect { session }), 0);
        assert_eq!(ok.id, 99);
        assert!(matches!(ok.body, Response::Inspect { .. }));
    }

    #[test]
    fn stale_prompt_delivered_does_not_ack_a_later_prompt() {
        let reg = Registry::new();
        let (session, _) = ready(&reg);
        let first = match reg
            .handle(
                req(
                    2,
                    Request::Prompt {
                        session,
                        text: "first".into(),
                    },
                ),
                2,
            )
            .body
        {
            Response::PromptAccepted { task } => task,
            other => panic!("{other:?}"),
        };
        let first_seq = effect_seq(
            &reg,
            |b| matches!(b, EffectBody::PromptRequested { task, .. } if *task == first),
        );
        reg.apply(AdapterUpdate::PromptDelivered { seq: first_seq }, 3)
            .unwrap();
        reg.apply(AdapterUpdate::TaskRunning { task: first }, 3)
            .unwrap();
        reg.apply(
            AdapterUpdate::TaskResult {
                task: first,
                text: "done".into(),
            },
            4,
        )
        .unwrap();
        let second = match reg
            .handle(
                req(
                    5,
                    Request::Prompt {
                        session,
                        text: "second".into(),
                    },
                ),
                5,
            )
            .body
        {
            Response::PromptAccepted { task } => task,
            other => panic!("{other:?}"),
        };
        let second_seq = effect_seq(
            &reg,
            |b| matches!(b, EffectBody::PromptRequested { task, .. } if *task == second),
        );
        assert_ne!(first_seq, second_seq);
        match reg.apply(AdapterUpdate::PromptDelivered { seq: first_seq }, 6) {
            Err(message) => assert!(message.contains("unknown effect"), "{message}"),
            Ok(()) => panic!("stale seq must not ack the later prompt"),
        }
        assert!(reg.peek_effects().iter().any(|e| e.seq == second_seq
            && matches!(e.body, EffectBody::PromptRequested { task, .. } if task == second)));
        match reg.handle(req(7, Request::Wait { task: second }), 7).body {
            Response::Wait { status, .. } => assert_eq!(status, TaskStatus::Dispatching),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn stale_focus_delivered_does_not_ack_a_later_focus() {
        let reg = Registry::new();
        let (session, _) = ready(&reg);
        assert!(matches!(
            reg.handle(req(2, Request::Focus { session }), 2).body,
            Response::FocusAccepted { .. }
        ));
        let first_seq = effect_seq(
            &reg,
            |b| matches!(b, EffectBody::FocusRequested { session: s } if *s == session),
        );
        reg.apply(AdapterUpdate::FocusDelivered { seq: first_seq }, 3)
            .unwrap();
        assert!(matches!(
            reg.handle(req(4, Request::Focus { session }), 4).body,
            Response::FocusAccepted { .. }
        ));
        let second_seq = effect_seq(
            &reg,
            |b| matches!(b, EffectBody::FocusRequested { session: s } if *s == session),
        );
        assert_ne!(first_seq, second_seq);
        match reg.apply(AdapterUpdate::FocusDelivered { seq: first_seq }, 5) {
            Err(message) => assert!(message.contains("unknown effect"), "{message}"),
            Ok(()) => panic!("stale seq must not ack the later focus"),
        }
        assert!(reg.peek_effects().iter().any(|e| e.seq == second_seq
            && matches!(e.body, EffectBody::FocusRequested { session: s } if s == session)));
    }

    #[test]
    fn ack_kind_mismatch_leaves_the_effect_queued() {
        let reg = Registry::new();
        let (session, _) = launch(&reg, 0);
        let seq = effect_seq(
            &reg,
            |b| matches!(b, EffectBody::LaunchRequested { session: s, .. } if *s == session),
        );
        match reg.apply(AdapterUpdate::PromptDelivered { seq }, 1) {
            Err(message) => {
                assert!(message.contains("not prompt_requested"), "{message}");
                assert!(message.contains("launch_requested"), "{message}");
            }
            Ok(()) => panic!("kind mismatch must fail"),
        }
        assert!(
            reg.peek_effects()
                .iter()
                .any(|e| e.seq == seq && matches!(e.body, EffectBody::LaunchRequested { .. }))
        );
    }

    #[test]
    fn unknown_effect_seq_is_an_error() {
        let reg = Registry::new();
        launch(&reg, 0);
        match reg.apply(AdapterUpdate::FocusDelivered { seq: 99 }, 1) {
            Err(message) => assert!(message.contains("unknown effect"), "{message}"),
            Ok(()) => panic!("missing seq must fail"),
        }
        assert_eq!(reg.peek_effects().len(), 1);
    }

    #[test]
    fn bind_pane_seq_must_name_the_session_launch() {
        let reg = Registry::new();
        let (a, _) = launch(&reg, 0);
        let (b, _) = launch(&reg, 1);
        let seq_a = effect_seq(
            &reg,
            |body| matches!(body, EffectBody::LaunchRequested { session, .. } if *session == a),
        );
        match reg.apply(
            AdapterUpdate::BindPane {
                seq: seq_a,
                session: b,
                pane: Uuid::from_u128(1),
            },
            2,
        ) {
            Err(message) => assert!(
                message.contains("different session") || message.contains("not launch_requested"),
                "{message}"
            ),
            Ok(()) => panic!("foreign launch seq must not bind"),
        }
        match reg.handle(req(3, Request::Inspect { session: b }), 3).body {
            Response::Inspect { session: snap } => assert!(snap.pane.is_none()),
            other => panic!("{other:?}"),
        }
        assert!(
            reg.peek_effects()
                .iter()
                .any(|e| e.seq == seq_a && matches!(e.body, EffectBody::LaunchRequested { .. }))
        );
    }

    #[test]
    fn close_delivered_acks_only_the_named_seq() {
        let reg = Registry::new();
        let (session, _) = ready(&reg);
        assert!(matches!(
            reg.handle(req(2, Request::Close { session }), 2).body,
            Response::CloseAccepted { .. }
        ));
        let first_seq = effect_seq(
            &reg,
            |b| matches!(b, EffectBody::CloseRequested { session: s } if *s == session),
        );
        reg.apply(AdapterUpdate::CloseDelivered { seq: first_seq }, 3)
            .unwrap();
        assert!(matches!(
            reg.handle(req(4, Request::Close { session }), 4).body,
            Response::CloseAccepted { .. }
        ));
        let second_seq = effect_seq(
            &reg,
            |b| matches!(b, EffectBody::CloseRequested { session: s } if *s == session),
        );
        match reg.apply(AdapterUpdate::CloseDelivered { seq: first_seq }, 5) {
            Err(message) => assert!(message.contains("unknown effect"), "{message}"),
            Ok(()) => panic!("stale close seq must not ack a later close"),
        }
        assert!(reg.peek_effects().iter().any(|e| e.seq == second_seq));
    }
}
