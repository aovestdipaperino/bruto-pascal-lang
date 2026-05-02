# Bruto Pascal Lang

A Mini-Pascal compiler implementing most of Wirth's original Pascal (1973 Report)
plus the practical subset of ISO 7185 / Turbo Pascal extensions, suitable for
[bruto-ide](https://github.com/aovestdipaperino/bruto-ide).

Recursive-descent parser, LLVM 18 codegen via inkwell with full DWARF debug
metadata, and a Pascal syntax highlighter.

## Supported Language

See [GRAMMAR.md](GRAMMAR.md) for the complete BNF grammar.

### Types

- Scalars: `integer`, `real`, `boolean`, `char`, `string` (`string[N]` accepted but ignored)
- Pointers: `^T`, with `nil`
- Arrays: `array[lo..hi] of T`, multi-dimensional `array[1..3, 1..3] of T`
- Records, including variant parts (`case tag: T of`)
- Sets: `set of T` (256-bit bitmask)
- Files: `text` and `file of T` with full text-mode I/O
- Enumerated: `(Red, Green, Blue)`
- Subrange: `1..100`
- `packed` keyword (parsed; layout unchanged)
- Procedural: `procedure(args)` / `function(args): T`
- Conformant array parameters: `array[lo..hi: integer] of T`

### Declarations

`program`, `label`, `const`, `type`, `var`, `procedure`, `function`,
nested procedures with full access to enclosing-scope variables (capture-lifted),
`forward`. Constants may be typed and mutable: `const x: integer = 42;`.
Program parameters in the header (`program Foo(input, output);`) are accepted.

### Statements

Assignment (with chained LValues like `a[i].field := x`), `if`/`then`/`else`,
`while`/`do`, `for`/`to`/`downto`, `repeat`/`until`, `case`/`of` (with ranges
and `else`), `with`, `goto`/`label`, `begin`/`end`, procedure calls, `new`/`dispose`.

### Expressions

Standard arithmetic and comparison operators. Short-circuit is not guaranteed.
Set membership `x in s`. Set algebra `+`, `-`, `*` (union/diff/inter) and
comparisons `=`, `<>`, `<=`, `>=`. Type casts: `integer(x)`, `real(x)`,
`char(x)`, `boolean(x)`.

### Built-ins

- Arithmetic: `abs`, `sqr`, `sqrt`, `trunc`, `round`, `sin`, `cos`, `arctan`, `exp`, `ln`, `odd`
- Ordinal: `succ`, `pred`, `inc`, `dec`, `low`, `high`, `ord`, `chr`
- String: `length`, `concat`, `copy`, `pos`, `delete`, `insert`, `str`, `val`, `upcase`
- Set: `include`, `exclude`
- Memory: `new`, `dispose`
- File: `assign`, `reset`, `rewrite`, `append`, `close`, `read`, `readln`,
  `write`, `writeln`, `eof`, `eoln`, `seek`, `filepos`, `filesize`, `page`,
  `get`, `put`, `f^` (buffer variable), `ioresult`
- Array: `pack`, `unpack`
- Predefined: `input`, `output`, `maxint`, `nil`, `true`, `false`

### Compiler Directives

- `{$R+}` / `{$R-}` — bounds checking on array indexing
- `{$Q+}` / `{$Q-}` — overflow checking on integer `+`, `-`, `*`
- `{$I+}` / `{$I-}` — I/O checking (parsed; ioresult is set on errors)

### Write Format

`write(expr:width)` and `write(real:width:precision)`.

## Prerequisites

LLVM 18 must be installed:

```bash
brew install llvm@18
```

## Usage

```rust
fn main() -> turbo_vision::core::error::Result<()> {
    bruto_ide::ide::run(Box::new(bruto_pascal_lang::MiniPascal))
}
```

## Build & Test

```bash
LLVM_SYS_181_PREFIX=/opt/homebrew/opt/llvm@18 cargo test
```

Tests share `/tmp/turbo_pascal_console.txt` so run with `--test-threads=1`.

## License

MIT
