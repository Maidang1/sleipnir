use std::time::Duration;

pub struct ExitWatch {
    queue: libc::c_int,
}

impl Drop for ExitWatch {
    fn drop(&mut self) {
        // SAFETY: queue is an owned descriptor created by kqueue.
        unsafe { libc::close(self.queue) };
    }
}

pub fn register_exit_watch(pid: u32) -> Result<ExitWatch, String> {
    // SAFETY: kqueue returns a new owned descriptor or -1.
    let queue = unsafe { libc::kqueue() };
    if queue < 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    let change = libc::kevent {
        ident: pid as usize,
        filter: libc::EVFILT_PROC,
        flags: libc::EV_ADD | libc::EV_ENABLE | libc::EV_ONESHOT,
        fflags: libc::NOTE_EXIT,
        data: 0,
        udata: std::ptr::null_mut(),
    };
    // SAFETY: queue is valid and change points to one initialized event.
    let registered =
        unsafe { libc::kevent(queue, &change, 1, std::ptr::null_mut(), 0, std::ptr::null()) };
    if registered < 0 {
        // ESRCH means the process exited before registration; this is success.
        let error = std::io::Error::last_os_error();
        unsafe { libc::close(queue) };
        return Err(error.to_string());
    }
    Ok(ExitWatch { queue })
}

impl ExitWatch {
    pub fn wait(&mut self, timeout: Duration) -> Result<bool, String> {
        let timespec = libc::timespec {
            tv_sec: timeout.as_secs() as _,
            tv_nsec: timeout.subsec_nanos() as _,
        };
        let mut event = std::mem::MaybeUninit::<libc::kevent>::uninit();
        loop {
            // SAFETY: event points to writable storage for one event and queue is valid.
            let result = unsafe {
                libc::kevent(
                    self.queue,
                    std::ptr::null(),
                    0,
                    event.as_mut_ptr(),
                    1,
                    &timespec,
                )
            };
            if result > 0 {
                return Ok(true);
            }
            if result == 0 {
                return Ok(false);
            }
            let error = std::io::Error::last_os_error();
            if error.kind() != std::io::ErrorKind::Interrupted {
                return Err(error.to_string());
            }
        }
    }
}

pub fn is_alive(pid: u32) -> Result<bool, String> {
    // SAFETY: signal zero performs an existence/permission check only.
    let result = unsafe { libc::kill(pid as i32, 0) };
    if result == 0 {
        return Ok(true);
    }
    let error = std::io::Error::last_os_error();
    if error.raw_os_error() == Some(libc::ESRCH) {
        Ok(false)
    } else {
        Err(error.to_string())
    }
}

pub fn terminate_and_wait(pid: u32, timeout: Duration) -> Result<bool, String> {
    // SAFETY: SIGTERM requests graceful termination of the target process.
    let result = unsafe { libc::kill(pid as i32, libc::SIGTERM) };
    if result != 0 {
        let error = std::io::Error::last_os_error();
        if error.raw_os_error() == Some(libc::ESRCH) {
            return Ok(true);
        }
        return Err(error.to_string());
    }
    let mut watch = match register_exit_watch(pid) {
        Ok(watch) => watch,
        Err(_) if !is_alive(pid)? => return Ok(true),
        Err(error) => return Err(error),
    };
    watch.wait(timeout)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    #[test]
    fn kqueue_observes_child_exit() {
        let mut child = Command::new("/bin/sh")
            .args(["-c", "sleep 0.05"])
            .spawn()
            .unwrap();
        let mut watch = register_exit_watch(child.id()).unwrap();
        assert!(watch.wait(Duration::from_secs(2)).unwrap());
        let _ = child.wait();
    }
}
