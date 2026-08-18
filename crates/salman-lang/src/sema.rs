// SPDX-License-Identifier: Apache-2.0
//! Semantic analysis: names, types and the facts the compiler needs.
//!
//! Citation policy: salman cites IEC 61131-3:2013 (Edition 3.0) clause, table
//! and figure numbers as references only. No IEC text is reproduced. Clause
//! numbers are edition-specific; see `docs/IEC_CITATIONS.md`.
//!
//! # What this produces
//!
//! [`check`] walks a parsed unit and returns a [`Checked`], which is a set of
//! **side tables indexed by [`NodeId`]** plus the symbol tables. Side tables
//! rather than an annotated tree, for two reasons: the tree stays small and
//! cheap to clone, and a later analysis can be added without changing the AST
//! that the parser, the formatter and the graphical editors all share.
//!
//! [`NodeId`]: crate::ast::NodeId
//!
//! # What is deliberately absent
//!
//! Anything salman does not implement is a diagnostic here, not a silent
//! success: references, the assignment attempt, classes, methods, interfaces,
//! namespaces and the graphical languages all produce a message naming what is
//! missing rather than a confusing downstream failure.

use salman_core::ident::IdentKey;
use salman_core::span::Span;
use salman_core::value::Value;

use crate::address::DirectAddress;
use crate::ast::{Name, PouKind, VarQualifiers, VarSection};
use crate::stdlib::NativeBlock;
use crate::types::{TypeArena, TypeId};

/// One declared variable, after its type has been resolved.
#[derive(Debug, Clone, PartialEq)]
pub struct Symbol {
    /// Its name, as written.
    pub name: Name,
    /// Which `VAR` section declared it.
    pub section: VarSection,
    /// `RETAIN`, `CONSTANT` and the rest.
    pub qualifiers: VarQualifiers,
    /// Its resolved type.
    pub ty: TypeId,
    /// The `AT %...` location, if it has one.
    pub address: Option<DirectAddress>,
    /// The declared initial value, folded to a constant.
    ///
    /// `None` means no initialiser was written, or the initialiser was not a
    /// constant — in which case a diagnostic was reported.
    pub init: Option<Value>,
}

impl Symbol {
    /// Whether a POU may assign to this variable.
    ///
    /// `VAR_INPUT` is read-only inside the POU that declares it, and a
    /// `CONSTANT` is read-only everywhere.
    #[must_use]
    pub const fn is_writable(&self) -> bool {
        self.section.is_writable_by_pou() && !self.qualifiers.constant
    }
}

/// What a name in an expression turned out to refer to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Resolution {
    /// A variable declared in the POU that contains the expression.
    Local {
        /// Index into [`Checked::pous`].
        pou: u32,
        /// Index into that POU's symbols.
        symbol: u32,
    },
    /// A file-scope `VAR_GLOBAL`.
    Global {
        /// Index into [`Checked::globals`].
        symbol: u32,
    },
    /// A call to a user-declared `FUNCTION`.
    Function {
        /// Index into [`Checked::pous`].
        pou: u32,
    },
    /// A field of a structure, or an input or output of a function block
    /// instance.
    Member {
        /// The type the field belongs to.
        owner: TypeId,
        /// The field's position within that type, in declaration order.
        offset: u32,
    },
    /// A value of an enumerated type.
    EnumValue {
        /// The enumeration.
        ty: TypeId,
        /// Its numeric value.
        value: i64,
    },
    /// Could not be resolved. A diagnostic has already been reported, and
    /// downstream passes should treat this node as poison rather than reporting
    /// again.
    Error,
}

/// One POU's symbol table.
#[derive(Debug, Clone, PartialEq)]
pub struct PouSymbols {
    /// Its name.
    pub name: Name,
    /// Program, function or function block.
    pub kind: PouKind,
    /// The whole declaration.
    pub span: Span,
    /// A function's return type.
    pub return_type: Option<TypeId>,
    /// Its variables, in declaration order — which is the order the compiler
    /// lays out slots in, so it must be stable.
    pub symbols: Vec<Symbol>,
}

impl PouSymbols {
    /// A symbol by name, compared case-insensitively as IEC requires.
    #[must_use]
    pub fn symbol(&self, name: &str) -> Option<(u32, &Symbol)> {
        self.symbols
            .iter()
            .enumerate()
            .find(|(_, s)| s.name.ident.eq_str(name))
            .and_then(|(i, s)| u32::try_from(i).ok().map(|i| (i, s)))
    }

    /// The symbols in one section, in declaration order.
    pub fn section(&self, section: VarSection) -> impl Iterator<Item = &Symbol> {
        self.symbols.iter().filter(move |s| s.section == section)
    }
}

/// What releases a checked task.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckedTrigger {
    /// `INTERVAL := <duration>`.
    Cyclic {
        /// The period, in nanoseconds.
        interval_ns: i64,
    },
    /// `SINGLE := <bool variable>`, resolved to the global it names.
    Event {
        /// Index into [`Checked::globals`].
        global: u32,
    },
    /// Neither was given.
    Freewheeling,
}

/// A program instance bound to a task.
#[derive(Debug, Clone, PartialEq)]
pub struct CheckedProgramInstance {
    /// The instance name.
    pub name: IdentKey,
    /// Index into [`Checked::pous`] of the `PROGRAM` it instantiates.
    pub pou: u32,
}

/// A checked task.
#[derive(Debug, Clone, PartialEq)]
pub struct CheckedTask {
    /// Its name.
    pub name: IdentKey,
    /// What releases it.
    pub trigger: CheckedTrigger,
    /// Lower is more urgent.
    pub priority: u16,
    /// The programs bound to it, in declaration order.
    pub programs: Vec<CheckedProgramInstance>,
}

/// A checked configuration.
#[derive(Debug, Clone, PartialEq)]
pub struct CheckedConfiguration {
    /// Its name.
    pub name: IdentKey,
    /// Its tasks, across every resource, in declaration order.
    pub tasks: Vec<CheckedTask>,
    /// Programs declared with no task, which run freewheeling.
    pub untasked: Vec<CheckedProgramInstance>,
}

/// Everything semantic analysis learned about one compilation unit.
#[derive(Debug, Clone)]
pub struct Checked {
    /// Every resolved type.
    pub types: TypeArena,
    /// File-scope `VAR_GLOBAL` variables, in declaration order.
    pub globals: Vec<Symbol>,
    /// The POUs, in source order.
    pub pous: Vec<PouSymbols>,
    /// The type of every expression, indexed by node id.
    ///
    /// `None` means the expression could not be typed and a diagnostic was
    /// reported.
    pub expr_types: Vec<Option<TypeId>>,
    /// What every name resolved to, indexed by node id.
    pub resolutions: Vec<Option<Resolution>>,
    /// The constant value of every expression that has one, indexed by node id.
    ///
    /// Populated for anything salman can fold: literals, arithmetic on
    /// constants, and references to `CONSTANT` variables. Array bounds, `CASE`
    /// labels and initialisers all require one.
    pub const_values: Vec<Option<Value>>,
    /// The configurations, in source order.
    pub configurations: Vec<CheckedConfiguration>,
}

impl Checked {
    /// The resolved type of an expression node.
    #[must_use]
    pub fn type_of(&self, node: crate::ast::NodeId) -> Option<TypeId> {
        self.expr_types.get(node.index()).copied().flatten()
    }

    /// What a node resolved to.
    #[must_use]
    pub fn resolution(&self, node: crate::ast::NodeId) -> Option<Resolution> {
        self.resolutions.get(node.index()).copied().flatten()
    }

    /// The folded constant value of a node, if it has one.
    #[must_use]
    pub fn constant(&self, node: crate::ast::NodeId) -> Option<&Value> {
        self.const_values.get(node.index()).and_then(Option::as_ref)
    }

    /// A POU by name, compared case-insensitively.
    #[must_use]
    pub fn pou(&self, name: &str) -> Option<(u32, &PouSymbols)> {
        self.pous
            .iter()
            .enumerate()
            .find(|(_, p)| p.name.ident.eq_str(name))
            .and_then(|(i, p)| u32::try_from(i).ok().map(|i| (i, p)))
    }

    /// The standard block a function block instance type names, if it is one.
    #[must_use]
    pub fn native_block(&self, ty: TypeId) -> Option<NativeBlock> {
        match self.types.get(ty) {
            crate::types::TypeData::FunctionBlock { native, .. } => *native,
            _ => None,
        }
    }
}
