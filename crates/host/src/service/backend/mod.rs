//! Platform backends of the service operations (private).

#[cfg(target_os = "linux")]
pub(super) mod busybox;
#[cfg(target_os = "macos")]
pub(super) mod launchd;
#[cfg(target_os = "linux")]
pub(super) mod systemd;
#[cfg(windows)]
pub(super) mod windows;
