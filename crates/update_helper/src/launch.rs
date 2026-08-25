use std::path::Path;

pub fn launch_application(bundle: &Path) -> Result<u32, String> {
    use block2::RcBlock;
    use objc2_app_kit::{NSRunningApplication, NSWorkspace, NSWorkspaceOpenConfiguration};
    use objc2_foundation::{NSError, NSString, NSURL};
    use std::sync::mpsc;
    use std::time::Duration;

    let path = bundle
        .to_str()
        .ok_or_else(|| "application path is not UTF-8".to_string())?;
    let url = NSURL::fileURLWithPath_isDirectory(&NSString::from_str(path), true);
    let configuration = NSWorkspaceOpenConfiguration::configuration();
    configuration.setCreatesNewApplicationInstance(true);
    configuration.setActivates(true);
    configuration.setPromptsUserIfNeeded(false);
    let (sender, receiver) = mpsc::sync_channel(1);
    let handler = RcBlock::new(
        move |application: *mut NSRunningApplication, error: *mut NSError| {
            if !error.is_null() {
                let _ = sender.send(Err("LaunchServices failed to start candidate".to_string()));
            } else if application.is_null() {
                let _ = sender.send(Err(
                    "LaunchServices returned no running application".to_string()
                ));
            } else {
                // SAFETY: NSWorkspace owns the callback object for the callback duration.
                let pid = unsafe { (&*application).processIdentifier() };
                if pid > 0 {
                    let _ = sender.send(Ok(pid as u32));
                } else {
                    let _ = sender.send(Err("candidate has no process identifier".to_string()));
                }
            }
        },
    );
    NSWorkspace::sharedWorkspace().openApplicationAtURL_configuration_completionHandler(
        &url,
        &configuration,
        Some(&handler),
    );
    receiver
        .recv_timeout(Duration::from_secs(15))
        .map_err(|_| "LaunchServices callback timed out".to_string())?
}
