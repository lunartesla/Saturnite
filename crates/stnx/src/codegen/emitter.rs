use crate::error::CompilerError;
use crate::target::TargetConfig;
use inkwell::module::Module;
use inkwell::targets::{FileType, TargetMachine, TargetTriple};
use std::path::Path;

pub struct ObjectEmitter<'ctx> {
    module: Module<'ctx>,
    target_machine: TargetMachine,
}

impl<'ctx> ObjectEmitter<'ctx> {
    pub fn new(module: Module<'ctx>, target_config: &TargetConfig) -> Result<Self, CompilerError> {
        let target_machine = target_config
            .create_target_machine()
            .map_err(CompilerError::Target)?;

        let module_triple = TargetTriple::create(&target_config.triple_str());
        module.set_triple(&module_triple);

        Ok(Self {
            module,
            target_machine,
        })
    }

    pub fn emit_object(&self, path: &Path) -> Result<(), CompilerError> {
        self.target_machine
            .write_to_file(&self.module, FileType::Object, path)
            .map_err(|e| CompilerError::codegen(format!("failed to write object file: {}", e)))
    }

    pub fn emit_ir(&self) -> String {
        self.module.print_to_string().to_string()
    }

    pub fn emit_ir_to_file(&self, path: &Path) -> Result<(), CompilerError> {
        self.module
            .print_to_file(path)
            .map_err(|e| CompilerError::codegen(format!("failed to write IR file: {}", e)))
    }
}
