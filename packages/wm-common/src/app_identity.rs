//! Identity constants that distinguish this fork from upstream.
//!
//! These allow the fork to be installed and configured side-by-side with
//! the upstream app. Instance locking and the IPC port are intentionally
//! shared with upstream (only one WM can run at a time, and zebar connects
//! to the same port).

/// Human-readable application name.
pub const APP_NAME: &str = "GlazeWM Scrolling";

/// Name of the CLI binary as installed on `PATH`.
pub const CLI_BIN_NAME: &str = "glazewm-scrolling";

/// Directory relative to the user's home directory that holds the config
/// file and error logs.
pub const APP_DATA_DIR: &str = ".glzr/glazewm-scrolling";

/// Environment variable for overriding the default config file path.
pub const CONFIG_PATH_ENV_VAR: &str = "GLAZEWM_SCROLLING_CONFIG_PATH";
