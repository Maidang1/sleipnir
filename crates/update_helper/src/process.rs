use std::time::{Duration, Instant};

pub fn wait_for_exit(pid: u32, timeout: Duration) -> Result<bool, String> {
    // SAFETY: kqueue returns a new owned descriptor or -1.
    let queue = unsafe { libc::kqueue() };
    if queue < 0 { return Err(std::io::Error::last_os_error().to_string()); }
    let change = libc::kevent {
        ident: pid as usize,
        filter: libc::EVFILT_PROC,
        flags: libc::EV_ADD | libc::EV_ENABLE | libc::EV_ONESHOT,
        fflags: libc::NOTE_EXIT,
        data: 0,
        udata: std::ptr::null_mut(),
    };
    // SAFETY: queue is valid and change points to one initialized event.
    let registered = unsafe { libc::kevent(queue, &change, 1, std::ptr::null_mut(), 0, std::ptr::null()) };
    if registered < 0 {
        // ESRCH means the process exited before registration; this is success.
        let error = std::io::Error::last_os_error();
        unsafe { libc::close(queue) };
        return if error.raw_os_error() == Some(libc::ESRCH) { Ok(true) } else { Err(error.to_string()) };
    }
    let start = Instant::now();
    let mut event = change;
    loop {
        let remaining = timeout.saturating_sub(start.elapsed());
        if remaining.is_zero() { unsafe { libc::close(queue) }; return Ok(false); }
        let timespec = libc::timespec { tv_sec: remaining.as_secs() as _, tv_nsec: remaining.subsec_nanos() as _ };
        // SAFETY: pointers reference initialized storage for the duration of the call.
        let result = unsafe { libc::kevent(queue, std::ptr::null(), 0, &mut event, 1, &timespec) };
        if result > 0 { unsafe { libc::close(queue) }; return Ok(true); }
        if result == 0 { unsafe { libc::close(queue) }; return Ok(false); }
        let error = std::io::Error::last_os_error();
        if error.kind() != std::io::ErrorKind::Interrupted {
            unsafe { libc::close(queue) };
            return Err(error.to_string());
        }
    }
}

pub fn is_alive(pid: u32) -> Result<bool, String> {
    // SAFETY: signal zero performs an existence/permission check only.
    let result = unsafe { libc::kill(pid as i32, 0) };
    if result == 0 { return Ok(true); }
    let error = std::io::Error::last_os_error();
    if error.raw_os_error() == Some(libc::ESRCH) { Ok(false) } else { Err(error.to_string()) }
}

pub fn terminate_and_wait(pid: u32, timeout: Duration) -> Result<bool, String> {
    // SAFETY: SIGTERM requests graceful termination of the target process.
    let result = unsafe { libc::kill(pid as i32, libc::SIGTERM) };
    if result != 0 {
        let error = std::io::Error::last_os_error();
        if error.raw_os_error() == Some(libc::ESRCH) { return Ok(true); }
        return Err(error.to_string());
    }
    wait_for_exit(pid, timeout)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    #[test]
    fn kqueue_observes_child_exit() {
        let mut child = Command::new("/bin/sh").args(["-c", "sleep 0.05"]).spawn().unwrap();
        assert!(wait_for_exit(child.id(), Duration::from_secs(2)).unwrap());
        let _ = child.wait();
    }
}
