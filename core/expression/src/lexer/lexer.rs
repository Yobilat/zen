use crate::lexer::codes::{is_token_type, token_type};
use crate::lexer::cursor::{Cursor, CursorItem};
use crate::lexer::error::LexerError::{UnexpectedEof, UnmatchedSymbol};
use crate::lexer::error::LexerResult;
use crate::lexer::token::{
    Bracket, ComparisonOperator, Identifier, LogicalOperator, Operator, Token, TokenKind,
};
use crate::lexer::{QuotationMark, TemplateString};

#[derive(Debug, Default)]
pub struct Lexer<'arena> {
    tokens: Vec<Token<'arena>>,
}

impl<'arena> Lexer<'arena> {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn tokenize(&mut self, source: &'arena str) -> LexerResult<&[Token<'arena>]> {
        self.tokens.clear();

        Scanner::new(source, &mut self.tokens).scan()?;
        Ok(&self.tokens)
    }
}

struct Scanner<'arena, 'self_ref> {
    cursor: Cursor<'arena>,
    tokens: &'self_ref mut Vec<Token<'arena>>,
    source: &'arena str,
}

impl<'arena, 'self_ref> Scanner<'arena, 'self_ref> {
    pub fn new(source: &'arena str, tokens: &'self_ref mut Vec<Token<'arena>>) -> Self {
        Self {
            cursor: Cursor::from(source),
            source,
            tokens,
        }
    }

    pub fn scan(&mut self) -> LexerResult<()> {
        while let Some(cursor_item) = self.cursor.peek() {
            self.scan_cursor_item(cursor_item)?;
        }

        Ok(())
    }

    pub(crate) fn scan_cursor_item(&mut self, cursor_item: CursorItem) -> LexerResult<()> {
        let (i, s) = cursor_item;

        match s {
            token_type!("space") => {
                self.cursor.next();
                Ok(())
            }
            '\'' => self.string(QuotationMark::SingleQuote),
            '"' => self.string(QuotationMark::DoubleQuote),
            token_type!("digit") => self.number(),
            token_type!("bracket") => self.bracket(),
            token_type!("cmp_operator") => self.cmp_operator(),
            token_type!("operator") => self.operator(),
            token_type!("question_mark") => self.question_mark(),
            '`' => self.template_string(),
            '.' => self.dot(),
            token_type!("alpha") => self.identifier(),
            _ => Err(UnmatchedSymbol {
                symbol: s,
                position: i,
            }),
        }
    }

    fn next(&self) -> LexerResult<CursorItem> {
        self.cursor.next().ok_or_else(|| {
            let (a, b) = self.cursor.peek_back().unwrap_or((0, ' '));

            UnexpectedEof {
                symbol: b,
                position: a,
            }
        })
    }

    fn push(&mut self, token: Token<'arena>) {
        self.tokens.push(token);
    }

    fn template_string(&mut self) -> LexerResult<()> {
        let (start, _) = self.next()?;

        self.tokens.push(Token {
            kind: TokenKind::QuotationMark(QuotationMark::Backtick),
            span: (start, start + 1),
            value: QuotationMark::Backtick.into(),
        });

        let mut in_expression = false;
        let mut str_start = start + 1;
        loop {
            let (e, c) = self.next()?;

            match (c, in_expression) {
                ('`', _) => {
                    if str_start < e {
                        self.tokens.push(Token {
                            kind: TokenKind::Literal,
                            span: (str_start, e),
                            value: &self.source[str_start..e],
                        });
                    }

                    self.tokens.push(Token {
                        kind: TokenKind::QuotationMark(QuotationMark::Backtick),
                        span: (e, e + 1),
                        value: QuotationMark::Backtick.into(),
                    });

                    break;
                }
                ('$', false) => {
                    in_expression = self.cursor.next_if_is("{");
                    if in_expression {
                        self.tokens.push(Token {
                            kind: TokenKind::Literal,
                            span: (str_start, e),
                            value: &self.source[str_start..e],
                        });

                        self.tokens.push(Token {
                            kind: TokenKind::TemplateString(TemplateString::ExpressionStart),
                            span: (e, e + 2),
                            value: TemplateString::ExpressionStart.into(),
                        });
                    }
                }
                ('}', true) => {
                    in_expression = false;
                    self.tokens.push(Token {
                        kind: TokenKind::TemplateString(TemplateString::ExpressionEnd),
                        span: (str_start, e),
                        value: TemplateString::ExpressionEnd.into(),
                    });

                    str_start = e + 1;
                }
                (_, false) => {
                    // Continue reading string
                }
                (_, true) => {
                    self.cursor.back();
                    self.scan_cursor_item((e, c))?;
                }
            }
        }

        Ok(())
    }

    fn string(&mut self, quote_kind: QuotationMark) -> LexerResult<()> {
        let (start, opener) = self.next()?;
        let end: usize;

        loop {
            let (e, c) = self.next()?;
            if c == opener {
                end = e;
                break;
            }
        }

        self.push(Token {
            kind: TokenKind::QuotationMark(quote_kind),
            span: (start, start + 1),
            value: quote_kind.into(),
        });

        self.push(Token {
            kind: TokenKind::Literal,
            span: (start + 1, end),
            value: &self.source[start + 1..end],
        });

        self.push(Token {
            kind: TokenKind::QuotationMark(quote_kind),
            span: (end, end + 1),
            value: quote_kind.into(),
        });

        Ok(())
    }

    fn number(&mut self) -> LexerResult<()> {
        let (start, _) = self.next()?;
        let mut end = start;
        let mut fractal = false;

        while let Some((e, c)) = self
            .cursor
            .next_if(|c| is_token_type!(c, "digit") || c == '_' || c == '.')
        {
            if fractal && c == '.' {
                self.cursor.back();
                break;
            }

            if c == '.' {
                if let Some((_, p)) = self.cursor.peek() {
                    if p == '.' {
                        self.cursor.back();
                        break;
                    }

                    fractal = true
                }
            }

            end = e;
        }

        self.push(Token {
            kind: TokenKind::Number,
            span: (start, end + 1),
            value: &self.source[start..=end],
        });

        Ok(())
    }

    fn bracket(&mut self) -> LexerResult<()> {
        let (start, _) = self.next()?;

        let value = &self.source[start..=start];
        self.push(Token {
            kind: TokenKind::Bracket(Bracket::try_from(value)?),
            span: (start, start + 1),
            value,
        });

        Ok(())
    }

    fn dot(&mut self) -> LexerResult<()> {
        let (start, _) = self.next()?;
        let mut end = start;

        if self.cursor.next_if(|c| c == '.').is_some() {
            end += 1;
        }

        let value = &self.source[start..=end];
        self.push(Token {
            kind: TokenKind::Operator(Operator::try_from(value)?),
            span: (start, end + 1),
            value,
        });

        Ok(())
    }

    fn cmp_operator(&mut self) -> LexerResult<()> {
        let (start, _) = self.next()?;
        let mut end = start;

        if self.cursor.next_if(|c| c == '=').is_some() {
            end += 1;
        }

        let value = &self.source[start..=end];
        self.push(Token {
            kind: TokenKind::Operator(Operator::try_from(value)?),
            span: (start, end + 1),
            value,
        });

        Ok(())
    }

    fn question_mark(&mut self) -> LexerResult<()> {
        let (start, _) = self.next()?;
        let mut kind = TokenKind::Operator(Operator::QuestionMark);
        let mut end = start;

        if self.cursor.next_if(|c| c == '?').is_some() {
            kind = TokenKind::Operator(Operator::Logical(LogicalOperator::NullishCoalescing));
            end += 1;
        }

        let value = &self.source[start..=end];
        self.push(Token {
            kind,
            value,
            span: (start, end + 1),
        });

        Ok(())
    }

    fn operator(&mut self) -> LexerResult<()> {
        let (start, _) = self.next()?;

        let value = &self.source[start..=start];
        self.push(Token {
            kind: TokenKind::Operator(Operator::try_from(value)?),
            span: (start, start + 1),
            value,
        });

        Ok(())
    }

    fn not(&mut self, start: usize) -> LexerResult<()> {
        if self.cursor.next_if_is(" in ") {
            let end = self.cursor.position();

            self.push(Token {
                kind: TokenKind::Operator(Operator::Comparison(ComparisonOperator::NotIn)),
                span: (start, end - 1),
                value: "not in",
            })
        } else {
            let end = self.cursor.position();

            self.push(Token {
                kind: TokenKind::Operator(Operator::Logical(LogicalOperator::Not)),
                span: (start, end),
                value: "not",
            })
        }

        Ok(())
    }

    fn identifier(&mut self) -> LexerResult<()> {
        let (start, _) = self.next()?;
        let mut end = start;

        while let Some((e, _)) = self.cursor.next_if(|c| is_token_type!(c, "alphanumeric")) {
            end = e;
        }

        let value = &self.source[start..=end];
        match value {
            "and" => self.push(Token {
                kind: TokenKind::Operator(Operator::Logical(LogicalOperator::And)),
                span: (start, end + 1),
                value,
            }),
            "or" => self.push(Token {
                kind: TokenKind::Operator(Operator::Logical(LogicalOperator::Or)),
                span: (start, end + 1),
                value,
            }),
            "in" => self.push(Token {
                kind: TokenKind::Operator(Operator::Comparison(ComparisonOperator::In)),
                span: (start, end + 1),
                value,
            }),
            "true" => self.push(Token {
                kind: TokenKind::Boolean(true),
                span: (start, end + 1),
                value,
            }),
            "false" => self.push(Token {
                kind: TokenKind::Boolean(false),
                span: (start, end + 1),
                value,
            }),
            "not" => self.not(start)?,
            _ => self.push(Token {
                kind: TokenKind::Identifier(Identifier::from(value)),
                span: (start, end + 1),
                value,
            }),
        }

        Ok(())
    }
}
