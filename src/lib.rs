#![allow(dead_code)]

mod ast;
pub mod codegen;
pub mod parser;
mod pascal_syntax;

use std::collections::HashSet;

use bruto_lang::language::{BuildResult, Language};
use codegen::CodeGen;
use inkwell::context::Context;
use parser::Parser;
use pascal_syntax::PascalHighlighter;

pub struct MiniPascal;

impl Language for MiniPascal {
    fn name(&self) -> &str {
        "Mini-Pascal"
    }

    fn file_extension(&self) -> &str {
        "pas"
    }

    fn sample_program(&self) -> &str {
        SAMPLE_PROGRAM
    }

    fn create_highlighter(&self) -> Box<dyn turbo_vision::views::syntax::SyntaxHighlighter> {
        Box::new(PascalHighlighter::new())
    }

    fn valid_breakpoint_lines(&self, source: &str) -> HashSet<usize> {
        let mut parser = Parser::new(source);
        let Ok(program) = parser.parse_program() else {
            return HashSet::new();
        };
        let mut lines = HashSet::new();
        for proc in &program.procedures {
            collect_block_lines(&proc.body, &mut lines);
        }
        collect_block_lines(&program.body, &mut lines);
        lines
    }

    fn build(&self, source: &str) -> Result<BuildResult, String> {
        // Use the OS-appropriate temp dir so the same code path works
        // on /tmp (Unix) and %TEMP% (Windows).
        let tmp = std::env::temp_dir();
        let source_path = tmp
            .join("bruto_pascal_src.pas")
            .to_string_lossy()
            .into_owned();
        let exe_path = tmp
            .join(if cfg!(windows) {
                "bruto_pascal_out.exe"
            } else {
                "bruto_pascal_out"
            })
            .to_string_lossy()
            .into_owned();

        std::fs::write(&source_path, source).map_err(|e| format!("Failed to write source: {e}"))?;

        let mut parser = Parser::new(source);
        let program = parser
            .parse_program()
            .map_err(|e| format!("Parse error: {e}"))?;

        let context = Context::create();
        let mut codegen = CodeGen::new(&context, &source_path);
        codegen.set_directives(parser.directives);
        codegen
            .compile(&program)
            .map_err(|e| format!("Codegen error: {e}"))?;

        codegen.emit_executable(&exe_path)?;
        let _ = codegen.write_metadata(&exe_path);

        Ok(BuildResult {
            exe_path,
            source_path,
            console_capture_path: bruto_lang::target::console_capture_path(),
        })
    }
}

/// Collect lines where codegen emits debug locations (i.e. executable lines).
fn collect_block_lines(block: &ast::Block, lines: &mut HashSet<usize>) {
    for stmt in &block.statements {
        collect_stmt_lines(stmt, lines);
    }
    // `end` keyword is breakpoint-eligible (codegen emits a debug alloca there)
    lines.insert(block.end_span.line as usize);
}

fn collect_stmt_lines(stmt: &ast::Statement, lines: &mut HashSet<usize>) {
    match stmt {
        ast::Statement::Assignment { span, .. }
        | ast::Statement::DerefAssignment { span, .. }
        | ast::Statement::WriteLn { span, .. }
        | ast::Statement::Write { span, .. }
        | ast::Statement::ReadLn { span, .. }
        | ast::Statement::New { span, .. }
        | ast::Statement::Dispose { span, .. }
        | ast::Statement::IndexAssignment { span, .. }
        | ast::Statement::MultiIndexAssignment { span, .. }
        | ast::Statement::FieldAssignment { span, .. }
        | ast::Statement::ProcCall { span, .. }
        | ast::Statement::ChainedAssignment { span, .. } => {
            lines.insert(span.line as usize);
        }
        ast::Statement::If {
            condition: _,
            then_branch,
            else_branch,
            span,
        } => {
            lines.insert(span.line as usize); // the `if` condition line
            collect_block_lines(then_branch, lines);
            if let Some(eb) = else_branch {
                collect_block_lines(eb, lines);
            }
        }
        ast::Statement::While {
            condition: _,
            body,
            span,
        } => {
            lines.insert(span.line as usize);
            collect_block_lines(body, lines);
        }
        ast::Statement::For { body, span, .. } => {
            lines.insert(span.line as usize);
            collect_block_lines(body, lines);
        }
        ast::Statement::RepeatUntil { body, span, .. } => {
            lines.insert(span.line as usize);
            for stmt in body {
                collect_stmt_lines(stmt, lines);
            }
        }
        ast::Statement::Case {
            branches,
            else_branch,
            span,
            ..
        } => {
            lines.insert(span.line as usize);
            for branch in branches {
                for stmt in &branch.body {
                    collect_stmt_lines(stmt, lines);
                }
            }
            if let Some(stmts) = else_branch {
                for stmt in stmts {
                    collect_stmt_lines(stmt, lines);
                }
            }
        }
        ast::Statement::Goto { span, .. } | ast::Statement::Label { span, .. } => {
            lines.insert(span.line as usize);
        }
        ast::Statement::With { body, span, .. } => {
            lines.insert(span.line as usize);
            collect_block_lines(body, lines);
        }
        ast::Statement::Block(block) => {
            collect_block_lines(block, lines);
        }
    }
}

const SAMPLE_PROGRAM: &str = r#"program Demo;
const
  Size = 5;

type
  Vector = array[1..5] of integer;
  Point = record
    x, y: real;
  end;

var
  nums: Vector;
  pt: Point;
  total, i: integer;
  avg: real;
  msg: string;
  p: ^integer;

procedure Fill(var a: Vector; n: integer);
var
  k: integer;
begin
  for k := 1 to n do
    a[k] := k * k
end;

function Sum(var a: Vector; n: integer): integer;
var
  s, j: integer;
begin
  s := 0;
  for j := 1 to n do
    s := s + a[j];
  Sum := s
end;

begin
  { Arrays and procedures }
  Fill(nums, Size);
  total := Sum(nums, Size);
  avg := total / Size;
  writeln('Squares: 1..', Size);
  for i := 1 to Size do
    write(nums[i], ' ');
  writeln;

  { Real arithmetic }
  writeln('Sum = ', total);
  writeln('Avg = ', avg);

  { Records }
  pt.x := avg;
  pt.y := 3.14;
  writeln('Point = (', pt.x, ', ', pt.y, ')');

  { Strings }
  msg := 'Hello' + ' ' + 'Pascal!';
  writeln(msg, ' len=', length(msg));

  { Heap pointers }
  new(p);
  p^ := 42;
  writeln('Heap value: ', p^);
  dispose(p);

  { Repeat-until }
  i := 1;
  repeat
    i := i * 2
  until i > 100;
  writeln('First power of 2 > 100: ', i);

  { ord / chr }
  writeln('ord(A) = ', ord('A'));
  write('chr(90) = ');
  writeln(chr(90));

  if total = 55 then
    writeln('All correct!')
  else
    writeln('Something is wrong!')
end.
"#;

#[cfg(test)]
mod tests {
    use super::*;

    fn build_and_run_source(source: &str) -> (bool, String) {
        let lang = MiniPascal;
        let result = lang.build(source).expect("build failed");
        let _ = std::fs::remove_file(&result.console_capture_path);
        let status = std::process::Command::new(&result.exe_path)
            .stdout(std::process::Stdio::null())
            .status()
            .expect("run failed");
        let captured = std::fs::read_to_string(&result.console_capture_path).unwrap_or_default();
        let _ = std::fs::remove_file(&result.exe_path);
        let _ = std::fs::remove_dir_all(format!("{}.dSYM", result.exe_path));
        let _ = std::fs::remove_file(&result.source_path);
        let _ = std::fs::remove_file(&result.console_capture_path);
        (status.success(), captured)
    }

    #[test]
    fn build_and_run_simple() {
        let (ok, out) = build_and_run_source(
            "program Test;\nvar\n  x: integer;\nbegin\n  x := 42;\n  writeln(x)\nend.\n",
        );
        assert!(ok);
        assert_eq!(out.trim(), "42");
    }

    #[test]
    fn sample_program_runs() {
        let (ok, out) = build_and_run_source(SAMPLE_PROGRAM);
        assert!(ok, "sample program failed to run");
        assert!(out.contains("Sum = 55"), "expected sum, got: {out}");
        assert!(out.contains("Avg = "), "expected avg, got: {out}");
        assert!(out.contains("Point = ("), "expected point, got: {out}");
        assert!(out.contains("Hello Pascal!"), "expected string, got: {out}");
        assert!(out.contains("Heap value: 42"), "expected heap, got: {out}");
        assert!(
            out.contains("First power of 2 > 100: 128"),
            "expected repeat, got: {out}"
        );
        assert!(out.contains("ord(A) = 65"), "expected ord, got: {out}");
        assert!(out.contains("chr(90) = Z"), "expected chr, got: {out}");
        assert!(out.contains("All correct!"), "expected correct, got: {out}");
    }

    #[test]
    fn for_loop() {
        let (ok, out) = build_and_run_source(
            "program T;\nvar i: integer;\nbegin\n  for i := 1 to 5 do\n    write(i);\n  writeln\nend.\n",
        );
        assert!(ok);
        assert_eq!(out.trim(), "12345");
    }

    #[test]
    fn repeat_until() {
        let (ok, out) = build_and_run_source(
            "program T;\nvar i: integer;\nbegin\n  i := 0;\n  repeat\n    i := i + 1\n  until i = 3;\n  writeln(i)\nend.\n",
        );
        assert!(ok);
        assert_eq!(out.trim(), "3");
    }

    #[test]
    fn procedure_call() {
        let (ok, out) = build_and_run_source(
            "program T;\n\nprocedure Hello;\nbegin\n  writeln('Hi')\nend;\n\nbegin\n  Hello\nend.\n",
        );
        assert!(ok);
        assert_eq!(out.trim(), "Hi");
    }

    #[test]
    fn function_call() {
        let (ok, out) = build_and_run_source(
            "program T;\n\nfunction Double(x: integer): integer;\nbegin\n  Double := x * 2\nend;\n\nbegin\n  writeln(Double(21))\nend.\n",
        );
        assert!(ok);
        assert_eq!(out.trim(), "42");
    }

    #[test]
    fn var_parameter() {
        let (ok, out) = build_and_run_source(
            "program T;\nvar a: integer;\n\nprocedure Inc(var x: integer);\nbegin\n  x := x + 1\nend;\n\nbegin\n  a := 5;\n  Inc(a);\n  writeln(a)\nend.\n",
        );
        assert!(ok);
        assert_eq!(out.trim(), "6");
    }

    #[test]
    fn real_arithmetic() {
        let (ok, out) = build_and_run_source(
            "program T;\nvar r: real;\nbegin\n  r := 3.14;\n  writeln(r)\nend.\n",
        );
        assert!(ok);
        assert!(out.trim().starts_with("3.14"), "got: {out}");
    }

    #[test]
    fn heap_new_dispose() {
        let (ok, out) = build_and_run_source(
            "program T;\nvar p: ^integer;\nbegin\n  new(p);\n  p^ := 99;\n  writeln(p^);\n  dispose(p)\nend.\n",
        );
        assert!(ok);
        assert_eq!(out.trim(), "99");
    }

    #[test]
    fn const_section() {
        let (ok, out) =
            build_and_run_source("program T;\nconst\n  X = 42;\nbegin\n  writeln(X)\nend.\n");
        assert!(ok);
        assert_eq!(out.trim(), "42");
    }

    #[test]
    fn array_indexing() {
        let (ok, out) = build_and_run_source(
            "program T;\nvar a: array[1..5] of integer;\n  i: integer;\nbegin\n  for i := 1 to 5 do\n    a[i] := i * i;\n  writeln(a[3])\nend.\n",
        );
        assert!(ok);
        assert_eq!(out.trim(), "9");
    }

    #[test]
    fn record_field_access() {
        let (ok, out) = build_and_run_source(
            "program T;\ntype\n  Point = record\n    x, y: integer;\n  end;\nvar p: Point;\nbegin\n  p.x := 10;\n  p.y := 20;\n  writeln(p.x + p.y)\nend.\n",
        );
        assert!(ok);
        assert_eq!(out.trim(), "30");
    }

    #[test]
    fn string_concat() {
        let (ok, out) = build_and_run_source(
            "program T;\nvar s: string;\nbegin\n  s := 'Hello' + ' ' + 'World';\n  writeln(s)\nend.\n",
        );
        assert!(ok);
        assert_eq!(out.trim(), "Hello World");
    }

    #[test]
    fn string_length() {
        let (ok, out) =
            build_and_run_source("program T;\nbegin\n  writeln(length('Hello'))\nend.\n");
        assert!(ok);
        assert_eq!(out.trim(), "5");
    }

    #[test]
    fn type_alias() {
        let (ok, out) = build_and_run_source(
            "program T;\ntype\n  Numbers = array[0..2] of integer;\nvar a: Numbers;\nbegin\n  a[0] := 10;\n  a[1] := 20;\n  a[2] := 30;\n  writeln(a[0] + a[1] + a[2])\nend.\n",
        );
        assert!(ok);
        assert_eq!(out.trim(), "60");
    }

    #[test]
    fn enum_type() {
        let (ok, out) = build_and_run_source(
            "program T;\ntype\n  Color = (Red, Green, Blue);\nvar c: Color;\nbegin\n  c := Green;\n  writeln(c)\nend.\n",
        );
        assert!(ok);
        assert_eq!(out.trim(), "1");
    }

    #[test]
    fn subrange_type() {
        let (ok, out) = build_and_run_source(
            "program T;\ntype\n  SmallInt = 1..10;\nvar x: SmallInt;\nbegin\n  x := 5;\n  writeln(x)\nend.\n",
        );
        assert!(ok);
        assert_eq!(out.trim(), "5");
    }

    #[test]
    fn set_operations() {
        let (ok, out) = build_and_run_source(
            "program T;\nvar s: set of integer;\n  x: boolean;\nbegin\n  s := [1, 3, 5..8];\n  x := 3 in s;\n  writeln(x);\n  x := 4 in s;\n  writeln(x)\nend.\n",
        );
        assert!(ok);
        let lines: Vec<&str> = out.trim().lines().collect();
        assert_eq!(lines[0], "true");
        assert_eq!(lines[1], "false");
    }

    #[test]
    fn case_statement() {
        let (ok, out) = build_and_run_source(
            "program T;\nvar x: integer;\nbegin\n  x := 2;\n  case x of\n    1: writeln('one');\n    2, 3: writeln('two or three');\n  else\n    writeln('other')\n  end\nend.\n",
        );
        assert!(ok);
        assert_eq!(out.trim(), "two or three");
    }

    #[test]
    fn goto_label() {
        let (ok, out) = build_and_run_source(
            "program T;\nlabel 10;\nvar i: integer;\nbegin\n  i := 0;\n  10: i := i + 1;\n  if i < 5 then\n    goto 10;\n  writeln(i)\nend.\n",
        );
        assert!(ok);
        assert_eq!(out.trim(), "5");
    }

    #[test]
    fn with_statement() {
        let (ok, out) = build_and_run_source(
            "program T;\ntype\n  Point = record\n    x, y: integer;\n  end;\nvar p: Point;\nbegin\n  with p do\n  begin\n    x := 10;\n    y := 20\n  end;\n  writeln(p.x + p.y)\nend.\n",
        );
        assert!(ok);
        assert_eq!(out.trim(), "30");
    }

    #[test]
    fn math_builtins() {
        let (ok, out) = build_and_run_source(
            "program T;\nvar x: integer;\n  r: real;\nbegin\n  x := abs(-5);\n  writeln(x);\n  x := sqr(4);\n  writeln(x);\n  r := sqrt(16.0);\n  writeln(trunc(r));\n  writeln(round(3.7))\nend.\n",
        );
        assert!(ok);
        let lines: Vec<&str> = out.trim().lines().collect();
        assert_eq!(lines[0], "5");
        assert_eq!(lines[1], "16");
        assert_eq!(lines[2], "4");
        assert_eq!(lines[3], "4");
    }

    #[test]
    fn trig_builtins() {
        let (ok, out) = build_and_run_source(
            "program T;\nvar r: real;\nbegin\n  r := sin(0.0);\n  writeln(trunc(r));\n  r := cos(0.0);\n  writeln(trunc(r));\n  r := exp(0.0);\n  writeln(trunc(r));\n  r := ln(1.0);\n  writeln(trunc(r))\nend.\n",
        );
        assert!(ok);
        let lines: Vec<&str> = out.trim().lines().collect();
        assert_eq!(lines[0], "0"); // sin(0) = 0
        assert_eq!(lines[1], "1"); // cos(0) = 1
        assert_eq!(lines[2], "1"); // exp(0) = 1
        assert_eq!(lines[3], "0"); // ln(1) = 0
    }

    #[test]
    fn ord_chr() {
        let (ok, out) = build_and_run_source(
            "program T;\nbegin\n  writeln(ord('A'));\n  write(chr(66))\nend.\n",
        );
        assert!(ok);
        let lines: Vec<&str> = out.trim().lines().collect();
        assert_eq!(lines[0], "65");
        assert_eq!(lines[1], "B");
    }

    #[test]
    fn multi_dim_array() {
        let (ok, out) = build_and_run_source(
            "program T;\nvar\n  m: array[1..2, 1..3] of integer;\nbegin\n  m[1, 2] := 42;\n  writeln(m[1, 2])\nend.\n",
        );
        assert!(ok);
        assert_eq!(out.trim(), "42");
    }

    #[test]
    fn variant_record() {
        let (ok, out) = build_and_run_source(
            "program T;\ntype\n  Shape = record\n    kind: integer;\n    case tag: integer of\n      1: (radius: integer);\n      2: (width, height: integer);\n  end;\nvar s: Shape;\nbegin\n  s.kind := 1;\n  s.tag := 1;\n  s.radius := 10;\n  writeln(s.radius)\nend.\n",
        );
        assert!(ok);
        assert_eq!(out.trim(), "10");
    }

    #[test]
    fn all_new_features() {
        let source = r#"program NewFeatures;
label 99;
type
  Color = (Red, Green, Blue);
  SmallInt = 1..10;
  Point = record
    x, y: integer;
  end;

var
  c: Color;
  n: SmallInt;
  s: set of integer;
  ok: boolean;
  pt: Point;
  m: array[1..2, 1..3] of integer;
  i: integer;

begin
  { Enumerated types }
  c := Blue;
  writeln('Blue = ', c);

  { Subrange }
  n := 7;
  writeln('n = ', n);

  { Sets }
  s := [1, 3, 5..9];
  ok := 5 in s;
  writeln('5 in set: ', ok);
  ok := 4 in s;
  writeln('4 in set: ', ok);

  { Case }
  case c of
    0: writeln('Red');
    1: writeln('Green');
    2: writeln('Blue')
  end;

  { With }
  with pt do
  begin
    x := 100;
    y := 200
  end;
  writeln('pt = ', pt.x + pt.y);

  { Multi-dim arrays }
  m[1, 2] := 42;
  writeln('m[1,2] = ', m[1, 2]);

  { Goto }
  i := 0;
  99: i := i + 1;
  if i < 3 then
    goto 99;
  writeln('goto count = ', i)
end.
"#;
        let (ok, out) = build_and_run_source(source);
        assert!(ok, "new features program failed to run");
        assert!(out.contains("Blue = 2"), "enum: {out}");
        assert!(out.contains("n = 7"), "subrange: {out}");
        assert!(out.contains("5 in set: true"), "set-in: {out}");
        assert!(out.contains("4 in set: false"), "set-not-in: {out}");
        assert!(out.contains("Blue"), "case: {out}");
        assert!(out.contains("pt = 300"), "with: {out}");
        assert!(out.contains("m[1,2] = 42"), "multi-dim: {out}");
        assert!(out.contains("goto count = 3"), "goto: {out}");
    }

    #[test]
    fn ordinal_builtins() {
        let (ok, out) = build_and_run_source(
            "program T;\nvar x: integer;\n  a: array[1..5] of integer;\nbegin\n  x := 10;\n  inc(x);\n  writeln(x);\n  dec(x, 3);\n  writeln(x);\n  writeln(succ(5));\n  writeln(pred(5));\n  writeln(low(a));\n  writeln(high(a))\nend.\n",
        );
        assert!(ok);
        let lines: Vec<&str> = out.trim().lines().collect();
        assert_eq!(lines[0], "11");
        assert_eq!(lines[1], "8");
        assert_eq!(lines[2], "6");
        assert_eq!(lines[3], "4");
        assert_eq!(lines[4], "1");
        assert_eq!(lines[5], "5");
    }

    #[test]
    fn set_include_exclude() {
        let (ok, out) = build_and_run_source(
            "program T;\nvar s: set of integer;\nbegin\n  s := [1, 3];\n  include(s, 5);\n  writeln(5 in s);\n  exclude(s, 3);\n  writeln(3 in s)\nend.\n",
        );
        assert!(ok);
        let lines: Vec<&str> = out.trim().lines().collect();
        assert_eq!(lines[0], "true");
        assert_eq!(lines[1], "false");
    }

    #[test]
    fn forward_declaration() {
        let (ok, out) = build_and_run_source(
            "program T;\n\nprocedure B; forward;\n\nprocedure A;\nbegin\n  B\nend;\n\nprocedure B;\nbegin\n  writeln('B called')\nend;\n\nbegin\n  A\nend.\n",
        );
        assert!(ok);
        assert_eq!(out.trim(), "B called");
    }

    #[test]
    fn nested_lvalue() {
        let (ok, out) = build_and_run_source(
            "program T;\ntype\n  Rec = record\n    vals: array[1..3] of integer;\n  end;\nvar r: Rec;\nbegin\n  r.vals[2] := 42;\n  writeln(r.vals[2])\nend.\n",
        );
        assert!(ok);
        assert_eq!(out.trim(), "42");
    }

    #[test]
    fn string_builtins() {
        let (ok, out) = build_and_run_source(
            "program T;\nvar s, t: string;\n  n, code: integer;\nbegin\n  s := 'Hello World';\n  t := copy(s, 7, 5);\n  writeln(t);\n  n := pos('World', s);\n  writeln(n);\n  writeln(upcase('a'));\n  t := concat('abc', 'def', 'ghi');\n  writeln(t);\n  str(42, t);\n  writeln(t);\n  val('123', n, code);\n  writeln(n)\nend.\n",
        );
        assert!(ok);
        let lines: Vec<&str> = out.trim().lines().collect();
        assert_eq!(lines[0], "World");
        assert_eq!(lines[1], "7");
        assert_eq!(lines[2], "A");
        assert_eq!(lines[3], "abcdefghi");
        assert_eq!(lines[4], "42");
        assert_eq!(lines[5], "123");
    }

    #[test]
    fn odd_builtin() {
        let (ok, out) = build_and_run_source(
            "program T;\nbegin\n  writeln(odd(3));\n  writeln(odd(4))\nend.\n",
        );
        assert!(ok);
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines[0], "true");
        assert_eq!(lines[1], "false");
    }

    #[test]
    fn maxint_constant() {
        let (ok, out) = build_and_run_source("program T;\nbegin\n  writeln(maxint)\nend.\n");
        assert!(ok);
        assert_eq!(out.trim(), "9223372036854775807");
    }

    #[test]
    fn packed_keyword_noop() {
        let (ok, out) = build_and_run_source(
            "program T;\nvar a: packed array[1..3] of integer;\nbegin\n  a[1] := 7;\n  writeln(a[1])\nend.\n",
        );
        assert!(ok);
        assert_eq!(out.trim(), "7");
    }

    #[test]
    fn program_parameters_header() {
        let (ok, out) =
            build_and_run_source("program T(input, output);\nbegin\n  writeln('hi')\nend.\n");
        assert!(ok);
        assert_eq!(out.trim(), "hi");
    }

    #[test]
    fn write_to_predefined_output() {
        let (ok, out) =
            build_and_run_source("program T;\nbegin\n  writeln(output, 'via output')\nend.\n");
        assert!(ok);
        // writes to stdout — capture file misses it, but stdout test would need direct capture.
        // Just verify it built and ran successfully.
        let _ = out;
    }

    #[test]
    fn pack_unpack_basic() {
        let (ok, out) = build_and_run_source(
            "program T;\nvar src: array[1..6] of integer;\n  dst: array[1..3] of integer;\n  i: integer;\nbegin\n  for i := 1 to 6 do src[i] := i * 10;\n  pack(src, 2, dst);\n  writeln(dst[1]);\n  writeln(dst[2]);\n  writeln(dst[3])\nend.\n",
        );
        assert!(ok);
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines[0], "20");
        assert_eq!(lines[1], "30");
        assert_eq!(lines[2], "40");
    }

    #[test]
    fn procedural_parameter() {
        let (ok, out) = build_and_run_source(
            r#"program T;

function Square(x: integer): integer;
begin
  Square := x * x
end;

function ApplyTwice(function f(n: integer): integer; v: integer): integer;
begin
  ApplyTwice := f(f(v))
end;

begin
  writeln(ApplyTwice(Square, 3))
end.
"#,
        );
        assert!(ok);
        assert_eq!(out.trim(), "81"); // (3*3)*(3*3) = 9*9 = 81
    }

    #[test]
    fn conformant_array_param() {
        let (ok, out) = build_and_run_source(
            r#"program T;

var a: array[1..5] of integer;
    i, total: integer;

function SumArr(var arr: array[lo..hi: integer] of integer): integer;
var k, s: integer;
begin
  s := 0;
  for k := lo to hi do
    s := s + arr[k];
  SumArr := s
end;

begin
  for i := 1 to 5 do a[i] := i;
  total := SumArr(a);
  writeln(total)
end.
"#,
        );
        assert!(ok);
        assert_eq!(out.trim(), "15");
    }

    #[test]
    fn fixed_length_string() {
        let (ok, out) = build_and_run_source(
            "program T;\nvar s: string[20];\nbegin\n  s := 'hi';\n  writeln(s)\nend.\n",
        );
        assert!(ok);
        assert_eq!(out.trim(), "hi");
    }

    #[test]
    fn ioresult_default_zero() {
        let (ok, out) = build_and_run_source(
            "program T;\nvar code: integer;\nbegin\n  code := ioresult;\n  writeln(code)\nend.\n",
        );
        assert!(ok);
        assert_eq!(out.trim(), "0");
    }

    #[test]
    fn file_filepos_filesize() {
        // Pascal accepts backslashes in single-quoted strings, but use
        // forward slashes for portability — fopen on Windows handles them.
        let path = std::env::temp_dir()
            .join("_bruto_test_pos.txt")
            .to_string_lossy()
            .replace('\\', "/");
        let _ = std::fs::remove_file(&path);
        let prog = format!(
            "program T;\nvar f: text;\n  size: integer;\nbegin\n  assign(f, '{path}');\n  rewrite(f);\n  writeln(f, 'ABC');\n  close(f);\n  assign(f, '{path}');\n  reset(f);\n  size := filesize(f);\n  writeln(size);\n  close(f)\nend.\n"
        );
        let (ok, out) = build_and_run_source(&prog);
        assert!(ok);
        // Pascal `text` files open in C text mode, so `writeln` emits the
        // platform line ending: 4 bytes on Unix ("ABC\n"), 5 on Windows
        // ("ABC\r\n").
        let expected = if cfg!(windows) { "5" } else { "4" };
        assert_eq!(out.trim(), expected);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn range_check_directive() {
        // {$R+} on; out-of-bounds index should abort the executable.
        let lang = MiniPascal;
        let result = lang.build(
            "{$R+}\nprogram T;\nvar a: array[1..3] of integer;\n  i: integer;\nbegin\n  i := 5;\n  a[i] := 0\nend.\n",
        ).expect("build failed");
        let status = std::process::Command::new(&result.exe_path)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .expect("run failed");
        assert!(!status.success(), "expected non-zero exit from range check");
        let _ = std::fs::remove_file(&result.exe_path);
        let _ = std::fs::remove_dir_all(format!("{}.dSYM", result.exe_path));
        let _ = std::fs::remove_file(&result.source_path);
    }

    #[test]
    fn overflow_check_directive() {
        let lang = MiniPascal;
        let result = lang.build(
            "{$Q+}\nprogram T;\nvar x: integer;\nbegin\n  x := 9223372036854775807;\n  x := x + 1\nend.\n",
        ).expect("build failed");
        let status = std::process::Command::new(&result.exe_path)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .expect("run failed");
        assert!(
            !status.success(),
            "expected non-zero exit from overflow check"
        );
        let _ = std::fs::remove_file(&result.exe_path);
        let _ = std::fs::remove_dir_all(format!("{}.dSYM", result.exe_path));
        let _ = std::fs::remove_file(&result.source_path);
    }

    #[test]
    fn nested_procedure() {
        let (ok, out) = build_and_run_source(
            r#"program T;
var t: integer;

procedure Outer(var total: integer);
var n: integer;

  procedure Inner;
  begin
    n := n + 10;
    total := total + n
  end;

begin
  n := 5;
  Inner;
  Inner
end;

begin
  t := 0;
  Outer(t);
  writeln(t)
end.
"#,
        );
        assert!(ok);
        // n=5 -> +10=15 (total=15) -> +10=25 (total=40)
        assert_eq!(out.trim(), "40");
    }

    #[test]
    fn typed_const() {
        let (ok, out) = build_and_run_source(
            "program T;\nconst\n  x: integer = 42;\n  pi: real = 3.14;\nbegin\n  writeln(x);\n  writeln(pi)\nend.\n",
        );
        assert!(ok);
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines[0], "42");
        assert!(lines[1].starts_with("3.14"), "got {:?}", lines[1]);
    }

    #[test]
    fn nil_constant() {
        let (ok, out) = build_and_run_source(
            "program T;\nvar p: ^integer;\nbegin\n  p := nil;\n  if p = nil then\n    writeln('null')\nend.\n",
        );
        assert!(ok);
        assert_eq!(out.trim(), "null");
    }

    #[test]
    fn type_cast_int_real() {
        let (ok, out) = build_and_run_source(
            "program T;\nvar i: integer;\n  r: real;\nbegin\n  r := 3.7;\n  i := integer(r);\n  writeln(i);\n  r := real(42);\n  writeln(trunc(r))\nend.\n",
        );
        assert!(ok);
        let lines: Vec<&str> = out.trim().lines().collect();
        assert_eq!(lines[0], "3");
        assert_eq!(lines[1], "42");
    }

    #[test]
    fn type_cast_char() {
        let (ok, out) = build_and_run_source(
            "program T;\nvar c: char;\nbegin\n  c := char(65);\n  writeln(c)\nend.\n",
        );
        assert!(ok);
        assert_eq!(out.trim(), "A");
    }

    #[test]
    fn write_format_int() {
        let (ok, out) =
            build_and_run_source("program T;\nbegin\n  writeln(42:5);\n  writeln(7:3)\nend.\n");
        assert!(ok);
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines[0], "   42");
        assert_eq!(lines[1], "  7");
    }

    #[test]
    fn write_format_real() {
        let (ok, out) = build_and_run_source(
            "program T;\nbegin\n  writeln(3.14:8:2);\n  writeln(1.5:6:1)\nend.\n",
        );
        assert!(ok);
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines[0], "    3.14");
        assert_eq!(lines[1], "   1.5");
    }

    #[test]
    fn file_io_write_read() {
        let path = std::env::temp_dir()
            .join("_bruto_test_io.txt")
            .to_string_lossy()
            .replace('\\', "/");
        let _ = std::fs::remove_file(&path);
        let prog = format!(
            "program T;\nvar f: text;\n  s: string;\nbegin\n  assign(f, '{path}');\n  rewrite(f);\n  writeln(f, 'hello world');\n  writeln(f, 42);\n  close(f);\n  assign(f, '{path}');\n  reset(f);\n  readln(f, s);\n  writeln(s);\n  close(f)\nend.\n"
        );
        let (ok, out) = build_and_run_source(&prog);
        assert!(ok);
        assert_eq!(out.trim(), "hello world");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn sample_pas_showcase() {
        let source = include_str!("../../SAMPLE.PAS");
        let (ok, out) = build_and_run_source(source);
        assert!(ok, "SAMPLE.PAS failed to run");
        assert!(
            out.contains("=== All features OK ==="),
            "missing final line: {out}"
        );
    }
}
