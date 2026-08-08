use crate::error::{TargetError, TargetResult};
use inkwell::targets::{
    CodeModel, FileType, InitializationConfig, RelocMode, Target, TargetMachine, TargetTriple,
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OptimizationLevel {
    None,
    Less,
    Default,
    Aggressive,
}

impl Default for OptimizationLevel {
    fn default() -> Self {
        OptimizationLevel::None
    }
}

#[derive(Clone, Debug)]
pub enum DebugInfo {
    Yes,
    No,
}

impl Default for DebugInfo {
    fn default() -> Self {
        DebugInfo::No
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Copy)]
pub enum OutputKind {
    Ir,
    Object,
    Exe,
}

impl Default for OutputKind {
    fn default() -> Self {
        OutputKind::Exe
    }
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
    pub fn host() -> TargetResult<Self> {
        Self::initialize_native_target()?;
        let triple = TargetMachine::get_default_triple();
        let triple_str = triple.as_str().to_str().map(|s| s.to_string()).unwrap_or_default();
        let parsed = Self::parse_triple(&triple_str);
        Ok(Self {
            triple,
            triple_str,
            architecture: parsed.0,
            os: parsed.1,
            environment: parsed.2,
            opt_level: OptimizationLevel::default(),
            debug_info: DebugInfo::No,
            output_kind: OutputKind::Exe,
            cpu: "generic".to_string(),
            features: String::new(),
        })
    }

    pub fn from_triple(triple_str: &str) -> TargetResult<Self> {
        Self::initialize_native_target()?;
        let triple = TargetTriple::create(triple_str);

        // Validate that the target actually exists in this LLVM build
        Target::from_triple(&triple)
            .map_err(|e| TargetError::target_lookup_failed(
                triple_str.to_string(),
                format!("unknown target triple: {}", e)
            ))?;

        let parsed = Self::parse_triple(triple_str);
        Ok(Self {
            triple,
            triple_str: triple_str.to_string(),
            architecture: parsed.0,
            os: parsed.1,
            environment: parsed.2,
            opt_level: OptimizationLevel::default(),
            debug_info: DebugInfo::No,
            output_kind: OutputKind::Exe,
            cpu: "generic".to_string(),
            features: String::new(),
        })
    }

    pub fn initialize_native_target() -> TargetResult<()> {
        let config = InitializationConfig::default();
        Target::initialize_native(&config)
            .map_err(|e| TargetError {
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
        let os = if parts.iter().any(|p| *p == "windows" || *p == "winnt") {
            OperatingSystem::Windows
        } else if parts.iter().any(|p| *p == "linux") {
            OperatingSystem::Linux
        } else if parts.iter().any(|p| *p == "darwin") {
            OperatingSystem::Darwin
        } else if parts.iter().any(|p| *p == "freebsd") {
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
                RelocMode::Default,
                CodeModel::Default,
            )
            .ok_or_else(|| TargetError::target_machine_failed(self.triple_str(), "failed to create target machine".to_string()))
    }

    pub fn default_file_type(&self) -> FileType {
        match self.output_kind {
            OutputKind::Ir => FileType::Assembly,
            OutputKind::Object => FileType::Object,
            OutputKind::Exe => FileType::Object,
        }
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
