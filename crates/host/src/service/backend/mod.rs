//! Platform backends of the service operations (private).

#[cfg(target_os = "linux")]
pub(super) mod busybox;
#[cfg(any(target_os = "linux", all(test, unix)))]
pub(super) mod busybox_script;
#[cfg(target_os = "macos")]
pub(super) mod launchd;
#[cfg(target_os = "linux")]
pub(super) mod systemd;
#[cfg(windows)]
pub(super) mod windows;
pub(super) mod windows_scm;
#[cfg(windows)]
pub(super) mod windows_scm_impl;
