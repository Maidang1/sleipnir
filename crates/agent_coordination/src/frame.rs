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

/// The position of one page within a `pack_pages` run, so callers can compute
/// per-page fields (e.g. a `next_cursor` only the last page zeroes, a `missed`
/// only the first page carries).
struct PageCtx {
    /// Index one past the last item on this page.
    end: usize,
    /// True when this is the first page.
    is_first: bool,
    /// True when this is the last page (no more items follow).
    is_last: bool,
}

fn pack_pages<T: Clone, F>(
    items: &[T],
    make_response: F,
    item_err: &str,
) -> Result<Vec<String>, String>
where
    F: Fn(&[T], &PageCtx) -> WireResponse,
{
    if items.is_empty() {
        let ctx = PageCtx {
            end: 0,
            is_first: true,
            is_last: true,
        };
        return Ok(vec![encode_response_line(&make_response(&[], &ctx))?]);
    }
    let mut frames = Vec::new();
    let mut start = 0;
    while start < items.len() {
        let mut best: Option<(usize, String)> = None;
        for end in start + 1..=items.len() {
            let ctx = PageCtx {
                end,
                is_first: start == 0,
                is_last: end == items.len(),
            };
            let resp = make_response(&items[start..end], &ctx);
            let line = encode_response_line(&resp)?;
            if line.len() <= MAX_LINE_BYTES {
                best = Some((end, line));
            } else {
                break;
            }
        }
        let Some((end, line)) = best else {
            return Err(item_err.into());
        };
        frames.push(line);
        start = end;
    }
    Ok(frames)
}

fn frame_agents(id: u64, agents: Vec<SessionSnapshot>) -> Result<Vec<String>, String> {
    pack_pages(
        &agents,
        |slice, ctx| WireResponse {
            id,
            body: Response::Agents {
                agents: slice.to_vec(),
                next_offset: (!ctx.is_last).then_some(ctx.end),
            },
        },
        "session snapshot exceeds line length cap",
    )
}

fn frame_inspect(id: u64, session: SessionSnapshot) -> Result<Vec<String>, String> {
    let tasks = session.tasks.clone();
    pack_pages(
        &tasks,
        |slice, ctx| {
            let snap = SessionSnapshot {
                tasks: slice.to_vec(),
                next_task_offset: (!ctx.is_last).then_some(ctx.end),
                ..session.clone()
            };
            WireResponse {
                id,
                body: Response::Inspect { session: snap },
            }
        },
        "task snapshot exceeds line length cap",
    )
}

fn frame_effects(id: u64, effects: Vec<Effect>) -> Result<Vec<String>, String> {
    pack_pages(
        &effects,
        |slice, ctx| WireResponse {
            id,
            body: Response::Effects {
                effects: slice.to_vec(),
                next_cursor: (!ctx.is_last).then(|| slice.last().unwrap().seq),
            },
        },
        "effect exceeds line length cap",
    )
}

fn frame_facts(
    id: u64,
    facts: Vec<Fact>,
    next_cursor: u64,
    missed: u64,
) -> Result<Vec<String>, String> {
    pack_pages(
        &facts,
        |slice, ctx| WireResponse {
            id,
            body: Response::Facts {
                facts: slice.to_vec(),
                // The client resumes from the last seq of a non-final page and
                // from the registry's cursor on the final page.
                next_cursor: if ctx.is_last {
                    next_cursor
                } else {
                    slice.last().unwrap().seq
                },
                // `missed` describes the gap at the start of the batch, so only
                // the first page carries it.
                missed: if ctx.is_first { missed } else { 0 },
                more: !ctx.is_last,
            },
        },
        "fact exceeds line length cap",
    )
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
