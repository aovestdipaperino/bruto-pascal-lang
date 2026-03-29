# Bruto Pascal Lang

Mini-Pascal language implementation for [bruto-ide](https://github.com/aovestdipaperino/bruto-ide).

Implements the `Language` trait with a recursive descent parser, LLVM code generation (via inkwell) with full DWARF debug metadata, and a Pascal syntax highlighter.

## Supported Language

The Mini-Pascal subset supports:

- `program` / `var` / `begin` / `end` structure
- Types: `integer`, `string`, `boolean`
- Statements: assignment (`:=`), `if`/`then`/`else`, `while`/`do`, `writeln`, `write`, `readln`
- Expressions: arithmetic (`+`, `-`, `*`, `div`, `mod`), comparisons (`=`, `<>`, `<`, `>`, `<=`, `>=`), boolean operators (`and`, `or`, `not`)
- Comments: `//`, `{ }`, `(* *)`
- Breakpoints on any statement line including `end`

## Prerequisites

LLVM 18 must be installed:

```bash
brew install llvm@18
```

## Usage

Add `bruto-pascal-lang` as a dependency alongside `bruto-ide`:

```rust
fn main() -> turbo_vision::core::error::Result<()> {
    bruto_ide::ide::run(Box::new(bruto_pascal_lang::MiniPascal))
}
```

See [bruto-pascal](https://github.com/aovestdipaperino/bruto-pascal) for the complete binary.

## Build & Test

```bash
LLVM_SYS_181_PREFIX=/opt/homebrew/opt/llvm@18 cargo test
```

## License

MIT
