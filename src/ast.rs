/// AST types for Mini-Pascal.
///
/// Every node carries a `Span` for error reporting and DWARF debug info generation.

/// Source location (1-based line and column).
#[derive(Debug, Clone, Copy, Default)]
pub struct Span {
    pub line: u32,
    pub column: u32,
}

impl Span {
    pub fn new(line: u32, column: u32) -> Self {
        Self { line, column }
    }
}

/// Top-level program: `program Name; [const ...] [type ...] [var ...] {proc|func} begin ... end.`
#[derive(Debug, Clone)]
pub struct Program {
    pub name: String,
    pub consts: Vec<ConstDecl>,
    pub type_decls: Vec<TypeDecl>,
    pub vars: Vec<VarDecl>,
    pub procedures: Vec<ProcDecl>,
    pub body: Block,
    pub span: Span,
}

/// Type alias declaration: `type Name = T;`
#[derive(Debug, Clone)]
pub struct TypeDecl {
    pub name: String,
    pub ty: PascalType,
    pub span: Span,
}

/// Constant declaration: `name = value`
#[derive(Debug, Clone)]
pub struct ConstDecl {
    pub name: String,
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
#[derive(Debug, Clone)]
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
    pub vars: Vec<VarDecl>,
    pub body: Block,
    pub span: Span,
}

/// Variable declaration: `name1, name2 : type`
#[derive(Debug, Clone)]
pub struct VarDecl {
    pub names: Vec<String>,
    pub ty: PascalType,
    pub span: Span,
}

/// Supported types in Mini-Pascal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PascalType {
    Integer,
    Real,
    String,
    Boolean,
    Char,
    Pointer(Box<PascalType>),
    Array {
        lo: i64,
        hi: i64,
        elem: Box<PascalType>,
    },
    Record {
        fields: Vec<(String, PascalType)>,
    },
    /// Enumerated type: (val1, val2, val3)
    Enum {
        /// The type name (e.g., "Color") — set during type section parsing
        name: String,
        values: Vec<String>,
    },
    /// Subrange type: lo..hi (stored as i64)
    Subrange { lo: i64, hi: i64 },
    /// Set of ordinal type — stored as 256-bit bitmask (4 x i64)
    Set { elem: Box<PascalType> },
    /// A named type alias (resolved to canonical type during compilation)
    Named(String),
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
    RepeatUntil {
        body: Vec<Statement>,
        condition: Expr,
        span: Span,
    },
    WriteLn {
        args: Vec<Expr>,
        span: Span,
    },
    Write {
        args: Vec<Expr>,
        span: Span,
    },
    ReadLn {
        target: String,
        span: Span,
    },
    Block(Block),
    /// Pointer dereference assignment: `p^ := expr`
    DerefAssignment {
        target: String,
        expr: Expr,
        span: Span,
    },
    New {
        target: String,
        span: Span,
    },
    Dispose {
        target: String,
        span: Span,
    },
    /// Array index assignment: `a[i] := expr`
    IndexAssignment {
        target: String,
        index: Expr,
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
}

impl Statement {
    pub fn span(&self) -> Span {
        match self {
            Self::Assignment { span, .. }
            | Self::DerefAssignment { span, .. }
            | Self::If { span, .. }
            | Self::While { span, .. }
            | Self::For { span, .. }
            | Self::RepeatUntil { span, .. }
            | Self::WriteLn { span, .. }
            | Self::Write { span, .. }
            | Self::ReadLn { span, .. }
            | Self::New { span, .. }
            | Self::Dispose { span, .. }
            | Self::IndexAssignment { span, .. }
            | Self::FieldAssignment { span, .. }
            | Self::ProcCall { span, .. } => *span,
            Self::Block(b) => b.span,
        }
    }
}

#[derive(Debug, Clone)]
pub enum Expr {
    IntLit(i64, Span),
    RealLit(f64, Span),
    CharLit(u8, Span),
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
            | Self::StrLit(_, s)
            | Self::BoolLit(_, s)
            | Self::Var(_, s) => *s,
            Self::BinOp { span, .. }
            | Self::UnaryOp { span, .. }
            | Self::Deref(_, span)
            | Self::Call { span, .. }
            | Self::Index { span, .. }
            | Self::FieldAccess { span, .. }
            | Self::SetConstructor { span, .. } => *span,
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
    And,
    Or,
    In, // element membership: x in S
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnaryOp {
    Neg,
    Not,
}
