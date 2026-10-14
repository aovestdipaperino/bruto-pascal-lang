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
use std::collections::HashSet;
use std::fmt;
use std::path::{Path, PathBuf};

// ── Token types ──────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    // Keywords
    Program,
    Var,
    Begin,
    End,
    If,
    Then,
    Else,
    While,
    Do,
    For,
    To,
    DownTo,
    Repeat,
    Until,
    KwWrite,
    KwWriteLn,
    KwReadLn,
    KwRead,
    KwDiv,
    KwMod,
    KwXor,
    KwShl,
    KwShr,
    KwAnd,
    KwOr,
    KwNot,
    KwTrue,
    KwFalse,
    KwNew,
    KwDispose,
    KwNil,
    Const,
    KwType,
    KwProcedure,
    KwFunction,
    KwForward,
    KwSet,
    KwIn,
    KwCase,
    KwLabel,
    KwGoto,
    KwWith,
    KwFile,
    KwPacked,
    KwUnit,
    KwInterface,
    KwImplementation,
    KwUses,
    KwInitialization,
    KwObject,
    KwConstructor,
    KwDestructor,
    KwInherited,
    TyReal,
    TyChar,
    TyText,
    // Type keywords
    TyInteger,
    TyString,
    TyBoolean,
    // Literals
    IntLit(i64),
    RealLit(f64),
    CharLit(u8),
    StrLit(String),
    // Identifier
    Ident(String),
    // Operators
    Plus,
    Minus,
    Star,
    Slash,
    Assign,
    Eq,
    Neq,
    Lt,
    Gt,
    Lte,
    Gte,
    KwArray,
    KwOf,
    KwRecord,
    // Delimiters
    LParen,
    RParen,
    LBracket,
    RBracket,
    Semi,
    Colon,
    Comma,
    Dot,
    DotDot,
    Caret,
    At,
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
            Tok::KwXor => write!(f, "'xor'"),
            Tok::KwShl => write!(f, "'shl'"),
            Tok::KwShr => write!(f, "'shr'"),
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
            Tok::KwUnit => write!(f, "'unit'"),
            Tok::KwInterface => write!(f, "'interface'"),
            Tok::KwImplementation => write!(f, "'implementation'"),
            Tok::KwUses => write!(f, "'uses'"),
            Tok::KwInitialization => write!(f, "'initialization'"),
            Tok::KwObject => write!(f, "'object'"),
            Tok::KwConstructor => write!(f, "'constructor'"),
            Tok::KwDestructor => write!(f, "'destructor'"),
            Tok::KwInherited => write!(f, "'inherited'"),
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
            Tok::At => write!(f, "'@'"),
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
        write!(f, "{}: {}", self.span.describe(), self.message)
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

/// One source being read: the program or unit, or a file it includes.
struct Frame {
    chars: Vec<char>,
    pos: usize,
    line: u32,
    col: u32,
    file: u32,
    dir: Option<PathBuf>,
}

/// One open `{$IF...}` / `{$IFDEF...}` group.
struct Condition {
    /// Whether tokens in the current branch are compiled.
    active: bool,
    /// Whether some branch of this group has already been taken.
    taken: bool,
    /// Whether the enclosing code is active at all.
    parent_active: bool,
    /// Where the group's `{$IF...}` is, for "missing {$ENDIF}".
    opened_at: Span,
}

/// Where and how to read a source file.
#[derive(Debug, Clone, Default)]
pub struct ParseOptions {
    /// The file being parsed, for `{$I}` lookups relative to it and for
    /// debug info; `None` for the main program held in memory.
    pub file: Option<PathBuf>,
    /// Extra directories searched by `{$I file}`.
    pub include_dirs: Vec<PathBuf>,
}

struct Lexer {
    // The frame being read.
    chars: Vec<char>,
    pos: usize,
    line: u32,
    col: u32,
    file: u32,
    dir: Option<PathBuf>,
    /// Frames suspended by `{$I}`, resumed when the include ends.
    suspended: Vec<Frame>,
    directives: Directives,
    /// Upper-case conditional symbols.
    defines: HashSet<String>,
    conditions: Vec<Condition>,
    include_dirs: Vec<PathBuf>,
    /// The first preprocessing error (missing include, `{$error}`, ...).
    error: Option<ParseError>,
}

/// The conditional symbols FPC would define for the host, plus `BRUTO`.
fn predefined_symbols() -> Vec<&'static str> {
    let mut symbols = vec![
        "FPC",
        "BRUTO",
        "CPU64",
        "ENDIAN_LITTLE",
        "FPC_LITTLE_ENDIAN",
    ];
    if cfg!(unix) {
        symbols.push("UNIX");
    }
    if cfg!(target_os = "macos") {
        symbols.extend(["DARWIN", "BSD"]);
    }
    if cfg!(target_os = "linux") {
        symbols.push("LINUX");
    }
    if cfg!(windows) {
        symbols.extend(["WINDOWS", "MSWINDOWS", "WIN64"]);
    }
    if cfg!(target_arch = "x86_64") {
        symbols.extend(["CPUX86_64", "CPUAMD64", "CPUX64"]);
    }
    if cfg!(target_arch = "aarch64") {
        symbols.push("CPUAARCH64");
    }
    symbols
}

impl Lexer {
    fn new(source: &str, options: &ParseOptions) -> Self {
        let file = options
            .file
            .as_deref()
            .map(crate::source_map::intern)
            .unwrap_or(crate::source_map::MAIN_FILE);
        Self {
            chars: source.chars().collect(),
            pos: 0,
            line: 1,
            col: 1,
            file,
            dir: options
                .file
                .as_deref()
                .and_then(|f| f.parent())
                .map(Path::to_path_buf),
            suspended: Vec::new(),
            directives: Directives::default(),
            defines: predefined_symbols().into_iter().map(String::from).collect(),
            conditions: Vec::new(),
            include_dirs: options.include_dirs.clone(),
            error: None,
        }
    }

    fn span(&self) -> Span {
        Span::in_file(self.line, self.col, self.file)
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

    /// Whether tokens at the current position are compiled (not inside a
    /// false `{$IFDEF}` branch).
    fn active(&self) -> bool {
        self.conditions.last().is_none_or(|c| c.active)
    }

    fn fail(&mut self, message: impl Into<String>, span: Span) {
        if self.error.is_none() {
            self.error = Some(ParseError {
                message: message.into(),
                span,
            });
        }
    }

    /// Handle the body of a `{$...}` or `(*$...*)` comment.
    fn handle_directive(&mut self, body: &str, span: Span) {
        // Only `{$` with nothing in between starts a directive;
        // `{ $ifdef X }` is an ordinary comment.
        let Some(body) = body.strip_prefix('$') else {
            return;
        };
        let body = body.trim();
        let name_len = body
            .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
            .unwrap_or(body.len());
        let name = body[..name_len].to_ascii_lowercase();
        let rest = body[name_len..].trim();
        let first_word = |s: &str| -> String {
            s.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
                .next()
                .unwrap_or("")
                .to_ascii_uppercase()
        };

        // Conditionals are tracked even inside inactive branches, so their
        // nesting stays right.
        match name.as_str() {
            "ifdef" | "ifndef" | "if" => {
                let parent_active = self.active();
                let holds = parent_active
                    && match name.as_str() {
                        "ifdef" => self.defines.contains(&first_word(rest)),
                        "ifndef" => !self.defines.contains(&first_word(rest)),
                        _ => self.eval_condition(rest, span),
                    };
                self.conditions.push(Condition {
                    active: holds,
                    taken: holds,
                    parent_active,
                    opened_at: span,
                });
                return;
            }
            "elseif" | "elif" | "else" => {
                let is_else = name == "else";
                let Some(top) = self.conditions.last() else {
                    self.fail(format!("{{${name}}} without a matching {{$IF}}"), span);
                    return;
                };
                let could_take = top.parent_active && !top.taken;
                let holds = could_take && (is_else || self.eval_condition(rest, span));
                let top = self.conditions.last_mut().unwrap();
                top.active = holds;
                top.taken |= holds;
                return;
            }
            "endif" | "ifend" => {
                if self.conditions.pop().is_none() {
                    self.fail("{$ENDIF} without a matching {$IF}", span);
                }
                return;
            }
            _ => {}
        }
        if !self.active() {
            return;
        }

        match name.as_str() {
            "define" => {
                let symbol = first_word(rest);
                if !symbol.is_empty() {
                    self.defines.insert(symbol);
                }
            }
            "undef" => {
                self.defines.remove(&first_word(rest));
            }
            "error" | "fatal" => self.fail(format!("{{$error}} {rest}"), span),
            "include" => self.include(rest, span),
            "i" if !rest.starts_with(['+', '-']) => self.include(rest, span),
            // One-letter switches, possibly several: `{$R+,Q-}`.
            _ if name.len() == 1 => {
                for entry in body.split(',') {
                    let e = entry.trim().as_bytes();
                    if e.len() < 2 {
                        continue;
                    }
                    let on = e[1] == b'+';
                    match e[0].to_ascii_uppercase() {
                        b'R' => self.directives.range_check = on,
                        b'Q' => self.directives.overflow_check = on,
                        b'I' => self.directives.io_check = on,
                        _ => {}
                    }
                }
            }
            // `{$mode}`, `{$H-}`-style long switches, `{$codepage}`,
            // `{$inline}`, `{$warning}` and the rest don't change how
            // Bruto compiles.
            _ => {}
        }
    }

    /// `{$I name}`: start reading the named file; the current one resumes
    /// when it ends.
    fn include(&mut self, rest: &str, span: Span) {
        let name = rest.trim().trim_matches('\'').trim();
        if name.is_empty() || name.starts_with('%') {
            // `{$I %DATE%}` and friends insert compiler information; not
            // supported, and nothing Free Vision needs.
            return;
        }
        let Some(path) = self.find_include(name) else {
            self.fail(format!("include file '{name}' not found"), span);
            return;
        };
        let source = match crate::source_map::read_source(&path) {
            Ok(text) => text,
            Err(e) => {
                self.fail(format!("can't read include file '{name}': {e}"), span);
                return;
            }
        };
        if self.suspended.len() >= 64 {
            self.fail("include files nested too deeply", span);
            return;
        }
        let frame = Frame {
            chars: std::mem::replace(&mut self.chars, source.chars().collect()),
            pos: std::mem::replace(&mut self.pos, 0),
            line: std::mem::replace(&mut self.line, 1),
            col: std::mem::replace(&mut self.col, 1),
            file: std::mem::replace(&mut self.file, crate::source_map::intern(&path)),
            dir: std::mem::replace(&mut self.dir, path.parent().map(Path::to_path_buf)),
        };
        self.suspended.push(frame);
    }

    /// Look for an include file next to the current file, then in the
    /// include directories, then in the working directory. A name without
    /// an extension also matches `.inc`, `.pp` and `.pas`.
    fn find_include(&self, name: &str) -> Option<PathBuf> {
        let candidates: Vec<String> = if Path::new(name).extension().is_some() {
            vec![name.to_string()]
        } else {
            ["", ".inc", ".pp", ".pas"]
                .iter()
                .map(|ext| format!("{name}{ext}"))
                .collect()
        };
        let dirs = self
            .dir
            .iter()
            .chain(self.include_dirs.iter())
            .cloned()
            .chain(std::iter::once(PathBuf::from(".")));
        for dir in dirs {
            for candidate in &candidates {
                let path = dir.join(candidate);
                if path.is_file() {
                    return Some(path);
                }
                // Case-insensitive match, for sources written on DOS.
                if let Ok(entries) = std::fs::read_dir(&dir) {
                    for entry in entries.flatten() {
                        if entry
                            .file_name()
                            .to_string_lossy()
                            .eq_ignore_ascii_case(candidate)
                        {
                            return Some(entry.path());
                        }
                    }
                }
            }
        }
        None
    }

    /// Resume the file that included the one that just ended.
    fn resume_includer(&mut self) -> bool {
        let Some(frame) = self.suspended.pop() else {
            return false;
        };
        self.chars = frame.chars;
        self.pos = frame.pos;
        self.line = frame.line;
        self.col = frame.col;
        self.file = frame.file;
        self.dir = frame.dir;
        true
    }

    /// Evaluate a `{$IF}` / `{$ELSEIF}` expression: `defined(X)`,
    /// `undefined(X)`, `declared(X)` (always false), `not`, `and`, `or`,
    /// parentheses, numbers, the FPC version symbols and comparisons.
    fn eval_condition(&mut self, text: &str, span: Span) -> bool {
        let tokens = condition_tokens(text);
        let mut pos = 0;
        match eval_or(&tokens, &mut pos, &self.defines) {
            Some(v) if pos == tokens.len() => v != 0,
            _ => {
                self.fail(format!("can't evaluate {{$IF {text}}}"), span);
                false
            }
        }
    }

    fn skip_whitespace_and_comments(&mut self) {
        loop {
            // Whitespace
            while self.pos < self.chars.len() && self.peek().is_whitespace() {
                self.advance();
            }
            if self.pos >= self.chars.len() {
                return;
            }

            // Line comment: //
            if self.peek() == '/' && self.chars.get(self.pos + 1) == Some(&'/') {
                while self.pos < self.chars.len() && self.peek() != '\n' {
                    self.advance();
                }
                continue;
            }

            // Block comment: { } (also `{$...}` directives). Braces nest, as
            // in FPC's default mode, so a commented-out block may itself
            // contain `{ }` comments and directives, which are ignored.
            if self.peek() == '{' {
                let span = self.span();
                self.advance();
                let mut body = String::new();
                let mut depth = 1;
                while self.pos < self.chars.len() {
                    match self.peek() {
                        '{' => depth += 1,
                        '}' => {
                            depth -= 1;
                            if depth == 0 {
                                break;
                            }
                        }
                        _ => {}
                    }
                    body.push(self.advance());
                }
                if self.pos < self.chars.len() {
                    self.advance();
                }
                self.handle_directive(&body, span);
                continue;
            }

            // Block comment: (* *)   (also `(*$...*)` directives)
            if self.peek() == '(' && self.chars.get(self.pos + 1) == Some(&'*') {
                let span = self.span();
                self.advance();
                self.advance();
                let mut body = String::new();
                loop {
                    if self.pos >= self.chars.len() {
                        break;
                    }
                    if self.peek() == '*' && self.chars.get(self.pos + 1) == Some(&')') {
                        self.advance();
                        self.advance();
                        break;
                    }
                    body.push(self.advance());
                }
                self.handle_directive(&body, span);
                continue;
            }

            break;
        }
    }

    fn next_token(&mut self) -> (Tok, Span) {
        loop {
            self.skip_whitespace_and_comments();
            if self.pos >= self.chars.len() {
                if self.resume_includer() {
                    continue;
                }
                let span = self.span();
                if let Some(open) = self.conditions.last() {
                    let at = open.opened_at.describe();
                    self.fail(
                        format!("missing {{$ENDIF}} for the {{$IF...}} at {at}"),
                        span,
                    );
                }
                return (Tok::Eof, span);
            }
            let token = self.lex_token();
            // Tokens in a false conditional branch are read (so strings and
            // comments are skipped correctly) and dropped.
            if self.active() {
                return token;
            }
        }
    }

    /// Read one token; whitespace and comments have been skipped.
    fn lex_token(&mut self) -> (Tok, Span) {
        let span = self.span();

        let ch = self.peek();

        // String literal (it can't span lines)
        if ch == '\'' {
            self.advance();
            let mut s = String::new();
            loop {
                if self.pos >= self.chars.len() || self.peek() == '\n' {
                    if self.active() {
                        self.fail("unterminated string", span);
                    }
                    break;
                }
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
                "program" => Tok::Program,
                "var" => Tok::Var,
                "begin" => Tok::Begin,
                "end" => Tok::End,
                "if" => Tok::If,
                "then" => Tok::Then,
                "else" => Tok::Else,
                "while" => Tok::While,
                "do" => Tok::Do,
                "write" => Tok::KwWrite,
                "writeln" => Tok::KwWriteLn,
                "readln" => Tok::KwReadLn,
                "div" => Tok::KwDiv,
                "mod" => Tok::KwMod,
                "xor" => Tok::KwXor,
                "shl" => Tok::KwShl,
                "shr" => Tok::KwShr,
                "and" => Tok::KwAnd,
                "or" => Tok::KwOr,
                "not" => Tok::KwNot,
                "true" => Tok::KwTrue,
                "false" => Tok::KwFalse,
                "new" => Tok::KwNew,
                "dispose" => Tok::KwDispose,
                "nil" => Tok::KwNil,
                "file" => Tok::KwFile,
                "packed" => Tok::KwPacked,
                "text" => Tok::TyText,
                "for" => Tok::For,
                "to" => Tok::To,
                "downto" => Tok::DownTo,
                "repeat" => Tok::Repeat,
                "until" => Tok::Until,
                "read" => Tok::KwRead,
                "const" => Tok::Const,
                "type" => Tok::KwType,
                "procedure" => Tok::KwProcedure,
                "function" => Tok::KwFunction,
                "forward" => Tok::KwForward,
                "set" => Tok::KwSet,
                "in" => Tok::KwIn,
                "case" => Tok::KwCase,
                "label" => Tok::KwLabel,
                "goto" => Tok::KwGoto,
                "with" => Tok::KwWith,
                "unit" => Tok::KwUnit,
                "interface" => Tok::KwInterface,
                "implementation" => Tok::KwImplementation,
                "uses" => Tok::KwUses,
                "initialization" => Tok::KwInitialization,
                "object" => Tok::KwObject,
                "constructor" => Tok::KwConstructor,
                "destructor" => Tok::KwDestructor,
                "inherited" => Tok::KwInherited,
                "array" => Tok::KwArray,
                "of" => Tok::KwOf,
                "record" => Tok::KwRecord,
                "integer" => Tok::TyInteger,
                "string" => Tok::TyString,
                "boolean" => Tok::TyBoolean,
                "real" => Tok::TyReal,
                "char" => Tok::TyChar,
                _ => Tok::Ident(word),
            };
            return (tok, span);
        }

        // Two-character operators
        if ch == '.' && self.chars.get(self.pos + 1) == Some(&'.') {
            self.advance();
            self.advance();
            return (Tok::DotDot, span);
        }
        if ch == ':' && self.chars.get(self.pos + 1) == Some(&'=') {
            self.advance();
            self.advance();
            return (Tok::Assign, span);
        }
        if ch == '<' && self.chars.get(self.pos + 1) == Some(&'>') {
            self.advance();
            self.advance();
            return (Tok::Neq, span);
        }
        if ch == '<' && self.chars.get(self.pos + 1) == Some(&'=') {
            self.advance();
            self.advance();
            return (Tok::Lte, span);
        }
        if ch == '>' && self.chars.get(self.pos + 1) == Some(&'=') {
            self.advance();
            self.advance();
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
            '@' => Tok::At,
            _ => {
                // Unknown character — return as identifier for error recovery
                return (Tok::Ident(ch.to_string()), span);
            }
        };
        (tok, span)
    }
}

// ── Parser ───────────────────────────────────────────────

/// A token of a `{$IF}` expression.
#[derive(Debug, Clone, PartialEq)]
enum CondTok {
    Word(String),
    Num(i64),
    Op(String),
}

fn condition_tokens(text: &str) -> Vec<CondTok> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() {
            i += 1;
        } else if c.is_ascii_alphabetic() || c == '_' {
            let start = i;
            while i < chars.len() && (chars[i].is_ascii_alphanumeric() || chars[i] == '_') {
                i += 1;
            }
            let word: String = chars[start..i].iter().collect();
            out.push(CondTok::Word(word.to_ascii_uppercase()));
        } else if c.is_ascii_digit() {
            let start = i;
            while i < chars.len() && chars[i].is_ascii_digit() {
                i += 1;
            }
            let digits: String = chars[start..i].iter().collect();
            out.push(CondTok::Num(digits.parse().unwrap_or(0)));
        } else {
            let two: String = chars[i..(i + 2).min(chars.len())].iter().collect();
            if ["<=", ">=", "<>"].contains(&two.as_str()) {
                out.push(CondTok::Op(two));
                i += 2;
            } else {
                out.push(CondTok::Op(c.to_string()));
                i += 1;
            }
        }
    }
    out
}

fn eval_or(t: &[CondTok], pos: &mut usize, defs: &HashSet<String>) -> Option<i64> {
    let mut v = eval_and(t, pos, defs)?;
    while t.get(*pos) == Some(&CondTok::Word("OR".into())) {
        *pos += 1;
        let r = eval_and(t, pos, defs)?;
        v = ((v != 0) || (r != 0)) as i64;
    }
    Some(v)
}

fn eval_and(t: &[CondTok], pos: &mut usize, defs: &HashSet<String>) -> Option<i64> {
    let mut v = eval_not(t, pos, defs)?;
    while t.get(*pos) == Some(&CondTok::Word("AND".into())) {
        *pos += 1;
        let r = eval_not(t, pos, defs)?;
        v = ((v != 0) && (r != 0)) as i64;
    }
    Some(v)
}

fn eval_not(t: &[CondTok], pos: &mut usize, defs: &HashSet<String>) -> Option<i64> {
    if t.get(*pos) == Some(&CondTok::Word("NOT".into())) {
        *pos += 1;
        return Some((eval_not(t, pos, defs)? == 0) as i64);
    }
    let l = eval_primary(t, pos, defs)?;
    let Some(CondTok::Op(op)) = t.get(*pos) else {
        return Some(l);
    };
    let op = op.clone();
    if !["=", "<>", "<", ">", "<=", ">="].contains(&op.as_str()) {
        return Some(l);
    }
    *pos += 1;
    let r = eval_primary(t, pos, defs)?;
    Some(match op.as_str() {
        "=" => l == r,
        "<>" => l != r,
        "<" => l < r,
        ">" => l > r,
        "<=" => l <= r,
        _ => l >= r,
    } as i64)
}

fn eval_primary(t: &[CondTok], pos: &mut usize, defs: &HashSet<String>) -> Option<i64> {
    let tok = t.get(*pos)?.clone();
    *pos += 1;
    match tok {
        CondTok::Num(n) => Some(n),
        CondTok::Op(op) if op == "(" => {
            let v = eval_or(t, pos, defs)?;
            (t.get(*pos) == Some(&CondTok::Op(")".into()))).then(|| *pos += 1)?;
            Some(v)
        }
        CondTok::Word(w) if matches!(w.as_str(), "DEFINED" | "UNDEFINED" | "DECLARED") => {
            (t.get(*pos) == Some(&CondTok::Op("(".into()))).then(|| *pos += 1)?;
            let Some(CondTok::Word(symbol)) = t.get(*pos).cloned() else {
                return None;
            };
            *pos += 1;
            (t.get(*pos) == Some(&CondTok::Op(")".into()))).then(|| *pos += 1)?;
            Some(match w.as_str() {
                "DEFINED" => defs.contains(&symbol) as i64,
                "UNDEFINED" => !defs.contains(&symbol) as i64,
                // Bruto can't see declarations while preprocessing.
                _ => 0,
            })
        }
        CondTok::Word(w) => Some(match w.as_str() {
            "TRUE" => 1,
            "FALSE" => 0,
            // FPC 3.2.2 is the dialect Bruto follows.
            "FPC_VERSION" => 3,
            "FPC_RELEASE" => 2,
            "FPC_PATCH" => 2,
            "FPC_FULLVERSION" => 30202,
            _ => 0,
        }),
        CondTok::Op(_) => None,
    }
}

/// The declarations of a program, unit section or routine.
#[derive(Default)]
struct Decls {
    labels: Vec<i64>,
    consts: Vec<ConstDecl>,
    type_decls: Vec<TypeDecl>,
    vars: Vec<VarDecl>,
    procedures: Vec<ProcDecl>,
}

pub struct Parser {
    tokens: Vec<(Tok, Span)>,
    pos: usize,
    pub directives: Directives,
    /// A preprocessing error, reported by the first `parse_*` call.
    lex_error: Option<ParseError>,
}

impl Parser {
    pub fn new(source: &str) -> Self {
        Self::with_options(source, &ParseOptions::default())
    }

    /// Parse `source`, read from `options.file` if given, resolving
    /// `{$I}` includes relative to it and to `options.include_dirs`.
    pub fn with_options(source: &str, options: &ParseOptions) -> Self {
        let mut lexer = Lexer::new(source, options);
        let mut tokens = Vec::new();
        loop {
            let (tok, span) = lexer.next_token();
            let is_eof = tok == Tok::Eof;
            tokens.push((tok, span));
            if is_eof {
                break;
            }
        }
        Self {
            tokens,
            pos: 0,
            directives: lexer.directives,
            lex_error: lexer.error,
        }
    }

    /// The preprocessing error found while reading the source, if any
    /// (also returned by the first `parse_*` call).
    pub fn preprocessing_error(&self) -> Option<&ParseError> {
        self.lex_error.as_ref()
    }

    /// How many tokens the preprocessed source produced.
    pub fn token_count(&self) -> usize {
        self.tokens.len()
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

    /// The name of a keyword token that FPC code also uses as an
    /// identifier (a method or function name).
    fn soft_keyword(tok: &Tok) -> Option<&'static str> {
        Some(match tok {
            Tok::KwWrite => "write",
            Tok::KwWriteLn => "writeln",
            Tok::KwRead => "read",
            Tok::KwReadLn => "readln",
            Tok::KwNew => "new",
            Tok::KwDispose => "dispose",
            _ => return None,
        })
    }

    /// A member name after `.` or a routine name: an identifier, or one of
    /// the words Bruto treats as keywords that FPC code also uses as
    /// method names (`TStream.Write`, `TStream.Read`, ...).
    fn expect_member_name(&mut self) -> Result<(String, Span), ParseError> {
        let Some(word) = Self::soft_keyword(self.peek()) else {
            return self.expect_ident();
        };
        let span = self.span();
        self.advance();
        Ok((word.to_string(), span))
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
        if let Some(e) = self.lex_error.take() {
            return Err(e);
        }
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

        let uses = if *self.peek() == Tok::KwUses {
            self.parse_uses_clause()?
        } else {
            Vec::new()
        };

        let decls = self.parse_decls(false)?;

        let body = self.parse_block()?;
        self.expect(&Tok::Dot)?;

        Ok(Program {
            name,
            uses,
            labels: decls.labels,
            consts: decls.consts,
            type_decls: decls.type_decls,
            vars: decls.vars,
            procedures: decls.procedures,
            body,
            span,
        })
    }

    /// Whether the next token starts a routine: `procedure`, `function`,
    /// `constructor` or `destructor`.
    fn at_routine(&self) -> bool {
        matches!(
            self.peek(),
            Tok::KwProcedure | Tok::KwFunction | Tok::KwConstructor | Tok::KwDestructor
        )
    }

    /// Parse a declaration part: `label`, `const`, `type` and `var`
    /// sections and routines, in any order and as often as they appear.
    /// With `headers_only` (a unit interface), routines are headers.
    fn parse_decls(&mut self, headers_only: bool) -> Result<Decls, ParseError> {
        let mut decls = Decls::default();
        loop {
            match self.peek() {
                Tok::KwLabel => decls.labels.extend(self.parse_label_section()?),
                Tok::Const => decls.consts.extend(self.parse_const_section()?),
                Tok::KwType => decls.type_decls.extend(self.parse_type_section()?),
                Tok::Var => decls.vars.extend(self.parse_var_section()?),
                _ if self.at_routine() => {
                    let routine = if headers_only {
                        self.parse_proc_header()?
                    } else {
                        self.parse_proc_decl()?
                    };
                    decls.procedures.push(routine);
                }
                _ => return Ok(decls),
            }
        }
    }

    /// Parse `uses A, B, C;` (Borland/Delphi style). Unit names are
    /// identifiers; resolution to .pas files is the build pipeline's
    /// problem, not the parser's.
    fn parse_uses_clause(&mut self) -> Result<Vec<String>, ParseError> {
        self.expect(&Tok::KwUses)?;
        let mut names = Vec::new();
        let (first, _) = self.expect_ident()?;
        names.push(first);
        while *self.peek() == Tok::Comma {
            self.advance();
            let (name, _) = self.expect_ident()?;
            names.push(name);
        }
        self.expect(&Tok::Semi)?;
        Ok(names)
    }

    /// Parse a unit file: `unit Name; interface ... implementation ... end.`
    ///
    /// Both the interface and implementation sections accept the same
    /// const/type/var blocks as a program. Procedures in the interface
    /// must be header-only (no body, no `forward`); their bodies live
    /// in the implementation. A bare initialization block (`initialization
    /// stmts end.`) or trailing `begin ... end.` is supported.
    pub fn parse_unit(&mut self) -> Result<Unit, ParseError> {
        if let Some(e) = self.lex_error.take() {
            return Err(e);
        }
        let span = self.span();
        self.expect(&Tok::KwUnit)?;
        let (name, _) = self.expect_ident()?;
        self.expect(&Tok::Semi)?;

        self.expect(&Tok::KwInterface)?;

        let uses = if *self.peek() == Tok::KwUses {
            self.parse_uses_clause()?
        } else {
            Vec::new()
        };

        let interface = self.parse_decls(true)?;

        self.expect(&Tok::KwImplementation)?;

        // Implementation may have its own (private) uses clause.
        // We accept it and merge it with the interface uses.
        let mut all_uses = uses;
        if *self.peek() == Tok::KwUses {
            for u in self.parse_uses_clause()? {
                if !all_uses.iter().any(|x| x.eq_ignore_ascii_case(&u)) {
                    all_uses.push(u);
                }
            }
        }

        let implementation = self.parse_decls(false)?;
        let interface_consts = interface.consts;
        let interface_types = interface.type_decls;
        let interface_vars = interface.vars;
        let interface_proc_headers = interface.procedures;
        let impl_consts = implementation.consts;
        let impl_types = implementation.type_decls;
        let impl_vars = implementation.vars;
        let procedures = implementation.procedures;

        // Optional unit initialization block. Accepts both Turbo Pascal
        // (`begin ... end.`) and Delphi (`initialization ... end.`)
        // styles. A bare `end.` is also valid (no init).
        let init = if *self.peek() == Tok::Begin {
            let body = self.parse_block()?;
            Some(body)
        } else if *self.peek() == Tok::KwInitialization {
            self.advance();
            let body_span = self.span();
            let mut statements = Vec::new();
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
            Some(Block {
                statements,
                span: body_span,
                end_span,
            })
        } else {
            self.expect(&Tok::End)?;
            None
        };
        self.expect(&Tok::Dot)?;

        Ok(Unit {
            name,
            uses: all_uses,
            interface_consts,
            interface_types,
            interface_vars,
            interface_proc_headers,
            impl_consts,
            impl_types,
            impl_vars,
            procedures,
            init,
            span,
        })
    }

    /// Parse a routine heading: `procedure Name(params);`,
    /// `function Name(params): T;`, `constructor` / `destructor`, or a
    /// method implementation's `procedure TFoo.Name(...)`, followed by any
    /// calling-convention modifiers (`inline;`, `cdecl;`, ...), which are
    /// accepted and ignored.
    fn parse_routine_heading(&mut self) -> Result<ProcDecl, ParseError> {
        let span = self.span();
        let kind = match self.peek() {
            Tok::KwProcedure => RoutineKind::Procedure,
            Tok::KwFunction => RoutineKind::Function,
            Tok::KwConstructor => RoutineKind::Constructor,
            Tok::KwDestructor => RoutineKind::Destructor,
            _ => {
                return Err(ParseError {
                    message: format!("expected a procedure or function, found {}", self.peek()),
                    span,
                });
            }
        };
        self.advance();
        let (first, _) = self.expect_member_name()?;
        let (owner, name) = if *self.peek() == Tok::Dot {
            self.advance();
            let (method, _) = self.expect_member_name()?;
            (Some(first), method)
        } else {
            (None, first)
        };
        let params = if *self.peek() == Tok::LParen {
            self.parse_param_list()?
        } else {
            Vec::new()
        };
        // A method or forward-declared function may leave out its result
        // type where it is implemented.
        let return_type = if kind == RoutineKind::Function && *self.peek() == Tok::Colon {
            self.advance();
            Some(self.parse_type()?)
        } else {
            None
        };
        self.expect(&Tok::Semi)?;
        self.skip_routine_modifiers(&[
            "inline",
            "overload",
            "cdecl",
            "register",
            "stdcall",
            "pascal",
            "far",
            "near",
            "export",
            "reintroduce",
            "static",
        ])?;
        let mut decl = ProcDecl::header(name, kind, params, return_type, span);
        decl.owner = owner;
        Ok(decl)
    }

    /// Skip `modifier;` pairs whose (lower-case) name is in `names`.
    /// Returns the ones found.
    fn skip_routine_modifiers(&mut self, names: &[&str]) -> Result<Vec<String>, ParseError> {
        let mut found = Vec::new();
        while let Tok::Ident(word) = self.peek() {
            let lower = word.to_ascii_lowercase();
            if !names.contains(&lower.as_str()) {
                break;
            }
            self.advance();
            self.expect(&Tok::Semi)?;
            found.push(lower);
        }
        Ok(found)
    }

    /// Parse just the header of a routine, as in a unit's interface
    /// section, where the body comes later in the implementation.
    fn parse_proc_header(&mut self) -> Result<ProcDecl, ParseError> {
        let mut decl = self.parse_routine_heading()?;
        decl.is_forward = true;
        Ok(decl)
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
            decls.push(TypeDecl {
                name: decl_name,
                ty,
                span,
            });
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
            decls.push(ConstDecl {
                name,
                ty,
                value,
                span,
            });
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
        // Subrange: starts with a number, a sign, or a constant name
        // followed by `..`.
        let starts_subrange = match self.peek() {
            Tok::IntLit(_) | Tok::Minus => true,
            Tok::Ident(_) => matches!(self.tokens.get(self.pos + 1), Some((Tok::DotDot, _))),
            _ => false,
        };
        if starts_subrange {
            let lo = self.parse_const_bound()?;
            self.expect(&Tok::DotDot)?;
            let hi = self.parse_const_bound()?;
            return Ok(Self::make_subrange(lo, hi));
        }
        if *self.peek() == Tok::KwSet {
            self.advance();
            self.expect(&Tok::KwOf)?;
            let elem = self.parse_type()?;
            return Ok(PascalType::Set {
                elem: Box::new(elem),
            });
        }
        if *self.peek() == Tok::KwFile {
            self.advance();
            self.expect(&Tok::KwOf)?;
            let elem = self.parse_type()?;
            return Ok(PascalType::File {
                elem: Box::new(elem),
            });
        }
        if *self.peek() == Tok::TyText {
            self.advance();
            return Ok(PascalType::File {
                elem: Box::new(PascalType::Char),
            });
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
                        Tok::Colon if depth == 1 => {
                            saw_colon = true;
                            break;
                        }
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
        if *self.peek() == Tok::KwObject {
            return self.parse_object_type();
        }
        // Procedural type: `procedure(params)` or `function(params): T`.
        if matches!(self.peek(), Tok::KwProcedure | Tok::KwFunction) {
            let is_func = *self.peek() == Tok::KwFunction;
            self.advance();
            return self.parse_proc_signature(is_func);
        }
        match self.peek() {
            Tok::TyInteger => {
                self.advance();
                Ok(PascalType::Integer)
            }
            Tok::TyReal => {
                self.advance();
                Ok(PascalType::Real)
            }
            Tok::TyString => {
                self.advance();
                // Optional length: `string[N]` — accepted but treated as plain string
                if *self.peek() == Tok::LBracket {
                    self.advance();
                    let _n = self.parse_int_literal()?;
                    self.expect(&Tok::RBracket)?;
                }
                Ok(PascalType::String)
            }
            Tok::TyBoolean => {
                self.advance();
                Ok(PascalType::Boolean)
            }
            Tok::TyChar => {
                self.advance();
                Ok(PascalType::Char)
            }
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
        Ok(PascalType::ConformantArray {
            lo_name,
            hi_name,
            elem: Box::new(elem),
        })
    }

    fn parse_array_type(&mut self) -> Result<PascalType, ParseError> {
        self.expect(&Tok::KwArray)?;
        self.expect(&Tok::LBracket)?;

        let mut dimensions = Vec::new();
        loop {
            let lo = self.parse_const_bound()?;
            self.expect(&Tok::DotDot)?;
            let hi = self.parse_const_bound()?;
            dimensions.push((lo, hi));
            if *self.peek() != Tok::Comma {
                break;
            }
            self.advance();
        }
        self.expect(&Tok::RBracket)?;
        self.expect(&Tok::KwOf)?;
        let elem = self.parse_type()?;

        // Build nested array types from innermost to outermost. Literal
        // bounds give a plain `Array`; anything else is folded later by
        // the semantic pass.
        let mut result = elem;
        for (lo, hi) in dimensions.into_iter().rev() {
            let elem = Box::new(result);
            result = match (lo.as_int(), hi.as_int()) {
                (Some(lo), Some(hi)) => PascalType::Array { lo, hi, elem },
                _ => PascalType::ArrayExpr { lo, hi, elem },
            };
        }
        Ok(result)
    }

    /// Parse one bound of an array or subrange: a constant expression
    /// such as `10`, `Max` or `Max - 1`.
    fn parse_const_bound(&mut self) -> Result<ConstExpr, ParseError> {
        let span = self.span();
        let expr = self.parse_additive()?;
        ConstExpr::from_expr(&expr).ok_or_else(|| ParseError {
            message: "expected a constant expression".into(),
            span,
        })
    }

    /// Build a subrange type from two parsed bounds.
    fn make_subrange(lo: ConstExpr, hi: ConstExpr) -> PascalType {
        match (lo.as_int(), hi.as_int()) {
            (Some(lo), Some(hi)) => PascalType::Subrange { lo, hi },
            _ => PascalType::SubrangeExpr { lo, hi },
        }
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

    /// `object [(Parent)] fields and method headers end`, with optional
    /// `private` / `public` / `protected` sections (accepted, not
    /// enforced).
    fn parse_object_type(&mut self) -> Result<PascalType, ParseError> {
        self.expect(&Tok::KwObject)?;
        let parent = if *self.peek() == Tok::LParen {
            self.advance();
            let (parent, _) = self.expect_ident()?;
            self.expect(&Tok::RParen)?;
            Some(parent)
        } else {
            None
        };
        let mut obj = ObjectType {
            parent,
            fields: Vec::new(),
            methods: Vec::new(),
        };
        loop {
            match self.peek().clone() {
                Tok::End => {
                    self.advance();
                    break;
                }
                _ if self.at_routine() => {
                    let heading = self.parse_routine_heading()?;
                    let mods = self.skip_routine_modifiers(&[
                        "virtual",
                        "abstract",
                        "inline",
                        "overload",
                        "static",
                        "reintroduce",
                        "cdecl",
                    ])?;
                    obj.methods.push(MethodHeader {
                        name: heading.name,
                        kind: heading.kind,
                        params: heading.params,
                        return_type: heading.return_type,
                        is_virtual: mods.iter().any(|m| m == "virtual"),
                        is_abstract: mods.iter().any(|m| m == "abstract"),
                        span: heading.span,
                    });
                }
                Tok::Ident(word)
                    if matches!(
                        word.to_ascii_lowercase().as_str(),
                        "private" | "public" | "protected" | "published" | "strict"
                    ) && !matches!(
                        self.tokens.get(self.pos + 1),
                        Some((Tok::Colon | Tok::Comma, _))
                    ) =>
                {
                    self.advance();
                }
                Tok::Ident(_) => {
                    let decl = self.parse_var_decl()?;
                    for name in decl.names {
                        obj.fields.push((name, decl.ty.clone()));
                    }
                    if *self.peek() == Tok::Semi {
                        self.advance();
                    }
                }
                _ => {
                    return Err(ParseError {
                        message: format!(
                            "expected a field or method in object, found {}",
                            self.peek()
                        ),
                        span: self.span(),
                    });
                }
            }
        }
        Ok(PascalType::Object(Box::new(obj)))
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
        Ok(PascalType::Record {
            fields,
            variant,
            object: None,
        })
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
        Ok(RecordVariant {
            tag_name,
            tag_type,
            variants,
        })
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
        Ok(PascalType::Enum {
            name: String::new(),
            values,
        })
    }

    // ── procedure / function ──────────────────────────────

    fn parse_proc_decl(&mut self) -> Result<ProcDecl, ParseError> {
        let mut decl = self.parse_routine_heading()?;

        // Forward declaration: procedure Foo; forward;
        if *self.peek() == Tok::KwForward {
            self.advance();
            self.expect(&Tok::Semi)?;
            decl.is_forward = true;
            return Ok(decl);
        }

        let decls = self.parse_decls(false)?;
        decl.consts = decls.consts;
        decl.type_decls = decls.type_decls;
        decl.vars = decls.vars;
        decl.nested_procs = decls.procedures;
        decl.body = self.parse_block()?;
        self.expect(&Tok::Semi)?;
        Ok(decl)
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
            let ty = self.parse_proc_signature(is_func)?;
            return Ok(ParamGroup {
                mode: ParamMode::Value,
                names: vec![pname],
                ty,
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

    /// Parse the rest of a procedural type after `procedure` / `function`
    /// (and the name, for a procedural parameter): `[(params)] [: T]`.
    /// Only the parameter types are kept.
    fn parse_proc_signature(&mut self, is_func: bool) -> Result<PascalType, ParseError> {
        let mut params: Vec<PascalType> = Vec::new();
        if *self.peek() == Tok::LParen {
            self.advance();
            if *self.peek() != Tok::RParen {
                params.extend(self.parse_proc_param_types()?);
                while *self.peek() == Tok::Semi {
                    self.advance();
                    params.extend(self.parse_proc_param_types()?);
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
        Ok(PascalType::Proc {
            params,
            return_type,
        })
    }

    /// Parse one parameter group inside a procedural type, `[var] a, b: T`,
    /// returning `T` once per name.
    fn parse_proc_param_types(&mut self) -> Result<Vec<PascalType>, ParseError> {
        if *self.peek() == Tok::Var {
            self.advance();
        }
        let mut count = 1;
        let _ = self.expect_ident()?;
        while *self.peek() == Tok::Comma {
            self.advance();
            let _ = self.expect_ident()?;
            count += 1;
        }
        self.expect(&Tok::Colon)?;
        let ty = self.parse_type()?;
        Ok(vec![ty; count])
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
        Ok(Block {
            statements,
            span,
            end_span,
        })
    }

    // ── statement ────────────────────────────────────────

    fn parse_statement(&mut self) -> Result<Statement, ParseError> {
        // Check for label: N: statement
        if let Tok::IntLit(n) = self.peek().clone()
            && self
                .tokens
                .get(self.pos + 1)
                .map(|(t, _)| t == &Tok::Colon)
                .unwrap_or(false)
        {
            let span = self.span();
            self.advance(); // consume number
            self.advance(); // consume colon
            return Ok(Statement::Label { label: n, span });
        }

        // `Read := x` / `Write := y`: a function whose name Bruto treats as
        // a keyword, assigning its result.
        if matches!(self.tokens.get(self.pos + 1), Some((Tok::Assign, _)))
            && let Some(word) = Self::soft_keyword(self.peek())
        {
            self.tokens[self.pos].0 = Tok::Ident(word.to_string());
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
            Tok::KwRead => {
                let span = self.span();
                self.advance();
                let args = self.parse_call_args()?;
                Ok(Statement::ProcCall {
                    name: "read".to_string(),
                    args,
                    span,
                })
            }
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
                        message: format!(
                            "expected label number after 'goto', found {}",
                            self.peek()
                        ),
                        span: self.span(),
                    }),
                }
            }
            Tok::Ident(_) if self.at_flow_statement() => self.parse_flow_statement(),
            Tok::KwInherited => {
                let span = self.span();
                self.advance();
                let (method, _) = self.expect_member_name()?;
                let args = self.parse_call_args()?;
                Ok(Statement::Inherited { method, args, span })
            }
            Tok::Ident(_) => self.parse_ident_statement(),
            _ => Err(ParseError {
                message: format!("expected statement, found {}", self.peek()),
                span: self.span(),
            }),
        }
    }

    /// `exit`, `break` and `continue` are predeclared procedures in Turbo
    /// Pascal rather than reserved words, so a variable may still be
    /// called `exit`; treat them as statements only when not assigned to.
    fn at_flow_statement(&self) -> bool {
        let Tok::Ident(name) = self.peek() else {
            return false;
        };
        let is_flow = ["exit", "break", "continue"]
            .iter()
            .any(|k| name.eq_ignore_ascii_case(k));
        let assigned = matches!(
            self.tokens.get(self.pos + 1),
            Some((Tok::Assign | Tok::LBracket | Tok::Dot | Tok::Caret, _))
        );
        is_flow && !assigned
    }

    /// `exit`, `exit(value)`, `break` or `continue`.
    fn parse_flow_statement(&mut self) -> Result<Statement, ParseError> {
        let span = self.span();
        let (name, _) = self.expect_ident()?;
        match name.to_ascii_lowercase().as_str() {
            "break" => Ok(Statement::Break { span }),
            "continue" => Ok(Statement::Continue { span }),
            _ => {
                let mut value = None;
                if *self.peek() == Tok::LParen {
                    self.advance();
                    if *self.peek() != Tok::RParen {
                        value = Some(self.parse_expr()?);
                    }
                    self.expect(&Tok::RParen)?;
                }
                Ok(Statement::Exit { value, span })
            }
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
                    let (field, _) = self.expect_member_name()?;
                    // `obj.Method(args)` as a statement.
                    if *self.peek() == Tok::LParen {
                        let object = Self::chain_to_expr(&name, chain, span);
                        let args = self.parse_call_args()?;
                        return Ok(Statement::MethodCall {
                            object,
                            method: field,
                            args,
                            span,
                        });
                    }
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
                return Ok(Statement::Assignment {
                    target: name,
                    expr,
                    span,
                });
            }
            // Single-step chains: emit the specific existing statement types
            if chain.len() == 1 {
                match chain.into_iter().next().unwrap() {
                    LValueAccess::Field(field) => {
                        return Ok(Statement::FieldAssignment {
                            target: name,
                            field,
                            expr,
                            span,
                        });
                    }
                    LValueAccess::Index(index) => {
                        return Ok(Statement::IndexAssignment {
                            target: name,
                            index,
                            expr,
                            span,
                        });
                    }
                    LValueAccess::Deref => {
                        return Ok(Statement::DerefAssignment {
                            target: name,
                            expr,
                            span,
                        });
                    }
                }
            }
            // Multi-index shorthand: a[i, j] := expr
            if chain.iter().all(|a| matches!(a, LValueAccess::Index(_))) {
                let indices: Vec<Expr> = chain
                    .into_iter()
                    .map(|a| match a {
                        LValueAccess::Index(e) => e,
                        _ => unreachable!(),
                    })
                    .collect();
                return Ok(Statement::MultiIndexAssignment {
                    target: name,
                    indices,
                    expr,
                    span,
                });
            }
            return Ok(Statement::ChainedAssignment {
                target: name,
                chain,
                expr,
                span,
            });
        }

        // `obj.Method` or `p^.Done` with no arguments: a method call.
        if let Some(LValueAccess::Field(_)) = chain.last() {
            let Some(LValueAccess::Field(method)) = chain.pop() else {
                unreachable!()
            };
            return Ok(Statement::MethodCall {
                object: Self::chain_to_expr(&name, chain, span),
                method,
                args: Vec::new(),
                span,
            });
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
            // `Cast(x)^.Field := value` and the like: a call (usually a type
            // cast) followed by postfix operators is an assignment target.
            if matches!(self.peek(), Tok::Caret | Tok::Dot | Tok::LBracket) {
                let target = self.parse_postfix(Expr::Call { name, args, span }, span)?;
                if *self.peek() != Tok::Assign {
                    // `PGroup(Owner)^.Lock` and the like: a method call.
                    return match target {
                        Expr::MethodCall {
                            object,
                            method,
                            args,
                            span,
                        } => Ok(Statement::MethodCall {
                            object: *object,
                            method,
                            args,
                            span,
                        }),
                        Expr::FieldAccess {
                            record,
                            field,
                            span,
                        } => Ok(Statement::MethodCall {
                            object: *record,
                            method: field,
                            args: Vec::new(),
                            span,
                        }),
                        _ => Err(ParseError {
                            message: format!("expected ':=', found {}", self.peek()),
                            span: self.span(),
                        }),
                    };
                }
                self.expect(&Tok::Assign)?;
                let expr = self.parse_expr()?;
                return Ok(Statement::PlaceAssignment { target, expr, span });
            }
            return Ok(Statement::ProcCall { name, args, span });
        }

        // No-arg procedure call
        Ok(Statement::ProcCall {
            name,
            args: Vec::new(),
            span,
        })
    }

    /// Rebuild `name` plus postfix accesses as an expression.
    fn chain_to_expr(name: &str, chain: Vec<LValueAccess>, span: Span) -> Expr {
        let mut expr = Expr::Var(name.to_string(), span);
        for access in chain {
            expr = match access {
                LValueAccess::Field(field) => Expr::FieldAccess {
                    record: Box::new(expr),
                    field,
                    span,
                },
                LValueAccess::Index(index) => Expr::Index {
                    array: Box::new(expr),
                    index: Box::new(index),
                    span,
                },
                LValueAccess::Deref => Expr::Deref(Box::new(expr), span),
            };
        }
        expr
    }

    fn parse_if(&mut self) -> Result<Statement, ParseError> {
        let span = self.span();
        self.expect(&Tok::If)?;
        let condition = self.parse_expr()?;
        self.expect(&Tok::Then)?;
        let then_stmt = self.parse_statement()?;
        let then_branch = match then_stmt {
            Statement::Block(b) => b,
            other => {
                let s = other.span();
                Block {
                    span: s,
                    end_span: s,
                    statements: vec![other],
                }
            }
        };
        let else_branch = if *self.peek() == Tok::Else {
            self.advance();
            let else_stmt = self.parse_statement()?;
            let block = match else_stmt {
                Statement::Block(b) => b,
                other => {
                    let s = other.span();
                    Block {
                        span: s,
                        end_span: s,
                        statements: vec![other],
                    }
                }
            };
            Some(block)
        } else {
            None
        };
        Ok(Statement::If {
            condition,
            then_branch,
            else_branch,
            span,
        })
    }

    fn parse_while(&mut self) -> Result<Statement, ParseError> {
        let span = self.span();
        self.expect(&Tok::While)?;
        let condition = self.parse_expr()?;
        self.expect(&Tok::Do)?;
        let body_stmt = self.parse_statement()?;
        let body = match body_stmt {
            Statement::Block(b) => b,
            other => {
                let s = other.span();
                Block {
                    span: s,
                    end_span: s,
                    statements: vec![other],
                }
            }
        };
        Ok(Statement::While {
            condition,
            body,
            span,
        })
    }

    fn parse_for(&mut self) -> Result<Statement, ParseError> {
        let span = self.span();
        self.expect(&Tok::For)?;
        let (var, _) = self.expect_ident()?;
        self.expect(&Tok::Assign)?;
        let from = self.parse_expr()?;
        let downto = match self.peek() {
            Tok::To => {
                self.advance();
                false
            }
            Tok::DownTo => {
                self.advance();
                true
            }
            _ => {
                return Err(ParseError {
                    message: format!("expected 'to' or 'downto', found {}", self.peek()),
                    span: self.span(),
                });
            }
        };
        let to = self.parse_expr()?;
        self.expect(&Tok::Do)?;
        let body_stmt = self.parse_statement()?;
        let body = match body_stmt {
            Statement::Block(b) => b,
            other => {
                let s = other.span();
                Block {
                    span: s,
                    end_span: s,
                    statements: vec![other],
                }
            }
        };
        Ok(Statement::For {
            var,
            from,
            to,
            downto,
            body,
            span,
        })
    }

    fn parse_repeat_until(&mut self) -> Result<Statement, ParseError> {
        let span = self.span();
        self.expect(&Tok::Repeat)?;
        let mut stmts = Vec::new();
        if *self.peek() != Tok::Until {
            stmts.push(self.parse_statement()?);
            while *self.peek() == Tok::Semi {
                self.advance();
                if *self.peek() == Tok::Until {
                    break;
                }
                stmts.push(self.parse_statement()?);
            }
        }
        self.expect(&Tok::Until)?;
        let condition = self.parse_expr()?;
        Ok(Statement::RepeatUntil {
            body: stmts,
            condition,
            span,
        })
    }

    fn parse_case(&mut self) -> Result<Statement, ParseError> {
        let span = self.span();
        self.expect(&Tok::KwCase)?;
        let expr = self.parse_expr()?;
        self.expect(&Tok::KwOf)?;

        let mut branches = Vec::new();
        let mut else_branch = None;

        loop {
            if *self.peek() == Tok::End {
                break;
            }
            if *self.peek() == Tok::Else {
                self.advance();
                let mut stmts = Vec::new();
                if *self.peek() != Tok::End {
                    stmts.push(self.parse_statement()?);
                    while *self.peek() == Tok::Semi {
                        self.advance();
                        if *self.peek() == Tok::End {
                            break;
                        }
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
            branches.push(CaseBranch {
                values,
                body,
                span: branch_span,
            });
            if *self.peek() == Tok::Semi {
                self.advance();
            }
        }

        self.expect(&Tok::End)?;
        Ok(Statement::Case {
            expr,
            branches,
            else_branch,
            span,
        })
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
                Block {
                    span: s,
                    end_span: s,
                    statements: vec![other],
                }
            }
        };
        Ok(Statement::With {
            record_var,
            body,
            span,
        })
    }

    fn parse_new(&mut self) -> Result<Statement, ParseError> {
        let span = self.span();
        self.advance(); // consume 'new'
        self.expect(&Tok::LParen)?;
        let (target, _) = self.expect_ident()?;
        let constructor = self.parse_optional_method_arg()?;
        self.expect(&Tok::RParen)?;
        Ok(Statement::New {
            target,
            constructor,
            span,
        })
    }

    fn parse_dispose(&mut self) -> Result<Statement, ParseError> {
        let span = self.span();
        self.advance(); // consume 'dispose'
        self.expect(&Tok::LParen)?;
        let (target, _) = self.expect_ident()?;
        let destructor = self.parse_optional_method_arg()?;
        self.expect(&Tok::RParen)?;
        Ok(Statement::Dispose {
            target,
            destructor,
            span,
        })
    }

    /// The `, Init(args)` / `, Done` part of `new` and `dispose`.
    fn parse_optional_method_arg(&mut self) -> Result<Option<(String, Vec<Expr>)>, ParseError> {
        if *self.peek() != Tok::Comma {
            return Ok(None);
        }
        self.advance();
        let (method, _) = self.expect_ident()?;
        let args = self.parse_call_args()?;
        Ok(Some((method, args)))
    }

    /// `(args)` if present, else no arguments.
    fn parse_call_args(&mut self) -> Result<Vec<Expr>, ParseError> {
        let mut args = Vec::new();
        if *self.peek() != Tok::LParen {
            return Ok(args);
        }
        self.advance();
        if *self.peek() != Tok::RParen {
            args.push(self.parse_expr()?);
            while *self.peek() == Tok::Comma {
                self.advance();
                args.push(self.parse_expr()?);
            }
        }
        self.expect(&Tok::RParen)?;
        Ok(args)
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
        Ok(WriteArg {
            expr,
            width,
            precision,
        })
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
                Tok::Eq => BinOp::Eq,
                Tok::Neq => BinOp::Neq,
                Tok::Lt => BinOp::Lt,
                Tok::Gt => BinOp::Gt,
                Tok::Lte => BinOp::Lte,
                Tok::Gte => BinOp::Gte,
                Tok::KwIn => BinOp::In,
                _ => break,
            };
            let span = self.span();
            self.advance();
            let right = self.parse_additive()?;
            left = Expr::BinOp {
                op,
                left: Box::new(left),
                right: Box::new(right),
                span,
            };
        }
        Ok(left)
    }

    fn parse_additive(&mut self) -> Result<Expr, ParseError> {
        let mut left = self.parse_multiplicative()?;
        loop {
            let op = match self.peek() {
                Tok::Plus => BinOp::Add,
                Tok::Minus => BinOp::Sub,
                Tok::KwOr => BinOp::Or,
                Tok::KwXor => BinOp::Xor,
                _ => break,
            };
            let span = self.span();
            self.advance();
            let right = self.parse_multiplicative()?;
            left = Expr::BinOp {
                op,
                left: Box::new(left),
                right: Box::new(right),
                span,
            };
        }
        Ok(left)
    }

    fn parse_multiplicative(&mut self) -> Result<Expr, ParseError> {
        let mut left = self.parse_unary()?;
        loop {
            let op = match self.peek() {
                Tok::Star => BinOp::Mul,
                Tok::Slash => BinOp::RealDiv,
                Tok::KwDiv => BinOp::Div,
                Tok::KwMod => BinOp::Mod,
                Tok::KwAnd => BinOp::And,
                Tok::KwShl => BinOp::Shl,
                Tok::KwShr => BinOp::Shr,
                _ => break,
            };
            let span = self.span();
            self.advance();
            let right = self.parse_unary()?;
            left = Expr::BinOp {
                op,
                left: Box::new(left),
                right: Box::new(right),
                span,
            };
        }
        Ok(left)
    }

    fn parse_unary(&mut self) -> Result<Expr, ParseError> {
        match self.peek() {
            Tok::Minus => {
                let span = self.span();
                self.advance();
                let operand = self.parse_primary()?;
                Ok(Expr::UnaryOp {
                    op: UnaryOp::Neg,
                    operand: Box::new(operand),
                    span,
                })
            }
            Tok::KwNot => {
                let span = self.span();
                self.advance();
                let operand = self.parse_primary()?;
                Ok(Expr::UnaryOp {
                    op: UnaryOp::Not,
                    operand: Box::new(operand),
                    span,
                })
            }
            Tok::At => {
                let span = self.span();
                self.advance();
                let operand = self.parse_primary()?;
                Ok(Expr::AddrOf(Box::new(operand), span))
            }
            _ => self.parse_primary(),
        }
    }

    /// Apply any postfix operators to `expr`: `[i]` (or `[i, j]`),
    /// `.field` and `^`.
    fn parse_postfix(&mut self, mut expr: Expr, span: Span) -> Result<Expr, ParseError> {
        loop {
            match self.peek() {
                Tok::LBracket => {
                    self.advance();
                    loop {
                        let index = self.parse_expr()?;
                        expr = Expr::Index {
                            array: Box::new(expr),
                            index: Box::new(index),
                            span,
                        };
                        if *self.peek() != Tok::Comma {
                            break;
                        }
                        self.advance();
                    }
                    self.expect(&Tok::RBracket)?;
                }
                Tok::Dot => {
                    self.advance();
                    let (field, _) = self.expect_member_name()?;
                    expr = if *self.peek() == Tok::LParen {
                        Expr::MethodCall {
                            object: Box::new(expr),
                            method: field,
                            args: self.parse_call_args()?,
                            span,
                        }
                    } else {
                        Expr::FieldAccess {
                            record: Box::new(expr),
                            field,
                            span,
                        }
                    };
                }
                Tok::Caret => {
                    self.advance();
                    expr = Expr::Deref(Box::new(expr), span);
                }
                _ => return Ok(expr),
            }
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
            Tok::KwInherited => {
                let span = self.span();
                self.advance();
                let (method, _) = self.expect_member_name()?;
                let args = self.parse_call_args()?;
                Ok(Expr::Inherited { method, args, span })
            }
            // `New(PType, Init(args))`: the function form of new.
            Tok::KwNew => {
                let span = self.span();
                self.advance();
                let args = self.parse_call_args()?;
                Ok(Expr::Call {
                    name: "new".to_string(),
                    args,
                    span,
                })
            }
            Tok::TyInteger | Tok::TyReal | Tok::TyChar | Tok::TyBoolean
                if self
                    .tokens
                    .get(self.pos + 1)
                    .map(|(t, _)| t == &Tok::LParen)
                    .unwrap_or(false) =>
            {
                let span = self.span();
                let name = match self.peek() {
                    Tok::TyInteger => "integer",
                    Tok::TyReal => "real",
                    Tok::TyChar => "char",
                    Tok::TyBoolean => "boolean",
                    _ => unreachable!(),
                }
                .to_string();
                self.advance();
                self.expect(&Tok::LParen)?;
                let arg = self.parse_expr()?;
                self.expect(&Tok::RParen)?;
                Ok(Expr::Call {
                    name,
                    args: vec![arg],
                    span,
                })
            }
            Tok::Ident(name) => {
                let span = self.span();
                self.advance();
                let expr = if *self.peek() == Tok::LParen {
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
                self.parse_postfix(expr, span)
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
