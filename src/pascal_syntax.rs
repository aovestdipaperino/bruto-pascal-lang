/// Pascal syntax highlighter for turbo-vision's Editor.
use turbo_vision::views::syntax::{SyntaxHighlighter, Token, TokenType};

pub struct PascalHighlighter {
    in_block_comment_brace: bool,
    in_block_comment_paren: bool,
}

impl PascalHighlighter {
    pub fn new() -> Self {
        Self {
            in_block_comment_brace: false,
            in_block_comment_paren: false,
        }
    }

    fn is_keyword(word: &str) -> bool {
        matches!(
            word,
            "program" | "var" | "begin" | "end" | "if" | "then" | "else"
                | "while" | "do" | "for" | "to" | "downto" | "repeat" | "until"
                | "write" | "writeln" | "read" | "readln"
                | "div" | "mod" | "and" | "or" | "not"
                | "true" | "false" | "const" | "type" | "procedure" | "function"
                | "array" | "of" | "record" | "nil" | "case" | "with"
        )
    }

    fn is_type_name(word: &str) -> bool {
        matches!(word, "integer" | "string" | "boolean" | "real" | "char" | "byte" | "word" | "longint")
    }
}

impl SyntaxHighlighter for PascalHighlighter {
    fn language(&self) -> &str {
        "pascal"
    }

    fn highlight_line(&self, line: &str, _line_number: usize) -> Vec<Token> {
        let mut tokens = Vec::new();
        let chars: Vec<char> = line.chars().collect();
        let len = chars.len();
        let mut i = 0;

        // If we're inside a block comment from a previous line, continue as comment
        let mut in_brace = self.in_block_comment_brace;
        let mut in_paren = self.in_block_comment_paren;

        while i < len {
            // Inside { } block comment
            if in_brace {
                let start = i;
                while i < len && chars[i] != '}' {
                    i += 1;
                }
                if i < len {
                    i += 1; // consume '}'
                    in_brace = false;
                }
                tokens.push(Token::new(start, i, TokenType::Comment));
                continue;
            }

            // Inside (* *) block comment
            if in_paren {
                let start = i;
                while i < len {
                    if i + 1 < len && chars[i] == '*' && chars[i + 1] == ')' {
                        i += 2;
                        in_paren = false;
                        break;
                    }
                    i += 1;
                }
                tokens.push(Token::new(start, i, TokenType::Comment));
                continue;
            }

            let ch = chars[i];

            // Skip whitespace
            if ch.is_whitespace() {
                let start = i;
                while i < len && chars[i].is_whitespace() {
                    i += 1;
                }
                tokens.push(Token::new(start, i, TokenType::Normal));
                continue;
            }

            // Line comment: //
            if ch == '/' && i + 1 < len && chars[i + 1] == '/' {
                tokens.push(Token::new(i, len, TokenType::Comment));
                break;
            }

            // Block comment: { }
            if ch == '{' {
                let start = i;
                i += 1;
                while i < len && chars[i] != '}' {
                    i += 1;
                }
                if i < len {
                    i += 1;
                } else {
                    in_brace = true;
                }
                tokens.push(Token::new(start, i, TokenType::Comment));
                continue;
            }

            // Block comment: (* *)
            if ch == '(' && i + 1 < len && chars[i + 1] == '*' {
                let start = i;
                i += 2;
                while i < len {
                    if i + 1 < len && chars[i] == '*' && chars[i + 1] == ')' {
                        i += 2;
                        break;
                    }
                    i += 1;
                }
                if i >= len && !(i >= 2 && chars[i - 2] == '*' && chars[i - 1] == ')') {
                    in_paren = true;
                }
                tokens.push(Token::new(start, i, TokenType::Comment));
                continue;
            }

            // String literals: 'text'
            if ch == '\'' {
                let start = i;
                i += 1;
                while i < len {
                    if chars[i] == '\'' {
                        // Check for escaped quote ''
                        if i + 1 < len && chars[i + 1] == '\'' {
                            i += 2;
                        } else {
                            i += 1;
                            break;
                        }
                    } else {
                        i += 1;
                    }
                }
                tokens.push(Token::new(start, i, TokenType::String));
                continue;
            }

            // Numbers
            if ch.is_ascii_digit() || (ch == '$' && i + 1 < len && chars[i + 1].is_ascii_hexdigit()) {
                let start = i;
                if ch == '$' {
                    i += 1;
                    while i < len && chars[i].is_ascii_hexdigit() {
                        i += 1;
                    }
                } else {
                    while i < len && chars[i].is_ascii_digit() {
                        i += 1;
                    }
                }
                tokens.push(Token::new(start, i, TokenType::Number));
                continue;
            }

            // Identifiers and keywords
            if ch.is_ascii_alphabetic() || ch == '_' {
                let start = i;
                while i < len && (chars[i].is_ascii_alphanumeric() || chars[i] == '_') {
                    i += 1;
                }
                let word: String = chars[start..i].iter().collect();
                let lower = word.to_lowercase();
                let token_type = if Self::is_keyword(&lower) {
                    TokenType::Keyword
                } else if Self::is_type_name(&lower) {
                    TokenType::Type
                } else {
                    TokenType::Identifier
                };
                tokens.push(Token::new(start, i, token_type));
                continue;
            }

            // Multi-character operators
            if i + 1 < len {
                let two: String = chars[i..i + 2].iter().collect();
                if matches!(two.as_str(), ":=" | "<>" | "<=" | ">=") {
                    tokens.push(Token::new(i, i + 2, TokenType::Operator));
                    i += 2;
                    continue;
                }
            }

            // Single-character operators and punctuation
            if matches!(ch, '+' | '-' | '*' | '=' | '<' | '>' | '/' ) {
                tokens.push(Token::new(i, i + 1, TokenType::Operator));
                i += 1;
                continue;
            }

            if matches!(ch, ';' | ':' | '.' | ',' | '(' | ')' | '[' | ']') {
                tokens.push(Token::new(i, i + 1, TokenType::Special));
                i += 1;
                continue;
            }

            // Anything else
            tokens.push(Token::new(i, i + 1, TokenType::Normal));
            i += 1;
        }

        tokens
    }

    fn is_multiline_context(&self, _line_number: usize) -> bool {
        self.in_block_comment_brace || self.in_block_comment_paren
    }

    fn update_multiline_state(&mut self, line: &str, _line_number: usize) {
        let chars: Vec<char> = line.chars().collect();
        let len = chars.len();
        let mut i = 0;

        while i < len {
            if self.in_block_comment_brace {
                if chars[i] == '}' {
                    self.in_block_comment_brace = false;
                }
                i += 1;
                continue;
            }
            if self.in_block_comment_paren {
                if i + 1 < len && chars[i] == '*' && chars[i + 1] == ')' {
                    self.in_block_comment_paren = false;
                    i += 2;
                    continue;
                }
                i += 1;
                continue;
            }

            // Check for comment starts (skip strings)
            if chars[i] == '\'' {
                i += 1;
                while i < len && chars[i] != '\'' {
                    i += 1;
                }
                if i < len { i += 1; }
                continue;
            }
            if chars[i] == '/' && i + 1 < len && chars[i + 1] == '/' {
                break; // rest of line is comment
            }
            if chars[i] == '{' {
                self.in_block_comment_brace = true;
                i += 1;
                continue;
            }
            if chars[i] == '(' && i + 1 < len && chars[i + 1] == '*' {
                self.in_block_comment_paren = true;
                i += 2;
                continue;
            }
            i += 1;
        }
    }
}
