/// AST types for Mini-Pascal.
///
/// Every node carries a `Span` for error reporting and DWARF debug info generation.
/// Source location (1-based line and column), plus the file it is in:
/// 0 for the program being compiled, otherwise an id from
/// [`crate::source_map`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Span {
    pub line: u32,
    pub column: u32,
    pub file: u32,
}

impl Span {
    pub fn new(line: u32, column: u32) -> Self {
        Self {
            line,
            column,
            file: crate::source_map::MAIN_FILE,
        }
    }

    pub fn in_file(line: u32, column: u32, file: u32) -> Self {
        Self { line, column, file }
    }

    /// `line L:C`, plus the file name when it isn't the main program.
    pub fn describe(&self) -> String {
        crate::source_map::describe(self.line, self.column, self.file)
    }
}

/// Top-level program: `program Name; [uses ...;] [const ...] [type ...] [var ...] {proc|func} begin ... end.`
#[derive(Debug, Clone)]
pub struct Program {
    pub name: String,
    pub uses: Vec<String>,
    /// Every unit loaded for this program (lower case), direct or not,
    /// filled in when `uses` is resolved. `Unit.Name` qualifiers naming
    /// these are dropped by the semantic pass.
    pub units: Vec<String>,
    /// Overloaded routine names: the base name -> one symbol per distinct
    /// signature (`name`, `name$2`, ...). Filled by the semantic pass.
    pub overloads: std::collections::HashMap<String, Vec<String>>,
    pub labels: Vec<i64>,
    pub consts: Vec<ConstDecl>,
    pub type_decls: Vec<TypeDecl>,
    pub vars: Vec<VarDecl>,
    pub procedures: Vec<ProcDecl>,
    pub body: Block,
    pub span: Span,
}

/// A unit module: `unit Name; interface ... implementation ... end.`
///
/// Interface decls are visible to anything that `uses` this unit;
/// implementation decls are private to the unit's own bodies. We
/// flatten both during compilation but keep the split for parsing.
#[derive(Debug, Clone)]
pub struct Unit {
    pub name: String,
    /// Every unit used, in the interface and the implementation.
    pub uses: Vec<String>,
    /// The units the interface uses, which must not lead back here; the
    /// implementation's may.
    pub interface_uses: Vec<String>,
    pub interface_consts: Vec<ConstDecl>,
    pub interface_types: Vec<TypeDecl>,
    pub interface_vars: Vec<VarDecl>,
    pub interface_proc_headers: Vec<ProcDecl>,
    pub impl_consts: Vec<ConstDecl>,
    pub impl_types: Vec<TypeDecl>,
    pub impl_vars: Vec<VarDecl>,
    pub procedures: Vec<ProcDecl>,
    pub init: Option<Block>,
    pub span: Span,
}

/// Type alias declaration: `type Name = T;`
#[derive(Debug, Clone)]
pub struct TypeDecl {
    pub name: String,
    pub ty: PascalType,
    pub span: Span,
}

/// Constant declaration: `name = value` or `name: type = value` (typed/mutable)
#[derive(Debug, Clone)]
pub struct ConstDecl {
    pub name: String,
    pub ty: Option<PascalType>,
    pub value: Expr,
    pub span: Span,
}

/// Parameter passing mode
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParamMode {
    Value,
    Var, // pass by reference
}

/// A single parameter group: `a, b: integer` or `var x: integer`
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParamGroup {
    pub mode: ParamMode,
    pub names: Vec<String>,
    pub ty: PascalType,
}

/// Procedure or function declaration
#[derive(Debug, Clone)]
pub struct ProcDecl {
    pub name: String,
    pub params: Vec<ParamGroup>,
    pub return_type: Option<PascalType>, // None = procedure, Some = function
    pub kind: RoutineKind,
    /// The object type a method implementation belongs to: `TFoo` in
    /// `procedure TFoo.Bar;`.
    pub owner: Option<String>,
    /// A `forward` declaration or a unit interface header: no body here.
    pub is_forward: bool,
    /// `external ['lib'] [name 'sym']`: implemented outside Pascal, by the
    /// C function `sym` (the routine's own name if not given).
    pub external: Option<String>,
    /// `varargs`: an external C function that takes further arguments
    /// after its declared parameters.
    pub varargs: bool,
    pub consts: Vec<ConstDecl>,
    pub type_decls: Vec<TypeDecl>,
    pub vars: Vec<VarDecl>,
    pub nested_procs: Vec<ProcDecl>,
    pub body: Block,
    pub span: Span,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoutineKind {
    Procedure,
    Function,
    Constructor,
    Destructor,
}

impl ProcDecl {
    /// A routine with just its header: no declarations and an empty body.
    pub fn header(
        name: String,
        kind: RoutineKind,
        params: Vec<ParamGroup>,
        return_type: Option<PascalType>,
        span: Span,
    ) -> Self {
        Self {
            name,
            params,
            return_type,
            kind,
            owner: None,
            is_forward: false,
            external: None,
            varargs: false,
            consts: Vec::new(),
            type_decls: Vec::new(),
            vars: Vec::new(),
            nested_procs: Vec::new(),
            body: Block {
                statements: Vec::new(),
                span,
                end_span: span,
            },
            span,
        }
    }
}

/// Variable declaration: `name1, name2 : type`
#[derive(Debug, Clone)]
pub struct VarDecl {
    pub names: Vec<String>,
    pub ty: PascalType,
    /// `absolute other`: the variables share `other`'s memory.
    pub absolute: Option<String>,
    pub span: Span,
}

/// Supported types in Mini-Pascal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PascalType {
    /// The 64-bit signed integer (`integer`, `int64`, `ptrint`, ...).
    Integer,
    /// A Turbo Pascal short string: `max + 1` bytes stored in place, the
    /// first holding the length. `string[N]`, `ShortString`, and `string`
    /// itself under `{$H-}` or `{$mode objfpc}`. In expressions it is
    /// read as an ordinary (C) string.
    ShortString {
        max: u32,
    },
    /// A fixed-width integer other than the 64-bit signed one: `byte`,
    /// `word`, `longint`, `qword` and so on. Stored at its real width;
    /// widened to 64 bits as soon as it is loaded into an expression.
    SizedInt {
        bits: u32,
        signed: bool,
    },
    Real,
    String,
    Boolean,
    Char,
    /// FPC's `WideChar`: one UTF-16 code unit. An `i16` in memory and
    /// in expressions (so it can be told apart from a `Char`, an `i8`,
    /// and from integers, which are widened to 64 bits).
    WideChar,
    /// FPC's `UnicodeString`: UTF-16 code units, NUL-terminated, behind
    /// one pointer (nil is the empty string). In expressions it is the
    /// struct `{ ptr }`, so conversions to and from `String` (UTF-8) can
    /// be chosen from the value alone.
    UnicodeString,
    Pointer(Box<PascalType>),
    Array {
        lo: i64,
        hi: i64,
        elem: Box<PascalType>,
    },
    Record {
        fields: Vec<(String, PascalType)>,
        /// Optional variant part: (tag_name, tag_type, variants)
        variant: Option<Box<RecordVariant>>,
        /// For the record an `object` type lowers to, the object type's
        /// name, so its methods can be found.
        object: Option<String>,
    },
    /// An `object` type as declared. Code generation lowers it to a
    /// `Record` (with `object` set) plus a method table.
    Object(Box<ObjectType>),
    /// Enumerated type: (val1, val2, val3)
    Enum {
        /// The type name (e.g., "Color") — set during type section parsing
        name: String,
        values: Vec<String>,
    },
    /// Subrange type: lo..hi (stored as i64)
    Subrange {
        lo: i64,
        hi: i64,
    },
    /// Set of ordinal type — stored as 256-bit bitmask (4 x i64)
    Set {
        elem: Box<PascalType>,
    },
    /// File of <type> — stored as opaque pointer to bruto_file struct.
    /// `text` is `File { elem: Char }`.
    File {
        elem: Box<PascalType>,
    },
    /// Procedural type — function pointer with a known signature.
    /// `return_type = None` means procedure (void); Some => function.
    /// A `nested` one (`... is nested`) is a closure that can hold a
    /// local routine together with the variables it uses.
    Proc {
        params: Vec<PascalType>,
        return_type: Option<Box<PascalType>>,
        nested: bool,
    },
    /// Conformant array parameter (ISO 7185):
    ///   `array[lo..hi: integer] of T`
    /// `lo_name`/`hi_name` become integer locals inside the procedure body.
    ConformantArray {
        lo_name: String,
        hi_name: String,
        elem: Box<PascalType>,
    },
    /// A named type alias (resolved to canonical type during compilation)
    Named(String),
    /// What an untyped `Pointer` points to. It has no size or value of its
    /// own; it only appears as `Pointer(Box::new(Untyped))`.
    Untyped,
    /// Array whose bounds are constant expressions (`array[0..Max-1] of T`).
    /// The semantic pass folds the bounds and turns it into `Array`; code
    /// generation never sees it.
    ArrayExpr {
        lo: ConstExpr,
        hi: ConstExpr,
        elem: Box<PascalType>,
    },
    /// Subrange whose bounds are constant expressions (`1..MaxItems`),
    /// folded into `Subrange` by the semantic pass.
    SubrangeExpr {
        lo: ConstExpr,
        hi: ConstExpr,
    },
}

impl PascalType {
    /// Call `f` with the values of every enumeration written inside this
    /// type, as in `Phase: (phFocused, phPreProcess)` in a record or
    /// object, each of which names a constant.
    pub fn for_each_enum(&self, f: &mut impl FnMut(&[String])) {
        match self {
            PascalType::Enum { values, .. } => f(values),
            PascalType::Record {
                fields, variant, ..
            } => {
                for (_, t) in fields {
                    t.for_each_enum(f);
                }
                if let Some(v) = variant {
                    v.tag_type.for_each_enum(f);
                    for (_, vfields) in &v.variants {
                        for (_, t) in vfields {
                            t.for_each_enum(f);
                        }
                    }
                }
            }
            PascalType::Object(obj) => {
                for (_, t) in &obj.fields {
                    t.for_each_enum(f);
                }
            }
            PascalType::Array { elem, .. }
            | PascalType::ArrayExpr { elem, .. }
            | PascalType::Set { elem }
            | PascalType::Pointer(elem) => elem.for_each_enum(f),
            _ => {}
        }
    }
}

/// A compile-time integer expression used where the language needs a
/// constant: array and subrange bounds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConstExpr {
    Int(i64),
    Char(u8),
    /// A named constant or enumeration value.
    Name(String),
    Neg(Box<ConstExpr>),
    Binary(ConstOp, Box<ConstExpr>, Box<ConstExpr>),
    /// `ord`, `chr`, `succ`, `pred` or `abs` applied to a constant.
    Call(String, Box<ConstExpr>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConstOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    And,
    Or,
    Xor,
    Shl,
    Shr,
}

impl ConstExpr {
    /// Convert a parsed expression into a constant expression, or `None`
    /// if it uses anything a constant can't contain.
    pub fn from_expr(expr: &Expr) -> Option<ConstExpr> {
        Some(match expr {
            Expr::IntLit(n, _) => ConstExpr::Int(*n),
            Expr::CharLit(c, _) => ConstExpr::Char(*c),
            Expr::WideCharLit(c, _) => ConstExpr::Int(i64::from(*c)),
            Expr::StrLit(s, _) if s.len() == 1 => ConstExpr::Char(s.as_bytes()[0]),
            Expr::Var(name, _) => ConstExpr::Name(name.clone()),
            Expr::UnaryOp {
                op: UnaryOp::Neg,
                operand,
                ..
            } => ConstExpr::Neg(Box::new(ConstExpr::from_expr(operand)?)),
            Expr::BinOp {
                op, left, right, ..
            } => {
                let op = match op {
                    BinOp::Add => ConstOp::Add,
                    BinOp::Sub => ConstOp::Sub,
                    BinOp::Mul => ConstOp::Mul,
                    BinOp::Div => ConstOp::Div,
                    BinOp::Mod => ConstOp::Mod,
                    BinOp::And => ConstOp::And,
                    BinOp::Or => ConstOp::Or,
                    BinOp::Xor => ConstOp::Xor,
                    BinOp::Shl => ConstOp::Shl,
                    BinOp::Shr => ConstOp::Shr,
                    _ => return None,
                };
                ConstExpr::Binary(
                    op,
                    Box::new(ConstExpr::from_expr(left)?),
                    Box::new(ConstExpr::from_expr(right)?),
                )
            }
            Expr::Call { name, args, .. } if args.len() == 1 => {
                let lower = name.to_ascii_lowercase();
                if !matches!(
                    lower.as_str(),
                    "ord"
                        | "chr"
                        | "succ"
                        | "pred"
                        | "abs"
                        | "length"
                        | "sizeof"
                        | "low"
                        | "high"
                        | "lo"
                        | "hi"
                ) {
                    return None;
                }
                ConstExpr::Call(lower, Box::new(ConstExpr::from_expr(&args[0])?))
            }
            _ => return None,
        })
    }

    /// The literal value, if this is a bare integer.
    pub fn as_int(&self) -> Option<i64> {
        match self {
            ConstExpr::Int(n) => Some(*n),
            ConstExpr::Neg(inner) => inner.as_int().map(|n| -n),
            _ => None,
        }
    }
}

/// A Turbo Pascal `object` type declaration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObjectType {
    pub parent: Option<String>,
    /// The object's own fields, in declaration order.
    pub fields: Vec<(String, PascalType)>,
    pub methods: Vec<MethodHeader>,
}

/// A method declared in an object type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MethodHeader {
    pub name: String,
    pub kind: RoutineKind,
    pub params: Vec<ParamGroup>,
    pub return_type: Option<PascalType>,
    pub is_virtual: bool,
    pub is_abstract: bool,
    pub span: Span,
}

/// Variant part of a record type: `case tag: type of ...`
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordVariant {
    pub tag_name: String,
    pub tag_type: PascalType,
    /// (case labels, fields). Labels are folded to `ConstExpr::Int` by the
    /// semantic pass.
    pub variants: Vec<(Vec<ConstExpr>, Vec<(String, PascalType)>)>,
}

/// A begin/end block containing statements.
#[derive(Debug, Clone)]
pub struct Block {
    pub statements: Vec<Statement>,
    pub span: Span,
    /// Span of the closing `end` keyword (for breakpoint support).
    pub end_span: Span,
}

#[derive(Debug, Clone)]
pub enum Statement {
    Assignment {
        target: String,
        expr: Expr,
        span: Span,
    },
    If {
        condition: Expr,
        then_branch: Block,
        else_branch: Option<Block>,
        span: Span,
    },
    While {
        condition: Expr,
        body: Block,
        span: Span,
    },
    For {
        var: String,
        from: Expr,
        to: Expr,
        downto: bool,
        body: Block,
        span: Span,
    },
    /// `for var in collection do body`: the characters of a string, the
    /// elements of an array, or what an enumerator object (`T.Create(...)`
    /// with `MoveNext` and `Current`) yields.
    ForIn {
        var: String,
        collection: Expr,
        body: Block,
        span: Span,
    },
    RepeatUntil {
        body: Vec<Statement>,
        condition: Expr,
        span: Span,
    },
    WriteLn {
        args: Vec<WriteArg>,
        span: Span,
    },
    Write {
        args: Vec<WriteArg>,
        span: Span,
    },
    ReadLn {
        targets: Vec<String>,
        span: Span,
    },
    Block(Block),
    /// Pointer dereference assignment: `p^ := expr`
    DerefAssignment {
        target: String,
        expr: Expr,
        span: Span,
    },
    /// `new(p)`, or `new(p, Init(args))` to construct an object.
    New {
        target: Expr,
        constructor: Option<(String, Vec<Expr>)>,
        span: Span,
    },
    /// `dispose(p)`, or `dispose(p, Done)` to destroy an object first.
    Dispose {
        target: Expr,
        destructor: Option<(String, Vec<Expr>)>,
        span: Span,
    },
    /// A method call statement: `obj.Method(args)`, `p^.Done`.
    MethodCall {
        object: Expr,
        method: String,
        args: Vec<Expr>,
        span: Span,
    },
    /// `str(value[:width[:precision]], target)`: format a number into a
    /// string variable.
    Str {
        value: WriteArg,
        target: Expr,
        span: Span,
    },
    /// A call through a computed procedural value, as a statement:
    /// `TErrorProc(StreamError)(Self)`.
    IndirectCall {
        call: Expr,
        span: Span,
    },
    /// `inherited Method(args)` as a statement.
    Inherited {
        method: String,
        args: Vec<Expr>,
        span: Span,
    },
    /// Array index assignment: `a[i] := expr`
    IndexAssignment {
        target: String,
        index: Expr,
        expr: Expr,
        span: Span,
    },
    /// Multi-dimensional array index assignment: `a[i, j] := expr`
    MultiIndexAssignment {
        target: String,
        indices: Vec<Expr>,
        expr: Expr,
        span: Span,
    },
    /// Record field assignment: `r.field := expr`
    FieldAssignment {
        target: String,
        field: String,
        expr: Expr,
        span: Span,
    },
    /// Procedure call: `proc(args)`
    ProcCall {
        name: String,
        args: Vec<Expr>,
        span: Span,
    },
    /// Case/of statement: `case expr of ... end`
    Case {
        expr: Expr,
        branches: Vec<CaseBranch>,
        else_branch: Option<Vec<Statement>>,
        span: Span,
    },
    /// Goto statement: `goto 10`
    Goto {
        label: i64,
        span: Span,
    },
    /// Label marker: `10: statement`
    Label {
        label: i64,
        span: Span,
    },
    /// With statement: `with target do statement`, where the target is a
    /// record or object (a variable, `p^`, a field, a cast...).
    With {
        target: Expr,
        body: Block,
        span: Span,
    },
    /// Assignment to any place expression, e.g. `PRec(p)^.Id := 7`.
    PlaceAssignment {
        target: Expr,
        expr: Expr,
        span: Span,
    },
    /// `exit` or `exit(value)`: leave the current routine, optionally
    /// setting a function's result first.
    Exit {
        value: Option<Expr>,
        span: Span,
    },
    /// `break`: leave the innermost loop.
    Break {
        span: Span,
    },
    /// `continue`: start the innermost loop's next iteration.
    Continue {
        span: Span,
    },
    /// Chained LValue assignment: a[i].field := expr, p^.next := expr, etc.
    ChainedAssignment {
        target: String,
        chain: Vec<LValueAccess>,
        expr: Expr,
        span: Span,
    },
}

/// A single write/writeln argument with optional formatting: `expr[:width[:precision]]`.
#[derive(Debug, Clone)]
pub struct WriteArg {
    pub expr: Expr,
    pub width: Option<Expr>,
    pub precision: Option<Expr>,
}

/// A single step in a chained LValue access path.
#[derive(Debug, Clone)]
pub enum LValueAccess {
    Field(String),
    Index(Expr),
    Deref,
}

impl Statement {
    pub fn span(&self) -> Span {
        match self {
            Self::Assignment { span, .. }
            | Self::DerefAssignment { span, .. }
            | Self::If { span, .. }
            | Self::While { span, .. }
            | Self::For { span, .. }
            | Self::ForIn { span, .. }
            | Self::RepeatUntil { span, .. }
            | Self::WriteLn { span, .. }
            | Self::Write { span, .. }
            | Self::ReadLn { span, .. }
            | Self::New { span, .. }
            | Self::Dispose { span, .. }
            | Self::IndexAssignment { span, .. }
            | Self::MultiIndexAssignment { span, .. }
            | Self::FieldAssignment { span, .. }
            | Self::ProcCall { span, .. }
            | Self::Case { span, .. }
            | Self::Goto { span, .. }
            | Self::Label { span, .. }
            | Self::With { span, .. }
            | Self::Exit { span, .. }
            | Self::PlaceAssignment { span, .. }
            | Self::Str { span, .. }
            | Self::IndirectCall { span, .. }
            | Self::MethodCall { span, .. }
            | Self::Inherited { span, .. }
            | Self::Break { span }
            | Self::Continue { span }
            | Self::ChainedAssignment { span, .. } => *span,
            Self::Block(b) => b.span,
        }
    }
}

/// A single case branch: one or more values -> a statement list
#[derive(Debug, Clone)]
pub struct CaseBranch {
    pub values: Vec<CaseValue>,
    pub body: Vec<Statement>,
    pub span: Span,
}

/// A value or range in a case label
#[derive(Debug, Clone)]
pub enum CaseValue {
    Single(Expr),
    Range(Expr, Expr),
}

#[derive(Debug, Clone)]
pub enum Expr {
    IntLit(i64, Span),
    RealLit(f64, Span),
    CharLit(u8, Span),
    /// A character code above 255, `#$2502`: a `WideChar`.
    WideCharLit(u16, Span),
    StrLit(String, Span),
    BoolLit(bool, Span),
    Var(String, Span),
    BinOp {
        op: BinOp,
        left: Box<Expr>,
        right: Box<Expr>,
        span: Span,
    },
    UnaryOp {
        op: UnaryOp,
        operand: Box<Expr>,
        span: Span,
    },
    /// Pointer dereference: `p^`
    Deref(Box<Expr>, Span),
    /// Function call: `func(args)`
    Call {
        name: String,
        args: Vec<Expr>,
        span: Span,
    },
    /// Array indexing: `a[i]`
    Index {
        array: Box<Expr>,
        index: Box<Expr>,
        span: Span,
    },
    /// Record field access: `r.field`
    FieldAccess {
        record: Box<Expr>,
        field: String,
        span: Span,
    },
    /// Set constructor: [1, 3, 5..10]
    SetConstructor {
        elements: Vec<SetElement>,
        span: Span,
    },
    /// `nil` pointer literal
    Nil(Span),
    /// Address of a variable, element, field or routine: `@x`
    AddrOf(Box<Expr>, Span),
    /// A method call: `obj.Method(args)`, `p^.Method`.
    MethodCall {
        object: Box<Expr>,
        method: String,
        args: Vec<Expr>,
        span: Span,
    },
    /// `inherited Method(args)`: the parent type's method, called on Self.
    Inherited {
        method: String,
        args: Vec<Expr>,
        span: Span,
    },
    /// A call through a computed procedural value: `TFunc(p)(args)`.
    IndirectCall {
        callee: Box<Expr>,
        args: Vec<Expr>,
        span: Span,
    },
    /// A typed constant's record value: `(X: 1; Y: 2)`.
    RecordInit {
        fields: Vec<(String, Expr)>,
        span: Span,
    },
    /// A typed constant's array value: `(1, 2, 3)`.
    ArrayInit {
        elements: Vec<Expr>,
        span: Span,
    },
}

#[derive(Debug, Clone)]
pub enum SetElement {
    Single(Expr),
    Range(Expr, Expr),
}

impl Expr {
    pub fn span(&self) -> Span {
        match self {
            Self::IntLit(_, s)
            | Self::RealLit(_, s)
            | Self::CharLit(_, s)
            | Self::WideCharLit(_, s)
            | Self::StrLit(_, s)
            | Self::BoolLit(_, s)
            | Self::Var(_, s) => *s,
            Self::BinOp { span, .. }
            | Self::UnaryOp { span, .. }
            | Self::Deref(_, span)
            | Self::AddrOf(_, span)
            | Self::MethodCall { span, .. }
            | Self::Inherited { span, .. }
            | Self::RecordInit { span, .. }
            | Self::ArrayInit { span, .. }
            | Self::IndirectCall { span, .. }
            | Self::Call { span, .. }
            | Self::Index { span, .. }
            | Self::FieldAccess { span, .. }
            | Self::SetConstructor { span, .. } => *span,
            Self::Nil(s) => *s,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,     // integer division (div)
    RealDiv, // real division (/)
    Mod,
    Eq,
    Neq,
    Lt,
    Gt,
    Lte,
    Gte,
    And, // logical on booleans, bitwise on integers (also Or, Xor, Not)
    Or,
    Xor,
    Shl,
    Shr, // logical shift, as in FPC
    In,  // element membership: x in S
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnaryOp {
    Neg,
    Not,
}
