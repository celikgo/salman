// SPDX-License-Identifier: Apache-2.0
//! The Structured Text abstract syntax tree.
//!
//! Citation policy: salman cites IEC 61131-3:2013 (Edition 3.0) clause, table
//! and figure numbers as references only. No IEC text is reproduced. Clause
//! numbers are edition-specific; see `docs/IEC_CITATIONS.md`.
//!
//! # Shape
//!
//! Every expression and statement is a `{ id, kind, span }` triple:
//!
//! * the **span** is what diagnostics point at;
//! * the **id** is a stable [`NodeId`] that later passes hang information on —
//!   the type checker records the resolved type of every expression in a side
//!   table indexed by id, and the compiler reads it back. Side tables keep the
//!   tree small and mean a new analysis can be added without changing the AST.
//!
//! Node identity is deliberately **not** a position in a list. When the
//! graphical languages arrive, a node's identity has to survive being moved on
//! a canvas, or every drag rewrites the file and destroys the git history that
//! is the point of salman.
//!
//! # Depth
//!
//! The tree is boxed and therefore recursive, so both building it and dropping
//! it consume stack proportional to nesting depth. The parser bounds nesting at
//! [`crate::dialect::Dialect::max_nesting_depth`]; that bound is what makes the
//! recursion safe on input salman did not write.

use salman_core::ident::Ident;
use salman_core::span::Span;
use salman_core::value::ElementaryType;

use crate::address::DirectAddress;
use crate::token::LiteralValue;

/// Identifies one node within a compilation unit.
///
/// Assigned by the parser in a single increasing sequence, so a side table can
/// be a `Vec` indexed by id rather than a map.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NodeId(pub u32);

impl NodeId {
    /// The index this id addresses in a side table.
    #[must_use]
    pub const fn index(self) -> usize {
        self.0 as usize
    }
}

/// An identifier as it appeared in the source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Name {
    /// The identifier, comparing case-insensitively as IEC requires.
    pub ident: Ident,
    /// Where it was written.
    pub span: Span,
}

impl Name {
    /// The spelling as written.
    #[must_use]
    pub fn as_str(&self) -> &str {
        self.ident.as_str()
    }
}

impl std::fmt::Display for Name {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.ident.fmt(f)
    }
}

// ---------------------------------------------------------------------------
// Expressions
// ---------------------------------------------------------------------------

/// A unary operator. IEC 61131-3:2013 Table 71 "Operators of the ST language",
/// rows 4, 5 and 6.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnaryOp {
    /// `-`
    Neg,
    /// `+`
    Plus,
    /// `NOT`
    Not,
}

impl UnaryOp {
    /// How the operator is written.
    #[must_use]
    pub const fn text(self) -> &'static str {
        match self {
            Self::Neg => "-",
            Self::Plus => "+",
            Self::Not => "NOT",
        }
    }
}

/// A binary operator. IEC 61131-3:2013 Table 71, rows 7 to 18.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinaryOp {
    /// `**`
    Pow,
    /// `*`
    Mul,
    /// `/`
    Div,
    /// `MOD`
    Mod,
    /// `+`
    Add,
    /// `-`
    Sub,
    /// `<`
    Lt,
    /// `>`
    Gt,
    /// `<=`
    Le,
    /// `>=`
    Ge,
    /// `=`
    Eq,
    /// `<>`
    Ne,
    /// `AND` or `&`
    And,
    /// `XOR`
    Xor,
    /// `OR`
    Or,
}

impl BinaryOp {
    /// How the operator is written. `AND` is rendered as the word, not `&`.
    #[must_use]
    pub const fn text(self) -> &'static str {
        match self {
            Self::Pow => "**",
            Self::Mul => "*",
            Self::Div => "/",
            Self::Mod => "MOD",
            Self::Add => "+",
            Self::Sub => "-",
            Self::Lt => "<",
            Self::Gt => ">",
            Self::Le => "<=",
            Self::Ge => ">=",
            Self::Eq => "=",
            Self::Ne => "<>",
            Self::And => "AND",
            Self::Xor => "XOR",
            Self::Or => "OR",
        }
    }

    /// Whether the operator yields `BOOL` whatever its operands are.
    #[must_use]
    pub const fn is_comparison(self) -> bool {
        matches!(
            self,
            Self::Lt | Self::Gt | Self::Le | Self::Ge | Self::Eq | Self::Ne
        )
    }
}

/// One argument in a call.
#[derive(Debug, Clone, PartialEq)]
pub enum Arg {
    /// A positional argument: `MAX(a, b)`.
    ///
    /// IEC 61131-3:2013 Table 20 "Function call" permits this for functions and
    /// Table 50 for methods. Table 42 "Function block call" offers no
    /// positional form, so it is an error on a function block; that is checked
    /// where the callee is known, not here.
    Positional(Expr),
    /// An input binding: `TON(IN := Start, PT := T#5s)`.
    Input {
        /// The formal parameter name.
        name: Name,
        /// The value.
        value: Expr,
    },
    /// An output binding: `TON(..., Q => Running)`.
    Output {
        /// The formal parameter name.
        name: Name,
        /// Where the output goes. `None` for `Q => ` with nothing after it,
        /// which is legal and means "discard".
        target: Option<Expr>,
    },
}

/// What kind of expression a node is.
#[derive(Debug, Clone, PartialEq)]
pub enum ExprKind {
    /// A literal.
    Literal(LiteralValue),
    /// A reference to a variable by name.
    Var(Name),
    /// A directly represented variable, `%IX0.0`.
    Direct(DirectAddress),
    /// Field or output access: `Timer.Q`, `Motor.Speed`.
    Member {
        /// What is being accessed.
        base: Box<Expr>,
        /// The field or output name.
        field: Name,
    },
    /// Array subscript: `Buffer[i]`, `Grid[i, j]`.
    Index {
        /// The array.
        base: Box<Expr>,
        /// One expression per dimension.
        indices: Vec<Expr>,
    },
    /// Dereference: `Ref^`. IEC 61131-3:2013 Table 71 row 3.
    ///
    /// Parsed so that code using references produces a clear "not implemented"
    /// message rather than a syntax error. salman implements no reference
    /// types.
    Deref(Box<Expr>),
    /// A unary operation.
    Unary {
        /// The operator.
        op: UnaryOp,
        /// Its operand.
        operand: Box<Expr>,
    },
    /// A binary operation.
    Binary {
        /// The operator.
        op: BinaryOp,
        /// Left operand.
        lhs: Box<Expr>,
        /// Right operand.
        rhs: Box<Expr>,
    },
    /// A call to a function, function block instance or method.
    Call {
        /// What is being called.
        callee: Box<Expr>,
        /// The arguments.
        args: Vec<Arg>,
    },
    /// A qualified enumeration value: `Colour#Red`.
    EnumValue {
        /// The enumeration type.
        ty: Name,
        /// The value.
        value: Name,
    },
    /// A parenthesised expression.
    ///
    /// Kept in the tree rather than folded away, because salman warns about the
    /// unparenthesised operand of `**` — where dialects disagree about binding
    /// — and because a formatter that discards the author's parentheses
    /// rewrites code nobody asked it to rewrite.
    Paren(Box<Expr>),
    /// A node the parser could not build. Carries no children.
    ///
    /// The parser produces this rather than stopping, so that one syntax error
    /// does not hide the rest of the file. Nothing downstream should ever be
    /// asked to compile one: a unit containing an error node has diagnostics.
    Error,
}

/// An expression.
#[derive(Debug, Clone, PartialEq)]
pub struct Expr {
    /// Stable identity, used to index analysis side tables.
    pub id: NodeId,
    /// What it is.
    pub kind: ExprKind,
    /// Where it is.
    pub span: Span,
}

// ---------------------------------------------------------------------------
// Statements
// ---------------------------------------------------------------------------

/// One `CASE` label. IEC 61131-3:2013 Table 72 "ST language statements" row 5.
#[derive(Debug, Clone, PartialEq)]
pub enum CaseLabel {
    /// A single value: `3:`.
    Single(Expr),
    /// An inclusive range: `3..7:`.
    Range {
        /// Lower bound.
        low: Expr,
        /// Upper bound, inclusive.
        high: Expr,
    },
}

/// One arm of a `CASE`.
#[derive(Debug, Clone, PartialEq)]
pub struct CaseArm {
    /// The labels that select this arm.
    pub labels: Vec<CaseLabel>,
    /// What it does.
    pub body: Vec<Stmt>,
    /// The whole arm.
    pub span: Span,
}

/// One branch of an `IF`.
#[derive(Debug, Clone, PartialEq)]
pub struct IfBranch {
    /// The condition, which must be `BOOL`.
    pub condition: Expr,
    /// What runs when it is true.
    pub body: Vec<Stmt>,
    /// The whole branch.
    pub span: Span,
}

/// What kind of statement a node is.
#[derive(Debug, Clone, PartialEq)]
pub enum StmtKind {
    /// A bare `;`. Table 72 row 11.
    Empty,
    /// `target := value;`. Table 72 row 1.
    Assign {
        /// What is assigned to.
        target: Expr,
        /// The value.
        value: Expr,
    },
    /// `target ?= value;`, the assignment attempt. Table 52.
    ///
    /// Parsed so that code using it gets a clear message; salman implements no
    /// references or interfaces, so it cannot be checked or compiled.
    AssignAttempt {
        /// What is assigned to.
        target: Expr,
        /// The value.
        value: Expr,
    },
    /// A call used as a statement: `Timer(IN := Start, PT := T#5s);`.
    Call(Expr),
    /// `IF ... THEN ... ELSIF ... ELSE ... END_IF`. Table 72 row 4.
    If {
        /// The `IF` branch followed by any `ELSIF` branches, in order.
        branches: Vec<IfBranch>,
        /// The `ELSE` body, if there is one.
        else_body: Option<Vec<Stmt>>,
    },
    /// `CASE ... OF ... ELSE ... END_CASE`. Table 72 row 5.
    Case {
        /// The selector.
        selector: Expr,
        /// The arms, in source order.
        arms: Vec<CaseArm>,
        /// The `ELSE` body, if there is one.
        else_body: Option<Vec<Stmt>>,
    },
    /// `FOR v := a TO b BY c DO ... END_FOR`. Table 72 row 6.
    For {
        /// The control variable.
        variable: Name,
        /// Initial value.
        from: Expr,
        /// Final value, inclusive.
        to: Expr,
        /// Step; absent means 1.
        by: Option<Expr>,
        /// The body.
        body: Vec<Stmt>,
    },
    /// `WHILE ... DO ... END_WHILE`. Table 72 row 7.
    While {
        /// The condition, tested before each pass.
        condition: Expr,
        /// The body.
        body: Vec<Stmt>,
    },
    /// `REPEAT ... UNTIL ... END_REPEAT`. Table 72 row 8.
    Repeat {
        /// The body, which runs at least once.
        body: Vec<Stmt>,
        /// The condition, tested after each pass; the loop ends when it is true.
        until: Expr,
    },
    /// `CONTINUE;`. Table 72 row 9 — new in Edition 3.
    Continue,
    /// `EXIT;`. Table 72 row 10.
    Exit,
    /// `RETURN;`. Table 72 row 3.
    Return,
    /// A statement the parser could not build.
    Error,
}

/// A statement.
#[derive(Debug, Clone, PartialEq)]
pub struct Stmt {
    /// Stable identity.
    pub id: NodeId,
    /// What it is.
    pub kind: StmtKind,
    /// Where it is.
    pub span: Span,
}

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// One dimension of an array declaration.
#[derive(Debug, Clone, PartialEq)]
pub struct ArrayDim {
    /// Lower bound.
    pub low: Expr,
    /// Upper bound, inclusive.
    pub high: Expr,
    /// The whole dimension.
    pub span: Span,
}

/// A type as written in a declaration.
#[derive(Debug, Clone, PartialEq)]
pub enum TypeRef {
    /// An elementary type: `DINT`.
    Elementary {
        /// Which one.
        ty: ElementaryType,
        /// Where it was written.
        span: Span,
    },
    /// `STRING[80]` or `WSTRING`.
    String {
        /// `STRING` or `WSTRING`.
        ty: ElementaryType,
        /// The declared maximum length; absent means the dialect's default.
        max_len: Option<Box<Expr>>,
        /// Where it was written.
        span: Span,
    },
    /// A user-defined type or a function block type used as a variable type.
    Named(Name),
    /// `ARRAY [0..9, 0..3] OF DINT`.
    Array {
        /// The dimensions, in order.
        dims: Vec<ArrayDim>,
        /// The element type.
        element: Box<TypeRef>,
        /// The whole type.
        span: Span,
    },
    /// `INT (0..100)`.
    Subrange {
        /// The base type.
        base: Box<TypeRef>,
        /// Lower bound.
        low: Box<Expr>,
        /// Upper bound, inclusive.
        high: Box<Expr>,
        /// The whole type.
        span: Span,
    },
    /// A type the parser could not build.
    Error(Span),
}

impl TypeRef {
    /// Where this type was written.
    #[must_use]
    pub fn span(&self) -> Span {
        match self {
            Self::Elementary { span, .. }
            | Self::String { span, .. }
            | Self::Array { span, .. }
            | Self::Subrange { span, .. }
            | Self::Error(span) => *span,
            Self::Named(name) => name.span,
        }
    }
}

/// One value of an enumerated type.
#[derive(Debug, Clone, PartialEq)]
pub struct EnumValue {
    /// The value's name.
    pub name: Name,
    /// Its explicit value, if one was given.
    pub value: Option<Expr>,
}

/// What kind of type declaration this is.
#[derive(Debug, Clone, PartialEq)]
pub enum TypeDeclKind {
    /// `TYPE Speed : INT; END_TYPE` — an alias, possibly with an initialiser.
    Alias(TypeRef),
    /// `STRUCT ... END_STRUCT`.
    Struct(Vec<VarDecl>),
    /// `(Red, Green, Blue)`.
    Enum {
        /// The base type, if one was named: `(Red, Green) INT`.
        base: Option<ElementaryType>,
        /// The values, in order.
        values: Vec<EnumValue>,
    },
    /// `INT (0..100)`.
    Subrange {
        /// The base type.
        base: TypeRef,
        /// Lower bound.
        low: Expr,
        /// Upper bound, inclusive.
        high: Expr,
    },
    /// `ARRAY [1..10] OF REAL`.
    Array {
        /// The dimensions.
        dims: Vec<ArrayDim>,
        /// The element type.
        element: TypeRef,
    },
}

/// A declaration inside `TYPE ... END_TYPE`.
#[derive(Debug, Clone, PartialEq)]
pub struct TypeDecl {
    /// The type's name.
    pub name: Name,
    /// What it declares.
    pub kind: TypeDeclKind,
    /// The default initial value, if one was given.
    pub init: Option<Expr>,
    /// The whole declaration.
    pub span: Span,
}

// ---------------------------------------------------------------------------
// Declarations
// ---------------------------------------------------------------------------

/// Which `VAR` section a declaration is in.
///
/// IEC 61131-3:2013 §6.5.2 "Variable sections".
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum VarSection {
    /// `VAR` — internal to the POU.
    Local,
    /// `VAR_INPUT` — read by the POU, written by its caller.
    Input,
    /// `VAR_OUTPUT` — written by the POU, read by its caller.
    Output,
    /// `VAR_IN_OUT` — passed by reference.
    InOut,
    /// `VAR_TEMP` — not retained between invocations.
    Temp,
    /// `VAR_GLOBAL` — visible across the configuration.
    Global,
    /// `VAR_EXTERNAL` — a reference to a `VAR_GLOBAL` declared elsewhere.
    External,
    /// `VAR_ACCESS` — a named access path.
    Access,
    /// `VAR_CONFIG` — configuration of instance-specific locations.
    Config,
}

impl VarSection {
    /// The keyword that opens this section.
    #[must_use]
    pub const fn keyword(self) -> &'static str {
        match self {
            Self::Local => "VAR",
            Self::Input => "VAR_INPUT",
            Self::Output => "VAR_OUTPUT",
            Self::InOut => "VAR_IN_OUT",
            Self::Temp => "VAR_TEMP",
            Self::Global => "VAR_GLOBAL",
            Self::External => "VAR_EXTERNAL",
            Self::Access => "VAR_ACCESS",
            Self::Config => "VAR_CONFIG",
        }
    }

    /// Whether a POU may write to variables in this section.
    #[must_use]
    pub const fn is_writable_by_pou(self) -> bool {
        !matches!(self, Self::Input)
    }
}

/// How a variable survives a power cycle. IEC 61131-3:2013 §6.5.6.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Retention {
    /// Not stated. The value is re-initialised on a warm or cold start.
    #[default]
    Unspecified,
    /// `RETAIN`.
    Retain,
    /// `NON_RETAIN`.
    NonRetain,
}

/// Qualifiers on a whole `VAR` block.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct VarQualifiers {
    /// `RETAIN` or `NON_RETAIN`.
    pub retention: Retention,
    /// `CONSTANT`.
    pub constant: bool,
    /// `PERSISTENT`.
    pub persistent: bool,
}

/// One variable declaration, which may declare several names at once.
#[derive(Debug, Clone, PartialEq)]
pub struct VarDecl {
    /// The names declared, in order: `A, B, C : DINT;`.
    pub names: Vec<Name>,
    /// `AT %IX0.0`, if present.
    pub located_at: Option<DirectAddress>,
    /// Where the `AT` clause was written.
    pub located_at_span: Option<Span>,
    /// The declared type.
    pub ty: TypeRef,
    /// The initial value, if one was given.
    pub init: Option<Expr>,
    /// The whole declaration.
    pub span: Span,
}

/// A `VAR ... END_VAR` block.
#[derive(Debug, Clone, PartialEq)]
pub struct VarBlock {
    /// Which section it is.
    pub section: VarSection,
    /// Its qualifiers.
    pub qualifiers: VarQualifiers,
    /// The declarations in it, in order.
    pub decls: Vec<VarDecl>,
    /// The whole block.
    pub span: Span,
}

/// What kind of program organization unit this is. IEC 61131-3:2013 §6.6.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PouKind {
    /// `PROGRAM`. §6.6.4.
    Program,
    /// `FUNCTION`. §6.6.2. Stateless, returns a value.
    Function,
    /// `FUNCTION_BLOCK`. §6.6.3. Stateful, instantiated.
    FunctionBlock,
}

impl PouKind {
    /// The keyword that opens it.
    #[must_use]
    pub const fn keyword(self) -> &'static str {
        match self {
            Self::Program => "PROGRAM",
            Self::Function => "FUNCTION",
            Self::FunctionBlock => "FUNCTION_BLOCK",
        }
    }

    /// The keyword that closes it.
    #[must_use]
    pub const fn end_keyword(self) -> &'static str {
        match self {
            Self::Program => "END_PROGRAM",
            Self::Function => "END_FUNCTION",
            Self::FunctionBlock => "END_FUNCTION_BLOCK",
        }
    }

    /// Whether instances of this kind keep state between invocations.
    #[must_use]
    pub const fn is_stateful(self) -> bool {
        matches!(self, Self::FunctionBlock | Self::Program)
    }
}

/// A program organization unit.
#[derive(Debug, Clone, PartialEq)]
pub struct Pou {
    /// Program, function or function block.
    pub kind: PouKind,
    /// Its name.
    pub name: Name,
    /// The return type, for a function. Always `None` otherwise.
    pub return_type: Option<TypeRef>,
    /// Its variable blocks, in source order.
    pub var_blocks: Vec<VarBlock>,
    /// Its body.
    pub body: Vec<Stmt>,
    /// The whole unit.
    pub span: Span,
}

/// A `TASK` declaration. IEC 61131-3:2013 §6.8.2 "Tasks".
#[derive(Debug, Clone, PartialEq)]
pub struct TaskDecl {
    /// Its name.
    pub name: Name,
    /// `SINGLE := <bool variable>` — an event-triggered task.
    pub single: Option<Expr>,
    /// `INTERVAL := <duration>` — a cyclic task.
    pub interval: Option<Expr>,
    /// `PRIORITY := <integer>`. Lower numbers are more urgent, per the
    /// standard.
    pub priority: Option<Expr>,
    /// The whole declaration.
    pub span: Span,
}

/// A `PROGRAM ... WITH ... : ...` instance inside a `RESOURCE`.
#[derive(Debug, Clone, PartialEq)]
pub struct ProgramInstance {
    /// The instance name.
    pub name: Name,
    /// The task it runs under, if one was named.
    pub task: Option<Name>,
    /// The `PROGRAM` type it instantiates.
    pub program_type: Name,
    /// The whole declaration.
    pub span: Span,
}

/// A `RESOURCE ... ON ... END_RESOURCE`.
#[derive(Debug, Clone, PartialEq)]
pub struct ResourceDecl {
    /// Its name.
    pub name: Name,
    /// The processor type named after `ON`.
    pub on_type: Option<Name>,
    /// Global variables scoped to this resource.
    pub var_blocks: Vec<VarBlock>,
    /// Its tasks.
    pub tasks: Vec<TaskDecl>,
    /// Its program instances.
    pub programs: Vec<ProgramInstance>,
    /// The whole declaration.
    pub span: Span,
}

/// A `CONFIGURATION ... END_CONFIGURATION`. IEC 61131-3:2013 §6.8.
#[derive(Debug, Clone, PartialEq)]
pub struct ConfigurationDecl {
    /// Its name.
    pub name: Name,
    /// Global variables scoped to the configuration.
    pub var_blocks: Vec<VarBlock>,
    /// Its resources.
    pub resources: Vec<ResourceDecl>,
    /// The whole declaration.
    pub span: Span,
}

/// A top-level item in a source file.
#[derive(Debug, Clone, PartialEq)]
pub enum Item {
    /// A program organization unit.
    Pou(Pou),
    /// A `TYPE ... END_TYPE` block.
    Types(Vec<TypeDecl>),
    /// A `VAR_GLOBAL ... END_VAR` block at file scope.
    Globals(VarBlock),
    /// A configuration.
    Configuration(ConfigurationDecl),
}

/// Everything parsed from one source file.
#[derive(Debug, Clone, PartialEq)]
pub struct CompilationUnit {
    /// The file it came from.
    pub file: salman_core::span::FileId,
    /// Its top-level items, in source order.
    pub items: Vec<Item>,
    /// How many [`NodeId`]s were allocated, so a side table can be sized once.
    pub node_count: u32,
}

impl CompilationUnit {
    /// Joins units parsed from disjoint id ranges into one.
    ///
    /// Semantic analysis works on a single unit, because a name declared in one
    /// file has to resolve against a POU declared in another. Joining is only
    /// sound when the units were parsed with
    /// [`parse_source_from`](crate::parse_source_from) so that no two share a
    /// [`NodeId`]; overlapping ids would make two nodes index the same entry in
    /// every side table downstream, which is a wrong answer rather than a
    /// crash. The `file` of the result is the first unit's, and it is only a
    /// label: every span carries its own file.
    ///
    /// Returns `None` if `units` is empty — there is nothing to check.
    #[must_use]
    pub fn join(units: Vec<Self>) -> Option<Self> {
        let mut units = units.into_iter();
        let mut first = units.next()?;
        for unit in units {
            first.node_count = first.node_count.max(unit.node_count);
            first.items.extend(unit.items);
        }
        Some(first)
    }

    /// Every POU in the unit, in source order.
    pub fn pous(&self) -> impl Iterator<Item = &Pou> {
        self.items.iter().filter_map(|item| match item {
            Item::Pou(p) => Some(p),
            _ => None,
        })
    }

    /// A POU by name, compared case-insensitively as IEC requires.
    #[must_use]
    pub fn pou(&self, name: &str) -> Option<&Pou> {
        self.pous().find(|p| p.name.ident.eq_str(name))
    }

    /// Every configuration in the unit.
    pub fn configurations(&self) -> impl Iterator<Item = &ConfigurationDecl> {
        self.items.iter().filter_map(|item| match item {
            Item::Configuration(c) => Some(c),
            _ => None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn comparison_operators_are_the_ones_that_yield_bool() {
        for op in [
            BinaryOp::Lt,
            BinaryOp::Gt,
            BinaryOp::Le,
            BinaryOp::Ge,
            BinaryOp::Eq,
            BinaryOp::Ne,
        ] {
            assert!(op.is_comparison(), "{} should be a comparison", op.text());
        }
        for op in [BinaryOp::Add, BinaryOp::And, BinaryOp::Pow, BinaryOp::Mod] {
            assert!(
                !op.is_comparison(),
                "{} should not be a comparison",
                op.text()
            );
        }
    }

    #[test]
    fn operator_spellings_are_distinct() {
        let ops = [
            BinaryOp::Pow,
            BinaryOp::Mul,
            BinaryOp::Div,
            BinaryOp::Mod,
            BinaryOp::Add,
            BinaryOp::Sub,
            BinaryOp::Lt,
            BinaryOp::Gt,
            BinaryOp::Le,
            BinaryOp::Ge,
            BinaryOp::Eq,
            BinaryOp::Ne,
            BinaryOp::And,
            BinaryOp::Xor,
            BinaryOp::Or,
        ];
        let mut texts: Vec<&str> = ops.iter().map(|o| o.text()).collect();
        let count = texts.len();
        texts.sort_unstable();
        texts.dedup();
        assert_eq!(texts.len(), count);
    }

    #[test]
    fn pou_keywords_pair_up() {
        for kind in [PouKind::Program, PouKind::Function, PouKind::FunctionBlock] {
            assert_eq!(kind.end_keyword(), format!("END_{}", kind.keyword()));
        }
    }

    #[test]
    fn functions_are_stateless_and_function_blocks_are_not() {
        // This is the whole difference between them, and the reason a FUNCTION
        // may not contain a TON.
        assert!(!PouKind::Function.is_stateful());
        assert!(PouKind::FunctionBlock.is_stateful());
        assert!(PouKind::Program.is_stateful());
    }

    #[test]
    fn a_pou_may_not_write_to_its_own_inputs() {
        assert!(!VarSection::Input.is_writable_by_pou());
        assert!(VarSection::Output.is_writable_by_pou());
        assert!(VarSection::Local.is_writable_by_pou());
        assert!(VarSection::InOut.is_writable_by_pou());
    }

    #[test]
    fn var_section_keywords_are_distinct() {
        let sections = [
            VarSection::Local,
            VarSection::Input,
            VarSection::Output,
            VarSection::InOut,
            VarSection::Temp,
            VarSection::Global,
            VarSection::External,
            VarSection::Access,
            VarSection::Config,
        ];
        let mut keywords: Vec<&str> = sections.iter().map(|s| s.keyword()).collect();
        let count = keywords.len();
        keywords.sort_unstable();
        keywords.dedup();
        assert_eq!(keywords.len(), count);
    }

    #[test]
    fn retention_defaults_to_unspecified_rather_than_to_retain() {
        // Defaulting to RETAIN would silently give every variable persistence
        // the engineer did not ask for, which is exactly the class of bug
        // salman's retain simulation exists to find.
        assert_eq!(Retention::default(), Retention::Unspecified);
        assert_eq!(VarQualifiers::default().retention, Retention::Unspecified);
        assert!(!VarQualifiers::default().constant);
    }
}
