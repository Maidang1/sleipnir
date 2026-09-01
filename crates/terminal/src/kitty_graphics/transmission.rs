use super::protocol::Transmission;

const MAX_FILE_SIZE: usize = 512 * 1024 * 1024;

pub fn read_transmission(
    transmission: Transmission,
    path_bytes: &[u8],
    compression: bool,
) -> Result<Vec<u8>, String> {
    let data = match transmission {
        Transmission::Direct => {
            return Err("EINVAL:direct transmission should not use read_transmission".to_string());
        }
        Transmission::File => read_file(path_bytes)?,
        Transmission::TempFile => read_temp_file(path_bytes)?,
        Transmission::SharedMemory => read_shared_memory(path_bytes)?,
    };

    if compression {
        decompress_zlib(&data)
    } else {
        Ok(data)
    }
}

fn parse_path(path_bytes: &[u8]) -> Result<std::path::PathBuf, String> {
    let s = std::str::from_utf8(path_bytes)
        .map_err(|_| "EINVAL:path is not valid UTF-8".to_string())?;
    let path = std::path::PathBuf::from(s);
    if !path.is_absolute() {
        return Err("EINVAL:path must be absolute".to_string());
    }
    if s.contains("..") {
        return Err("EINVAL:path must not contain '..' segments".to_string());
    }
    Ok(path)
}

fn read_file(path_bytes: &[u8]) -> Result<Vec<u8>, String> {
    let path = parse_path(path_bytes)?;
    let meta = std::fs::metadata(&path)
        .map_err(|e| format!("ENOENT:cannot stat file: {e}"))?;
    if meta.len() > MAX_FILE_SIZE as u64 {
        return Err("EINVAL:file too large".to_string());
    }
    std::fs::read(&path).map_err(|e| format!("EIO:cannot read file: {e}"))
}

fn read_temp_file(path_bytes: &[u8]) -> Result<Vec<u8>, String> {
    let path = parse_path(path_bytes)?;
    let tmp_dir = std::env::temp_dir();
    if !path.starts_with(&tmp_dir) {
        return Err(format!(
            "EINVAL:temp file path must be under {}",
            tmp_dir.display()
        ));
    }
    let data = read_file(path_bytes)?;
    let _ = std::fs::remove_file(&path);
    Ok(data)
}

#[cfg(unix)]
fn read_shared_memory(name_bytes: &[u8]) -> Result<Vec<u8>, String> {
    use std::ffi::CString;
    use std::os::unix::io::FromRawFd;

    let name = std::str::from_utf8(name_bytes)
        .map_err(|_| "EINVAL:shm name is not valid UTF-8".to_string())?;
    let c_name =
        CString::new(name).map_err(|_| "EINVAL:shm name contains null byte".to_string())?;

    let fd = unsafe { libc::shm_open(c_name.as_ptr(), libc::O_RDONLY, 0) };
    if fd < 0 {
        return Err(format!(
            "ENOENT:shm_open failed: {}",
            std::io::Error::last_os_error()
        ));
    }

    let result = (|| -> Result<Vec<u8>, String> {
        let mut stat: libc::stat = unsafe { std::mem::zeroed() };
        if unsafe { libc::fstat(fd, &mut stat) } != 0 {
            return Err(format!(
                "EIO:fstat failed: {}",
                std::io::Error::last_os_error()
            ));
        }
        let size = stat.st_size as usize;
        if size > MAX_FILE_SIZE {
            return Err("EINVAL:shared memory segment too large".to_string());
        }
        if size == 0 {
            return Ok(Vec::new());
        }

        let file = unsafe { std::fs::File::from_raw_fd(fd) };
        let mmap = unsafe {
            memmap2::MmapOptions::new()
                .len(size)
                .map(&file)
                .map_err(|e| format!("EIO:mmap failed: {e}"))?
        };
        let data = mmap[..size].to_vec();
        std::mem::forget(file);
        Ok(data)
    })();

    unsafe {
        libc::shm_unlink(c_name.as_ptr());
        libc::close(fd);
    }

    result
}

#[cfg(not(unix))]
fn read_shared_memory(_name_bytes: &[u8]) -> Result<Vec<u8>, String> {
    Err("ENOTSUP:shared memory transmission not supported on this platform".to_string())
}

fn decompress_zlib(data: &[u8]) -> Result<Vec<u8>, String> {
    use std::io::Read;
    let mut decoder = flate2::read::ZlibDecoder::new(data);
    let mut out = Vec::new();
    decoder
        .read_to_end(&mut out)
        .map_err(|e| format!("EIO:zlib decompression failed: {e}"))?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn file_read_basic() {
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        tmp.write_all(b"hello kitty").unwrap();
        let path = tmp.path().to_str().unwrap().as_bytes().to_vec();
        let data = read_transmission(Transmission::File, &path, false).unwrap();
        assert_eq!(data, b"hello kitty");
    }

    #[test]
    fn file_rejects_relative_path() {
        let err = read_transmission(Transmission::File, b"relative/path.png", false).unwrap_err();
        assert!(err.contains("EINVAL:path must be absolute"));
    }

    #[test]
    fn file_rejects_dotdot() {
        let err =
            read_transmission(Transmission::File, b"/tmp/../etc/passwd", false).unwrap_err();
        assert!(err.contains("EINVAL:path must not contain '..'"));
    }

    #[test]
    fn file_nonexistent() {
        let err = read_transmission(
            Transmission::File,
            b"/tmp/sleipnir_nonexistent_test_file_42",
            false,
        )
        .unwrap_err();
        assert!(err.contains("ENOENT"));
    }

    #[test]
    fn temp_file_read_and_delete() {
        let tmp_dir = std::env::temp_dir();
        let file_path = tmp_dir.join("sleipnir_test_tempfile_xmit");
        std::fs::write(&file_path, b"temp data").unwrap();
        let path_bytes = file_path.to_str().unwrap().as_bytes().to_vec();
        let data = read_transmission(Transmission::TempFile, &path_bytes, false).unwrap();
        assert_eq!(data, b"temp data");
        assert!(!file_path.exists(), "temp file should be deleted after read");
    }

    #[test]
    fn temp_file_rejects_outside_tmp() {
        let err =
            read_transmission(Transmission::TempFile, b"/usr/local/evil.png", false).unwrap_err();
        assert!(err.contains("EINVAL:temp file path must be under"));
    }

    #[test]
    fn zlib_decompression() {
        use flate2::write::ZlibEncoder;
        use flate2::Compression;

        let original = b"decompression test data for kitty graphics";
        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(original).unwrap();
        let compressed = encoder.finish().unwrap();

        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        tmp.write_all(&compressed).unwrap();
        let path = tmp.path().to_str().unwrap().as_bytes().to_vec();
        let data = read_transmission(Transmission::File, &path, true).unwrap();
        assert_eq!(data, original);
    }

    #[test]
    fn direct_transmission_errors() {
        let err = read_transmission(Transmission::Direct, b"", false).unwrap_err();
        assert!(err.contains("EINVAL"));
    }

    #[cfg(unix)]
    #[test]
    fn shm_nonexistent() {
        let err = read_transmission(
            Transmission::SharedMemory,
            b"/sleipnir_nonexistent_shm_42",
            false,
        )
        .unwrap_err();
        assert!(err.contains("ENOENT"));
    }

    #[cfg(unix)]
    #[test]
    fn shm_roundtrip() {
        use std::ffi::CString;

        let shm_name = "/sleipnir_test_shm_roundtrip";
        let c_name = CString::new(shm_name).unwrap();
        let payload = b"shared memory pixels";

        unsafe {
            let fd = libc::shm_open(
                c_name.as_ptr(),
                libc::O_CREAT | libc::O_RDWR,
                0o600,
            );
            assert!(fd >= 0, "shm_open for write failed");
            let rc = libc::ftruncate(fd, payload.len() as libc::off_t);
            assert_eq!(rc, 0, "ftruncate failed");
            let ptr = libc::mmap(
                std::ptr::null_mut(),
                payload.len(),
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_SHARED,
                fd,
                0,
            );
            assert_ne!(ptr, libc::MAP_FAILED);
            std::ptr::copy_nonoverlapping(payload.as_ptr(), ptr as *mut u8, payload.len());
            libc::munmap(ptr, payload.len());
            libc::close(fd);
        }

        let data =
            read_transmission(Transmission::SharedMemory, shm_name.as_bytes(), false).unwrap();
        assert!(data.len() >= payload.len());
        assert_eq!(&data[..payload.len()], payload);
    }
}
