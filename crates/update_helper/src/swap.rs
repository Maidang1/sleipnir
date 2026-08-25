use std::ffi::CString;
use std::os::unix::ffi::OsStrExt as _;
use std::path::Path;

const RENAME_SWAP: u32 = 0x0000_0002;
const AT_FDCWD: i32 = -2;

unsafe extern "C" {
    fn renameatx_np(
        from_fd: libc::c_int,
        from: *const libc::c_char,
        to_fd: libc::c_int,
        to: *const libc::c_char,
        flags: libc::c_uint,
    ) -> libc::c_int;
}

pub fn swap_paths(first: &Path, second: &Path) -> Result<(), String> {
    let first = CString::new(first.as_os_str().as_bytes())
        .map_err(|_| "first path contains NUL".to_string())?;
    let second = CString::new(second.as_os_str().as_bytes())
        .map_err(|_| "second path contains NUL".to_string())?;
    // SAFETY: both C strings are NUL terminated and remain alive for the call.
    let result = unsafe {
        renameatx_np(
            AT_FDCWD,
            first.as_ptr(),
            AT_FDCWD,
            second.as_ptr(),
            RENAME_SWAP,
        )
    };
    if result == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error().to_string())
    }
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
