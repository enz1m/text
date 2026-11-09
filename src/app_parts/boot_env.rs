//! TextApp boot environment snapshot.

// Session delivery precedes settled document reads; capture readiness covers both.
pub(crate) const CAPTURE_SETTLED_ENV: &str = "TEXT_APP_CAPTURE_SETTLED";

pub(crate) fn capture_settle_requested() -> bool {
    std::env::var_os(CAPTURE_SETTLED_ENV).is_some_and(|v| v == "1")
}

// `/dev/null/` is not a directory, so a save fails if persistence is enabled
// on such an app by mistake. Tests that persist use
// [`TextApp::for_test_persisting_to`] with a sandbox path.
#[cfg(test)]
pub(crate) const TEST_INERT_SETTINGS_PATH: &str = "/dev/null/text-app-inert-settings.conf";

// Snapshot environment once so session and settings resolve identical roots.
#[derive(Clone, Debug)]
pub(crate) struct BootEnv {
    pub(crate) xdg_config_home: Option<std::path::PathBuf>,
    pub(crate) xdg_data_home: Option<std::path::PathBuf>,
    pub(crate) home: Option<std::path::PathBuf>,
    pub(crate) cli_args: Vec<std::path::PathBuf>,
    pub(crate) runtime_fallback: Option<std::path::PathBuf>,
}

impl BootEnv {
    pub(crate) fn from_parts(
        xdg_config_home: Option<std::ffi::OsString>,
        xdg_data_home: Option<std::ffi::OsString>,
        home: Option<std::ffi::OsString>,
        cli_args: impl Iterator<Item = std::ffi::OsString>,
        runtime_fallback: Option<std::path::PathBuf>,
    ) -> Self {
        Self {
            xdg_config_home: xdg_config_home.map(std::path::PathBuf::from),
            xdg_data_home: xdg_data_home.map(std::path::PathBuf::from),
            home: home.map(std::path::PathBuf::from),
            cli_args: cli_args.map(std::path::PathBuf::from).collect(),
            runtime_fallback,
        }
    }

    pub(crate) fn from_launch(intent: &enzui_core::activation::ApplicationIntent) -> Self {
        Self::from_parts(
            std::env::var_os("XDG_CONFIG_HOME"),
            std::env::var_os("XDG_DATA_HOME"),
            std::env::var_os("HOME"),
            crate::desktop::local_paths(intent)
                .into_iter()
                .map(std::path::PathBuf::into_os_string),
            Self::resolve_runtime_fallback(),
        )
    }

    // `None` (no owner-verified directory) makes boot disable persistence
    // rather than write to a predictable `/tmp` path.
    pub(crate) fn resolve_runtime_fallback() -> Option<std::path::PathBuf> {
        enzui_core::runtime_dir::runtime_dir()
            .ok()
            .map(|dir| dir.path().to_path_buf())
    }
}
