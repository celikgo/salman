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

use std::collections::{BTreeMap, BTreeSet};

use salman_core::clause;
use salman_core::diag::{Diagnostic, Diagnostics, Edit};
use salman_core::ident::IdentKey;
use salman_core::span::Span;
use salman_core::value::{ElementaryType, GenericType, Value};

use crate::address::{AddressSize, DirectAddress};
use crate::ast::{
    Arg, ArrayDim, BinaryOp, CaseLabel, CompilationUnit, ConfigurationDecl, Expr, ExprKind, Item,
    Name, NodeId, Pou, PouKind, ProgramInstance, Stmt, StmtKind, TaskDecl, TypeDecl, TypeDeclKind,
    TypeRef, UnaryOp, VarBlock, VarDecl, VarQualifiers, VarSection,
};
use crate::codes;
use crate::dialect::Dialect;
use crate::stdlib::{self, FieldRole, NativeBlock};
use crate::token::LiteralValue;
use crate::types::{
    ArrayBounds, BoolWidening, Field, OpResult, TypeArena, TypeData, TypeId, check_binary,
    check_unary, common_type, default_literal_type, implicit_conversion_allowed, integer_fits,
    integer_range,
};

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

// ---------------------------------------------------------------------------
// The entry point
// ---------------------------------------------------------------------------

/// Checks one compilation unit against one dialect.
///
/// Never panics and never fails: a [`Checked`] comes back however broken the
/// input was. That is deliberate. The compiler, the formatter and the editors
/// all run on the result, and a front end that has two failure modes — "here
/// are your errors" and "here is nothing at all" — makes every one of them
/// carry a second code path for the second mode. Anything that could not be
/// worked out is [`TypeArena::error`] or [`Resolution::Error`], which every
/// rule downstream accepts silently, so one bad declaration produces one
/// diagnostic rather than one per use.
///
/// Recursion in this pass is bounded by the tree's depth, which the parser has
/// already bounded at [`Dialect::max_nesting_depth`]; that is what makes it
/// safe on a file salman did not write.
///
/// [`Dialect::max_nesting_depth`]: crate::dialect::Dialect::max_nesting_depth
#[must_use]
pub fn check(unit: &CompilationUnit, dialect: &Dialect) -> (Checked, Diagnostics) {
    let count = usize::try_from(unit.node_count).unwrap_or(0);
    let mut checker = Checker {
        unit,
        dialect,
        diags: Diagnostics::new(),
        types: TypeArena::new(),
        globals: Vec::new(),
        pous: Vec::new(),
        expr_types: vec![None; count],
        resolutions: vec![None; count],
        const_values: vec![None; count],
        folded: vec![false; count],
        named_types: BTreeMap::new(),
        fb_types: BTreeMap::new(),
        configurations: Vec::new(),
        current: None,
        loop_depth: 0,
    };

    // Pass one: every declaration, so that a POU may name one declared further
    // down the file. Engineers write the program first and the helpers after
    // it, and a language that refused that would be a language nobody used.
    checker.collect_declarations();
    checker.check_recursion();
    // Pass two: the bodies.
    checker.check_bodies();
    checker.check_configurations();

    let checked = Checked {
        types: checker.types,
        globals: checker.globals,
        pous: checker.pous,
        expr_types: checker.expr_types,
        resolutions: checker.resolutions,
        const_values: checker.const_values,
        configurations: checker.configurations,
    };
    (checked, checker.diags)
}

/// The state one run of [`check`] carries.
struct Checker<'a> {
    unit: &'a CompilationUnit,
    dialect: &'a Dialect,
    diags: Diagnostics,
    types: TypeArena,
    globals: Vec<Symbol>,
    pous: Vec<PouSymbols>,
    expr_types: Vec<Option<TypeId>>,
    resolutions: Vec<Option<Resolution>>,
    const_values: Vec<Option<Value>>,
    /// Whether a node has already been through the constant folder, so that
    /// folding a chain of a thousand additions stays linear rather than
    /// quadratic. A `None` in `const_values` means "not constant"; this says
    /// whether that was decided or merely not yet asked.
    folded: Vec<bool>,
    /// User `TYPE` declarations, by lower-cased name.
    named_types: BTreeMap<IdentKey, TypeId>,
    /// Function block instance types, by lower-cased block name. Holds both
    /// the user-declared blocks and any standard block the unit names.
    fb_types: BTreeMap<IdentKey, TypeId>,
    configurations: Vec<CheckedConfiguration>,
    /// The POU whose body is being checked, if one is.
    current: Option<u32>,
    /// How many loops enclose the statement being checked, which is what makes
    /// `EXIT` and `CONTINUE` legal.
    loop_depth: u32,
}

// ---------------------------------------------------------------------------
// Pass one: declarations
// ---------------------------------------------------------------------------

impl Checker<'_> {
    fn collect_declarations(&mut self) {
        self.collect_pou_headers();
        self.collect_function_block_types();
        self.collect_type_declarations();
        self.collect_globals();
        self.collect_pou_variables();
    }

    /// Records every POU's name and kind before anything is resolved.
    fn collect_pou_headers(&mut self) {
        let unit = self.unit;
        for pou in unit.pous() {
            if let Some(existing) = self.pous.iter().find(|p| p.name.ident == pou.name.ident) {
                let earlier = existing.name.span;
                self.duplicate(&pou.name, earlier, "a program organization unit");
                // The first declaration wins and the second is dropped, so that
                // every later reference to the name means exactly one thing.
                continue;
            }
            self.pous.push(PouSymbols {
                name: pou.name.clone(),
                kind: pou.kind,
                span: pou.span,
                return_type: None,
                symbols: Vec::new(),
            });
        }
    }

    /// Interns one instance type per declared `FUNCTION_BLOCK`.
    ///
    /// Done before the `TYPE` declarations because a structure may hold a
    /// function block instance as a field.
    fn collect_function_block_types(&mut self) {
        for index in 0..u32::try_from(self.pous.len()).unwrap_or(0) {
            let Some(pou) = self.pous.get(index as usize) else {
                continue;
            };
            if pou.kind != PouKind::FunctionBlock {
                continue;
            }
            let name = pou.name.ident.to_key();
            let id = self.types.intern(TypeData::FunctionBlock {
                name: name.clone(),
                native: None,
                pou: Some(index),
            });
            self.fb_types.insert(name, id);
        }
    }

    /// Resolves the `TYPE ... END_TYPE` declarations, in dependency order.
    ///
    /// Dependency order rather than source order, because `TYPE` blocks are
    /// routinely written with the aggregate first and its parts after it, and
    /// because the arena interns types by structure and therefore cannot patch
    /// one up afterwards. A declaration that is part of a cycle is reported and
    /// resolved to the error type: a structure that contains itself has no
    /// finite size, and pretending otherwise would produce a compiler that
    /// loops rather than a diagnostic that explains.
    fn collect_type_declarations(&mut self) {
        let unit = self.unit;
        let decls: Vec<&TypeDecl> = unit
            .items
            .iter()
            .filter_map(|item| match item {
                Item::Types(decls) => Some(decls),
                _ => None,
            })
            .flatten()
            .collect();

        let mut declared: BTreeSet<IdentKey> = BTreeSet::new();
        let mut unique: Vec<&TypeDecl> = Vec::new();
        for decl in &decls {
            let key = decl.name.ident.to_key();
            if let Some(earlier) = unique
                .iter()
                .find(|d| d.name.ident == decl.name.ident)
                .map(|d| d.name.span)
            {
                self.duplicate(&decl.name, earlier, "a type");
                continue;
            }
            declared.insert(key);
            unique.push(decl);
        }

        // Kahn's algorithm over "this declaration names that one".
        let mut pending: Vec<&TypeDecl> = unique;
        while !pending.is_empty() {
            let mut progressed = false;
            let mut still_pending: Vec<&TypeDecl> = Vec::new();
            for decl in pending {
                let ready = type_decl_dependencies(decl, &declared)
                    .iter()
                    .all(|dep| self.named_types.contains_key(dep));
                if ready {
                    let id = self.resolve_type_decl(decl);
                    self.named_types.insert(decl.name.ident.to_key(), id);
                    progressed = true;
                } else {
                    still_pending.push(decl);
                }
            }
            if !progressed {
                for decl in &still_pending {
                    let names: Vec<String> = type_decl_dependencies(decl, &declared)
                        .iter()
                        .filter(|dep| !self.named_types.contains_key(*dep))
                        .map(ToString::to_string)
                        .collect();
                    self.diags.push(
                        Diagnostic::error(
                            codes::E_RECURSIVE_TYPE,
                            format!("type `{}` contains itself", decl.name),
                        )
                        .with_primary(
                            decl.name.span,
                            format!("this type is defined in terms of {}", join_names(&names)),
                        )
                        .with_note(
                            "A type that contains itself has no finite size, so salman cannot lay \
                             it out. A reference would break the cycle, and salman implements no \
                             references.",
                        )
                        .with_clause(clause::USER_DEFINED_DATA_TYPES),
                    );
                    let error = self.types.error();
                    self.named_types.insert(decl.name.ident.to_key(), error);
                }
                return;
            }
            pending = still_pending;
        }
    }

    /// Collects every `VAR_GLOBAL` in the unit into one scope.
    ///
    /// salman policy: the globals declared at file scope, inside a
    /// `CONFIGURATION` and inside a `RESOURCE` all land in the same list.
    /// IEC 61131-3:2013 §6.8 "Configuration elements" scopes the latter two to
    /// the configuration and the resource respectively; salman implements one
    /// configuration at v0.1, so the three scopes cannot yet be told apart by
    /// anything that would be observable. Flattening them is what lets a
    /// `TASK`'s `SINGLE` variable and a POU body name the same global.
    fn collect_globals(&mut self) {
        let unit = self.unit;
        let mut blocks: Vec<&VarBlock> = Vec::new();
        for item in &unit.items {
            match item {
                Item::Globals(block) => blocks.push(block),
                Item::Configuration(config) => {
                    blocks.extend(config.var_blocks.iter());
                    for resource in &config.resources {
                        blocks.extend(resource.var_blocks.iter());
                    }
                }
                _ => {}
            }
        }
        for block in blocks {
            // One declaration at a time, and each is in scope for the next:
            // `VAR_GLOBAL CONSTANT Size : INT := 4; Limit : INT := Size; END_VAR`
            // is ordinary code, and it only works if the folder can already see
            // `Size` when it reaches `Limit`.
            for decl in &block.decls {
                for symbol in self.symbols_of(block, decl) {
                    if let Some(earlier) = self
                        .globals
                        .iter()
                        .find(|s| s.name.ident == symbol.name.ident)
                        .map(|s| s.name.span)
                    {
                        self.duplicate(&symbol.name, earlier, "a global variable");
                        continue;
                    }
                    self.globals.push(symbol);
                }
            }
        }
    }

    /// Resolves each POU's variables and return type.
    fn collect_pou_variables(&mut self) {
        // Indices are walked rather than the POUs borrowed, because resolving a
        // type needs `&mut self` for the arena and for diagnostics.
        let unit = self.unit;
        let mut index = 0u32;
        for pou in unit.pous() {
            let Some(slot) = self.pous.get(index as usize) else {
                break;
            };
            if slot.name.ident != pou.name.ident {
                // A duplicate declaration that `collect_pou_headers` dropped.
                continue;
            }
            self.current = Some(index);
            let return_type = pou.return_type.as_ref().map(|ty| self.resolve_type_ref(ty));
            let mut symbols: Vec<Symbol> = Vec::new();
            for block in &pou.var_blocks {
                for decl in &block.decls {
                    for symbol in self.symbols_of(block, decl) {
                        if let Some(earlier) = symbols
                            .iter()
                            .find(|s| s.name.ident == symbol.name.ident)
                            .map(|s| s.name.span)
                        {
                            self.duplicate(&symbol.name, earlier, "a variable");
                            continue;
                        }
                        symbols.push(symbol);
                    }
                    // Each declaration is installed as it is resolved, so that
                    // the next one's initialiser and the next array bound can
                    // name a CONSTANT this one declared.
                    if let Some(slot) = self.pous.get_mut(index as usize) {
                        slot.symbols.clone_from(&symbols);
                    }
                }
            }
            if let Some(slot) = self.pous.get_mut(index as usize) {
                slot.return_type = return_type;
                slot.symbols = symbols;
            }
            self.current = None;
            index = index.saturating_add(1);
        }
    }

    /// Every symbol one declaration in a `VAR ... END_VAR` block declares.
    ///
    /// One declaration rather than the whole block, so that the caller can put
    /// each name in scope before resolving the next: `Size` has to be visible
    /// when the folder reaches `ARRAY [1..Size]` on the line below it.
    fn symbols_of(&mut self, block: &VarBlock, decl: &VarDecl) -> Vec<Symbol> {
        let ty = self.resolve_type_ref(&decl.ty);
        let init = self.declared_initial_value(decl, ty);
        let mut out = Vec::new();
        for (position, name) in decl.names.iter().enumerate() {
            // EN and ENO belong to the calling convention, not to a POU. A
            // variable of either name would shadow the execution control on
            // every call to this POU, and the name would then mean two things.
            if name.ident.eq_str("EN") || name.ident.eq_str("ENO") {
                self.diags.push(
                    Diagnostic::error(
                        codes::E_RESERVED_PARAMETER_NAME,
                        format!("`{name}` cannot be declared as a variable"),
                    )
                    .with_primary(name.span, "this name belongs to the calling convention")
                    .with_note(
                        "IEC 61131-3:2013 Table 18 \"Execution control graphically using EN and \
                         ENO\" (Ed 3.0) makes EN and ENO available on every call without being \
                         declared: EN decides whether the call happens, ENO reports whether it \
                         did. salman implements them, so a variable of the same name would mean \
                         two different things at one call site.",
                    )
                    .with_clause(clause::TABLE_EN_ENO),
                );
                continue;
            }
            // `A, B AT %IX0.0 : BOOL;` would put two variables at one address,
            // so only a single-name declaration keeps the address.
            let address = if decl.names.len() == 1 {
                decl.located_at.clone()
            } else {
                if position == 0 && decl.located_at.is_some() {
                    self.diags.push(
                        Diagnostic::error(
                            codes::E_DUPLICATE_DECLARATION,
                            "several variables cannot share one direct address",
                        )
                        .with_primary(
                            decl.located_at_span.unwrap_or(decl.span),
                            "this `AT` clause names one location",
                        )
                        .with_note(
                            "Declare each located variable on a line of its own, so that it is \
                             visible which name owns which address.",
                        )
                        .with_clause(clause::DIRECTLY_REPRESENTED_VARIABLES),
                    );
                }
                None
            };
            out.push(Symbol {
                name: name.clone(),
                section: block.section,
                qualifiers: block.qualifiers,
                ty,
                address,
                init: init.clone(),
            });
        }
        out
    }

    /// Type-checks and folds a declaration's initialiser.
    fn declared_initial_value(&mut self, decl: &VarDecl, ty: TypeId) -> Option<Value> {
        let init = decl.init.as_ref()?;
        let complaints = self.diags.len();
        let value_ty = self.expr(init, Some(ty));
        self.check_assignable(value_ty, ty, init.span, "this initial value");
        let value = self.fold(init)?;
        let coerced = self.coerce(&value, ty);
        // Folding produced something, but not something this type can hold.
        // The assignability check above has already said so.
        coerced.as_ref()?;
        // Only when nothing above has complained: an untyped literal already
        // reports its own subrange and length failures, and one mistake is
        // worth one diagnostic.
        if self.diags.len() == complaints
            && let Some(value) = coerced.as_ref()
        {
            self.check_declared_constraint(value, ty, init.span);
        }
        if let Some(slot) = self.const_values.get_mut(init.id.index()) {
            slot.clone_from(&coerced);
        }
        coerced
    }

    /// Checks a folded initial value against the constraint its declared type
    /// carries.
    ///
    /// `Body::coerce` in the compiler is the single place a value becomes a
    /// value of a declared type **at run time**, and a declared initial value
    /// never reaches it: it is written straight into the slot before the first
    /// scan. Without this, `Level : INT (0..100) := INT#200;` started outside
    /// its own declared range and the run-time check that exists to catch
    /// exactly that never saw the value — and `Level := 200;` a line later
    /// faulted, on a variable already holding 200.
    ///
    /// A bare literal reports itself, in `integer_literal_type` and
    /// `string_literal_type`, with a span on the literal. This catches
    /// everything else that folds: a typed literal such as `INT#200`, an
    /// arithmetic expression, and a `CONSTANT` named on the line above.
    fn check_declared_constraint(&mut self, value: &Value, ty: TypeId, span: Span) {
        enum Promise {
            Range(i64, i64),
            Length(u32),
            OneOf(Vec<i64>),
        }
        let promise = match self.types.get(ty) {
            TypeData::Subrange { low, high, .. } => Promise::Range(*low, *high),
            TypeData::Str { max_len, .. } => Promise::Length(*max_len),
            TypeData::Enum { values, .. } => {
                Promise::OneOf(values.iter().map(|(_, value)| *value).collect())
            }
            _ => return,
        };
        match promise {
            Promise::Range(low, high) => {
                let Some(number) = value.as_i64() else {
                    return;
                };
                if number < low || number > high {
                    self.diags.push(
                        Diagnostic::error(
                            codes::E_LITERAL_DOES_NOT_FIT,
                            format!("{number} is outside the subrange {low}..{high}"),
                        )
                        .with_primary(span, "this initial value is not one the subrange holds")
                        .with_note(
                            "A declared initial value is written into the variable before the \
                             first scan, so no run-time check sees it. A variable that starts \
                             outside its own declared range would fault on being assigned the \
                             value it already holds.",
                        )
                        .with_clause(clause::USER_DEFINED_DATA_TYPES),
                    );
                }
            }
            Promise::Length(max_len) => {
                let length = match value {
                    Value::String(bytes) => bytes.len(),
                    Value::WString(units) => units.len(),
                    _ => return,
                };
                if length > max_len as usize {
                    self.diags.push(
                        Diagnostic::error(
                            codes::E_LITERAL_DOES_NOT_FIT,
                            format!(
                                "this initial value is {length} characters long, and the target \
                                 holds {max_len}"
                            ),
                        )
                        .with_primary(span, "too long for its declared type")
                        .with_note(
                            "An assignment to a shorter string keeps the characters that fit. A \
                             declaration is not an assignment: it says how long the variable is, \
                             and an initial value that contradicts it is a mistake worth \
                             reporting rather than quietly cutting.",
                        )
                        .with_clause(clause::CHARACTER_STRING_LITERALS),
                    );
                }
            }
            Promise::OneOf(permitted) => {
                let Some(number) = value.as_i64() else {
                    return;
                };
                if !permitted.contains(&number) {
                    let list = permitted
                        .iter()
                        .map(i64::to_string)
                        .collect::<Vec<_>>()
                        .join(", ");
                    self.diags.push(
                        Diagnostic::error(
                            codes::E_LITERAL_DOES_NOT_FIT,
                            format!("{number} is not one of the enumeration's values ({list})"),
                        )
                        .with_primary(span, "this initial value is not a value of this type")
                        .with_note(
                            "An enumeration is a base type and a set of legal values. Naming the \
                             value — `Colour#Green`, or `Green` where the context settles the \
                             type — says which one is meant and cannot be off by one.",
                        )
                        .with_clause(clause::USER_DEFINED_DATA_TYPES),
                    );
                }
            }
        }
    }
}

/// The user type names one `TYPE` declaration depends on.
fn type_decl_dependencies(decl: &TypeDecl, declared: &BTreeSet<IdentKey>) -> Vec<IdentKey> {
    let mut out = Vec::new();
    match &decl.kind {
        TypeDeclKind::Alias(ty) => type_ref_dependencies(ty, declared, &mut out),
        TypeDeclKind::Struct(fields) => {
            for field in fields {
                type_ref_dependencies(&field.ty, declared, &mut out);
            }
        }
        TypeDeclKind::Enum { .. } => {}
        TypeDeclKind::Subrange { base, .. } => type_ref_dependencies(base, declared, &mut out),
        TypeDeclKind::Array { element, .. } => type_ref_dependencies(element, declared, &mut out),
    }
    // A declaration naming itself is a cycle of one; leaving it in is what
    // makes `TYPE T : STRUCT a : T; END_STRUCT; END_TYPE` reportable.
    out
}

/// The user type names one written type depends on.
fn type_ref_dependencies(ty: &TypeRef, declared: &BTreeSet<IdentKey>, out: &mut Vec<IdentKey>) {
    match ty {
        TypeRef::Named(name) => {
            let key = name.ident.to_key();
            if declared.contains(&key) {
                out.push(key);
            }
        }
        TypeRef::Array { element, .. } => type_ref_dependencies(element, declared, out),
        TypeRef::Subrange { base, .. } => type_ref_dependencies(base, declared, out),
        TypeRef::Elementary { .. } | TypeRef::String { .. } | TypeRef::Error(_) => {}
    }
}

// ---------------------------------------------------------------------------
// Type resolution
// ---------------------------------------------------------------------------

impl Checker<'_> {
    /// Resolves one written type to an interned type.
    fn resolve_type_ref(&mut self, ty: &TypeRef) -> TypeId {
        match ty {
            TypeRef::Elementary { ty, .. } => self.types.elementary(*ty),
            TypeRef::String { ty, max_len, span } => {
                let wide = *ty == ElementaryType::WString;
                let max_len = self.string_length(max_len.as_deref(), *span);
                self.types.intern(TypeData::Str { wide, max_len })
            }
            TypeRef::Named(name) => self.resolve_named_type(name),
            TypeRef::Array { dims, element, .. } => {
                let element = self.resolve_type_ref(element);
                self.array_type(dims, element)
            }
            TypeRef::Subrange {
                base,
                low,
                high,
                span,
            } => {
                let base = self.resolve_type_ref(base);
                self.subrange_type(base, low, high, *span)
            }
            // The parser has already said what was wrong here.
            TypeRef::Error(_) => self.types.error(),
        }
    }

    /// The declared maximum length of a `STRING`.
    fn string_length(&mut self, max_len: Option<&Expr>, span: Span) -> u32 {
        let default = u32::from(self.dialect.default_string_length);
        let Some(expr) = max_len else {
            return default;
        };
        let Some(value) = self.const_int(expr, "a STRING length") else {
            return default;
        };
        let limit = i128::try_from(salman_core::value::MAX_STRING_LEN).unwrap_or(i128::MAX);
        if value < 1 || value > limit {
            self.diags.push(
                Diagnostic::error(
                    codes::E_BAD_STRING_LENGTH,
                    format!("a STRING length of {value} is not usable"),
                )
                .with_primary(
                    expr.span.join(span),
                    format!("the length must be between 1 and {limit}"),
                )
                .with_note(
                    "The ceiling is salman's, not the standard's: string values are built from \
                     files and device traffic salman did not produce, and a bound is what keeps a \
                     malformed length from becoming an unbounded allocation.",
                )
                .with_clause(clause::ELEMENTARY_DATA_TYPES),
            );
            return default;
        }
        u32::try_from(value).unwrap_or(default)
    }

    /// Builds an array type from its written dimensions.
    fn array_type(&mut self, dims: &[ArrayDim], element: TypeId) -> TypeId {
        if dims.is_empty() {
            // Only reachable after the parser has refused `ARRAY [*]`.
            return self.types.error();
        }
        let mut bounds = Vec::with_capacity(dims.len());
        for dim in dims {
            let low = self.const_int(&dim.low, "an array bound");
            let high = self.const_int(&dim.high, "an array bound");
            let (Some(low), Some(high)) = (low, high) else {
                return self.types.error();
            };
            let (Ok(low), Ok(high)) = (i64::try_from(low), i64::try_from(high)) else {
                self.bad_bounds(dim.span, low, high, "does not fit a 64-bit index");
                return self.types.error();
            };
            if high < low {
                self.bad_bounds(
                    dim.span,
                    i128::from(low),
                    i128::from(high),
                    "is empty, because its upper bound is below its lower bound",
                );
                return self.types.error();
            }
            bounds.push(ArrayBounds { low, high });
        }
        self.types.intern(TypeData::Array {
            element,
            dims: bounds,
        })
    }

    fn bad_bounds(&mut self, span: Span, low: i128, high: i128, what: &str) {
        self.diags.push(
            Diagnostic::error(
                codes::E_BAD_ARRAY_BOUNDS,
                format!("the dimension {low}..{high} {what}"),
            )
            .with_primary(span, "an array dimension must hold at least one element")
            .with_clause(clause::USER_DEFINED_DATA_TYPES),
        );
    }

    /// Builds a subrange type from its written bounds.
    fn subrange_type(&mut self, base: TypeId, low: &Expr, high: &Expr, span: Span) -> TypeId {
        if self.types.is_error(base) {
            return base;
        }
        let Some(elementary) = self.types.as_elementary(base) else {
            let detail = format!("{} has no ordering", self.types.describe(base));
            self.bad_subrange(span, &detail);
            return self.types.error();
        };
        if !GenericType::AnyInt.contains(elementary) {
            self.bad_subrange(
                span,
                &format!("a subrange of {elementary} is not a subrange of an integer type"),
            );
            return self.types.error();
        }
        let low_value = self.const_int(low, "a subrange bound");
        let high_value = self.const_int(high, "a subrange bound");
        let (Some(low_value), Some(high_value)) = (low_value, high_value) else {
            return self.types.error();
        };
        if high_value < low_value {
            self.bad_subrange(
                span,
                &format!("the range {low_value}..{high_value} is empty"),
            );
            return self.types.error();
        }
        for bound in [low_value, high_value] {
            if !integer_fits(bound, elementary) {
                self.bad_subrange(span, &format!("{bound} does not fit {elementary}"));
                return self.types.error();
            }
        }
        let (Ok(low_value), Ok(high_value)) = (i64::try_from(low_value), i64::try_from(high_value))
        else {
            self.bad_subrange(span, "the bounds do not fit a 64-bit range");
            return self.types.error();
        };
        self.types.intern(TypeData::Subrange {
            base: elementary,
            low: low_value,
            high: high_value,
        })
    }

    fn bad_subrange(&mut self, span: Span, detail: &str) {
        self.diags.push(
            Diagnostic::error(
                codes::E_BAD_SUBRANGE,
                format!("this subrange is not usable: {detail}"),
            )
            .with_primary(
                span,
                "a subrange restricts an integer type to a range of it",
            )
            .with_clause(clause::USER_DEFINED_DATA_TYPES),
        );
    }

    /// Resolves a name written where a type was expected.
    ///
    /// Three things can answer to a name here: a user `TYPE`, a
    /// `FUNCTION_BLOCK` declared in this unit, and one of the standard blocks.
    fn resolve_named_type(&mut self, name: &Name) -> TypeId {
        let key = name.ident.to_key();
        if let Some(id) = self.named_types.get(&key) {
            return *id;
        }
        if let Some(id) = self.fb_types.get(&key) {
            return *id;
        }
        if let Some(native) = NativeBlock::lookup(name.as_str()) {
            let id = self.types.intern(TypeData::FunctionBlock {
                name: key.clone(),
                native: Some(native),
                pou: None,
            });
            self.fb_types.insert(key, id);
            return id;
        }
        let candidates: Vec<String> = self
            .named_types
            .keys()
            .map(ToString::to_string)
            .chain(self.fb_types.keys().map(ToString::to_string))
            .chain(NativeBlock::all().iter().map(|b| b.name().to_string()))
            .collect();
        let mut diag = Diagnostic::error(
            codes::E_UNKNOWN_TYPE,
            format!("no type named `{name}` is declared"),
        )
        .with_primary(name.span, "not a type salman knows")
        .with_clause(clause::DATA_TYPES);
        if let Some(suggestion) = closest(name.as_str(), &candidates) {
            diag = diag.with_suggestion(
                format!("a declared type is spelled `{suggestion}`"),
                vec![Edit {
                    span: name.span,
                    replacement: suggestion,
                }],
            );
        }
        self.diags.push(diag);
        self.types.error()
    }

    /// Resolves one `TYPE` declaration to an interned type.
    fn resolve_type_decl(&mut self, decl: &TypeDecl) -> TypeId {
        match &decl.kind {
            // An alias is the type it names. salman does not make a distinct
            // type of it: IEC 61131-3:2013 §6.4.4 "User-defined data types"
            // gives no rule salman could verify that would make `Speed : INT`
            // incompatible with `INT`, and inventing one would refuse code
            // every vendor accepts.
            TypeDeclKind::Alias(ty) => self.resolve_type_ref(ty),
            TypeDeclKind::Struct(fields) => self.struct_type(decl, fields),
            TypeDeclKind::Enum { base, values } => self.enum_type(decl, *base, values),
            TypeDeclKind::Subrange { base, low, high } => {
                let base = self.resolve_type_ref(base);
                self.subrange_type(base, low, high, decl.span)
            }
            TypeDeclKind::Array { dims, element } => {
                let element = self.resolve_type_ref(element);
                self.array_type(dims, element)
            }
        }
    }

    fn struct_type(&mut self, decl: &TypeDecl, fields: &[VarDecl]) -> TypeId {
        let mut out: Vec<Field> = Vec::new();
        for field in fields {
            let ty = self.resolve_type_ref(&field.ty);
            for name in &field.names {
                let key = name.ident.to_key();
                if let Some(earlier) = fields
                    .iter()
                    .flat_map(|f| f.names.iter())
                    .find(|n| n.ident == name.ident)
                    .map(|n| n.span)
                    && out.iter().any(|f| f.name == key)
                {
                    self.duplicate(name, earlier, "a field");
                    continue;
                }
                out.push(Field { name: key, ty });
            }
        }
        self.types.intern(TypeData::Struct {
            name: decl.name.ident.to_key(),
            fields: out,
        })
    }

    fn enum_type(
        &mut self,
        decl: &TypeDecl,
        base: Option<ElementaryType>,
        values: &[crate::ast::EnumValue],
    ) -> TypeId {
        // salman policy: an enumeration that names no base type is stored in
        // INT. The standard leaves the representation to the implementation,
        // and INT is what the dialects salman looked at use.
        let base = base.unwrap_or(ElementaryType::Int);
        if !GenericType::AnyInt.contains(base) {
            self.diags.push(
                Diagnostic::error(
                    codes::E_UNKNOWN_TYPE,
                    format!("an enumeration cannot be stored in {base}"),
                )
                .with_primary(decl.name.span, "the base type must be an integer type")
                .with_clause(clause::USER_DEFINED_DATA_TYPES),
            );
            return self.types.error();
        }

        // A value with no explicit number continues from the previous one, and
        // the first is 0. Every implementation salman looked at does this and
        // it is what makes `(Red, Green, Blue)` mean 0, 1, 2.
        //
        // UNVERIFIED: salman could not confirm from a public source which
        // clause of IEC 61131-3:2013 states the continuation rule, so it is
        // cited here as §6.4.4 "User-defined data types" — the clause that
        // covers enumerations — rather than as a numbered rule salman can point
        // at.
        let mut next: i64 = 0;
        let mut out: Vec<(IdentKey, i64)> = Vec::new();
        for value in values {
            let number = match &value.value {
                Some(expr) => match self.const_int(expr, "an enumeration value") {
                    Some(number) => i64::try_from(number).unwrap_or(next),
                    None => next,
                },
                None => next,
            };
            if !integer_fits(i128::from(number), base) {
                self.diags.push(
                    Diagnostic::error(
                        codes::E_LITERAL_DOES_NOT_FIT,
                        format!("the enumeration value {number} does not fit {base}"),
                    )
                    .with_primary(value.name.span, format!("`{}` is out of range", value.name))
                    .with_clause(clause::USER_DEFINED_DATA_TYPES),
                );
            }
            let key = value.name.ident.to_key();
            if let Some(earlier) = values
                .iter()
                .find(|v| v.name.ident == value.name.ident)
                .map(|v| v.name.span)
                && out.iter().any(|(name, _)| *name == key)
            {
                self.duplicate(&value.name, earlier, "an enumeration value");
            } else {
                out.push((key, number));
            }
            next = number.saturating_add(1);
        }
        self.types.intern(TypeData::Enum {
            name: decl.name.ident.to_key(),
            base,
            values: out,
        })
    }

    fn duplicate(&mut self, name: &Name, earlier: Span, what: &str) {
        self.diags.push(
            Diagnostic::error(
                codes::E_DUPLICATE_DECLARATION,
                format!("`{name}` is declared twice"),
            )
            .with_primary(
                name.span,
                format!("this declares {what} that already exists"),
            )
            .with_secondary(earlier, "the first declaration is here")
            .with_note(
                "IEC identifiers compare without regard to case, so `Motor` and `MOTOR` are one \
                 name.",
            )
            .with_clause(clause::VARIABLES),
        );
    }
}

// ---------------------------------------------------------------------------
// Name resolution
// ---------------------------------------------------------------------------

impl Checker<'_> {
    /// What a name means, without saying anything about it.
    ///
    /// Pure, so that the constant folder and the call-graph builder can ask the
    /// same question the checker asks without producing a second diagnostic.
    /// The order is the one IEC scoping gives: the POU's own variables, then
    /// the file's globals, then the POUs.
    fn lookup_in(&self, pou: Option<u32>, name: &str) -> Option<Resolution> {
        if let Some(index) = pou
            && let Some(owner) = self.pous.get(index as usize)
        {
            if let Some((symbol, _)) = owner.symbol(name) {
                return Some(Resolution::Local { pou: index, symbol });
            }
            // Inside a FUNCTION, its own name is the result variable.
            // IEC 61131-3:2013 §6.6.2 "Functions".
            if owner.kind == PouKind::Function && owner.name.ident.eq_str(name) {
                return Some(Resolution::Function { pou: index });
            }
        }
        if let Some(index) = self.globals.iter().position(|s| s.name.ident.eq_str(name))
            && let Ok(symbol) = u32::try_from(index)
        {
            return Some(Resolution::Global { symbol });
        }
        if let Some((index, found)) = self
            .pous
            .iter()
            .enumerate()
            .find(|(_, p)| p.name.ident.eq_str(name))
            && found.kind == PouKind::Function
            && let Ok(index) = u32::try_from(index)
        {
            return Some(Resolution::Function { pou: index });
        }
        None
    }

    fn lookup(&self, name: &str) -> Option<Resolution> {
        self.lookup_in(self.current, name)
    }

    fn symbol_at(&self, pou: u32, symbol: u32) -> Option<&Symbol> {
        self.pous.get(pou as usize)?.symbols.get(symbol as usize)
    }

    fn global_at(&self, symbol: u32) -> Option<&Symbol> {
        self.globals.get(symbol as usize)
    }

    /// The type a resolution has, or the error type when it has none.
    fn type_of_resolution(&self, resolution: Resolution) -> TypeId {
        match resolution {
            Resolution::Local { pou, symbol } => self
                .symbol_at(pou, symbol)
                .map_or_else(|| self.types.error(), |s| s.ty),
            Resolution::Global { symbol } => self
                .global_at(symbol)
                .map_or_else(|| self.types.error(), |s| s.ty),
            Resolution::Function { pou } => self
                .pous
                .get(pou as usize)
                .and_then(|p| p.return_type)
                .unwrap_or_else(|| self.types.error()),
            Resolution::Member { owner, offset } => self.member_type(owner, offset),
            Resolution::EnumValue { ty, .. } => ty,
            Resolution::Error => self.types.error(),
        }
    }

    fn member_type(&self, owner: TypeId, offset: u32) -> TypeId {
        let index = offset as usize;
        match self.types.get(owner) {
            TypeData::Struct { fields, .. } => fields
                .get(index)
                .map_or_else(|| self.types.error(), |f| f.ty),
            TypeData::FunctionBlock {
                native: Some(block),
                ..
            } => stdlib::layout(*block)
                .get(index)
                .map_or_else(|| self.types.error(), |f| self.types.elementary(f.ty)),
            TypeData::FunctionBlock { pou: Some(pou), .. } => self
                .symbol_at(*pou, offset)
                .map_or_else(|| self.types.error(), |s| s.ty),
            _ => self.types.error(),
        }
    }

    fn record(&mut self, node: NodeId, resolution: Resolution) {
        if let Some(slot) = self.resolutions.get_mut(node.index()) {
            *slot = Some(resolution);
        }
    }

    /// The names that could have been meant where `name` was written.
    fn names_in_scope(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        if let Some(index) = self.current
            && let Some(owner) = self.pous.get(index as usize)
        {
            out.extend(owner.symbols.iter().map(|s| s.name.to_string()));
        }
        out.extend(self.globals.iter().map(|s| s.name.to_string()));
        out.extend(self.pous.iter().map(|p| p.name.to_string()));
        out
    }
}

// ---------------------------------------------------------------------------
// Expressions
// ---------------------------------------------------------------------------

impl Checker<'_> {
    /// Types one expression, recording its type and any constant value.
    ///
    /// Bidirectional: `expected` is what the context requires, and it is what
    /// decides the type of an untyped literal. Everything else ignores it.
    fn expr(&mut self, e: &Expr, expected: Option<TypeId>) -> TypeId {
        let ty = self.expr_kind(e, expected);
        if let Some(slot) = self.expr_types.get_mut(e.id.index()) {
            *slot = Some(ty);
        }
        if let Some(value) = self.fold(e)
            && let Some(coerced) = self.coerce(&value, ty)
            && let Some(slot) = self.const_values.get_mut(e.id.index())
        {
            *slot = Some(coerced);
        }
        ty
    }

    fn expr_kind(&mut self, e: &Expr, expected: Option<TypeId>) -> TypeId {
        match &e.kind {
            ExprKind::Literal(literal) => self.literal_type(e, literal, expected),
            ExprKind::Var(name) => self.var_type(e, name, expected),
            ExprKind::Direct(address) => Self::direct_address_type(&self.types, address),
            ExprKind::Member { base, field } => self.member_expr(e, base, field),
            ExprKind::Index { base, indices } => self.index_expr(base, indices),
            ExprKind::Deref(_) => {
                self.references_not_implemented(e.span, "the dereference operator `^`");
                self.types.error()
            }
            ExprKind::Unary { op, operand } => self.unary_expr(e, *op, operand, expected),
            ExprKind::Binary { op, lhs, rhs } => self.binary_expr(e, *op, lhs, rhs, expected),
            ExprKind::Call { callee, args } => self.call_expr(e, callee, args, false),
            ExprKind::EnumValue { ty, value } => self.qualified_enum_value(e, ty, value),
            ExprKind::Paren(inner) => self.expr(inner, expected),
            // The parser has already reported whatever went wrong here. Poison
            // stops it becoming a second complaint about the same text.
            ExprKind::Error => self.types.error(),
        }
    }

    /// The type of a directly represented variable.
    ///
    /// It comes from the size letter alone: `%IX0.0` is a bit and therefore
    /// `BOOL`, `%QW1` is a word. IEC 61131-3:2013 Table 16 "Directly
    /// represented variables".
    fn direct_address_type(types: &TypeArena, address: &DirectAddress) -> TypeId {
        types.elementary(match address.size {
            AddressSize::Bit => ElementaryType::Bool,
            AddressSize::Byte => ElementaryType::Byte,
            AddressSize::Word => ElementaryType::Word,
            AddressSize::DoubleWord => ElementaryType::Dword,
            AddressSize::LongWord => ElementaryType::Lword,
        })
    }

    fn references_not_implemented(&mut self, span: Span, what: &str) {
        self.diags.push(
            Diagnostic::error(
                codes::U_REFERENCES,
                format!("salman does not implement {what}"),
            )
            .with_primary(span, "references are not implemented")
            .with_note(
                "salman has no reference types, so there is nothing to dereference and nothing \
                 for the assignment attempt to test. This is a missing feature, not a type error: \
                 the code may well be correct, and salman cannot check it.",
            )
            .with_clause(clause::TABLE_ASSIGNMENT_ATTEMPT),
        );
    }

    /// The type of a literal.
    ///
    /// **salman policy, and the reason this function exists.** An untyped
    /// literal takes the type its context requires when the value fits it, and
    /// falls back to [`default_literal_type`] when there is no context. No
    /// standard default could be verified from a public source. This is what
    /// makes `x : SINT := 5;` — which every vendor accepts — legal without
    /// salman inventing a rule and calling it the standard's.
    fn literal_type(
        &mut self,
        e: &Expr,
        literal: &LiteralValue,
        expected: Option<TypeId>,
    ) -> TypeId {
        match literal {
            LiteralValue::Bool(_) => self.types.elementary(ElementaryType::Bool),
            LiteralValue::Int {
                magnitude,
                negative,
                declared,
            } => {
                let Ok(magnitude) = i128::try_from(*magnitude) else {
                    // The lexer has already reported this as out of range.
                    return self.types.elementary(default_literal_type(false));
                };
                let value = if *negative { -magnitude } else { magnitude };
                self.integer_literal_type(e, value, *declared, expected)
            }
            LiteralValue::Real { declared, .. } => {
                if let Some(declared) = declared {
                    return self.types.elementary(*declared);
                }
                if let Some(expected) = expected
                    && self
                        .types
                        .as_elementary(expected)
                        .is_some_and(|t| GenericType::AnyReal.contains(t))
                {
                    return expected;
                }
                self.types.elementary(default_literal_type(true))
            }
            LiteralValue::Duration { long, .. } => self.types.elementary(if *long {
                ElementaryType::LTime
            } else {
                ElementaryType::Time
            }),
            LiteralValue::Date(_) => self.types.elementary(ElementaryType::Date),
            LiteralValue::TimeOfDay(_) => self.types.elementary(ElementaryType::TimeOfDay),
            LiteralValue::DateAndTime(_) => self.types.elementary(ElementaryType::DateAndTime),
            LiteralValue::String(bytes) => {
                self.string_literal_type(e, bytes.len(), false, expected)
            }
            LiteralValue::WString(units) => {
                self.string_literal_type(e, units.len(), true, expected)
            }
        }
    }

    fn integer_literal_type(
        &mut self,
        e: &Expr,
        value: i128,
        declared: Option<ElementaryType>,
        expected: Option<TypeId>,
    ) -> TypeId {
        // A typed literal says what it is; the only question is whether the
        // value fits.
        if let Some(declared) = declared {
            if integer_range(declared).is_some() && !integer_fits(value, declared) {
                self.literal_does_not_fit(e.span, value, declared);
            }
            return self.types.elementary(declared);
        }

        if let Some(expected) = expected
            && let Some(target) = self.types.as_elementary(expected)
        {
            // An integer literal in a real context is that real.
            if GenericType::AnyReal.contains(target) {
                return expected;
            }
            if integer_range(target).is_some() {
                if !integer_fits(value, target) {
                    self.literal_does_not_fit(e.span, value, target);
                    return expected;
                }
                match self.types.get(expected) {
                    TypeData::Subrange { low, high, .. } => {
                        let (low, high) = (*low, *high);
                        if value < i128::from(low) || value > i128::from(high) {
                            self.diags.push(
                                Diagnostic::error(
                                    codes::E_LITERAL_DOES_NOT_FIT,
                                    format!("{value} is outside the subrange {low}..{high}"),
                                )
                                .with_primary(e.span, "this value is not one the subrange holds")
                                .with_clause(clause::USER_DEFINED_DATA_TYPES),
                            );
                        }
                    }
                    // An enumeration is a set of legal values, and a bare
                    // integer in an enumeration context is only legal when it
                    // is one of them. Without this, `Shade := 77;` on a
                    // three-value `Colour` was accepted and stored 77.
                    TypeData::Enum { values, .. } => {
                        let permitted: Vec<i64> = values.iter().map(|(_, value)| *value).collect();
                        if !permitted.contains(&(value as i64)) {
                            let list = permitted
                                .iter()
                                .map(i64::to_string)
                                .collect::<Vec<_>>()
                                .join(", ");
                            self.diags.push(
                                Diagnostic::error(
                                    codes::E_LITERAL_DOES_NOT_FIT,
                                    format!(
                                        "{value} is not one of the enumeration's values ({list})"
                                    ),
                                )
                                .with_primary(e.span, "this value is not a value of this type")
                                .with_note(
                                    "Naming the value — `Colour#Green`, or `Green` where the \
                                     context settles the type — says which one is meant and \
                                     cannot be off by one.",
                                )
                                .with_clause(clause::USER_DEFINED_DATA_TYPES),
                            );
                        }
                    }
                    _ => {}
                }
                return expected;
            }
        }

        let fallback = default_literal_type(false);
        if !integer_fits(value, fallback) {
            self.literal_does_not_fit(e.span, value, fallback);
        }
        self.types.elementary(fallback)
    }

    fn literal_does_not_fit(&mut self, span: Span, value: i128, ty: ElementaryType) {
        let range = integer_range(ty).map_or_else(
            || "no integer range".to_string(),
            |(low, high)| format!("{low}..{high}"),
        );
        self.diags.push(
            Diagnostic::error(
                codes::E_LITERAL_DOES_NOT_FIT,
                format!("{value} does not fit {ty}"),
            )
            .with_primary(span, format!("{ty} holds {range}"))
            .with_note(
                "salman gives an untyped literal the type its context requires and falls back to \
                 DINT when there is none. That is a salman policy: no standard default could be \
                 verified from a public source.",
            )
            .with_clause(clause::TABLE_ELEMENTARY_DATA_TYPES),
        );
    }

    fn string_literal_type(
        &mut self,
        e: &Expr,
        len: usize,
        wide: bool,
        expected: Option<TypeId>,
    ) -> TypeId {
        if let Some(expected) = expected
            && let TypeData::Str {
                wide: target_wide,
                max_len,
            } = *self.types.get(expected)
            && target_wide == wide
        {
            if len > max_len as usize {
                self.diags.push(
                    Diagnostic::error(
                        codes::E_LITERAL_DOES_NOT_FIT,
                        format!(
                            "this literal is {len} characters long, and the target holds {max_len}"
                        ),
                    )
                    .with_primary(e.span, "too long for its declared type")
                    .with_clause(clause::CHARACTER_STRING_LITERALS),
                );
            }
            return expected;
        }
        let default = u32::from(self.dialect.default_string_length);
        let max_len = u32::try_from(len).unwrap_or(default).max(default);
        self.types.intern(TypeData::Str { wide, max_len })
    }

    /// Resolves a name used as a value.
    fn var_type(&mut self, e: &Expr, name: &Name, expected: Option<TypeId>) -> TypeId {
        if let Some(resolution) = self.lookup(name.as_str()) {
            self.record(e.id, resolution);
            if let Resolution::Function { pou } = resolution
                && self.current != Some(pou)
            {
                self.diags.push(
                    Diagnostic::error(
                        codes::E_NOT_CALLABLE,
                        format!("`{name}` is a FUNCTION, and this does not call it"),
                    )
                    .with_primary(e.span, "a function is used by calling it")
                    .with_note(
                        "Inside a function, its own name is the variable its result is assigned \
                         to. Outside it, the name means the function itself, which is not a \
                         value.",
                    )
                    .with_clause(clause::FUNCTIONS),
                );
            }
            return self.type_of_resolution(resolution);
        }

        // An unqualified enumeration value, when the context says which
        // enumeration is meant. This is what lets a CASE arm be written
        // `Red:` rather than `Colour#Red:`.
        if let Some(expected) = expected
            && let Some((ty, value)) = self.enum_member(expected, name)
        {
            self.record(e.id, Resolution::EnumValue { ty, value });
            return ty;
        }

        self.record(e.id, Resolution::Error);
        let candidates = self.names_in_scope();
        let mut diag =
            Diagnostic::error(codes::E_UNKNOWN_NAME, format!("`{name}` is not declared"))
                .with_primary(
                    e.span,
                    "no variable, global or POU of this name is in scope",
                )
                .with_clause(clause::VARIABLES);
        if let Some(suggestion) = closest(name.as_str(), &candidates) {
            diag = diag.with_suggestion(
                format!("there is a `{suggestion}` in scope"),
                vec![Edit {
                    span: e.span,
                    replacement: suggestion,
                }],
            );
        }
        self.diags.push(diag);
        self.types.error()
    }

    /// The value of `name` in `ty`, when `ty` is an enumeration that has one.
    fn enum_member(&self, ty: TypeId, name: &Name) -> Option<(TypeId, i64)> {
        let key = name.ident.to_key();
        match self.types.get(ty) {
            TypeData::Enum { values, .. } => values
                .iter()
                .find(|(value, _)| *value == key)
                .map(|(_, value)| (ty, *value)),
            _ => None,
        }
    }

    /// `Colour#Red`.
    fn qualified_enum_value(&mut self, e: &Expr, ty: &Name, value: &Name) -> TypeId {
        let Some(id) = self.named_types.get(&ty.ident.to_key()).copied() else {
            self.record(e.id, Resolution::Error);
            return self.resolve_named_type(ty);
        };
        if let Some((ty, number)) = self.enum_member(id, value) {
            self.record(e.id, Resolution::EnumValue { ty, value: number });
            return ty;
        }
        self.record(e.id, Resolution::Error);
        let known: Vec<String> = match self.types.get(id) {
            TypeData::Enum { values, .. } => values.iter().map(|(v, _)| v.to_string()).collect(),
            _ => Vec::new(),
        };
        if known.is_empty() {
            self.diags.push(
                Diagnostic::error(
                    codes::E_UNKNOWN_MEMBER,
                    format!("`{ty}` is not an enumeration"),
                )
                .with_primary(e.span, "only an enumeration has qualified values")
                .with_clause(clause::USER_DEFINED_DATA_TYPES),
            );
        } else {
            self.diags.push(
                Diagnostic::error(
                    codes::E_UNKNOWN_MEMBER,
                    format!("`{ty}` has no value named `{value}`"),
                )
                .with_primary(e.span, format!("its values are {}", join_names(&known)))
                .with_clause(clause::USER_DEFINED_DATA_TYPES),
            );
        }
        self.types.error()
    }

    /// `a.b` — a structure field, or an input or output of a function block
    /// instance.
    fn member_expr(&mut self, e: &Expr, base: &Expr, field: &Name) -> TypeId {
        let base_ty = self.expr(base, None);
        if self.types.is_error(base_ty) {
            self.record(e.id, Resolution::Error);
            return base_ty;
        }
        match self.types.get(base_ty).clone() {
            TypeData::Struct { fields, .. } => {
                let key = field.ident.to_key();
                if let Some(offset) = fields
                    .iter()
                    .position(|f| f.name == key)
                    .and_then(|i| u32::try_from(i).ok())
                {
                    self.record(
                        e.id,
                        Resolution::Member {
                            owner: base_ty,
                            offset,
                        },
                    );
                    return self.member_type(base_ty, offset);
                }
                let names: Vec<String> = fields.iter().map(|f| f.name.to_string()).collect();
                let owner = self.types.describe(base_ty);
                self.unknown_member(e, field, &owner, &names);
                self.types.error()
            }
            TypeData::FunctionBlock {
                native: Some(block),
                ..
            } => self.native_block_member(e, base_ty, block, field),
            TypeData::FunctionBlock { pou: Some(pou), .. } => {
                self.user_block_member(e, base_ty, pou, field)
            }
            _ => {
                self.record(e.id, Resolution::Error);
                self.diags.push(
                    Diagnostic::error(
                        codes::E_NOT_AN_AGGREGATE,
                        format!("{} has no fields", self.types.describe(base_ty)),
                    )
                    .with_primary(
                        base.span,
                        "only a structure and a function block instance have fields",
                    )
                    .with_clause(clause::DATA_TYPES),
                );
                self.types.error()
            }
        }
    }

    fn native_block_member(
        &mut self,
        e: &Expr,
        owner: TypeId,
        block: NativeBlock,
        field: &Name,
    ) -> TypeId {
        let fields = stdlib::layout(block);
        let found = fields
            .iter()
            .find(|f| f.name.eq_ignore_ascii_case(field.as_str()));
        let Some(found) = found else {
            let names: Vec<String> = fields
                .iter()
                .filter(|f| f.role != FieldRole::Internal)
                .map(|f| f.name.to_string())
                .collect();
            self.unknown_member(e, field, block.name(), &names);
            return self.types.error();
        };
        if found.role == FieldRole::Internal {
            self.internal_field(e, field, block.name());
            return self.types.error();
        }
        let Some(offset) = stdlib::field_offset(block, field.as_str()) else {
            return self.types.error();
        };
        self.record(e.id, Resolution::Member { owner, offset });
        self.types.elementary(found.ty)
    }

    fn user_block_member(&mut self, e: &Expr, owner: TypeId, pou: u32, field: &Name) -> TypeId {
        let Some(block) = self.pous.get(pou as usize) else {
            return self.types.error();
        };
        let block_name = block.name.to_string();
        let Some((offset, symbol)) = block.symbol(field.as_str()) else {
            let names: Vec<String> = block
                .symbols
                .iter()
                .filter(|s| is_interface_section(s.section))
                .map(|s| s.name.to_string())
                .collect();
            self.unknown_member(e, field, &block_name, &names);
            return self.types.error();
        };
        let section = symbol.section;
        let ty = symbol.ty;
        if !is_interface_section(section) {
            self.internal_field(e, field, &block_name);
            return self.types.error();
        }
        self.record(e.id, Resolution::Member { owner, offset });
        ty
    }

    fn unknown_member(&mut self, e: &Expr, field: &Name, owner: &str, names: &[String]) {
        self.record(e.id, Resolution::Error);
        let mut diag = Diagnostic::error(
            codes::E_UNKNOWN_MEMBER,
            format!("`{owner}` has no field named `{field}`"),
        )
        .with_primary(field.span, format!("its fields are {}", join_names(names)))
        .with_clause(clause::DATA_TYPES);
        if let Some(suggestion) = closest(field.as_str(), names) {
            diag = diag.with_suggestion(
                format!("there is a field `{suggestion}`"),
                vec![Edit {
                    span: field.span,
                    replacement: suggestion,
                }],
            );
        }
        self.diags.push(diag);
    }

    /// Reading or writing a block's own state from outside it.
    ///
    /// **salman policy.** The internal fields of a function block are real
    /// storage and salman shows them in a watch list, because a timer whose
    /// internals you cannot see is a timer you cannot debug. Naming one in code
    /// is a different thing: it welds the caller to an implementation detail
    /// that the standard does not fix and that salman may change.
    fn internal_field(&mut self, e: &Expr, field: &Name, owner: &str) {
        self.record(e.id, Resolution::Error);
        self.diags.push(
            Diagnostic::error(
                codes::E_INTERNAL_FIELD,
                format!("`{field}` is internal to `{owner}` and cannot be named here"),
            )
            .with_primary(
                field.span,
                "only inputs and outputs are visible from outside",
            )
            .with_note(
                "This is a salman rule. salman shows a block's internals in a watch list, because \
                 a timer whose internals you cannot see is a timer you cannot debug — but code \
                 that reads them is welded to an implementation detail neither the standard nor \
                 salman promises to keep.",
            )
            .with_clause(clause::FUNCTION_BLOCKS),
        );
    }

    /// `Buffer[i]`, `Grid[i, j]`.
    fn index_expr(&mut self, base: &Expr, indices: &[Expr]) -> TypeId {
        let base_ty = self.expr(base, None);
        let dint = self.types.elementary(ElementaryType::Dint);
        let TypeData::Array { element, dims } = self.types.get(base_ty).clone() else {
            for index in indices {
                self.expr(index, Some(dint));
            }
            if !self.types.is_error(base_ty) {
                self.diags.push(
                    Diagnostic::error(
                        codes::E_NOT_AN_ARRAY,
                        format!("{} cannot be subscripted", self.types.describe(base_ty)),
                    )
                    .with_primary(base.span, "only an array has elements")
                    .with_clause(clause::USER_DEFINED_DATA_TYPES),
                );
            }
            return self.types.error();
        };

        if indices.len() != dims.len() {
            self.diags.push(
                Diagnostic::error(
                    codes::E_WRONG_SUBSCRIPT_COUNT,
                    format!(
                        "this array has {} dimension{}, and {} subscript{} were written",
                        dims.len(),
                        plural(dims.len()),
                        indices.len(),
                        plural(indices.len()),
                    ),
                )
                .with_primary(base.span, self.types.describe(base_ty))
                .with_clause(clause::USER_DEFINED_DATA_TYPES),
            );
        }

        for (position, index) in indices.iter().enumerate() {
            let ty = self.expr(index, Some(dint));
            match self.types.as_elementary(ty) {
                Some(elementary) if GenericType::AnyInt.contains(elementary) => {}
                Some(elementary) => {
                    self.diags.push(
                        Diagnostic::error(
                            codes::E_SUBSCRIPT_NOT_INTEGER,
                            format!("a subscript cannot be {elementary}"),
                        )
                        .with_primary(index.span, "an array is indexed by an integer")
                        .with_clause(clause::USER_DEFINED_DATA_TYPES),
                    );
                }
                None if !self.types.is_error(ty) => {
                    self.diags.push(
                        Diagnostic::error(
                            codes::E_SUBSCRIPT_NOT_INTEGER,
                            format!("a subscript cannot be {}", self.types.describe(ty)),
                        )
                        .with_primary(index.span, "an array is indexed by an integer")
                        .with_clause(clause::USER_DEFINED_DATA_TYPES),
                    );
                }
                None => {}
            }
            // A constant subscript outside the bounds is a fault salman can
            // find now rather than at three in the morning.
            if let Some(bounds) = dims.get(position)
                && let Some(value) = self.fold(index).as_ref().and_then(Value::as_i64)
                && (value < bounds.low || value > bounds.high)
            {
                self.diags.push(
                    Diagnostic::error(
                        codes::E_SUBSCRIPT_OUT_OF_BOUNDS,
                        format!(
                            "the subscript {value} is outside {}..{}",
                            bounds.low, bounds.high
                        ),
                    )
                    .with_primary(index.span, "this index can never be valid")
                    .with_note(
                        "A constant index outside the declared bounds is a compile-time error in \
                         salman, because the alternative is a fault on a running machine.",
                    )
                    .with_clause(clause::USER_DEFINED_DATA_TYPES),
                );
            }
        }
        element
    }

    fn unary_expr(
        &mut self,
        e: &Expr,
        op: UnaryOp,
        operand: &Expr,
        expected: Option<TypeId>,
    ) -> TypeId {
        // `-128` is an operator and a magnitude in the tree and one value to
        // the engineer who wrote it. SINT holds -128 and does not hold 128, so
        // the range check has to be done on the negated value — otherwise the
        // most negative value of every signed type would be unwritable.
        if op == UnaryOp::Neg
            && let Some(magnitude) = plain_integer_literal(operand)
        {
            let ty = self.integer_literal_type(e, -magnitude, None, expected);
            self.record_type_through_parentheses(operand, ty);
            return ty;
        }
        let operand_ty = self.expr(operand, expected);
        if self.types.is_error(operand_ty) {
            return operand_ty;
        }
        let Some(elementary) = self.types.as_elementary(operand_ty) else {
            self.outside_domain(e.span, op.text(), &[self.types.describe(operand_ty)], None);
            return self.types.error();
        };
        match check_unary(op, elementary) {
            OpResult::Ok(result) => self.types.elementary(result),
            OpResult::OutsideDomain(domain) => {
                self.outside_domain(e.span, op.text(), &[elementary.to_string()], Some(domain));
                self.types.error()
            }
            OpResult::NoCommonType => self.types.error(),
        }
    }

    /// The type an *operand* inherits from a context that wants `ty`.
    ///
    /// A subrange's bounds constrain what one variable may hold. An operand of
    /// an arithmetic expression is not that variable, so it inherits the width
    /// and nothing else, and the result is judged against the declaration where
    /// the value is known.
    ///
    /// An enumeration is **not** stripped, and the difference is deliberate: an
    /// enumeration's values are its identity rather than a bound on it, and it
    /// is what resolves an unqualified value name — strip it and `Shade =
    /// Green` no longer knows what `Green` is.
    fn value_type_of(&mut self, ty: TypeId) -> TypeId {
        match *self.types.get(ty) {
            TypeData::Subrange { base, .. } => self.types.elementary(base),
            _ => ty,
        }
    }

    fn binary_expr(
        &mut self,
        e: &Expr,
        op: BinaryOp,
        lhs: &Expr,
        rhs: &Expr,
        expected: Option<TypeId>,
    ) -> TypeId {
        // A comparison's result type says nothing about its operands, so the
        // context's expectation is not passed into them.
        //
        // Neither does a subrange bound or an enumeration's value set. Those
        // constrain what the **result** may be, not what the operands may be:
        // `V : INT (0..10) := 50 - 40;` is 10, which the range holds, and
        // passing the subrange down reported the 50 and the 40 as out of range
        // for a variable neither of them was ever going to be. Only the base
        // type is inherited, which is what makes `Count + 1` keep `Count`'s
        // width. The result is judged against the whole declaration by
        // `check_declared_constraint`, where the value is known.
        let inherited = if op.is_comparison() {
            None
        } else {
            expected.map(|ty| self.value_type_of(ty))
        };
        let left = self.expr(lhs, inherited);
        // The right operand is typed against the left, which is what makes
        // `Count + 1` keep `Count`'s type rather than promoting it to DINT.
        // ... and by the same argument the left operand's own type is passed on
        // for its width, not for its bounds: in `Level + 100` over
        // `Level : INT (0..10)`, the 100 is not a value of `Level`'s type.
        let right_expectation = if self.types.is_error(left) {
            inherited
        } else {
            Some(self.value_type_of(left))
        };
        let right = self.expr(rhs, right_expectation);
        if self.types.is_error(left) || self.types.is_error(right) {
            return self.types.error();
        }
        let (Some(left_elementary), Some(right_elementary)) = (
            self.types.as_elementary(left),
            self.types.as_elementary(right),
        ) else {
            self.no_common_type(e.span, op, left, right);
            return self.types.error();
        };
        match check_binary(op, left_elementary, right_elementary, self.bool_widening()) {
            OpResult::Ok(result) => {
                // The node's type is the common type; each operand keeps its
                // own, which is exactly what the compiler compares against to
                // decide where a conversion has to be emitted.
                self.types.elementary(result)
            }
            OpResult::NoCommonType => {
                self.no_common_type(e.span, op, left, right);
                self.types.error()
            }
            OpResult::OutsideDomain(domain) => {
                self.outside_domain(
                    e.span,
                    op.text(),
                    &[left_elementary.to_string(), right_elementary.to_string()],
                    Some(domain),
                );
                self.types.error()
            }
        }
    }

    /// Records one type on an expression and on every parenthesis round it.
    fn record_type_through_parentheses(&mut self, expr: &Expr, ty: TypeId) {
        if let Some(slot) = self.expr_types.get_mut(expr.id.index()) {
            *slot = Some(ty);
        }
        if let ExprKind::Paren(inner) = &expr.kind {
            self.record_type_through_parentheses(inner, ty);
        }
    }

    fn no_common_type(&mut self, span: Span, op: BinaryOp, left: TypeId, right: TypeId) {
        let left_name = self.types.describe(left);
        let right_name = self.types.describe(right);
        let mut diag = Diagnostic::error(
            codes::E_NO_COMMON_TYPE,
            format!(
                "`{}` has no meaning between {left_name} and {right_name}",
                op.text()
            ),
        )
        .with_primary(
            span,
            format!("neither {left_name} nor {right_name} converts to the other"),
        )
        .with_note(
            "salman converts implicitly only along IEC 61131-3:2013 Figure 12 \"Supported \
             implicit type conversions\". Write the conversion out where the figure has no edge.",
        )
        .with_clause(clause::FIGURE_IMPLICIT_CONVERSIONS);
        if self.mentions_bool(left, right) {
            diag = diag.with_dialect_rule(self.bool_widening_rule());
        }
        self.diags.push(diag);
    }

    fn outside_domain(
        &mut self,
        span: Span,
        op: &str,
        operands: &[String],
        domain: Option<GenericType>,
    ) {
        let accepts = domain.map_or_else(
            || "an elementary type".to_string(),
            |domain| domain.name().to_string(),
        );
        self.diags.push(
            Diagnostic::error(
                codes::E_OPERAND_OUTSIDE_DOMAIN,
                format!("`{op}` does not accept {}", join_names(operands)),
            )
            .with_primary(span, format!("`{op}` is defined on {accepts}"))
            .with_clause(clause::TABLE_ST_OPERATORS),
        );
    }

    fn mentions_bool(&self, left: TypeId, right: TypeId) -> bool {
        [left, right]
            .iter()
            .any(|ty| self.types.as_elementary(*ty) == Some(ElementaryType::Bool))
    }

    fn bool_widening(&self) -> BoolWidening {
        if self.dialect.bool_widens_to_bit_strings {
            BoolWidening::Permitted
        } else {
            BoolWidening::Refused
        }
    }

    fn bool_widening_rule(&self) -> String {
        self.dialect.rule(
            "BOOL widening to the bit strings",
            if self.dialect.bool_widens_to_bit_strings {
                "permitted, following one vendor's rendering of Figure 12"
            } else {
                "refused, because salman could not verify that Figure 12 has that edge"
            },
        )
    }
}

/// The magnitude of an untyped, unsigned integer literal, through any
/// parentheses round it.
fn plain_integer_literal(expr: &Expr) -> Option<i128> {
    match &expr.kind {
        ExprKind::Paren(inner) => plain_integer_literal(inner),
        ExprKind::Literal(LiteralValue::Int {
            magnitude,
            negative: false,
            declared: None,
        }) => i128::try_from(*magnitude).ok(),
        _ => None,
    }
}

/// Whether a function block's callers may see a variable in this section.
fn is_interface_section(section: VarSection) -> bool {
    matches!(
        section,
        VarSection::Input | VarSection::Output | VarSection::InOut
    )
}

// ---------------------------------------------------------------------------
// Calls
// ---------------------------------------------------------------------------

/// What a call turned out to be calling.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Callee {
    /// A user-declared `FUNCTION`, by index into the POU list.
    Function(u32),
    /// An instance of a function block, by its instance type.
    Instance(TypeId),
    /// Something that cannot be called. A diagnostic has been reported.
    Unusable,
}

impl Checker<'_> {
    /// Type-checks a call.
    ///
    /// `as_statement` says whether the call stands on its own. A function block
    /// call produces no value, so one used inside an expression is an error and
    /// one used as a statement is the ordinary case.
    fn call_expr(&mut self, e: &Expr, callee: &Expr, args: &[Arg], as_statement: bool) -> TypeId {
        match self.resolve_callee(callee) {
            Callee::Function(pou) => {
                let formals = pou_formals(self.pous.get(pou as usize));
                self.check_arguments(&formals, args, e.span, true);
                self.pous
                    .get(pou as usize)
                    .and_then(|p| p.return_type)
                    .unwrap_or_else(|| self.types.error())
            }
            Callee::Instance(ty) => {
                self.check_instance_call(ty, args, e.span);
                if !as_statement {
                    self.diags.push(
                        Diagnostic::error(
                            codes::E_FUNCTION_BLOCK_HAS_NO_VALUE,
                            format!("calling `{}` produces no value", self.types.describe(ty)),
                        )
                        .with_primary(e.span, "a function block call is a statement, not a value")
                        .with_note(
                            "Call the instance as a statement, then read the output you want off \
                             it: `Timer(IN := Start); Running := Timer.Q;`.",
                        )
                        .with_clause(clause::TABLE_FUNCTION_BLOCK_CALL),
                    );
                }
                self.types.error()
            }
            Callee::Unusable => {
                // The arguments are still checked, so that one unresolved
                // callee does not hide every error inside its own arguments.
                for arg in args {
                    match arg {
                        Arg::Positional(value) | Arg::Input { value, .. } => {
                            self.expr(value, None);
                        }
                        Arg::Output {
                            target: Some(target),
                            ..
                        } => {
                            self.expr(target, None);
                        }
                        Arg::Output { target: None, .. } => {}
                    }
                }
                self.types.error()
            }
        }
    }

    fn resolve_callee(&mut self, callee: &Expr) -> Callee {
        match &callee.kind {
            ExprKind::Var(name) => self.resolve_var_callee(callee, name),
            ExprKind::Member { .. } => {
                let ty = self.expr(callee, None);
                if matches!(self.types.get(ty), TypeData::FunctionBlock { .. }) {
                    Callee::Instance(ty)
                } else {
                    if !self.types.is_error(ty) {
                        let name = self.types.describe(ty);
                        self.not_callable(callee.span, &name);
                    }
                    Callee::Unusable
                }
            }
            ExprKind::Error => Callee::Unusable,
            _ => {
                self.not_callable(callee.span, "this expression");
                Callee::Unusable
            }
        }
    }

    fn resolve_var_callee(&mut self, callee: &Expr, name: &Name) -> Callee {
        if let Some(resolution) = self.lookup(name.as_str()) {
            self.record(callee.id, resolution);
            let ty = self.type_of_resolution(resolution);
            if let Some(slot) = self.expr_types.get_mut(callee.id.index()) {
                *slot = Some(ty);
            }
            return match resolution {
                Resolution::Function { pou } => Callee::Function(pou),
                Resolution::Local { .. } | Resolution::Global { .. } => {
                    if matches!(self.types.get(ty), TypeData::FunctionBlock { .. }) {
                        Callee::Instance(ty)
                    } else {
                        self.not_callable(callee.span, &format!("`{name}`"));
                        Callee::Unusable
                    }
                }
                _ => {
                    self.not_callable(callee.span, &format!("`{name}`"));
                    Callee::Unusable
                }
            };
        }

        self.record(callee.id, Resolution::Error);
        // The three cases worth their own sentence: a PROGRAM, a function block
        // type written where one of its instances belongs, and a standard block
        // used the same way.
        if let Some(pou) = self
            .pous
            .iter()
            .find(|p| p.name.ident.eq_str(name.as_str()))
        {
            let kind = pou.kind;
            let span = pou.span;
            match kind {
                PouKind::Program => self.diags.push(
                    Diagnostic::error(
                        codes::E_WRONG_CALL_TARGET,
                        format!("`{name}` is a PROGRAM and cannot be called"),
                    )
                    .with_primary(callee.span, "a PROGRAM is run by a task, not called")
                    .with_secondary(span, "declared here")
                    .with_note(
                        "Bind the program to a task in a CONFIGURATION. If what you wanted was \
                         shared code, make it a FUNCTION or a FUNCTION_BLOCK.",
                    )
                    .with_clause(clause::PROGRAMS),
                ),
                PouKind::FunctionBlock => self.diags.push(
                    Diagnostic::error(
                        codes::E_WRONG_CALL_TARGET,
                        format!("`{name}` is a FUNCTION_BLOCK type, not an instance"),
                    )
                    .with_primary(callee.span, "there is no instance here to call")
                    .with_secondary(span, "declared here")
                    .with_note(
                        "A function block keeps state, so it is called through an instance: \
                         declare `Motor : Conveyor;` in a VAR block and call `Motor(...)`.",
                    )
                    .with_clause(clause::FUNCTION_BLOCKS),
                ),
                PouKind::Function => {}
            }
            return Callee::Unusable;
        }
        if let Some(block) = NativeBlock::lookup(name.as_str()) {
            self.diags.push(
                Diagnostic::error(
                    codes::E_WRONG_CALL_TARGET,
                    format!(
                        "`{}` is a standard function block type, not an instance",
                        block.name()
                    ),
                )
                .with_primary(callee.span, "there is no instance here to call")
                .with_note(format!(
                    "Declare an instance — `Delay : {};` — and call that. The instance is where \
                     the block's state lives.",
                    block.name()
                ))
                .with_clause(clause::TABLE_FUNCTION_BLOCK_CALL),
            );
            return Callee::Unusable;
        }

        let candidates = self.names_in_scope();
        let mut diag =
            Diagnostic::error(codes::E_UNKNOWN_NAME, format!("`{name}` is not declared"))
                .with_primary(
                    callee.span,
                    "no function or instance of this name is in scope",
                )
                .with_clause(clause::FUNCTIONS);
        if let Some(suggestion) = closest(name.as_str(), &candidates) {
            diag = diag.with_suggestion(
                format!("there is a `{suggestion}` in scope"),
                vec![Edit {
                    span: callee.span,
                    replacement: suggestion,
                }],
            );
        }
        self.diags.push(diag);
        Callee::Unusable
    }

    fn not_callable(&mut self, span: Span, what: &str) {
        self.diags.push(
            Diagnostic::error(codes::E_NOT_CALLABLE, format!("{what} cannot be called"))
                .with_primary(
                    span,
                    "only a FUNCTION and a function block instance are callable",
                )
                .with_clause(clause::PROGRAM_ORGANIZATION_UNITS),
        );
    }

    /// Checks a call to a function block instance.
    ///
    /// Positional arguments are refused: IEC 61131-3:2013 Table 42 "Function
    /// block call" gives only the form that names each parameter, and there is
    /// no positional form to fall back on.
    fn check_instance_call(&mut self, ty: TypeId, args: &[Arg], span: Span) {
        self.check_no_duplicate_arguments(args, span);
        let block = self.types.get(ty).clone();
        let name = self.types.describe(ty);
        for arg in args {
            match arg {
                Arg::Positional(value) => {
                    self.expr(value, None);
                    self.diags.push(
                        Diagnostic::error(
                            codes::E_POSITIONAL_FUNCTION_BLOCK_ARGUMENT,
                            format!("`{name}` must be called with named parameters"),
                        )
                        .with_primary(value.span, "this argument names no parameter")
                        .with_note(
                            "IEC 61131-3:2013 Table 42 \"Function block call\" offers no \
                             non-formal — positional — form for a function block, so there is no \
                             order for salman to match this against. Write `IN := ...`.",
                        )
                        .with_clause(clause::TABLE_FUNCTION_BLOCK_CALL),
                    );
                }
                Arg::Input { name: field, value } => {
                    let expected = self
                        .execution_control(field, true, span)
                        .or_else(|| self.block_parameter(&block, field, true, span));
                    match expected {
                        Some(target) => {
                            let value_ty = self.expr(value, Some(target));
                            self.check_assignable(
                                value_ty,
                                target,
                                value.span,
                                &format!("the input `{field}`"),
                            );
                        }
                        None => {
                            self.expr(value, None);
                        }
                    }
                }
                Arg::Output {
                    name: field,
                    target,
                } => {
                    let source = self
                        .execution_control(field, false, span)
                        .or_else(|| self.block_parameter(&block, field, false, span));
                    if let Some(target) = target {
                        let target_ty = self.assign_target(target);
                        if let Some(source) = source {
                            self.check_assignable(
                                source,
                                target_ty,
                                target.span,
                                &format!("the output `{field}`"),
                            );
                        }
                    }
                }
            }
        }
    }

    /// Refuses a call that names one parameter twice.
    ///
    /// The compiler binds named arguments in order, so a repeated parameter
    /// quietly kept the last one written. For `EN` that meant
    /// `A(EN := TRUE, EN := FALSE)` skipping a call the reader can see enabled;
    /// for `ENO` it was worse, because the first `ENO => Ok` was never written
    /// at all and `Ok` held whatever it held before — a variable the engineer
    /// bound, silently left alone.
    ///
    /// There is no reading of IEC 61131-3 under which one parameter takes two
    /// arguments, so this is refused rather than given a rule.
    fn check_no_duplicate_arguments(&mut self, args: &[Arg], span: Span) {
        let mut seen: Vec<(String, Span)> = Vec::new();
        let mut repeats: Vec<(String, Span, Span)> = Vec::new();
        for arg in args {
            let name = match arg {
                Arg::Input { name, .. } | Arg::Output { name, .. } => name,
                Arg::Positional(_) => continue,
            };
            match seen
                .iter()
                .find(|(seen, _)| seen.eq_ignore_ascii_case(name.as_str()))
            {
                Some((_, first)) => repeats.push((name.to_string(), name.span, *first)),
                None => seen.push((name.as_str().to_string(), name.span)),
            }
        }
        for (name, here, first) in repeats {
            self.diags.push(
                Diagnostic::error(
                    codes::E_DUPLICATE_ARGUMENT,
                    format!("`{name}` is given an argument twice in this call"),
                )
                .with_primary(here, "this parameter already has an argument")
                .with_secondary(first, "it was given one here")
                .with_secondary(span, "in this call")
                .with_note(
                    "One parameter takes one argument. Binding it twice has no meaning, and the \
                     reading salman would otherwise fall into — the last one wins — makes the \
                     first argument invisible.",
                )
                .with_clause(clause::TABLE_FUNCTION_CALL),
            );
        }
    }

    /// Whether a named call parameter is one of the two implicit
    /// execution-control parameters, and which.
    ///
    /// IEC 61131-3:2013 Table 18 "Execution control graphically using EN and
    /// ENO" (Ed 3.0) makes `EN` and `ENO` part of the **calling convention**
    /// rather than something a POU declares. `EN` decides whether the call
    /// happens at all; `ENO` reports whether it did. They are therefore
    /// available on every function and function block call, and no POU may
    /// declare a variable of either name.
    ///
    /// Reporting `EN` used as an output, or `ENO` as an input, by name matters:
    /// `F(ENO := ok)` looks plausible and does the opposite of what it says.
    fn execution_control(&mut self, name: &Name, input: bool, span: Span) -> Option<TypeId> {
        let is_enable = name.ident.eq_str("EN");
        let is_enable_out = name.ident.eq_str("ENO");
        if !is_enable && !is_enable_out {
            return None;
        }
        let bool_ty = self.types.elementary(ElementaryType::Bool);
        if is_enable && !input {
            self.diags.push(
                Diagnostic::error(
                    codes::E_UNKNOWN_PARAMETER,
                    "`EN` is an input, not an output",
                )
                .with_primary(
                    name.span,
                    "write `EN := ...` to decide whether the call happens",
                )
                .with_secondary(span, "in this call")
                .with_clause(clause::TABLE_EN_ENO),
            );
            return Some(bool_ty);
        }
        if is_enable_out && input {
            self.diags.push(
                Diagnostic::error(
                    codes::E_UNKNOWN_PARAMETER,
                    "`ENO` is an output, not an input",
                )
                .with_primary(
                    name.span,
                    "write `ENO => ...` to read whether the call happened",
                )
                .with_secondary(span, "in this call")
                .with_clause(clause::TABLE_EN_ENO),
            );
            return Some(bool_ty);
        }
        Some(bool_ty)
    }

    /// The type of one named parameter of a function block instance.
    fn block_parameter(
        &mut self,
        block: &TypeData,
        field: &Name,
        input: bool,
        span: Span,
    ) -> Option<TypeId> {
        let (found, names) = match block {
            TypeData::FunctionBlock {
                native: Some(native),
                ..
            } => {
                let wanted = if input {
                    FieldRole::Input
                } else {
                    FieldRole::Output
                };
                let found = stdlib::layout(*native)
                    .iter()
                    .find(|f| f.name.eq_ignore_ascii_case(field.as_str()) && f.role == wanted)
                    .map(|f| self.types.elementary(f.ty));
                let names = stdlib::layout(*native)
                    .iter()
                    .filter(|f| f.role == wanted)
                    .map(|f| f.name.to_string())
                    .collect::<Vec<String>>();
                (found, names)
            }
            TypeData::FunctionBlock { pou: Some(pou), .. } => {
                let owner = self.pous.get(*pou as usize)?;
                let wanted = |section: VarSection| {
                    if input {
                        matches!(section, VarSection::Input | VarSection::InOut)
                    } else {
                        section == VarSection::Output
                    }
                };
                let found = owner
                    .symbols
                    .iter()
                    .find(|s| s.name.ident.eq_str(field.as_str()) && wanted(s.section))
                    .map(|s| s.ty);
                let names = owner
                    .symbols
                    .iter()
                    .filter(|s| wanted(s.section))
                    .map(|s| s.name.to_string())
                    .collect::<Vec<String>>();
                (found, names)
            }
            _ => return None,
        };
        if found.is_some() {
            return found;
        }
        let what = if input { "input" } else { "output" };
        let mut diag = Diagnostic::error(
            codes::E_UNKNOWN_PARAMETER,
            format!("this call has no {what} named `{field}`"),
        )
        .with_primary(
            field.span,
            format!("its {what}s are {}", join_names(&names)),
        )
        .with_secondary(span, "in this call")
        .with_clause(clause::TABLE_FUNCTION_BLOCK_CALL);
        if let Some(suggestion) = closest(field.as_str(), &names) {
            diag = diag.with_suggestion(
                format!("there is a parameter `{suggestion}`"),
                vec![Edit {
                    span: field.span,
                    replacement: suggestion,
                }],
            );
        }
        self.diags.push(diag);
        None
    }

    /// Checks the arguments of a call to a `FUNCTION`.
    ///
    /// Both argument forms are legal here: IEC 61131-3:2013 Table 20 "Function
    /// call" gives a positional list and a list that names each parameter. What
    /// is not legal is mixing them in one call, because then nothing says which
    /// position the named ones occupy.
    fn check_arguments(&mut self, formals: &[Formal], args: &[Arg], span: Span, require_all: bool) {
        self.check_no_duplicate_arguments(args, span);
        let positional = args
            .iter()
            .filter(|a| matches!(a, Arg::Positional(_)))
            .count();
        let named = args.len() - positional;
        if positional > 0 && named > 0 {
            self.diags.push(
                Diagnostic::error(
                    codes::E_MIXED_ARGUMENT_FORMS,
                    "this call mixes positional and named arguments",
                )
                .with_primary(span, "use one form or the other")
                .with_note(
                    "IEC 61131-3:2013 Table 20 \"Function call\" gives a positional form and a \
                     form that names each parameter. Mixing them leaves no rule for which \
                     position the named arguments take.",
                )
                .with_clause(clause::TABLE_FUNCTION_CALL),
            );
        }

        let inputs: Vec<&Formal> = formals.iter().filter(|f| f.input).collect();
        // Only when the call is purely positional: a mixed call has already
        // been refused, and counting its arguments would add a second
        // complaint about one mistake.
        if positional > 0 && named == 0 && positional != inputs.len() {
            self.diags.push(
                Diagnostic::error(
                    codes::E_WRONG_ARGUMENT_COUNT,
                    format!(
                        "this call passes {positional} argument{} and {} {} declared",
                        plural(positional),
                        inputs.len(),
                        if inputs.len() == 1 { "is" } else { "are" }
                    ),
                )
                .with_primary(
                    span,
                    format!(
                        "the parameters are {}",
                        join_names(
                            &inputs
                                .iter()
                                .map(|f| f.name.clone())
                                .collect::<Vec<String>>()
                        )
                    ),
                )
                .with_clause(clause::TABLE_FUNCTION_CALL),
            );
        }

        let mut bound: Vec<String> = Vec::new();
        let mut index = 0usize;
        for arg in args {
            match arg {
                Arg::Positional(value) => {
                    let formal = inputs.get(index);
                    index = index.saturating_add(1);
                    match formal {
                        Some(formal) => {
                            let value_ty = self.expr(value, Some(formal.ty));
                            self.check_assignable(
                                value_ty,
                                formal.ty,
                                value.span,
                                &format!("the parameter `{}`", formal.name),
                            );
                            bound.push(formal.name.clone());
                        }
                        None => {
                            self.expr(value, None);
                        }
                    }
                }
                Arg::Input { name, value } => {
                    if let Some(ty) = self.execution_control(name, true, span) {
                        let value_ty = self.expr(value, Some(ty));
                        self.check_assignable(value_ty, ty, value.span, "the parameter `EN`");
                        continue;
                    }
                    let formal = formals
                        .iter()
                        .find(|f| f.input && f.name.eq_ignore_ascii_case(name.as_str()))
                        .map(|formal| (formal.ty, formal.name.clone()));
                    if let Some((ty, formal_name)) = formal {
                        let value_ty = self.expr(value, Some(ty));
                        self.check_assignable(
                            value_ty,
                            ty,
                            value.span,
                            &format!("the parameter `{formal_name}`"),
                        );
                        bound.push(formal_name);
                    } else {
                        self.expr(value, None);
                        self.unknown_parameter(name, formals, true, span);
                    }
                }
                Arg::Output { name, target } => {
                    if let Some(ty) = self.execution_control(name, false, span) {
                        if let Some(target) = target {
                            let target_ty = self.assign_target(target);
                            self.check_assignable(ty, target_ty, target.span, "the output `ENO`");
                        }
                        continue;
                    }
                    let formal = formals
                        .iter()
                        .find(|f| !f.input && f.name.eq_ignore_ascii_case(name.as_str()))
                        .map(|f| (f.ty, f.name.clone()));
                    match (formal, target) {
                        (Some((ty, formal_name)), Some(target)) => {
                            let target_ty = self.assign_target(target);
                            self.check_assignable(
                                ty,
                                target_ty,
                                target.span,
                                &format!("the output `{formal_name}`"),
                            );
                        }
                        (Some(_), None) => {}
                        (None, target) => {
                            if let Some(target) = target {
                                self.expr(target, None);
                            }
                            self.unknown_parameter(name, formals, false, span);
                        }
                    }
                }
            }
        }

        if require_all && positional == 0 {
            let missing: Vec<String> = formals
                .iter()
                .filter(|f| f.input && !bound.iter().any(|b| b.eq_ignore_ascii_case(&f.name)))
                .map(|f| f.name.clone())
                .collect();
            if !missing.is_empty() {
                self.diags.push(
                    Diagnostic::error(
                        codes::E_WRONG_ARGUMENT_COUNT,
                        format!("this call leaves {} unbound", join_names(&missing)),
                    )
                    .with_primary(span, "a function needs a value for every parameter")
                    .with_note(
                        "A function keeps no state between calls, so an unbound parameter has no \
                         previous value to fall back on. A function block does, which is why the \
                         same omission is legal there.",
                    )
                    .with_clause(clause::FUNCTIONS),
                );
            }
        }
    }

    fn unknown_parameter(&mut self, name: &Name, formals: &[Formal], input: bool, span: Span) {
        let names: Vec<String> = formals
            .iter()
            .filter(|f| f.input == input)
            .map(|f| f.name.clone())
            .collect();
        let what = if input { "input" } else { "output" };
        let mut diag = Diagnostic::error(
            codes::E_UNKNOWN_PARAMETER,
            format!("this call has no {what} named `{name}`"),
        )
        .with_primary(name.span, format!("its {what}s are {}", join_names(&names)))
        .with_secondary(span, "in this call")
        .with_clause(clause::TABLE_FUNCTION_CALL);
        if let Some(suggestion) = closest(name.as_str(), &names) {
            diag = diag.with_suggestion(
                format!("there is a parameter `{suggestion}`"),
                vec![Edit {
                    span: name.span,
                    replacement: suggestion,
                }],
            );
        }
        self.diags.push(diag);
    }
}

/// One formal parameter of a POU, flattened out of its symbol table.
struct Formal {
    name: String,
    ty: TypeId,
    input: bool,
}

/// The formal parameters of a POU, in declaration order.
///
/// `VAR_IN_OUT` is counted as an input: it is written at the call site like
/// one. salman passes it by value and copies it back, which is a difference the
/// caller cannot see here but which `docs/CONFORMANCE.md` records.
fn pou_formals(pou: Option<&PouSymbols>) -> Vec<Formal> {
    let Some(pou) = pou else {
        return Vec::new();
    };
    pou.symbols
        .iter()
        .filter(|s| is_interface_section(s.section))
        .map(|s| Formal {
            name: s.name.to_string(),
            ty: s.ty,
            input: s.section != VarSection::Output,
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Assignment
// ---------------------------------------------------------------------------

impl Checker<'_> {
    /// Types the left-hand side of an assignment and checks it may be written.
    fn assign_target(&mut self, target: &Expr) -> TypeId {
        let ty = self.expr(target, None);
        if !is_assignable_form(target) {
            self.diags.push(
                Diagnostic::error(codes::E_NOT_ASSIGNABLE, "this cannot be assigned to")
                    .with_primary(
                        target.span,
                        "only a variable, an array element, a field or a direct address can be",
                    )
                    .with_clause(clause::TABLE_ST_STATEMENTS),
            );
            return self.types.error();
        }
        self.check_writable(target);
        ty
    }

    /// Reports an assignment to something the enclosing POU may not write.
    fn check_writable(&mut self, e: &Expr) {
        match &e.kind {
            ExprKind::Paren(inner) => self.check_writable(inner),
            ExprKind::Member { base, .. } | ExprKind::Index { base, .. } => {
                self.check_writable(base);
            }
            ExprKind::Var(name) => {
                let resolution = self.resolutions.get(e.id.index()).copied().flatten();
                let symbol = match resolution {
                    Some(Resolution::Local { pou, symbol }) => self.symbol_at(pou, symbol),
                    Some(Resolution::Global { symbol }) => self.global_at(symbol),
                    _ => None,
                };
                if let Some(symbol) = symbol {
                    let constant = symbol.qualifiers.constant;
                    let section = symbol.section;
                    let declared = symbol.name.span;
                    if constant {
                        self.not_writable(
                            e.span,
                            name,
                            declared,
                            "it is declared CONSTANT, and a constant keeps its value for the \
                             whole run",
                            clause::VARIABLES,
                        );
                    } else if !section.is_writable_by_pou() {
                        self.not_writable(
                            e.span,
                            name,
                            declared,
                            "it is a VAR_INPUT of this POU, which its caller writes and it reads",
                            clause::VARIABLE_SECTIONS,
                        );
                    }
                }
            }
            _ => {}
        }
    }

    fn not_writable(
        &mut self,
        span: Span,
        name: &Name,
        declared: Span,
        why: &str,
        clause: clause::ClauseRef,
    ) {
        self.diags.push(
            Diagnostic::error(
                codes::E_NOT_WRITABLE,
                format!("`{name}` cannot be assigned to: {why}"),
            )
            .with_primary(span, "this assignment is refused")
            .with_secondary(declared, "declared here")
            .with_clause(clause),
        );
    }

    /// Checks that a value of one type may be assigned to a target of another.
    fn check_assignable(&mut self, value: TypeId, target: TypeId, span: Span, what: &str) {
        if self.types.is_error(value) || self.types.is_error(target) || value == target {
            return;
        }
        let (Some(from), Some(to)) = (
            self.types.as_elementary(value),
            self.types.as_elementary(target),
        ) else {
            self.type_mismatch(span, value, target, what, None);
            return;
        };
        if implicit_conversion_allowed(from, to, self.bool_widening()) {
            return;
        }
        // A conversion that exists in the other direction is a narrowing, and
        // saying so — with the name of the function that does it — is worth
        // more than a paragraph about the conversion graph.
        let narrowing = implicit_conversion_allowed(to, from, self.bool_widening());
        let suggestion = if narrowing {
            Some(format!("{from}_TO_{to}"))
        } else {
            None
        };
        self.type_mismatch(span, value, target, what, suggestion);
    }

    fn type_mismatch(
        &mut self,
        span: Span,
        value: TypeId,
        target: TypeId,
        what: &str,
        conversion: Option<String>,
    ) {
        let value_name = self.types.describe(value);
        let target_name = self.types.describe(target);
        let mut diag = Diagnostic::error(
            codes::E_TYPE_MISMATCH,
            format!("{what} is {target_name}, and this value is {value_name}"),
        )
        .with_primary(
            span,
            format!("{value_name} does not convert to {target_name} on its own"),
        )
        .with_clause(clause::FIGURE_IMPLICIT_CONVERSIONS);
        if let Some(conversion) = conversion {
            diag = diag
                .with_note(format!(
                    "This is a narrowing conversion: {value_name} holds values {target_name} \
                     cannot. IEC names the function that does it explicitly `{conversion}`. \
                     salman does not implement the standard conversion functions at v0.1, so \
                     until it does, the fix is to widen the target rather than narrow the value."
                ))
                .with_suggestion(
                    format!("write {conversion}(...) round the value"),
                    Vec::new(),
                );
        }
        if self.mentions_bool(value, target) {
            diag = diag.with_dialect_rule(self.bool_widening_rule());
        }
        self.diags.push(diag);
    }
}

/// Whether an expression names storage, and can therefore be assigned to.
fn is_assignable_form(e: &Expr) -> bool {
    match &e.kind {
        ExprKind::Var(_)
        | ExprKind::Direct(_)
        | ExprKind::Member { .. }
        | ExprKind::Index { .. } => true,
        ExprKind::Paren(inner) => is_assignable_form(inner),
        // An error node has already been reported; calling it unassignable
        // would be a second complaint about one piece of broken text.
        ExprKind::Error => true,
        _ => false,
    }
}

// ---------------------------------------------------------------------------
// Pass two: bodies
// ---------------------------------------------------------------------------

impl Checker<'_> {
    fn check_bodies(&mut self) {
        let unit = self.unit;
        let mut index = 0u32;
        for pou in unit.pous() {
            let Some(slot) = self.pous.get(index as usize) else {
                break;
            };
            if slot.name.ident != pou.name.ident {
                continue;
            }
            self.current = Some(index);
            self.loop_depth = 0;
            self.stmts(&pou.body);
            if pou.kind == PouKind::Function {
                self.check_function_result(pou);
            }
            self.current = None;
            index = index.saturating_add(1);
        }
    }

    /// Warns about a `FUNCTION` that never assigns its result.
    ///
    /// **salman policy: a warning, not an error.** Deciding whether every path
    /// through a body assigns the result needs a definite-assignment analysis,
    /// and salman does not have one at v0.1. What it can see is whether the
    /// name is ever assigned at all, which catches the mistake that actually
    /// happens — a function that computes into a local and forgets to hand it
    /// back — without failing a build over a branch salman cannot follow.
    fn check_function_result(&mut self, pou: &Pou) {
        let mut assigned = false;
        for_each_assignment(&pou.body, &mut |target| {
            if let ExprKind::Var(name) = &target.kind
                && name.ident == pou.name.ident
            {
                assigned = true;
            }
        });
        if assigned {
            return;
        }
        self.diags.push(
            Diagnostic::warning(
                codes::W_FUNCTION_RESULT_NOT_ASSIGNED,
                format!("`{}` never assigns its result", pou.name),
            )
            .with_primary(
                pou.name.span,
                "the function's own name is the variable its result comes from",
            )
            .with_note(
                "A warning rather than an error: salman looks only for an assignment to the \
                 name somewhere in the body, not for one on every path. A full \
                 definite-assignment analysis is more than v0.1 needs, and a build that failed \
                 on a branch salman cannot follow would be worse than this warning.",
            )
            .with_clause(clause::FUNCTIONS),
        );
    }

    fn stmts(&mut self, body: &[Stmt]) {
        for stmt in body {
            self.stmt(stmt);
        }
    }

    fn stmt(&mut self, stmt: &Stmt) {
        match &stmt.kind {
            StmtKind::Empty | StmtKind::Error => {}
            StmtKind::Assign { target, value } => {
                let target_ty = self.assign_target(target);
                let value_ty = self.expr(value, Some(target_ty));
                self.check_assignable(value_ty, target_ty, value.span, "this target");
            }
            StmtKind::AssignAttempt { target, value } => {
                self.expr(target, None);
                self.expr(value, None);
                self.references_not_implemented(stmt.span, "the assignment attempt `?=`");
            }
            StmtKind::Call(call) => match &call.kind {
                ExprKind::Call { callee, args } => {
                    let ty = self.call_expr(call, callee, args, true);
                    if let Some(slot) = self.expr_types.get_mut(call.id.index()) {
                        *slot = Some(ty);
                    }
                }
                _ => {
                    self.expr(call, None);
                }
            },
            StmtKind::If {
                branches,
                else_body,
            } => {
                for branch in branches {
                    self.condition(&branch.condition, "IF");
                    self.stmts(&branch.body);
                }
                if let Some(body) = else_body {
                    self.stmts(body);
                }
            }
            StmtKind::Case {
                selector,
                arms,
                else_body,
            } => self.case(selector, arms, else_body.as_deref()),
            StmtKind::For {
                variable,
                from,
                to,
                by,
                body,
            } => self.for_loop(stmt, variable, from, to, by.as_ref(), body),
            StmtKind::While { condition, body } => {
                self.condition(condition, "WHILE");
                self.loop_depth = self.loop_depth.saturating_add(1);
                self.stmts(body);
                self.loop_depth = self.loop_depth.saturating_sub(1);
            }
            StmtKind::Repeat { body, until } => {
                self.loop_depth = self.loop_depth.saturating_add(1);
                self.stmts(body);
                self.loop_depth = self.loop_depth.saturating_sub(1);
                self.condition(until, "REPEAT ... UNTIL");
            }
            // CONTINUE is a statement of Edition 3, Table 72 row 9. One major
            // vendor's documentation still calls it a non-standard extension;
            // that is Edition-2-era wording, and salman does not warn about it.
            StmtKind::Continue => self.jump(stmt.span, "CONTINUE"),
            StmtKind::Exit => self.jump(stmt.span, "EXIT"),
            // RETURN is legal in every POU kind. In a FUNCTION it leaves the
            // result at whatever has been assigned to the function's name so
            // far, which is what `check_function_result` warns about.
            StmtKind::Return => {}
        }
    }

    fn jump(&mut self, span: Span, keyword: &str) {
        if self.loop_depth > 0 {
            return;
        }
        self.diags.push(
            Diagnostic::error(
                codes::E_JUMP_OUTSIDE_LOOP,
                format!("`{keyword}` is outside any loop"),
            )
            .with_primary(
                span,
                format!("`{keyword}` acts on the loop that encloses it"),
            )
            .with_note("FOR, WHILE and REPEAT are the loops. `RETURN` is what leaves a POU.")
            .with_clause(clause::TABLE_ST_STATEMENTS),
        );
    }

    /// An `IF`, `WHILE` or `REPEAT` condition, which must be `BOOL` exactly.
    fn condition(&mut self, condition: &Expr, keyword: &str) {
        let bool_ty = self.types.elementary(ElementaryType::Bool);
        let ty = self.expr(condition, Some(bool_ty));
        if self.types.is_error(ty) || ty == bool_ty {
            return;
        }
        let name = self.types.describe(ty);
        let numeric = self
            .types
            .as_elementary(ty)
            .is_some_and(|t| GenericType::AnyNum.contains(t));
        let mut diag = Diagnostic::error(
            codes::E_CONDITION_NOT_BOOL,
            format!("a `{keyword}` condition must be BOOL, and this is {name}"),
        )
        .with_primary(condition.span, format!("this is {name}"))
        .with_clause(clause::TABLE_ST_STATEMENTS);
        if numeric {
            diag = diag.with_note(
                "There is no \"non-zero is true\" rule in IEC 61131-3, so a number is not a \
                 condition. Write the comparison you mean: `Count > 0`.",
            );
        }
        self.diags.push(diag);
    }

    fn case(&mut self, selector: &Expr, arms: &[crate::ast::CaseArm], else_body: Option<&[Stmt]>) {
        let selector_ty = self.expr(selector, None);
        let usable = match self.types.as_elementary(selector_ty) {
            _ if self.types.is_error(selector_ty) => false,
            Some(elementary) if GenericType::AnyInt.contains(elementary) => true,
            _ => {
                self.diags.push(
                    Diagnostic::error(
                        codes::E_CASE_SELECTOR_TYPE,
                        format!(
                            "a `CASE` selector must be an integer or an enumeration, and this is {}",
                            self.types.describe(selector_ty)
                        ),
                    )
                    .with_primary(selector.span, "this cannot select an arm")
                    .with_clause(clause::TABLE_ST_STATEMENTS),
                );
                false
            }
        };

        let mut covered: Vec<(i128, i128, Span)> = Vec::new();
        for arm in arms {
            for label in &arm.labels {
                if let Some(range) = self.case_label(label, selector_ty, usable) {
                    covered.push(range);
                }
            }
            self.stmts(&arm.body);
        }
        if let Some(body) = else_body {
            self.stmts(body);
        }

        // salman's rule, checked here rather than in the parser because only
        // here are `Colour#Red` and a CONSTANT worth a number. The parser makes
        // a syntactic best effort over plain literals; both can fire on the
        // same pair when the two passes' diagnostics are merged, and
        // deduplicating by code and span belongs in the driver rather than in
        // either pass.
        covered.sort_by_key(|(low, high, span)| (*low, *high, span.start));
        let mut current: Option<(i128, i128, Span)> = None;
        for (low, high, span) in covered {
            match current {
                Some((first_low, first_high, first_span)) if low <= first_high => {
                    let duplicate = low == first_low && high == first_high;
                    self.case_clash(span, first_span, duplicate);
                    current = Some((first_low, first_high.max(high), first_span));
                }
                _ => current = Some((low, high, span)),
            }
        }
    }

    /// One `CASE` label, folded to the range of values it covers.
    fn case_label(
        &mut self,
        label: &CaseLabel,
        selector: TypeId,
        usable: bool,
    ) -> Option<(i128, i128, Span)> {
        match label {
            CaseLabel::Single(expr) => {
                let value = self.case_label_value(expr, selector, usable)?;
                Some((value, value, expr.span))
            }
            CaseLabel::Range { low, high } => {
                let span = low.span.join(high.span);
                let low_value = self.case_label_value(low, selector, usable);
                let high_value = self.case_label_value(high, selector, usable);
                let (Some(low_value), Some(high_value)) = (low_value, high_value) else {
                    return None;
                };
                if high_value < low_value {
                    self.diags.push(
                        Diagnostic::error(
                            codes::E_CASE_LABEL_TYPE,
                            format!("the label range {low_value}..{high_value} is empty"),
                        )
                        .with_primary(span, "no value can select this arm")
                        .with_clause(clause::TABLE_ST_STATEMENTS),
                    );
                    return None;
                }
                Some((low_value, high_value, span))
            }
        }
    }

    fn case_label_value(&mut self, expr: &Expr, selector: TypeId, usable: bool) -> Option<i128> {
        let ty = self.expr(expr, Some(selector));
        if !usable {
            // The selector has already been refused. Whatever the labels are,
            // complaining about them as well would say nothing new.
            return None;
        }
        if !self.types.is_error(ty) && ty != selector {
            let compatible = self
                .types
                .as_elementary(ty)
                .is_some_and(|t| GenericType::AnyInt.contains(t));
            if !compatible {
                self.diags.push(
                    Diagnostic::error(
                        codes::E_CASE_LABEL_TYPE,
                        format!(
                            "a label of {} cannot select on {}",
                            self.types.describe(ty),
                            self.types.describe(selector)
                        ),
                    )
                    .with_primary(expr.span, "this label and the selector are different types")
                    .with_clause(clause::TABLE_ST_STATEMENTS),
                );
                return None;
            }
        }
        let value = self.fold(expr).as_ref().and_then(Value::as_i64);
        if value.is_none() && !self.types.is_error(ty) {
            self.not_constant(expr.span, "a CASE label");
        }
        value.map(i128::from)
    }

    fn case_clash(&mut self, span: Span, earlier: Span, duplicate: bool) {
        let (code, message, label) = if duplicate {
            (
                codes::E_DUPLICATE_CASE_LABEL,
                "this `CASE` label is already covered by an earlier one",
                "duplicate label",
            )
        } else {
            (
                codes::E_OVERLAPPING_CASE_LABELS,
                "this `CASE` label overlaps an earlier one",
                "overlapping label",
            )
        };
        self.diags.push(
            Diagnostic::error(code, message)
                .with_primary(span, label)
                .with_secondary(earlier, "already covered here")
                .with_note(
                    "This is a salman rule, not a rule salman could verify in the standard: no \
                     public source available to salman states whether IEC 61131-3:2013 Table 72 \
                     \"ST language statements\" forbids duplicate or overlapping CASE labels. \
                     salman refuses them because which arm runs would otherwise depend on the \
                     order the arms happen to be written in.",
                )
                .with_clause(clause::ST_STATEMENTS),
        );
    }

    /// A `FOR` loop.
    ///
    /// salman policy, both recorded here because neither could be verified
    /// against the standard from a public source:
    ///
    /// * `TO` and `BY` are evaluated **exactly once, at loop entry**, so a
    ///   body that changes what they were computed from does not change the
    ///   trip count. Every dialect salman looked at does this.
    /// * The body may not assign to the control variable. A loop whose counter
    ///   the body moves has a trip count no reader can work out from the
    ///   source; use `WHILE` when the counter really is yours to move.
    fn for_loop(
        &mut self,
        stmt: &Stmt,
        variable: &Name,
        from: &Expr,
        to: &Expr,
        by: Option<&Expr>,
        body: &[Stmt],
    ) {
        let control = self.control_variable(stmt, variable);
        // `BY` is a **step**, not a value of the control variable, so it is
        // checked against the control variable's base type rather than against
        // its declared one. Checking it against the declared type refused
        // `FOR I := 3 TO 0 BY -1;` over `I : INT (0..3)` because -1 is outside
        // 0..3 — a descending loop over a subrange could not be written at all,
        // although every value it gives the control variable is in range.
        let step_ty = control
            .and_then(|ty| self.types.as_elementary(ty))
            .map(|base| self.types.elementary(base))
            .or(control);
        for (expr, what, expected) in [
            Some((from, "FOR ... :=", control)),
            Some((to, "TO", control)),
            by.map(|b| (b, "BY", step_ty)),
        ]
        .into_iter()
        .flatten()
        {
            let ty = self.expr(expr, expected);
            let integer = self
                .types
                .as_elementary(ty)
                .is_some_and(|t| GenericType::AnyInt.contains(t));
            if !integer && !self.types.is_error(ty) {
                self.diags.push(
                    Diagnostic::error(
                        codes::E_FOR_BOUND_TYPE,
                        format!(
                            "the `{what}` value of a `FOR` loop must be an integer, and this is {}",
                            self.types.describe(ty)
                        ),
                    )
                    .with_primary(expr.span, "a loop counts in integers")
                    .with_clause(clause::TABLE_ST_STATEMENTS),
                );
            }
        }

        if let Some(by) = by
            && let Some(value) = self.fold(by).as_ref().and_then(Value::as_i64)
            && value == 0
        {
            self.diags.push(
                Diagnostic::error(
                    codes::E_FOR_STEP_ZERO,
                    "a `FOR` loop with a step of zero never ends",
                )
                .with_primary(by.span, "this step never moves the control variable")
                .with_clause(clause::TABLE_ST_STATEMENTS),
            );
        }

        self.loop_depth = self.loop_depth.saturating_add(1);
        self.stmts(body);
        self.loop_depth = self.loop_depth.saturating_sub(1);

        let mut assignments: Vec<Span> = Vec::new();
        for_each_assignment(body, &mut |target| {
            if let ExprKind::Var(name) = &target.kind
                && name.ident == variable.ident
            {
                assignments.push(target.span);
            }
        });
        for span in assignments {
            self.diags.push(
                Diagnostic::error(
                    codes::E_FOR_CONTROL_VARIABLE_ASSIGNED,
                    format!("`{variable}` is the control variable of this `FOR` loop"),
                )
                .with_primary(span, "assigning to it here is a salman rule violation")
                .with_secondary(variable.span, "the loop controls this variable")
                .with_note(
                    "This is a salman rule, not a rule salman could verify in the standard. A \
                     loop whose counter the body moves has a trip count no reader can work out \
                     from the source. Use a WHILE loop when the counter really is yours to move.",
                )
                .with_clause(clause::ST_STATEMENTS),
            );
        }
    }

    /// Resolves a `FOR` loop's control variable, which must be a writable
    /// integer variable of the POU the loop is in.
    ///
    /// The resolution is recorded against the **statement's** node id, because
    /// the control variable is a [`Name`] rather than an expression and has no
    /// node of its own.
    fn control_variable(&mut self, stmt: &Stmt, variable: &Name) -> Option<TypeId> {
        let index = self.current?;
        let Some((symbol, found)) = self
            .pous
            .get(index as usize)
            .and_then(|pou| pou.symbol(variable.as_str()))
        else {
            self.diags.push(
                Diagnostic::error(
                    codes::E_FOR_CONTROL_VARIABLE,
                    format!("`{variable}` is not a variable of this POU"),
                )
                .with_primary(
                    variable.span,
                    "a FOR loop counts in a variable the POU declares",
                )
                .with_note(
                    "A global cannot be a control variable: two tasks running the same loop \
                     would then share one counter.",
                )
                .with_clause(clause::TABLE_ST_STATEMENTS),
            );
            return None;
        };
        let ty = found.ty;
        let writable = found.is_writable();
        let declared = found.name.span;
        self.record(stmt.id, Resolution::Local { pou: index, symbol });
        if !writable {
            self.diags.push(
                Diagnostic::error(
                    codes::E_FOR_CONTROL_VARIABLE,
                    format!("`{variable}` cannot be a control variable: the loop must write it"),
                )
                .with_primary(variable.span, "this variable is read-only here")
                .with_secondary(declared, "declared here")
                .with_clause(clause::TABLE_ST_STATEMENTS),
            );
            return None;
        }
        let integer = self
            .types
            .as_elementary(ty)
            .is_some_and(|t| GenericType::AnyInt.contains(t));
        if !integer {
            if !self.types.is_error(ty) {
                self.diags.push(
                    Diagnostic::error(
                        codes::E_FOR_CONTROL_VARIABLE,
                        format!(
                            "a control variable must be an integer, and `{variable}` is {}",
                            self.types.describe(ty)
                        ),
                    )
                    .with_primary(variable.span, "this variable cannot count")
                    .with_secondary(declared, "declared here")
                    .with_clause(clause::TABLE_ST_STATEMENTS),
                );
            }
            return None;
        }
        Some(ty)
    }

    fn not_constant(&mut self, span: Span, what: &str) {
        self.diags.push(
            Diagnostic::error(
                codes::E_NOT_CONSTANT,
                format!("{what} must be a constant salman can work out now"),
            )
            .with_primary(span, "this is not a compile-time constant")
            .with_note(
                "salman folds literals, arithmetic on them, and variables declared CONSTANT with \
                 a constant initialiser. Nothing else has a value before the program runs.",
            )
            .with_clause(clause::TABLE_ELEMENTARY_DATA_TYPES),
        );
    }
}

// ---------------------------------------------------------------------------
// Recursion
// ---------------------------------------------------------------------------

impl Checker<'_> {
    /// Rejects every cycle in the call graph.
    ///
    /// This is what makes salman's single static frame per POU sound: every
    /// POU's storage is allocated once, at load, and a POU that could call
    /// itself would need a stack of them. It is therefore not optional, and it
    /// is checked before any body is, so that a recursive unit still produces
    /// one clear message.
    ///
    /// UNVERIFIED: IEC 61131-3 is widely understood to forbid recursive
    /// invocation of a POU, and every implementation salman looked at refuses
    /// it, but salman could not confirm the clause from a public source. The
    /// diagnostic therefore says that salman rejects it, not that the standard
    /// does.
    fn check_recursion(&mut self) {
        let count = self.pous.len();
        let mut edges: Vec<Vec<u32>> = vec![Vec::new(); count];
        let unit = self.unit;
        let mut index = 0u32;
        for pou in unit.pous() {
            let Some(slot) = self.pous.get(index as usize) else {
                break;
            };
            if slot.name.ident != pou.name.ident {
                continue;
            }
            let mut callees: Vec<u32> = Vec::new();
            for_each_expr(&pou.body, &mut |expr| {
                if let ExprKind::Call { callee, .. } = &expr.kind
                    && let Some(target) = self.static_callee(index, callee)
                    && !callees.contains(&target)
                {
                    callees.push(target);
                }
            });
            if let Some(slot) = edges.get_mut(index as usize) {
                *slot = callees;
            }
            index = index.saturating_add(1);
        }

        // Iterative depth-first search: the stack is explicit so that a unit
        // with thousands of POUs cannot overflow the machine's.
        let mut colour = vec![0u8; count];
        let mut reported: BTreeSet<Vec<u32>> = BTreeSet::new();
        for start in 0..count {
            if colour.get(start).copied().unwrap_or(2) != 0 {
                continue;
            }
            let Ok(start_index) = u32::try_from(start) else {
                continue;
            };
            let mut stack: Vec<(u32, usize)> = vec![(start_index, 0)];
            let mut path: Vec<u32> = vec![start_index];
            if let Some(slot) = colour.get_mut(start) {
                *slot = 1;
            }
            while let Some((node, position)) = stack.last().copied() {
                let next = edges
                    .get(node as usize)
                    .and_then(|list| list.get(position))
                    .copied();
                if let Some((_, position)) = stack.last_mut() {
                    *position = position.saturating_add(1);
                }
                let Some(next) = next else {
                    if let Some(slot) = colour.get_mut(node as usize) {
                        *slot = 2;
                    }
                    stack.pop();
                    path.pop();
                    continue;
                };
                match colour.get(next as usize).copied().unwrap_or(0) {
                    0 => {
                        if let Some(slot) = colour.get_mut(next as usize) {
                            *slot = 1;
                        }
                        stack.push((next, 0));
                        path.push(next);
                    }
                    1 => {
                        let cycle: Vec<u32> = path
                            .iter()
                            .skip_while(|node| **node != next)
                            .copied()
                            .collect();
                        let mut key = cycle.clone();
                        key.sort_unstable();
                        if reported.insert(key) {
                            self.report_cycle(&cycle);
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    /// The POU a callee expression names, worked out from the declarations
    /// alone so that the call graph can be built before any body is checked.
    fn static_callee(&self, owner: u32, callee: &Expr) -> Option<u32> {
        match &callee.kind {
            ExprKind::Var(name) => match self.lookup_in(Some(owner), name.as_str())? {
                Resolution::Function { pou } => Some(pou),
                Resolution::Local { pou, symbol } => {
                    self.instance_pou(self.symbol_at(pou, symbol)?.ty)
                }
                Resolution::Global { symbol } => self.instance_pou(self.global_at(symbol)?.ty),
                _ => None,
            },
            ExprKind::Member { .. } | ExprKind::Index { .. } | ExprKind::Paren(_) => {
                self.instance_pou(self.static_type(owner, callee)?)
            }
            _ => None,
        }
    }

    fn instance_pou(&self, ty: TypeId) -> Option<u32> {
        match self.types.get(ty) {
            TypeData::FunctionBlock { pou, .. } => *pou,
            _ => None,
        }
    }

    /// The type of an expression, worked out without reporting anything.
    fn static_type(&self, owner: u32, expr: &Expr) -> Option<TypeId> {
        match &expr.kind {
            ExprKind::Var(name) => {
                Some(self.type_of_resolution(self.lookup_in(Some(owner), name.as_str())?))
            }
            ExprKind::Paren(inner) => self.static_type(owner, inner),
            ExprKind::Index { base, .. } => match self.types.get(self.static_type(owner, base)?) {
                TypeData::Array { element, .. } => Some(*element),
                _ => None,
            },
            ExprKind::Member { base, field } => {
                let base_ty = self.static_type(owner, base)?;
                let key = field.ident.to_key();
                match self.types.get(base_ty) {
                    TypeData::Struct { fields, .. } => {
                        fields.iter().find(|f| f.name == key).map(|f| f.ty)
                    }
                    TypeData::FunctionBlock { pou: Some(pou), .. } => self
                        .pous
                        .get(*pou as usize)
                        .and_then(|block| block.symbol(field.as_str()))
                        .map(|(_, symbol)| symbol.ty),
                    _ => None,
                }
            }
            _ => None,
        }
    }

    fn report_cycle(&mut self, cycle: &[u32]) {
        let names: Vec<String> = cycle
            .iter()
            .filter_map(|index| self.pous.get(*index as usize))
            .map(|pou| pou.name.to_string())
            .collect();
        let Some(head) = cycle
            .first()
            .and_then(|index| self.pous.get(*index as usize))
        else {
            return;
        };
        let span = head.name.span;
        let mut route = names.clone();
        if let Some(first) = names.first() {
            route.push(first.clone());
        }
        self.diags.push(
            Diagnostic::error(
                codes::E_RECURSIVE_CALL,
                format!("recursive call: {}", route.join(" calls ")),
            )
            .with_primary(span, "this POU can reach itself")
            .with_note(
                "salman rejects recursion. Every POU's storage is laid out once, at load, so \
                 there is no stack of frames for a second entry to use — which is also what makes \
                 salman's memory use knowable before the program runs. IEC 61131-3 is widely \
                 understood to forbid recursive invocation, but salman could not verify the \
                 clause from a public source, so this refusal is stated as salman's.",
            )
            .with_clause(clause::PROGRAM_ORGANIZATION_UNITS),
        );
    }
}

// ---------------------------------------------------------------------------
// Tree walks
// ---------------------------------------------------------------------------

/// Visits every expression in a statement list, outermost first.
fn for_each_expr<'e>(body: &'e [Stmt], visit: &mut impl FnMut(&'e Expr)) {
    for stmt in body {
        match &stmt.kind {
            StmtKind::Empty
            | StmtKind::Error
            | StmtKind::Continue
            | StmtKind::Exit
            | StmtKind::Return => {}
            StmtKind::Assign { target, value } | StmtKind::AssignAttempt { target, value } => {
                walk_expr(target, visit);
                walk_expr(value, visit);
            }
            StmtKind::Call(call) => walk_expr(call, visit),
            StmtKind::If {
                branches,
                else_body,
            } => {
                for branch in branches {
                    walk_expr(&branch.condition, visit);
                    for_each_expr(&branch.body, visit);
                }
                if let Some(body) = else_body {
                    for_each_expr(body, visit);
                }
            }
            StmtKind::Case {
                selector,
                arms,
                else_body,
            } => {
                walk_expr(selector, visit);
                for arm in arms {
                    for label in &arm.labels {
                        match label {
                            CaseLabel::Single(expr) => walk_expr(expr, visit),
                            CaseLabel::Range { low, high } => {
                                walk_expr(low, visit);
                                walk_expr(high, visit);
                            }
                        }
                    }
                    for_each_expr(&arm.body, visit);
                }
                if let Some(body) = else_body {
                    for_each_expr(body, visit);
                }
            }
            StmtKind::For {
                from, to, by, body, ..
            } => {
                walk_expr(from, visit);
                walk_expr(to, visit);
                if let Some(by) = by {
                    walk_expr(by, visit);
                }
                for_each_expr(body, visit);
            }
            StmtKind::While { condition, body } => {
                walk_expr(condition, visit);
                for_each_expr(body, visit);
            }
            StmtKind::Repeat { body, until } => {
                for_each_expr(body, visit);
                walk_expr(until, visit);
            }
        }
    }
}

fn walk_expr<'e>(expr: &'e Expr, visit: &mut impl FnMut(&'e Expr)) {
    visit(expr);
    match &expr.kind {
        ExprKind::Literal(_)
        | ExprKind::Var(_)
        | ExprKind::Direct(_)
        | ExprKind::EnumValue { .. }
        | ExprKind::Error => {}
        ExprKind::Member { base, .. } | ExprKind::Deref(base) | ExprKind::Paren(base) => {
            walk_expr(base, visit);
        }
        ExprKind::Index { base, indices } => {
            walk_expr(base, visit);
            for index in indices {
                walk_expr(index, visit);
            }
        }
        ExprKind::Unary { operand, .. } => walk_expr(operand, visit),
        ExprKind::Binary { lhs, rhs, .. } => {
            walk_expr(lhs, visit);
            walk_expr(rhs, visit);
        }
        ExprKind::Call { callee, args } => {
            walk_expr(callee, visit);
            for arg in args {
                match arg {
                    Arg::Positional(value) | Arg::Input { value, .. } => walk_expr(value, visit),
                    Arg::Output {
                        target: Some(target),
                        ..
                    } => walk_expr(target, visit),
                    Arg::Output { target: None, .. } => {}
                }
            }
        }
    }
}

/// Visits the target of every assignment in a statement list, at any depth.
fn for_each_assignment<'e>(body: &'e [Stmt], visit: &mut impl FnMut(&'e Expr)) {
    for stmt in body {
        match &stmt.kind {
            StmtKind::Assign { target, .. } | StmtKind::AssignAttempt { target, .. } => {
                visit(target);
            }
            StmtKind::If {
                branches,
                else_body,
            } => {
                for branch in branches {
                    for_each_assignment(&branch.body, visit);
                }
                if let Some(body) = else_body {
                    for_each_assignment(body, visit);
                }
            }
            StmtKind::Case {
                arms, else_body, ..
            } => {
                for arm in arms {
                    for_each_assignment(&arm.body, visit);
                }
                if let Some(body) = else_body {
                    for_each_assignment(body, visit);
                }
            }
            StmtKind::For { body, .. }
            | StmtKind::While { body, .. }
            | StmtKind::Repeat { body, .. } => for_each_assignment(body, visit),
            // An output binding `Q => Target` writes Target, and that is an
            // assignment as far as these walks are concerned.
            StmtKind::Call(call) => {
                if let ExprKind::Call { args, .. } = &call.kind {
                    for arg in args {
                        if let Arg::Output {
                            target: Some(target),
                            ..
                        } = arg
                        {
                            visit(target);
                        }
                    }
                }
            }
            StmtKind::Empty
            | StmtKind::Error
            | StmtKind::Continue
            | StmtKind::Exit
            | StmtKind::Return => {}
        }
    }
}

// ---------------------------------------------------------------------------
// Constant folding
// ---------------------------------------------------------------------------

impl Checker<'_> {
    /// The constant value of an expression, if it has one.
    ///
    /// Memoised per node, so folding a chain of a thousand additions stays
    /// linear. Integer arithmetic **wraps**, which is what the runtime does:
    /// if the folder saturated where the runtime wrapped, a constant and the
    /// same computation done at run time would disagree, and the trace would
    /// depend on which side of the fold a value happened to fall.
    fn fold(&mut self, e: &Expr) -> Option<Value> {
        if self.folded.get(e.id.index()).copied().unwrap_or(false) {
            return self
                .const_values
                .get(e.id.index())
                .and_then(Option::as_ref)
                .cloned();
        }
        // Set before recursing, so that a tree the parser somehow made cyclic
        // could not make this loop for ever.
        if let Some(slot) = self.folded.get_mut(e.id.index()) {
            *slot = true;
        }
        let value = self.fold_kind(e);
        if let Some(slot) = self.const_values.get_mut(e.id.index()) {
            slot.clone_from(&value);
        }
        value
    }

    fn fold_kind(&mut self, e: &Expr) -> Option<Value> {
        match &e.kind {
            ExprKind::Literal(literal) => self.literal_value(literal, e.id),
            ExprKind::Paren(inner) => self.fold(inner),
            ExprKind::Var(name) => {
                // An unqualified enumeration value resolved from its context
                // has no symbol to look up; its number is in the resolution.
                if let Some(Resolution::EnumValue { ty, value }) =
                    self.resolutions.get(e.id.index()).copied().flatten()
                {
                    let base = match self.types.get(ty) {
                        TypeData::Enum { base, .. } => *base,
                        _ => return None,
                    };
                    return value_from_int(base, i128::from(value));
                }
                let resolution = self.lookup(name.as_str())?;
                let symbol = match resolution {
                    Resolution::Local { pou, symbol } => self.symbol_at(pou, symbol),
                    Resolution::Global { symbol } => self.global_at(symbol),
                    _ => None,
                }?;
                if symbol.qualifiers.constant {
                    symbol.init.clone()
                } else {
                    None
                }
            }
            ExprKind::EnumValue { ty, value } => {
                let id = self.named_types.get(&ty.ident.to_key()).copied()?;
                let (base, number) = match self.types.get(id) {
                    TypeData::Enum { base, values, .. } => {
                        let key = value.ident.to_key();
                        (
                            *base,
                            values
                                .iter()
                                .find(|(name, _)| *name == key)
                                .map(|(_, number)| *number)?,
                        )
                    }
                    _ => return None,
                };
                value_from_int(base, i128::from(number))
            }
            ExprKind::Unary { op, operand } => {
                let value = self.fold(operand)?;
                fold_unary(*op, &value)
            }
            ExprKind::Binary { op, lhs, rhs } => {
                let left = self.fold(lhs)?;
                let right = self.fold(rhs)?;
                self.fold_binary(*op, &left, &right, e.span)
            }
            _ => None,
        }
    }

    /// The value of a literal, in whatever type the checker gave the node.
    fn literal_value(&self, literal: &LiteralValue, node: NodeId) -> Option<Value> {
        let recorded = self
            .expr_types
            .get(node.index())
            .copied()
            .flatten()
            .and_then(|ty| self.types.as_elementary(ty));
        match literal {
            LiteralValue::Bool(value) => Some(Value::Bool(*value)),
            LiteralValue::Int {
                magnitude,
                negative,
                declared,
            } => {
                let magnitude = i128::try_from(*magnitude).ok()?;
                let value = if *negative { -magnitude } else { magnitude };
                let ty = recorded
                    .or(*declared)
                    .unwrap_or_else(|| default_literal_type(false));
                if GenericType::AnyReal.contains(ty) {
                    return Some(real_value(ty, value as f64));
                }
                wrap_to(ty, value)
            }
            LiteralValue::Real { value, declared } => {
                let ty = recorded
                    .or(*declared)
                    .unwrap_or_else(|| default_literal_type(true));
                Some(real_value(ty, *value))
            }
            LiteralValue::Duration { value, long } => {
                Some(if *long || recorded == Some(ElementaryType::LTime) {
                    Value::LTime(*value)
                } else {
                    Value::Time(*value)
                })
            }
            LiteralValue::Date(value) => Some(Value::Date(*value)),
            LiteralValue::TimeOfDay(value) => Some(Value::TimeOfDay(*value)),
            LiteralValue::DateAndTime(value) => Some(Value::DateAndTime(*value)),
            LiteralValue::String(bytes) => Some(Value::string(bytes)),
            LiteralValue::WString(units) => Some(Value::wstring(units)),
        }
    }

    fn fold_binary(
        &mut self,
        op: BinaryOp,
        left: &Value,
        right: &Value,
        span: Span,
    ) -> Option<Value> {
        // Durations first: they are outside ANY_NUM and have their own rules.
        if let Some(value) = fold_duration(op, left, right) {
            return Some(value);
        }
        if let (Some(left_value), Some(right_value)) = (left.as_f64(), right.as_f64()) {
            return fold_real(op, left.type_of(), right.type_of(), left_value, right_value);
        }
        if let (Value::Bool(left_value), Value::Bool(right_value)) = (left, right) {
            return fold_bool(op, *left_value, *right_value);
        }
        let (Some(left_value), Some(right_value)) = (left.as_i64(), right.as_i64()) else {
            return None;
        };
        // A real operand mixed with an integer one: promote and fold as reals.
        if GenericType::AnyReal.contains(left.type_of())
            || GenericType::AnyReal.contains(right.type_of())
        {
            return fold_real(
                op,
                left.type_of(),
                right.type_of(),
                left_value as f64,
                right_value as f64,
            );
        }
        let common = common_type(left.type_of(), right.type_of(), self.bool_widening())?;
        let left_value = i128::from(left_value);
        let right_value = i128::from(right_value);
        if op.is_comparison() {
            return Some(Value::Bool(compare(op, left_value, right_value)?));
        }
        if matches!(op, BinaryOp::Div | BinaryOp::Mod) && right_value == 0 {
            self.diags.push(
                Diagnostic::error(
                    codes::E_CONSTANT_DIVISION_BY_ZERO,
                    "this constant expression divides by zero",
                )
                .with_primary(span, "the divisor is a constant zero")
                .with_note(
                    "Found before the program runs, which is the only good time to find it: at \
                     run time this is a fault on a machine that is already moving.",
                )
                .with_clause(clause::ST_EXPRESSIONS),
            );
            return None;
        }
        let result = match op {
            BinaryOp::Add => left_value.checked_add(right_value)?,
            BinaryOp::Sub => left_value.checked_sub(right_value)?,
            BinaryOp::Mul => left_value.checked_mul(right_value)?,
            BinaryOp::Div => left_value.checked_div(right_value)?,
            BinaryOp::Mod => left_value.checked_rem(right_value)?,
            BinaryOp::And => left_value & right_value,
            BinaryOp::Or => left_value | right_value,
            BinaryOp::Xor => left_value ^ right_value,
            // Exponentiation is deliberately not folded: its result type
            // depends on rules salman could not verify, and there is no
            // portable integer power in std that salman is willing to use.
            BinaryOp::Pow => return None,
            _ => return None,
        };
        wrap_to(common, result)
    }

    /// Converts a folded value to the type the checker gave its node.
    fn coerce(&self, value: &Value, ty: TypeId) -> Option<Value> {
        let target = self.types.as_elementary(ty)?;
        if value.type_of() == target {
            return Some(value.clone());
        }
        if GenericType::AnyReal.contains(target) {
            let number = value
                .as_f64()
                .or_else(|| value.as_i64().map(|v| v as f64))?;
            return Some(real_value(target, number));
        }
        if integer_range(target).is_some() {
            let number = match value {
                Value::Bool(flag) => i128::from(*flag),
                other => i128::from(other.as_i64()?),
            };
            if !integer_fits(number, target) {
                return None;
            }
            return value_from_int(target, number);
        }
        if GenericType::AnyDuration.contains(target) {
            let duration = value.as_duration()?;
            return Some(if target == ElementaryType::LTime {
                Value::LTime(duration)
            } else {
                Value::Time(duration)
            });
        }
        None
    }

    /// Folds an expression that has to be a constant integer.
    fn const_int(&mut self, e: &Expr, what: &str) -> Option<i128> {
        let Some(value) = self.fold(e) else {
            self.not_constant(e.span, what);
            return None;
        };
        if let Some(value) = value.as_i64() {
            Some(i128::from(value))
        } else {
            self.diags.push(
                Diagnostic::error(
                    codes::E_NOT_CONSTANT,
                    format!("{what} must be an integer, and this is {}", value.type_of()),
                )
                .with_primary(e.span, "not an integer constant")
                .with_clause(clause::TABLE_ELEMENTARY_DATA_TYPES),
            );
            None
        }
    }
}

fn fold_unary(op: UnaryOp, value: &Value) -> Option<Value> {
    match op {
        UnaryOp::Plus => Some(value.clone()),
        UnaryOp::Neg => {
            if let Some(duration) = value.as_duration() {
                let negated =
                    salman_core::time::Duration::from_nanos(duration.nanos().checked_neg()?);
                return Some(match value {
                    Value::LTime(_) => Value::LTime(negated),
                    _ => Value::Time(negated),
                });
            }
            if let Some(number) = value.as_f64() {
                return Some(real_value(value.type_of(), -number));
            }
            let number = i128::from(value.as_i64()?);
            match check_unary(UnaryOp::Neg, value.type_of()) {
                OpResult::Ok(ty) => wrap_to(ty, -number),
                _ => None,
            }
        }
        UnaryOp::Not => match value {
            Value::Bool(flag) => Some(Value::Bool(!flag)),
            other => {
                let number = i128::from(other.as_i64()?);
                wrap_to(other.type_of(), !number)
            }
        },
    }
}

fn fold_bool(op: BinaryOp, left: bool, right: bool) -> Option<Value> {
    Some(Value::Bool(match op {
        BinaryOp::And => left && right,
        BinaryOp::Or => left || right,
        BinaryOp::Xor | BinaryOp::Ne => left != right,
        BinaryOp::Eq => left == right,
        _ => return None,
    }))
}

#[allow(
    clippy::float_cmp,
    reason = "ST's `=` and `<>` on REAL are exact IEEE comparisons; folding them with an epsilon \
              would silently give the constant a different answer from the runtime"
)]
fn fold_real(
    op: BinaryOp,
    left_ty: ElementaryType,
    right_ty: ElementaryType,
    left: f64,
    right: f64,
) -> Option<Value> {
    if op.is_comparison() {
        return Some(Value::Bool(match op {
            BinaryOp::Lt => left < right,
            BinaryOp::Gt => left > right,
            BinaryOp::Le => left <= right,
            BinaryOp::Ge => left >= right,
            BinaryOp::Eq => left == right,
            _ => left != right,
        }));
    }
    // The four arithmetic operations are exactly specified by IEEE 754 and are
    // therefore portable. Exponentiation is not, and is not folded.
    let value = match op {
        BinaryOp::Add => left + right,
        BinaryOp::Sub => left - right,
        BinaryOp::Mul => left * right,
        BinaryOp::Div => left / right,
        _ => return None,
    };
    let wide = left_ty == ElementaryType::Lreal || right_ty == ElementaryType::Lreal;
    Some(real_value(
        if wide {
            ElementaryType::Lreal
        } else {
            ElementaryType::Real
        },
        value,
    ))
}

fn fold_duration(op: BinaryOp, left: &Value, right: &Value) -> Option<Value> {
    let long = matches!(left, Value::LTime(_)) || matches!(right, Value::LTime(_));
    let build = |nanos: i64| {
        if long {
            Value::LTime(salman_core::time::Duration::from_nanos(nanos))
        } else {
            Value::Time(salman_core::time::Duration::from_nanos(nanos))
        }
    };
    match (left.as_duration(), right.as_duration()) {
        (Some(left_value), Some(right_value)) => {
            let nanos = match op {
                BinaryOp::Add => left_value.nanos().checked_add(right_value.nanos())?,
                BinaryOp::Sub => left_value.nanos().checked_sub(right_value.nanos())?,
                _ => {
                    return op
                        .is_comparison()
                        .then(|| {
                            compare(
                                op,
                                i128::from(left_value.nanos()),
                                i128::from(right_value.nanos()),
                            )
                            .map(Value::Bool)
                        })
                        .flatten();
                }
            };
            Some(build(nanos))
        }
        (Some(left_value), None) => {
            let factor = right.as_i64()?;
            let nanos = match op {
                BinaryOp::Mul => left_value.nanos().checked_mul(factor)?,
                BinaryOp::Div => left_value.nanos().checked_div(factor)?,
                _ => return None,
            };
            Some(build(nanos))
        }
        (None, Some(right_value)) => {
            let factor = left.as_i64()?;
            match op {
                BinaryOp::Mul => Some(build(right_value.nanos().checked_mul(factor)?)),
                _ => None,
            }
        }
        (None, None) => None,
    }
}

fn compare(op: BinaryOp, left: i128, right: i128) -> Option<bool> {
    Some(match op {
        BinaryOp::Lt => left < right,
        BinaryOp::Gt => left > right,
        BinaryOp::Le => left <= right,
        BinaryOp::Ge => left >= right,
        BinaryOp::Eq => left == right,
        BinaryOp::Ne => left != right,
        _ => return None,
    })
}

/// Wraps an integer into a type the way the runtime does.
fn wrap_to(ty: ElementaryType, value: i128) -> Option<Value> {
    let (low, high) = integer_range(ty)?;
    let width = high.checked_sub(low)?.checked_add(1)?;
    let wrapped = (value - low).rem_euclid(width) + low;
    value_from_int(ty, wrapped)
}

/// Builds a value of an integer type from a number that fits it.
fn value_from_int(ty: ElementaryType, value: i128) -> Option<Value> {
    Some(match ty {
        ElementaryType::Bool => Value::Bool(value != 0),
        ElementaryType::Sint => Value::Sint(i8::try_from(value).ok()?),
        ElementaryType::Int => Value::Int(i16::try_from(value).ok()?),
        ElementaryType::Dint => Value::Dint(i32::try_from(value).ok()?),
        ElementaryType::Lint => Value::Lint(i64::try_from(value).ok()?),
        ElementaryType::Usint => Value::Usint(u8::try_from(value).ok()?),
        ElementaryType::Uint => Value::Uint(u16::try_from(value).ok()?),
        ElementaryType::Udint => Value::Udint(u32::try_from(value).ok()?),
        ElementaryType::Ulint => Value::Ulint(u64::try_from(value).ok()?),
        ElementaryType::Byte => Value::Byte(u8::try_from(value).ok()?),
        ElementaryType::Word => Value::Word(u16::try_from(value).ok()?),
        ElementaryType::Dword => Value::Dword(u32::try_from(value).ok()?),
        ElementaryType::Lword => Value::Lword(u64::try_from(value).ok()?),
        _ => return None,
    })
}

fn real_value(ty: ElementaryType, value: f64) -> Value {
    if ty == ElementaryType::Real {
        Value::real(value as f32)
    } else {
        Value::lreal(value)
    }
}

// ---------------------------------------------------------------------------
// Configurations
// ---------------------------------------------------------------------------

impl Checker<'_> {
    fn check_configurations(&mut self) {
        let unit = self.unit;
        for config in unit.configurations() {
            let checked = self.check_configuration(config);
            self.configurations.push(checked);
        }
    }

    fn check_configuration(&mut self, config: &ConfigurationDecl) -> CheckedConfiguration {
        let mut tasks: Vec<CheckedTask> = Vec::new();
        let mut untasked: Vec<CheckedProgramInstance> = Vec::new();
        let mut instances: Vec<(&ProgramInstance, Option<usize>)> = Vec::new();

        for resource in &config.resources {
            for task in &resource.tasks {
                if tasks.iter().any(|t| t.name == task.name.ident.to_key()) {
                    self.duplicate(&task.name, task.span, "a task");
                    continue;
                }
                let checked = self.check_task(task);
                tasks.push(checked);
            }
            for instance in &resource.programs {
                let bound = match &instance.task {
                    Some(name) => {
                        let key = name.ident.to_key();
                        let found = tasks.iter().position(|t| t.name == key);
                        if found.is_none() {
                            self.diags.push(
                                Diagnostic::error(
                                    codes::E_BAD_CONFIGURATION,
                                    format!("no task named `{name}` is declared here"),
                                )
                                .with_primary(
                                    name.span,
                                    "a program runs under a task the resource declares",
                                )
                                .with_clause(clause::TASKS),
                            );
                            continue;
                        }
                        found
                    }
                    None => None,
                };
                instances.push((instance, bound));
            }
        }

        for (instance, bound) in instances {
            let Some(checked) = self.check_program_instance(instance) else {
                continue;
            };
            match bound.and_then(|index| tasks.get_mut(index)) {
                Some(task) => task.programs.push(checked),
                // salman policy: a program with no WITH clause runs
                // freewheeling. IEC 61131-3:2013 §6.8.2 "Tasks" says a program
                // with no task association runs continuously at the lowest
                // priority; `untasked` is where the caller finds them.
                None => untasked.push(checked),
            }
        }

        CheckedConfiguration {
            name: config.name.ident.to_key(),
            tasks,
            untasked,
        }
    }

    fn check_task(&mut self, task: &TaskDecl) -> CheckedTask {
        let interval = task.interval.as_ref().and_then(|expr| {
            self.expr(expr, None);
            match self.fold(expr).as_ref().and_then(Value::as_duration) {
                Some(duration) if duration.nanos() > 0 => Some(duration.nanos()),
                found => {
                    self.diags.push(
                        Diagnostic::error(
                            codes::E_BAD_CONFIGURATION,
                            "an `INTERVAL` must be a constant duration greater than zero",
                        )
                        .with_primary(
                            expr.span,
                            match found {
                                Some(_) => "a cycle time of zero or less releases nothing",
                                None => "this is not a duration salman can work out now",
                            },
                        )
                        .with_clause(clause::TASKS),
                    );
                    None
                }
            }
        });

        let single = task
            .single
            .as_ref()
            .and_then(|expr| self.event_global(expr));

        if interval.is_some() && single.is_some() {
            self.diags.push(
                Diagnostic::error(
                    codes::E_BAD_CONFIGURATION,
                    format!("task `{}` is both cyclic and event-triggered", task.name),
                )
                .with_primary(task.span, "give it an INTERVAL or a SINGLE, not both")
                .with_note("salman uses the INTERVAL and ignores the SINGLE.")
                .with_clause(clause::TASKS),
            );
        }

        let priority = match task.priority.as_ref() {
            Some(expr) => {
                let dint = self.types.elementary(ElementaryType::Dint);
                self.expr(expr, Some(dint));
                let value = self.const_int(expr, "a task PRIORITY").unwrap_or(0);
                if let Ok(priority) = u16::try_from(value) {
                    priority
                } else {
                    self.diags.push(
                        Diagnostic::error(
                            codes::E_BAD_CONFIGURATION,
                            format!("a task priority of {value} is outside 0..65535"),
                        )
                        .with_primary(expr.span, "salman holds a priority in 16 bits")
                        .with_note("Lower is more urgent, as the standard has it.")
                        .with_clause(clause::TASKS),
                    );
                    0
                }
            }
            // salman policy: a task that states no priority gets 0. The
            // standard leaves the default to the implementation.
            None => 0,
        };

        let trigger = match (interval, single) {
            (Some(interval_ns), _) => CheckedTrigger::Cyclic { interval_ns },
            (None, Some(global)) => CheckedTrigger::Event { global },
            (None, None) => CheckedTrigger::Freewheeling,
        };

        CheckedTask {
            name: task.name.ident.to_key(),
            trigger,
            priority,
            programs: Vec::new(),
        }
    }

    /// Resolves a `SINGLE :=` expression, which has to name a global `BOOL`.
    fn event_global(&mut self, expr: &Expr) -> Option<u32> {
        let bool_ty = self.types.elementary(ElementaryType::Bool);
        let ty = self.expr(expr, Some(bool_ty));
        let global = match self.resolutions.get(expr.id.index()).copied().flatten() {
            Some(Resolution::Global { symbol }) => Some(symbol),
            _ => None,
        };
        match global {
            Some(symbol) if ty == bool_ty => Some(symbol),
            _ => {
                self.diags.push(
                    Diagnostic::error(
                        codes::E_BAD_CONFIGURATION,
                        "a `SINGLE` trigger must name a global BOOL",
                    )
                    .with_primary(
                        expr.span,
                        if global.is_some() {
                            "this global is not a BOOL"
                        } else {
                            "this is not a global variable"
                        },
                    )
                    .with_note(
                        "The task is released when the variable goes from FALSE to TRUE, so it \
                         has to be a variable something else can write.",
                    )
                    .with_clause(clause::TASKS),
                );
                None
            }
        }
    }

    fn check_program_instance(
        &mut self,
        instance: &ProgramInstance,
    ) -> Option<CheckedProgramInstance> {
        let found = self
            .pous
            .iter()
            .enumerate()
            .find(|(_, pou)| pou.name.ident == instance.program_type.ident)
            .and_then(|(index, pou)| {
                u32::try_from(index)
                    .ok()
                    .map(|index| (index, pou.kind, pou.span))
            });
        match found {
            Some((index, PouKind::Program, _)) => Some(CheckedProgramInstance {
                name: instance.name.ident.to_key(),
                pou: index,
            }),
            Some((_, kind, span)) => {
                self.diags.push(
                    Diagnostic::error(
                        codes::E_BAD_CONFIGURATION,
                        format!(
                            "`{}` is a {}, and only a PROGRAM can be instantiated here",
                            instance.program_type,
                            kind.keyword()
                        ),
                    )
                    .with_primary(instance.program_type.span, "not a PROGRAM")
                    .with_secondary(span, "declared here")
                    .with_clause(clause::PROGRAMS),
                );
                None
            }
            None => {
                let candidates: Vec<String> = self
                    .pous
                    .iter()
                    .filter(|pou| pou.kind == PouKind::Program)
                    .map(|pou| pou.name.to_string())
                    .collect();
                let mut diag = Diagnostic::error(
                    codes::E_BAD_CONFIGURATION,
                    format!("no PROGRAM named `{}` is declared", instance.program_type),
                )
                .with_primary(instance.program_type.span, "nothing to instantiate")
                .with_clause(clause::PROGRAMS);
                if let Some(suggestion) = closest(instance.program_type.as_str(), &candidates) {
                    diag = diag.with_suggestion(
                        format!("there is a PROGRAM named `{suggestion}`"),
                        vec![Edit {
                            span: instance.program_type.span,
                            replacement: suggestion,
                        }],
                    );
                }
                self.diags.push(diag);
                None
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Small helpers
// ---------------------------------------------------------------------------

/// `"s"` when a count needs a plural, `""` when it does not.
fn plural(count: usize) -> &'static str {
    if count == 1 { "" } else { "s" }
}

/// Renders a list of names the way a sentence needs it.
fn join_names(names: &[String]) -> String {
    match names {
        [] => "none".to_string(),
        [only] => format!("`{only}`"),
        [rest @ .., last] => format!(
            "{} and `{last}`",
            rest.iter()
                .map(|name| format!("`{name}`"))
                .collect::<Vec<String>>()
                .join(", ")
        ),
    }
}

/// The candidate closest to `target`, when one is close enough to be worth
/// offering.
///
/// A suggestion for a typo is worth more than a paragraph of prose, and a
/// suggestion that is wrong costs a reader one glance. The bound is
/// deliberately tight: at most a third of the name, and never more than three
/// edits.
fn closest(target: &str, candidates: &[String]) -> Option<String> {
    let limit = (target.len() / 3).clamp(1, 3);
    let mut best: Option<(usize, &String)> = None;
    for candidate in candidates {
        if candidate.eq_ignore_ascii_case(target) {
            continue;
        }
        let distance = edit_distance(target, candidate);
        if distance > limit {
            continue;
        }
        match best {
            Some((best_distance, best_candidate))
                if best_distance < distance
                    || (best_distance == distance && best_candidate <= candidate) => {}
            _ => best = Some((distance, candidate)),
        }
    }
    best.map(|(_, candidate)| candidate.clone())
}

/// Levenshtein distance, case-insensitively, because IEC identifiers are.
fn edit_distance(left: &str, right: &str) -> usize {
    let left: Vec<u8> = left.bytes().map(|b| b.to_ascii_lowercase()).collect();
    let right: Vec<u8> = right.bytes().map(|b| b.to_ascii_lowercase()).collect();
    let mut previous: Vec<usize> = (0..=right.len()).collect();
    let mut current: Vec<usize> = vec![0; right.len() + 1];
    for (i, left_byte) in left.iter().enumerate() {
        if let Some(slot) = current.first_mut() {
            *slot = i + 1;
        }
        for (j, right_byte) in right.iter().enumerate() {
            let cost = usize::from(left_byte != right_byte);
            let deletion = previous.get(j + 1).copied().unwrap_or(usize::MAX);
            let insertion = current.get(j).copied().unwrap_or(usize::MAX);
            let substitution = previous.get(j).copied().unwrap_or(usize::MAX);
            if let Some(slot) = current.get_mut(j + 1) {
                *slot = deletion
                    .saturating_add(1)
                    .min(insertion.saturating_add(1))
                    .min(substitution.saturating_add(cost));
            }
        }
        std::mem::swap(&mut previous, &mut current);
    }
    previous.last().copied().unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    use salman_core::diag::Severity;
    use salman_core::span::SourceMap;

    use crate::parser::parse_source;

    /// Codes the parser raises that the checker raises again, knowing more.
    /// A test source is allowed to carry these out of the parser without the
    /// helpers below calling it a broken source.
    const SHARED_WITH_PARSER: &[&str] = &["E0208", "E0209", "E0210"];

    fn check_with(dialect: &Dialect, src: &str) -> (Checked, Diagnostics, SourceMap) {
        let mut map = SourceMap::new();
        let file = map.add("t.st", src).expect("a source file");
        let (unit, parse_diags) = parse_source(file, src, dialect);
        let unexpected: Vec<&str> = parse_diags
            .items()
            .iter()
            .filter(|d| d.severity == Severity::Error && !SHARED_WITH_PARSER.contains(&d.code.0))
            .map(|d| d.code.0)
            .collect();
        assert!(
            unexpected.is_empty(),
            "the test source does not parse: {unexpected:?}\n{}",
            parse_diags.render(&map)
        );
        let (checked, diags) = check(&unit, dialect);
        (checked, diags, map)
    }

    fn check_text(src: &str) -> (Checked, Diagnostics, SourceMap) {
        check_with(&Dialect::generic(), src)
    }

    fn codes_of(diags: &Diagnostics, severity: Severity) -> Vec<&'static str> {
        diags
            .items()
            .iter()
            .filter(|d| d.severity == severity)
            .map(|d| d.code.0)
            .collect()
    }

    fn errors(src: &str) -> Vec<&'static str> {
        let (_, diags, _) = check_text(src);
        codes_of(&diags, Severity::Error)
    }

    fn warnings(src: &str) -> Vec<&'static str> {
        let (_, diags, _) = check_text(src);
        codes_of(&diags, Severity::Warning)
    }

    /// Checks a source that must be clean, and hands back what was learned.
    fn checked_ok(src: &str) -> Checked {
        let (checked, diags, map) = check_text(src);
        assert!(
            !diags.has_errors(),
            "unexpected errors:\n{src}\n{}",
            diags.render(&map)
        );
        checked
    }

    /// A `PROGRAM Main` with `body` for its statements.
    fn in_program(body: &str) -> String {
        format!("PROGRAM Main\n{body}\nEND_PROGRAM\n")
    }

    /// A `PROGRAM Main` with one `VAR` block and a body.
    fn program(vars: &str, body: &str) -> String {
        format!("PROGRAM Main\nVAR\n{vars}\nEND_VAR\n{body}\nEND_PROGRAM\n")
    }

    fn program_errors(vars: &str, body: &str) -> Vec<&'static str> {
        errors(&program(vars, body))
    }

    /// The message of the first diagnostic carrying `code`.
    fn message(diags: &Diagnostics, code: &str) -> String {
        let diag = diags
            .items()
            .iter()
            .find(|d| d.code.0 == code)
            .unwrap_or_else(|| {
                panic!(
                    "no {code} was reported: {:?}",
                    codes_of(diags, Severity::Error)
                )
            });
        let mut out = diag.message.clone();
        for note in &diag.notes {
            out.push('\n');
            out.push_str(note);
        }
        for suggestion in &diag.suggestions {
            out.push('\n');
            out.push_str(&suggestion.message);
        }
        for label in &diag.labels {
            out.push('\n');
            out.push_str(&label.message);
        }
        out
    }

    fn parse_only(src: &str) -> CompilationUnit {
        let mut map = SourceMap::new();
        let file = map.add("t.st", src).expect("a source file");
        let (unit, _) = parse_source(file, src, &Dialect::generic());
        unit
    }

    /// The statements of `PROGRAM Main`.
    fn main_body(unit: &CompilationUnit) -> Vec<Stmt> {
        unit.pou("Main").expect("PROGRAM Main").body.clone()
    }

    /// The value expression of the first assignment in `PROGRAM Main`.
    fn first_value(unit: &CompilationUnit) -> Expr {
        for stmt in main_body(unit) {
            if let StmtKind::Assign { value, .. } = stmt.kind {
                return value;
            }
        }
        panic!("no assignment in Main");
    }

    fn symbol_of(checked: &Checked, pou: &str, name: &str) -> Symbol {
        let (_, found) = checked.pou(pou).expect("the POU");
        found.symbol(name).expect("the symbol").1.clone()
    }

    // -- pass one: declarations ---------------------------------------------

    #[test]
    fn a_pou_may_call_one_declared_further_down_the_file() {
        // Declaration order is not use order in this language: engineers write
        // the program first and its helpers after it.
        let checked = checked_ok(
            "PROGRAM Main VAR x : INT; END_VAR x := Double(2); END_PROGRAM\n\
             FUNCTION Double : INT VAR_INPUT n : INT; END_VAR Double := n * 2; END_FUNCTION\n",
        );
        assert!(checked.pou("Double").is_some());
    }

    #[test]
    fn a_type_may_be_declared_after_the_type_that_contains_it() {
        let checked = checked_ok(
            "TYPE Outer : STRUCT inner : Inner; END_STRUCT; END_TYPE\n\
             TYPE Inner : STRUCT count : INT; END_STRUCT; END_TYPE\n\
             PROGRAM Main VAR o : Outer; n : INT; END_VAR n := o.inner.count; END_PROGRAM\n",
        );
        assert!(!checked.types.is_empty());
    }

    #[test]
    fn a_type_that_contains_itself_is_refused() {
        assert_eq!(
            errors(
                "TYPE Loop : STRUCT self : Loop; END_STRUCT; END_TYPE\n\
                 PROGRAM Main END_PROGRAM\n"
            ),
            ["E0308"]
        );
    }

    #[test]
    fn two_types_that_contain_each_other_are_refused() {
        let found = errors(
            "TYPE A : STRUCT b : B; END_STRUCT; END_TYPE\n\
             TYPE B : STRUCT a : A; END_STRUCT; END_TYPE\n\
             PROGRAM Main END_PROGRAM\n",
        );
        assert!(found.iter().all(|code| *code == "E0308"), "{found:?}");
        assert_eq!(found.len(), 2);
    }

    #[test]
    fn an_unknown_type_name_is_refused_and_the_nearest_declared_one_is_offered() {
        let src = "TYPE Speed : INT; END_TYPE\n\
                   PROGRAM Main VAR v : Sped; END_VAR END_PROGRAM\n";
        let (_, diags, _) = check_text(src);
        assert_eq!(codes_of(&diags, Severity::Error), ["E0301"]);
        assert!(
            message(&diags, "E0301").contains("speed"),
            "{}",
            message(&diags, "E0301")
        );
    }

    #[test]
    fn a_declared_type_resolves_without_complaint() {
        let checked = checked_ok(
            "TYPE Speed : INT; END_TYPE\n\
             PROGRAM Main VAR v : Speed; END_VAR v := 3; END_PROGRAM\n",
        );
        let symbol = symbol_of(&checked, "Main", "v");
        assert_eq!(
            checked.types.as_elementary(symbol.ty),
            Some(ElementaryType::Int)
        );
    }

    #[test]
    fn two_pous_of_one_name_are_refused_and_the_first_one_wins() {
        let src = "PROGRAM Main END_PROGRAM\nPROGRAM MAIN END_PROGRAM\n";
        let (checked, diags, _) = check_text(src);
        assert_eq!(codes_of(&diags, Severity::Error), ["E0303"]);
        assert_eq!(checked.pous.len(), 1);
    }

    #[test]
    fn two_variables_of_one_name_in_one_pou_are_refused() {
        assert_eq!(
            program_errors("a : INT;\na : DINT;", ""),
            ["E0303"],
            "IEC identifiers compare without case, so this is one name declared twice"
        );
    }

    #[test]
    fn two_globals_of_one_name_are_refused() {
        assert_eq!(
            errors(
                "VAR_GLOBAL g : INT; END_VAR\nVAR_GLOBAL G : BOOL; END_VAR\nPROGRAM Main END_PROGRAM\n"
            ),
            ["E0303"]
        );
    }

    #[test]
    fn several_names_cannot_share_one_direct_address() {
        assert_eq!(program_errors("a, b AT %IX0.0 : BOOL;", ""), ["E0303"]);
    }

    // -- types ---------------------------------------------------------------

    #[test]
    fn a_string_with_no_declared_length_takes_the_dialect_default() {
        let checked = checked_ok(&program("s : STRING;", ""));
        let symbol = symbol_of(&checked, "Main", "s");
        assert_eq!(
            *checked.types.get(symbol.ty),
            TypeData::Str {
                wide: false,
                max_len: u32::from(Dialect::generic().default_string_length)
            }
        );
    }

    #[test]
    fn a_string_length_that_is_not_a_usable_constant_is_refused() {
        assert_eq!(program_errors("s : STRING[0];", ""), ["E0306"]);
        assert_eq!(program_errors("n : INT;\ns : STRING[n];", ""), ["E0307"]);
    }

    #[test]
    fn a_declared_string_length_is_kept() {
        let checked = checked_ok(&program("s : STRING[12];", ""));
        let symbol = symbol_of(&checked, "Main", "s");
        assert_eq!(
            *checked.types.get(symbol.ty),
            TypeData::Str {
                wide: false,
                max_len: 12
            }
        );
    }

    #[test]
    fn an_inverted_array_dimension_names_both_bounds() {
        let src = program("a : ARRAY [5..1] OF INT;", "");
        let (_, diags, _) = check_text(&src);
        assert_eq!(codes_of(&diags, Severity::Error), ["E0304"]);
        assert!(message(&diags, "E0304").contains("5..1"));
    }

    #[test]
    fn an_array_bound_that_is_not_constant_is_refused() {
        assert_eq!(
            program_errors("n : INT;\na : ARRAY [1..n] OF INT;", ""),
            ["E0307"]
        );
    }

    #[test]
    fn an_array_bound_may_be_a_constant_variable() {
        let checked = checked_ok(&format!(
            "PROGRAM Main\nVAR CONSTANT Size : INT := 4; END_VAR\nVAR a : ARRAY [1..Size] OF INT; END_VAR\n{}\nEND_PROGRAM\n",
            "a[4] := 1;"
        ));
        let symbol = symbol_of(&checked, "Main", "a");
        assert_eq!(
            *checked.types.get(symbol.ty),
            TypeData::Array {
                element: checked.types.elementary(ElementaryType::Int),
                dims: vec![ArrayBounds { low: 1, high: 4 }]
            }
        );
    }

    #[test]
    fn a_declaration_can_name_a_constant_the_line_above_it_declared() {
        let checked = checked_ok(
            "PROGRAM Main\n\
             VAR CONSTANT Size : INT := 4; Limit : INT := Size - 1; END_VAR\n\
             VAR a : ARRAY [1..Size] OF INT; END_VAR\n\
             END_PROGRAM\n",
        );
        assert_eq!(
            symbol_of(&checked, "Main", "Limit").init,
            Some(Value::Int(3))
        );
    }

    #[test]
    fn a_global_can_name_a_constant_global_declared_before_it() {
        let checked = checked_ok(
            "VAR_GLOBAL CONSTANT Size : INT := 4; END_VAR\n\
             VAR_GLOBAL Limit : INT := Size; END_VAR\n\
             PROGRAM Main END_PROGRAM\n",
        );
        let limit = checked
            .globals
            .iter()
            .find(|s| s.name.ident.eq_str("Limit"))
            .expect("the global");
        assert_eq!(limit.init, Some(Value::Int(4)));
    }

    #[test]
    fn an_inverted_subrange_is_refused() {
        assert_eq!(program_errors("v : INT (100..0);", ""), ["E0305"]);
    }

    #[test]
    fn a_subrange_of_a_type_that_is_not_an_integer_is_refused() {
        assert_eq!(program_errors("v : REAL (0..10);", ""), ["E0305"]);
    }

    #[test]
    fn a_subrange_bound_outside_its_base_type_is_refused() {
        assert_eq!(program_errors("v : SINT (0..300);", ""), ["E0305"]);
    }

    /// The values of the enumeration a variable of `name` is declared with.
    fn enum_values(checked: &Checked, variable: &str) -> Vec<(String, i64)> {
        let symbol = symbol_of(checked, "Main", variable);
        match checked.types.get(symbol.ty) {
            TypeData::Enum { values, .. } => values
                .iter()
                .map(|(name, value)| (name.to_string(), *value))
                .collect(),
            other => panic!("not an enumeration: {other:?}"),
        }
    }

    #[test]
    fn enumeration_values_continue_from_the_previous_one_starting_at_zero() {
        // The continuation rule is what every implementation salman looked at
        // does; salman could not verify which clause of the standard states it.
        let checked = checked_ok(
            "TYPE Colour : (Red, Green, Blue); END_TYPE\n\
             PROGRAM Main VAR c : Colour; END_VAR END_PROGRAM\n",
        );
        let values: Vec<i64> = enum_values(&checked, "c")
            .into_iter()
            .map(|(_, value)| value)
            .collect();
        assert_eq!(values, [0, 1, 2]);
    }

    #[test]
    fn an_explicit_enumeration_value_restarts_the_continuation() {
        let checked = checked_ok(
            "TYPE Mode : (Off, Manual := 10, Auto); END_TYPE\n\
             PROGRAM Main VAR m : Mode; END_VAR END_PROGRAM\n",
        );
        let values: Vec<i64> = enum_values(&checked, "m")
            .into_iter()
            .map(|(_, value)| value)
            .collect();
        assert_eq!(values, [0, 10, 11]);
    }

    #[test]
    fn a_function_block_instance_type_knows_which_pou_declared_it() {
        let checked = checked_ok(
            "FUNCTION_BLOCK Motor VAR_INPUT Run : BOOL; END_VAR END_FUNCTION_BLOCK\n\
             PROGRAM Main VAR m : Motor; END_VAR m(Run := TRUE); END_PROGRAM\n",
        );
        let symbol = symbol_of(&checked, "Main", "m");
        let (index, _) = checked.pou("Motor").expect("the function block");
        assert_eq!(
            *checked.types.get(symbol.ty),
            TypeData::FunctionBlock {
                name: checked
                    .pous
                    .get(index as usize)
                    .expect("the POU")
                    .name
                    .ident
                    .to_key(),
                native: None,
                pou: Some(index)
            }
        );
    }

    #[test]
    fn a_standard_block_name_resolves_to_a_native_instance_type() {
        let checked = checked_ok(&program("t : TON;", "t(IN := TRUE, PT := T#1s);"));
        let symbol = symbol_of(&checked, "Main", "t");
        assert_eq!(checked.native_block(symbol.ty), Some(NativeBlock::Ton));
    }

    // -- the untyped literal rule, which is salman policy ---------------------

    #[test]
    fn an_untyped_integer_literal_takes_the_type_its_context_requires() {
        // salman policy: no standard default could be verified, so a literal
        // takes the type its context needs. `x : SINT := 5;` is what every
        // vendor accepts, and this is what makes it legal here.
        let checked = checked_ok(&program("x : SINT := 5;", ""));
        assert_eq!(symbol_of(&checked, "Main", "x").init, Some(Value::Sint(5)));
    }

    #[test]
    fn an_untyped_integer_literal_that_does_not_fit_its_context_names_the_value_and_the_range() {
        let src = program("x : SINT := 300;", "");
        let (_, diags, _) = check_text(&src);
        assert_eq!(codes_of(&diags, Severity::Error), ["E0404"]);
        let text = message(&diags, "E0404");
        assert!(text.contains("300"), "{text}");
        assert!(text.contains("-128..127"), "{text}");
    }

    #[test]
    fn an_untyped_integer_literal_falls_back_to_dint_when_nothing_asks_for_a_type() {
        let src = in_program("VAR x : DINT; END_VAR x := 5;");
        let unit = parse_only(&src);
        let (checked, _, _) = check_text(&src);
        let value = first_value(&unit);
        assert_eq!(
            checked.type_of(value.id),
            Some(checked.types.elementary(ElementaryType::Dint))
        );
        assert_eq!(checked.constant(value.id), Some(&Value::Dint(5)));
    }

    #[test]
    fn an_untyped_real_literal_falls_back_to_lreal() {
        let checked = checked_ok(&program("x : LREAL := 1.5;", ""));
        assert_eq!(
            symbol_of(&checked, "Main", "x").init,
            Some(Value::Lreal(1.5))
        );
    }

    #[test]
    fn a_negative_literal_at_the_edge_of_its_type_is_accepted() {
        // -128 fits SINT and 128 does not. Whether the sign is part of the
        // literal or a unary minus applied to it is the lexer's business, and
        // the answer must not change whether this compiles.
        assert!(program_errors("x : SINT := -128;", "").is_empty());
        assert_eq!(program_errors("x : SINT := 128;", ""), ["E0404"]);
        assert!(program_errors("x : SINT := SINT#-128;", "").is_empty());
        assert!(program_errors("x : SINT := -(128);", "").is_empty());
        let checked = checked_ok(&program("x : SINT := -128;", ""));
        assert_eq!(
            symbol_of(&checked, "Main", "x").init,
            Some(Value::Sint(-128))
        );
    }

    #[test]
    fn a_typed_literal_keeps_the_type_its_prefix_names() {
        let checked = checked_ok(&program("x : DINT := INT#5;", ""));
        assert_eq!(symbol_of(&checked, "Main", "x").init, Some(Value::Dint(5)));
    }

    #[test]
    fn a_typed_literal_whose_value_does_not_fit_is_refused() {
        assert_eq!(program_errors("x : DINT := SINT#300;", ""), ["E0404"]);
    }

    #[test]
    fn a_literal_outside_a_subrange_is_refused() {
        assert_eq!(program_errors("v : INT (0..10) := 50;", ""), ["E0404"]);
        assert!(program_errors("v : INT (0..10) := 5;", "").is_empty());
    }

    #[test]
    fn a_string_literal_longer_than_its_target_is_refused() {
        assert_eq!(program_errors("s : STRING[2] := 'abcd';", ""), ["E0404"]);
        assert!(program_errors("s : STRING[8] := 'abcd';", "").is_empty());
    }

    #[test]
    fn a_literal_in_a_real_context_becomes_that_real() {
        let checked = checked_ok(&program("x : REAL := 3;", ""));
        assert_eq!(
            symbol_of(&checked, "Main", "x").init,
            Some(Value::real(3.0))
        );
    }

    // -- implicit conversion --------------------------------------------------

    #[test]
    fn int_widens_to_real_and_dint_does_not() {
        // A 24-bit significand cannot hold every 32-bit integer, which is why
        // IEC 61131-3:2013 Figure 12 has the one edge and not the other.
        assert!(program_errors("i : INT;\nr : REAL;", "r := i;").is_empty());
        assert_eq!(
            program_errors("wide : DINT;\nr : REAL;", "r := wide;"),
            ["E0401"]
        );
    }

    #[test]
    fn a_narrowing_assignment_names_the_conversion_function() {
        let src = program("wide : DINT;\ni : INT;", "i := wide;");
        let (_, diags, _) = check_text(&src);
        assert_eq!(codes_of(&diags, Severity::Error), ["E0401"]);
        assert!(message(&diags, "E0401").contains("DINT_TO_INT"));
    }

    #[test]
    fn bool_widening_follows_the_dialect_and_the_diagnostic_says_which_rule_was_applied() {
        let src = program("flag : BOOL;\nbits : BYTE;", "bits := flag;");
        assert!(errors(&src).is_empty(), "the generic dialect permits it");

        let (_, diags, _) = check_with(&Dialect::strict_iec(), &src);
        assert_eq!(codes_of(&diags, Severity::Error), ["E0401"]);
        let rule = diags
            .items()
            .iter()
            .find(|d| d.code.0 == "E0401")
            .and_then(|d| d.dialect_rule.clone())
            .expect("the dialect rule");
        assert!(rule.starts_with("iec61131-3:2013-strict:"), "{rule}");
    }

    #[test]
    fn a_value_of_an_unrelated_type_cannot_be_assigned() {
        assert_eq!(
            program_errors("flag : BOOL;\nn : INT;", "n := flag;"),
            ["E0401"]
        );
        assert_eq!(
            program_errors("s : STRING;\nn : INT;", "n := s;"),
            ["E0401"]
        );
    }

    // -- name resolution ------------------------------------------------------

    #[test]
    fn an_unknown_name_is_reported_once_and_a_similar_one_is_offered() {
        let src = program("Motor_Run : BOOL;", "Motor_Rn := TRUE;");
        let (_, diags, _) = check_text(&src);
        assert_eq!(codes_of(&diags, Severity::Error), ["E0302"]);
        assert!(message(&diags, "E0302").contains("Motor_Run"));
    }

    #[test]
    fn a_local_shadows_a_global_of_the_same_name() {
        let src = "VAR_GLOBAL Level : INT; END_VAR\n\
                   PROGRAM Main VAR Level : DINT; END_VAR Level := 1; END_PROGRAM\n";
        let checked = checked_ok(src);
        let unit = parse_only(src);
        let stmt = main_body(&unit).first().cloned().expect("a statement");
        let StmtKind::Assign { target, .. } = stmt.kind else {
            panic!("expected an assignment");
        };
        assert!(matches!(
            checked.resolution(target.id),
            Some(Resolution::Local { .. })
        ));
    }

    #[test]
    fn a_global_is_found_when_no_local_hides_it() {
        let src = "VAR_GLOBAL Level : INT; END_VAR\n\
                   PROGRAM Main Level := 1; END_PROGRAM\n";
        let checked = checked_ok(src);
        let unit = parse_only(src);
        let stmt = main_body(&unit).first().cloned().expect("a statement");
        let StmtKind::Assign { target, .. } = stmt.kind else {
            panic!("expected an assignment");
        };
        assert_eq!(
            checked.resolution(target.id),
            Some(Resolution::Global { symbol: 0 })
        );
    }

    #[test]
    fn a_function_name_used_as_a_value_outside_the_function_is_refused() {
        assert_eq!(
            errors(
                "FUNCTION F : INT F := 1; END_FUNCTION\n\
                 PROGRAM Main VAR n : INT; END_VAR n := F; END_PROGRAM\n"
            ),
            ["E0314"]
        );
    }

    // -- members --------------------------------------------------------------

    #[test]
    fn a_timers_output_reads_as_a_member_of_its_instance() {
        let src = program("t : TON;\nrunning : BOOL;", "running := t.Q;");
        let checked = checked_ok(&src);
        let unit = parse_only(&src);
        let value = first_value(&unit);
        assert_eq!(
            checked.type_of(value.id),
            Some(checked.types.elementary(ElementaryType::Bool))
        );
        let Some(Resolution::Member { offset, .. }) = checked.resolution(value.id) else {
            panic!("Timer.Q did not resolve to a member");
        };
        assert_eq!(
            Some(offset),
            crate::stdlib::field_offset(NativeBlock::Ton, "Q")
        );
    }

    #[test]
    fn a_timers_preset_can_be_written_from_outside() {
        assert!(program_errors("t : TON;", "t.PT := T#5s;").is_empty());
    }

    #[test]
    fn a_blocks_internal_field_cannot_be_named_in_code() {
        let src = program("t : TON;\nb : BYTE;", "b := t.PHASE;");
        let (_, diags, _) = check_text(&src);
        assert_eq!(codes_of(&diags, Severity::Error), ["E0311"]);
        assert!(message(&diags, "E0311").contains("watch list"));
    }

    #[test]
    fn a_user_blocks_local_variable_cannot_be_named_from_outside() {
        assert_eq!(
            errors(
                "FUNCTION_BLOCK Motor VAR_INPUT Run : BOOL; END_VAR VAR Ticks : INT; END_VAR END_FUNCTION_BLOCK\n\
                 PROGRAM Main VAR m : Motor; n : INT; END_VAR n := m.Ticks; END_PROGRAM\n"
            ),
            ["E0311"]
        );
    }

    #[test]
    fn a_user_blocks_output_can_be_read_from_outside() {
        assert!(
            errors(
                "FUNCTION_BLOCK Motor VAR_OUTPUT Speed : INT; END_VAR END_FUNCTION_BLOCK\n\
                 PROGRAM Main VAR m : Motor; n : INT; END_VAR n := m.Speed; END_PROGRAM\n"
            )
            .is_empty()
        );
    }

    #[test]
    fn a_field_a_type_does_not_have_lists_the_ones_it_does() {
        let src = program("t : TON;\nb : BOOL;", "b := t.Nope;");
        let (_, diags, _) = check_text(&src);
        assert_eq!(codes_of(&diags, Severity::Error), ["E0310"]);
        let text = message(&diags, "E0310");
        assert!(
            text.contains("IN") && text.contains("PT") && text.contains('Q'),
            "{text}"
        );
        assert!(!text.contains("PHASE"), "internals are not offered: {text}");
    }

    #[test]
    fn a_structure_field_resolves_and_an_unknown_one_does_not() {
        let src = "TYPE Point : STRUCT x : INT; y : INT; END_STRUCT; END_TYPE\n\
                   PROGRAM Main VAR p : Point; n : INT; END_VAR n := p.x; END_PROGRAM\n";
        assert!(errors(src).is_empty());
        let bad = "TYPE Point : STRUCT x : INT; END_STRUCT; END_TYPE\n\
                   PROGRAM Main VAR p : Point; n : INT; END_VAR n := p.z; END_PROGRAM\n";
        assert_eq!(errors(bad), ["E0310"]);
    }

    #[test]
    fn something_with_no_fields_cannot_be_asked_for_one() {
        assert_eq!(program_errors("n : INT;", "n := n.field;"), ["E0323"]);
    }

    // -- enumerations ---------------------------------------------------------

    #[test]
    fn a_qualified_enumeration_value_resolves_to_its_number() {
        let src = "TYPE Colour : (Red, Green, Blue); END_TYPE\n\
                   PROGRAM Main VAR c : Colour; END_VAR c := Colour#Green; END_PROGRAM\n";
        let checked = checked_ok(src);
        let unit = parse_only(src);
        let value = first_value(&unit);
        assert!(matches!(
            checked.resolution(value.id),
            Some(Resolution::EnumValue { value: 1, .. })
        ));
    }

    #[test]
    fn an_unqualified_enumeration_value_resolves_from_the_type_the_context_wants() {
        let src = "TYPE Colour : (Red, Green, Blue); END_TYPE\n\
                   PROGRAM Main VAR c : Colour; END_VAR c := Blue; END_PROGRAM\n";
        let checked = checked_ok(src);
        let unit = parse_only(src);
        let value = first_value(&unit);
        assert!(matches!(
            checked.resolution(value.id),
            Some(Resolution::EnumValue { value: 2, .. })
        ));
    }

    #[test]
    fn a_value_the_enumeration_does_not_have_is_refused() {
        assert_eq!(
            errors(
                "TYPE Colour : (Red, Green); END_TYPE\n\
                 PROGRAM Main VAR c : Colour; END_VAR c := Colour#Purple; END_PROGRAM\n"
            ),
            ["E0310"]
        );
    }

    // -- operators ------------------------------------------------------------

    #[test]
    fn a_comparison_yields_bool_whatever_its_operands_were() {
        let src = program("a : INT;\nb : DINT;\nflag : BOOL;", "flag := a < b;");
        let checked = checked_ok(&src);
        let unit = parse_only(&src);
        let value = first_value(&unit);
        assert_eq!(
            checked.type_of(value.id),
            Some(checked.types.elementary(ElementaryType::Bool))
        );
    }

    #[test]
    fn two_operands_with_no_common_type_are_both_named() {
        // DINT and UDINT are both in ANY_NUM and neither widens to the other:
        // IEC 61131-3:2013 Figure 12 has no edge either way.
        let src = program(
            "wide : DINT;\nunsigned : UDINT;",
            "wide := wide + unsigned;",
        );
        let (_, diags, _) = check_text(&src);
        assert!(codes_of(&diags, Severity::Error).contains(&"E0402"));
        let text = message(&diags, "E0402");
        assert!(text.contains("DINT") && text.contains("UDINT"), "{text}");
    }

    #[test]
    fn an_operand_outside_the_operators_domain_names_the_generic_type_it_accepts() {
        let src = program("r : LREAL;\nn : LREAL;", "n := r MOD r;");
        let (_, diags, _) = check_text(&src);
        assert_eq!(codes_of(&diags, Severity::Error), ["E0403"]);
        assert!(message(&diags, "E0403").contains("ANY_INT"));
    }

    #[test]
    fn each_operand_keeps_its_own_type_and_the_operation_takes_the_common_one() {
        // This is what tells the compiler where a conversion has to be emitted.
        let src = program("small : INT;\nwide : DINT;", "wide := wide + small;");
        let checked = checked_ok(&src);
        let unit = parse_only(&src);
        let value = first_value(&unit);
        let ExprKind::Binary { lhs, rhs, .. } = &value.kind else {
            panic!("expected a binary operation");
        };
        assert_eq!(
            checked.type_of(value.id),
            Some(checked.types.elementary(ElementaryType::Dint))
        );
        assert_eq!(
            checked.type_of(lhs.id),
            Some(checked.types.elementary(ElementaryType::Dint))
        );
        assert_eq!(
            checked.type_of(rhs.id),
            Some(checked.types.elementary(ElementaryType::Int))
        );
    }

    #[test]
    fn a_literal_operand_takes_the_type_of_the_variable_it_is_combined_with() {
        let src = program("count : SINT;", "count := count + 1;");
        let checked = checked_ok(&src);
        let unit = parse_only(&src);
        let value = first_value(&unit);
        let ExprKind::Binary { rhs, .. } = &value.kind else {
            panic!("expected a binary operation");
        };
        assert_eq!(
            checked.type_of(rhs.id),
            Some(checked.types.elementary(ElementaryType::Sint))
        );
    }

    #[test]
    fn the_dereference_operator_is_reported_as_not_implemented() {
        assert_eq!(program_errors("n : INT;", "n := n^;"), ["U0301"]);
    }

    #[test]
    fn the_assignment_attempt_is_reported_as_not_implemented() {
        assert_eq!(program_errors("n : INT;\nm : INT;", "n ?= m;"), ["U0301"]);
    }

    // -- constant folding -----------------------------------------------------

    #[test]
    fn arithmetic_on_constants_is_folded_and_recorded() {
        let src = program("n : DINT;", "n := 2 * 3 + 4;");
        let checked = checked_ok(&src);
        let unit = parse_only(&src);
        let value = first_value(&unit);
        assert_eq!(checked.constant(value.id), Some(&Value::Dint(10)));
    }

    #[test]
    fn folding_wraps_the_way_the_runtime_wraps() {
        // If the folder saturated where the runtime wraps, a constant and the
        // same sum computed at run time would disagree.
        let src = program("n : SINT;", "n := SINT#127 + SINT#1;");
        let checked = checked_ok(&src);
        let unit = parse_only(&src);
        let value = first_value(&unit);
        assert_eq!(checked.constant(value.id), Some(&Value::Sint(-128)));
    }

    #[test]
    fn a_constant_variable_folds_to_its_initial_value() {
        let src = "PROGRAM Main\nVAR CONSTANT Limit : INT := 7; END_VAR\nVAR n : INT; END_VAR\n\
                   n := Limit + 1;\nEND_PROGRAM\n";
        let checked = checked_ok(src);
        let unit = parse_only(src);
        let value = first_value(&unit);
        assert_eq!(checked.constant(value.id), Some(&Value::Int(8)));
    }

    #[test]
    fn a_variable_that_is_not_constant_does_not_fold() {
        let src = program("n : INT;\nm : INT;", "m := n + 1;");
        let checked = checked_ok(&src);
        let unit = parse_only(&src);
        let value = first_value(&unit);
        assert_eq!(checked.constant(value.id), None);
    }

    #[test]
    fn division_by_a_constant_zero_is_found_before_the_program_runs() {
        assert_eq!(program_errors("n : DINT;", "n := 10 / 0;"), ["E0415"]);
        assert!(program_errors("n : DINT;\ndivisor : DINT;", "n := 10 / divisor;").is_empty());
    }

    #[test]
    fn a_duration_multiplied_by_a_constant_folds() {
        let src = program("t : TIME;", "t := T#2s * 3;");
        let checked = checked_ok(&src);
        let unit = parse_only(&src);
        let value = first_value(&unit);
        assert_eq!(
            checked
                .constant(value.id)
                .and_then(Value::as_duration)
                .map(salman_core::time::Duration::nanos),
            Some(6_000_000_000)
        );
    }

    // -- subscripts -----------------------------------------------------------

    #[test]
    fn only_an_array_can_be_subscripted() {
        assert_eq!(program_errors("n : INT;", "n := n[1];"), ["E0406"]);
    }

    #[test]
    fn the_number_of_subscripts_must_match_the_number_of_dimensions() {
        let src = program("a : ARRAY [1..4] OF INT;\nn : INT;", "n := a[1, 2];");
        let (_, diags, _) = check_text(&src);
        assert_eq!(codes_of(&diags, Severity::Error), ["E0407"]);
        assert!(message(&diags, "E0407").contains("1 dimension"));
    }

    #[test]
    fn a_subscript_must_be_an_integer() {
        assert_eq!(
            program_errors(
                "a : ARRAY [1..4] OF INT;\nn : INT;\nflag : BOOL;",
                "n := a[flag];"
            ),
            ["E0408"]
        );
    }

    #[test]
    fn a_constant_subscript_outside_the_declared_bounds_is_a_compile_time_error() {
        let src = program("a : ARRAY [1..4] OF INT;\nn : INT;", "n := a[9];");
        let (_, diags, _) = check_text(&src);
        assert_eq!(codes_of(&diags, Severity::Error), ["E0409"]);
        assert!(message(&diags, "E0409").contains("1..4"));
    }

    #[test]
    fn a_constant_subscript_inside_the_bounds_is_accepted() {
        assert!(program_errors("a : ARRAY [1..4] OF INT;\nn : INT;", "n := a[4];").is_empty());
    }

    #[test]
    fn a_two_dimensional_array_indexes_by_both_bounds() {
        assert!(
            program_errors(
                "grid : ARRAY [0..3, 0..1] OF INT;\nn : INT;",
                "n := grid[3, 1];"
            )
            .is_empty()
        );
        assert_eq!(
            program_errors(
                "grid : ARRAY [0..3, 0..1] OF INT;\nn : INT;",
                "n := grid[3, 2];"
            ),
            ["E0409"]
        );
    }

    // -- calls ----------------------------------------------------------------

    const ADD: &str =
        "FUNCTION Add : INT VAR_INPUT a : INT; b : INT; END_VAR Add := a + b; END_FUNCTION\n";

    #[test]
    fn a_function_takes_positional_or_named_arguments() {
        // IEC 61131-3:2013 Table 20 "Function call" gives both forms.
        assert!(errors(&format!("{ADD}{}", program("n : INT;", "n := Add(1, 2);"))).is_empty());
        assert!(
            errors(&format!(
                "{ADD}{}",
                program("n : INT;", "n := Add(a := 1, b := 2);")
            ))
            .is_empty()
        );
    }

    #[test]
    fn a_call_with_the_wrong_number_of_arguments_is_refused() {
        assert_eq!(
            errors(&format!("{ADD}{}", program("n : INT;", "n := Add(1);"))),
            ["E0317"]
        );
    }

    #[test]
    fn a_named_call_that_leaves_a_parameter_unbound_is_refused() {
        assert_eq!(
            errors(&format!(
                "{ADD}{}",
                program("n : INT;", "n := Add(a := 1);")
            )),
            ["E0317"]
        );
    }

    #[test]
    fn an_argument_of_the_wrong_type_is_refused() {
        assert_eq!(
            errors(&format!(
                "{ADD}{}",
                program("n : INT;\nwide : DINT;", "n := Add(wide, 2);")
            )),
            ["E0401"]
        );
    }

    #[test]
    fn a_call_may_not_mix_positional_and_named_arguments() {
        assert_eq!(
            errors(&format!(
                "{ADD}{}",
                program("n : INT;", "n := Add(1, b := 2);")
            )),
            ["E0322"]
        );
    }

    #[test]
    fn an_unknown_function_parameter_lists_the_ones_that_exist() {
        let src = format!("{ADD}{}", program("n : INT;", "n := Add(a := 1, c := 2);"));
        let (_, diags, _) = check_text(&src);
        assert!(codes_of(&diags, Severity::Error).contains(&"E0316"));
        assert!(message(&diags, "E0316").contains("`a`"));
    }

    #[test]
    fn a_functions_result_type_is_the_type_of_the_call() {
        let src = format!("{ADD}{}", program("n : INT;", "n := Add(1, 2);"));
        let checked = checked_ok(&src);
        let unit = parse_only(&src);
        let value = first_value(&unit);
        assert_eq!(
            checked.type_of(value.id),
            Some(checked.types.elementary(ElementaryType::Int))
        );
    }

    #[test]
    fn positional_arguments_to_a_function_block_are_refused_citing_the_call_table() {
        let src = program("t : TON;", "t(TRUE, T#1s);");
        let (_, diags, _) = check_text(&src);
        assert!(codes_of(&diags, Severity::Error).contains(&"E0315"));
        let text = message(&diags, "E0315");
        assert!(text.contains("Table 42"), "{text}");
        assert!(text.contains("Function block call"), "{text}");
    }

    #[test]
    fn an_unknown_function_block_parameter_lists_the_ones_that_exist() {
        let src = program("t : TON;", "t(IN := TRUE, PRESET := T#1s);");
        let (_, diags, _) = check_text(&src);
        assert_eq!(codes_of(&diags, Severity::Error), ["E0316"]);
        let text = message(&diags, "E0316");
        assert!(text.contains("`IN`") && text.contains("`PT`"), "{text}");
    }

    #[test]
    fn a_function_block_call_produces_no_value() {
        assert_eq!(
            program_errors("t : TON;\nflag : BOOL;", "flag := t(IN := TRUE);"),
            ["E0318"]
        );
    }

    #[test]
    fn a_program_cannot_be_called() {
        assert_eq!(
            errors("PROGRAM Other END_PROGRAM\nPROGRAM Main Other(); END_PROGRAM\n"),
            ["E0319"]
        );
    }

    #[test]
    fn a_function_block_type_cannot_be_called_in_place_of_an_instance() {
        assert_eq!(
            errors(
                "FUNCTION_BLOCK Motor VAR_INPUT Run : BOOL; END_VAR END_FUNCTION_BLOCK\n\
                 PROGRAM Main Motor(Run := TRUE); END_PROGRAM\n"
            ),
            ["E0319"]
        );
    }

    #[test]
    fn a_standard_block_type_cannot_be_called_in_place_of_an_instance() {
        assert_eq!(
            program_errors("flag : BOOL;", "TON(IN := flag);"),
            ["E0319"]
        );
    }

    #[test]
    fn a_plain_variable_cannot_be_called() {
        assert_eq!(program_errors("n : INT;", "n();"), ["E0314"]);
    }

    #[test]
    fn an_output_binding_writes_the_variable_it_names() {
        assert!(
            program_errors(
                "t : TON;\nrunning : BOOL;",
                "t(IN := TRUE, PT := T#1s, Q => running);"
            )
            .is_empty()
        );
        assert_eq!(
            program_errors("t : TON;\nn : INT;", "t(IN := TRUE, PT := T#1s, Q => n);"),
            ["E0401"]
        );
    }

    #[test]
    fn an_output_binding_may_not_write_a_constant() {
        assert_eq!(
            errors(
                "PROGRAM Main\nVAR CONSTANT Fixed : BOOL := FALSE; END_VAR\nVAR t : TON; END_VAR\n\
                 t(IN := TRUE, PT := T#1s, Q => Fixed);\nEND_PROGRAM\n"
            ),
            ["E0313"]
        );
    }

    #[test]
    fn a_ton_instance_works_from_declaration_to_output() {
        let src = program(
            "Delay : TON;\nStart : BOOL;\nRunning : BOOL;\nElapsed : TIME;",
            "Delay(IN := Start, PT := T#5s);\nRunning := Delay.Q;\nElapsed := Delay.ET;",
        );
        let checked = checked_ok(&src);
        let symbol = symbol_of(&checked, "Main", "Delay");
        assert_eq!(checked.native_block(symbol.ty), Some(NativeBlock::Ton));
        let unit = parse_only(&src);
        let value = first_value(&unit);
        assert_eq!(
            checked.type_of(value.id),
            Some(checked.types.elementary(ElementaryType::Bool))
        );
    }

    #[test]
    fn a_user_function_block_is_called_through_its_instance() {
        assert!(
            errors(
                "FUNCTION_BLOCK Ramp VAR_INPUT Amount : INT; END_VAR VAR_OUTPUT Level : INT; END_VAR \
                 Level := Level + Amount; END_FUNCTION_BLOCK\n\
                 PROGRAM Main VAR r : Ramp; n : INT; END_VAR r(Amount := 2); n := r.Level; END_PROGRAM\n"
            )
            .is_empty()
        );
    }

    // -- recursion ------------------------------------------------------------

    #[test]
    fn direct_recursion_is_rejected_statically() {
        let src =
            "FUNCTION Fact : DINT VAR_INPUT n : DINT; END_VAR Fact := Fact(n); END_FUNCTION\n";
        let (_, diags, _) = check_text(src);
        assert!(codes_of(&diags, Severity::Error).contains(&"E0309"));
        assert!(message(&diags, "E0309").contains("Fact calls Fact"));
    }

    #[test]
    fn mutual_recursion_names_the_whole_cycle() {
        let src = "FUNCTION A : INT A := B(); END_FUNCTION\n\
                   FUNCTION B : INT B := A(); END_FUNCTION\n";
        let (_, diags, _) = check_text(src);
        assert!(codes_of(&diags, Severity::Error).contains(&"E0309"));
        let text = message(&diags, "E0309");
        assert!(text.contains('A') && text.contains('B'), "{text}");
        assert!(
            text.contains("salman"),
            "the refusal is stated as salman's: {text}"
        );
    }

    #[test]
    fn recursion_through_a_function_block_instance_is_rejected() {
        let src = "FUNCTION_BLOCK Looper VAR self : Looper; END_VAR self(); END_FUNCTION_BLOCK\n";
        assert!(errors(src).contains(&"E0309"));
    }

    #[test]
    fn a_call_graph_without_a_cycle_is_accepted() {
        assert!(
            errors(
                "FUNCTION Inner : INT Inner := 1; END_FUNCTION\n\
                 FUNCTION Outer : INT Outer := Inner() + Inner(); END_FUNCTION\n\
                 PROGRAM Main VAR n : INT; END_VAR n := Outer(); END_PROGRAM\n"
            )
            .is_empty()
        );
    }

    #[test]
    fn one_cycle_is_reported_once_however_many_ways_round_it_there_are() {
        let src = "FUNCTION A : INT A := B(); END_FUNCTION\n\
                   FUNCTION B : INT B := A() + A(); END_FUNCTION\n";
        let (_, diags, _) = check_text(src);
        let cycles = codes_of(&diags, Severity::Error)
            .into_iter()
            .filter(|code| *code == "E0309")
            .count();
        assert_eq!(cycles, 1);
    }

    // -- assignment -----------------------------------------------------------

    #[test]
    fn a_pou_may_not_assign_to_its_own_var_input() {
        let src =
            "FUNCTION_BLOCK Motor VAR_INPUT Run : BOOL; END_VAR Run := FALSE; END_FUNCTION_BLOCK\n";
        let (_, diags, _) = check_text(src);
        assert_eq!(codes_of(&diags, Severity::Error), ["E0313"]);
        assert!(message(&diags, "E0313").contains("VAR_INPUT"));
    }

    #[test]
    fn a_constant_may_not_be_assigned_to() {
        let src =
            "PROGRAM Main\nVAR CONSTANT Limit : INT := 7; END_VAR\nLimit := 8;\nEND_PROGRAM\n";
        let (_, diags, _) = check_text(src);
        assert_eq!(codes_of(&diags, Severity::Error), ["E0313"]);
        assert!(message(&diags, "E0313").contains("CONSTANT"));
    }

    #[test]
    fn a_pou_may_assign_to_its_own_var_output_and_locals() {
        assert!(
            errors(
                "FUNCTION_BLOCK Motor VAR_INPUT Run : BOOL; END_VAR VAR_OUTPUT Speed : INT; END_VAR \
                 VAR Ticks : INT; END_VAR Speed := 1; Ticks := 2; END_FUNCTION_BLOCK\n"
            )
            .is_empty()
        );
    }

    #[test]
    fn a_literal_cannot_be_assigned_to() {
        // The parser accepts `1 := 2;` as an assignment so that the checker can
        // say what is actually wrong with it.
        assert!(program_errors("n : INT;", "1 := n;").contains(&"E0312"));
    }

    #[test]
    fn an_array_element_and_a_field_can_be_assigned_to() {
        assert!(
            program_errors(
                "a : ARRAY [1..4] OF INT;\nt : TON;",
                "a[1] := 2;\nt.IN := TRUE;"
            )
            .is_empty()
        );
    }

    #[test]
    fn a_direct_address_can_be_assigned_to_and_takes_its_type_from_its_size_letter() {
        let src = in_program("VAR w : WORD; END_VAR %QW1 := w;");
        let checked = checked_ok(&src);
        let unit = parse_only(&src);
        let stmt = main_body(&unit).first().cloned().expect("a statement");
        let StmtKind::Assign { target, .. } = stmt.kind else {
            panic!("expected an assignment");
        };
        assert_eq!(
            checked.type_of(target.id),
            Some(checked.types.elementary(ElementaryType::Word))
        );
    }

    // -- conditions -----------------------------------------------------------

    #[test]
    fn a_condition_that_is_not_bool_is_refused_and_says_why() {
        let src = program("count : DINT;", "IF count THEN ; END_IF");
        let (_, diags, _) = check_text(&src);
        assert_eq!(codes_of(&diags, Severity::Error), ["E0405"]);
        assert!(message(&diags, "E0405").contains("Count > 0"));
    }

    #[test]
    fn a_bool_condition_is_accepted_in_all_three_loops_and_in_if() {
        assert!(
            program_errors(
                "flag : BOOL;",
                "IF flag THEN ; END_IF\n\
                 WHILE flag DO ; END_WHILE\n\
                 REPEAT ; UNTIL flag END_REPEAT"
            )
            .is_empty()
        );
    }

    #[test]
    fn a_while_condition_that_is_not_bool_is_refused() {
        assert_eq!(
            program_errors("n : INT;", "WHILE n DO ; END_WHILE"),
            ["E0405"]
        );
    }

    #[test]
    fn a_repeat_condition_that_is_not_bool_is_refused() {
        assert_eq!(
            program_errors("n : INT;", "REPEAT ; UNTIL n END_REPEAT"),
            ["E0405"]
        );
    }

    // -- CASE -----------------------------------------------------------------

    #[test]
    fn duplicate_case_labels_are_refused_by_a_salman_rule() {
        let src = program("n : INT;", "CASE n OF 1: ; 1: ; END_CASE");
        let (_, diags, _) = check_text(&src);
        assert_eq!(codes_of(&diags, Severity::Error), ["E0208"]);
        assert!(message(&diags, "E0208").contains("salman rule"));
    }

    #[test]
    fn overlapping_case_labels_are_refused_by_a_salman_rule() {
        assert_eq!(
            program_errors("n : INT;", "CASE n OF 1..5: ; 4..8: ; END_CASE"),
            ["E0209"]
        );
    }

    #[test]
    fn two_enumeration_labels_that_are_the_same_value_clash_however_they_are_written() {
        // The parser cannot see this: `Colour#Red` and `Red` are two spellings
        // of one value, and only the checker knows the enumeration.
        assert_eq!(
            errors(
                "TYPE Colour : (Red, Green); END_TYPE\n\
                 PROGRAM Main VAR c : Colour; END_VAR \
                 CASE c OF Colour#Red: ; Red: ; END_CASE END_PROGRAM\n"
            ),
            ["E0208"]
        );
    }

    #[test]
    fn distinct_case_labels_are_accepted() {
        assert!(
            program_errors("n : INT;", "CASE n OF 1: ; 2..4: ; 5, 6: ; ELSE ; END_CASE").is_empty()
        );
    }

    #[test]
    fn a_case_selector_must_be_an_integer_or_an_enumeration() {
        assert_eq!(
            program_errors("flag : BOOL;", "CASE flag OF 1: ; END_CASE"),
            ["E0410"]
        );
    }

    #[test]
    fn an_enumeration_selects_a_case_arm() {
        assert!(
            errors(
                "TYPE Colour : (Red, Green); END_TYPE\n\
                 PROGRAM Main VAR c : Colour; END_VAR CASE c OF Red: ; Green: ; END_CASE END_PROGRAM\n"
            )
            .is_empty()
        );
    }

    #[test]
    fn a_case_label_of_the_wrong_type_is_refused() {
        assert_eq!(
            program_errors(
                "n : INT;\ns : STRING;",
                "CASE n OF 1: ; END_CASE\nCASE s OF 1: ; END_CASE"
            ),
            ["E0410"]
        );
        assert_eq!(
            program_errors("n : INT;", "CASE n OF T#1s: ; END_CASE"),
            ["E0411"]
        );
    }

    #[test]
    fn a_case_label_that_is_not_constant_is_refused() {
        assert_eq!(
            program_errors("n : INT;\nm : INT;", "CASE n OF m: ; END_CASE"),
            ["E0307"]
        );
    }

    #[test]
    fn a_case_label_may_be_a_constant_variable() {
        assert!(
            errors(
                "PROGRAM Main\nVAR CONSTANT Ready : INT := 3; END_VAR\nVAR n : INT; END_VAR\n\
                 CASE n OF Ready: ; ELSE ; END_CASE\nEND_PROGRAM\n"
            )
            .is_empty()
        );
    }

    #[test]
    fn two_constant_labels_of_one_value_clash_even_though_they_are_spelled_differently() {
        assert_eq!(
            errors(
                "PROGRAM Main\nVAR CONSTANT Ready : INT := 3; END_VAR\nVAR n : INT; END_VAR\n\
                 CASE n OF Ready: ; 3: ; END_CASE\nEND_PROGRAM\n"
            ),
            ["E0208"]
        );
    }

    // -- FOR ------------------------------------------------------------------

    #[test]
    fn a_for_control_variable_must_be_a_variable_of_the_pou() {
        assert_eq!(
            program_errors("n : INT;", "FOR missing := 1 TO 3 DO ; END_FOR"),
            ["E0412"]
        );
        assert_eq!(
            errors(
                "VAR_GLOBAL i : INT; END_VAR\nPROGRAM Main FOR i := 1 TO 3 DO ; END_FOR END_PROGRAM\n"
            ),
            ["E0412"],
            "a global control variable would be shared between tasks"
        );
    }

    #[test]
    fn a_for_control_variable_must_be_an_integer() {
        assert_eq!(
            program_errors("r : REAL;", "FOR r := 1 TO 3 DO ; END_FOR"),
            ["E0412"]
        );
    }

    #[test]
    fn a_for_control_variable_must_be_writable() {
        assert_eq!(
            errors(
                "FUNCTION_BLOCK B VAR_INPUT i : INT; END_VAR FOR i := 1 TO 3 DO ; END_FOR END_FUNCTION_BLOCK\n"
            ),
            ["E0412"]
        );
    }

    #[test]
    fn a_for_bound_that_is_not_an_integer_is_refused() {
        assert_eq!(
            program_errors("i : INT;\nr : REAL;", "FOR i := 1 TO r DO ; END_FOR"),
            ["E0413"]
        );
    }

    #[test]
    fn a_for_loop_with_a_constant_step_of_zero_is_refused() {
        assert_eq!(
            program_errors("i : INT;", "FOR i := 1 TO 10 BY 0 DO ; END_FOR"),
            ["E0414"]
        );
        assert!(program_errors("i : INT;", "FOR i := 1 TO 10 BY 2 DO ; END_FOR").is_empty());
    }

    #[test]
    fn the_body_of_a_for_loop_may_not_assign_to_its_control_variable() {
        let src = program("i : INT;", "FOR i := 1 TO 10 DO i := 4; END_FOR");
        let (_, diags, _) = check_text(&src);
        assert_eq!(codes_of(&diags, Severity::Error), ["E0210"]);
        assert!(message(&diags, "E0210").contains("salman rule"));
    }

    #[test]
    fn a_for_loops_control_variable_is_recorded_against_the_statement() {
        let src = program("i : INT;", "FOR i := 1 TO 10 DO ; END_FOR");
        let checked = checked_ok(&src);
        let unit = parse_only(&src);
        let stmt = main_body(&unit).first().cloned().expect("a statement");
        assert!(matches!(
            checked.resolution(stmt.id),
            Some(Resolution::Local { .. })
        ));
    }

    // -- EXIT, CONTINUE and RETURN --------------------------------------------

    #[test]
    fn exit_outside_a_loop_is_refused() {
        assert_eq!(program_errors("n : INT;", "EXIT;"), ["E0320"]);
    }

    #[test]
    fn continue_outside_a_loop_is_refused() {
        // CONTINUE is standard in Edition 3 (Table 72 row 9); salman does not
        // warn about the statement itself, only about where it is.
        assert_eq!(program_errors("n : INT;", "CONTINUE;"), ["E0320"]);
    }

    #[test]
    fn exit_and_continue_inside_a_loop_are_accepted() {
        assert!(
            program_errors(
                "i : INT;\nflag : BOOL;",
                "FOR i := 1 TO 3 DO IF flag THEN EXIT; ELSE CONTINUE; END_IF END_FOR\n\
                 WHILE flag DO EXIT; END_WHILE\n\
                 REPEAT CONTINUE; UNTIL flag END_REPEAT"
            )
            .is_empty()
        );
    }

    #[test]
    fn return_is_accepted_anywhere_including_inside_a_function() {
        assert!(errors("FUNCTION F : INT F := 1; RETURN; END_FUNCTION\n").is_empty());
        assert!(program_errors("n : INT;", "RETURN;").is_empty());
    }

    #[test]
    fn a_function_that_never_assigns_its_result_gets_a_warning_and_not_an_error() {
        let src = "FUNCTION F : INT VAR n : INT; END_VAR n := 1; END_FUNCTION\n";
        assert!(errors(src).is_empty());
        assert_eq!(warnings(src), ["W0301"]);
    }

    #[test]
    fn a_function_that_assigns_its_result_on_one_branch_only_is_not_warned_about() {
        // salman policy: this is not a definite-assignment analysis, and says so.
        assert!(
            warnings(
                "FUNCTION F : INT VAR_INPUT flag : BOOL; END_VAR IF flag THEN F := 1; END_IF END_FUNCTION\n"
            )
            .is_empty()
        );
    }

    // -- configurations -------------------------------------------------------

    const PLANT: &str = "PROGRAM Conveyor VAR n : INT; END_VAR n := 1; END_PROGRAM\n\
                         VAR_GLOBAL Trigger : BOOL; END_VAR\n";

    fn configuration(body: &str) -> String {
        format!(
            "{PLANT}CONFIGURATION Plant\nRESOURCE R1 ON CPU\n{body}\nEND_RESOURCE\nEND_CONFIGURATION\n"
        )
    }

    #[test]
    fn a_configuration_produces_its_tasks_and_the_programs_bound_to_them() {
        let src = configuration(
            "TASK Fast (INTERVAL := T#10ms, PRIORITY := 1);\nPROGRAM P1 WITH Fast : Conveyor;",
        );
        let checked = checked_ok(&src);
        assert_eq!(checked.configurations.len(), 1);
        let config = checked.configurations.first().expect("the configuration");
        assert_eq!(config.name.as_str(), "plant");
        assert_eq!(config.tasks.len(), 1);
        let task = config.tasks.first().expect("the task");
        assert_eq!(
            task.trigger,
            CheckedTrigger::Cyclic {
                interval_ns: 10_000_000
            }
        );
        assert_eq!(task.priority, 1);
        assert_eq!(task.programs.len(), 1);
        assert!(config.untasked.is_empty());
    }

    #[test]
    fn a_program_with_no_task_runs_freewheeling_and_is_listed_as_untasked() {
        let src = configuration("PROGRAM P1 : Conveyor;");
        let checked = checked_ok(&src);
        let config = checked.configurations.first().expect("the configuration");
        assert!(config.tasks.is_empty());
        assert_eq!(config.untasked.len(), 1);
        let instance = config.untasked.first().expect("the instance");
        assert_eq!(instance.name.as_str(), "p1");
        assert_eq!(
            checked.pous.get(instance.pou as usize).map(|p| p.kind),
            Some(PouKind::Program)
        );
    }

    #[test]
    fn a_unit_with_no_configuration_produces_none() {
        let checked = checked_ok("PROGRAM Main END_PROGRAM\n");
        assert!(checked.configurations.is_empty());
    }

    #[test]
    fn an_interval_that_is_not_a_positive_constant_duration_is_refused() {
        assert_eq!(
            errors(&configuration("TASK Fast (INTERVAL := 10);")),
            ["E0321"]
        );
        assert_eq!(
            errors(&configuration("TASK Fast (INTERVAL := T#0s);")),
            ["E0321"]
        );
    }

    #[test]
    fn a_priority_that_does_not_fit_sixteen_bits_is_refused() {
        assert_eq!(
            errors(&configuration(
                "TASK Fast (INTERVAL := T#1s, PRIORITY := 70000);"
            )),
            ["E0321"]
        );
    }

    #[test]
    fn a_single_trigger_must_name_a_global_bool() {
        let src =
            configuration("TASK OnEvent (SINGLE := Trigger);\nPROGRAM P1 WITH OnEvent : Conveyor;");
        let checked = checked_ok(&src);
        let config = checked.configurations.first().expect("the configuration");
        let task = config.tasks.first().expect("the task");
        assert!(matches!(task.trigger, CheckedTrigger::Event { .. }));
    }

    #[test]
    fn a_single_trigger_that_names_something_else_is_refused() {
        assert_eq!(
            errors(&format!(
                "{PLANT}VAR_GLOBAL Level : INT; END_VAR\n\
                 CONFIGURATION Plant\nRESOURCE R1 ON CPU\nTASK T (SINGLE := Level);\n\
                 END_RESOURCE\nEND_CONFIGURATION\n"
            )),
            ["E0321"]
        );
    }

    #[test]
    fn a_task_that_is_both_cyclic_and_event_triggered_is_refused() {
        assert_eq!(
            errors(&configuration(
                "TASK Both (SINGLE := Trigger, INTERVAL := T#1s);"
            )),
            ["E0321"]
        );
    }

    #[test]
    fn a_program_instance_must_name_a_pou_that_is_a_program() {
        assert_eq!(
            errors(
                "FUNCTION_BLOCK Motor VAR_INPUT Run : BOOL; END_VAR END_FUNCTION_BLOCK\n\
                 CONFIGURATION Plant\nRESOURCE R1 ON CPU\nPROGRAM P1 : Motor;\n\
                 END_RESOURCE\nEND_CONFIGURATION\n"
            ),
            ["E0321"]
        );
        assert_eq!(errors(&configuration("PROGRAM P1 : Missing;")), ["E0321"]);
    }

    #[test]
    fn a_with_clause_naming_no_task_is_refused() {
        assert_eq!(
            errors(&configuration("PROGRAM P1 WITH Nothing : Conveyor;")),
            ["E0321"]
        );
    }

    #[test]
    fn two_tasks_of_one_name_are_refused() {
        assert_eq!(
            errors(&configuration(
                "TASK Fast (INTERVAL := T#1s);\nTASK FAST (INTERVAL := T#2s);"
            )),
            ["E0303"]
        );
    }

    #[test]
    fn a_configuration_global_is_visible_to_a_pou_body() {
        // salman flattens the three global scopes into one at v0.1, and this is
        // what that buys: a POU can name a variable the configuration declares.
        assert!(
            errors(
                "PROGRAM Conveyor Level := 1; END_PROGRAM\n\
                 CONFIGURATION Plant\nVAR_GLOBAL Level : INT; END_VAR\n\
                 RESOURCE R1 ON CPU\nPROGRAM P1 : Conveyor;\nEND_RESOURCE\nEND_CONFIGURATION\n"
            )
            .is_empty()
        );
    }

    // -- robustness -----------------------------------------------------------

    #[test]
    fn check_never_panics_on_a_unit_the_parser_could_not_finish() {
        // Each of these produces parse errors and an incomplete tree. The
        // checker has to run on it without a second failure mode, because
        // every editor feature downstream runs on exactly this.
        let broken = [
            "",
            "PROGRAM",
            "PROGRAM Main",
            "PROGRAM Main VAR x : END_VAR END_PROGRAM",
            "PROGRAM Main x := ; END_PROGRAM",
            "PROGRAM Main IF THEN END_IF END_PROGRAM",
            "FUNCTION F : END_FUNCTION",
            "FUNCTION_BLOCK",
            "TYPE T : END_TYPE",
            "TYPE T : STRUCT END_TYPE",
            "CONFIGURATION",
            "CONFIGURATION C RESOURCE R TASK T ( END_RESOURCE END_CONFIGURATION",
            "PROGRAM Main FOR := 1 TO DO END_FOR END_PROGRAM",
            "PROGRAM Main CASE OF END_CASE END_PROGRAM",
            "PROGRAM Main a[ := 1; END_PROGRAM",
            "))) *** &&&",
            "PROGRAM Main VAR a : ARRAY [ ] OF ; END_VAR END_PROGRAM",
            "PROGRAM Main t(IN := ); END_PROGRAM",
        ];
        for src in broken {
            let mut map = SourceMap::new();
            let file = map.add("broken.st", src).expect("a source file");
            let (unit, _) = parse_source(file, src, &Dialect::generic());
            let (checked, _) = check(&unit, &Dialect::generic());
            // A Checked always comes back, however broken the input was.
            assert_eq!(
                checked.expr_types.len(),
                usize::try_from(unit.node_count).unwrap_or(0)
            );
        }
    }

    #[test]
    fn a_program_with_ten_distinct_errors_reports_about_ten_diagnostics_not_one() {
        let src = "PROGRAM Main\n\
                   VAR\n\
                   a : INT;\n\
                   flag : BOOL;\n\
                   arr : ARRAY [1..4] OF INT;\n\
                   t : TON;\n\
                   END_VAR\n\
                   VAR CONSTANT k : INT := 3; END_VAR\n\
                   Missing := 1;\n\
                   a := TRUE;\n\
                   k := 2;\n\
                   IF a THEN ; END_IF\n\
                   arr[9] := 1;\n\
                   a := arr[1, 2];\n\
                   t(5);\n\
                   a := flag.Nope;\n\
                   EXIT;\n\
                   a := t.PHASE;\n\
                   END_PROGRAM\n";
        let (_, diags, map) = check_text(src);
        let mut codes = codes_of(&diags, Severity::Error);
        let count = codes.len();
        codes.sort_unstable();
        codes.dedup();
        assert!(
            codes.len() >= 10,
            "expected ten distinct errors, got {codes:?}\n{}",
            diags.render(&map)
        );
        assert!(
            (10..=16).contains(&count),
            "one mistake should produce about one diagnostic, got {count}\n{}",
            diags.render(&map)
        );
    }

    #[test]
    fn every_expression_in_a_clean_program_is_given_a_type() {
        let src = program(
            "a : INT;\nb : DINT;\nflag : BOOL;\narr : ARRAY [0..3] OF INT;\nt : TON;",
            "b := b + a;\n\
             flag := a < b;\n\
             arr[2] := a;\n\
             t(IN := flag, PT := T#1s);\n\
             flag := t.Q;\n\
             IF flag THEN a := -a; END_IF",
        );
        let checked = checked_ok(&src);
        let unit = parse_only(&src);
        let mut untyped = Vec::new();
        for stmt in &main_body(&unit) {
            for_each_expr(std::slice::from_ref(stmt), &mut |expr| {
                // A callee that is a function block instance has a type; the
                // call node itself is the one thing with no value.
                if checked.type_of(expr.id).is_none() {
                    untyped.push(expr.span);
                }
            });
        }
        assert!(untyped.is_empty(), "{untyped:?}");
    }

    #[test]
    fn checking_the_same_unit_twice_produces_the_same_diagnostics() {
        // Determinism is the whole promise; a checker whose output depended on
        // a hash order would break it before the runtime ever ran.
        let src = program(
            "a : INT;\nb : BOOL;",
            "a := b;\nUnknown := 1;\nIF a THEN ; END_IF",
        );
        let (_, first, map) = check_text(&src);
        let (_, second, _) = check_text(&src);
        assert_eq!(first.render(&map), second.render(&map));
    }
}
