//! Workspace helpers derived from a pane cwd.

use std::path::{Path, PathBuf};

/// Tab-chip path: last two cwd components, e.g. `/Users/me/src/app` → `src/app`.
/// No cwd → `~`.
pub fn tab_path_label(cwd: Option<&Path>) -> String {
    let Some(path) = cwd else {
        return "~".into();
    };
    let names: Vec<&std::ffi::OsStr> = path
        .components()
        .filter_map(|c| match c {
            std::path::Component::Normal(name) => Some(name),
            _ => None,
        })
        .collect();
    match names.as_slice() {
        [] => "~".into(),
        [one] => one.to_string_lossy().into_owned(),
        [.., parent, last] => {
            format!("{}/{}", parent.to_string_lossy(), last.to_string_lossy())
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
    fn tab_path_label_uses_last_two_components() {
        assert_eq!(tab_path_label(None), "~");
        assert_eq!(tab_path_label(Some(Path::new("/"))), "~");
        assert_eq!(tab_path_label(Some(Path::new("/tmp"))), "tmp");
        assert_eq!(
            tab_path_label(Some(Path::new("/Users/bytedance"))),
            "Users/bytedance"
        );
        assert_eq!(
            tab_path_label(Some(Path::new("/Users/bytedance/docs/huangjin"))),
            "docs/huangjin"
        );
        assert_eq!(
            tab_path_label(Some(Path::new("/Users/bytedance/.config"))),
            "bytedance/.config"
        );
        assert_eq!(
            tab_path_label(Some(Path::new("/Users/bytedance/codes/myself/harbor"))),
            "myself/harbor"
        );
    }

    #[test]
    fn spawn_cwd_prefers_git_root() {
        // No real .git here, so spawn_cwd returns the input.
        let path = Path::new("/tmp/not-a-repo");
        assert_eq!(spawn_cwd(path), path.to_path_buf());
    }
}
