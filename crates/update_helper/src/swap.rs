use std::path::Path;

pub fn swap_paths(first: &Path, second: &Path) -> Result<(), String> {
    updater::install::swap_paths(first, second)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn swaps_two_directories_atomically() {
        let root = tempdir().unwrap();
        let a = root.path().join("a");
        let b = root.path().join("b");
        std::fs::create_dir(&a).unwrap();
        std::fs::create_dir(&b).unwrap();
        std::fs::write(a.join("version"), "old").unwrap();
        std::fs::write(b.join("version"), "new").unwrap();
        swap_paths(&a, &b).unwrap();
        assert_eq!(std::fs::read_to_string(a.join("version")).unwrap(), "new");
        assert_eq!(std::fs::read_to_string(b.join("version")).unwrap(), "old");
        swap_paths(&a, &b).unwrap();
        assert_eq!(std::fs::read_to_string(a.join("version")).unwrap(), "old");
    }
}
