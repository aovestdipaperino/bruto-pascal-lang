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

/// Top-level program: `program Name; [var ...] begin ... end.`
#[derive(Debug, Clone)]
pub struct Program {
    pub name: String,
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PascalType {
    Integer,
    String,
    Boolean,
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
}

impl Statement {
    pub fn span(&self) -> Span {
        match self {
            Self::Assignment { span, .. }
            | Self::If { span, .. }
            | Self::While { span, .. }
            | Self::WriteLn { span, .. }
            | Self::Write { span, .. }
            | Self::ReadLn { span, .. } => *span,
            Self::Block(b) => b.span,
        }
    }
}

#[derive(Debug, Clone)]
pub enum Expr {
    IntLit(i64, Span),
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
}

impl Expr {
    pub fn span(&self) -> Span {
        match self {
            Self::IntLit(_, s)
            | Self::StrLit(_, s)
            | Self::BoolLit(_, s)
            | Self::Var(_, s) => *s,
            Self::BinOp { span, .. } | Self::UnaryOp { span, .. } => *span,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Eq,
    Neq,
    Lt,
    Gt,
    Lte,
    Gte,
    And,
    Or,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnaryOp {
    Neg,
    Not,
}
