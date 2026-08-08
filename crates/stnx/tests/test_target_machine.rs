use inkwell::targets::{Target, InitializationConfig, TargetMachine, RelocMode, CodeModel, FileType};
use inkwell::context::Context;
use inkwell::OptimizationLevel;

#[test]
fn test_target_machine_creation() {
    let config = InitializationConfig::default();
    Target::initialize_native(&config).expect("init native");
    
    let triple = TargetMachine::get_default_triple();
    println!("Triple: {}", triple.as_str().to_str().unwrap());
    
    let target = Target::from_triple(&triple).expect("from_triple");
    let tm = target.create_target_machine(
        &triple,
        "generic",
        "",
        OptimizationLevel::Default,
        RelocMode::Default,
        CodeModel::Default,
    ).expect("target machine");
    
    println!("Target machine created successfully");
    
    let context = Context::create();
    let module = context.create_module("test");
    
    tm.write_to_file(&module, FileType::Object, std::path::Path::new("/tmp/test_tm.o"))
        .expect("write_to_file");
    
    println!("Object file written successfully");
}
