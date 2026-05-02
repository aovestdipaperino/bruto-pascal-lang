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
    For, To, DownTo, Repeat, Until,
    KwWrite, KwWriteLn, KwReadLn, KwRead,
    KwDiv, KwMod, KwAnd, KwOr, KwNot,
    KwTrue, KwFalse,
    KwNew, KwDispose, KwNil,
    Const, KwType,
    KwProcedure, KwFunction, KwForward,
    KwSet, KwIn, KwCase, KwLabel, KwGoto, KwWith,
    KwFile, KwPacked,
    TyReal, TyChar, TyText,
    // Type keywords
    TyInteger, TyString, TyBoolean,
    // Literals
    IntLit(i64),
    RealLit(f64),
    CharLit(u8),
    StrLit(String),
    // Identifier
    Ident(String),
    // Operators
    Plus, Minus, Star, Slash, Assign, Eq, Neq, Lt, Gt, Lte, Gte,
    KwArray, KwOf, KwRecord,
    // Delimiters
    LParen, RParen, LBracket, RBracket, Semi, Colon, Comma, Dot, DotDot, Caret,
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
            Tok::KwNew => write!(f, "'new'"),
            Tok::KwDispose => write!(f, "'dispose'"),
            Tok::KwNil => write!(f, "'nil'"),
            Tok::KwFile => write!(f, "'file'"),
            Tok::KwPacked => write!(f, "'packed'"),
            Tok::TyText => write!(f, "'text'"),
            Tok::For => write!(f, "'for'"),
            Tok::To => write!(f, "'to'"),
            Tok::DownTo => write!(f, "'downto'"),
            Tok::Repeat => write!(f, "'repeat'"),
            Tok::Until => write!(f, "'until'"),
            Tok::KwRead => write!(f, "'read'"),
            Tok::Const => write!(f, "'const'"),
            Tok::KwType => write!(f, "'type'"),
            Tok::KwProcedure => write!(f, "'procedure'"),
            Tok::KwFunction => write!(f, "'function'"),
            Tok::KwForward => write!(f, "'forward'"),
            Tok::KwSet => write!(f, "'set'"),
            Tok::KwIn => write!(f, "'in'"),
            Tok::KwCase => write!(f, "'case'"),
            Tok::KwLabel => write!(f, "'label'"),
            Tok::KwGoto => write!(f, "'goto'"),
            Tok::KwWith => write!(f, "'with'"),
            Tok::TyReal => write!(f, "'real'"),
            Tok::TyChar => write!(f, "'char'"),
            Tok::KwArray => write!(f, "'array'"),
            Tok::KwOf => write!(f, "'of'"),
            Tok::KwRecord => write!(f, "'record'"),
            Tok::TyInteger => write!(f, "'integer'"),
            Tok::TyString => write!(f, "'string'"),
            Tok::TyBoolean => write!(f, "'boolean'"),
            Tok::IntLit(n) => write!(f, "{n}"),
            Tok::RealLit(r) => write!(f, "{r}"),
            Tok::CharLit(c) => write!(f, "#{c}"),
            Tok::StrLit(s) => write!(f, "'{s}'"),
            Tok::Ident(s) => write!(f, "identifier '{s}'"),
            Tok::Plus => write!(f, "'+'"),
            Tok::Minus => write!(f, "'-'"),
            Tok::Star => write!(f, "'*'"),
            Tok::Slash => write!(f, "'/'"),
            Tok::Assign => write!(f, "':='"),
            Tok::Eq => write!(f, "'='"),
            Tok::Neq => write!(f, "'<>'"),
            Tok::Lt => write!(f, "'<'"),
            Tok::Gt => write!(f, "'>'"),
            Tok::Lte => write!(f, "'<='"),
            Tok::Gte => write!(f, "'>='"),
            Tok::LParen => write!(f, "'('"),
            Tok::RParen => write!(f, "')'"),
            Tok::LBracket => write!(f, "'['"),
            Tok::RBracket => write!(f, "']'"),
            Tok::DotDot => write!(f, "'..'"),
            Tok::Semi => write!(f, "';'"),
            Tok::Colon => write!(f, "':'"),
            Tok::Comma => write!(f, "','"),
            Tok::Dot => write!(f, "'.'"),
            Tok::Caret => write!(f, "'^'"),
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

/// Compiler directives parsed from `{$X+/-}` or `(*$X+/-*)` comments.
/// Directives are program-global in this implementation: the last value wins.
#[derive(Debug, Clone, Copy, Default)]
pub struct Directives {
    pub range_check: bool,    // {$R+}
    pub overflow_check: bool, // {$Q+}
    pub io_check: bool,       // {$I+}  (default true in Turbo Pascal; we keep false to be lenient)
}

struct Lexer {
    chars: Vec<char>,
    pos: usize,
    line: u32,
    col: u32,
    directives: Directives,
}

impl Lexer {
    fn new(source: &str) -> Self {
        Self {
            chars: source.chars().collect(),
            pos: 0,
            line: 1,
            col: 1,
            directives: Directives::default(),
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

    fn handle_directive(&mut self, body: &str) {
        // A directive looks like `$R+`, `$Q-`, `$I+`, possibly with multiple
        // entries separated by commas, e.g. `$R+,Q+`.
        let body = body.trim();
        if !body.starts_with('$') { return; }
        for entry in body[1..].split(',') {
            let e = entry.trim();
            if e.len() < 2 { continue; }
            let kind = e.as_bytes()[0].to_ascii_uppercase();
            let sign = e.as_bytes()[1];
            let on = sign == b'+';
            match kind {
                b'R' => self.directives.range_check = on,
                b'Q' => self.directives.overflow_check = on,
                b'I' => self.directives.io_check = on,
                _ => {}
            }
        }
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

            // Block comment: { }   (also `{$X+/-}` directives)
            if self.peek() == '{' {
                self.advance();
                let mut body = String::new();
                while self.pos < self.chars.len() && self.peek() != '}' {
                    body.push(self.advance());
                }
                if self.pos < self.chars.len() { self.advance(); }
                self.handle_directive(&body);
                continue;
            }

            // Block comment: (* *)   (also `(*$X+/-*)` directives)
            if self.peek() == '(' && self.chars.get(self.pos + 1) == Some(&'*') {
                self.advance(); self.advance();
                let mut body = String::new();
                loop {
                    if self.pos >= self.chars.len() { break; }
                    if self.peek() == '*' && self.chars.get(self.pos + 1) == Some(&')') {
                        self.advance(); self.advance();
                        break;
                    }
                    body.push(self.advance());
                }
                self.handle_directive(&body);
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

        // Number (integer or real)
        if ch.is_ascii_digit() {
            let mut n: i64 = 0;
            while self.pos < self.chars.len() && self.peek().is_ascii_digit() {
                n = n * 10 + (self.advance() as i64 - '0' as i64);
            }
            // Check for decimal point (but not ".." range operator)
            if self.peek() == '.' && self.chars.get(self.pos + 1) != Some(&'.') {
                self.advance(); // consume '.'
                let mut frac = String::new();
                frac.push_str(&n.to_string());
                frac.push('.');
                while self.pos < self.chars.len() && self.peek().is_ascii_digit() {
                    frac.push(self.advance());
                }
                let val: f64 = frac.parse().unwrap_or(0.0);
                return (Tok::RealLit(val), span);
            }
            return (Tok::IntLit(n), span);
        }

        // Char literal: #nn
        if ch == '#' {
            self.advance();
            let mut n: u8 = 0;
            while self.pos < self.chars.len() && self.peek().is_ascii_digit() {
                n = n.wrapping_mul(10).wrapping_add(self.advance() as u8 - b'0');
            }
            return (Tok::CharLit(n), span);
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
                "new"      => Tok::KwNew,
                "dispose"  => Tok::KwDispose,
                "nil"      => Tok::KwNil,
                "file"     => Tok::KwFile,
                "packed"   => Tok::KwPacked,
                "text"     => Tok::TyText,
                "for"      => Tok::For,
                "to"       => Tok::To,
                "downto"   => Tok::DownTo,
                "repeat"   => Tok::Repeat,
                "until"    => Tok::Until,
                "read"     => Tok::KwRead,
                "const"    => Tok::Const,
                "type"     => Tok::KwType,
                "procedure" => Tok::KwProcedure,
                "function" => Tok::KwFunction,
                "forward"  => Tok::KwForward,
                "set"      => Tok::KwSet,
                "in"       => Tok::KwIn,
                "case"     => Tok::KwCase,
                "label"    => Tok::KwLabel,
                "goto"     => Tok::KwGoto,
                "with"     => Tok::KwWith,
                "array"    => Tok::KwArray,
                "of"       => Tok::KwOf,
                "record"   => Tok::KwRecord,
                "integer"  => Tok::TyInteger,
                "string"   => Tok::TyString,
                "boolean"  => Tok::TyBoolean,
                "real"     => Tok::TyReal,
                "char"     => Tok::TyChar,
                _          => Tok::Ident(word),
            };
            return (tok, span);
        }

        // Two-character operators
        if ch == '.' && self.chars.get(self.pos + 1) == Some(&'.') {
            self.advance(); self.advance();
            return (Tok::DotDot, span);
        }
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
            '/' => Tok::Slash,
            '=' => Tok::Eq,
            '<' => Tok::Lt,
            '>' => Tok::Gt,
            '(' => Tok::LParen,
            ')' => Tok::RParen,
            '[' => Tok::LBracket,
            ']' => Tok::RBracket,
            ';' => Tok::Semi,
            ':' => Tok::Colon,
            ',' => Tok::Comma,
            '.' => Tok::Dot,
            '^' => Tok::Caret,
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
    pub directives: Directives,
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
        Self { tokens, pos: 0, directives: lexer.directives }
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
        // Optional program parameters: `program Foo(input, output, dataFile);`
        // Parsed but ignored; predefined input/output already exist.
        if *self.peek() == Tok::LParen {
            self.advance();
            if *self.peek() != Tok::RParen {
                let _ = self.expect_ident()?;
                while *self.peek() == Tok::Comma {
                    self.advance();
                    let _ = self.expect_ident()?;
                }
            }
            self.expect(&Tok::RParen)?;
        }
        self.expect(&Tok::Semi)?;

        let labels = if *self.peek() == Tok::KwLabel {
            self.parse_label_section()?
        } else {
            Vec::new()
        };

        let consts = if *self.peek() == Tok::Const {
            self.parse_const_section()?
        } else {
            Vec::new()
        };

        let type_decls = if *self.peek() == Tok::KwType {
            self.parse_type_section()?
        } else {
            Vec::new()
        };

        let vars = if *self.peek() == Tok::Var {
            self.parse_var_section()?
        } else {
            Vec::new()
        };

        let mut procedures = Vec::new();
        while *self.peek() == Tok::KwProcedure || *self.peek() == Tok::KwFunction {
            procedures.push(self.parse_proc_decl()?);
        }

        let body = self.parse_block()?;
        self.expect(&Tok::Dot)?;

        Ok(Program { name, labels, consts, type_decls, vars, procedures, body, span })
    }

    // ── label section ────────────────────────────────────

    fn parse_label_section(&mut self) -> Result<Vec<i64>, ParseError> {
        self.expect(&Tok::KwLabel)?;
        let mut labels = Vec::new();
        labels.push(self.parse_int_literal()?);
        while *self.peek() == Tok::Comma {
            self.advance();
            labels.push(self.parse_int_literal()?);
        }
        self.expect(&Tok::Semi)?;
        Ok(labels)
    }

    // ── type section ─────────────────────────────────────

    fn parse_type_section(&mut self) -> Result<Vec<TypeDecl>, ParseError> {
        self.expect(&Tok::KwType)?;
        let mut decls = Vec::new();
        while matches!(self.peek(), Tok::Ident(_)) {
            let span = self.span();
            let (decl_name, _) = self.expect_ident()?;
            self.expect(&Tok::Eq)?;
            let mut ty = self.parse_type()?;
            if let PascalType::Enum { ref mut name, .. } = ty {
                *name = decl_name.clone();
            }
            self.expect(&Tok::Semi)?;
            decls.push(TypeDecl { name: decl_name, ty, span });
        }
        if decls.is_empty() {
            return Err(ParseError {
                message: "expected at least one type declaration after 'type'".into(),
                span: self.span(),
            });
        }
        Ok(decls)
    }

    // ── const section ────────────────────────────────────

    fn parse_const_section(&mut self) -> Result<Vec<ConstDecl>, ParseError> {
        self.expect(&Tok::Const)?;
        let mut decls = Vec::new();
        while matches!(self.peek(), Tok::Ident(_)) {
            let span = self.span();
            let (name, _) = self.expect_ident()?;
            // Optional type annotation: `name : type = value`
            let ty = if *self.peek() == Tok::Colon {
                self.advance();
                Some(self.parse_type()?)
            } else {
                None
            };
            self.expect(&Tok::Eq)?;
            let value = self.parse_expr()?;
            self.expect(&Tok::Semi)?;
            decls.push(ConstDecl { name, ty, value, span });
        }
        if decls.is_empty() {
            return Err(ParseError {
                message: "expected at least one constant declaration after 'const'".into(),
                span: self.span(),
            });
        }
        Ok(decls)
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
        // `packed` is accepted but ignored (no special layout).
        if *self.peek() == Tok::KwPacked {
            self.advance();
        }
        if *self.peek() == Tok::Caret {
            self.advance();
            let pointed = self.parse_type()?;
            return Ok(PascalType::Pointer(Box::new(pointed)));
        }
        if *self.peek() == Tok::LParen {
            return self.parse_enum_type();
        }
        if matches!(self.peek(), Tok::IntLit(_) | Tok::Minus) {
            let lo = self.parse_int_literal()?;
            self.expect(&Tok::DotDot)?;
            let hi = self.parse_int_literal()?;
            return Ok(PascalType::Subrange { lo, hi });
        }
        if *self.peek() == Tok::KwSet {
            self.advance();
            self.expect(&Tok::KwOf)?;
            let elem = self.parse_type()?;
            return Ok(PascalType::Set { elem: Box::new(elem) });
        }
        if *self.peek() == Tok::KwFile {
            self.advance();
            self.expect(&Tok::KwOf)?;
            let elem = self.parse_type()?;
            return Ok(PascalType::File { elem: Box::new(elem) });
        }
        if *self.peek() == Tok::TyText {
            self.advance();
            return Ok(PascalType::File { elem: Box::new(PascalType::Char) });
        }
        if *self.peek() == Tok::KwArray {
            // Look ahead: `array[ident .. ident : type ] of T` is a conformant
            // array parameter; otherwise a regular fixed-size array.
            if let Some((Tok::LBracket, _)) = self.tokens.get(self.pos + 1) {
                let mut p = self.pos + 2;
                let mut depth = 1;
                let mut saw_colon = false;
                while depth > 0 && p < self.tokens.len() {
                    match &self.tokens[p].0 {
                        Tok::LBracket => depth += 1,
                        Tok::RBracket => depth -= 1,
                        Tok::Colon if depth == 1 => { saw_colon = true; break; }
                        Tok::Semi | Tok::Eof => break,
                        _ => {}
                    }
                    p += 1;
                }
                if saw_colon {
                    return self.parse_conformant_array_type();
                }
            }
            return self.parse_array_type();
        }
        if *self.peek() == Tok::KwRecord {
            return self.parse_record_type();
        }
        match self.peek() {
            Tok::TyInteger => { self.advance(); Ok(PascalType::Integer) }
            Tok::TyReal    => { self.advance(); Ok(PascalType::Real) }
            Tok::TyString  => {
                self.advance();
                // Optional length: `string[N]` — accepted but treated as plain string
                if *self.peek() == Tok::LBracket {
                    self.advance();
                    let _n = self.parse_int_literal()?;
                    self.expect(&Tok::RBracket)?;
                }
                Ok(PascalType::String)
            }
            Tok::TyBoolean => { self.advance(); Ok(PascalType::Boolean) }
            Tok::TyChar    => { self.advance(); Ok(PascalType::Char) }
            Tok::Ident(name) => {
                let name = name.clone();
                self.advance();
                Ok(PascalType::Named(name))
            }
            _ => Err(ParseError {
                message: format!("expected type name, found {}", self.peek()),
                span: self.span(),
            }),
        }
    }

    fn parse_conformant_array_type(&mut self) -> Result<PascalType, ParseError> {
        self.expect(&Tok::KwArray)?;
        self.expect(&Tok::LBracket)?;
        let (lo_name, _) = self.expect_ident()?;
        self.expect(&Tok::DotDot)?;
        let (hi_name, _) = self.expect_ident()?;
        self.expect(&Tok::Colon)?;
        let _bound_ty = self.parse_type()?;
        self.expect(&Tok::RBracket)?;
        self.expect(&Tok::KwOf)?;
        let elem = self.parse_type()?;
        Ok(PascalType::ConformantArray { lo_name, hi_name, elem: Box::new(elem) })
    }

    fn parse_array_type(&mut self) -> Result<PascalType, ParseError> {
        self.expect(&Tok::KwArray)?;
        self.expect(&Tok::LBracket)?;

        let mut dimensions = Vec::new();
        let lo = self.parse_int_literal()?;
        self.expect(&Tok::DotDot)?;
        let hi = self.parse_int_literal()?;
        dimensions.push((lo, hi));

        while *self.peek() == Tok::Comma {
            self.advance();
            let lo = self.parse_int_literal()?;
            self.expect(&Tok::DotDot)?;
            let hi = self.parse_int_literal()?;
            dimensions.push((lo, hi));
        }
        self.expect(&Tok::RBracket)?;
        self.expect(&Tok::KwOf)?;
        let elem = self.parse_type()?;

        // Build nested array types from innermost to outermost
        let mut result = elem;
        for (lo, hi) in dimensions.into_iter().rev() {
            result = PascalType::Array { lo, hi, elem: Box::new(result) };
        }
        Ok(result)
    }

    fn parse_int_literal(&mut self) -> Result<i64, ParseError> {
        let neg = if *self.peek() == Tok::Minus {
            self.advance();
            true
        } else {
            false
        };
        match self.peek().clone() {
            Tok::IntLit(n) => {
                self.advance();
                Ok(if neg { -n } else { n })
            }
            _ => Err(ParseError {
                message: format!("expected integer literal, found {}", self.peek()),
                span: self.span(),
            }),
        }
    }

    fn parse_record_type(&mut self) -> Result<PascalType, ParseError> {
        self.expect(&Tok::KwRecord)?;
        let mut fields = Vec::new();
        let mut variant = None;

        while *self.peek() != Tok::End {
            if *self.peek() == Tok::KwCase {
                variant = Some(Box::new(self.parse_variant_part()?));
                break;
            }
            // field1, field2: type;
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
            for name in names {
                fields.push((name, ty.clone()));
            }
            if *self.peek() == Tok::Semi {
                self.advance();
            }
        }
        self.expect(&Tok::End)?;
        Ok(PascalType::Record { fields, variant })
    }

    fn parse_variant_part(&mut self) -> Result<RecordVariant, ParseError> {
        self.expect(&Tok::KwCase)?;
        let (tag_name, _) = self.expect_ident()?;
        self.expect(&Tok::Colon)?;
        let tag_type = self.parse_type()?;
        self.expect(&Tok::KwOf)?;

        let mut variants = Vec::new();
        while *self.peek() != Tok::End {
            let mut values = Vec::new();
            values.push(self.parse_int_literal()?);
            while *self.peek() == Tok::Comma {
                self.advance();
                values.push(self.parse_int_literal()?);
            }
            self.expect(&Tok::Colon)?;
            self.expect(&Tok::LParen)?;
            let mut vfields = Vec::new();
            while *self.peek() != Tok::RParen {
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
                for name in names {
                    vfields.push((name, ty.clone()));
                }
                if *self.peek() == Tok::Semi {
                    self.advance();
                }
            }
            self.expect(&Tok::RParen)?;
            variants.push((values, vfields));
            if *self.peek() == Tok::Semi {
                self.advance();
            }
        }
        Ok(RecordVariant { tag_name, tag_type, variants })
    }

    fn parse_enum_type(&mut self) -> Result<PascalType, ParseError> {
        self.expect(&Tok::LParen)?;
        let mut values = Vec::new();
        let (first, _) = self.expect_ident()?;
        values.push(first);
        while *self.peek() == Tok::Comma {
            self.advance();
            let (name, _) = self.expect_ident()?;
            values.push(name);
        }
        self.expect(&Tok::RParen)?;
        Ok(PascalType::Enum { name: String::new(), values })
    }

    // ── procedure / function ──────────────────────────────

    fn parse_proc_decl(&mut self) -> Result<ProcDecl, ParseError> {
        let span = self.span();
        let is_function = *self.peek() == Tok::KwFunction;
        self.advance(); // consume 'procedure' or 'function'
        let (name, _) = self.expect_ident()?;

        let params = if *self.peek() == Tok::LParen {
            self.parse_param_list()?
        } else {
            Vec::new()
        };

        let return_type = if is_function {
            self.expect(&Tok::Colon)?;
            Some(self.parse_type()?)
        } else {
            None
        };

        self.expect(&Tok::Semi)?;

        // Forward declaration: procedure Foo; forward;
        if *self.peek() == Tok::KwForward {
            self.advance();
            self.expect(&Tok::Semi)?;
            return Ok(ProcDecl {
                name, params, return_type,
                vars: Vec::new(),
                nested_procs: Vec::new(),
                body: Block { statements: Vec::new(), span, end_span: span },
                span,
            });
        }

        let vars = if *self.peek() == Tok::Var {
            self.parse_var_section()?
        } else {
            Vec::new()
        };

        // Nested procedures/functions
        let mut nested_procs = Vec::new();
        while *self.peek() == Tok::KwProcedure || *self.peek() == Tok::KwFunction {
            nested_procs.push(self.parse_proc_decl()?);
        }

        let body = self.parse_block()?;
        self.expect(&Tok::Semi)?;

        Ok(ProcDecl { name, params, return_type, vars, nested_procs, body, span })
    }

    fn parse_param_list(&mut self) -> Result<Vec<ParamGroup>, ParseError> {
        self.expect(&Tok::LParen)?;
        let mut groups = Vec::new();
        if *self.peek() != Tok::RParen {
            groups.push(self.parse_param_group()?);
            while *self.peek() == Tok::Semi {
                self.advance();
                groups.push(self.parse_param_group()?);
            }
        }
        self.expect(&Tok::RParen)?;
        Ok(groups)
    }

    fn parse_param_group(&mut self) -> Result<ParamGroup, ParseError> {
        // Procedural/functional parameter:
        //   `procedure p`                      → procedure with no params
        //   `procedure p(args)`                → procedure with params
        //   `function f(args): T`              → function returning T
        //   `function f: T`                    → function with no params
        if *self.peek() == Tok::KwProcedure || *self.peek() == Tok::KwFunction {
            let is_func = *self.peek() == Tok::KwFunction;
            self.advance();
            let (pname, _) = self.expect_ident()?;
            let mut params: Vec<PascalType> = Vec::new();
            if *self.peek() == Tok::LParen {
                self.advance();
                if *self.peek() != Tok::RParen {
                    params.push(self.parse_proc_param_type()?);
                    while *self.peek() == Tok::Semi {
                        self.advance();
                        params.push(self.parse_proc_param_type()?);
                    }
                }
                self.expect(&Tok::RParen)?;
            }
            let return_type = if is_func {
                self.expect(&Tok::Colon)?;
                Some(Box::new(self.parse_type()?))
            } else {
                None
            };
            return Ok(ParamGroup {
                mode: ParamMode::Value,
                names: vec![pname],
                ty: PascalType::Proc { params, return_type },
            });
        }

        let mode = if *self.peek() == Tok::Var {
            self.advance();
            ParamMode::Var
        } else {
            ParamMode::Value
        };
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
        Ok(ParamGroup { mode, names, ty })
    }

    /// Parse a parameter-of-procedural type: just the type, ignoring names.
    /// Used inside the parens of `procedure p(name: T; ...)`.
    fn parse_proc_param_type(&mut self) -> Result<PascalType, ParseError> {
        // `var name: T` or `name: T` — we capture only the type.
        if *self.peek() == Tok::Var { self.advance(); }
        let _ = self.expect_ident()?;
        while *self.peek() == Tok::Comma {
            self.advance();
            let _ = self.expect_ident()?;
        }
        self.expect(&Tok::Colon)?;
        self.parse_type()
    }

    // ── block ────────────────────────────────────────────

    fn parse_block(&mut self) -> Result<Block, ParseError> {
        let span = self.span();
        self.expect(&Tok::Begin)?;
        let mut statements = Vec::new();
        // Parse statements separated by semicolons, until 'end'
        if *self.peek() != Tok::End {
            let stmt = self.parse_statement()?;
            let is_label = matches!(&stmt, Statement::Label { .. });
            statements.push(stmt);
            // After a label, the labeled statement follows without a semicolon
            if is_label && *self.peek() != Tok::End && *self.peek() != Tok::Semi {
                statements.push(self.parse_statement()?);
            }
            while *self.peek() == Tok::Semi {
                self.advance();
                if *self.peek() == Tok::End {
                    break;
                }
                let stmt = self.parse_statement()?;
                let is_label = matches!(&stmt, Statement::Label { .. });
                statements.push(stmt);
                if is_label && *self.peek() != Tok::End && *self.peek() != Tok::Semi {
                    statements.push(self.parse_statement()?);
                }
            }
        }
        let end_span = self.span();
        self.expect(&Tok::End)?;
        Ok(Block { statements, span, end_span })
    }

    // ── statement ────────────────────────────────────────

    fn parse_statement(&mut self) -> Result<Statement, ParseError> {
        // Check for label: N: statement
        if let Tok::IntLit(n) = self.peek().clone() {
            if self.tokens.get(self.pos + 1).map(|(t, _)| t == &Tok::Colon).unwrap_or(false) {
                let span = self.span();
                self.advance(); // consume number
                self.advance(); // consume colon
                return Ok(Statement::Label { label: n, span });
            }
        }

        match self.peek() {
            Tok::Begin => Ok(Statement::Block(self.parse_block()?)),
            Tok::If => self.parse_if(),
            Tok::While => self.parse_while(),
            Tok::For => self.parse_for(),
            Tok::Repeat => self.parse_repeat_until(),
            Tok::KwWrite => self.parse_write(false),
            Tok::KwWriteLn => self.parse_write(true),
            Tok::KwReadLn => self.parse_readln(),
            Tok::KwNew => self.parse_new(),
            Tok::KwDispose => self.parse_dispose(),
            Tok::KwCase => self.parse_case(),
            Tok::KwWith => self.parse_with(),
            Tok::KwGoto => {
                let span = self.span();
                self.advance();
                match self.peek().clone() {
                    Tok::IntLit(n) => {
                        self.advance();
                        Ok(Statement::Goto { label: n, span })
                    }
                    _ => Err(ParseError {
                        message: format!("expected label number after 'goto', found {}", self.peek()),
                        span: self.span(),
                    }),
                }
            }
            Tok::Ident(_) => self.parse_ident_statement(),
            _ => Err(ParseError {
                message: format!("expected statement, found {}", self.peek()),
                span: self.span(),
            }),
        }
    }

    fn parse_ident_statement(&mut self) -> Result<Statement, ParseError> {
        let span = self.span();
        let (name, _) = self.expect_ident()?;

        // Collect chain of postfix accesses (.field, [index], ^)
        let mut chain: Vec<LValueAccess> = Vec::new();
        loop {
            match self.peek() {
                Tok::Dot => {
                    self.advance();
                    let (field, _) = self.expect_ident()?;
                    chain.push(LValueAccess::Field(field));
                }
                Tok::LBracket => {
                    self.advance();
                    let index = self.parse_expr()?;
                    chain.push(LValueAccess::Index(index));
                    while *self.peek() == Tok::Comma {
                        self.advance();
                        let next = self.parse_expr()?;
                        chain.push(LValueAccess::Index(next));
                    }
                    self.expect(&Tok::RBracket)?;
                }
                Tok::Caret => {
                    self.advance();
                    chain.push(LValueAccess::Deref);
                }
                _ => break,
            }
        }

        if *self.peek() == Tok::Assign {
            self.advance();
            let expr = self.parse_expr()?;
            if chain.is_empty() {
                return Ok(Statement::Assignment { target: name, expr, span });
            }
            // Single-step chains: emit the specific existing statement types
            if chain.len() == 1 {
                match chain.into_iter().next().unwrap() {
                    LValueAccess::Field(field) => return Ok(Statement::FieldAssignment { target: name, field, expr, span }),
                    LValueAccess::Index(index) => return Ok(Statement::IndexAssignment { target: name, index, expr, span }),
                    LValueAccess::Deref => return Ok(Statement::DerefAssignment { target: name, expr, span }),
                }
            }
            // Multi-index shorthand: a[i, j] := expr
            if chain.iter().all(|a| matches!(a, LValueAccess::Index(_))) {
                let indices: Vec<Expr> = chain.into_iter().map(|a| match a { LValueAccess::Index(e) => e, _ => unreachable!() }).collect();
                return Ok(Statement::MultiIndexAssignment { target: name, indices, expr, span });
            }
            return Ok(Statement::ChainedAssignment { target: name, chain, expr, span });
        }

        // Procedure call (chain should be empty for proc calls)
        if *self.peek() == Tok::LParen {
            self.advance();
            let mut args = Vec::new();
            if *self.peek() != Tok::RParen {
                args.push(self.parse_expr()?);
                while *self.peek() == Tok::Comma {
                    self.advance();
                    args.push(self.parse_expr()?);
                }
            }
            self.expect(&Tok::RParen)?;
            return Ok(Statement::ProcCall { name, args, span });
        }

        // No-arg procedure call
        Ok(Statement::ProcCall { name, args: Vec::new(), span })
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

    fn parse_for(&mut self) -> Result<Statement, ParseError> {
        let span = self.span();
        self.expect(&Tok::For)?;
        let (var, _) = self.expect_ident()?;
        self.expect(&Tok::Assign)?;
        let from = self.parse_expr()?;
        let downto = match self.peek() {
            Tok::To => { self.advance(); false }
            Tok::DownTo => { self.advance(); true }
            _ => return Err(ParseError {
                message: format!("expected 'to' or 'downto', found {}", self.peek()),
                span: self.span(),
            }),
        };
        let to = self.parse_expr()?;
        self.expect(&Tok::Do)?;
        let body_stmt = self.parse_statement()?;
        let body = match body_stmt {
            Statement::Block(b) => b,
            other => { let s = other.span(); Block { span: s, end_span: s, statements: vec![other] } },
        };
        Ok(Statement::For { var, from, to, downto, body, span })
    }

    fn parse_repeat_until(&mut self) -> Result<Statement, ParseError> {
        let span = self.span();
        self.expect(&Tok::Repeat)?;
        let mut stmts = Vec::new();
        if *self.peek() != Tok::Until {
            stmts.push(self.parse_statement()?);
            while *self.peek() == Tok::Semi {
                self.advance();
                if *self.peek() == Tok::Until { break; }
                stmts.push(self.parse_statement()?);
            }
        }
        self.expect(&Tok::Until)?;
        let condition = self.parse_expr()?;
        Ok(Statement::RepeatUntil { body: stmts, condition, span })
    }

    fn parse_case(&mut self) -> Result<Statement, ParseError> {
        let span = self.span();
        self.expect(&Tok::KwCase)?;
        let expr = self.parse_expr()?;
        self.expect(&Tok::KwOf)?;

        let mut branches = Vec::new();
        let mut else_branch = None;

        loop {
            if *self.peek() == Tok::End { break; }
            if *self.peek() == Tok::Else {
                self.advance();
                let mut stmts = Vec::new();
                if *self.peek() != Tok::End {
                    stmts.push(self.parse_statement()?);
                    while *self.peek() == Tok::Semi {
                        self.advance();
                        if *self.peek() == Tok::End { break; }
                        stmts.push(self.parse_statement()?);
                    }
                }
                else_branch = Some(stmts);
                break;
            }

            let branch_span = self.span();
            let mut values = Vec::new();
            values.push(self.parse_case_value()?);
            while *self.peek() == Tok::Comma {
                self.advance();
                values.push(self.parse_case_value()?);
            }
            self.expect(&Tok::Colon)?;
            let mut body = Vec::new();
            body.push(self.parse_statement()?);
            branches.push(CaseBranch { values, body, span: branch_span });
            if *self.peek() == Tok::Semi { self.advance(); }
        }

        self.expect(&Tok::End)?;
        Ok(Statement::Case { expr, branches, else_branch, span })
    }

    fn parse_case_value(&mut self) -> Result<CaseValue, ParseError> {
        let first = self.parse_expr()?;
        if *self.peek() == Tok::DotDot {
            self.advance();
            let last = self.parse_expr()?;
            Ok(CaseValue::Range(first, last))
        } else {
            Ok(CaseValue::Single(first))
        }
    }

    fn parse_with(&mut self) -> Result<Statement, ParseError> {
        let span = self.span();
        self.expect(&Tok::KwWith)?;
        let (record_var, _) = self.expect_ident()?;
        self.expect(&Tok::Do)?;
        let body_stmt = self.parse_statement()?;
        let body = match body_stmt {
            Statement::Block(b) => b,
            other => {
                let s = other.span();
                Block { span: s, end_span: s, statements: vec![other] }
            }
        };
        Ok(Statement::With { record_var, body, span })
    }

    fn parse_new(&mut self) -> Result<Statement, ParseError> {
        let span = self.span();
        self.advance(); // consume 'new'
        self.expect(&Tok::LParen)?;
        let (target, _) = self.expect_ident()?;
        self.expect(&Tok::RParen)?;
        Ok(Statement::New { target, span })
    }

    fn parse_dispose(&mut self) -> Result<Statement, ParseError> {
        let span = self.span();
        self.advance(); // consume 'dispose'
        self.expect(&Tok::LParen)?;
        let (target, _) = self.expect_ident()?;
        self.expect(&Tok::RParen)?;
        Ok(Statement::Dispose { target, span })
    }

    fn parse_write(&mut self, is_writeln: bool) -> Result<Statement, ParseError> {
        let span = self.span();
        self.advance(); // consume 'write' or 'writeln'
        let mut args = Vec::new();
        if *self.peek() == Tok::LParen {
            self.advance();
            if *self.peek() != Tok::RParen {
                args.push(self.parse_write_arg()?);
                while *self.peek() == Tok::Comma {
                    self.advance();
                    args.push(self.parse_write_arg()?);
                }
            }
            self.expect(&Tok::RParen)?;
        }
        if is_writeln {
            Ok(Statement::WriteLn { args, span })
        } else {
            Ok(Statement::Write { args, span })
        }
    }

    fn parse_write_arg(&mut self) -> Result<WriteArg, ParseError> {
        let expr = self.parse_expr()?;
        let mut width = None;
        let mut precision = None;
        if *self.peek() == Tok::Colon {
            self.advance();
            width = Some(self.parse_expr()?);
            if *self.peek() == Tok::Colon {
                self.advance();
                precision = Some(self.parse_expr()?);
            }
        }
        Ok(WriteArg { expr, width, precision })
    }

    fn parse_readln(&mut self) -> Result<Statement, ParseError> {
        let span = self.span();
        self.advance(); // consume 'readln'
        let mut targets = Vec::new();
        if *self.peek() == Tok::LParen {
            self.advance();
            if *self.peek() != Tok::RParen {
                let (t, _) = self.expect_ident()?;
                targets.push(t);
                while *self.peek() == Tok::Comma {
                    self.advance();
                    let (t, _) = self.expect_ident()?;
                    targets.push(t);
                }
            }
            self.expect(&Tok::RParen)?;
        }
        Ok(Statement::ReadLn { targets, span })
    }

    // ── expressions (precedence climbing) ────────────────

    fn parse_set_element(&mut self) -> Result<SetElement, ParseError> {
        let first = self.parse_expr()?;
        if *self.peek() == Tok::DotDot {
            self.advance();
            let last = self.parse_expr()?;
            Ok(SetElement::Range(first, last))
        } else {
            Ok(SetElement::Single(first))
        }
    }

    fn parse_expr(&mut self) -> Result<Expr, ParseError> {
        self.parse_comparison()
    }

    fn parse_comparison(&mut self) -> Result<Expr, ParseError> {
        let mut left = self.parse_additive()?;
        loop {
            let op = match self.peek() {
                Tok::Eq   => BinOp::Eq,
                Tok::Neq  => BinOp::Neq,
                Tok::Lt   => BinOp::Lt,
                Tok::Gt   => BinOp::Gt,
                Tok::Lte  => BinOp::Lte,
                Tok::Gte  => BinOp::Gte,
                Tok::KwIn => BinOp::In,
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
                Tok::Slash => BinOp::RealDiv,
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
            Tok::RealLit(r) => {
                let span = self.span();
                self.advance();
                Ok(Expr::RealLit(r, span))
            }
            Tok::CharLit(c) => {
                let span = self.span();
                self.advance();
                Ok(Expr::CharLit(c, span))
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
            Tok::KwNil => {
                let span = self.span();
                self.advance();
                Ok(Expr::Nil(span))
            }
            Tok::TyInteger | Tok::TyReal | Tok::TyChar | Tok::TyBoolean
                if self.tokens.get(self.pos + 1).map(|(t, _)| t == &Tok::LParen).unwrap_or(false) =>
            {
                let span = self.span();
                let name = match self.peek() {
                    Tok::TyInteger => "integer",
                    Tok::TyReal => "real",
                    Tok::TyChar => "char",
                    Tok::TyBoolean => "boolean",
                    _ => unreachable!(),
                }.to_string();
                self.advance();
                self.expect(&Tok::LParen)?;
                let arg = self.parse_expr()?;
                self.expect(&Tok::RParen)?;
                Ok(Expr::Call { name, args: vec![arg], span })
            }
            Tok::Ident(name) => {
                let span = self.span();
                self.advance();
                let mut expr = if *self.peek() == Tok::LParen {
                    // Function call
                    self.advance();
                    let mut args = Vec::new();
                    if *self.peek() != Tok::RParen {
                        args.push(self.parse_expr()?);
                        while *self.peek() == Tok::Comma {
                            self.advance();
                            args.push(self.parse_expr()?);
                        }
                    }
                    self.expect(&Tok::RParen)?;
                    Expr::Call { name, args, span }
                } else {
                    Expr::Var(name, span)
                };
                // Postfix operators: [i], .field, ^
                loop {
                    match self.peek() {
                        Tok::LBracket => {
                            self.advance();
                            let index = self.parse_expr()?;
                            if *self.peek() == Tok::Comma {
                                let mut expr_so_far = Expr::Index { array: Box::new(expr), index: Box::new(index), span };
                                while *self.peek() == Tok::Comma {
                                    self.advance();
                                    let next_index = self.parse_expr()?;
                                    expr_so_far = Expr::Index { array: Box::new(expr_so_far), index: Box::new(next_index), span };
                                }
                                self.expect(&Tok::RBracket)?;
                                expr = expr_so_far;
                            } else {
                                self.expect(&Tok::RBracket)?;
                                expr = Expr::Index { array: Box::new(expr), index: Box::new(index), span };
                            }
                        }
                        Tok::Dot => {
                            self.advance();
                            let (field, _) = self.expect_ident()?;
                            expr = Expr::FieldAccess { record: Box::new(expr), field, span };
                        }
                        Tok::Caret => {
                            self.advance();
                            expr = Expr::Deref(Box::new(expr), span);
                        }
                        _ => break,
                    }
                }
                Ok(expr)
            }
            Tok::LParen => {
                self.advance();
                let expr = self.parse_expr()?;
                self.expect(&Tok::RParen)?;
                Ok(expr)
            }
            Tok::LBracket => {
                self.advance();
                let mut elements = Vec::new();
                if *self.peek() != Tok::RBracket {
                    elements.push(self.parse_set_element()?);
                    while *self.peek() == Tok::Comma {
                        self.advance();
                        elements.push(self.parse_set_element()?);
                    }
                }
                let span = self.span();
                self.expect(&Tok::RBracket)?;
                Ok(Expr::SetConstructor { elements, span })
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
