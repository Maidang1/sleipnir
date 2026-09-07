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

/// Parse /proc/net/tcp or /proc/net/tcp6 content into LISTEN `(port, inode)`
/// pairs. Both files share the column layout:
/// `sl local_address rem_address st tx_queue:rx_queue tr:tm->when retrnsmt uid
/// timeout inode ...` where `local_address` is `<hex-ip>:<hex-port>` and
/// `st == "0A"` means LISTEN. The inode is kept so the socket can be
/// attributed to a pid through `/proc/<pid>/fd` symlinks; without a pid the
/// port would be dropped by the pane-tree filter in `build_pane_facts`.
/// Malformed lines and non-LISTEN rows are skipped; anything unreadable
/// yields an empty set rather than a panic.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub fn parse_proc_net_listen(text: &str) -> HashSet<(u16, u64)> {
    let mut out = HashSet::new();
    for line in text.lines() {
        let cols: Vec<&str> = line.split_whitespace().collect();
        if cols.len() < 10 || cols[3] != "0A" {
            continue;
        }
        let Some((_, port_hex)) = cols[1].rsplit_once(':') else {
            continue;
        };
        let Ok(port) = u16::from_str_radix(port_hex, 16) else {
            continue;
        };
        let Ok(inode) = cols[9].parse::<u64>() else {
            continue;
        };
        out.insert((port, inode));
    }
    out
}

/// `lsof -nP -iTCP -sTCP:LISTEN` → `(pid, addr)` rows; empty on any failure.
#[cfg(not(windows))]
fn lsof_listeners() -> Vec<(u32, String)> {
    let output = std::process::Command::new("lsof")
        .args(["-nP", "-iTCP", "-sTCP:LISTEN"])
        .output();
    match output {
        Ok(out) if out.status.success() => parse_lsof_listen(&String::from_utf8_lossy(&out.stdout)),
        _ => Vec::new(),
    }
}

/// Linux listen table: prefer `lsof` (richer address strings); fall back to
/// /proc when lsof is missing or fails — many distributions do not ship it.
#[cfg(target_os = "linux")]
fn linux_listeners() -> Vec<(u32, String)> {
    let lsof = lsof_listeners();
    if !lsof.is_empty() {
        return lsof;
    }
    proc_net_listeners()
}

/// Listen table from /proc/net/tcp{,6} + /proc/<pid>/fd socket symlinks.
/// Addresses are reported as `*:<port>` (matching lsof's wildcard form, which
/// `localhost_copy` accepts) since /proc stores IPs as raw hex.
#[cfg(target_os = "linux")]
fn proc_net_listeners() -> Vec<(u32, String)> {
    let mut rows = HashSet::new();
    for path in ["/proc/net/tcp", "/proc/net/tcp6"] {
        if let Ok(text) = std::fs::read_to_string(path) {
            rows.extend(parse_proc_net_listen(&text));
        }
    }
    if rows.is_empty() {
        return Vec::new();
    }
    let port_by_inode: HashMap<u64, u16> = rows.into_iter().map(|(p, i)| (i, p)).collect();

    let mut out = Vec::new();
    let Ok(proc_dir) = std::fs::read_dir("/proc") else {
        return Vec::new();
    };
    for entry in proc_dir.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        let Ok(pid) = name.parse::<u32>() else {
            continue;
        };
        let Ok(fds) = std::fs::read_dir(entry.path().join("fd")) else {
            continue;
        };
        let mut ports = HashSet::new();
        for fd in fds.flatten() {
            let Ok(target) = std::fs::read_link(fd.path()) else {
                continue;
            };
            let target = target.to_string_lossy();
            let Some(inode) = target
                .strip_prefix("socket:[")
                .and_then(|s| s.strip_suffix(']'))
                .and_then(|s| s.parse::<u64>().ok())
            else {
                continue;
            };
            if let Some(port) = port_by_inode.get(&inode) {
                ports.insert(*port);
            }
        }
        out.extend(ports.into_iter().map(|port| (pid, format!("*:{port}"))));
    }
    out
}

/// Live process table + listen table (macOS: sysinfo + lsof; Linux: sysinfo +
/// lsof with a /proc fallback; Windows: sysinfo only).
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
        #[cfg(target_os = "linux")]
        {
            linux_listeners()
        }
        #[cfg(not(any(windows, target_os = "linux")))]
        {
            lsof_listeners()
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
        assert_eq!(
            facts.tree.iter().map(|r| r.pid).collect::<Vec<_>>(),
            vec![10, 20]
        );
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
                listens: vec![(20, "127.0.0.1:3000".into()), (99, "0.0.0.0:22".into())],
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
    fn parse_proc_net_listen_extracts_tcp_listen_ports() {
        // Realistic /proc/net/tcp: header, one loopback LISTEN (0x0BB8 =
        // 3000), one wildcard LISTEN (0x0016 = 22), one ESTABLISHED (st 01),
        // and a truncated garbage line.
        let text = "\
  sl  local_address rem_address   st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode
   0: 0100007F:0BB8 00000000:0000 0A 00000000:00000000 00:00000000 00000000  1000        0 123456 1 0000000000000000 100 0 0 10 0
   1: 00000000:0016 00000000:0000 0A 00000000:00000000 00:00000000 00000000     0        0 234567 1 0000000000000000 100 0 0 10 0
   2: 0100007F:9999 0100007F:1234 01 00000000:00000000 00:00000000 00000000  1000        0 345678 1 0000000000000000 20 4 30 10 -1
   3: 0100007F:AAAA
";
        let rows = parse_proc_net_listen(text);
        assert_eq!(rows.len(), 2);
        assert!(rows.contains(&(3000, 123456)));
        assert!(rows.contains(&(22, 234567)));
        // ESTABLISHED and malformed rows are skipped.
        assert!(!rows.iter().any(|(port, _)| *port == 0x9999));
        assert!(!rows.iter().any(|(port, _)| *port == 0xAAAA));
    }

    #[test]
    fn parse_proc_net_listen_extracts_tcp6_listen_ports() {
        // /proc/net/tcp6 uses 32-hex-digit addresses; 0x1F90 = 8080, and the
        // loopback form 00000000000000000000000000000001 also parses.
        let text = "\
  sl  local_address                         remote_address                        st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode
   0: 00000000000000000000000000000000:1F90 00000000000000000000000000000000:0000 0A 00000000:00000000 00:00000000 00000000  1000        0 456789 1 0000000000000000 100 0 0 10 0
   1: 00000000000000000000000000000001:0050 00000000000000000000000000000000:0000 0A 00000000:00000000 00:00000000 00000000     0        0 567890 1 0000000000000000 100 0 0 10 0
";
        let rows = parse_proc_net_listen(text);
        assert_eq!(rows.len(), 2);
        assert!(rows.contains(&(8080, 456789)));
        assert!(rows.contains(&(80, 567890)));
    }

    #[test]
    fn parse_proc_net_listen_empty_and_garbage_yield_empty() {
        assert!(parse_proc_net_listen("").is_empty());
        assert!(parse_proc_net_listen("not a proc file\n0A 0A 0A").is_empty());
        // Header alone ("st" column is not "0A").
        let header = "  sl  local_address rem_address   st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode\n";
        assert!(parse_proc_net_listen(header).is_empty());
    }

    #[test]
    fn localhost_copy_only_for_loopback() {
        assert_eq!(
            localhost_copy("127.0.0.1:3000").as_deref(),
            Some("localhost:3000")
        );
        assert_eq!(
            localhost_copy("[::1]:8080").as_deref(),
            Some("localhost:8080")
        );
        assert_eq!(localhost_copy("10.0.0.4:80"), None);
    }
}
