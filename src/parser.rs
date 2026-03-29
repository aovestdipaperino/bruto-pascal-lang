/// Recursive descent parser for Mini-Pascal.
///
/// Grammar:
///   program      = "program" IDENT ";" [var_section] block "."
///   var_section  = "var" (var_decl ";")+
///   var_decl     = IDENT ("," IDENT)* ":" type
///   type         = "integer" | "string" | "boolean"
///   block        = "begin" statement (";" statement)* "end"
///   statement    = assignment | if_stmt | while_stmt | write_stmt | readln_stmt | block
///   assignment   = IDENT ":=" expr
///   if_stmt      = "if" expr "then" statement ["else" statement]
///   while_stmt   = "while" expr "do" statement
///   write_stmt   = ("write"|"writeln") "(" [expr ("," expr)*] ")"
///   readln_stmt  = "readln" "(" IDENT ")"
///   expr         = comparison
///   comparison   = additive (("=" | "<>" | "<" | ">" | "<=" | ">=") additive)?
///   additive     = multiplicative (("+" | "-" | "or") multiplicative)*
///   multiplicative = unary (("*" | "div" | "mod" | "and") unary)*
///   unary        = ["-" | "not"] primary
///   primary      = INT_LIT | STR_LIT | "true" | "false" | IDENT | "(" expr ")"

use crate::ast::*;
use std::fmt;

// ── Token types ──────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    // Keywords
    Program, Var, Begin, End, If, Then, Else, While, Do,
    KwWrite, KwWriteLn, KwReadLn,
    KwDiv, KwMod, KwAnd, KwOr, KwNot,
    KwTrue, KwFalse,
    // Type keywords
    TyInteger, TyString, TyBoolean,
    // Literals
    IntLit(i64),
    StrLit(String),
    // Identifier
    Ident(String),
    // Operators
    Plus, Minus, Star, Assign, Eq, Neq, Lt, Gt, Lte, Gte,
    // Delimiters
    LParen, RParen, Semi, Colon, Comma, Dot,
    // End of input
    Eof,
}

impl fmt::Display for Tok {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Tok::Program => write!(f, "'program'"),
            Tok::Var => write!(f, "'var'"),
            Tok::Begin => write!(f, "'begin'"),
            Tok::End => write!(f, "'end'"),
            Tok::If => write!(f, "'if'"),
            Tok::Then => write!(f, "'then'"),
            Tok::Else => write!(f, "'else'"),
            Tok::While => write!(f, "'while'"),
            Tok::Do => write!(f, "'do'"),
            Tok::KwWrite => write!(f, "'write'"),
            Tok::KwWriteLn => write!(f, "'writeln'"),
            Tok::KwReadLn => write!(f, "'readln'"),
            Tok::KwDiv => write!(f, "'div'"),
            Tok::KwMod => write!(f, "'mod'"),
            Tok::KwAnd => write!(f, "'and'"),
            Tok::KwOr => write!(f, "'or'"),
            Tok::KwNot => write!(f, "'not'"),
            Tok::KwTrue => write!(f, "'true'"),
            Tok::KwFalse => write!(f, "'false'"),
            Tok::TyInteger => write!(f, "'integer'"),
            Tok::TyString => write!(f, "'string'"),
            Tok::TyBoolean => write!(f, "'boolean'"),
            Tok::IntLit(n) => write!(f, "{n}"),
            Tok::StrLit(s) => write!(f, "'{s}'"),
            Tok::Ident(s) => write!(f, "identifier '{s}'"),
            Tok::Plus => write!(f, "'+'"),
            Tok::Minus => write!(f, "'-'"),
            Tok::Star => write!(f, "'*'"),
            Tok::Assign => write!(f, "':='"),
            Tok::Eq => write!(f, "'='"),
            Tok::Neq => write!(f, "'<>'"),
            Tok::Lt => write!(f, "'<'"),
            Tok::Gt => write!(f, "'>'"),
            Tok::Lte => write!(f, "'<='"),
            Tok::Gte => write!(f, "'>='"),
            Tok::LParen => write!(f, "'('"),
            Tok::RParen => write!(f, "')'"),
            Tok::Semi => write!(f, "';'"),
            Tok::Colon => write!(f, "':'"),
            Tok::Comma => write!(f, "','"),
            Tok::Dot => write!(f, "'.'"),
            Tok::Eof => write!(f, "end of file"),
        }
    }
}

// ── Parse error ──────────────────────────────────────────

#[derive(Debug)]
pub struct ParseError {
    pub message: String,
    pub span: Span,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "line {}:{}: {}", self.span.line, self.span.column, self.message)
    }
}

// ── Lexer ────────────────────────────────────────────────

struct Lexer {
    chars: Vec<char>,
    pos: usize,
    line: u32,
    col: u32,
}

impl Lexer {
    fn new(source: &str) -> Self {
        Self {
            chars: source.chars().collect(),
            pos: 0,
            line: 1,
            col: 1,
        }
    }

    fn span(&self) -> Span {
        Span::new(self.line, self.col)
    }

    fn peek(&self) -> char {
        self.chars.get(self.pos).copied().unwrap_or('\0')
    }

    fn advance(&mut self) -> char {
        let ch = self.peek();
        if ch != '\0' {
            self.pos += 1;
            if ch == '\n' {
                self.line += 1;
                self.col = 1;
            } else {
                self.col += 1;
            }
        }
        ch
    }

    fn skip_whitespace_and_comments(&mut self) {
        loop {
            // Whitespace
            while self.pos < self.chars.len() && self.peek().is_whitespace() {
                self.advance();
            }
            if self.pos >= self.chars.len() { return; }

            // Line comment: //
            if self.peek() == '/' && self.chars.get(self.pos + 1) == Some(&'/') {
                while self.pos < self.chars.len() && self.peek() != '\n' {
                    self.advance();
                }
                continue;
            }

            // Block comment: { }
            if self.peek() == '{' {
                self.advance();
                while self.pos < self.chars.len() && self.peek() != '}' {
                    self.advance();
                }
                if self.pos < self.chars.len() { self.advance(); }
                continue;
            }

            // Block comment: (* *)
            if self.peek() == '(' && self.chars.get(self.pos + 1) == Some(&'*') {
                self.advance(); self.advance();
                loop {
                    if self.pos >= self.chars.len() { break; }
                    if self.peek() == '*' && self.chars.get(self.pos + 1) == Some(&')') {
                        self.advance(); self.advance();
                        break;
                    }
                    self.advance();
                }
                continue;
            }

            break;
        }
    }

    fn next_token(&mut self) -> (Tok, Span) {
        self.skip_whitespace_and_comments();

        let span = self.span();

        if self.pos >= self.chars.len() {
            return (Tok::Eof, span);
        }

        let ch = self.peek();

        // String literal
        if ch == '\'' {
            self.advance();
            let mut s = String::new();
            loop {
                if self.pos >= self.chars.len() { break; }
                if self.peek() == '\'' {
                    self.advance();
                    if self.peek() == '\'' {
                        s.push('\'');
                        self.advance();
                    } else {
                        break;
                    }
                } else {
                    s.push(self.advance());
                }
            }
            return (Tok::StrLit(s), span);
        }

        // Number
        if ch.is_ascii_digit() {
            let mut n: i64 = 0;
            while self.pos < self.chars.len() && self.peek().is_ascii_digit() {
                n = n * 10 + (self.advance() as i64 - '0' as i64);
            }
            return (Tok::IntLit(n), span);
        }

        // Hex number: $FF
        if ch == '$' {
            self.advance();
            let mut n: i64 = 0;
            while self.pos < self.chars.len() && self.peek().is_ascii_hexdigit() {
                let d = self.advance();
                n = n * 16 + d.to_digit(16).unwrap_or(0) as i64;
            }
            return (Tok::IntLit(n), span);
        }

        // Identifier / keyword
        if ch.is_ascii_alphabetic() || ch == '_' {
            let mut word = String::new();
            while self.pos < self.chars.len()
                && (self.peek().is_ascii_alphanumeric() || self.peek() == '_')
            {
                word.push(self.advance());
            }
            let lower = word.to_lowercase();
            let tok = match lower.as_str() {
                "program"  => Tok::Program,
                "var"      => Tok::Var,
                "begin"    => Tok::Begin,
                "end"      => Tok::End,
                "if"       => Tok::If,
                "then"     => Tok::Then,
                "else"     => Tok::Else,
                "while"    => Tok::While,
                "do"       => Tok::Do,
                "write"    => Tok::KwWrite,
                "writeln"  => Tok::KwWriteLn,
                "readln"   => Tok::KwReadLn,
                "div"      => Tok::KwDiv,
                "mod"      => Tok::KwMod,
                "and"      => Tok::KwAnd,
                "or"       => Tok::KwOr,
                "not"      => Tok::KwNot,
                "true"     => Tok::KwTrue,
                "false"    => Tok::KwFalse,
                "integer"  => Tok::TyInteger,
                "string"   => Tok::TyString,
                "boolean"  => Tok::TyBoolean,
                _          => Tok::Ident(word),
            };
            return (tok, span);
        }

        // Two-character operators
        if ch == ':' && self.chars.get(self.pos + 1) == Some(&'=') {
            self.advance(); self.advance();
            return (Tok::Assign, span);
        }
        if ch == '<' && self.chars.get(self.pos + 1) == Some(&'>') {
            self.advance(); self.advance();
            return (Tok::Neq, span);
        }
        if ch == '<' && self.chars.get(self.pos + 1) == Some(&'=') {
            self.advance(); self.advance();
            return (Tok::Lte, span);
        }
        if ch == '>' && self.chars.get(self.pos + 1) == Some(&'=') {
            self.advance(); self.advance();
            return (Tok::Gte, span);
        }

        // Single-character tokens
        self.advance();
        let tok = match ch {
            '+' => Tok::Plus,
            '-' => Tok::Minus,
            '*' => Tok::Star,
            '=' => Tok::Eq,
            '<' => Tok::Lt,
            '>' => Tok::Gt,
            '(' => Tok::LParen,
            ')' => Tok::RParen,
            ';' => Tok::Semi,
            ':' => Tok::Colon,
            ',' => Tok::Comma,
            '.' => Tok::Dot,
            _ => {
                // Unknown character — return as identifier for error recovery
                return (Tok::Ident(ch.to_string()), span);
            }
        };
        (tok, span)
    }
}

// ── Parser ───────────────────────────────────────────────

pub struct Parser {
    tokens: Vec<(Tok, Span)>,
    pos: usize,
}

impl Parser {
    pub fn new(source: &str) -> Self {
        let mut lexer = Lexer::new(source);
        let mut tokens = Vec::new();
        loop {
            let (tok, span) = lexer.next_token();
            let is_eof = tok == Tok::Eof;
            tokens.push((tok, span));
            if is_eof { break; }
        }
        Self { tokens, pos: 0 }
    }

    fn peek(&self) -> &Tok {
        &self.tokens[self.pos].0
    }

    fn span(&self) -> Span {
        self.tokens[self.pos].1
    }

    fn advance(&mut self) -> (Tok, Span) {
        let t = self.tokens[self.pos].clone();
        if self.pos + 1 < self.tokens.len() {
            self.pos += 1;
        }
        t
    }

    fn expect(&mut self, expected: &Tok) -> Result<Span, ParseError> {
        if self.peek() == expected {
            let (_, span) = self.advance();
            Ok(span)
        } else {
            Err(ParseError {
                message: format!("expected {expected}, found {}", self.peek()),
                span: self.span(),
            })
        }
    }

    fn expect_ident(&mut self) -> Result<(String, Span), ParseError> {
        match self.peek().clone() {
            Tok::Ident(name) => {
                let span = self.span();
                self.advance();
                Ok((name, span))
            }
            _ => Err(ParseError {
                message: format!("expected identifier, found {}", self.peek()),
                span: self.span(),
            }),
        }
    }

    // ── program ──────────────────────────────────────────

    pub fn parse_program(&mut self) -> Result<Program, ParseError> {
        let span = self.span();
        self.expect(&Tok::Program)?;
        let (name, _) = self.expect_ident()?;
        self.expect(&Tok::Semi)?;

        let vars = if *self.peek() == Tok::Var {
            self.parse_var_section()?
        } else {
            Vec::new()
        };

        let body = self.parse_block()?;
        self.expect(&Tok::Dot)?;

        Ok(Program { name, vars, body, span })
    }

    // ── var section ──────────────────────────────────────

    fn parse_var_section(&mut self) -> Result<Vec<VarDecl>, ParseError> {
        self.expect(&Tok::Var)?;
        let mut decls = Vec::new();
        // Parse var declarations until we see 'begin' or another section keyword
        while matches!(self.peek(), Tok::Ident(_)) {
            decls.push(self.parse_var_decl()?);
            self.expect(&Tok::Semi)?;
        }
        if decls.is_empty() {
            return Err(ParseError {
                message: "expected at least one variable declaration after 'var'".into(),
                span: self.span(),
            });
        }
        Ok(decls)
    }

    fn parse_var_decl(&mut self) -> Result<VarDecl, ParseError> {
        let span = self.span();
        let mut names = Vec::new();
        let (first, _) = self.expect_ident()?;
        names.push(first);
        while *self.peek() == Tok::Comma {
            self.advance();
            let (name, _) = self.expect_ident()?;
            names.push(name);
        }
        self.expect(&Tok::Colon)?;
        let ty = self.parse_type()?;
        Ok(VarDecl { names, ty, span })
    }

    fn parse_type(&mut self) -> Result<PascalType, ParseError> {
        match self.peek() {
            Tok::TyInteger => { self.advance(); Ok(PascalType::Integer) }
            Tok::TyString  => { self.advance(); Ok(PascalType::String) }
            Tok::TyBoolean => { self.advance(); Ok(PascalType::Boolean) }
            _ => Err(ParseError {
                message: format!("expected type name, found {}", self.peek()),
                span: self.span(),
            }),
        }
    }

    // ── block ────────────────────────────────────────────

    fn parse_block(&mut self) -> Result<Block, ParseError> {
        let span = self.span();
        self.expect(&Tok::Begin)?;
        let mut statements = Vec::new();
        // Parse statements separated by semicolons, until 'end'
        if *self.peek() != Tok::End {
            statements.push(self.parse_statement()?);
            while *self.peek() == Tok::Semi {
                self.advance();
                if *self.peek() == Tok::End {
                    break;
                }
                statements.push(self.parse_statement()?);
            }
        }
        let end_span = self.span();
        self.expect(&Tok::End)?;
        Ok(Block { statements, span, end_span })
    }

    // ── statement ────────────────────────────────────────

    fn parse_statement(&mut self) -> Result<Statement, ParseError> {
        match self.peek() {
            Tok::Begin => Ok(Statement::Block(self.parse_block()?)),
            Tok::If => self.parse_if(),
            Tok::While => self.parse_while(),
            Tok::KwWrite => self.parse_write(false),
            Tok::KwWriteLn => self.parse_write(true),
            Tok::KwReadLn => self.parse_readln(),
            Tok::Ident(_) => self.parse_assignment(),
            _ => Err(ParseError {
                message: format!("expected statement, found {}", self.peek()),
                span: self.span(),
            }),
        }
    }

    fn parse_assignment(&mut self) -> Result<Statement, ParseError> {
        let span = self.span();
        let (target, _) = self.expect_ident()?;
        self.expect(&Tok::Assign)?;
        let expr = self.parse_expr()?;
        Ok(Statement::Assignment { target, expr, span })
    }

    fn parse_if(&mut self) -> Result<Statement, ParseError> {
        let span = self.span();
        self.expect(&Tok::If)?;
        let condition = self.parse_expr()?;
        self.expect(&Tok::Then)?;
        let then_stmt = self.parse_statement()?;
        let then_branch = match then_stmt {
            Statement::Block(b) => b,
            other => { let s = other.span(); Block { span: s, end_span: s, statements: vec![other] } },
        };
        let else_branch = if *self.peek() == Tok::Else {
            self.advance();
            let else_stmt = self.parse_statement()?;
            let block = match else_stmt {
                Statement::Block(b) => b,
                other => { let s = other.span(); Block { span: s, end_span: s, statements: vec![other] } },
            };
            Some(block)
        } else {
            None
        };
        Ok(Statement::If { condition, then_branch, else_branch, span })
    }

    fn parse_while(&mut self) -> Result<Statement, ParseError> {
        let span = self.span();
        self.expect(&Tok::While)?;
        let condition = self.parse_expr()?;
        self.expect(&Tok::Do)?;
        let body_stmt = self.parse_statement()?;
        let body = match body_stmt {
            Statement::Block(b) => b,
            other => { let s = other.span(); Block { span: s, end_span: s, statements: vec![other] } },
        };
        Ok(Statement::While { condition, body, span })
    }

    fn parse_write(&mut self, is_writeln: bool) -> Result<Statement, ParseError> {
        let span = self.span();
        self.advance(); // consume 'write' or 'writeln'
        self.expect(&Tok::LParen)?;
        let mut args = Vec::new();
        if *self.peek() != Tok::RParen {
            args.push(self.parse_expr()?);
            while *self.peek() == Tok::Comma {
                self.advance();
                args.push(self.parse_expr()?);
            }
        }
        self.expect(&Tok::RParen)?;
        if is_writeln {
            Ok(Statement::WriteLn { args, span })
        } else {
            Ok(Statement::Write { args, span })
        }
    }

    fn parse_readln(&mut self) -> Result<Statement, ParseError> {
        let span = self.span();
        self.advance(); // consume 'readln'
        self.expect(&Tok::LParen)?;
        let (target, _) = self.expect_ident()?;
        self.expect(&Tok::RParen)?;
        Ok(Statement::ReadLn { target, span })
    }

    // ── expressions (precedence climbing) ────────────────

    fn parse_expr(&mut self) -> Result<Expr, ParseError> {
        self.parse_comparison()
    }

    fn parse_comparison(&mut self) -> Result<Expr, ParseError> {
        let mut left = self.parse_additive()?;
        loop {
            let op = match self.peek() {
                Tok::Eq  => BinOp::Eq,
                Tok::Neq => BinOp::Neq,
                Tok::Lt  => BinOp::Lt,
                Tok::Gt  => BinOp::Gt,
                Tok::Lte => BinOp::Lte,
                Tok::Gte => BinOp::Gte,
                _ => break,
            };
            let span = self.span();
            self.advance();
            let right = self.parse_additive()?;
            left = Expr::BinOp { op, left: Box::new(left), right: Box::new(right), span };
        }
        Ok(left)
    }

    fn parse_additive(&mut self) -> Result<Expr, ParseError> {
        let mut left = self.parse_multiplicative()?;
        loop {
            let op = match self.peek() {
                Tok::Plus  => BinOp::Add,
                Tok::Minus => BinOp::Sub,
                Tok::KwOr  => BinOp::Or,
                _ => break,
            };
            let span = self.span();
            self.advance();
            let right = self.parse_multiplicative()?;
            left = Expr::BinOp { op, left: Box::new(left), right: Box::new(right), span };
        }
        Ok(left)
    }

    fn parse_multiplicative(&mut self) -> Result<Expr, ParseError> {
        let mut left = self.parse_unary()?;
        loop {
            let op = match self.peek() {
                Tok::Star  => BinOp::Mul,
                Tok::KwDiv => BinOp::Div,
                Tok::KwMod => BinOp::Mod,
                Tok::KwAnd => BinOp::And,
                _ => break,
            };
            let span = self.span();
            self.advance();
            let right = self.parse_unary()?;
            left = Expr::BinOp { op, left: Box::new(left), right: Box::new(right), span };
        }
        Ok(left)
    }

    fn parse_unary(&mut self) -> Result<Expr, ParseError> {
        match self.peek() {
            Tok::Minus => {
                let span = self.span();
                self.advance();
                let operand = self.parse_primary()?;
                Ok(Expr::UnaryOp { op: UnaryOp::Neg, operand: Box::new(operand), span })
            }
            Tok::KwNot => {
                let span = self.span();
                self.advance();
                let operand = self.parse_primary()?;
                Ok(Expr::UnaryOp { op: UnaryOp::Not, operand: Box::new(operand), span })
            }
            _ => self.parse_primary(),
        }
    }

    fn parse_primary(&mut self) -> Result<Expr, ParseError> {
        match self.peek().clone() {
            Tok::IntLit(n) => {
                let span = self.span();
                self.advance();
                Ok(Expr::IntLit(n, span))
            }
            Tok::StrLit(s) => {
                let span = self.span();
                self.advance();
                Ok(Expr::StrLit(s, span))
            }
            Tok::KwTrue => {
                let span = self.span();
                self.advance();
                Ok(Expr::BoolLit(true, span))
            }
            Tok::KwFalse => {
                let span = self.span();
                self.advance();
                Ok(Expr::BoolLit(false, span))
            }
            Tok::Ident(name) => {
                let span = self.span();
                self.advance();
                Ok(Expr::Var(name, span))
            }
            Tok::LParen => {
                self.advance();
                let expr = self.parse_expr()?;
                self.expect(&Tok::RParen)?;
                Ok(expr)
            }
            _ => Err(ParseError {
                message: format!("expected expression, found {}", self.peek()),
                span: self.span(),
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_minimal_program() {
        let source = r#"
program Hello;
begin
  writeln('Hello, World!')
end.
"#;
        let mut parser = Parser::new(source);
        let prog = parser.parse_program().expect("should parse");
        assert_eq!(prog.name, "Hello");
        assert!(prog.vars.is_empty());
        assert_eq!(prog.body.statements.len(), 1);
    }

    #[test]
    fn parse_variables_and_assignment() {
        let source = r#"
program Calc;
var
  x, y: integer;
  name: string;
begin
  x := 10;
  y := x + 5
end.
"#;
        let mut parser = Parser::new(source);
        let prog = parser.parse_program().expect("should parse");
        assert_eq!(prog.vars.len(), 2);
        assert_eq!(prog.vars[0].names, vec!["x", "y"]);
        assert_eq!(prog.body.statements.len(), 2);
    }

    #[test]
    fn parse_if_else() {
        let source = r#"
program Test;
var x: integer;
begin
  x := 5;
  if x > 3 then
    writeln('big')
  else
    writeln('small')
end.
"#;
        let mut parser = Parser::new(source);
        let prog = parser.parse_program().expect("should parse");
        assert_eq!(prog.body.statements.len(), 2);
    }

    #[test]
    fn parse_while_loop() {
        let source = r#"
program Loop;
var i: integer;
begin
  i := 0;
  while i < 10 do
  begin
    writeln(i);
    i := i + 1
  end
end.
"#;
        let mut parser = Parser::new(source);
        let prog = parser.parse_program().expect("should parse");
        assert_eq!(prog.body.statements.len(), 2);
    }
}
