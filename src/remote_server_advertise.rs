//! mDNS service advertisement for the remote Boo daemon.
//!
//! This module owns the tiny lifecycle wrapper around the platform-specific
//! publisher process used to announce `_boo._tcp`.

use std::process::{Child, Command, Stdio};

pub(crate) struct ServiceAdvertiser {
    child: Child,
}

impl Drop for ServiceAdvertiser {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl ServiceAdvertiser {
    pub(crate) fn spawn(service_name: &str, port: u16) -> Option<Self> {
        #[cfg(target_os = "macos")]
        let mut command = {
            let mut command = Command::new("dns-sd");
            command
                .args(["-R", service_name, "_boo._tcp", "local", &port.to_string()])
                .stdout(Stdio::null())
                .stderr(Stdio::null());
            command
        };

        #[cfg(target_os = "linux")]
        let mut command = {
            let mut command = Command::new("avahi-publish-service");
            command
                .args([service_name, "_boo._tcp", &port.to_string()])
                .stdout(Stdio::null())
                .stderr(Stdio::null());
            command
        };

        command.spawn().ok().map(|child| Self { child })
    }
}
