//! Derived workspace identity: git work tree of a pane cwd.

use std::path::{Path, PathBuf};

/// Grouping key for tabs. Derived at render time; not persisted.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum WorkspaceKey {
    /// A directory — the git work tree if one exists, otherwise the cwd itself.
    Path(PathBuf),
    /// No cwd is known (fresh pane, vanished restore path).
    Home,
}

impl WorkspaceKey {
    /// Group key for a pane cwd. Walks up for `.git`; falls back to the cwd
    /// itself; `None` becomes [`WorkspaceKey::Home`].
    pub fn of(cwd: Option<&Path>) -> Self {
        match cwd {
            Some(path) => Self::Path(git_root(path).unwrap_or_else(|| path.to_path_buf())),
            None => Self::Home,
        }
    }

    /// Short label for a workspace header: directory basename, or `~`.
    pub fn name(&self) -> String {
        match self {
            Self::Home => "~".into(),
            Self::Path(path) => path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .filter(|name| !name.is_empty())
                .unwrap_or_else(|| path.display().to_string()),
        }
    }
}

/// Cwd a new tab should inherit: the git root when there is one, else `cwd`.
pub fn spawn_cwd(cwd: &Path) -> PathBuf {
    git_root(cwd).unwrap_or_else(|| cwd.to_path_buf())
}

/// Nearest ancestor of `cwd` that contains a `.git` file or directory.
pub fn git_root(cwd: &Path) -> Option<PathBuf> {
    git_root_in(cwd, |candidate| candidate.exists())
}

/// Testable walk: `exists` is called with `dir.join(".git")`.
pub fn git_root_in(cwd: &Path, exists: impl Fn(&Path) -> bool) -> Option<PathBuf> {
    for dir in cwd.ancestors() {
        if exists(&dir.join(".git")) {
            return Some(dir.to_path_buf());
        }
    }
    None
}

/// Cluster tabs by workspace, preserving first-seen group order and
/// relative tab order inside each group. `items` is `(tab_index, key)`.
pub fn group_tabs<I>(items: I) -> Vec<(WorkspaceKey, Vec<usize>)>
where
    I: IntoIterator<Item = (usize, WorkspaceKey)>,
{
    let mut groups: Vec<(WorkspaceKey, Vec<usize>)> = Vec::new();
    for (index, key) in items {
        if let Some((_, tabs)) = groups.iter_mut().find(|(existing, _)| existing == &key) {
            tabs.push(index);
        } else {
            groups.push((key, vec![index]));
        }
    }
    groups
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn git_root_finds_repo_from_nested_cwd() {
        let exists = |path: &Path| path == Path::new("/a/b/repo/.git");
        assert_eq!(
            git_root_in(Path::new("/a/b/repo/src/lib"), exists),
            Some(PathBuf::from("/a/b/repo"))
        );
    }

    #[test]
    fn git_root_accepts_git_file() {
        // Worktrees and submodules use a `.git` *file*. exists() does not
        // distinguish; a file or a directory both count.
        let exists = |path: &Path| path == Path::new("/work/.git");
        assert_eq!(
            git_root_in(Path::new("/work"), exists),
            Some(PathBuf::from("/work"))
        );
    }

    #[test]
    fn git_root_none_without_repo() {
        let exists = |_path: &Path| false;
        assert_eq!(git_root_in(Path::new("/tmp/scratch"), exists), None);
    }

    #[test]
    fn workspace_of_uses_git_root_then_cwd_then_home() {
        assert_eq!(WorkspaceKey::of(None), WorkspaceKey::Home);
        assert_eq!(
            WorkspaceKey::of(Some(Path::new("/tmp/scratch"))),
            WorkspaceKey::Path(PathBuf::from("/tmp/scratch"))
        );
    }

    #[test]
    fn workspace_name_is_basename_or_home() {
        assert_eq!(WorkspaceKey::Home.name(), "~");
        assert_eq!(
            WorkspaceKey::Path(PathBuf::from("/Users/me/harbor")).name(),
            "harbor"
        );
    }

    #[test]
    fn spawn_cwd_prefers_git_root() {
        // No real .git here, so spawn_cwd returns the input.
        let path = Path::new("/tmp/not-a-repo");
        assert_eq!(spawn_cwd(path), path.to_path_buf());
    }

    #[test]
    fn group_tabs_keeps_relative_order_and_first_seen_groups() {
        let harbor = WorkspaceKey::Path(PathBuf::from("/src/harbor"));
        let other = WorkspaceKey::Path(PathBuf::from("/src/other"));
        let groups = group_tabs([(0, harbor.clone()), (1, other.clone()), (2, harbor.clone())]);
        assert_eq!(groups, vec![(harbor, vec![0, 2]), (other, vec![1])]);
    }
}
