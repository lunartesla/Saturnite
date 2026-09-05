use crate::error::{TargetError, TargetResult};
use inkwell::targets::{
    CodeModel, InitializationConfig, RelocMode, Target, TargetMachine, TargetTriple,
};
use inkwell::OptimizationLevel as InkwellOptLevel;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Architecture {
    X86_64,
    Aarch64,
    X86,
    Arm,
    Riscv64,
    Mips,
    Powerpc64,
    Wasm32,
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OperatingSystem {
    Windows,
    Linux,
    Darwin,
    FreeBSD,
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Environment {
    Msvc,
    Gnu,
    Musl,
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub enum OptimizationLevel {
    #[default]
    None,
    Less,
    Default,
    Aggressive,
}

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub enum DebugInfo {
    Yes,
    #[default]
    No,
}

/// Build profile controlling the default optimization level and debug-info
/// emission.  This centralizes the mapping that previously appeared inline in
/// `main.rs` so callers can derive a coherent `(OptimizationLevel, DebugInfo)`
/// pair from a single value.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Profile {
    #[default]
    Debug,
    Release,
}

impl Profile {
    /// Human-readable name of the profile (`"debug"` / `"release"`).
    pub fn as_str(&self) -> &'static str {
        match self {
            Profile::Debug => "debug",
            Profile::Release => "release",
        }
    }

    /// Whether this is the release (optimized, no debug info) profile.
    pub fn is_release(&self) -> bool {
        matches!(self, Profile::Release)
    }

    /// The default `OptimizationLevel` for this profile.
    pub fn opt_level(&self) -> OptimizationLevel {
        match self {
            Profile::Debug => OptimizationLevel::None,
            Profile::Release => OptimizationLevel::Aggressive,
        }
    }

    /// The default `DebugInfo` setting for this profile.
    pub fn debug_info(&self) -> DebugInfo {
        match self {
            Profile::Debug => DebugInfo::Yes,
            Profile::Release => DebugInfo::No,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Copy, Default)]
pub enum OutputKind {
    Ir,
    Object,
    #[default]
    Exe,
}

#[derive(Debug)]
pub struct TargetConfig {
    triple: TargetTriple,
    triple_str: String,
    architecture: Architecture,
    os: OperatingSystem,
    environment: Environment,
    opt_level: OptimizationLevel,
    debug_info: DebugInfo,
    output_kind: OutputKind,
    cpu: String,
    features: String,
}

impl TargetConfig {
    /// Build a [`TargetConfig`] from an already-parsed triple and its
    /// string form, applying the shared default field values used by both
    /// [`TargetConfig::host`] and [`TargetConfig::from_triple`].
    ///
    /// The caller is responsible for the target triple and parsed components;
    /// this helper centralizes the *remaining* default initialization so the
    /// optimization level, debug-info, output kind, CPU and feature fields all
    /// stay in lock-step.
    fn with_defaults(
        triple: TargetTriple,
        triple_str: String,
        architecture: Architecture,
        os: OperatingSystem,
        environment: Environment,
    ) -> Self {
        Self {
            triple,
            triple_str,
            architecture,
            os,
            environment,
            opt_level: OptimizationLevel::default(),
            debug_info: DebugInfo::No,
            output_kind: OutputKind::Exe,
            cpu: "generic".to_string(),
            features: String::new(),
        }
    }

    pub fn host() -> TargetResult<Self> {
        Self::initialize_native_target()?;
        let triple = TargetMachine::get_default_triple();
        let triple_str = triple
            .as_str()
            .to_str()
            .map(|s| s.to_string())
            .unwrap_or_default();
        let parsed = Self::parse_triple(&triple_str);
        Ok(Self::with_defaults(
            triple, triple_str, parsed.0, parsed.1, parsed.2,
        ))
    }

    pub fn from_triple(triple_str: &str) -> TargetResult<Self> {
        Self::initialize_native_target()?;
        let triple = TargetTriple::create(triple_str);

        // Validate that the target actually exists in this LLVM build
        Target::from_triple(&triple).map_err(|e| {
            TargetError::target_lookup_failed(
                triple_str.to_string(),
                format!("unknown target triple: {}", e),
            )
        })?;

        let parsed = Self::parse_triple(triple_str);
        Ok(Self::with_defaults(
            triple,
            triple_str.to_string(),
            parsed.0,
            parsed.1,
            parsed.2,
        ))
    }

    /// Construct a [`TargetConfig`] for the host target that has its
    /// optimization level and debug-info fields set according to `profile`.
    ///
    /// This is the single, centralized mapping from [`Profile`] to the
    /// `(OptimizationLevel, DebugInfo)` pair — callers that previously
    /// re-derived these values by hand (e.g. in `main.rs`) should use this
    /// constructor instead.
    pub fn with_profile(profile: Profile) -> TargetResult<Self> {
        let mut config = Self::host()?;
        config.apply_profile(profile);
        Ok(config)
    }

    /// Apply a [`Profile`] to this config in place, setting the optimization
    /// level and debug-info fields to their profile-appropriate defaults.
    ///
    /// Explicit overrides applied afterwards (e.g. via `set_opt_level` /
    /// `set_debug_info`) still take precedence — this only establishes the
    /// baseline pair for the chosen profile.
    pub fn apply_profile(&mut self, profile: Profile) {
        self.opt_level = profile.opt_level();
        self.debug_info = profile.debug_info();
    }

    pub fn initialize_native_target() -> TargetResult<()> {
        let config = InitializationConfig::default();
        Target::initialize_native(&config).map_err(|e| TargetError {
            message: format!("failed to initialize native LLVM target: {}", e),
            triple: None,
        })?;
        Ok(())
    }

    fn parse_triple(s: &str) -> (Architecture, OperatingSystem, Environment) {
        let parts: Vec<&str> = s.split('-').collect();

        // First part is always the architecture
        let arch = match parts.first().copied() {
            Some("x86_64" | "amd64") => Architecture::X86_64,
            Some("aarch64" | "arm64") => Architecture::Aarch64,
            Some("i686" | "i386" | "i486" | "i586") => Architecture::X86,
            Some("arm" | "armv7" | "armv6") => Architecture::Arm,
            Some("riscv64") => Architecture::Riscv64,
            Some("mips" | "mips64") => Architecture::Mips,
            Some("ppc64" | "powerpc64") => Architecture::Powerpc64,
            Some("wasm32") => Architecture::Wasm32,
            _ => Architecture::Unknown,
        };

        // OS is found by scanning parts for known OS keywords (handles the
        // optional `unknown` vendor field that may sit between arch and os).
        let os = if parts.contains(&"windows") || parts.contains(&"winnt") {
            OperatingSystem::Windows
        } else if parts.contains(&"linux") {
            OperatingSystem::Linux
        } else if parts.contains(&"darwin") {
            OperatingSystem::Darwin
        } else if parts.contains(&"freebsd") {
            OperatingSystem::FreeBSD
        } else {
            OperatingSystem::Unknown
        };

        // Environment is typically the last part of the triple
        let env = match parts.last().copied() {
            Some("msvc") => Environment::Msvc,
            Some("gnu") => Environment::Gnu,
            Some("musl") => Environment::Musl,
            _ => Environment::Unknown,
        };

        (arch, os, env)
    }

    pub fn triple(&self) -> &TargetTriple {
        &self.triple
    }

    pub fn triple_str(&self) -> String {
        self.triple_str.clone()
    }

    pub fn architecture(&self) -> &Architecture {
        &self.architecture
    }

    pub fn os(&self) -> &OperatingSystem {
        &self.os
    }

    pub fn environment(&self) -> &Environment {
        &self.environment
    }

    pub fn opt_level(&self) -> &OptimizationLevel {
        &self.opt_level
    }

    pub fn debug_info(&self) -> &DebugInfo {
        &self.debug_info
    }

    pub fn output_kind(&self) -> &OutputKind {
        &self.output_kind
    }

    pub fn set_opt_level(&mut self, level: OptimizationLevel) {
        self.opt_level = level;
    }

    pub fn set_debug_info(&mut self, info: DebugInfo) {
        self.debug_info = info;
    }

    pub fn set_output_kind(&mut self, kind: OutputKind) {
        self.output_kind = kind;
    }

    pub fn set_cpu(&mut self, cpu: impl Into<String>) {
        self.cpu = cpu.into();
    }

    pub fn set_features(&mut self, features: impl Into<String>) {
        self.features = features.into();
    }

    pub fn to_inkwell_opt_level(&self) -> InkwellOptLevel {
        match self.opt_level {
            OptimizationLevel::None => InkwellOptLevel::None,
            OptimizationLevel::Less => InkwellOptLevel::Less,
            OptimizationLevel::Default => InkwellOptLevel::Default,
            OptimizationLevel::Aggressive => InkwellOptLevel::Aggressive,
        }
    }

    /// The LLVM module-level optimization pass pipeline name corresponding to
    /// this config's optimization level.
    ///
    /// This centralizes the mapping previously duplicated in `mir::codegen`
    /// (`OptimizationLevel → "default<O0>" | ... | "default<O3>"`); that crate
    /// should call this method instead of matching on `OptimizationLevel`
    /// itself, so the mapping lives in exactly one place.
    pub fn opt_pass_name(&self) -> &'static str {
        match self.opt_level {
            OptimizationLevel::None => "default<O0>",
            OptimizationLevel::Less => "default<O1>",
            OptimizationLevel::Default => "default<O2>",
            OptimizationLevel::Aggressive => "default<O3>",
        }
    }

    pub fn create_target_machine(&self) -> TargetResult<TargetMachine> {
        Self::initialize_native_target()?;

        let target = Target::from_triple(&self.triple)
            .map_err(|e| TargetError::target_lookup_failed(self.triple_str(), e.to_string()))?;

        target
            .create_target_machine(
                &self.triple,
                &self.cpu,
                &self.features,
                self.to_inkwell_opt_level(),
                RelocMode::PIC,
                CodeModel::Default,
            )
            .ok_or_else(|| {
                TargetError::target_machine_failed(
                    self.triple_str(),
                    "failed to create target machine".to_string(),
                )
            })
    }
}

impl std::fmt::Display for TargetConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Triple: {}, Architecture: {:?}, OS: {:?}, Environment: {:?}",
            self.triple_str(),
            self.architecture,
            self.os,
            self.environment
        )
    }
}

// ---------------------------------------------------------------------------
// Profile <-> (OptimizationLevel, DebugInfo) mapping
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_is_release_only_for_release() {
        assert!(!Profile::Debug.is_release());
        assert!(Profile::Release.is_release());
    }

    #[test]
    fn profile_default_is_debug() {
        assert_eq!(Profile::default(), Profile::Debug);
    }

    #[test]
    fn profile_opt_level_mapping() {
        assert_eq!(Profile::Debug.opt_level(), OptimizationLevel::None);
        assert_eq!(Profile::Release.opt_level(), OptimizationLevel::Aggressive);
    }

    #[test]
    fn profile_debug_info_mapping() {
        assert_eq!(Profile::Debug.debug_info(), DebugInfo::Yes);
        assert_eq!(Profile::Release.debug_info(), DebugInfo::No);
    }

    #[test]
    fn profile_maps_to_expected_pairs() {
        // Mirrors the behaviour that main.rs previously inlined 3 times:
        //   debug     -> None / Yes
        //   release   -> Aggressive / No
        let debug_pair = (Profile::Debug.opt_level(), Profile::Debug.debug_info());
        assert_eq!(debug_pair, (OptimizationLevel::None, DebugInfo::Yes));

        let release_pair = (Profile::Release.opt_level(), Profile::Release.debug_info());
        assert_eq!(release_pair, (OptimizationLevel::Aggressive, DebugInfo::No));
    }

    #[test]
    fn opt_pass_name_matches_optimization_level() {
        // The pass-pipeline name must stay in sync with the optimization level,
        // matching what mir::codegen previously hard-coded per variant.
        let cases = [
            (OptimizationLevel::None, "default<O0>"),
            (OptimizationLevel::Less, "default<O1>"),
            (OptimizationLevel::Default, "default<O2>"),
            (OptimizationLevel::Aggressive, "default<O3>"),
        ];
        for (level, expected) in cases {
            // Build a config purely to test the method; we avoid LLVM-dependent
            // constructors and set the field directly through the struct path.
            // `OptimizationLevel` is not `Copy`, so clone it for the field.
            let config = TargetConfig {
                triple: TargetTriple::create("x86_64-unknown-linux"),
                triple_str: "x86_64-unknown-linux".to_string(),
                architecture: Architecture::X86_64,
                os: OperatingSystem::Linux,
                environment: Environment::Unknown,
                opt_level: level.clone(),
                debug_info: DebugInfo::No,
                output_kind: OutputKind::Exe,
                cpu: "generic".to_string(),
                features: String::new(),
            };
            assert_eq!(
                config.opt_pass_name(),
                expected,
                "pass name mismatch for {:?}",
                level
            );

            // And the inkwell translation must agree on non-None => a level
            // other than None (None => None, everything else => Some level).
            let ink = config.to_inkwell_opt_level();
            match level {
                OptimizationLevel::None => assert_eq!(ink, InkwellOptLevel::None),
                _ => assert_ne!(ink, InkwellOptLevel::None),
            }
        }
    }

    #[test]
    fn apply_profile_sets_opt_level_and_debug_info() {
        let mut config = TargetConfig {
            triple: TargetTriple::create("x86_64-unknown-linux"),
            triple_str: "x86_64-unknown-linux".to_string(),
            architecture: Architecture::X86_64,
            os: OperatingSystem::Linux,
            environment: Environment::Unknown,
            opt_level: OptimizationLevel::Less,
            debug_info: DebugInfo::No,
            output_kind: OutputKind::Exe,
            cpu: "generic".to_string(),
            features: String::new(),
        };

        // Start from a non-default state to prove apply_profile overwrites.
        config.apply_profile(Profile::Release);
        assert_eq!(*config.opt_level(), OptimizationLevel::Aggressive);
        assert_eq!(*config.debug_info(), DebugInfo::No);

        config.apply_profile(Profile::Debug);
        assert_eq!(*config.opt_level(), OptimizationLevel::None);
        assert_eq!(*config.debug_info(), DebugInfo::Yes);
    }
}
