//! Focused-pane facts: cwd, foreground name, descendant process tree, listen ports.
//!
//! The snapshot is a pure function over an injected process/port reader so tests
//! do not need a live PTY or GPUI.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

/// One process in the pane's descendant tree, preorder, with depth.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProcRow {
    pub pid: u32,
    pub name: Option<String>,
    pub depth: usize,
}

/// A TCP listen address owned by a pid in the tree.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListenPort {
    pub pid: u32,
    pub addr: String,
}

/// Read-only facts for the focused pane. Empty fields stay `None` / empty.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct PaneFacts {
    pub cwd: Option<PathBuf>,
    pub foreground: Option<String>,
    pub tree: Vec<ProcRow>,
    pub ports: Vec<ListenPort>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawProc {
    pub pid: u32,
    pub parent: Option<u32>,
    pub name: Option<String>,
}

pub trait ProcReader {
    fn processes(&self) -> Vec<RawProc>;
    fn listeners(&self) -> Vec<(u32, String)>;
}

/// Build the snapshot. `root_pid` is the pane's shell (not the foreground job).
pub fn build_pane_facts(
    cwd: Option<PathBuf>,
    foreground: Option<String>,
    root_pid: Option<u32>,
    reader: &impl ProcReader,
) -> PaneFacts {
    let cwd = cwd.filter(|p| !p.as_os_str().is_empty());
    let foreground = foreground.and_then(|s| {
        let t = s.trim();
        if t.is_empty() {
            None
        } else {
            Some(t.to_string())
        }
    });

    let Some(root) = root_pid.filter(|p| *p > 0) else {
        return PaneFacts {
            cwd,
            foreground,
            tree: Vec::new(),
            ports: Vec::new(),
        };
    };

    let procs = reader.processes();
    let by_pid: HashMap<u32, &RawProc> = procs.iter().map(|p| (p.pid, p)).collect();
    let mut kids: HashMap<u32, Vec<u32>> = HashMap::new();
    for p in &procs {
        if let Some(parent) = p.parent {
            kids.entry(parent).or_default().push(p.pid);
        }
    }
    for list in kids.values_mut() {
        list.sort_unstable();
        list.dedup();
    }

    let mut tree = Vec::new();
    let mut seen = HashSet::new();
    walk(root, 0, &by_pid, &kids, &mut seen, &mut tree);

    let in_tree: HashSet<u32> = tree.iter().map(|r| r.pid).collect();
    let mut ports: Vec<ListenPort> = reader
        .listeners()
        .into_iter()
        .filter(|(pid, _)| in_tree.contains(pid))
        .map(|(pid, addr)| ListenPort { pid, addr })
        .collect();
    ports.sort_by(|a, b| a.addr.cmp(&b.addr).then(a.pid.cmp(&b.pid)));
    ports.dedup();

    PaneFacts {
        cwd,
        foreground,
        tree,
        ports,
    }
}

fn walk(
    pid: u32,
    depth: usize,
    by_pid: &HashMap<u32, &RawProc>,
    kids: &HashMap<u32, Vec<u32>>,
    seen: &mut HashSet<u32>,
    out: &mut Vec<ProcRow>,
) {
    if !seen.insert(pid) {
        return;
    }
    let name = by_pid.get(&pid).and_then(|p| p.name.clone()).and_then(|n| {
        let t = n.trim();
        if t.is_empty() {
            None
        } else {
            Some(t.to_string())
        }
    });
    out.push(ProcRow { pid, name, depth });
    if let Some(children) = kids.get(&pid) {
        for child in children {
            walk(*child, depth + 1, by_pid, kids, seen, out);
        }
    }
}

/// Parse `lsof -nP -iTCP -sTCP:LISTEN` (or an injected table) into `(pid, addr)`.
#[cfg_attr(windows, allow(dead_code))]
pub fn parse_lsof_listen(text: &str) -> Vec<(u32, String)> {
    let mut out = Vec::new();
    for line in text.lines() {
        if !line.contains("LISTEN") {
            continue;
        }
        let cols: Vec<&str> = line.split_whitespace().collect();
        if cols.len() < 3 {
            continue;
        }
        // COMMAND PID USER FD TYPE ... NAME
        let Ok(pid) = cols[1].parse::<u32>() else {
            continue;
        };
        let addr = cols
            .iter()
            .rev()
            .find(|c| !c.starts_with('(') && c.contains(':'))
            .map(|s| (*s).to_string());
        let Some(addr) = addr else {
            continue;
        };
        out.push((pid, addr));
    }
    out
}

/// Live process table + listen table (macOS: sysinfo + lsof).
pub struct LiveProcReader;

impl ProcReader for LiveProcReader {
    fn processes(&self) -> Vec<RawProc> {
        use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};
        let mut system = System::new();
        system.refresh_processes_specifics(
            ProcessesToUpdate::All,
            true,
            ProcessRefreshKind::nothing()
                .with_cmd(UpdateKind::OnlyIfNotSet)
                .without_tasks(),
        );
        system
            .processes()
            .iter()
            .map(|(pid, proc)| {
                let name = proc.name().to_string_lossy();
                RawProc {
                    pid: pid.as_u32(),
                    parent: proc.parent().map(|p| p.as_u32()),
                    name: if name.is_empty() {
                        None
                    } else {
                        Some(name.into_owned())
                    },
                }
            })
            .collect()
    }

    fn listeners(&self) -> Vec<(u32, String)> {
        #[cfg(windows)]
        {
            Vec::new()
        }
        #[cfg(not(windows))]
        {
            let output = std::process::Command::new("lsof")
                .args(["-nP", "-iTCP", "-sTCP:LISTEN"])
                .output();
            match output {
                Ok(out) if out.status.success() => {
                    parse_lsof_listen(&String::from_utf8_lossy(&out.stdout))
                }
                _ => Vec::new(),
            }
        }
    }
}

/// Collect facts for a live pane. `root_pid` is the shell child.
pub fn collect_live_facts(
    cwd: Option<PathBuf>,
    foreground: Option<String>,
    root_pid: Option<u32>,
) -> PaneFacts {
    build_pane_facts(cwd, foreground, root_pid, &LiveProcReader)
}

/// True when `path` is a localhost listen the user can copy as `localhost:PORT`.
pub fn localhost_copy(addr: &str) -> Option<String> {
    let addr = addr.trim();
    let host = addr.rsplit_once(':')?;
    let (h, port) = host;
    if port.is_empty() || !port.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let h = h.trim_matches(['[', ']']);
    if matches!(h, "127.0.0.1" | "::1" | "localhost" | "*") {
        Some(format!("localhost:{port}"))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    struct Fake {
        procs: Vec<RawProc>,
        listens: Vec<(u32, String)>,
    }

    impl ProcReader for Fake {
        fn processes(&self) -> Vec<RawProc> {
            self.procs.clone()
        }
        fn listeners(&self) -> Vec<(u32, String)> {
            self.listens.clone()
        }
    }

    fn proc(pid: u32, parent: Option<u32>, name: &str) -> RawProc {
        RawProc {
            pid,
            parent,
            name: Some(name.into()),
        }
    }

    #[test]
    fn idle_shell_has_tree_and_no_ports() {
        let facts = build_pane_facts(
            Some(PathBuf::from("/tmp/proj")),
            None,
            Some(10),
            &Fake {
                procs: vec![proc(10, Some(1), "zsh")],
                listens: vec![],
            },
        );
        assert_eq!(facts.cwd.as_deref(), Some(Path::new("/tmp/proj")));
        assert!(facts.foreground.is_none());
        assert_eq!(facts.tree.len(), 1);
        assert_eq!(facts.tree[0].pid, 10);
        assert_eq!(facts.tree[0].name.as_deref(), Some("zsh"));
        assert_eq!(facts.tree[0].depth, 0);
        assert!(facts.ports.is_empty());
    }

    #[test]
    fn unknown_cwd_does_not_block_other_fields() {
        let facts = build_pane_facts(
            None,
            Some("node".into()),
            Some(10),
            &Fake {
                procs: vec![proc(10, Some(1), "zsh"), proc(20, Some(10), "node")],
                listens: vec![(20, "127.0.0.1:3000".into())],
            },
        );
        assert!(facts.cwd.is_none());
        assert_eq!(facts.foreground.as_deref(), Some("node"));
        assert_eq!(facts.tree.iter().map(|r| r.pid).collect::<Vec<_>>(), vec![10, 20]);
        assert_eq!(facts.ports.len(), 1);
        assert_eq!(facts.ports[0].addr, "127.0.0.1:3000");
    }

    #[test]
    fn empty_cwd_and_blank_name_are_omitted() {
        let facts = build_pane_facts(
            Some(PathBuf::new()),
            Some("   ".into()),
            Some(10),
            &Fake {
                procs: vec![RawProc {
                    pid: 10,
                    parent: None,
                    name: Some(String::new()),
                }],
                listens: vec![],
            },
        );
        assert!(facts.cwd.is_none());
        assert!(facts.foreground.is_none());
        assert_eq!(facts.tree[0].name, None);
    }

    #[test]
    fn tree_is_preorder_parent_then_children() {
        let facts = build_pane_facts(
            None,
            None,
            Some(1),
            &Fake {
                procs: vec![
                    proc(1, None, "zsh"),
                    proc(3, Some(1), "sleep"),
                    proc(2, Some(1), "node"),
                    proc(4, Some(2), "node"),
                ],
                listens: vec![],
            },
        );
        let ids: Vec<(u32, usize)> = facts.tree.iter().map(|r| (r.pid, r.depth)).collect();
        assert_eq!(ids, vec![(1, 0), (2, 1), (4, 2), (3, 1)]);
    }

    #[test]
    fn listen_on_listed_pid_appears_foreign_pid_does_not() {
        let facts = build_pane_facts(
            None,
            None,
            Some(10),
            &Fake {
                procs: vec![proc(10, Some(1), "zsh"), proc(20, Some(10), "node")],
                listens: vec![
                    (20, "127.0.0.1:3000".into()),
                    (99, "0.0.0.0:22".into()),
                ],
            },
        );
        assert_eq!(facts.ports.len(), 1);
        assert_eq!(facts.ports[0].pid, 20);
        assert_eq!(facts.ports[0].addr, "127.0.0.1:3000");
    }

    #[test]
    fn no_root_pid_yields_no_tree_or_ports() {
        let facts = build_pane_facts(
            Some(PathBuf::from("/tmp")),
            Some("zsh".into()),
            None,
            &Fake {
                procs: vec![proc(10, Some(1), "zsh")],
                listens: vec![(10, "127.0.0.1:9".into())],
            },
        );
        assert_eq!(facts.cwd.as_deref(), Some(Path::new("/tmp")));
        assert_eq!(facts.foreground.as_deref(), Some("zsh"));
        assert!(facts.tree.is_empty());
        assert!(facts.ports.is_empty());
    }

    #[test]
    fn parse_lsof_listen_extracts_pid_and_addr() {
        let text = "\
COMMAND   PID USER   FD   TYPE DEVICE SIZE/OFF NODE NAME
node    4242  me    23u  IPv4  0t0  TCP 127.0.0.1:3000 (LISTEN)
sshd    99    me    4u   IPv4  0t0  TCP *:22 (LISTEN)
not-a-header
";
        let rows = parse_lsof_listen(text);
        assert!(rows.contains(&(4242, "127.0.0.1:3000".into())));
        assert!(rows.contains(&(99, "*:22".into())));
    }

    #[test]
    fn localhost_copy_only_for_loopback() {
        assert_eq!(
            localhost_copy("127.0.0.1:3000").as_deref(),
            Some("localhost:3000")
        );
        assert_eq!(localhost_copy("[::1]:8080").as_deref(), Some("localhost:8080"));
        assert_eq!(localhost_copy("10.0.0.4:80"), None);
    }
}
