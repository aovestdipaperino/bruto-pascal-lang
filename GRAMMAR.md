# Bruto Pascal — BNF Grammar

This is the full grammar accepted by `bruto-pascal-lang`. It implements most of
Wirth's 1973 Pascal Report, parts of ISO 7185 (conformant arrays), and a few
Turbo Pascal extensions (string ops, format specifiers, `forward`, `nil`,
type casts, directives).

Notation:

```
A ::= B          A is defined as B
A | B            either A or B
[ A ]            optional
{ A }            zero or more
( A )            grouping
"keyword"        terminal token
IDENT            identifier
INT_LIT          integer literal (decimal or `$hex`)
REAL_LIT         real literal
CHAR_LIT         `#nnn`
STR_LIT          'text'
```

Comments (`//` line, `{ ... }` block, `(* ... *)` block) are ignored at any
point. Block comments starting with `{$` or `(*$` are compiler directives
(`{$R+}`, `{$R-}`, `{$Q+}`, `{$Q-}`, `{$I+}`, `{$I-}`).

## Program

```
program          ::= "program" IDENT [ "(" IDENT { "," IDENT } ")" ] ";"
                     [ label-section ]
                     [ const-section ]
                     [ type-section ]
                     [ var-section ]
                     { proc-decl }
                     block
                     "."

label-section    ::= "label" INT_LIT { "," INT_LIT } ";"

const-section    ::= "const" const-decl { const-decl }
const-decl       ::= IDENT [ ":" type ] "=" expr ";"

type-section     ::= "type" type-decl { type-decl }
type-decl        ::= IDENT "=" type ";"

var-section      ::= "var" var-decl ";" { var-decl ";" }
var-decl         ::= IDENT { "," IDENT } ":" type
```

## Procedures and Functions

```
proc-decl        ::= ( "procedure" | "function" ) IDENT
                     [ param-list ]
                     [ ":" type ]                  -- functions only
                     ";"
                     ( "forward" ";"
                     | [ var-section ]
                       { proc-decl }               -- nested procs
                       block ";"
                     )

param-list       ::= "(" param-group { ";" param-group } ")"

param-group      ::= ( "var" )? IDENT { "," IDENT } ":" type
                   | "procedure" IDENT [ proc-param-list ]
                   | "function"  IDENT [ proc-param-list ] ":" type

proc-param-list  ::= "(" proc-param { ";" proc-param } ")"
proc-param       ::= ( "var" )? IDENT { "," IDENT } ":" type
```

## Types

```
type             ::= [ "packed" ]
                     ( simple-type
                     | structured-type
                     | "^" type
                     | enum-type
                     | subrange-type
                     | "string" [ "[" INT_LIT "]" ]
                     | IDENT                          -- alias
                     )

simple-type      ::= "integer" | "real" | "boolean" | "char"
                   | "text" | "string"

structured-type  ::= array-type
                   | record-type
                   | set-type
                   | file-type
                   | conformant-array

array-type       ::= "array" "[" range { "," range } "]" "of" type
range            ::= INT_LIT ".." INT_LIT
record-type      ::= "record" field-list "end"
field-list       ::= [ field-decl { ";" field-decl } ]
                     [ ( ";" )? variant-part ]
field-decl       ::= IDENT { "," IDENT } ":" type
variant-part     ::= "case" IDENT ":" type "of" variant { ";" variant }
variant          ::= INT_LIT { "," INT_LIT } ":"
                     "(" field-list ")"
set-type         ::= "set" "of" type
file-type        ::= "file" "of" type | "text"

enum-type        ::= "(" IDENT { "," IDENT } ")"
subrange-type    ::= INT_LIT ".." INT_LIT

conformant-array ::= "array" "[" IDENT ".." IDENT ":" type "]" "of" type
```

## Block

```
block            ::= "begin" [ stmt { ";" stmt } ] "end"
```

## Statements

```
stmt             ::= [ INT_LIT ":" ]            -- label prefix
                     ( assignment
                     | proc-call
                     | block
                     | if-stmt
                     | while-stmt
                     | for-stmt
                     | repeat-stmt
                     | case-stmt
                     | with-stmt
                     | goto-stmt
                     | new-stmt
                     | dispose-stmt
                     | write-stmt
                     | readln-stmt
                     )

assignment       ::= lvalue ":=" expr
lvalue           ::= IDENT { "." IDENT | "[" expr { "," expr } "]" | "^" }

proc-call        ::= IDENT [ "(" expr { "," expr } ")" ]

if-stmt          ::= "if" expr "then" stmt [ "else" stmt ]
while-stmt       ::= "while" expr "do" stmt
for-stmt         ::= "for" IDENT ":=" expr ( "to" | "downto" ) expr "do" stmt
repeat-stmt      ::= "repeat" stmt { ";" stmt } "until" expr
case-stmt        ::= "case" expr "of"
                     case-branch { ";" case-branch }
                     [ "else" stmt { ";" stmt } ]
                     "end"
case-branch      ::= case-value { "," case-value } ":" stmt
case-value       ::= expr [ ".." expr ]
with-stmt        ::= "with" IDENT "do" stmt
goto-stmt        ::= "goto" INT_LIT
new-stmt         ::= "new"     "(" IDENT ")"
dispose-stmt     ::= "dispose" "(" IDENT ")"

write-stmt       ::= ( "write" | "writeln" ) [ "(" write-arg { "," write-arg } ")" ]
write-arg        ::= expr [ ":" expr [ ":" expr ] ]   -- width [ : precision ]
readln-stmt      ::= "readln" [ "(" IDENT { "," IDENT } ")" ]
```

## Expressions

```
expr             ::= comparison
comparison       ::= additive [ rel-op additive ]
additive         ::= multiplicative { add-op multiplicative }
multiplicative   ::= unary { mul-op unary }
unary            ::= [ "-" | "not" ] primary

rel-op           ::= "=" | "<>" | "<" | ">" | "<=" | ">=" | "in"
add-op           ::= "+" | "-" | "or"
mul-op           ::= "*" | "/" | "div" | "mod" | "and"

primary          ::= INT_LIT | REAL_LIT | CHAR_LIT | STR_LIT
                   | "true" | "false" | "nil"
                   | type-cast
                   | IDENT [ "(" expr-list ")" ]      -- var or call
                     { "[" expr { "," expr } "]"
                     | "." IDENT
                     | "^"
                     }
                   | "(" expr ")"
                   | "[" [ set-elem { "," set-elem } ] "]"

type-cast        ::= ( "integer" | "real" | "char" | "boolean" )
                     "(" expr ")"

set-elem         ::= expr [ ".." expr ]

expr-list        ::= [ expr { "," expr } ]
```

## Built-in Procedures and Functions

These are recognised by name (case-sensitive) when they appear in a call.

| Category   | Name                                                              |
|------------|-------------------------------------------------------------------|
| Arithmetic | `abs`, `sqr`, `sqrt`, `trunc`, `round`, `sin`, `cos`, `arctan`, `exp`, `ln`, `odd` |
| Ordinal    | `succ`, `pred`, `inc`, `dec`, `low`, `high`, `ord`, `chr`         |
| String     | `length`, `concat`, `copy`, `pos`, `delete`, `insert`, `str`, `val`, `upcase` |
| Set        | `include`, `exclude`                                              |
| Memory     | `new`, `dispose`                                                  |
| Files      | `assign`, `reset`, `rewrite`, `append`, `close`, `read`, `readln`, `write`, `writeln`, `eof`, `eoln`, `seek`, `filepos`, `filesize`, `page`, `get`, `put`, `ioresult` |
| Array      | `pack`, `unpack`                                                  |

## Predefined Identifiers

`true`, `false`, `nil`, `maxint`, `input`, `output`.

## Compiler Directives

Inside any `{ ... }` or `(* ... *)` block whose body starts with `$`:

| Directive  | Effect                                              |
|------------|-----------------------------------------------------|
| `{$R+}`    | Enable runtime array-bounds checking                |
| `{$R-}`    | Disable bounds checking (default)                   |
| `{$Q+}`    | Enable overflow checking on `+`, `-`, `*`           |
| `{$Q-}`    | Disable overflow checking (default)                 |
| `{$I+}`    | I/O checking enabled (set `ioresult` on error)      |
| `{$I-}`    | I/O checking off                                    |

Multiple flags may appear in one comment, comma-separated: `{$R+,Q+}`.

## Differences from Wirth's Original Pascal

Implemented from Wirth (1973): all built-in types, control flow, sets,
records with variants, files, pointers, enums, subranges, nested procedures
with enclosing-scope access, `forward`, `pack`/`unpack`, `get`/`put`, `f^`
buffer variable, predefined `input`/`output`/`maxint`, conformant array
parameters (ISO 7185), procedural/functional parameters.

Not yet implemented (P2): units (`unit`/`interface`/`implementation`/`uses`),
object types (`object`/`virtual`/VMT), exceptions (`try`/`except`/`finally`/
`raise`), full dynamic strings (currently leaks heap concatenations).

Extensions beyond Wirth (Turbo / Borland heritage):

- Native `string` type with O(1) length
- `inc`, `dec`, `length`, `concat`, `copy`, `pos`, `delete`, `insert`, `str`,
  `val`, `upcase`, `forward`, `append`
- Type casts: `integer(x)`, `char(x)`, `real(x)`, `boolean(x)`
- Hex literals `$FF`, char literals `#65`
- `{$R+}`, `{$Q+}`, `{$I+}` directives
- `writeln(x:N)`, `writeln(r:N:M)` format specifiers
