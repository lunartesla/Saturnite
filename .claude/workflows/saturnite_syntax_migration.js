export const meta = {
  name: 'saturnite-syntax-migration',
  description: 'Native syntax overhaul, ecosystem import syntax, README/docs scrub, legacy quarantine, tests, and verification',
  phases: [
    { title: 'Audit' },
    { title: 'Implement' },
    { title: 'Document' },
    { title: 'Test' },
    { title: 'Verify' },
  ],
}

const LLVM = 'export LLVM_SYS_211_PREFIX="C:\\LLVM-Saturnite"; '

phase('Audit')

// Parallel: scan docs, scan code for legacy syntax, understand module system
const [docsAudit, codeAudit, moduleAudit] = await parallel([
  () => agent(
    'Read ALL files in the docs/ directory of the Saturnite project at C:\\Users\\atimo\\Saturnite. ' +
    'For each file, classify it as one of: CANONICAL, INTERNAL, HISTORICAL, LEGACY, EXTERNAL-REFERENCE. ' +
    'Use these criteria:\n' +
    '- CANONICAL: current user-facing Saturnite language documentation (syntax, quick start, features)\n' +
    '- INTERNAL: compiler architecture, data-flow, internal Rust implementations\n' +
    '- HISTORICAL: old design notes, phase reports, architecture reviews\n' +
    '- LEGACY: superseded syntax/reference material describing old language behavior\n' +
    '- EXTERNAL-REFERENCE: docs about external ecosystems (Rust crates, Python APIs)\n\n' +
    'For each file report: filename, classification, one-line summary, whether it contains legacy Saturnite syntax ' +
    '(return, println, fn main() with braces, mod keyword, i64/str as user-facing types, or claims about unimplemented features), ' +
    'legacy_patterns list, and action (migrate/quarantine/leave). Also note any files containing "rustc" or "CPython" source. ' +
    'Output as JSON: {files: [{name, classification, summary, has_legacy_syntax, legacy_patterns, action, notes}]}',
    { phase: 'Audit', schema: {
      type: 'object',
      properties: {
        files: { type: 'array', items: { type: 'object', properties: {
          name: { type: 'string' },
          classification: { type: 'string', enum: ['CANONICAL', 'INTERNAL', 'HISTORICAL', 'LEGACY', 'EXTERNAL-REFERENCE'] },
          summary: { type: 'string' },
          has_legacy_syntax: { type: 'boolean' },
          legacy_patterns: { type: 'array', items: { type: 'string' } },
          action: { type: 'string' },
          notes: { type: 'string' },
        }, required: ['name', 'classification', 'summary', 'has_legacy_syntax', 'legacy_patterns', 'action', 'notes'] } }
      }, required: ['files']
    } }
  ),
  () => agent(
    'Scan ALL source files in C:\\Users\\atimo\\Saturnite for legacy Saturnite syntax patterns that should be migrated in canonical documentation. ' +
    'Search: README.md, docs/*.md, docs/audit_notes/*.md, examples/*, all crates/stnx/tests/*.rs (only string literals and comments, not Rust source code). ' +
    'For each match report: file path, line number, pattern type, pattern text, context. ' +
    'Categorize patterns as: fn_main_brace, return_kw, println_kw, mod_kw, i64_user, str_user, brace_block, semicolon_term, rust_path, external_form. ' +
    'ALSO: report whether the use_decl parser supports "use python:numpy" / "use rust:serde". ' +
    'Does the AST ItemKind::UseDecl have an ecosystem field? ' +
    'Output as JSON: {patterns: [{file, line, pattern_type, pattern_text, context}], use_syntax_supported: boolean, ast_has_ecosystem: boolean, parser_use_decl: string}',
    { phase: 'Audit', schema: {
      type: 'object',
      properties: {
        patterns: { type: 'array', items: { type: 'object', properties: {
          file: { type: 'string' }, line: { type: 'number' }, pattern_type: { type: 'string' },
          pattern_text: { type: 'string' }, context: { type: 'string' },
        }, required: ['file', 'line', 'pattern_type', 'pattern_text', 'context'] } },
        use_syntax_supported: { type: 'boolean' },
        ast_has_ecosystem: { type: 'boolean' },
        parser_use_decl: { type: 'string' },
      }, required: ['patterns', 'use_syntax_supported', 'ast_has_ecosystem', 'parser_use_decl']
    } }
  ),
  () => agent(
    'Read C:\\Users\\atimo\\Saturnite\\crates\\stnx\\src\\module.rs. ' +
    'Understand: (1) how ModuleGraph discovers modules from mod declarations, (2) how ModuleScope stores items and imports, ' +
    '(3) how the resolver walks use declaration paths, (4) what ModulePath looks like. ' +
    'Report the module graph data structures (Module, ModuleGraph, ModuleScope, ModuleId, ModulePath), how mod/use resolution works, ' +
    'and how a use python:numpy declaration would need to integrate. ' +
    'Output as JSON: {structures: [{name, fields}], mod_discovery: string, scope_lookup: string, use_resolution: string, ecosystem_integration: string}',
    { phase: 'Audit', schema: {
      type: 'object',
      properties: {
        structures: { type: 'array', items: { type: 'object', properties: {
          name: { type: 'string' }, fields: { type: 'string' },
        }, required: ['name', 'fields'] } },
        mod_discovery: { type: 'string' },
        scope_lookup: { type: 'string' },
        use_resolution: { type: 'string' },
        ecosystem_integration: { type: 'string' },
      }, required: ['structures', 'mod_discovery', 'scope_lookup', 'use_resolution', 'ecosystem_integration']
    } }
  ),
])

const totalFiles = docsAudit.files.length
const legacyFiles = docsAudit.files.filter(f => f.classification === 'LEGACY').length
const hasLegacy = docsAudit.files.filter(f => f.has_legacy_syntax).length
log('Audit complete: ' + totalFiles + ' doc files, ' + legacyFiles + ' legacy, ' + hasLegacy + ' with legacy syntax. Use syntax supported: ' + codeAudit.use_syntax_supported + ', AST has ecosystem: ' + codeAudit.ast_has_ecosystem)

phase('Implement')

const implResult = await agent(
  'Implement the ecosystem-boundary import syntax for Saturnite at C:\\Users\\atimo\\Saturnite.\n\n' +
  'The current parser only supports "use foo::bar" (Rust-style paths). I need to add support for:\n' +
  '- use python:numpy  (Python package import)\n' +
  '- use rust:serde     (Rust crate import)\n' +
  '- use native:sqlite  (Native library import)\n' +
  '- use math           (Saturnite-native package import, plain identifier)\n\n' +
  'Implementation steps:\n' +
  '1. In ast.rs: Add "ecosystem: Option<ExternalKind>" field to ItemKind::UseDecl\n' +
  '2. In parser/mod.rs: Modify use_decl() to detect "use <ident>:<ident>" and parse it as ecosystem import. Ecosystem identifiers: python, rust, native.\n' +
  '   For "use math" (plain identifier), ecosystem should be None.\n' +
  '   For "use foo::bar" (double colon), ecosystem is None (existing path syntax).\n' +
  '3. In hir/function.rs: Add "ecosystem: Option<ExternalKind>" to HirUseDecl\n' +
  '4. In hir/lower.rs: Pass the ecosystem field through when building HirUseDecl from AST UseDecl\n' +
  '5. In resolver.rs: In resolve_one_use(), when a use declaration has an ecosystem, skip module-graph resolution and return Ok(None)\n' +
  '6. Fix the duplicate ExternalFunction match arm in lower.rs (there is a duplicate)\n' +
  '7. Make sure existing use foo::bar syntax still works\n' +
  '8. Run: cargo check --workspace\n\n' +
  'Constraints: Do NOT copy rustc or CPython source. Keep legacy syntax working. Only change what is needed for the import syntax migration.',
  { phase: 'Implement', schema: {
    type: 'object',
    properties: {
      success: { type: 'boolean' },
      files_changed: { type: 'array', items: { type: 'string' } },
      errors: { type: 'array', items: { type: 'string' } },
      warnings: { type: 'array', items: { type: 'string' } },
      test_results: { type: 'string' },
    }, required: ['success', 'files_changed', 'errors', 'warnings', 'test_results']
  } }
)

log('Implementation complete: success=' + implResult.success + ', files=' + implResult.files_changed.length)

phase('Document')

// Parallel: README scrub, syntax doc update, interoperability doc, dependencies doc, legacy quarantine, examples scrub
await parallel([
  () => agent(
    'Completely rewrite C:\\Users\\atimo\\Saturnite\\README.md using canonical Saturnite native syntax.\n\n' +
    'BASELINE: Read the current README.md first. It currently uses legacy syntax (fn main() -> i64 { ... }, return, println, mod, i64, str). ' +
    'Replace ALL with native syntax (main:, give, say, module, number, text).\n\n' +
    'CANONICAL SYNTAX (actually implemented):\n' +
    '- module name\n' +
    '- fn name(params) -> type:  (colon-indented body)\n' +
    '- main:  (entry point)\n' +
    '- say expr  (prints text or number)\n' +
    '- give expr  (returns from function)\n' +
    '- let name = expr  /  let mut name = expr\n' +
    '- if cond: / elif cond: / else:\n' +
    '- for item in iter:  / while cond:\n' +
    '- struct Name:\n' +
    '- number (i64), text (str), bool\n' +
    '- List<T> for lists, [1, 2, 3]\n' +
    '- for i in 0..10:\n' +
    '- items.length, items[0]\n' +
    '- use math, use python:numpy, use rust:serde, use native:sqlite\n' +
    '- external rust "crate" "symbol"(params) -> ret  (low-level)\n' +
    '- "Hello {name}"  (interpolation)\n' +
    '- |>  (pipeline)\n' +
    '- x -> body  (closure)\n' +
    '- f(name: value)  (named args)\n\n' +
    'Key constraints:\n' +
    '1. Every code example must use canonical native syntax (NO braced fn main, NO return, NO println as primary example)\n' +
    '2. Do NOT claim "supports all Rust crates" or "supports all Python libraries"\n' +
    '3. Hello world must use: main: + say + give\n' +
    '4. Replace the complete example with native syntax\n' +
    '5. Update features list to use native type names\n' +
    '6. README should make Saturnite understandable in under a minute\n' +
    '7. Only modify README.md, not Rust source\n'
  ),
  () => agent(
    'Create the authoritative docs/SATURNITE_SYNTAX.md for Saturnite native syntax. ' +
    'BASELINE: Read the current docs/SATURNITE_SYNTAX.md and the parser implementation. ' +
    'The document must cover ALL canonical syntax using ONLY native syntax: modules, imports (use math, use python:numpy, use rust:serde, use native:sqlite), ' +
    'functions, main:, variables, types (number, text, bool, List<T>), conditions, loops, structs, lists, pipelines, closures, ' +
    'named arguments, string interpolation, printing (say), return (give), raise. ' +
    'For each: show canonical example, internal desugaring, semantic notes. ' +
    'Include CANONICAL warning + LEGACY COMPATIBILITY section. ' +
    'Only document constructs that are actually implemented.'
  ),
  () => agent(
    'Create docs/INTEROPERABILITY.md — canonical documentation of Saturnite Rust/Python/Native interop. ' +
    'BASELINE: Read docs/INTEROPERABILITY.md (existing), docs/INTEROPERABILITY_REPORT.md, crates/stnx/src/interop.rs, ' +
    'crates/stnx/src/interop_rust.rs, crates/stnx/src/interop_python.rs, external declaration parser code. ' +
    'Document: simple user-facing import syntax (use rust:serde, use python:numpy, use native:sqlite), ' +
    'how they map to external declarations, ecosystem-boundary semantics, ABI-safe type subset, ' +
    'low-level external syntax for advanced users, DependencyKind/ExternalFunctionKind taxonomy, generic import system. ' +
    'Include CANONICAL warning. Use canonical syntax in examples. Do not claim universal compatibility.'
  ),
  () => agent(
    'Create docs/DEPENDENCIES.md — canonical dependency model documentation. ' +
    'BASELINE: Read crates/stnx/src/interop.rs, crates/stnx/src/config.rs, crates/stnx/src/module.rs. ' +
    'Document: DependencyKind taxonomy (Saturnite, Rust, Python, Native), how saturn.toml [dependencies] maps to ecosystem imports, ' +
    'how use python:numpy resolves to a Python dependency, dependency metadata (name, version, kind, target), ' +
    'how compiler tracks dependencies through pipeline (resolver to HIR to MIR to codegen), ExternalLibrary metadata in MIR. ' +
    'Include CANONICAL warning. Be honest about what is implemented vs deferred.'
  ),
  () => agent(
    'Quarantine legacy documentation in docs/ to docs/legacy/ at C:\\Users\\atimo\\Saturnite. ' +
    'Move obsolete/legacy docs: SATURNITE_SYNTAX_MIGRATION.md, SATURNITE_VS_RUST_SIDE_BY_SIDE_COMPARISON.md, ' +
    'SATURNITE_RUST_SIDE_BY_SIDE_2026.md, SATURNITE_0_3_*.md, SATURNITE_0_4_*.md, SATURNITE_1_0_*.md, ' +
    'SATURNITE_CODE_REUSE_ANALYSIS.md, SATURNITE_RUST_REUSE_PLAN.md, SATURNITE_RUST_FORENSIC_AUDIT.md, ' +
    'SATURNITE_CRATE_DEPENDENCY_AUDIT.md, SATURNITE_CODE_LEVEL_REUSE_2026.md, SATURNITE_SCALABILITY.md, ' +
    'SCALABILITY_THOUGHT_EXPERIMENTS.md, SATURNITE_INCREMENTAL_COMPILATION.md, ' +
    'SATURNITE_POST_MODULE_ARCHITECTURE_AUDIT.md, SATURNITE_LICENSE_COMPATIBILITY_2026.md, ' +
    'SATURNITE_AGENT_STRATEGY.md, SATURNITE_FINAL_VERIFICATION.md, SATURNITE_FINAL_VERIFICATION_AUDIT_2026.md, ' +
    'SATURNITE_ACTUAL_ARCHITECTURE.md, SATURNITE_ACTUAL_ARCHITECTURE_AUDIT*.md, ' +
    'PHASE9_*.md, CAMPAIGN_FINAL_REPORT.md, STAGE_B_PYTHON.md, STAGE_C_HARDENING.md, ' +
    'PERFORMANCE_AND_OPTIMIZATION_ANALYSIS.md, SECURITY_AUDIT.md, SOUNDNESS_AND_SAFETY_ANALYSIS.md, ' +
    'THIRD_PARTY_PROVENANCE.md, INTEROPERABILITY_REPORT.md.\n\n' +
    'Create docs/legacy/README.md with clear "LEGACY DOCUMENTATION — DO NOT USE AS CURRENT LANGUAGE SPECIFICATION" warning. ' +
    'Add a header comment to each legacy file. Keep docs/SATURNITE_SYNTAX.md, docs/INTEROPERABILITY.md, docs/DEPENDENCIES.md as canonical. ' +
    'Create docs/README.md as the index with clear guidance. Use git mv to preserve history. Do NOT delete unique historical info.'
  ),
  () => agent(
    'Scrub all example files in C:\\Users\\atimo\\Saturnite\\examples/ to canonical native syntax. ' +
    'BASELINE: Read examples/hello.stn, examples/list_demo.stnx, examples/smoke_test.stnx, examples/native_demo.stn, examples/interpolation_demo.stn. ' +
    '1. hello.stn — rewrite from legacy (fn main() -> i64 { }, println, return) to native (main:, say, give). ' +
    '2. list_demo.stnx — rewrite to native syntax with List literals. ' +
    '3. smoke_test.stnx — rewrite factorial, is_even, etc. to native syntax (if/else:, for, give, say). ' +
    '4. native_demo.stn and interpolation_demo.stn — already native, verify and improve if needed. ' +
    '5. Verify: cargo run --release -- check examples/native_demo.stn\n\n' +
    'Only modify example files. Use canonical native syntax only.'
  ),
])

log('Document phase complete')

phase('Test')

// Add parser, resolver, and native syntax tests in parallel
await parallel([
  () => agent(
    'Add comprehensive tests for ecosystem-boundary import syntax to C:\\Users\\atimo\\Saturnite\\crates\\stnx\\tests\\test_native_syntax.rs. ' +
    'Add tests for: use python:numpy (parses with ecosystem=Python), use rust:serde (ecosystem=Rust), ' +
    'use native:sqlite (ecosystem=Native), use math (ecosystem=None), use foo::bar still works, ' +
    'use python:numpy as np (alias). Use the existing try_parse helper. Verify the AST ItemKind::UseDecl has correct ecosystem field. ' +
    'Only add tests for syntax that is actually implemented.'
  ),
  () => agent(
    'Add resolver tests for ecosystem imports to C:\\Users\\atimo\\Saturnite\\crates\\stnx\\tests\\test_resolver_dedicated.rs. ' +
    'Tests: use python:numpy resolves successfully (deferred to link/runtime), use rust:serde resolves OK, ' +
    'use native:sqlite resolves OK, use math where math is local function resolves, mixed native+ecosystem imports work, ' +
    'ecosystem imports do not pollute module_scopes. Use existing test patterns.'
  ),
  () => agent(
    'Add parser tests for ecosystem imports to the parser inline tests in crates/stnx/src/parser/mod.rs tests module. ' +
    'Tests: use python:numpy, use rust:serde, use native:sqlite, use math, use python:numpy as np, ' +
    'use foo::bar still works (backward compat). Use the existing parse_src helper in the tests.'
  ),
])

log('Test phase complete')

phase('Verify')

const verifyResult = await agent(
  'Run the full verification suite for Saturnite at C:\\Users\\atimo\\Saturnite with LLVM_SYS_211_PREFIX=C:\\LLVM-Saturnite.\n\n' +
  'Commands:\n' +
  '1. cargo fmt --check\n' +
  '2. cargo check --workspace\n' +
  '3. cargo clippy --workspace --all-targets\n' +
  '4. cargo test --workspace\n\n' +
  'Report: fmt_ok, check_ok, clippy_ok, tests_pass count, tests_fail count, ' +
  'failures list, new_failures list (anything beyond the pre-existing test_smoke_test_mir_structure CRLF failure), summary.',
  { phase: 'Verify', schema: {
    type: 'object',
    properties: {
      fmt_ok: { type: 'boolean' },
      check_ok: { type: 'boolean' },
      clippy_ok: { type: 'boolean' },
      tests_pass: { type: 'number' },
      tests_fail: { type: 'number' },
      failures: { type: 'array', items: { type: 'string' } },
      new_failures: { type: 'array', items: { type: 'string' } },
      summary: { type: 'string' },
    }, required: ['fmt_ok', 'check_ok', 'clippy_ok', 'tests_pass', 'tests_fail', 'failures', 'new_failures', 'summary']
  } }
)

log('Verification: fmt=' + verifyResult.fmt_ok + ' check=' + verifyResult.check_ok + ' clippy=' + verifyResult.clippy_ok + ' tests=' + verifyResult.tests_pass + '/' + (verifyResult.tests_pass + verifyResult.tests_fail))
