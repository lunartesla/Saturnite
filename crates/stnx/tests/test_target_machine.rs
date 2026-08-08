//! Tests raw inkwell TargetMachine creation and object-file writing using
//! an isolated temp directory (no fixed /tmp paths).

use inkwell::context::Context;
use inkwell::targets::{
    CodeModel, FileType, InitializationConfig, RelocMode, Target, TargetMachine,
};
use inkwell::OptimizationLevel;
use tempfile::TempDir;

#[test]
fn test_target_machine_creation() {
    let config = InitializationConfig::default();
    Target::initialize_native(&config).expect("init native");

    let triple = TargetMachine::get_default_triple();
    println!("Triple: {}", triple.as_str().to_str().unwrap());

    let target = Target::from_triple(&triple).expect("from_triple");
    let tm = target
        .create_target_machine(
            &triple,
            "generic",
            "",
            OptimizationLevel::Default,
            RelocMode::Default,
            CodeModel::Default,
        )
        .expect("target machine");

    println!("Target machine created successfully");

    let context = Context::create();
    let module = context.create_module("test");

    let temp_dir = TempDir::new().expect("failed to create temp dir");
    let obj_path = temp_dir.path().join("test_tm.o");

    tm.write_to_file(&module, FileType::Object, std::path::Path::new(&obj_path))
        .expect("write_to_file");

    println!("Object file written to: {}", obj_path.display());
}
