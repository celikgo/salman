// SPDX-License-Identifier: Apache-2.0
//! Tokens produced by the lexer.
//!
//! Tokens are deliberately small — a kind and a span. Literal *values* live in
//! a side table indexed from the token, so that a file of ten thousand tokens
//! does not carry ten thousand heap-allocated strings around with it.

use salman_core::span::Span;
use salman_core::time::{Date, DateTime, Duration, TimeOfDay};
use salman_core::value::ElementaryType;

/// Longest keyword, used to size the stack buffer keyword lookup uses.
const MAX_KEYWORD_LEN: usize = 24;

macro_rules! keywords {
    ($($variant:ident => $text:literal),* $(,)?) => {
        /// A reserved word of IEC 61131-3.
        ///
        /// Keywords are matched case-insensitively, as the standard requires.
        // Each variant is named after the keyword it represents; a doc comment
        // per variant would say nothing the name does not.
        #[allow(missing_docs)]
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub enum Keyword {
            $($variant),*
        }

        impl Keyword {
            /// The canonical upper-case spelling.
            #[must_use]
            pub const fn text(self) -> &'static str {
                match self {
                    $(Self::$variant => $text),*
                }
            }

            /// Every keyword, in the order declared here.
            #[must_use]
            pub const fn all() -> &'static [Self] {
                &[$(Self::$variant),*]
            }
        }
    };
}

// Sorted by canonical spelling: `Keyword::lookup` binary-searches this order,
// and a test enforces that it stays sorted.
keywords! {
    Action => "ACTION",
    And => "AND",
    Array => "ARRAY",
    At => "AT",
    By => "BY",
    Case => "CASE",
    Class => "CLASS",
    Configuration => "CONFIGURATION",
    Constant => "CONSTANT",
    Continue => "CONTINUE",
    Do => "DO",
    Else => "ELSE",
    Elsif => "ELSIF",
    EndAction => "END_ACTION",
    EndCase => "END_CASE",
    EndClass => "END_CLASS",
    EndConfiguration => "END_CONFIGURATION",
    EndFor => "END_FOR",
    EndFunction => "END_FUNCTION",
    EndFunctionBlock => "END_FUNCTION_BLOCK",
    EndIf => "END_IF",
    EndInterface => "END_INTERFACE",
    EndMethod => "END_METHOD",
    EndProgram => "END_PROGRAM",
    EndRepeat => "END_REPEAT",
    EndResource => "END_RESOURCE",
    EndStep => "END_STEP",
    EndStruct => "END_STRUCT",
    EndTransition => "END_TRANSITION",
    EndType => "END_TYPE",
    EndVar => "END_VAR",
    EndWhile => "END_WHILE",
    Exit => "EXIT",
    Extends => "EXTENDS",
    False => "FALSE",
    For => "FOR",
    Function => "FUNCTION",
    FunctionBlock => "FUNCTION_BLOCK",
    If => "IF",
    Implements => "IMPLEMENTS",
    InitialStep => "INITIAL_STEP",
    Interface => "INTERFACE",
    Interval => "INTERVAL",
    Method => "METHOD",
    Mod => "MOD",
    NonRetain => "NON_RETAIN",
    Not => "NOT",
    Null => "NULL",
    Of => "OF",
    On => "ON",
    Or => "OR",
    Persistent => "PERSISTENT",
    Priority => "PRIORITY",
    Program => "PROGRAM",
    Ref => "REF",
    Repeat => "REPEAT",
    Resource => "RESOURCE",
    Retain => "RETAIN",
    Return => "RETURN",
    Single => "SINGLE",
    Step => "STEP",
    Struct => "STRUCT",
    Super => "SUPER",
    Task => "TASK",
    Then => "THEN",
    This => "THIS",
    To => "TO",
    Transition => "TRANSITION",
    True => "TRUE",
    Type => "TYPE",
    Until => "UNTIL",
    Var => "VAR",
    VarAccess => "VAR_ACCESS",
    VarConfig => "VAR_CONFIG",
    VarExternal => "VAR_EXTERNAL",
    VarGlobal => "VAR_GLOBAL",
    // Byte order, not alphabetical intuition: '_' (0x5F) sorts after 'P'
    // (0x50), so VAR_INPUT precedes VAR_IN_OUT. The sortedness test caught
    // this; binary search would otherwise have silently missed VAR_IN_OUT.
    VarInput => "VAR_INPUT",
    VarInOut => "VAR_IN_OUT",
    VarOutput => "VAR_OUTPUT",
    VarTemp => "VAR_TEMP",
    While => "WHILE",
    With => "WITH",
    Xor => "XOR",
}

impl Keyword {
    /// Looks a word up as a keyword, case-insensitively.
    ///
    /// Returns `None` for anything that is not reserved, including every
    /// elementary type name — those are looked up through
    /// [`elementary_type_from_word`] instead, because a type name is a
    /// different kind of thing from `IF` and treating them alike makes error
    /// messages worse.
    #[must_use]
    pub fn lookup(word: &str) -> Option<Self> {
        if word.is_empty() || word.len() > MAX_KEYWORD_LEN {
            return None;
        }
        let mut upper = [0u8; MAX_KEYWORD_LEN];
        for (slot, byte) in upper.iter_mut().zip(word.bytes()) {
            *slot = byte.to_ascii_uppercase();
        }
        let upper = core::str::from_utf8(upper.get(..word.len())?).ok()?;
        let all = Self::all();
        all.binary_search_by(|k| k.text().cmp(upper))
            .ok()
            .and_then(|i| all.get(i).copied())
    }

    /// Whether this keyword names a construct salman does not implement.
    ///
    /// These are reserved so that they cannot be used as identifiers — which is
    /// what the standard requires — and so that meeting one produces "salman
    /// does not implement `CLASS` yet" rather than a baffling syntax error.
    #[must_use]
    pub const fn is_unimplemented_construct(self) -> bool {
        matches!(
            self,
            Self::Class
                | Self::EndClass
                | Self::Interface
                | Self::EndInterface
                | Self::Method
                | Self::EndMethod
                | Self::Extends
                | Self::Implements
                | Self::This
                | Self::Super
                | Self::Step
                | Self::EndStep
                | Self::InitialStep
                | Self::Transition
                | Self::EndTransition
                | Self::Action
                | Self::EndAction
        )
    }
}

/// Maps a word to an elementary type name, case-insensitively.
///
/// Handles both the long and short spellings the standard gives for the date
/// types: `TIME_OF_DAY` and `TOD` name the same type, as do `DATE_AND_TIME`
/// and `DT`, and `DATE` and `D`.
#[must_use]
pub fn elementary_type_from_word(word: &str) -> Option<ElementaryType> {
    use ElementaryType as E;
    let candidates: &[(&str, ElementaryType)] = &[
        ("BOOL", E::Bool),
        ("SINT", E::Sint),
        ("INT", E::Int),
        ("DINT", E::Dint),
        ("LINT", E::Lint),
        ("USINT", E::Usint),
        ("UINT", E::Uint),
        ("UDINT", E::Udint),
        ("ULINT", E::Ulint),
        ("BYTE", E::Byte),
        ("WORD", E::Word),
        ("DWORD", E::Dword),
        ("LWORD", E::Lword),
        ("REAL", E::Real),
        ("LREAL", E::Lreal),
        ("TIME", E::Time),
        ("LTIME", E::LTime),
        ("DATE", E::Date),
        ("D", E::Date),
        ("TIME_OF_DAY", E::TimeOfDay),
        ("TOD", E::TimeOfDay),
        ("DATE_AND_TIME", E::DateAndTime),
        ("DT", E::DateAndTime),
        ("STRING", E::String),
        ("WSTRING", E::WString),
    ];
    candidates
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case(word))
        .map(|(_, ty)| *ty)
}

/// Punctuation and operator tokens.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Punct {
    /// `:=`
    Assign,
    /// `?=` — assignment attempt (IEC Ed. 3 Table 52).
    AssignAttempt,
    /// `=>` — output binding in a formal call.
    RightArrow,
    /// `;`
    Semicolon,
    /// `:`
    Colon,
    /// `,`
    Comma,
    /// `.`
    Dot,
    /// `..` — a range, as in a `CASE` label or an array bound.
    DotDot,
    /// `(`
    LParen,
    /// `)`
    RParen,
    /// `[`
    LBracket,
    /// `]`
    RBracket,
    /// `{`
    LBrace,
    /// `}`
    RBrace,
    /// `+`
    Plus,
    /// `-`
    Minus,
    /// `*`
    Star,
    /// `/`
    Slash,
    /// `**`
    Power,
    /// `<`
    Less,
    /// `>`
    Greater,
    /// `<=`
    LessEqual,
    /// `>=`
    GreaterEqual,
    /// `=`
    Equal,
    /// `<>`
    NotEqual,
    /// `&` — the other spelling of `AND`.
    Ampersand,
    /// `^` — dereference (IEC Ed. 3 Table 71 row 3).
    Caret,
    /// `%` — introduces a directly represented variable such as `%IX0.0`.
    Percent,
    /// `#` — separates a type or radix prefix from a literal.
    Hash,
}

impl Punct {
    /// The characters this token is written with.
    #[must_use]
    pub const fn text(self) -> &'static str {
        match self {
            Self::Assign => ":=",
            Self::AssignAttempt => "?=",
            Self::RightArrow => "=>",
            Self::Semicolon => ";",
            Self::Colon => ":",
            Self::Comma => ",",
            Self::Dot => ".",
            Self::DotDot => "..",
            Self::LParen => "(",
            Self::RParen => ")",
            Self::LBracket => "[",
            Self::RBracket => "]",
            Self::LBrace => "{",
            Self::RBrace => "}",
            Self::Plus => "+",
            Self::Minus => "-",
            Self::Star => "*",
            Self::Slash => "/",
            Self::Power => "**",
            Self::Less => "<",
            Self::Greater => ">",
            Self::LessEqual => "<=",
            Self::GreaterEqual => ">=",
            Self::Equal => "=",
            Self::NotEqual => "<>",
            Self::Ampersand => "&",
            Self::Caret => "^",
            Self::Percent => "%",
            Self::Hash => "#",
        }
    }
}

/// The value carried by a literal token.
#[derive(Debug, Clone, PartialEq)]
pub enum LiteralValue {
    /// `TRUE`, `FALSE`, `BOOL#1`, `BOOL#0`.
    Bool(bool),
    /// An integer literal, with the type its prefix declared, if any.
    ///
    /// The magnitude is held unsigned and the sign kept separately, so that
    /// `SINT#-128` — whose magnitude does not fit in the type but whose value
    /// does — survives lexing and is checked where the type is known.
    Int {
        /// Magnitude, before any sign is applied.
        magnitude: u128,
        /// Whether the literal carried a leading `-`.
        negative: bool,
        /// The type named by a prefix, e.g. `INT` in `INT#5`.
        declared: Option<ElementaryType>,
    },
    /// A real literal.
    Real {
        /// The value.
        value: f64,
        /// The type named by a prefix, e.g. `LREAL` in `LREAL#1.5`.
        declared: Option<ElementaryType>,
    },
    /// A duration literal, `T#...` or `LTIME#...`.
    Duration {
        /// The duration.
        value: Duration,
        /// Whether the prefix was one of the `LTIME` forms.
        long: bool,
    },
    /// A `DATE` literal.
    Date(Date),
    /// A `TIME_OF_DAY` literal.
    TimeOfDay(TimeOfDay),
    /// A `DATE_AND_TIME` literal.
    DateAndTime(DateTime),
    /// A single-quoted string, as bytes.
    String(Vec<u8>),
    /// A double-quoted string, as 16-bit code units.
    WString(Vec<u16>),
}

/// What a token is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenKind {
    /// A user identifier.
    Ident,
    /// A reserved word.
    Keyword(Keyword),
    /// An elementary type name used as a type, e.g. `DINT`.
    TypeName(ElementaryType),
    /// A literal; the value is at this index in the stream's literal table.
    Literal(u32),
    /// A directly represented variable such as `%IX0.0`; the value is at this
    /// index in the stream's address table.
    ///
    /// Lexed whole rather than reassembled by the parser, because `%QX7.5`
    /// would otherwise arrive as an identifier, a dot and a number.
    DirectAddress(u32),
    /// Punctuation or an operator.
    Punct(Punct),
    /// End of input. Always present exactly once, as the last token.
    Eof,
}

/// A token: what it is, and where it came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Token {
    /// What kind of token it is.
    pub kind: TokenKind,
    /// Where in the source it came from.
    pub span: Span,
}

impl Token {
    /// Whether this token is the given punctuation.
    #[must_use]
    pub const fn is_punct(&self, p: Punct) -> bool {
        matches!(self.kind, TokenKind::Punct(x) if x as u8 == p as u8)
    }

    /// Whether this token is the given keyword.
    #[must_use]
    pub const fn is_keyword(&self, k: Keyword) -> bool {
        matches!(self.kind, TokenKind::Keyword(x) if x as u8 == k as u8)
    }

    /// A description fit to appear in "expected X, found Y".
    #[must_use]
    pub fn describe(&self) -> String {
        match self.kind {
            TokenKind::Ident => "an identifier".to_string(),
            TokenKind::Keyword(k) => format!("`{}`", k.text()),
            TokenKind::TypeName(t) => format!("the type `{}`", t.name()),
            TokenKind::Literal(_) => "a literal".to_string(),
            TokenKind::DirectAddress(_) => "a direct address".to_string(),
            TokenKind::Punct(p) => format!("`{}`", p.text()),
            TokenKind::Eof => "end of file".to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_keyword_table_is_sorted_because_lookup_binary_searches_it() {
        let all = Keyword::all();
        for pair in all.windows(2) {
            let (a, b) = (pair[0], pair[1]);
            assert!(
                a.text() < b.text(),
                "keyword table out of order: {} before {}",
                a.text(),
                b.text()
            );
        }
    }

    #[test]
    fn every_keyword_looks_itself_up_in_any_case() {
        for k in Keyword::all() {
            assert_eq!(Keyword::lookup(k.text()), Some(*k));
            assert_eq!(Keyword::lookup(&k.text().to_lowercase()), Some(*k));
            let mixed: String = k
                .text()
                .chars()
                .enumerate()
                .map(|(i, c)| {
                    if i % 2 == 0 {
                        c
                    } else {
                        c.to_ascii_lowercase()
                    }
                })
                .collect();
            assert_eq!(Keyword::lookup(&mixed), Some(*k));
        }
    }

    #[test]
    fn no_keyword_is_longer_than_the_lookup_buffer() {
        for k in Keyword::all() {
            assert!(
                k.text().len() <= MAX_KEYWORD_LEN,
                "{} is too long",
                k.text()
            );
        }
    }

    #[test]
    fn ordinary_words_are_not_keywords() {
        for word in ["Motor_Run", "ton", "Conveyor", "", "x", "IFX", "END_IFX"] {
            assert_eq!(
                Keyword::lookup(word),
                None,
                "{word} was treated as a keyword"
            );
        }
    }

    #[test]
    fn keyword_lookup_never_panics_on_arbitrary_input() {
        for word in ["\u{1F600}", "É", &"x".repeat(10_000), "\0", "END_"] {
            let _ = Keyword::lookup(word);
        }
    }

    #[test]
    fn elementary_type_names_include_the_short_date_spellings() {
        use ElementaryType as E;
        assert_eq!(elementary_type_from_word("DINT"), Some(E::Dint));
        assert_eq!(elementary_type_from_word("dint"), Some(E::Dint));
        assert_eq!(elementary_type_from_word("TOD"), Some(E::TimeOfDay));
        assert_eq!(elementary_type_from_word("TIME_OF_DAY"), Some(E::TimeOfDay));
        assert_eq!(elementary_type_from_word("DT"), Some(E::DateAndTime));
        assert_eq!(elementary_type_from_word("D"), Some(E::Date));
        assert_eq!(elementary_type_from_word("Motor"), None);
    }

    #[test]
    fn type_names_are_not_also_keywords() {
        // If a type name were in both tables the lexer would have to choose,
        // and the choice would be silent. Keep them disjoint.
        for ty in ElementaryType::all() {
            assert_eq!(
                Keyword::lookup(ty.name()),
                None,
                "{} is in both the keyword and type tables",
                ty.name()
            );
        }
    }

    #[test]
    fn unimplemented_constructs_are_reserved_so_they_produce_a_clear_message() {
        assert!(Keyword::Class.is_unimplemented_construct());
        assert!(Keyword::Method.is_unimplemented_construct());
        assert!(Keyword::InitialStep.is_unimplemented_construct());
        assert!(!Keyword::If.is_unimplemented_construct());
        assert!(!Keyword::Program.is_unimplemented_construct());
    }

    #[test]
    fn punctuation_spellings_are_distinct() {
        let all = [
            Punct::Assign,
            Punct::AssignAttempt,
            Punct::RightArrow,
            Punct::Semicolon,
            Punct::Colon,
            Punct::Comma,
            Punct::Dot,
            Punct::DotDot,
            Punct::LParen,
            Punct::RParen,
            Punct::LBracket,
            Punct::RBracket,
            Punct::LBrace,
            Punct::RBrace,
            Punct::Plus,
            Punct::Minus,
            Punct::Star,
            Punct::Slash,
            Punct::Power,
            Punct::Less,
            Punct::Greater,
            Punct::LessEqual,
            Punct::GreaterEqual,
            Punct::Equal,
            Punct::NotEqual,
            Punct::Ampersand,
            Punct::Caret,
            Punct::Percent,
            Punct::Hash,
        ];
        let mut texts: Vec<&str> = all.iter().map(|p| p.text()).collect();
        let count = texts.len();
        texts.sort_unstable();
        texts.dedup();
        assert_eq!(texts.len(), count);
    }
}
