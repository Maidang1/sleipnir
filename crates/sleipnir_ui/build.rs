//! Detect macOS SDK major version for traffic-light leading pad (71 vs 78).
//! Package-local cfg — mirrors Zed `crates/ui/build.rs` at pin 371a7d4.

#![allow(clippy::disallowed_methods, reason = "build scripts are exempt")]

fn main() {
    println!("cargo::rustc-check-cfg=cfg(macos_sdk_26_or_later)");

    #[cfg(target_os = "macos")]
    {
        use std::process::Command;

        let output = Command::new("xcrun")
            .args(["--sdk", "macosx", "--show-sdk-version"])
            .output();

        if let Ok(output) = output {
            let sdk_version = String::from_utf8_lossy(&output.stdout);
            let major_version: Option<u32> = sdk_version
                .trim()
                .split('.')
                .next()
                .and_then(|v| v.parse().ok());

            if let Some(major) = major_version
                && major >= 26
            {
                println!("cargo:rustc-cfg=macos_sdk_26_or_later");
            }
        }
    }
}
