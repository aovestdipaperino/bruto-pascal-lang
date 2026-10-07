//! Code obfuscation passes for Mini-Pascal.
//!
//! Two passes, applied when the user enables obfuscation in the Build
//! Options dialog:
//! - [`rename_identifiers`] renames every user-defined identifier to a
//!   meaningless name so the generated IR/assembly hides the original
//!   names. It is a *global, consistent* rename: every occurrence of a
//!   declared name (the declaration and all references to it) maps to the
//!   same fresh name, so the program still compiles and runs. Identifiers
//!   that are never declared by the user — built-ins like `writeln`,
//!   `integer`, `true` — are left untouched, because only names found in
//!   the collected declaration set are ever rewritten.
//! - [`apply_bogus_control_flow`] inserts an opaque, always-true predicate
//!   in front of each unconditional branch, routing through a dead "bogus"
//!   block to obscure the CFG. The predicate loads a `volatile` global so
//!   the optimizer can't fold it away.
//!
//! ## Debug info / disassembly alignment
//!
//! The synthetic instructions BCF adds carry a line-0 debug location
//! (`.loc <file> 0`) rather than inheriting the previous statement's
//! `.loc`. LLVM's `.loc` directive sets the current source line for every
//! instruction that follows until the next `.loc`, so leaving the bogus
//! instructions unannotated would make them *inherit* the prior real
//! line and shove every following instruction's apparent source line out
//! of step in the IDE's Disassembly window. Line 0 is LLVM's "no source"
//! marker (`bruto_lang::disasm` already maps it to `None`), so the
//! synthetic ops show up unmapped and the real instructions stay aligned
//! to their Pascal lines.

use std::collections::HashMap;

use inkwell::IntPredicate;
use inkwell::context::Context;
use inkwell::debug_info::{AsDIScope, DebugInfoBuilder};
use inkwell::module::Module;
use inkwell::values::{InstructionOpcode, Operand};

use crate::ast::*;

// ───────────────────────── identifier renaming ─────────────────────────

/// Collects user declarations and hands out a fresh, meaningless name for
/// each distinct one. A name is declared once and reused for every
/// reference, so the rename stays consistent.
struct Namer {
    map: HashMap<String, String>,
    counter: usize,
}

impl Namer {
    fn new() -> Self {
        Self {
            map: HashMap::new(),
            counter: 0,
        }
    }

    /// Record `name` as user-declared, assigning it a fresh replacement the
    /// first time it is seen. Empty names (defensive) are ignored.
    fn declare(&mut self, name: &str) {
        if name.is_empty() || self.map.contains_key(name) {
            return;
        }
        // `l` + counter: never collides with a Pascal keyword, a built-in,
        // or another generated name, and is a legal LLVM symbol.
        let fresh = format!("l{}", self.counter);
        self.counter += 1;
        self.map.insert(name.to_string(), fresh);
    }

    /// The replacement for `name`, or `name` unchanged when it was never
    /// declared by the user (a built-in, or a reference with no matching
    /// declaration).
    fn lookup<'a>(&'a self, name: &'a str) -> &'a str {
        self.map.get(name).map(String::as_str).unwrap_or(name)
    }
}

/// Rename every user-defined identifier in `program` in place.
pub fn rename_identifiers(program: &mut Program) {
    let mut namer = Namer::new();
    collect_program(program, &mut namer);
    rewrite_program(program, &namer);
}

fn collect_program(program: &Program, n: &mut Namer) {
    n.declare(&program.name);
    for c in &program.consts {
        n.declare(&c.name);
        if let Some(ty) = &c.ty {
            collect_type(ty, n);
        }
    }
    for td in &program.type_decls {
        n.declare(&td.name);
        collect_type(&td.ty, n);
    }
    for v in &program.vars {
        for name in &v.names {
            n.declare(name);
        }
        collect_type(&v.ty, n);
    }
    for p in &program.procedures {
        collect_proc(p, n);
    }
}

fn collect_proc(p: &ProcDecl, n: &mut Namer) {
    n.declare(&p.name);
    for group in &p.params {
        for name in &group.names {
            n.declare(name);
        }
        collect_type(&group.ty, n);
    }
    if let Some(rt) = &p.return_type {
        collect_type(rt, n);
    }
    for v in &p.vars {
        for name in &v.names {
            n.declare(name);
        }
        collect_type(&v.ty, n);
    }
    for np in &p.nested_procs {
        collect_proc(np, n);
    }
}

/// Declare the identifiers a type introduces: enum value names, record
/// field names, and conformant-array bound names. Type *references*
/// (`Named`) are not declared here — the matching `type` declaration is.
fn collect_type(ty: &PascalType, n: &mut Namer) {
    match ty {
        PascalType::Enum { name, values } => {
            n.declare(name);
            for v in values {
                n.declare(v);
            }
        }
        PascalType::Record { fields, variant } => {
            for (fname, fty) in fields {
                n.declare(fname);
                collect_type(fty, n);
            }
            if let Some(var) = variant {
                n.declare(&var.tag_name);
                collect_type(&var.tag_type, n);
                for (_vals, vfields) in &var.variants {
                    for (fname, fty) in vfields {
                        n.declare(fname);
                        collect_type(fty, n);
                    }
                }
            }
        }
        PascalType::Array { elem, .. }
        | PascalType::Pointer(elem)
        | PascalType::Set { elem }
        | PascalType::File { elem } => collect_type(elem, n),
        PascalType::ConformantArray {
            lo_name,
            hi_name,
            elem,
        } => {
            n.declare(lo_name);
            n.declare(hi_name);
            collect_type(elem, n);
        }
        PascalType::Proc {
            params,
            return_type,
        } => {
            for p in params {
                collect_type(p, n);
            }
            if let Some(rt) = return_type {
                collect_type(rt, n);
            }
        }
        // References and primitives introduce no new names.
        PascalType::Named(_)
        | PascalType::Integer
        | PascalType::Real
        | PascalType::String
        | PascalType::Boolean
        | PascalType::Char
        | PascalType::Subrange { .. } => {}
    }
}

fn rewrite_program(program: &mut Program, n: &Namer) {
    program.name = n.lookup(&program.name).to_string();
    for c in &mut program.consts {
        c.name = n.lookup(&c.name).to_string();
        if let Some(ty) = &mut c.ty {
            rewrite_type(ty, n);
        }
        rewrite_expr(&mut c.value, n);
    }
    for td in &mut program.type_decls {
        td.name = n.lookup(&td.name).to_string();
        rewrite_type(&mut td.ty, n);
    }
    for v in &mut program.vars {
        for name in &mut v.names {
            *name = n.lookup(name).to_string();
        }
        rewrite_type(&mut v.ty, n);
    }
    for p in &mut program.procedures {
        rewrite_proc(p, n);
    }
    rewrite_block(&mut program.body, n);
}

fn rewrite_proc(p: &mut ProcDecl, n: &Namer) {
    p.name = n.lookup(&p.name).to_string();
    for group in &mut p.params {
        for name in &mut group.names {
            *name = n.lookup(name).to_string();
        }
        rewrite_type(&mut group.ty, n);
    }
    if let Some(rt) = &mut p.return_type {
        rewrite_type(rt, n);
    }
    for v in &mut p.vars {
        for name in &mut v.names {
            *name = n.lookup(name).to_string();
        }
        rewrite_type(&mut v.ty, n);
    }
    for np in &mut p.nested_procs {
        rewrite_proc(np, n);
    }
    rewrite_block(&mut p.body, n);
}

fn rewrite_type(ty: &mut PascalType, n: &Namer) {
    match ty {
        PascalType::Enum { name, values } => {
            *name = n.lookup(name).to_string();
            for v in values {
                *v = n.lookup(v).to_string();
            }
        }
        PascalType::Record { fields, variant } => {
            for (fname, fty) in fields {
                *fname = n.lookup(fname).to_string();
                rewrite_type(fty, n);
            }
            if let Some(var) = variant {
                var.tag_name = n.lookup(&var.tag_name).to_string();
                rewrite_type(&mut var.tag_type, n);
                for (_vals, vfields) in &mut var.variants {
                    for (fname, fty) in vfields {
                        *fname = n.lookup(fname).to_string();
                        rewrite_type(fty, n);
                    }
                }
            }
        }
        PascalType::Array { elem, .. }
        | PascalType::Pointer(elem)
        | PascalType::Set { elem }
        | PascalType::File { elem } => rewrite_type(elem, n),
        PascalType::ConformantArray {
            lo_name,
            hi_name,
            elem,
        } => {
            *lo_name = n.lookup(lo_name).to_string();
            *hi_name = n.lookup(hi_name).to_string();
            rewrite_type(elem, n);
        }
        PascalType::Proc {
            params,
            return_type,
        } => {
            for p in params {
                rewrite_type(p, n);
            }
            if let Some(rt) = return_type {
                rewrite_type(rt, n);
            }
        }
        PascalType::Named(name) => *name = n.lookup(name).to_string(),
        PascalType::Integer
        | PascalType::Real
        | PascalType::String
        | PascalType::Boolean
        | PascalType::Char
        | PascalType::Subrange { .. } => {}
    }
}

fn rewrite_block(block: &mut Block, n: &Namer) {
    for s in &mut block.statements {
        rewrite_stmt(s, n);
    }
}

fn rewrite_stmts(stmts: &mut [Statement], n: &Namer) {
    for s in stmts {
        rewrite_stmt(s, n);
    }
}

fn rewrite_stmt(s: &mut Statement, n: &Namer) {
    match s {
        Statement::Assignment { target, expr, .. } => {
            *target = n.lookup(target).to_string();
            rewrite_expr(expr, n);
        }
        Statement::DerefAssignment { target, expr, .. } => {
            *target = n.lookup(target).to_string();
            rewrite_expr(expr, n);
        }
        Statement::If {
            condition,
            then_branch,
            else_branch,
            ..
        } => {
            rewrite_expr(condition, n);
            rewrite_block(then_branch, n);
            if let Some(eb) = else_branch {
                rewrite_block(eb, n);
            }
        }
        Statement::While {
            condition, body, ..
        } => {
            rewrite_expr(condition, n);
            rewrite_block(body, n);
        }
        Statement::For {
            var, from, to, body, ..
        } => {
            *var = n.lookup(var).to_string();
            rewrite_expr(from, n);
            rewrite_expr(to, n);
            rewrite_block(body, n);
        }
        Statement::RepeatUntil {
            body, condition, ..
        } => {
            rewrite_stmts(body, n);
            rewrite_expr(condition, n);
        }
        Statement::WriteLn { args, .. } | Statement::Write { args, .. } => {
            for a in args {
                rewrite_expr(&mut a.expr, n);
                if let Some(w) = &mut a.width {
                    rewrite_expr(w, n);
                }
                if let Some(p) = &mut a.precision {
                    rewrite_expr(p, n);
                }
            }
        }
        Statement::ReadLn { targets, .. } => {
            for t in targets {
                *t = n.lookup(t).to_string();
            }
        }
        Statement::Block(b) => rewrite_block(b, n),
        Statement::New { target, .. } | Statement::Dispose { target, .. } => {
            *target = n.lookup(target).to_string();
        }
        Statement::IndexAssignment {
            target,
            index,
            expr,
            ..
        } => {
            *target = n.lookup(target).to_string();
            rewrite_expr(index, n);
            rewrite_expr(expr, n);
        }
        Statement::MultiIndexAssignment {
            target,
            indices,
            expr,
            ..
        } => {
            *target = n.lookup(target).to_string();
            for i in indices {
                rewrite_expr(i, n);
            }
            rewrite_expr(expr, n);
        }
        Statement::FieldAssignment {
            target,
            field,
            expr,
            ..
        } => {
            *target = n.lookup(target).to_string();
            *field = n.lookup(field).to_string();
            rewrite_expr(expr, n);
        }
        Statement::ProcCall { name, args, .. } => {
            *name = n.lookup(name).to_string();
            for a in args {
                rewrite_expr(a, n);
            }
        }
        Statement::Case {
            expr,
            branches,
            else_branch,
            ..
        } => {
            rewrite_expr(expr, n);
            for b in branches {
                for v in &mut b.values {
                    match v {
                        CaseValue::Single(e) => rewrite_expr(e, n),
                        CaseValue::Range(a, c) => {
                            rewrite_expr(a, n);
                            rewrite_expr(c, n);
                        }
                    }
                }
                rewrite_stmts(&mut b.body, n);
            }
            if let Some(eb) = else_branch {
                rewrite_stmts(eb, n);
            }
        }
        Statement::With {
            record_var, body, ..
        } => {
            *record_var = n.lookup(record_var).to_string();
            rewrite_block(body, n);
        }
        Statement::ChainedAssignment {
            target,
            chain,
            expr,
            ..
        } => {
            *target = n.lookup(target).to_string();
            for step in chain {
                match step {
                    LValueAccess::Field(f) => *f = n.lookup(f).to_string(),
                    LValueAccess::Index(e) => rewrite_expr(e, n),
                    LValueAccess::Deref => {}
                }
            }
            rewrite_expr(expr, n);
        }
        // No identifiers to rewrite.
        Statement::Goto { .. } | Statement::Label { .. } => {}
    }
}

fn rewrite_expr(e: &mut Expr, n: &Namer) {
    match e {
        Expr::Var(name, _) => *name = n.lookup(name).to_string(),
        Expr::BinOp { left, right, .. } => {
            rewrite_expr(left, n);
            rewrite_expr(right, n);
        }
        Expr::UnaryOp { operand, .. } => rewrite_expr(operand, n),
        Expr::Deref(inner, _) => rewrite_expr(inner, n),
        Expr::Call { name, args, .. } => {
            *name = n.lookup(name).to_string();
            for a in args {
                rewrite_expr(a, n);
            }
        }
        Expr::Index { array, index, .. } => {
            rewrite_expr(array, n);
            rewrite_expr(index, n);
        }
        Expr::FieldAccess { record, field, .. } => {
            rewrite_expr(record, n);
            *field = n.lookup(field).to_string();
        }
        Expr::SetConstructor { elements, .. } => {
            for el in elements {
                match el {
                    SetElement::Single(e) => rewrite_expr(e, n),
                    SetElement::Range(a, b) => {
                        rewrite_expr(a, n);
                        rewrite_expr(b, n);
                    }
                }
            }
        }
        Expr::IntLit(..)
        | Expr::RealLit(..)
        | Expr::CharLit(..)
        | Expr::StrLit(..)
        | Expr::BoolLit(..)
        | Expr::Nil(_) => {}
    }
}

// ─────────────────────── bogus control flow (BCF) ───────────────────────

/// Name of the module-level `volatile` global that feeds every opaque
/// predicate. Initialised to 1 so the predicates are always true.
const OPAQUE_GLOBAL: &str = "__bz_opaque";

/// Insert opaque, always-true predicates ahead of unconditional branches,
/// adding a dead "bogus" block to each, to obscure the control-flow graph.
///
/// The public entry point runs without touching debug info — used in tests
/// and anywhere no `DebugInfoBuilder` is at hand. Codegen calls
/// [`apply_bogus_control_flow_with_debug`] so the synthetic instructions
/// get a line-0 location (see the module docs).
pub fn apply_bogus_control_flow(module: &Module) {
    bcf(module, None);
}

/// Like [`apply_bogus_control_flow`] but annotates the synthetic
/// instructions with a line-0 debug location, keeping the IDE's
/// Disassembly window aligned to the real Pascal source lines.
pub fn apply_bogus_control_flow_with_debug<'ctx>(
    module: &Module<'ctx>,
    context: &'ctx Context,
    di_builder: &DebugInfoBuilder<'ctx>,
) {
    bcf(module, Some((context, di_builder)));
}

fn bcf<'ctx>(module: &Module<'ctx>, dbg: Option<(&'ctx Context, &DebugInfoBuilder<'ctx>)>) {
    let ctx = module.get_context();
    let i32_ty = ctx.i32_type();

    // One shared volatile global; initialised to 1 so `x != 0` is always
    // true at runtime but opaque to the optimizer.
    let opaque = module.get_global(OPAQUE_GLOBAL).unwrap_or_else(|| {
        let g = module.add_global(i32_ty, None, OPAQUE_GLOBAL);
        g.set_initializer(&i32_ty.const_int(1, false));
        g.set_linkage(inkwell::module::Linkage::Internal);
        g
    });
    let opaque_ptr = opaque.as_pointer_value();

    let builder = ctx.create_builder();

    for func in module.get_functions() {
        // Skip declarations (no body).
        if func.count_basic_blocks() == 0 {
            continue;
        }
        // A line-0 location scoped to this function, when debug info is
        // present. Functions without a subprogram (runtime helpers) get no
        // location — they carry no `.loc` either, so nothing to align.
        let loc0 = dbg.and_then(|(context, dib)| {
            func.get_subprogram().map(|sp| {
                dib.create_debug_location(context, 0, 0, sp.as_debug_info_scope(), None)
            })
        });

        for block in func.get_basic_blocks() {
            let Some(term) = block.get_terminator() else {
                continue;
            };
            // Only unconditional branches: operand 0 is the sole successor.
            if term.get_opcode() != InstructionOpcode::Br || term.get_num_operands() != 1 {
                continue;
            }
            let Some(Operand::Block(succ)) = term.get_operand(0) else {
                continue;
            };
            // A φ in the successor records its predecessors; adding the
            // bogus edge would leave it missing an incoming value and fail
            // verification. Pascal codegen is alloca-based (no φs), but
            // skip defensively if one ever appears.
            if succ
                .get_first_instruction()
                .is_some_and(|i| i.get_opcode() == InstructionOpcode::Phi)
            {
                continue;
            }

            let bogus = ctx.append_basic_block(func, "bz_bogus");

            // Replace `br succ` with `br (opaque ? succ : bogus)`.
            term.erase_from_basic_block();
            builder.position_at_end(block);
            if let Some(loc) = loc0 {
                builder.set_current_debug_location(loc);
            }
            let Ok(loaded) = builder.build_load(i32_ty, opaque_ptr, "bz_v") else {
                continue;
            };
            let loaded = loaded.into_int_value();
            if let Some(inst) = loaded.as_instruction() {
                let _ = inst.set_volatile(true);
            }
            let Ok(cond) = builder.build_int_compare(
                IntPredicate::NE,
                loaded,
                i32_ty.const_zero(),
                "bz_p",
            ) else {
                continue;
            };
            let _ = builder.build_conditional_branch(cond, succ, bogus);

            // The bogus block just rejoins `succ`; it is never taken at
            // runtime (predicate always true) but the optimizer can't prove
            // that through the volatile load.
            builder.position_at_end(bogus);
            if let Some(loc) = loc0 {
                builder.set_current_debug_location(loc);
            }
            let _ = builder.build_unconditional_branch(succ);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::Parser;
    use inkwell::context::Context;

    #[test]
    fn rename_identifiers_renames_user_names_and_keeps_builtins() {
        let source = r#"
program Calc;
type
  Color = (Red, Green, Blue);
  Pair = record
    a, b: integer;
  end;
var
  x, y: integer;
  c: Color;
procedure AddOne(var n: integer);
begin
  n := n + 1
end;
begin
  x := y;
  c := Red;
  AddOne(x);
  writeln(x)
end.
"#;
        let mut parser = Parser::new(source);
        let mut program = parser.parse_program().expect("should parse");
        rename_identifiers(&mut program);

        // Program name, type names, enum values, record fields, vars and
        // procedure names all become meaningless names.
        assert_ne!(program.name, "Calc");
        assert_ne!(program.type_decls[0].name, "Color");
        assert_ne!(program.type_decls[1].name, "Pair");
        assert_ne!(program.vars[0].names[0], "x");
        assert_ne!(program.vars[0].names[1], "y");
        assert_ne!(program.procedures[0].name, "AddOne");
        assert_ne!(program.procedures[0].params[0].names[0], "n");

        // A reference follows its declaration: the assignment target `x`
        // gets the same new name the var declaration did.
        let renamed_x = &program.vars[0].names[0];
        let Statement::Assignment { target, .. } = &program.body.statements[0] else {
            panic!("expected assignment");
        };
        assert_eq!(target, renamed_x);

        // The call to a user procedure follows the (renamed) declaration —
        // references follow decls — so it is no longer "AddOne".
        let renamed_addone = &program.procedures[0].name;
        let Statement::ProcCall { name, .. } = &program.body.statements[2] else {
            panic!("expected proc call");
        };
        assert_eq!(name, renamed_addone);
        assert_ne!(name, "AddOne");

        // Built-in identifiers are never declared by the user, so they are
        // left untouched.
        let Statement::WriteLn { .. } = &program.body.statements[3] else {
            panic!("expected writeln");
        };
    }

    #[test]
    fn rename_keeps_enum_value_references_consistent() {
        let source = r#"
program E;
type Color = (Red, Green);
var c: Color;
begin
  c := Green
end.
"#;
        let mut program = Parser::new(source).parse_program().unwrap();
        rename_identifiers(&mut program);
        // The enum value declaration and its use in `c := Green` must map
        // to the same new name, or codegen can't resolve it.
        let PascalType::Enum { values, .. } = &program.type_decls[0].ty else {
            panic!("expected enum");
        };
        let renamed_green = &values[1];
        let Statement::Assignment { expr, .. } = &program.body.statements[0] else {
            panic!("expected assignment");
        };
        let Expr::Var(name, _) = expr else {
            panic!("expected var ref");
        };
        assert_eq!(name, renamed_green);
        assert_ne!(name, "Green");
    }

    #[test]
    fn renamed_program_still_compiles_and_assembly_stays_mapped() {
        // Rename then codegen: the module must verify, and the Debug `.s`
        // listing must still map instructions back to Pascal lines (the
        // rename doesn't touch debug locations).
        use crate::codegen::CodeGen;

        let source =
            "program P;\nvar x: integer;\nbegin\n  x := 42;\n  writeln(x)\nend.\n";
        let tmp = std::env::temp_dir();
        let src_path = tmp.join("obf_rename_src.pas");
        std::fs::write(&src_path, source).unwrap();

        let mut program = Parser::new(source).parse_program().unwrap();
        rename_identifiers(&mut program);

        let context = Context::create();
        let mut cg = CodeGen::new(&context, &src_path.to_string_lossy());
        cg.compile(&program).expect("renamed program should compile");

        let out = tmp.join("obf_rename_out");
        let artifacts = cg.emit_object(&out.to_string_lossy()).unwrap();
        let asm = std::fs::read_to_string(artifacts.asm_path.as_ref().unwrap()).unwrap();
        let lines = bruto_lang::disasm::parse(&asm);
        assert!(
            lines.iter().any(|l| l.source_line.is_some()),
            "renamed Debug build still maps some instructions to source"
        );
        let _ = std::fs::remove_file(&artifacts.obj_path);
        if let Some(p) = &artifacts.asm_path {
            let _ = std::fs::remove_file(p);
        }
        let _ = std::fs::remove_file(&src_path);
    }

    #[test]
    fn fully_obfuscated_program_builds_and_runs_correctly() {
        // The real guarantee: rename + BCF together must still produce an
        // executable that computes the right answer. Sum 1..10 == 55; the
        // loop gives BCF unconditional branches to work on.
        use crate::codegen::CodeGen;

        let source = "program Sum;\nvar i, s: integer;\nbegin\n  s := 0;\n  for i := 1 to 10 do\n    s := s + i;\n  writeln(s)\nend.\n";
        let tmp = std::env::temp_dir();
        let src_path = tmp.join("obf_e2e_src.pas");
        std::fs::write(&src_path, source).unwrap();

        let mut program = Parser::new(source).parse_program().unwrap();
        rename_identifiers(&mut program);

        let context = Context::create();
        let mut cg = CodeGen::new(&context, &src_path.to_string_lossy());
        cg.compile(&program).expect("obfuscated program compiles");
        cg.apply_bogus_control_flow();
        // BCF must not have broken the module.
        assert!(cg.module_is_valid(), "BCF kept the module valid");

        let exe = tmp.join(if cfg!(windows) {
            "obf_e2e_out.exe"
        } else {
            "obf_e2e_out"
        });
        let exe = exe.to_string_lossy();
        let _ = std::fs::remove_file(&bruto_lang::target::console_capture_path());
        cg.emit_executable(&exe).unwrap();

        let output = std::process::Command::new(exe.as_ref())
            .output()
            .expect("run failed");
        assert!(output.status.success(), "obfuscated program exited non-zero");
        let stdout = String::from_utf8_lossy(&output.stdout);
        let captured =
            std::fs::read_to_string(bruto_lang::target::console_capture_path()).unwrap_or_default();
        assert!(
            stdout.contains("55") || captured.contains("55"),
            "expected 55, got stdout={stdout:?} capture={captured:?}"
        );

        let _ = std::fs::remove_file(exe.as_ref());
        let _ = std::fs::remove_file(&src_path);
    }

    #[test]
    fn apply_bogus_control_flow_adds_blocks_and_verifies() {
        let context = Context::create();
        let module = context.create_module("obf_test");
        let fn_type = context.void_type().fn_type(&[], false);
        let func = module.add_function("main", fn_type, None);
        let entry = context.append_basic_block(func, "entry");
        let body = context.append_basic_block(func, "body");
        let builder = context.create_builder();
        builder.position_at_end(entry);
        builder.build_unconditional_branch(body).unwrap();
        builder.position_at_end(body);
        builder.build_return(None).unwrap();

        let before = func.count_basic_blocks();
        apply_bogus_control_flow(&module);
        let after = func.count_basic_blocks();
        assert!(after > before, "BCF should add bogus blocks");
        assert!(module.verify().is_ok(), "BCF must keep the module valid");
    }

    #[test]
    fn bcf_leaves_blocks_without_unconditional_branches_alone() {
        // A lone ret block has no unconditional branch to hang a predicate
        // on, so BCF must not corrupt it.
        let context = Context::create();
        let module = context.create_module("obf_ret");
        let fn_type = context.void_type().fn_type(&[], false);
        let func = module.add_function("main", fn_type, None);
        let entry = context.append_basic_block(func, "entry");
        let builder = context.create_builder();
        builder.position_at_end(entry);
        builder.build_return(None).unwrap();

        apply_bogus_control_flow(&module);
        assert!(module.verify().is_ok());
    }
}
