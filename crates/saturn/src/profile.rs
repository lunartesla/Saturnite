//! Build profiles and platform abstraction.
//!
//! Mirrors the compiler-level [`stnx::target::Profile`] but is owned by the
//! toolchain so the CLI never depends directly on compiler internals beyond
//! the `Profile` enum re-export.

use stnx::target::{DebugInfo, OptimizationLevel, TargetConfig};

/// Build profile — debug (default) or release (optimized).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum BuildProfile {
    /// Debug profile: low/no optimization, debug info enabled.
    #[default]
    Debug,
    /// Release profile: LLVM optimizations enabled, no debug info.
    Release,
}

impl BuildProfile {
    /// Human-readable name: `"debug"` or `"release"`.
    pub fn as_str(self) -> &'static str {
        match self {
            BuildProfile::Debug => "debug",
            BuildProfile::Release => "release",
        }
    }

    /// Directory name under `target/` for this profile.
    pub fn dir_name(self) -> &'static str {
        self.as_str()
    }

    /// Whether this is the release profile.
    pub fn is_release(self) -> bool {
        matches!(self, BuildProfile::Release)
    }

    /// Apply this profile to a [`TargetConfig`], setting optimization level
    /// and debug-info appropriately.
    pub fn apply_to_config(self, config: &mut TargetConfig) {
        let opt = match self {
            BuildProfile::Debug => OptimizationLevel::None,
            BuildProfile::Release => OptimizationLevel::Aggressive,
        };
        let dbg = match self {
            BuildProfile::Debug => DebugInfo::Yes,
            BuildProfile::Release => DebugInfo::No,
        };
        config.set_opt_level(opt);
        config.set_debug_info(dbg);
    }

    /// Convert to the compiler's `Profile` enum.
    pub fn to_compiler_profile(self) -> stnx::target::Profile {
        match self {
            BuildProfile::Debug => stnx::target::Profile::Debug,
            BuildProfile::Release => stnx::target::Profile::Release,
        }
    }
}

/// Platform detection and target-aware helpers.
pub struct PlatformInfo;

impl PlatformInfo {
    /// Whether the current platform is Windows.
    pub fn is_windows() -> bool {
        std::env::consts::OS == "windows"
    }

    /// Whether the current platform is Linux.
    pub fn is_linux() -> bool {
        std::env::consts::OS == "linux"
    }

    /// Whether the current platform is macOS.
    pub fn is_macos() -> bool {
        std::env::consts::OS == "macos"
    }

    /// The host target triple as a string.
    pub fn triple() -> String {
        TargetConfig::host()
            .map(|c| c.triple_str())
            .unwrap_or_else(|_| "unknown".to_string())
    }
}

/// Build the platform-appropriate executable file name for a given package
/// name.
///
/// On Windows this appends `.exe`; on other platforms it is the bare name.
pub fn executable_name(package_name: &str) -> String {
    if PlatformInfo::is_windows() {
        format!("{}.exe", package_name)
    } else {
        package_name.to_string()
    }
}

/// Return the host platform triple string (best-effort).
pub fn platform_triple() -> String {
    PlatformInfo::triple()
}
