//! Transport framing for coordination responses.
//!
//! The registry returns complete domain values. This module splits a
//! [`WireResponse`] into `MAX_LINE_BYTES` JSON-lines frames. Page request
//! ops do not exist: the client reads successive frames on one connection.

use crate::line::MAX_LINE_BYTES;
use crate::protocol::{
    CoordinationTaskId, Effect, Fact, Response, SessionSnapshot, TaskStatus, WireResponse,
    encode_response_line,
};

pub fn frame_response(resp: WireResponse) -> Result<Vec<String>, String> {
    let encoded = encode_response_line(&resp)?;
    if encoded.len() <= MAX_LINE_BYTES {
        return Ok(vec![encoded]);
    }
    match resp.body {
        Response::Agents { agents, .. } => frame_agents(resp.id, agents),
        Response::Inspect { session } => frame_inspect(resp.id, session),
        Response::Effects { effects, .. } => frame_effects(resp.id, effects),
        Response::Wait {
            task,
            status,
            terminal,
            result,
            detail,
            ..
        } => frame_wait(resp.id, task, status, terminal, result, detail),
        Response::Facts {
            facts,
            next_cursor,
            missed,
            ..
        } => frame_facts(resp.id, facts, next_cursor, missed),
        _ => Err("response exceeds line length cap".into()),
    }
}

fn frame_agents(id: u64, agents: Vec<SessionSnapshot>) -> Result<Vec<String>, String> {
    frame_agent_pages(id, &agents)
}

fn frame_agent_pages(id: u64, agents: &[SessionSnapshot]) -> Result<Vec<String>, String> {
    if agents.is_empty() {
        return Ok(vec![encode_response_line(&WireResponse {
            id,
            body: Response::Agents {
                agents: vec![],
                next_offset: None,
            },
        })?]);
    }
    let mut frames = Vec::new();
    let mut start = 0;
    while start < agents.len() {
        let mut best: Option<(usize, String)> = None;
        for end in start + 1..=agents.len() {
            let next_offset = (end < agents.len()).then_some(end);
            let resp = WireResponse {
                id,
                body: Response::Agents {
                    agents: agents[start..end].to_vec(),
                    next_offset,
                },
            };
            let line = encode_response_line(&resp)?;
            if line.len() <= MAX_LINE_BYTES {
                best = Some((end, line));
            } else {
                break;
            }
        }
        let Some((end, line)) = best else {
            return Err("session snapshot exceeds line length cap".into());
        };
        frames.push(line);
        start = end;
    }
    Ok(frames)
}

fn frame_inspect(id: u64, session: SessionSnapshot) -> Result<Vec<String>, String> {
    let tasks = session.tasks.clone();
    if tasks.is_empty() {
        let mut snap = session;
        snap.next_task_offset = None;
        return Ok(vec![encode_response_line(&WireResponse {
            id,
            body: Response::Inspect { session: snap },
        })?]);
    }
    let mut frames = Vec::new();
    let mut start = 0;
    while start < tasks.len() {
        let mut best: Option<(usize, String)> = None;
        for end in start + 1..=tasks.len() {
            let next_task_offset = (end < tasks.len()).then_some(end);
            let snap = SessionSnapshot {
                tasks: tasks[start..end].to_vec(),
                next_task_offset,
                ..session.clone()
            };
            let line = encode_response_line(&WireResponse {
                id,
                body: Response::Inspect { session: snap },
            })?;
            if line.len() <= MAX_LINE_BYTES {
                best = Some((end, line));
            } else {
                break;
            }
        }
        let Some((end, line)) = best else {
            return Err("task snapshot exceeds line length cap".into());
        };
        frames.push(line);
        start = end;
    }
    Ok(frames)
}

fn frame_effects(id: u64, effects: Vec<Effect>) -> Result<Vec<String>, String> {
    if effects.is_empty() {
        return Ok(vec![encode_response_line(&WireResponse {
            id,
            body: Response::Effects {
                effects: vec![],
                next_cursor: None,
            },
        })?]);
    }
    let mut frames = Vec::new();
    let mut start = 0;
    while start < effects.len() {
        let mut best: Option<(usize, String)> = None;
        for end in start + 1..=effects.len() {
            let next_cursor = (end < effects.len()).then_some(effects[end - 1].seq);
            let line = encode_response_line(&WireResponse {
                id,
                body: Response::Effects {
                    effects: effects[start..end].to_vec(),
                    next_cursor,
                },
            })?;
            if line.len() <= MAX_LINE_BYTES {
                best = Some((end, line));
            } else {
                break;
            }
        }
        let Some((end, line)) = best else {
            return Err("effect exceeds line length cap".into());
        };
        frames.push(line);
        start = end;
    }
    Ok(frames)
}

fn frame_facts(
    id: u64,
    facts: Vec<Fact>,
    next_cursor: u64,
    missed: u64,
) -> Result<Vec<String>, String> {
    if facts.is_empty() {
        return Ok(vec![encode_response_line(&WireResponse {
            id,
            body: Response::Facts {
                facts: vec![],
                next_cursor,
                missed,
                more: false,
            },
        })?]);
    }
    let mut frames = Vec::new();
    let mut start = 0;
    while start < facts.len() {
        let mut best: Option<(usize, String)> = None;
        for end in start + 1..=facts.len() {
            let last = end == facts.len();
            let line = encode_response_line(&WireResponse {
                id,
                body: Response::Facts {
                    facts: facts[start..end].to_vec(),
                    next_cursor: if last {
                        next_cursor
                    } else {
                        facts[end - 1].seq
                    },
                    missed: if start == 0 { missed } else { 0 },
                    more: !last,
                },
            })?;
            if line.len() <= MAX_LINE_BYTES {
                best = Some((end, line));
            } else {
                break;
            }
        }
        let Some((end, line)) = best else {
            return Err("fact exceeds line length cap".into());
        };
        frames.push(line);
        start = end;
    }
    Ok(frames)
}

fn frame_wait(
    id: u64,
    task: CoordinationTaskId,
    status: TaskStatus,
    terminal: bool,
    result: Option<String>,
    detail: Option<String>,
) -> Result<Vec<String>, String> {
    let Some(result) = result else {
        return Ok(vec![encode_response_line(&WireResponse {
            id,
            body: Response::Wait {
                task,
                status,
                terminal,
                result: None,
                detail,
                next_result_offset: None,
            },
        })?]);
    };

    let mut boundaries: Vec<usize> = result.char_indices().map(|(i, _)| i).collect();
    boundaries.push(result.len());
    let total_chars = boundaries.len().saturating_sub(1);
    let mut frames = Vec::new();
    let mut offset = 0;
    while offset < total_chars {
        let (end, next) = wait_page_end(
            id,
            task,
            status,
            terminal,
            &result,
            detail.as_deref(),
            &boundaries,
            offset,
            total_chars,
        )?;
        let slice = result
            .get(boundaries[offset]..boundaries[end])
            .ok_or_else(|| "result offset split a code point".to_string())?;
        frames.push(encode_response_line(&WireResponse {
            id,
            body: Response::Wait {
                task,
                status,
                terminal,
                result: Some(slice.to_string()),
                detail: detail.clone(),
                next_result_offset: next,
            },
        })?);
        offset = end;
    }
    if frames.is_empty() {
        frames.push(encode_response_line(&WireResponse {
            id,
            body: Response::Wait {
                task,
                status,
                terminal,
                result: Some(String::new()),
                detail,
                next_result_offset: None,
            },
        })?);
    }
    Ok(frames)
}

fn wait_page_end(
    id: u64,
    task: CoordinationTaskId,
    status: TaskStatus,
    terminal: bool,
    result: &str,
    detail: Option<&str>,
    boundaries: &[usize],
    start: usize,
    total_chars: usize,
) -> Result<(usize, Option<usize>), String> {
    if wait_fits(
        id,
        task,
        status,
        terminal,
        result,
        detail,
        boundaries,
        start,
        total_chars,
        None,
    )? {
        return Ok((total_chars, None));
    }
    if start + 1 > total_chars {
        return Err("wait result page did not advance".into());
    }
    if !wait_fits(
        id,
        task,
        status,
        terminal,
        result,
        detail,
        boundaries,
        start,
        start + 1,
        Some(start + 1),
    )? {
        return Err("wait response exceeds line length cap".into());
    }
    let mut low = start + 1;
    let mut high = total_chars;
    let mut best = start + 1;
    while low <= high {
        let mid = low + (high - low) / 2;
        let next = (mid < total_chars).then_some(mid);
        if wait_fits(
            id, task, status, terminal, result, detail, boundaries, start, mid, next,
        )? {
            best = mid;
            low = mid.saturating_add(1);
        } else {
            high = mid.saturating_sub(1);
        }
    }
    let next = (best < total_chars).then_some(best);
    Ok((best, next))
}

fn wait_fits(
    id: u64,
    task: CoordinationTaskId,
    status: TaskStatus,
    terminal: bool,
    result: &str,
    detail: Option<&str>,
    boundaries: &[usize],
    start: usize,
    end: usize,
    next_result_offset: Option<usize>,
) -> Result<bool, String> {
    let slice = result
        .get(boundaries[start]..boundaries[end])
        .ok_or_else(|| "result offset split a code point".to_string())?;
    let encoded = encode_response_line(&WireResponse {
        id,
        body: Response::Wait {
            task,
            status,
            terminal,
            result: Some(slice.to_string()),
            detail: detail.map(str::to_string),
            next_result_offset,
        },
    })?;
    Ok(encoded.len() <= MAX_LINE_BYTES)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{AgentKind, AgentSessionId, TaskSnapshot, Writer};
    use uuid::Uuid;

    fn sid(n: u128) -> AgentSessionId {
        AgentSessionId::from_uuid(Uuid::from_u128(n))
    }

    fn tid(n: u128) -> CoordinationTaskId {
        CoordinationTaskId::from_uuid(Uuid::from_u128(n))
    }

    #[test]
    fn small_responses_are_one_frame() {
        let resp = WireResponse {
            id: 1,
            body: Response::Agents {
                agents: vec![],
                next_offset: None,
            },
        };
        let frames = frame_response(resp).unwrap();
        assert_eq!(frames.len(), 1);
        assert!(frames[0].len() < MAX_LINE_BYTES);
    }

    #[test]
    fn wait_pages_are_lossless_for_large_unicode() {
        let payload = "界".repeat(30 * 1024);
        let resp = WireResponse {
            id: 7,
            body: Response::Wait {
                task: tid(1),
                status: TaskStatus::Settled,
                terminal: true,
                result: Some(payload.clone()),
                detail: None,
                next_result_offset: None,
            },
        };
        let frames = frame_response(resp).unwrap();
        assert!(frames.len() > 1);
        let mut merged = String::new();
        for (i, line) in frames.iter().enumerate() {
            assert!(line.len() <= MAX_LINE_BYTES, "{}", line.len());
            let parsed = crate::decode_response_line(line).unwrap();
            match parsed.body {
                Response::Wait {
                    result,
                    next_result_offset,
                    ..
                } => {
                    merged.push_str(result.as_deref().unwrap_or_default());
                    if i + 1 == frames.len() {
                        assert!(next_result_offset.is_none());
                    } else {
                        assert!(next_result_offset.is_some());
                    }
                }
                other => panic!("{other:?}"),
            }
        }
        assert_eq!(merged, payload);
    }

    #[test]
    fn inspect_pages_omit_nothing_and_fit() {
        let tasks: Vec<TaskSnapshot> = (0..400)
            .map(|i| TaskSnapshot {
                task: tid(i),
                session: sid(1),
                status: TaskStatus::Settled,
                accepted_at_ms: i as u64,
                result: None,
                detail: Some("x".repeat(200)),
            })
            .collect();
        let resp = WireResponse {
            id: 3,
            body: Response::Inspect {
                session: SessionSnapshot {
                    session: sid(1),
                    kind: AgentKind::Codex,
                    cwd: "/".into(),
                    name: None,
                    writer: Writer::Coordinator,
                    open: true,
                    pane: None,
                    tasks,
                    next_task_offset: None,
                },
            },
        };
        let frames = frame_response(resp).unwrap();
        assert!(frames.len() > 1);
        let mut n = 0;
        for line in &frames {
            assert!(line.len() <= MAX_LINE_BYTES);
            match crate::decode_response_line(line).unwrap().body {
                Response::Inspect { session } => n += session.tasks.len(),
                other => panic!("{other:?}"),
            }
        }
        assert_eq!(n, 400);
    }
}
