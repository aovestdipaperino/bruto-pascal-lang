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
    WideCharLit(u16),
    StrLit(String),
    /// A string in a `{$codepage cp437}` file holding characters above
    /// 127: its bytes, and the Unicode text they stand for.
    CodePageStrLit(String, String),
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
            Tok::WideCharLit(c) => write!(f, "#{c}"),
            Tok::StrLit(s) | Tok::CodePageStrLit(s, _) => write!(f, "'{s}'"),
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
    /// Whether `string` means `ShortString` here: `{$H-}`, or a
    /// `{$mode}` (objfpc, fpc, tp) whose strings are short.
    short_strings: bool,
    /// Whether `Integer` means a 32-bit LongInt here: `{$mode objfpc}`
    /// and `{$mode delphi}`, as in FPC.
    int32: bool,
    /// `{$DEFINE NAME := text}` macros: upper-case name -> replacement.
    macros: std::collections::HashMap<String, String>,
    conditions: Vec<Condition>,
    include_dirs: Vec<PathBuf>,
    /// The first preprocessing error (missing include, `{$error}`, ...).
    error: Option<ParseError>,
    /// `{$codepage cp437}`: the source's characters 128..=255 are code
    /// page 437's.
    cp437: bool,
}

/// Code page 437's characters 128..=255, for `{$codepage cp437}`.
const CP437_HIGH: [u16; 128] = [
    0x00C7, 0x00FC, 0x00E9, 0x00E2, 0x00E4, 0x00E0, 0x00E5, 0x00E7, 0x00EA, 0x00EB, 0x00E8, 0x00EF,
    0x00EE, 0x00EC, 0x00C4, 0x00C5, 0x00C9, 0x00E6, 0x00C6, 0x00F4, 0x00F6, 0x00F2, 0x00FB, 0x00F9,
    0x00FF, 0x00D6, 0x00DC, 0x00A2, 0x00A3, 0x00A5, 0x20A7, 0x0192, 0x00E1, 0x00ED, 0x00F3, 0x00FA,
    0x00F1, 0x00D1, 0x00AA, 0x00BA, 0x00BF, 0x2310, 0x00AC, 0x00BD, 0x00BC, 0x00A1, 0x00AB, 0x00BB,
    0x2591, 0x2592, 0x2593, 0x2502, 0x2524, 0x2561, 0x2562, 0x2556, 0x2555, 0x2563, 0x2551, 0x2557,
    0x255D, 0x255C, 0x255B, 0x2510, 0x2514, 0x2534, 0x252C, 0x251C, 0x2500, 0x253C, 0x255E, 0x255F,
    0x255A, 0x2554, 0x2569, 0x2566, 0x2560, 0x2550, 0x256C, 0x2567, 0x2568, 0x2564, 0x2565, 0x2559,
    0x2558, 0x2552, 0x2553, 0x256B, 0x256A, 0x2518, 0x250C, 0x2588, 0x2584, 0x258C, 0x2590, 0x2580,
    0x03B1, 0x00DF, 0x0393, 0x03C0, 0x03A3, 0x03C3, 0x00B5, 0x03C4, 0x03A6, 0x0398, 0x03A9, 0x03B4,
    0x221E, 0x03C6, 0x03B5, 0x2229, 0x2261, 0x00B1, 0x2265, 0x2264, 0x2320, 0x2321, 0x00F7, 0x2248,
    0x00B0, 0x2219, 0x00B7, 0x221A, 0x207F, 0x00B2, 0x25A0, 0x00A0,
];

/// The conditional symbols FPC would define for the host, plus `BRUTO`.
fn predefined_symbols() -> Vec<&'static str> {
    // `CPULLVM`: FPC's LLVM code generator, which Bruto also is. Code that
    // checks for it (`objects.pp`'s typed nested callbacks) then takes the
    // path that doesn't rely on raw frame pointers.
    let mut symbols = vec![
        "FPC",
        "BRUTO",
        "CPU64",
        "CPULLVM",
        "ENDIAN_LITTLE",
        "FPC_LITTLE_ENDIAN",
        // The string types Bruto has, as FPC announces them.
        "FPC_HAS_FEATURE_ANSISTRINGS",
        "FPC_HAS_FEATURE_UNICODESTRINGS",
        "FPC_HAS_FEATURE_WIDESTRINGS",
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
            macros: std::collections::HashMap::new(),
            short_strings: false,
            int32: false,
            conditions: Vec::new(),
            include_dirs: options.include_dirs.clone(),
            error: None,
            cp437: false,
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
                    // `{$DEFINE NAME := text}` (with `{$MACRO ON}`) also makes
                    // NAME expand to text.
                    if let Some((_, text)) = rest.split_once(":=") {
                        self.macros.insert(symbol.clone(), text.trim().to_string());
                    }
                    self.defines.insert(symbol);
                }
            }
            "undef" => {
                let symbol = first_word(rest);
                self.macros.remove(&symbol);
                self.defines.remove(&symbol);
            }
            "error" | "fatal" => self.fail(format!("{{$error}} {rest}"), span),
            // FPC's default modes use short strings; Delphi mode doesn't.
            "mode" => match first_word(rest).as_str() {
                "OBJFPC" => {
                    self.short_strings = true;
                    self.int32 = true;
                }
                "FPC" | "TP" | "MACPAS" | "ISO" => {
                    self.short_strings = true;
                    self.int32 = false;
                }
                "DELPHI" | "DELPHIUNICODE" => {
                    self.short_strings = false;
                    self.int32 = true;
                }
                _ => {}
            },
            "longstrings" => self.short_strings = first_word(rest) == "OFF",
            "codepage" => self.cp437 = matches!(first_word(rest).as_str(), "CP437" | "437"),
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
                        b'H' => self.short_strings = !on,
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

    /// Start reading a macro's replacement text at `span`; the current
    /// source resumes when it ends.
    fn expand_macro(&mut self, text: &str, span: Span) {
        if self.suspended.len() >= 64 {
            self.fail("macros nested too deeply", span);
            return;
        }
        let frame = Frame {
            chars: std::mem::replace(&mut self.chars, text.chars().collect()),
            pos: std::mem::replace(&mut self.pos, 0),
            line: std::mem::replace(&mut self.line, span.line),
            col: std::mem::replace(&mut self.col, span.column),
            file: self.file,
            dir: self.dir.clone(),
        };
        self.suspended.push(frame);
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
            if !self.active() {
                continue;
            }
            // A macro name expands to its text, read as if it were here.
            if let Tok::Ident(word) = &token.0
                && let Some(text) = self.macros.get(&word.to_ascii_uppercase()).cloned()
            {
                self.expand_macro(&text, token.1);
                continue;
            }
            return token;
        }
    }

    /// A run of quoted strings and `#n` / `#$hh` character codes with
    /// nothing between them. Each character is one byte (Latin-1).
    fn lex_string_sequence(&mut self, span: Span) -> (Tok, Span) {
        let mut s = String::new();
        let mut pieces = 0;
        let mut only_codes = true;
        loop {
            match self.peek() {
                '\'' => {
                    only_codes = false;
                    self.advance();
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
                }
                '#' => {
                    self.advance();
                    let radix = if self.peek() == '$' {
                        self.advance();
                        16
                    } else {
                        10
                    };
                    let digits = |c: char| c.is_digit(radix);
                    let mut n: u32 = 0;
                    while self.pos < self.chars.len() && digits(self.peek()) {
                        let d = self.advance().to_digit(radix).unwrap_or(0);
                        n = n.wrapping_mul(radix).wrapping_add(d);
                    }
                    // Codes above 255 are UTF-16 code units (FPC's
                    // WideChar); a lone surrogate can't be held here.
                    s.push(char::from_u32(n & 0xFFFF).unwrap_or('\u{FFFD}'));
                }
                _ => break,
            }
            pieces += 1;
        }
        if pieces == 1 && only_codes {
            let code = u32::from(s.chars().next().unwrap_or('\0'));
            return match u8::try_from(code) {
                Ok(c) => (Tok::CharLit(c), span),
                Err(_) => (Tok::WideCharLit(code as u16), span),
            };
        }
        if self.cp437
            && s.chars().count() > 1
            && s.chars().any(|c| ('\u{80}'..='\u{FF}').contains(&c))
        {
            let text = s
                .chars()
                .map(|c| match u32::from(c) {
                    n @ 0x80..=0xFF => {
                        char::from_u32(u32::from(CP437_HIGH[n as usize - 0x80])).unwrap_or(c)
                    }
                    _ => c,
                })
                .collect();
            return (Tok::CodePageStrLit(s, text), span);
        }
        (Tok::StrLit(s), span)
    }

    /// Read one token; whitespace and comments have been skipped.
    fn lex_token(&mut self) -> (Tok, Span) {
        let span = self.span();

        let ch = self.peek();

        // String and character literals. Adjacent pieces form one string,
        // as in `'abc'#13#10` or `#27'[201~'`. A lone `#n` is a char.
        if ch == '\'' || ch == '#' {
            return self.lex_string_sequence(span);
        }

        // Number (integer or real)
        if ch.is_ascii_digit() {
            let mut n: i64 = 0;
            while self.pos < self.chars.len() && self.peek().is_ascii_digit() {
                n = n * 10 + (self.advance() as i64 - '0' as i64);
            }
            // An exponent: `e`/`E`, an optional sign, digits.
            let at_exponent = |lx: &Self| {
                matches!(lx.chars.get(lx.pos), Some('e' | 'E'))
                    && match lx.chars.get(lx.pos + 1) {
                        Some(c) if c.is_ascii_digit() => true,
                        Some('+' | '-') => {
                            lx.chars.get(lx.pos + 2).is_some_and(|c| c.is_ascii_digit())
                        }
                        _ => false,
                    }
            };
            // Check for decimal point (but not ".." range operator)
            let has_point = self.peek() == '.'
                && self
                    .chars
                    .get(self.pos + 1)
                    .is_some_and(|c| c.is_ascii_digit() || *c == 'e' || *c == 'E')
                || (self.peek() == '.' && self.chars.get(self.pos + 1) != Some(&'.'));
            if has_point || at_exponent(self) {
                let mut text = n.to_string();
                if has_point {
                    self.advance(); // consume '.'
                    text.push('.');
                    while self.pos < self.chars.len() && self.peek().is_ascii_digit() {
                        text.push(self.advance());
                    }
                }
                if at_exponent(self) {
                    text.push(self.advance());
                    if matches!(self.peek(), '+' | '-') {
                        text.push(self.advance());
                    }
                    while self.pos < self.chars.len() && self.peek().is_ascii_digit() {
                        text.push(self.advance());
                    }
                }
                let val: f64 = text.parse().unwrap_or(0.0);
                return (Tok::RealLit(val), span);
            }
            return (Tok::IntLit(n), span);
        }

        // Hex `$FF`, binary `%1010` and octal `&17` numbers.
        let radix = match ch {
            '$' => Some(16),
            '%' => Some(2),
            '&' if self.chars.get(self.pos + 1).is_some_and(|c| c.is_digit(8)) => Some(8),
            _ => None,
        };
        if let Some(radix) = radix {
            self.advance();
            let mut n: i64 = 0;
            while self.pos < self.chars.len() && self.peek().is_digit(radix) {
                let d = self.advance();
                n = n.wrapping_mul(radix as i64) + d.to_digit(radix).unwrap_or(0) as i64;
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
        // `sizeof(T)`. The preprocessor can't see declarations, so this
        // knows the built-in types, plus Free Vision's `Sw_` aliases,
        // which follow `FV_UNICODE`.
        CondTok::Word(w) if w == "SIZEOF" => {
            (t.get(*pos) == Some(&CondTok::Op("(".into()))).then(|| *pos += 1)?;
            let Some(CondTok::Word(name)) = t.get(*pos).cloned() else {
                return None;
            };
            *pos += 1;
            (t.get(*pos) == Some(&CondTok::Op(")".into()))).then(|| *pos += 1)?;
            let unicode = defs.contains("FV_UNICODE");
            Some(match name.as_str() {
                "BYTE" | "SHORTINT" | "CHAR" | "ANSICHAR" | "BOOLEAN" => 1,
                "WORD" | "SMALLINT" | "WIDECHAR" => 2,
                "LONGINT" | "LONGWORD" | "CARDINAL" | "DWORD" => 4,
                "INTEGER" | "INT64" | "QWORD" | "POINTER" | "PTRINT" | "PTRUINT" | "SIZEINT"
                | "SIZEUINT" | "ANSISTRING" | "UNICODESTRING" | "WIDESTRING" | "REAL"
                | "DOUBLE" => 8,
                "SHORTSTRING" => 256,
                "SW_STRING" if unicode => 8,
                "SW_STRING" => 256,
                "SW_CHAR" if unicode => 2,
                "SW_CHAR" => 1,
                "SW_WORD" | "SW_INTEGER" => 8,
                _ => return None,
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
    /// For each token, whether `string` meant `ShortString` there.
    short_strings: Vec<bool>,
    /// For each token, whether `Integer` meant a 32-bit LongInt there.
    int32: Vec<bool>,
    /// Aliases for the specializations written in expressions.
    hidden_types: Vec<TypeDecl>,
    pos: usize,
    pub directives: Directives,
    /// A preprocessing error, reported by the first `parse_*` call.
    lex_error: Option<ParseError>,
    /// The routines whose bodies are being parsed, innermost last: name
    /// and parameter names, for a bare `inherited`.
    routines: Vec<(String, Vec<String>)>,
}

impl Parser {
    /// Whether the source has `try`, `raise` or `array of const`, which
    /// need the runtime in `rtl/objpas.pp`.
    fn uses_exceptions(&self) -> bool {
        self.tokens.iter().any(|(t, _)| {
            matches!(t, Tok::Ident(w) if w.eq_ignore_ascii_case("try") || w.eq_ignore_ascii_case("raise"))
        }) || self
            .tokens
            .windows(3)
            .any(|w| matches!((&w[0].0, &w[1].0, &w[2].0), (Tok::KwArray, Tok::KwOf, Tok::Const)))
    }

    pub fn new(source: &str) -> Self {
        Self::with_options(source, &ParseOptions::default())
    }

    /// Parse `source`, read from `options.file` if given, resolving
    /// `{$I}` includes relative to it and to `options.include_dirs`.
    pub fn with_options(source: &str, options: &ParseOptions) -> Self {
        let mut lexer = Lexer::new(source, options);
        let mut tokens = Vec::new();
        let mut short_strings = Vec::new();
        let mut int32 = Vec::new();
        loop {
            let (tok, span) = lexer.next_token();
            let is_eof = tok == Tok::Eof;
            // `MaxInt` is High(Integer): a LongInt's in objfpc and delphi
            // modes.
            let tok = match tok {
                Tok::Ident(w) if lexer.int32 && w.eq_ignore_ascii_case("maxint") => {
                    Tok::IntLit(i32::MAX as i64)
                }
                other => other,
            };
            tokens.push((tok, span));
            short_strings.push(lexer.short_strings);
            int32.push(lexer.int32);
            if is_eof {
                break;
            }
        }
        Self {
            tokens,
            short_strings,
            int32,
            hidden_types: Vec::new(),
            pos: 0,
            directives: lexer.directives,
            lex_error: lexer.error,
            routines: Vec::new(),
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
            units: Vec::new(),
            overloads: std::collections::HashMap::new(),
            labels: decls.labels,
            consts: decls.consts,
            type_decls: decls
                .type_decls
                .into_iter()
                .chain(std::mem::take(&mut self.hidden_types))
                .collect(),
            vars: decls.vars,
            procedures: decls.procedures,
            body,
            span,
            uses_exceptions: self.uses_exceptions(),
        })
    }

    /// Whether the next token starts a routine: `procedure`, `function`,
    /// `constructor` or `destructor`.
    fn at_routine(&self) -> bool {
        let routine = |t: &Tok| {
            matches!(
                t,
                Tok::KwProcedure | Tok::KwFunction | Tok::KwConstructor | Tok::KwDestructor
            )
        };
        routine(self.peek())
            || (matches!(self.peek(), Tok::Ident(w)
                if w.eq_ignore_ascii_case("class") || w.eq_ignore_ascii_case("generic"))
                && self
                    .tokens
                    .get(self.pos + 1)
                    .is_some_and(|(t, _)| routine(t)))
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
                Tok::Ident(w) if w.eq_ignore_ascii_case("resourcestring") => {
                    decls.consts.extend(self.parse_const_section()?)
                }
                Tok::KwType => decls.type_decls.extend(self.parse_type_section()?),
                Tok::Var => {
                    let (vars, initialized) = self.parse_var_section()?;
                    decls.vars.extend(vars);
                    decls.consts.extend(initialized);
                }
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
        let mut names = vec![self.parse_unit_name()?];
        while *self.peek() == Tok::Comma {
            self.advance();
            names.push(self.parse_unit_name()?);
        }
        self.expect(&Tok::Semi)?;
        Ok(names)
    }

    /// A unit's name, dotted ones (`Generics.Collections`) included.
    fn parse_unit_name(&mut self) -> Result<String, ParseError> {
        let (mut name, _) = self.expect_ident()?;
        while *self.peek() == Tok::Dot
            && matches!(self.tokens.get(self.pos + 1), Some((Tok::Ident(_), _)))
        {
            self.advance();
            let (part, _) = self.expect_ident()?;
            name = format!("{name}.{part}");
        }
        Ok(name)
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
        let name = self.parse_unit_name()?;
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
        let interface_uses = uses.clone();
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
        // External routines declared in the interface have no body in
        // the implementation, so they are kept from the headers.
        let mut procedures: Vec<ProcDecl> = interface_proc_headers
            .iter()
            .filter(|p| p.external.is_some())
            .cloned()
            .collect();
        procedures.extend(implementation.procedures);

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
            interface_uses,
            interface_consts,
            interface_types,
            interface_vars,
            interface_proc_headers,
            impl_consts,
            impl_types: impl_types
                .into_iter()
                .chain(std::mem::take(&mut self.hidden_types))
                .collect(),
            impl_vars,
            procedures,
            init,
            span,
            uses_exceptions: self.uses_exceptions(),
        })
    }

    /// Parse a routine heading: `procedure Name(params);`,
    /// `function Name(params): T;`, `constructor` / `destructor`, or a
    /// method implementation's `procedure TFoo.Name(...)`, followed by any
    /// calling-convention modifiers (`inline;`, `cdecl;`, ...), which are
    /// accepted and ignored.
    fn parse_routine_heading(&mut self) -> Result<ProcDecl, ParseError> {
        // `class procedure TFoo.Bar`: whether it is a class method comes
        // from the class's declaration.
        if matches!(self.peek(), Tok::Ident(w)
            if w.eq_ignore_ascii_case("class") || w.eq_ignore_ascii_case("generic"))
        {
            self.advance();
        }
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
        // `function TList<T>.Add`: the generic's parameters, repeated.
        if *self.peek() == Tok::Lt && self.type_args_then_dot(self.pos) {
            while *self.peek() != Tok::Gt {
                self.advance();
            }
            self.advance();
        }
        let (owner, name) = if *self.peek() == Tok::Dot {
            self.advance();
            let (method, _) = self.expect_member_name()?;
            (Some(first), method)
        } else {
            (None, first)
        };
        // `function Max<T>(...)`: a generic routine.
        let type_params = if owner.is_none() && *self.peek() == Tok::Lt {
            self.parse_type_params()?
        } else {
            Vec::new()
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
        let mods = self.skip_routine_modifiers(&[
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
            "platform",
            "experimental",
            "unimplemented",
            "varargs",
        ])?;
        let mut decl = ProcDecl::header(name, kind, params, return_type, span);
        decl.owner = owner;
        decl.type_params = type_params;
        decl.varargs = mods.iter().any(|m| m == "varargs");
        // `external ['lib'] [name 'sym'];`, then possibly more modifiers.
        if matches!(self.peek(), Tok::Ident(w) if w.eq_ignore_ascii_case("external")) {
            self.advance();
            let mut symbol = decl.name.clone();
            if matches!(self.peek(), Tok::StrLit(_) | Tok::Ident(_))
                && !matches!(self.peek(), Tok::Ident(w) if w.eq_ignore_ascii_case("name"))
            {
                self.advance();
            }
            if matches!(self.peek(), Tok::Ident(w) if w.eq_ignore_ascii_case("name")) {
                self.advance();
                match self.peek().clone() {
                    Tok::StrLit(sym) => {
                        self.advance();
                        symbol = sym;
                    }
                    other => {
                        return Err(ParseError {
                            message: format!("expected the external symbol's name, found {other}"),
                            span: self.span(),
                        });
                    }
                }
            }
            self.expect(&Tok::Semi)?;
            let more = self.skip_routine_modifiers(&["cdecl", "varargs", "inline", "overload"])?;
            decl.varargs |= more.iter().any(|m| m == "varargs");
            decl.external = Some(symbol);
            decl.is_forward = true;
        }
        Ok(decl)
    }

    /// Skip `modifier;` pairs whose (lower-case) name is in `names`.
    /// Returns the ones found.
    fn skip_routine_modifiers(&mut self, names: &[&str]) -> Result<Vec<String>, ParseError> {
        let mut found = Vec::new();
        while let Tok::Ident(word) = self.peek() {
            let lower = word.to_ascii_lowercase();
            let is_hint = lower == "deprecated";
            if !is_hint && !names.contains(&lower.as_str()) {
                break;
            }
            self.advance();
            // `deprecated 'message';`
            if is_hint && matches!(self.peek(), Tok::StrLit(_)) {
                self.advance();
            }
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
        while self.at_decl_name() {
            let span = self.span();
            // `generic TList<T> = ...` (objfpc) or `TList<T> = ...` (delphi).
            if matches!(self.peek(), Tok::Ident(w) if w.eq_ignore_ascii_case("generic"))
                && matches!(self.tokens.get(self.pos + 1), Some((Tok::Ident(_), _)))
            {
                self.advance();
            }
            let (decl_name, _) = self.expect_ident()?;
            let type_params = if *self.peek() == Tok::Lt {
                self.parse_type_params()?
            } else {
                Vec::new()
            };
            self.expect(&Tok::Eq)?;
            let mut ty = self.parse_type()?;
            if let PascalType::Enum { ref mut name, .. } = ty {
                *name = decl_name.clone();
            }
            self.skip_hint_directives();
            self.expect(&Tok::Semi)?;
            decls.push(TypeDecl {
                name: decl_name,
                ty,
                span,
                type_params,
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

    /// `<T, U: class; V>`: a generic's type parameters (constraints are
    /// accepted and ignored).
    fn parse_type_params(&mut self) -> Result<Vec<String>, ParseError> {
        self.expect(&Tok::Lt)?;
        let mut params = Vec::new();
        loop {
            let (name, _) = self.expect_ident()?;
            params.push(name);
            if *self.peek() == Tok::Colon {
                self.advance();
                // `class`, `record`, `constructor`, a type name, or several.
                loop {
                    match self.peek() {
                        Tok::Gt | Tok::Semi => break,
                        Tok::Comma
                            if !matches!(
                                self.tokens.get(self.pos + 1),
                                Some((Tok::Ident(_), _))
                            ) || matches!(
                                self.tokens.get(self.pos + 2),
                                Some((Tok::Colon | Tok::Comma | Tok::Gt | Tok::Semi, _))
                            ) =>
                        {
                            break;
                        }
                        _ => {
                            self.advance();
                        }
                    }
                }
            }
            match self.peek() {
                Tok::Comma | Tok::Semi => {
                    self.advance();
                }
                _ => break,
            }
        }
        self.expect(&Tok::Gt)?;
        Ok(params)
    }

    /// `<A, B>` after a generic's name: the type arguments.
    fn parse_type_args(&mut self) -> Result<Vec<PascalType>, ParseError> {
        self.expect(&Tok::Lt)?;
        let mut args = vec![self.parse_type()?];
        while *self.peek() == Tok::Comma {
            self.advance();
            args.push(self.parse_type()?);
        }
        self.expect(&Tok::Gt)?;
        Ok(args)
    }

    /// Whether `<` at `pos` starts type arguments followed by `.`, as in
    /// `TList<Integer>.Create` (Delphi mode).
    fn type_args_then_dot(&self, pos: usize) -> bool {
        self.type_args_then(pos, false)
    }

    /// Whether `<` at `pos` starts type arguments followed by `.` (or with
    /// `call`, by `(`: `Max<Integer>(1, 2)`).
    fn type_args_then(&self, pos: usize, call: bool) -> bool {
        let mut depth = 0;
        let mut p = pos;
        while let Some((t, _)) = self.tokens.get(p) {
            match t {
                Tok::Lt => depth += 1,
                Tok::Gt => {
                    depth -= 1;
                    if depth == 0 {
                        return match self.tokens.get(p + 1) {
                            Some((Tok::Dot, _)) => true,
                            Some((Tok::LParen, _)) => call,
                            _ => false,
                        };
                    }
                }
                Tok::Ident(_)
                | Tok::Comma
                | Tok::TyInteger
                | Tok::TyString
                | Tok::TyReal
                | Tok::TyChar
                | Tok::TyBoolean
                | Tok::Dot => {}
                _ => return false,
            }
            p += 1;
        }
        false
    }

    /// A specialization used in an expression: a hidden alias type for it,
    /// named so no program can spell it.
    fn hidden_specialization(&mut self, name: String, args: Vec<PascalType>, span: Span) -> String {
        let alias = format!("specialize${}", self.hidden_types.len() + 1);
        self.hidden_types.push(TypeDecl {
            name: alias.clone(),
            ty: PascalType::Specialize { name, args },
            span,
            type_params: Vec::new(),
        });
        alias
    }

    // ── const section ────────────────────────────────────

    /// Whether the next token is an identifier that is really a section
    /// keyword (`resourcestring`), ending the current section.
    fn at_decl_name(&self) -> bool {
        matches!(self.peek(), Tok::Ident(w) if !w.eq_ignore_ascii_case("resourcestring"))
    }

    fn parse_const_section(&mut self) -> Result<Vec<ConstDecl>, ParseError> {
        // `resourcestring` sections are string constants.
        if matches!(self.peek(), Tok::Ident(w) if w.eq_ignore_ascii_case("resourcestring")) {
            self.advance();
        } else {
            self.expect(&Tok::Const)?;
        }
        let mut decls = Vec::new();
        while self.at_decl_name() {
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
            let value = if ty.is_some() {
                self.parse_initializer()?
            } else {
                self.parse_expr()?
            };
            self.skip_hint_directives();
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

    /// A typed constant's value: an expression, `(Field: value; ...)` for
    /// a record, or `(value, value, ...)` for an array, nested as needed.
    fn parse_initializer(&mut self) -> Result<Expr, ParseError> {
        if *self.peek() != Tok::LParen {
            return self.parse_expr();
        }
        let span = self.span();
        // `()`: a record with every field left zero.
        if matches!(self.tokens.get(self.pos + 1), Some((Tok::RParen, _))) {
            self.advance();
            self.advance();
            return Ok(Expr::RecordInit {
                fields: Vec::new(),
                span,
            });
        }
        let is_record = matches!(self.tokens.get(self.pos + 1), Some((Tok::Ident(_), _)))
            && matches!(self.tokens.get(self.pos + 2), Some((Tok::Colon, _)));
        if is_record {
            self.advance();
            let mut fields = Vec::new();
            while *self.peek() != Tok::RParen {
                let (name, _) = self.expect_ident()?;
                self.expect(&Tok::Colon)?;
                fields.push((name, self.parse_initializer()?));
                if *self.peek() == Tok::Semi {
                    self.advance();
                } else {
                    break;
                }
            }
            self.expect(&Tok::RParen)?;
            return Ok(Expr::RecordInit { fields, span });
        }
        // `(a, b, ...)` is an array; a single `(e)` is just a parenthesised
        // expression, possibly the start of a longer one.
        let start = self.pos;
        self.advance();
        let mut elements = vec![self.parse_initializer()?];
        while *self.peek() == Tok::Comma {
            self.advance();
            elements.push(self.parse_initializer()?);
        }
        if elements.len() == 1 {
            self.pos = start;
            return self.parse_expr();
        }
        self.expect(&Tok::RParen)?;
        Ok(Expr::ArrayInit { elements, span })
    }

    /// Skip FPC hint directives after a declaration: `deprecated
    /// ['message']`, `platform`, `experimental`, `unimplemented`,
    /// `library`.
    fn skip_hint_directives(&mut self) {
        while let Tok::Ident(word) = self.peek() {
            let word = word.to_ascii_lowercase();
            if !matches!(
                word.as_str(),
                "deprecated" | "platform" | "experimental" | "unimplemented" | "library"
            ) {
                return;
            }
            self.advance();
            if word == "deprecated" && matches!(self.peek(), Tok::StrLit(_)) {
                self.advance();
            }
        }
    }

    // ── var section ──────────────────────────────────────

    /// A `var` section: its variables, and those it initializes (`X:
    /// Integer = 5;`), which are typed constants (writable, as FPC's are).
    fn parse_var_section(&mut self) -> Result<(Vec<VarDecl>, Vec<ConstDecl>), ParseError> {
        self.expect(&Tok::Var)?;
        let mut decls = Vec::new();
        let mut initialized = Vec::new();
        // Parse var declarations until we see 'begin' or another section keyword
        while self.at_decl_name() {
            let decl = self.parse_var_decl()?;
            if *self.peek() == Tok::Eq && decl.names.len() == 1 && decl.absolute.is_none() {
                self.advance();
                let value = self.parse_initializer()?;
                self.skip_hint_directives();
                initialized.push(ConstDecl {
                    name: decl.names[0].clone(),
                    ty: Some(decl.ty),
                    value,
                    span: decl.span,
                });
            } else {
                decls.push(decl);
            }
            self.expect(&Tok::Semi)?;
        }
        if decls.is_empty() && initialized.is_empty() {
            return Err(ParseError {
                message: "expected at least one variable declaration after 'var'".into(),
                span: self.span(),
            });
        }
        Ok((decls, initialized))
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
        // `x: T absolute y`: x shares y's memory.
        let absolute = if matches!(self.peek(), Tok::Ident(w) if w.eq_ignore_ascii_case("absolute"))
        {
            self.advance();
            Some(self.expect_ident()?.0)
        } else {
            None
        };
        self.skip_hint_directives();
        Ok(VarDecl {
            names,
            ty,
            absolute,
            span,
        })
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
            // `file of T`, or an untyped `file` read and written in blocks.
            let elem = if *self.peek() == Tok::KwOf {
                self.advance();
                self.parse_type()?
            } else {
                PascalType::Untyped
            };
            return Ok(PascalType::File {
                elem: Box::new(elem),
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
        if matches!(self.peek(), Tok::KwObject | Tok::KwInterface) {
            return self.parse_object_type();
        }
        if matches!(self.peek(), Tok::Ident(w) if w.eq_ignore_ascii_case("class")) {
            // `class of T`: a class reference type.
            if matches!(self.tokens.get(self.pos + 1), Some((Tok::KwOf, _))) {
                self.advance();
                self.advance();
                let (name, _) = self.expect_ident()?;
                return Ok(PascalType::ClassOf(name));
            }
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
                let int32 = self.int32[self.pos];
                self.advance();
                if int32 {
                    return Ok(PascalType::SizedInt {
                        bits: 32,
                        signed: true,
                    });
                }
                Ok(PascalType::Integer)
            }
            Tok::TyReal => {
                self.advance();
                Ok(PascalType::Real)
            }
            Tok::TyString => {
                let short = self.short_strings[self.pos];
                self.advance();
                // `string[N]` is always a short string of at most N
                // characters (255 when N isn't a plain number).
                if *self.peek() == Tok::LBracket {
                    self.advance();
                    let n = self.parse_const_bound()?;
                    self.expect(&Tok::RBracket)?;
                    let max = n.as_int().filter(|n| (1..=255).contains(n)).unwrap_or(255);
                    return Ok(PascalType::ShortString { max: max as u32 });
                }
                Ok(if short {
                    PascalType::ShortString { max: 255 }
                } else {
                    PascalType::String
                })
            }
            Tok::TyBoolean => {
                self.advance();
                Ok(PascalType::Boolean)
            }
            Tok::TyChar => {
                self.advance();
                Ok(PascalType::Char)
            }
            // `PString` points at whatever `string` means here.
            Tok::Ident(name)
                if name.eq_ignore_ascii_case("pstring") && self.short_strings[self.pos] =>
            {
                self.advance();
                Ok(PascalType::Pointer(Box::new(PascalType::ShortString {
                    max: 255,
                })))
            }
            Tok::Ident(w)
                if w.eq_ignore_ascii_case("specialize")
                    && matches!(self.tokens.get(self.pos + 1), Some((Tok::Ident(_), _))) =>
            {
                self.advance();
                let (name, _) = self.expect_ident()?;
                let args = self.parse_type_args()?;
                Ok(PascalType::Specialize { name, args })
            }
            Tok::Ident(name) if matches!(self.tokens.get(self.pos + 1), Some((Tok::Lt, _))) => {
                let name = name.clone();
                self.advance();
                let args = self.parse_type_args()?;
                Ok(PascalType::Specialize { name, args })
            }
            Tok::Ident(name) => {
                let mut name = name.clone();
                self.advance();
                // `Objects.TRect` keeps its qualifier until the unit
                // scopes are resolved; only the last two parts matter.
                while *self.peek() == Tok::Dot
                    && matches!(self.tokens.get(self.pos + 1), Some((Tok::Ident(_), _)))
                {
                    self.advance();
                    let part = self.expect_ident()?.0;
                    let unit = name.rsplit('.').next().unwrap_or(&name).to_string();
                    name = format!("{unit}.{part}");
                }
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
        // `array of T`: a dynamic array (an open array as a parameter);
        // `array of const`: an open array of TVarRec.
        if *self.peek() == Tok::KwOf {
            self.advance();
            if *self.peek() == Tok::Const {
                self.advance();
                return Ok(PascalType::OpenArray(Box::new(PascalType::Named(
                    "TVarRec".to_string(),
                ))));
            }
            let elem = self.parse_type()?;
            return Ok(PascalType::DynArray(Box::new(elem)));
        }
        self.expect(&Tok::LBracket)?;

        let mut dimensions = Vec::new();
        loop {
            // An index type (`array[Boolean]`, `array[TColor]`) instead of
            // a range: its bounds are `low(T)..high(T)`.
            let index_type = match (self.peek(), self.tokens.get(self.pos + 1)) {
                (Tok::TyBoolean, _) => Some("boolean".to_string()),
                (Tok::TyChar, _) => Some("char".to_string()),
                (Tok::Ident(name), Some((Tok::RBracket | Tok::Comma, _))) => Some(name.clone()),
                _ => None,
            };
            if let Some(type_name) = index_type {
                self.advance();
                let bound = |f: &str| {
                    ConstExpr::Call(f.to_string(), Box::new(ConstExpr::Name(type_name.clone())))
                };
                dimensions.push((bound("low"), bound("high")));
                if *self.peek() != Tok::Comma {
                    break;
                }
                self.advance();
                continue;
            }
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

    /// `object [(Parent)] fields and method headers end`, or `class
    /// [(Parent, Interfaces...)] ... end`, with optional `private` /
    /// `public` / `protected` / `published` sections (accepted, not
    /// enforced). A class may also be forward-declared (`TFoo = class;`)
    /// or declared with no members (`EFoo = class(Exception);`), and may
    /// have class methods and properties.
    fn parse_object_type(&mut self) -> Result<PascalType, ParseError> {
        let is_interface = *self.peek() == Tok::KwInterface;
        let is_class = !is_interface && *self.peek() != Tok::KwObject;
        self.advance();
        let mut parent = None;
        let mut parent_args = Vec::new();
        let mut interfaces = Vec::new();
        if *self.peek() == Tok::LParen {
            self.advance();
            if matches!(self.peek(), Tok::Ident(w) if w.eq_ignore_ascii_case("specialize")) {
                self.advance();
            }
            let (first, _) = self.expect_ident()?;
            parent = Some(first);
            if *self.peek() == Tok::Lt {
                parent_args = self.parse_type_args()?;
            }
            // The interfaces it implements (`IBox<Integer>` by a hidden
            // alias).
            while *self.peek() == Tok::Comma {
                self.advance();
                if matches!(self.peek(), Tok::Ident(w) if w.eq_ignore_ascii_case("specialize")) {
                    self.advance();
                }
                let (name, span) = self.expect_ident()?;
                if *self.peek() == Tok::Lt {
                    let args = self.parse_type_args()?;
                    interfaces.push(self.hidden_specialization(name, args, span));
                } else {
                    interfaces.push(name);
                }
            }
            self.expect(&Tok::RParen)?;
        }
        // An interface's GUID: `['{...}']`.
        let mut guid = None;
        if is_interface && *self.peek() == Tok::LBracket {
            self.advance();
            if let Tok::StrLit(text) = self.peek().clone() {
                guid = Some(text);
                self.advance();
            } else {
                self.parse_expr()?;
            }
            self.expect(&Tok::RBracket)?;
        }
        let mut obj = ObjectType {
            parent,
            fields: Vec::new(),
            methods: Vec::new(),
            is_class,
            is_forward: false,
            properties: Vec::new(),
            class_vars: Vec::new(),
            parent_args,
            is_interface,
            interfaces,
            guid,
        };
        // After `class var`, fields are class variables until the next
        // section.
        let mut in_class_vars = false;
        if (is_class || is_interface) && *self.peek() == Tok::Semi {
            obj.is_forward = obj.parent.is_none();
            return Ok(PascalType::Object(Box::new(obj)));
        }
        let is_word = |t: &Tok, w: &str| matches!(t, Tok::Ident(x) if x.eq_ignore_ascii_case(w));
        loop {
            // `class procedure` / `class function`: a class method.
            let class_method = is_class
                && is_word(self.peek(), "class")
                && matches!(
                    self.tokens.get(self.pos + 1),
                    Some((
                        Tok::KwProcedure | Tok::KwFunction | Tok::KwConstructor | Tok::KwDestructor,
                        _
                    ))
                );
            if class_method {
                self.advance();
            }
            if !matches!(self.peek(), Tok::Ident(_)) || self.at_routine() {
                in_class_vars = false;
            }
            match self.peek().clone() {
                Tok::End => {
                    self.advance();
                    break;
                }
                _ if self.at_routine() => {
                    let heading = self.parse_routine_heading()?;
                    let mods = self.skip_routine_modifiers(&[
                        "virtual",
                        "dynamic",
                        "override",
                        "abstract",
                        "inline",
                        "overload",
                        "static",
                        "reintroduce",
                        "final",
                        "cdecl",
                    ])?;
                    let has = |m: &str| mods.iter().any(|x| x == m);
                    obj.methods.push(MethodHeader {
                        name: heading.name,
                        kind: heading.kind,
                        params: heading.params,
                        return_type: heading.return_type,
                        is_virtual: has("virtual") || has("dynamic") || has("override"),
                        is_abstract: has("abstract"),
                        is_override: has("override"),
                        is_class_method: class_method,
                        span: heading.span,
                    });
                }
                Tok::Ident(word)
                    if word.eq_ignore_ascii_case("property")
                        && (is_class || is_interface)
                        && matches!(self.tokens.get(self.pos + 1), Some((Tok::Ident(_), _))) =>
                {
                    self.advance();
                    if let Some(prop) = self.parse_property()? {
                        obj.properties.push(prop);
                    }
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
                    in_class_vars = false;
                }
                // `var` before a group of fields.
                Tok::Var if is_class => {
                    self.advance();
                    in_class_vars = false;
                }
                Tok::Ident(word)
                    if is_class
                        && word.eq_ignore_ascii_case("class")
                        && matches!(self.tokens.get(self.pos + 1), Some((Tok::Var, _))) =>
                {
                    self.advance();
                    self.advance();
                    in_class_vars = true;
                }
                Tok::Ident(_) => {
                    let decl = self.parse_var_decl()?;
                    for name in decl.names {
                        if in_class_vars {
                            obj.class_vars.push((name, decl.ty.clone()));
                        } else {
                            obj.fields.push((name, decl.ty.clone()));
                        }
                    }
                    if *self.peek() == Tok::Semi {
                        self.advance();
                    }
                }
                _ => {
                    return Err(ParseError {
                        message: format!(
                            "expected a field or method in {}, found {}",
                            if is_class { "class" } else { "object" },
                            self.peek()
                        ),
                        span: self.span(),
                    });
                }
            }
        }
        Ok(PascalType::Object(Box::new(obj)))
    }

    /// After `property`: `Name [Params]: Type read R write W [index N]
    /// [default V] [stored S] [nodefault]; [default;]`. A redeclaration
    /// that only changes visibility (`property Name;`) gives `None`.
    fn parse_property(&mut self) -> Result<Option<PropertyDecl>, ParseError> {
        let span = self.span();
        let (name, _) = self.expect_ident()?;
        let mut params = Vec::new();
        if *self.peek() == Tok::LBracket {
            self.advance();
            params.push(self.parse_param_group()?);
            while *self.peek() == Tok::Semi {
                self.advance();
                params.push(self.parse_param_group()?);
            }
            self.expect(&Tok::RBracket)?;
        }
        let ty = if *self.peek() == Tok::Colon {
            self.advance();
            Some(self.parse_type()?)
        } else {
            None
        };
        let mut prop = PropertyDecl {
            name,
            params,
            ty: ty.clone().unwrap_or(PascalType::Integer),
            read: None,
            write: None,
            index: None,
            is_default: false,
            span,
        };
        loop {
            match self.peek().clone() {
                Tok::KwRead => {
                    self.advance();
                    prop.read = Some(self.parse_property_accessor()?);
                }
                Tok::KwWrite => {
                    self.advance();
                    prop.write = Some(self.parse_property_accessor()?);
                }
                Tok::Ident(w) if w.eq_ignore_ascii_case("index") => {
                    self.advance();
                    let negative = *self.peek() == Tok::Minus;
                    if negative {
                        self.advance();
                    }
                    let n = self.parse_int_literal()?;
                    prop.index = Some(if negative { -n } else { n });
                }
                // `default V` / `stored S`: streaming hints, skipped.
                Tok::Ident(w)
                    if (w.eq_ignore_ascii_case("default") || w.eq_ignore_ascii_case("stored"))
                        && !matches!(self.tokens.get(self.pos + 1), Some((Tok::Semi, _))) =>
                {
                    self.advance();
                    self.parse_expr()?;
                }
                Tok::Ident(w) if w.eq_ignore_ascii_case("nodefault") => {
                    self.advance();
                }
                _ => break,
            }
        }
        self.expect(&Tok::Semi)?;
        if matches!(self.peek(), Tok::Ident(w) if w.eq_ignore_ascii_case("default"))
            && matches!(self.tokens.get(self.pos + 1), Some((Tok::Semi, _)))
        {
            self.advance();
            self.advance();
            prop.is_default = true;
        }
        Ok(ty.map(|_| prop))
    }

    /// A property's `read`/`write` target: a field or method name, or a
    /// field path such as `FPoint.X`.
    fn parse_property_accessor(&mut self) -> Result<String, ParseError> {
        let (mut name, _) = self.expect_member_name()?;
        while *self.peek() == Tok::Dot {
            self.advance();
            let (part, _) = self.expect_member_name()?;
            name = format!("{name}.{part}");
        }
        Ok(name)
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
        // `case Tag: T of` or, with no tag field, `case T of`.
        let (tag_name, tag_type) = if matches!(self.tokens.get(self.pos + 1), Some((Tok::Colon, _)))
        {
            let (tag_name, _) = self.expect_ident()?;
            self.expect(&Tok::Colon)?;
            (tag_name, self.parse_type()?)
        } else {
            (String::new(), self.parse_type()?)
        };
        self.expect(&Tok::KwOf)?;

        let mut variants = Vec::new();
        while *self.peek() != Tok::End && *self.peek() != Tok::RParen {
            let mut values = Vec::new();
            values.push(self.parse_const_bound()?);
            while *self.peek() == Tok::Comma {
                self.advance();
                values.push(self.parse_const_bound()?);
            }
            self.expect(&Tok::Colon)?;
            self.expect(&Tok::LParen)?;
            let mut vfields = Vec::new();
            while *self.peek() != Tok::RParen {
                // A nested variant part: kept as an unnamed field holding an
                // anonymous variant record, whose fields are found by name.
                if *self.peek() == Tok::KwCase {
                    let nested = self.parse_variant_part()?;
                    vfields.push((
                        String::new(),
                        PascalType::Record {
                            fields: Vec::new(),
                            variant: Some(Box::new(nested)),
                            object: None,
                        },
                    ));
                    continue;
                }
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
        if decl.external.is_some() {
            return Ok(decl);
        }

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
        let params = decl
            .params
            .iter()
            .flat_map(|g| g.names.iter().cloned())
            .collect();
        self.routines.push((decl.name.clone(), params));
        let body = self.parse_block();
        self.routines.pop();
        decl.body = body?;
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
                is_const: false,
                mode: ParamMode::Value,
                names: vec![pname],
                ty,
                default: None,
            });
        }

        // `var`, `out` and `constref` pass by reference; `const` passes a
        // typed value by value and an untyped one by reference.
        let keyword = match self.peek() {
            Tok::Var => Some("var"),
            Tok::Const => Some("const"),
            Tok::Ident(w)
                if w.eq_ignore_ascii_case("out")
                    && matches!(self.tokens.get(self.pos + 1), Some((Tok::Ident(_), _))) =>
            {
                Some("out")
            }
            Tok::Ident(w) if w.eq_ignore_ascii_case("constref") => Some("constref"),
            _ => None,
        };
        if keyword.is_some() {
            self.advance();
        }
        let mut names = Vec::new();
        let (first, _) = self.expect_ident()?;
        names.push(first);
        while *self.peek() == Tok::Comma {
            self.advance();
            let (name, _) = self.expect_ident()?;
            names.push(name);
        }
        if *self.peek() != Tok::Colon && keyword.is_some() {
            // Untyped: `var Buf` / `const Data`, always by reference.
            return Ok(ParamGroup {
                mode: ParamMode::Var,
                names,
                ty: PascalType::Untyped,
                default: None,
                is_const: false,
            });
        }
        self.expect(&Tok::Colon)?;
        let ty = match self.parse_type()? {
            // `A: array of T` in a parameter list is an open array.
            PascalType::DynArray(elem) => PascalType::OpenArray(elem),
            other => other,
        };
        let mode = match keyword {
            // An open array is passed as its address and bounds either way.
            _ if matches!(ty, PascalType::OpenArray(_)) => ParamMode::Value,
            Some("var" | "out" | "constref") => ParamMode::Var,
            _ => ParamMode::Value,
        };
        // `X: T = Value`.
        let default = if *self.peek() == Tok::Eq {
            self.advance();
            Some(DefaultValue(self.parse_expr()?))
        } else {
            None
        };
        Ok(ParamGroup {
            mode,
            names,
            ty,
            default,
            is_const: keyword == Some("const"),
        })
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
        // `is nested`: a closure over a local routine; `of object`: a
        // method pointer (parsed, treated like a plain procedural value).
        let mut nested = false;
        let mut of_object = false;
        if matches!(self.peek(), Tok::Ident(w) if w.eq_ignore_ascii_case("is"))
            && matches!(self.tokens.get(self.pos + 1), Some((Tok::Ident(w), _)) if w.eq_ignore_ascii_case("nested"))
        {
            self.advance();
            self.advance();
            nested = true;
        } else if *self.peek() == Tok::KwOf
            && matches!(self.tokens.get(self.pos + 1), Some((Tok::KwObject, _)))
        {
            self.advance();
            self.advance();
            of_object = true;
        }
        Ok(PascalType::Proc {
            params,
            return_type,
            nested,
            of_object,
        })
    }

    /// Parse one parameter group inside a procedural type, `[var] a, b: T`,
    /// returning `T` once per name.
    fn parse_proc_param_types(&mut self) -> Result<Vec<PascalType>, ParseError> {
        // `var`, `out` and `constref` pass by reference, as does an untyped
        // `const`.
        let keyword = match self.peek() {
            Tok::Var => Some("var"),
            Tok::Const => Some("const"),
            Tok::Ident(w)
                if (w.eq_ignore_ascii_case("out") || w.eq_ignore_ascii_case("constref"))
                    && matches!(self.tokens.get(self.pos + 1), Some((Tok::Ident(_), _))) =>
            {
                Some("var")
            }
            _ => None,
        };
        if keyword.is_some() {
            self.advance();
        }
        let mut count = 1;
        let _ = self.expect_ident()?;
        while *self.peek() == Tok::Comma {
            self.advance();
            let _ = self.expect_ident()?;
            count += 1;
        }
        if *self.peek() != Tok::Colon && keyword.is_some() {
            return Ok(vec![
                PascalType::VarParam(Box::new(PascalType::Untyped));
                count
            ]);
        }
        self.expect(&Tok::Colon)?;
        let ty = self.parse_type()?;
        let ty = if keyword == Some("var") {
            PascalType::VarParam(Box::new(ty))
        } else {
            ty
        };
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

    /// Whether the next token is `word` (`try`, `raise`) starting a
    /// statement, rather than a variable of that name being assigned.
    fn at_word_statement(&self, current: &str, word: &str) -> bool {
        current.eq_ignore_ascii_case(word)
            && !matches!(
                self.tokens.get(self.pos + 1),
                Some((Tok::Assign | Tok::Dot | Tok::LBracket | Tok::Caret, _))
            )
    }

    /// Whether the next token ends a section of a `try` statement:
    /// `except` or `finally`.
    fn at_try_section_end(&self) -> bool {
        matches!(self.peek(), Tok::Ident(w)
            if w.eq_ignore_ascii_case("except") || w.eq_ignore_ascii_case("finally"))
    }

    /// Statements separated by semicolons, up to (not including) a token
    /// for which `stop` holds.
    fn parse_statements_until(
        &mut self,
        stop: impl Fn(&Self) -> bool,
    ) -> Result<Vec<Statement>, ParseError> {
        let mut statements = Vec::new();
        loop {
            if stop(self) {
                break;
            }
            let stmt = self.parse_statement()?;
            let is_label = matches!(&stmt, Statement::Label { .. });
            statements.push(stmt);
            if is_label && !stop(self) && *self.peek() != Tok::Semi {
                continue;
            }
            if *self.peek() == Tok::Semi {
                self.advance();
            } else {
                break;
            }
        }
        Ok(statements)
    }

    /// `try S... except [on [E:] T do S; ...] [else S...] end`, or
    /// `try S... finally S... end`.
    fn parse_try(&mut self) -> Result<Statement, ParseError> {
        let span = self.span();
        self.advance();
        let body_statements = self.parse_statements_until(|p| p.at_try_section_end())?;
        let body = Block {
            statements: body_statements,
            span,
            end_span: self.span(),
        };
        let is_finally = matches!(self.peek(), Tok::Ident(w) if w.eq_ignore_ascii_case("finally"));
        if !self.at_try_section_end() {
            return Err(ParseError {
                message: format!("expected 'except' or 'finally', found {}", self.peek()),
                span: self.span(),
            });
        }
        let section_span = self.span();
        self.advance();
        let at_end = |p: &Self| *p.peek() == Tok::End;
        let handler = if is_finally {
            let statements = self.parse_statements_until(at_end)?;
            TryHandler::Finally(Block {
                statements,
                span: section_span,
                end_span: self.span(),
            })
        } else if self.at_on_handler() {
            let mut handlers = Vec::new();
            let mut otherwise = None;
            while self.at_on_handler() {
                let hspan = self.span();
                self.advance();
                let (first, _) = self.expect_ident()?;
                let (var, class) = if *self.peek() == Tok::Colon {
                    self.advance();
                    let (class, _) = self.expect_ident()?;
                    (Some(first), class)
                } else {
                    (None, first)
                };
                self.expect(&Tok::Do)?;
                let stmt = self.parse_statement()?;
                handlers.push(ExceptHandler {
                    var,
                    class,
                    body: Block {
                        statements: vec![stmt],
                        span: hspan,
                        end_span: self.span(),
                    },
                    span: hspan,
                });
                if *self.peek() == Tok::Semi {
                    self.advance();
                }
            }
            if *self.peek() == Tok::Else {
                let espan = self.span();
                self.advance();
                let statements = self.parse_statements_until(at_end)?;
                otherwise = Some(Block {
                    statements,
                    span: espan,
                    end_span: self.span(),
                });
            }
            TryHandler::Except {
                handlers,
                otherwise,
            }
        } else {
            let statements = self.parse_statements_until(at_end)?;
            TryHandler::Except {
                handlers: Vec::new(),
                otherwise: Some(Block {
                    statements,
                    span: section_span,
                    end_span: self.span(),
                }),
            }
        };
        self.expect(&Tok::End)?;
        Ok(Statement::Try {
            body,
            handler,
            span,
        })
    }

    /// `on Name: Class do` / `on Class do` in an `except` section.
    fn at_on_handler(&self) -> bool {
        matches!(self.peek(), Tok::Ident(w) if w.eq_ignore_ascii_case("on"))
            && matches!(self.tokens.get(self.pos + 1), Some((Tok::Ident(_), _)))
            && matches!(
                self.tokens.get(self.pos + 2),
                Some((Tok::Colon | Tok::Do, _))
            )
    }

    /// `raise [Expr [at Address]]`.
    fn parse_raise(&mut self) -> Result<Statement, ParseError> {
        let span = self.span();
        self.advance();
        if matches!(self.peek(), Tok::Semi | Tok::End | Tok::Else | Tok::Until)
            || self.at_try_section_end()
        {
            return Ok(Statement::Raise {
                exception: None,
                span,
            });
        }
        let exception = self.parse_expr()?;
        // `at Address`: where the exception is reported; ignored.
        if matches!(self.peek(), Tok::Ident(w) if w.eq_ignore_ascii_case("at")) {
            self.advance();
            self.parse_expr()?;
        }
        Ok(Statement::Raise {
            exception: Some(exception),
            span,
        })
    }

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

        // An empty statement: `while Busy do;`, `begin ; end`.
        if matches!(self.peek(), Tok::Semi | Tok::End | Tok::Else | Tok::Until)
            || self.at_try_section_end()
        {
            let span = self.span();
            return Ok(Statement::Block(Block {
                statements: Vec::new(),
                span,
                end_span: span,
            }));
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
            // `specialize Show<Integer>(X);`, `Show<Integer>(X);`.
            Tok::Ident(w)
                if (w.eq_ignore_ascii_case("specialize")
                    && matches!(self.tokens.get(self.pos + 2), Some((Tok::Lt, _))))
                    || (matches!(self.tokens.get(self.pos + 1), Some((Tok::Lt, _)))
                        && self.type_args_then(self.pos + 1, true)) =>
            {
                let span = self.span();
                match self.parse_primary()? {
                    Expr::Call { name, args, span } => Ok(Statement::ProcCall { name, args, span }),
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
                    _ => Err(ParseError {
                        message: "expected a call".into(),
                        span,
                    }),
                }
            }
            Tok::Ident(w) if self.at_word_statement(w, "try") => self.parse_try(),
            Tok::Ident(w) if self.at_word_statement(w, "raise") => self.parse_raise(),
            Tok::KwInherited => {
                let span = self.span();
                self.advance();
                // A bare `inherited;` calls the same method with the same
                // arguments.
                if matches!(self.peek(), Tok::Semi | Tok::End | Tok::Else | Tok::Until)
                    && let Some((method, params)) = self.routines.last().cloned()
                {
                    let args = params.into_iter().map(|p| Expr::Var(p, span)).collect();
                    return Ok(Statement::Inherited { method, args, span });
                }
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

    /// The rest of a statement whose start, `value`, is a call's result:
    /// `.Field`, `.Method(args)`, `[index]` and `^` steps, then a method
    /// call or an assignment.
    fn parse_chained_call_statement(
        &mut self,
        mut value: Expr,
        span: Span,
    ) -> Result<Statement, ParseError> {
        loop {
            match self.peek() {
                Tok::Dot => {
                    self.advance();
                    let (field, _) = self.expect_member_name()?;
                    value = if *self.peek() == Tok::LParen {
                        let args = self.parse_call_args()?;
                        Expr::MethodCall {
                            object: Box::new(value),
                            method: field,
                            args,
                            span,
                        }
                    } else {
                        Expr::FieldAccess {
                            record: Box::new(value),
                            field,
                            span,
                        }
                    };
                }
                Tok::LBracket => {
                    self.advance();
                    loop {
                        let index = self.parse_expr()?;
                        value = Expr::Index {
                            array: Box::new(value),
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
                Tok::Caret => {
                    self.advance();
                    value = Expr::Deref(Box::new(value), span);
                }
                _ => break,
            }
        }
        if *self.peek() == Tok::Assign {
            self.advance();
            let expr = self.parse_expr()?;
            return Ok(Statement::PlaceAssignment {
                target: value,
                expr,
                span,
            });
        }
        match value {
            Expr::MethodCall {
                object,
                method,
                args,
                ..
            } => Ok(Statement::MethodCall {
                object: *object,
                method,
                args,
                span,
            }),
            Expr::FieldAccess { record, field, .. } => Ok(Statement::MethodCall {
                object: *record,
                method: field,
                args: Vec::new(),
                span,
            }),
            _ => Err(ParseError {
                message: "expected a method call or an assignment".into(),
                span,
            }),
        }
    }

    fn parse_ident_statement(&mut self) -> Result<Statement, ParseError> {
        let span = self.span();
        let (name, _) = self.expect_ident()?;

        // `str(x:width:precision, s)`: the value takes write-style formats.
        if name.eq_ignore_ascii_case("str") && *self.peek() == Tok::LParen {
            self.advance();
            let value = self.parse_write_arg()?;
            self.expect(&Tok::Comma)?;
            let target = self.parse_expr()?;
            self.expect(&Tok::RParen)?;
            return Ok(Statement::Str {
                value,
                target,
                span,
            });
        }

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
                        // `TFoo.Create(...).Free`: the chain goes on.
                        if matches!(self.peek(), Tok::Dot | Tok::LBracket | Tok::Caret) {
                            let call = Expr::MethodCall {
                                object: Box::new(object),
                                method: field,
                                args,
                                span,
                            };
                            return self.parse_chained_call_statement(call, span);
                        }
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
            // `TProc(p)(args);`: call the procedural value a cast gives.
            if *self.peek() == Tok::LParen {
                let call = self.parse_postfix(Expr::Call { name, args, span }, span)?;
                return Ok(Statement::IndirectCall { call, span });
            }
            // `Pointer(P) := nil`: assignment through a type cast.
            if *self.peek() == Tok::Assign {
                self.advance();
                let expr = self.parse_expr()?;
                return Ok(Statement::PlaceAssignment {
                    target: Expr::Call { name, args, span },
                    expr,
                    span,
                });
            }
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
        if *self.peek() == Tok::KwIn {
            self.advance();
            let collection = self.parse_expr()?;
            self.expect(&Tok::Do)?;
            let body = match self.parse_statement()? {
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
            return Ok(Statement::ForIn {
                var,
                collection,
                body,
                span,
            });
        }
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
            // `else`, or FPC's `otherwise`.
            let is_else = *self.peek() == Tok::Else
                || matches!(self.peek(), Tok::Ident(w) if w.eq_ignore_ascii_case("otherwise"));
            if is_else {
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
        // `with a, b do s` means `with a do with b do s`.
        let mut targets = vec![self.parse_expr()?];
        while *self.peek() == Tok::Comma {
            self.advance();
            targets.push(self.parse_expr()?);
        }
        self.expect(&Tok::Do)?;
        let body_stmt = self.parse_statement()?;
        let mut body = match body_stmt {
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
        while let Some(target) = targets.pop() {
            let statement = Statement::With { target, body, span };
            if targets.is_empty() {
                return Ok(statement);
            }
            body = Block {
                span,
                end_span: span,
                statements: vec![statement],
            };
        }
        unreachable!("with has at least one target")
    }

    fn parse_new(&mut self) -> Result<Statement, ParseError> {
        let span = self.span();
        self.advance(); // consume 'new'
        self.expect(&Tok::LParen)?;
        let target = self.parse_expr()?;
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
        let target = self.parse_expr()?;
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

    /// The class after `is` / `as`: a name, or a specialization
    /// (`TBox<string>`, `specialize TBox<string>`).
    fn parse_class_operand(&mut self) -> Result<Expr, ParseError> {
        let span = self.span();
        if matches!(self.peek(), Tok::Ident(w) if w.eq_ignore_ascii_case("specialize")) {
            self.advance();
        }
        if matches!(self.peek(), Tok::Ident(_))
            && matches!(self.tokens.get(self.pos + 1), Some((Tok::Lt, _)))
        {
            let (name, _) = self.expect_ident()?;
            let args = self.parse_type_args()?;
            let alias = self.hidden_specialization(name, args, span);
            return Ok(Expr::Var(alias, span));
        }
        self.parse_unary()
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
                Tok::Ident(w) if w.eq_ignore_ascii_case("is") => BinOp::Is,
                _ => break,
            };
            let span = self.span();
            self.advance();
            let right = if op == BinOp::Is {
                self.parse_class_operand()?
            } else {
                self.parse_additive()?
            };
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
                Tok::Ident(w) if w.eq_ignore_ascii_case("as") => BinOp::As,
                _ => break,
            };
            let span = self.span();
            self.advance();
            let right = if op == BinOp::As {
                self.parse_class_operand()?
            } else {
                self.parse_unary()?
            };
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
                // `Cast(p)(args)`: call the procedural value just computed.
                Tok::LParen if matches!(expr, Expr::Call { .. } | Expr::IndirectCall { .. }) => {
                    let args = self.parse_call_args()?;
                    expr = Expr::IndirectCall {
                        callee: Box::new(expr),
                        args,
                        span,
                    };
                }
                _ => return Ok(expr),
            }
        }
    }

    fn parse_primary(&mut self) -> Result<Expr, ParseError> {
        // `N := Read(Buf, Count)` in a stream's method: in an expression,
        // `read` and `write` can only be functions of the program's own.
        if matches!(self.peek(), Tok::KwRead | Tok::KwWrite)
            && let Some(word) = Self::soft_keyword(self.peek())
        {
            self.tokens[self.pos].0 = Tok::Ident(word.to_string());
        }
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
            Tok::WideCharLit(c) => {
                let span = self.span();
                self.advance();
                Ok(Expr::WideCharLit(c, span))
            }
            Tok::StrLit(s) => {
                let span = self.span();
                self.advance();
                Ok(Expr::StrLit(s, span))
            }
            Tok::CodePageStrLit(bytes, text) => {
                let span = self.span();
                self.advance();
                Ok(Expr::CodePageStrLit(bytes, text, span))
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
            // `^M^J`: control characters (`^A` is #1). Several in a row
            // form a string.
            Tok::Caret
                if matches!(self.tokens.get(self.pos + 1), Some((Tok::Ident(w), _))
                    if w.len() == 1 && w.as_bytes()[0].is_ascii_alphabetic()) =>
            {
                let span = self.span();
                let mut text = String::new();
                while *self.peek() == Tok::Caret
                    && let Some((Tok::Ident(w), _)) = self.tokens.get(self.pos + 1)
                    && w.len() == 1
                    && w.as_bytes()[0].is_ascii_alphabetic()
                {
                    let code = w.as_bytes()[0].to_ascii_uppercase() - b'@';
                    text.push(char::from(code));
                    self.advance();
                    self.advance();
                }
                Ok(if text.chars().count() == 1 {
                    Expr::CharLit(text.as_bytes()[0], span)
                } else {
                    Expr::StrLit(text, span)
                })
            }
            // `String(x)`: a cast to string.
            Tok::TyString if matches!(self.tokens.get(self.pos + 1), Some((Tok::LParen, _))) => {
                let span = self.span();
                self.advance();
                let args = self.parse_call_args()?;
                Ok(Expr::Call {
                    name: "string".to_string(),
                    args,
                    span,
                })
            }
            // A type name used as a value, as in `SizeOf(Real)`.
            Tok::TyInteger | Tok::TyReal | Tok::TyChar | Tok::TyBoolean | Tok::TyString
                if !matches!(self.tokens.get(self.pos + 1), Some((Tok::LParen, _))) =>
            {
                let span = self.span();
                let int32 = self.int32[self.pos];
                let name = match self.advance().0 {
                    Tok::TyInteger if int32 => "longint",
                    Tok::TyInteger => "integer",
                    Tok::TyReal => "real",
                    Tok::TyChar => "char",
                    Tok::TyBoolean => "boolean",
                    _ => "string",
                };
                Ok(Expr::Var(name.to_string(), span))
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
                    Tok::TyInteger if self.int32[self.pos] => "longint",
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
            // `specialize TList<Integer>.Create`, `TList<Integer>.Create`.
            Tok::Ident(w)
                if (w.eq_ignore_ascii_case("specialize")
                    && matches!(self.tokens.get(self.pos + 1), Some((Tok::Ident(_), _)))
                    && matches!(self.tokens.get(self.pos + 2), Some((Tok::Lt, _))))
                    || (matches!(self.tokens.get(self.pos + 1), Some((Tok::Lt, _)))
                        && self.type_args_then(self.pos + 1, true)) =>
            {
                let span = self.span();
                if w.eq_ignore_ascii_case("specialize") {
                    self.advance();
                }
                let (name, _) = self.expect_ident()?;
                let args = self.parse_type_args()?;
                let alias = self.hidden_specialization(name, args, span);
                // `specialize Max<Integer>(1, 2)`: a generic routine's call.
                if *self.peek() == Tok::LParen {
                    let args = self.parse_call_args()?;
                    let call = Expr::Call {
                        name: alias,
                        args,
                        span,
                    };
                    return self.parse_postfix(call, span);
                }
                self.parse_postfix(Expr::Var(alias, span), span)
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
                let span = self.span();
                self.advance();
                let expr = self.parse_expr()?;
                self.expect(&Tok::RParen)?;
                // `(p + n)^`, `(x).Field`: postfix operators apply too.
                self.parse_postfix(expr, span)
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
