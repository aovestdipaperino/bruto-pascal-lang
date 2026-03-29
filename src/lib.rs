#![allow(dead_code)]

mod ast;
mod codegen;
mod parser;
mod pascal_syntax;

use bruto_ide::language::{BuildResult, Language};
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

    fn build(&self, source: &str) -> Result<BuildResult, String> {
        let source_path = "/tmp/bruto_pascal_src.pas".to_string();
        std::fs::write(&source_path, source)
            .map_err(|e| format!("Failed to write source: {e}"))?;

        let mut parser = Parser::new(source);
        let program = parser
            .parse_program()
            .map_err(|e| format!("Parse error: {e}"))?;

        let context = Context::create();
        let mut codegen = CodeGen::new(&context, &source_path);
        codegen
            .compile(&program)
            .map_err(|e| format!("Codegen error: {e}"))?;

        let exe_path = "/tmp/bruto_pascal_out".to_string();
        codegen.emit_executable(&exe_path)?;

        Ok(BuildResult {
            exe_path,
            source_path,
            console_capture_path: "/tmp/turbo_pascal_console.txt".to_string(),
        })
    }
}

const SAMPLE_PROGRAM: &str = r#"program Hello;
var
  x: integer;
  i: integer;
begin
  x := 0;
  i := 1;
  while i <= 10 do
  begin
    x := x + i;
    i := i + 1
  end;
  writeln('Sum of 1..10 = ', x);
  if x = 55 then
    writeln('Correct!')
  else
    writeln('Wrong!')
end.
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_and_run() {
        let lang = MiniPascal;
        let result = lang.build(
            "program Test;\nvar\n  x: integer;\nbegin\n  x := 42;\n  writeln(x)\nend.\n",
        ).expect("build failed");

        let _ = std::fs::remove_file("/tmp/turbo_pascal_console.txt");
        let status = std::process::Command::new(&result.exe_path)
            .stdout(std::process::Stdio::null())
            .status()
            .expect("run failed");
        assert!(status.success());

        let captured = std::fs::read_to_string(&result.console_capture_path)
            .expect("capture file missing");
        assert_eq!(captured.trim(), "42");

        let _ = std::fs::remove_file(&result.exe_path);
        let _ = std::fs::remove_dir_all(format!("{}.dSYM", result.exe_path));
        let _ = std::fs::remove_file(&result.source_path);
        let _ = std::fs::remove_file(&result.console_capture_path);
    }
}
