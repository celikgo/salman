// SPDX-License-Identifier: Apache-2.0
//! Compiling a checked unit to bytecode.
//!
//! Citation policy: salman cites IEC 61131-3:2013 (Edition 3.0) clause, table
//! and figure numbers as references only. No IEC text is reproduced. Clause
//! numbers are edition-specific; see `docs/IEC_CITATIONS.md`.
//!
//! # Memory layout
//!
//! Every POU is compiled **once** and runs against whichever block of memory
//! belongs to the instance, exactly as a controller keeps one copy of the code
//! and one data block per instance. Slot references inside a body are offsets
//! from the instance base; only globals are absolute.
//!
//! Nothing is allocated at run time. A function block instance declared inside
//! a program lives inside that program's frame, and a function has one static
//! frame because IEC 61131-3 does not permit a POU to invoke itself — the
//! checker rejects recursion, and that rejection is what makes this sound.
//!
//! # Slot names
//!
//! Every slot gets a dotted name — `Main.Delay.ET` — built while the layout is
//! computed. It costs nothing and it is what makes a watch list, a trace and a
//! force list possible without a second symbol table.

use std::collections::BTreeMap;

use salman_core::diag::{DiagCode, Diagnostic, Diagnostics};
use salman_core::span::Span;
use salman_core::time::Duration;
use salman_core::value::{ElementaryType, Value};
use salman_lang::address::DirectAddress;
use salman_lang::ast::{
    Arg, BinaryOp, CaseLabel, CompilationUnit, Expr, ExprKind, Pou, PouKind, Stmt, StmtKind,
    UnaryOp, VarSection,
};
use salman_lang::dialect::Dialect;
use salman_lang::sema::{Checked, CheckedTrigger, Resolution};
use salman_lang::stdlib;
use salman_lang::types::{BoolWidening, TypeData, TypeId, common_type};

use crate::bytecode::{BinOp, Op, Program, Routine, UnOp};
use crate::memory::{ImageLayout, Memory, Persistence, SlotId};
use crate::task::{ProgramBinding, TaskConfig, TaskTrigger};

/// A construct the compiler does not implement yet.
pub const U_NOT_COMPILED: DiagCode = DiagCode("U0301");
/// Something the compiler could not lay out, such as a type of unknown size.
pub const E_LAYOUT: DiagCode = DiagCode("E0501");
/// A compiled unit that has nothing to run.
pub const E_NOTHING_TO_RUN: DiagCode = DiagCode("E0502");

/// How large the process image is, in bytes, for each of `%I`, `%Q` and `%M`.
///
/// Fixed at 0.0.1. A real controller sizes its image from its IO configuration,
/// which salman will have when the IO mapping layer arrives; until then a fixed
/// area is honest and an address past the end is a clear diagnostic rather than
/// a silent wrong answer.
pub const IMAGE_BYTES: usize = 4096;

/// A program compiled and ready to run.
#[derive(Debug, Clone)]
pub struct Compiled {
    /// The bytecode.
    pub program: Program,
    /// Memory sized and initialised for it.
    pub memory: Memory,
    /// The tasks to schedule.
    pub tasks: Vec<TaskConfig>,
}

/// Compiles a checked unit.
///
/// Returns `None` when nothing could be produced; the diagnostics say why.
/// A unit whose semantic analysis reported errors is not compiled at all —
/// generating code from a program known to be wrong produces confusing faults
/// instead of the errors the engineer needs to read.
#[must_use]
pub fn compile(
    unit: &CompilationUnit,
    checked: &Checked,
    dialect: &Dialect,
) -> (Option<Compiled>, Diagnostics) {
    let mut compiler = Compiler {
        unit,
        checked,
        widening: if dialect.bool_widens_to_bit_strings {
            BoolWidening::Permitted
        } else {
            BoolWidening::Refused
        },
        diags: Diagnostics::new(),
        constants: Vec::new(),
        messages: Vec::new(),
        enum_sets: Vec::new(),
        addresses: Vec::new(),
        layouts: Vec::new(),
        slot_types: Vec::new(),
        slot_names: Vec::new(),
        persistence: Vec::new(),
        initial: Vec::new(),
        global_offsets: Vec::new(),
        function_bases: BTreeMap::new(),
        routines: BTreeMap::new(),
    };
    let compiled = compiler.run();
    (compiled, compiler.diags)
}

/// Where every symbol of one POU sits, relative to its instance base.
#[derive(Debug, Clone, Default)]
struct PouLayout {
    offsets: Vec<u32>,
    size: u32,
    /// First temporary slot; temporaries follow the declared symbols.
    temp_base: u32,
    /// The result slot of a function, which follows the temporaries.
    result: Option<u32>,
}

struct Compiler<'a> {
    unit: &'a CompilationUnit,
    checked: &'a Checked,
    widening: BoolWidening,
    diags: Diagnostics,
    constants: Vec<Value>,
    messages: Vec<String>,
    enum_sets: Vec<Box<[i64]>>,
    addresses: Vec<DirectAddress>,
    layouts: Vec<PouLayout>,
    slot_types: Vec<ElementaryType>,
    slot_names: Vec<String>,
    persistence: Vec<Persistence>,
    initial: Vec<(u32, Value)>,
    global_offsets: Vec<u32>,
    function_bases: BTreeMap<u32, u32>,
    routines: BTreeMap<u32, u32>,
}

impl Compiler<'_> {
    fn error(
        &mut self,
        code: DiagCode,
        span: Span,
        message: impl Into<String>,
        label: impl Into<String>,
    ) {
        self.diags
            .push(Diagnostic::error(code, message).with_primary(span, label));
    }

    fn run(&mut self) -> Option<Compiled> {
        let unsettled = self.compute_layouts();
        if !unsettled.is_empty() {
            // Every slot offset below would be a guess. Nothing is emitted from
            // a layout that did not settle, because a variable sharing another
            // one's storage is exactly the failure this compiler exists to
            // prevent.
            for index in unsettled {
                self.report_containment_cycle(index);
            }
            return None;
        }
        self.report_located_variables();
        self.report_external_variables();
        self.allocate_globals();
        self.allocate_functions();

        let mut routine_list: Vec<Routine> = Vec::new();
        for (index, pou) in self.checked.pous.iter().enumerate() {
            if pou.kind == PouKind::Function
                || pou.kind == PouKind::FunctionBlock
                || pou.kind == PouKind::Program
            {
                let routine_index = routine_list.len() as u32;
                self.routines.insert(index as u32, routine_index);
                routine_list.push(Routine {
                    name: pou.name.as_str().to_string(),
                    code: Vec::new(),
                    result_slot: self.layouts.get(index).and_then(|l| l.result).map(SlotId),
                    frame_size: self.layouts.get(index).map_or(0, |l| l.size),
                    max_stack: 0,
                });
            }
        }

        let tasks = self.allocate_program_instances();

        for (index, pou_ast) in self.unit.pous().enumerate() {
            let Some(routine_index) = self.routines.get(&(index as u32)).copied() else {
                continue;
            };
            let (code, max_stack) = self.compile_body(index as u32, pou_ast);
            if let Some(routine) = routine_list.get_mut(routine_index as usize) {
                routine.code = code;
                routine.max_stack = max_stack;
            }
        }

        if tasks.is_empty() {
            self.diags.push(
                Diagnostic::error(E_NOTHING_TO_RUN, "this project has nothing to run").with_note(
                    "salman runs PROGRAM instances. Declare a PROGRAM, or a CONFIGURATION that \
                 instantiates one",
                ),
            );
            return None;
        }

        let mut memory = Memory::new(&self.slot_types, IMAGE_BYTES, ImageLayout::default());
        for (slot, value) in &self.initial {
            memory.set_initial(SlotId(*slot), value.clone());
        }
        for (slot, persistence) in self.persistence.iter().enumerate() {
            if *persistence != Persistence::Volatile {
                memory.set_persistence(SlotId(slot as u32), *persistence);
            }
        }

        Some(Compiled {
            program: Program {
                routines: routine_list,
                constants: std::mem::take(&mut self.constants),
                messages: std::mem::take(&mut self.messages),
                enum_sets: std::mem::take(&mut self.enum_sets),
                addresses: std::mem::take(&mut self.addresses),
                slot_types: std::mem::take(&mut self.slot_types),
                slot_names: std::mem::take(&mut self.slot_names),
                image_bytes: IMAGE_BYTES,
            },
            memory,
            tasks,
        })
    }

    // -- layout ----------------------------------------------------------

    /// How many slots a type occupies.
    fn slot_size(&self, ty: TypeId, depth: u32) -> u32 {
        if depth > 16 {
            // A type cannot legitimately nest this deeply; the checker rejects
            // cycles, and this bound stops a malformed arena from looping.
            return 1;
        }
        match self.checked.types.get(ty) {
            TypeData::Array { element, dims } => {
                let count: u64 = dims.iter().filter_map(|d| d.len()).product();
                let each = u64::from(self.slot_size(*element, depth + 1));
                u32::try_from(count.saturating_mul(each)).unwrap_or(u32::MAX)
            }
            TypeData::Struct { fields, .. } => fields
                .iter()
                .map(|f| self.slot_size(f.ty, depth + 1))
                .fold(0u32, u32::saturating_add),
            TypeData::FunctionBlock {
                native: Some(block),
                ..
            } => stdlib::slot_count(*block),
            TypeData::FunctionBlock {
                pou: Some(index), ..
            } => self.layouts.get(*index as usize).map_or(1, |l| l.size),
            _ => 1,
        }
    }

    /// The elementary type a slot holds, for anything that occupies one slot.
    fn slot_type(&self, ty: TypeId) -> ElementaryType {
        self.checked
            .types
            .as_elementary(ty)
            .unwrap_or(ElementaryType::Dint)
    }

    /// Lays out every POU, and says whether the layout settled.
    ///
    /// A POU's size depends on the sizes of the function blocks it
    /// instantiates, and a POU may be written **above** the blocks it uses, so
    /// one pass in declaration order is not enough: a pass propagates a size
    /// only one level up the containment chain in that case. A fixed number of
    /// passes is therefore wrong — with three, a block nested three deep left
    /// its container's variables overlapping each other, silently — so this
    /// iterates to a fixpoint instead.
    ///
    /// Each pass settles at least one more level, so an acyclic unit converges
    /// in at most one pass per POU. A unit that has not converged by then holds
    /// a function block that contains itself, whose size is not finite; the
    /// indices returned are the POUs on that cycle.
    fn compute_layouts(&mut self) -> Vec<usize> {
        let count = self.checked.pous.len();
        self.layouts = vec![PouLayout::default(); count];
        for _ in 0..=count {
            let mut changed = false;
            for index in 0..count {
                let layout = self.layout_for(index);
                if let Some(slot) = self.layouts.get_mut(index) {
                    if slot.size != layout.size || slot.offsets != layout.offsets {
                        changed = true;
                    }
                    *slot = layout;
                }
            }
            if !changed {
                return Vec::new();
            }
        }
        (0..count)
            .filter(|index| self.contains_itself(*index))
            .collect()
    }

    /// Whether a POU holds an instance of itself, directly or through another
    /// block. Such a thing has no finite size and cannot be laid out.
    fn contains_itself(&self, index: usize) -> bool {
        let Ok(target) = u32::try_from(index) else {
            return false;
        };
        let mut seen: Vec<u32> = Vec::new();
        let mut pending: Vec<u32> = Vec::new();
        self.contained_pous(index, &mut pending);
        while let Some(next) = pending.pop() {
            if next == target {
                return true;
            }
            if seen.contains(&next) {
                continue;
            }
            seen.push(next);
            self.contained_pous(next as usize, &mut pending);
        }
        false
    }

    /// The POUs whose instances one POU's variables hold, at any depth within
    /// an array or a structure.
    fn contained_pous(&self, index: usize, out: &mut Vec<u32>) {
        let Some(pou) = self.checked.pous.get(index) else {
            return;
        };
        for symbol in &pou.symbols {
            self.instances_in(symbol.ty, 0, out);
        }
    }

    fn instances_in(&self, ty: TypeId, depth: u32, out: &mut Vec<u32>) {
        if depth > 16 {
            return;
        }
        match self.checked.types.get(ty) {
            TypeData::Array { element, .. } => self.instances_in(*element, depth + 1, out),
            TypeData::Struct { fields, .. } => {
                for field in fields {
                    self.instances_in(field.ty, depth + 1, out);
                }
            }
            TypeData::FunctionBlock { pou: Some(pou), .. } => out.push(*pou),
            _ => {}
        }
    }

    /// Reports a function block whose size depends on itself.
    fn report_containment_cycle(&mut self, index: usize) {
        let Some(pou) = self.checked.pous.get(index) else {
            return;
        };
        let name = pou.name.to_string();
        let span = pou.name.span;
        self.diags.push(
            Diagnostic::error(E_LAYOUT, format!("`{name}` holds an instance of itself"))
                .with_primary(span, "salman cannot work out how large this is")
                .with_note(
                    "A function block that contains an instance of itself, directly or through \
                 another block, has no finite size. salman lays every instance out once, at \
                 load, so there is nowhere for the inner one to live.",
                ),
        );
    }

    fn layout_for(&self, index: usize) -> PouLayout {
        let Some(pou) = self.checked.pous.get(index) else {
            return PouLayout::default();
        };
        let mut offsets = Vec::with_capacity(pou.symbols.len());
        let mut next = 0u32;
        for symbol in &pou.symbols {
            offsets.push(next);
            next = next.saturating_add(self.slot_size(symbol.ty, 0));
        }
        let temp_base = next;
        let temps = self
            .unit
            .pous()
            .nth(index)
            .map_or(0, |ast| count_loop_temps(&ast.body));
        next = next.saturating_add(temps);
        let result = if pou.kind == PouKind::Function && pou.return_type.is_some() {
            let slot = next;
            next = next.saturating_add(1);
            Some(slot)
        } else {
            None
        };
        PouLayout {
            offsets,
            size: next,
            temp_base,
            result,
        }
    }

    /// Records the slot types and dotted names for one instance.
    fn emit_slots(&mut self, prefix: &str, ty: TypeId, persistence: Persistence, depth: u32) {
        if depth > 16 {
            self.push_slot(prefix, ElementaryType::Dint, persistence);
            return;
        }
        match self.checked.types.get(ty).clone() {
            TypeData::Array { element, dims } => {
                let mut indices: Vec<i64> = dims.iter().map(|d| d.low).collect();
                let total: u64 = dims.iter().filter_map(|d| d.len()).product();
                for _ in 0..total {
                    let suffix: Vec<String> = indices.iter().map(i64::to_string).collect();
                    let name = format!("{prefix}[{}]", suffix.join(","));
                    self.emit_slots(&name, element, persistence, depth + 1);
                    // Increment the odometer, last dimension fastest.
                    for (position, dim) in dims.iter().enumerate().rev() {
                        let Some(value) = indices.get_mut(position) else {
                            continue;
                        };
                        *value += 1;
                        if *value <= dim.high {
                            break;
                        }
                        *value = dim.low;
                    }
                }
            }
            TypeData::Struct { fields, .. } => {
                for field in &fields {
                    let name = format!("{prefix}.{}", field.name);
                    self.emit_slots(&name, field.ty, persistence, depth + 1);
                }
            }
            TypeData::FunctionBlock {
                native: Some(block),
                ..
            } => {
                for field in stdlib::layout(block) {
                    self.push_slot(&format!("{prefix}.{}", field.name), field.ty, persistence);
                }
            }
            TypeData::FunctionBlock {
                pou: Some(index), ..
            } => {
                let symbols = self
                    .checked
                    .pous
                    .get(index as usize)
                    .map(|p| p.symbols.clone())
                    .unwrap_or_default();
                for symbol in &symbols {
                    let name = format!("{prefix}.{}", symbol.name.as_str());
                    let offset = self.slot_types.len() as u32;
                    // A RETAIN inside a function block retains for every
                    // instance of it. Taking the container's persistence and
                    // nothing else — which is what this used to do — quietly
                    // cleared a retained counter on the next warm restart.
                    let inner = match Self::persistence_of(symbol.qualifiers) {
                        Persistence::Volatile => persistence,
                        declared => declared,
                    };
                    self.emit_slots(&name, symbol.ty, inner, depth + 1);
                    // A declared initial value inside a function block belongs
                    // to every instance of it. Dropping it here — which is what
                    // this used to do — started `VAR Setpoint : REAL := 20.0;`
                    // at zero in every instance, and said nothing.
                    if let Some(init) = &symbol.init {
                        self.initial.push((offset, init.clone()));
                    }
                }
                let layout = self
                    .layouts
                    .get(index as usize)
                    .cloned()
                    .unwrap_or_default();
                let declared: u32 = symbols
                    .iter()
                    .map(|s| self.slot_size(s.ty, 0))
                    .fold(0u32, u32::saturating_add);
                for extra in declared..layout.size {
                    self.push_slot(
                        &format!("{prefix}.<temp{extra}>"),
                        ElementaryType::Dint,
                        Persistence::Volatile,
                    );
                }
            }
            other => {
                let elementary = match other {
                    TypeData::Elementary(t) => t,
                    TypeData::Subrange { base, .. } | TypeData::Enum { base, .. } => base,
                    TypeData::Str { wide, .. } => {
                        if wide {
                            ElementaryType::WString
                        } else {
                            ElementaryType::String
                        }
                    }
                    _ => ElementaryType::Dint,
                };
                let slot = self.slot_types.len() as u32;
                self.push_slot(prefix, elementary, persistence);
                // A subrange or an enumeration whose default is not the
                // elementary default has to say so here: `Memory` initialises a
                // slot from its elementary type, which knows nothing about the
                // declaration. A declared `:=` initialiser is pushed after this
                // by the caller and wins, because `set_initial` is applied in
                // order.
                if let Some(value) = self.declared_default(ty) {
                    self.initial.push((slot, value));
                }
            }
        }
    }

    /// The value a variable of `ty` starts at when its declaration gives none,
    /// where that is not simply the elementary type's default.
    ///
    /// salman policy: **a variable never starts at a value its own declaration
    /// excludes.** IEC 61131-3 gives every elementary type a default initial
    /// value, and gives a subrange no rule of its own — so `Level : INT
    /// (10..20);` with no initialiser would start at the `INT` default of 0,
    /// which its declared range excludes. A variable that violates its own
    /// declaration from the first scan is indefensible once the bound is
    /// enforced at all: reading it and writing it straight back would fault.
    ///
    /// So a subrange starts at the elementary default when its range holds it,
    /// and otherwise at whichever declared bound is nearer that default —
    /// `low` for a range wholly above it, `high` for one wholly below. That
    /// keeps the standard's value wherever the declaration permits it and
    /// changes as little as possible where it does not.
    ///
    /// An enumeration starts at its **first declared value**, which is both the
    /// widely documented rule and the only choice that is always a member of
    /// the set.
    fn declared_default(&self, ty: TypeId) -> Option<Value> {
        match self.checked.types.get(ty) {
            TypeData::Subrange { base, low, high } => {
                let default = base.default_value().as_i64().unwrap_or(0);
                let chosen = if default < *low {
                    *low
                } else if default > *high {
                    *high
                } else {
                    return None;
                };
                Some(integer_value(i128::from(chosen), *base))
            }
            TypeData::Enum { base, values, .. } => {
                let (_, first) = values.first()?;
                if *first == base.default_value().as_i64().unwrap_or(0) {
                    return None;
                }
                Some(integer_value(i128::from(*first), *base))
            }
            _ => None,
        }
    }

    /// The declared default of every slot a value of `ty` occupies, in slot
    /// order, mirroring the traversal [`Compiler::emit_slots`] uses.
    ///
    /// `None` where the elementary default is already what the declaration
    /// permits. A `FUNCTION`'s frame is re-initialised on every call and cannot
    /// read the `initial` table, which belongs to load time, so it needs this.
    fn declared_defaults(&self, ty: TypeId, depth: u32, out: &mut Vec<Option<Value>>) {
        if depth > 16 {
            out.push(None);
            return;
        }
        match self.checked.types.get(ty).clone() {
            TypeData::Array { element, dims } => {
                let total: u64 = dims.iter().filter_map(|d| d.len()).product();
                for _ in 0..total {
                    self.declared_defaults(element, depth + 1, out);
                }
            }
            TypeData::Struct { fields, .. } => {
                for field in &fields {
                    self.declared_defaults(field.ty, depth + 1, out);
                }
            }
            TypeData::FunctionBlock { .. } => {
                for _ in 0..self.slot_size(ty, depth) {
                    out.push(None);
                }
            }
            _ => out.push(self.declared_default(ty)),
        }
    }

    fn push_slot(&mut self, name: &str, ty: ElementaryType, persistence: Persistence) {
        self.slot_types.push(ty);
        self.slot_names.push(name.to_string());
        self.persistence.push(persistence);
    }

    fn persistence_of(qualifiers: salman_lang::ast::VarQualifiers) -> Persistence {
        use salman_lang::ast::Retention;
        if qualifiers.persistent {
            Persistence::Persistent
        } else if qualifiers.retention == Retention::Retain {
            Persistence::Retain
        } else {
            Persistence::Volatile
        }
    }

    /// Reports every `AT %...` located variable as not implemented.
    ///
    /// The lexer, parser and checker all understand `AT %IX0.0`; what does not
    /// exist yet is the IO mapping layer that would make such a variable
    /// **be** that bit of the process image. Giving it an ordinary slot instead
    /// would produce a variable that looks located, watches like a variable,
    /// and never changes when the input does — a quiet lie. salman says so.
    ///
    /// A directly represented variable used in an expression, `%IX0.0`, works
    /// today; it is the `AT` binding that does not.
    fn report_located_variables(&mut self) {
        let mut located: Vec<(Span, String)> = Vec::new();
        for symbol in &self.checked.globals {
            if let Some(address) = &symbol.address {
                located.push((symbol.name.span, address.to_string()));
            }
        }
        for pou in &self.checked.pous {
            for symbol in &pou.symbols {
                if let Some(address) = &symbol.address {
                    located.push((symbol.name.span, address.to_string()));
                }
            }
        }
        for (span, address) in located {
            self.diags.push(
                Diagnostic::error(
                    U_NOT_COMPILED,
                    format!("salman does not implement `AT {address}` yet"),
                )
                .with_primary(span, "the IO mapping layer is not in this version")
                .with_note(
                    "a directly represented variable used in an expression, such as                      `%IX0.0 := TRUE;`, does work. It is the AT binding that does not,                      because there is nothing yet to bind it to",
                )
                .with_note("see docs/ROADMAP.md; IO mapping arrives with the Modbus layer"),
            );
        }
    }

    /// Reports every `VAR_EXTERNAL` declaration as not implemented.
    ///
    /// A `VAR_EXTERNAL` is a name for a `VAR_GLOBAL` declared elsewhere, and
    /// nothing here binds it to one: it is given storage of its own, so a POU
    /// that wrote it wrote a private copy that no other POU could see, and a
    /// POU that read it read whatever it had last written rather than what the
    /// global holds. That is the same quiet lie as an unbound `AT`, and it is
    /// refused for the same reason.
    fn report_external_variables(&mut self) {
        let mut external: Vec<(Span, String)> = Vec::new();
        for pou in &self.checked.pous {
            for symbol in &pou.symbols {
                if symbol.section == VarSection::External {
                    external.push((symbol.name.span, symbol.name.to_string()));
                }
            }
        }
        for (span, name) in external {
            self.diags.push(
                Diagnostic::error(
                    U_NOT_COMPILED,
                    format!("salman does not implement `VAR_EXTERNAL {name}` yet"),
                )
                .with_primary(span, "there is nothing here that binds this to a global")
                .with_note(
                    "A VAR_GLOBAL is visible to every POU by name, so deleting the VAR_EXTERNAL \
                     block gives the behaviour this was asking for. Binding the two together \
                     needs one name to mean one slot everywhere it is written, including as the \
                     control variable of a FOR loop, which is more than salman does at 0.0.1",
                )
                .with_note("see docs/CONFORMANCE.md for what is and is not implemented"),
            );
        }
    }

    fn allocate_globals(&mut self) {
        let globals = self.checked.globals.clone();
        for symbol in &globals {
            let offset = self.slot_types.len() as u32;
            self.global_offsets.push(offset);
            let persistence = Self::persistence_of(symbol.qualifiers);
            self.emit_slots(symbol.name.as_str(), symbol.ty, persistence, 0);
            if let Some(init) = &symbol.init {
                self.initial.push((offset, init.clone()));
            }
        }
    }

    fn allocate_functions(&mut self) {
        for index in 0..self.checked.pous.len() {
            let Some(pou) = self.checked.pous.get(index) else {
                continue;
            };
            if pou.kind != PouKind::Function {
                continue;
            }
            let base = self.slot_types.len() as u32;
            self.function_bases.insert(index as u32, base);
            let name = pou.name.as_str().to_string();
            self.emit_instance_slots(index, &name);
        }
    }

    /// Lays out one instance of a POU, naming every slot under `prefix`.
    fn emit_instance_slots(&mut self, index: usize, prefix: &str) {
        let symbols = self
            .checked
            .pous
            .get(index)
            .map(|p| p.symbols.clone())
            .unwrap_or_default();
        let layout = self.layouts.get(index).cloned().unwrap_or_default();
        let start = self.slot_types.len() as u32;
        for symbol in &symbols {
            let persistence = Self::persistence_of(symbol.qualifiers);
            let name = format!("{prefix}.{}", symbol.name.as_str());
            let offset = self.slot_types.len() as u32;
            self.emit_slots(&name, symbol.ty, persistence, 0);
            if let Some(init) = &symbol.init {
                self.initial.push((offset, init.clone()));
            }
        }
        let used = self.slot_types.len() as u32 - start;
        for extra in used..layout.size {
            let name = if layout.result == Some(extra) {
                format!("{prefix}.<result>")
            } else {
                format!("{prefix}.<temp{extra}>")
            };
            let ty = if layout.result == Some(extra) {
                self.checked
                    .pous
                    .get(index)
                    .and_then(|p| p.return_type)
                    .map_or(ElementaryType::Dint, |t| self.slot_type(t))
            } else {
                ElementaryType::Lint
            };
            self.push_slot(&name, ty, Persistence::Volatile);
        }
    }

    fn allocate_program_instances(&mut self) -> Vec<TaskConfig> {
        let mut tasks = Vec::new();
        let configurations = self.checked.configurations.clone();

        if configurations.is_empty() {
            // No CONFIGURATION: every PROGRAM gets one freewheeling task, in
            // declaration order. This is what makes `salman run` useful on a
            // single file, and it is a salman convenience, not a standard rule.
            for index in 0..self.checked.pous.len() {
                let Some(pou) = self.checked.pous.get(index) else {
                    continue;
                };
                if pou.kind != PouKind::Program {
                    continue;
                }
                let name = pou.name.as_str().to_string();
                let base = self.slot_types.len() as u32;
                self.emit_instance_slots(index, &name);
                let Some(routine) = self.routines.get(&(index as u32)).copied() else {
                    continue;
                };
                tasks.push(TaskConfig {
                    name: format!("{name}_freewheeling"),
                    trigger: TaskTrigger::Freewheeling,
                    priority: u16::try_from(tasks.len()).unwrap_or(u16::MAX),
                    programs: vec![ProgramBinding { routine, base }],
                    execution_time: Duration::ZERO,
                });
            }
            return tasks;
        }

        for configuration in &configurations {
            for task in &configuration.tasks {
                let mut programs = Vec::new();
                for instance in &task.programs {
                    if let Some(binding) = self.instantiate(instance.pou, instance.name.as_str()) {
                        programs.push(binding);
                    }
                }
                let trigger = match task.trigger {
                    CheckedTrigger::Cyclic { interval_ns } => TaskTrigger::Cyclic {
                        interval: Duration::from_nanos(interval_ns),
                    },
                    CheckedTrigger::Event { global } => TaskTrigger::Event {
                        slot: SlotId(
                            self.global_offsets
                                .get(global as usize)
                                .copied()
                                .unwrap_or(0),
                        ),
                    },
                    CheckedTrigger::Freewheeling => TaskTrigger::Freewheeling,
                };
                tasks.push(TaskConfig {
                    name: task.name.as_str().to_string(),
                    trigger,
                    priority: task.priority,
                    programs,
                    execution_time: Duration::ZERO,
                });
            }
            for instance in &configuration.untasked {
                if let Some(binding) = self.instantiate(instance.pou, instance.name.as_str()) {
                    tasks.push(TaskConfig {
                        name: format!("{}_freewheeling", instance.name),
                        trigger: TaskTrigger::Freewheeling,
                        priority: u16::try_from(tasks.len()).unwrap_or(u16::MAX),
                        programs: vec![binding],
                        execution_time: Duration::ZERO,
                    });
                }
            }
        }
        tasks
    }

    fn instantiate(&mut self, pou: u32, name: &str) -> Option<ProgramBinding> {
        let base = self.slot_types.len() as u32;
        self.emit_instance_slots(pou as usize, name);
        let routine = self.routines.get(&pou).copied()?;
        Some(ProgramBinding { routine, base })
    }
}

/// Three temporary slots per `FOR` statement: its limit, its step, and the
/// candidate the increment produces before the loop decides whether it wants
/// it.
///
/// salman policy: `TO` and `BY` are evaluated exactly once, at loop entry.
/// IEC 61131-3 does not say whether they are re-evaluated, and no public source
/// settles it, so salman chose the reading that makes a loop's length knowable
/// when it starts and reserved the memory to implement it.
fn count_loop_temps(body: &[Stmt]) -> u32 {
    let mut total = 0u32;
    for statement in body {
        total = total.saturating_add(match &statement.kind {
            StmtKind::For { body, .. } => 3 + count_loop_temps(body),
            StmtKind::While { body, .. } | StmtKind::Repeat { body, .. } => count_loop_temps(body),
            StmtKind::If {
                branches,
                else_body,
            } => branches
                .iter()
                .map(|b| count_loop_temps(&b.body))
                .chain(else_body.iter().map(|b| count_loop_temps(b)))
                .fold(0u32, u32::saturating_add),
            // A CASE needs one temporary to hold its selector, which is
            // evaluated once and then compared against every label.
            StmtKind::Case {
                arms, else_body, ..
            } => arms
                .iter()
                .map(|a| count_loop_temps(&a.body))
                .chain(else_body.iter().map(|b| count_loop_temps(b)))
                .fold(1u32, u32::saturating_add),
            _ => 0,
        });
    }
    total
}

/// Where a value lives, once its address is known.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Place {
    /// At a fixed offset from the instance base.
    Local(u32),
    /// At an absolute slot.
    Global(u32),
    /// In the process image.
    Address(u32),
    /// In an array within the instance; the linear index is on the stack.
    LocalIndexed { base: u32, len: u32 },
    /// In a global array; the linear index is on the stack.
    GlobalIndexed { base: u32, len: u32 },
}

/// A statically addressable location, before any subscript.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Anchor {
    Local(u32),
    Global(u32),
}

/// One enclosing loop, and the jumps waiting to be told where it ends.
#[derive(Debug, Default)]
struct LoopFrame {
    /// Indices of `EXIT` jumps to patch to the loop's end.
    exits: Vec<usize>,
    /// Indices of `CONTINUE` jumps to patch to the loop's next iteration.
    continues: Vec<usize>,
}

impl Compiler<'_> {
    fn compile_body(&mut self, pou: u32, ast: &Pou) -> (Vec<Op>, u32) {
        let temp_base = self.layouts.get(pou as usize).map_or(0, |l| l.temp_base);
        let mut body = Body {
            compiler: self,
            pou,
            code: Vec::new(),
            depth: 0,
            max_depth: 0,
            loops: Vec::new(),
            next_temp: temp_base,
            function_name: if ast.kind == PouKind::Function {
                Some(ast.name.as_str().to_string())
            } else {
                None
            },
        };
        if ast.kind == PouKind::Function {
            body.function_prologue(pou);
        }
        body.statements(&ast.body);
        body.emit(Op::Return);
        (body.code, body.max_depth)
    }

    /// The slot offset of a field within the type that owns it.
    ///
    /// `Resolution::Member` carries a field *position*; a field wider than one
    /// slot means position and offset differ, so this converts.
    fn field_slot_offset(&self, owner: TypeId, position: u32) -> Option<u32> {
        match self.checked.types.get(owner) {
            TypeData::FunctionBlock {
                native: Some(_), ..
            } => Some(position),
            TypeData::FunctionBlock {
                pou: Some(index), ..
            } => {
                let symbols = &self.checked.pous.get(*index as usize)?.symbols;
                let mut offset = 0u32;
                for symbol in symbols.iter().take(position as usize) {
                    offset = offset.saturating_add(self.slot_size(symbol.ty, 0));
                }
                Some(offset)
            }
            TypeData::Struct { fields, .. } => {
                let mut offset = 0u32;
                for field in fields.iter().take(position as usize) {
                    offset = offset.saturating_add(self.slot_size(field.ty, 0));
                }
                Some(offset)
            }
            _ => None,
        }
    }

    /// Interns a name used in a runtime message.
    fn message(&mut self, text: &str) -> u32 {
        if let Some(index) = self.messages.iter().position(|existing| existing == text) {
            return index as u32;
        }
        self.messages.push(text.to_string());
        (self.messages.len() - 1) as u32
    }

    /// Interns the permitted values of one enumeration.
    fn enum_set(&mut self, values: Box<[i64]>) -> u32 {
        if let Some(index) = self
            .enum_sets
            .iter()
            .position(|existing| *existing == values)
        {
            return index as u32;
        }
        self.enum_sets.push(values);
        (self.enum_sets.len() - 1) as u32
    }

    fn constant(&mut self, value: Value) -> u32 {
        if let Some(index) = self
            .constants
            .iter()
            .position(|existing| *existing == value)
        {
            return index as u32;
        }
        self.constants.push(value);
        (self.constants.len() - 1) as u32
    }

    fn address(&mut self, address: &DirectAddress) -> u32 {
        if let Some(index) = self
            .addresses
            .iter()
            .position(|existing| existing == address)
        {
            return index as u32;
        }
        self.addresses.push(address.clone());
        (self.addresses.len() - 1) as u32
    }
}

/// Code generation for one POU body.
struct Body<'c, 'a> {
    compiler: &'c mut Compiler<'a>,
    pou: u32,
    code: Vec<Op>,
    depth: i64,
    max_depth: u32,
    loops: Vec<LoopFrame>,
    next_temp: u32,
    function_name: Option<String>,
}

impl Body<'_, '_> {
    fn error(&mut self, span: Span, message: impl Into<String>, label: impl Into<String>) {
        self.compiler.error(E_LAYOUT, span, message, label);
    }

    fn unsupported(&mut self, span: Span, what: &str) {
        self.compiler.diags.push(
            Diagnostic::error(
                U_NOT_COMPILED,
                format!("salman does not compile {what} yet"),
            )
            .with_primary(span, "not implemented in this version")
            .with_note("see docs/CONFORMANCE.md for what is and is not implemented"),
        );
    }

    fn emit(&mut self, op: Op) {
        self.depth += stack_delta(op);
        if self.depth > i64::from(self.max_depth) {
            self.max_depth = u32::try_from(self.depth).unwrap_or(u32::MAX);
        }
        self.code.push(op);
    }

    fn here(&self) -> u32 {
        u32::try_from(self.code.len()).unwrap_or(u32::MAX)
    }

    /// Emits a jump whose target is not known yet, returning its index.
    fn emit_patch(&mut self, op: Op) -> usize {
        let index = self.code.len();
        self.emit(op);
        index
    }

    fn patch_to_here(&mut self, index: usize) {
        let target = self.here();
        self.patch(index, target);
    }

    fn patch(&mut self, index: usize, target: u32) {
        if let Some(op) = self.code.get_mut(index) {
            *op = match *op {
                Op::Jump(_) => Op::Jump(target),
                Op::JumpIfFalse(_) => Op::JumpIfFalse(target),
                Op::JumpIfTrue(_) => Op::JumpIfTrue(target),
                other => other,
            };
        }
    }

    fn temp(&mut self) -> u32 {
        let slot = self.next_temp;
        self.next_temp = self.next_temp.saturating_add(1);
        slot
    }

    fn local_offset(&self, symbol: u32) -> Option<u32> {
        self.compiler
            .layouts
            .get(self.pou as usize)?
            .offsets
            .get(symbol as usize)
            .copied()
    }

    fn elementary(&self, expr: &Expr) -> Option<ElementaryType> {
        let ty = self.compiler.checked.type_of(expr.id)?;
        self.compiler.checked.types.as_elementary(ty)
    }

    // -- expressions -----------------------------------------------------

    /// Emits everything that has to happen for the value on top of the stack to
    /// become a value of `target`: the conversion, and any constraint the
    /// target's declared type carries.
    ///
    /// **This is the single place a value becomes a value of a declared type.**
    /// Every site that stores one goes through here, which is the point: a
    /// subrange bound and a string length are promises made in a declaration,
    /// and a promise enforced at some assignment sites and not others is worse
    /// than one enforced nowhere, because the gap is invisible.
    ///
    /// `name` is the declared name of what is being written, for the runtime
    /// message. It is the variable's own name rather than an instance path,
    /// because the same check runs for every instance of a function block.
    fn coerce(&mut self, target: Option<TypeId>, value: Option<ElementaryType>, name: &str) {
        let Some(target) = target else {
            return;
        };
        // The conversion comes **first**, so the bound is checked against the
        // value that is actually stored rather than against the one the source
        // expression produced. That ordering is only sound because every
        // implicit conversion salman performs is value-preserving: IEC
        // 61131-3:2013 Figure 12 "Supported implicit type conversions" (Ed 3.0)
        // is a widening graph, and the checker refuses a narrowing assignment
        // by name (`DINT` into `SINT (0..100)` is `E0401`, not a truncation to
        // 44 that happens to land inside the range). If salman ever gains an
        // implicit conversion that can change a value, this order stops being
        // defensible and the check has to move in front of it.
        let want = self.compiler.checked.types.as_elementary(target);
        if let (Some(want), Some(have)) = (want, value)
            && want != have
        {
            self.emit(Op::Convert { to: want });
        }
        match self.compiler.checked.types.get(target).clone() {
            // `Level : INT (0..100);` is a promise about what the variable can
            // hold. The checker refuses `Level := 200;` because it can see the
            // constant; only this makes the same 200 through a variable fail.
            TypeData::Subrange { low, high, .. } => {
                let name = self.compiler.message(name);
                self.emit(Op::CheckRange { low, high, name });
            }
            // An enumeration is a base type and a set of legal values, which is
            // a subrange in all but name, and it gets the same treatment for
            // the same reason. It is not a range check: the values need not be
            // contiguous, and `Colour : (Red := 0, Blue := 2)` must refuse 1.
            TypeData::Enum { values, .. } => {
                let permitted: Box<[i64]> = values.iter().map(|(_, value)| *value).collect();
                let set = self.compiler.enum_set(permitted);
                let name = self.compiler.message(name);
                self.emit(Op::CheckEnum { set, name });
            }
            // `STRING[4]` means at most four characters. IEC 61131-3 gives the
            // target the leading characters that fit rather than refusing the
            // assignment, so this truncates rather than faulting.
            TypeData::Str { max_len, .. } => {
                self.emit(Op::TruncateString { max: max_len });
            }
            _ => {}
        }
    }

    /// The declared name of an assignment target, for a runtime message.
    fn target_name(expr: &Expr) -> String {
        match &expr.kind {
            ExprKind::Var(name) => name.as_str().to_string(),
            ExprKind::Paren(inner) => Self::target_name(inner),
            ExprKind::Member { base, field } => {
                format!("{}.{}", Self::target_name(base), field.as_str())
            }
            ExprKind::Index { base, .. } => format!("{}[..]", Self::target_name(base)),
            ExprKind::Direct(address) => address.to_string(),
            _ => "a variable".to_string(),
        }
    }

    fn expr(&mut self, expr: &Expr) {
        match &expr.kind {
            ExprKind::Literal(literal) => {
                let ty = self.elementary(expr).unwrap_or(ElementaryType::Dint);
                match literal_value(literal, ty) {
                    Some(value) => {
                        let index = self.compiler.constant(value);
                        self.emit(Op::Const(index));
                    }
                    None => self.error(
                        expr.span,
                        "this literal has no runtime representation",
                        "salman could not turn this literal into a value",
                    ),
                }
            }
            ExprKind::Direct(address) => {
                let index = self.compiler.address(address);
                self.emit(Op::LoadAddress(index));
            }
            ExprKind::Paren(inner) => self.expr(inner),
            ExprKind::Var(_) | ExprKind::Member { .. } | ExprKind::Index { .. } => {
                // An enumeration value written without its type — `Green`
                // rather than `Colour#Green` — is a name in the tree and a
                // constant at run time. The checker resolves it from the type
                // the context wants; without this it reached `place`, which has
                // no address to give for a constant.
                if self.enum_constant(expr) {
                    return;
                }
                match self.place(expr) {
                    Some(Place::Local(offset)) => self.emit(Op::LoadLocal(offset)),
                    Some(Place::Global(slot)) => self.emit(Op::LoadSlot(slot)),
                    Some(Place::Address(index)) => self.emit(Op::LoadAddress(index)),
                    Some(Place::LocalIndexed { base, len }) => {
                        self.emit(Op::LoadIndexedLocal { base, len, low: 0 });
                    }
                    Some(Place::GlobalIndexed { base, len }) => {
                        self.emit(Op::LoadIndexed { base, len, low: 0 });
                    }
                    None => self.error(
                        expr.span,
                        "this expression has no address salman can compute",
                        "salman could not resolve this to a variable",
                    ),
                }
            }
            ExprKind::Unary { op, operand } => {
                self.expr(operand);
                let ty = self.elementary(expr).unwrap_or(ElementaryType::Dint);
                let operand_ty = self.elementary(operand).unwrap_or(ty);
                if operand_ty != ty {
                    self.emit(Op::Convert { to: ty });
                }
                if let Some(op) = map_unary(*op) {
                    self.emit(Op::Unary { op, ty });
                }
            }
            ExprKind::Binary { op, lhs, rhs } => self.binary(expr, *op, lhs, rhs),
            ExprKind::Call { .. } => {
                self.call(expr, true);
            }
            ExprKind::EnumValue { .. } => {
                if !self.enum_constant(expr) {
                    self.error(
                        expr.span,
                        "this enumeration value was not resolved",
                        "salman could not find this value",
                    );
                }
            }
            ExprKind::Deref(_) => self.unsupported(expr.span, "references"),
            ExprKind::Error => {}
        }
    }

    /// Emits an enumeration value as a constant, if that is what this is.
    ///
    /// Returns whether anything was emitted, so that a name which is an
    /// ordinary variable falls through to the address path.
    fn enum_constant(&mut self, expr: &Expr) -> bool {
        let Some(Resolution::EnumValue { value, .. }) = self.compiler.checked.resolution(expr.id)
        else {
            return false;
        };
        let ty = self.elementary(expr).unwrap_or(ElementaryType::Dint);
        let literal = integer_value(i128::from(value), ty);
        let index = self.compiler.constant(literal);
        self.emit(Op::Const(index));
        true
    }

    fn binary(&mut self, whole: &Expr, op: BinaryOp, lhs: &Expr, rhs: &Expr) {
        if op == BinaryOp::Pow {
            self.unsupported(
                whole.span,
                "exponentiation (`**`), because salman implements no transcendental functions \
                 in this version",
            );
            return;
        }
        let left = self.elementary(lhs).unwrap_or(ElementaryType::Dint);
        let right = self.elementary(rhs).unwrap_or(left);
        let widening = self.compiler.widening;
        let operand_ty = if is_duration(left) || is_duration(right) {
            // Duration arithmetic keeps the duration's own type; the interpreter
            // reads the other operand as a plain integer factor.
            if is_duration(left) { left } else { right }
        } else {
            common_type(left, right, widening).unwrap_or(left)
        };

        self.expr(lhs);
        if left != operand_ty && !is_duration(operand_ty) {
            self.emit(Op::Convert { to: operand_ty });
        }
        self.expr(rhs);
        if right != operand_ty && !is_duration(operand_ty) {
            self.emit(Op::Convert { to: operand_ty });
        }
        self.emit(Op::Binary {
            op: map_binary(op),
            ty: operand_ty,
        });
    }

    /// Resolves an expression to a place, emitting any subscript it needs.
    fn place(&mut self, expr: &Expr) -> Option<Place> {
        match &expr.kind {
            ExprKind::Paren(inner) => self.place(inner),
            ExprKind::Direct(address) => Some(Place::Address(self.compiler.address(address))),
            ExprKind::Var(name) => {
                // Inside a FUNCTION, the function's own name is its result.
                if let Some(function) = &self.function_name
                    && name.ident.eq_str(function)
                    && let Some(result) = self
                        .compiler
                        .layouts
                        .get(self.pou as usize)
                        .and_then(|l| l.result)
                {
                    return Some(Place::Local(result));
                }
                match self.compiler.checked.resolution(expr.id)? {
                    Resolution::Local { symbol, .. } => {
                        Some(Place::Local(self.local_offset(symbol)?))
                    }
                    Resolution::Global { symbol } => Some(Place::Global(
                        self.compiler.global_offsets.get(symbol as usize).copied()?,
                    )),
                    _ => None,
                }
            }
            ExprKind::Member { base, .. } => {
                let anchor = self.anchor(base)?;
                let Resolution::Member { owner, offset } =
                    self.compiler.checked.resolution(expr.id)?
                else {
                    return None;
                };
                let field = self.compiler.field_slot_offset(owner, offset)?;
                Some(match anchor {
                    Anchor::Local(base) => Place::Local(base.saturating_add(field)),
                    Anchor::Global(base) => Place::Global(base.saturating_add(field)),
                })
            }
            ExprKind::Index { base, indices } => {
                let anchor = self.anchor(base)?;
                let ty = self.compiler.checked.type_of(base.id)?;
                let TypeData::Array { element, dims } = self.compiler.checked.types.get(ty).clone()
                else {
                    return None;
                };
                if self.compiler.slot_size(element, 0) != 1 {
                    self.unsupported(
                        expr.span,
                        "subscripting an array whose elements are structures or function block \
                         instances",
                    );
                    return None;
                }
                if dims.len() != indices.len() {
                    return None;
                }
                let total: u64 = dims.iter().filter_map(|d| d.len()).product();
                let len = u32::try_from(total).unwrap_or(u32::MAX);

                // lin = 0; for each dimension: lin = lin * len_d + (index - low_d)
                let zero = self.compiler.constant(Value::Dint(0));
                self.emit(Op::Const(zero));
                for (dimension, index) in dims.iter().zip(indices) {
                    let stride = i64::try_from(dimension.len().unwrap_or(1)).unwrap_or(1);
                    let stride_const = self.compiler.constant(Value::Lint(stride));
                    self.emit(Op::Const(stride_const));
                    self.emit(Op::Binary {
                        op: BinOp::Mul,
                        ty: ElementaryType::Lint,
                    });

                    self.expr(index);
                    let index_ty = self.elementary(index).unwrap_or(ElementaryType::Dint);
                    if index_ty != ElementaryType::Lint {
                        self.emit(Op::Convert {
                            to: ElementaryType::Lint,
                        });
                    }
                    // Each dimension is checked on its own. Checking only the
                    // folded linear index would let an out-of-range subscript
                    // in one dimension alias into another, which is how a
                    // program ends up reading somebody else's tag.
                    self.emit(Op::BoundsCheck {
                        low: dimension.low,
                        high: dimension.high,
                    });
                    let low_const = self.compiler.constant(Value::Lint(dimension.low));
                    self.emit(Op::Const(low_const));
                    self.emit(Op::Binary {
                        op: BinOp::Sub,
                        ty: ElementaryType::Lint,
                    });
                    self.emit(Op::Binary {
                        op: BinOp::Add,
                        ty: ElementaryType::Lint,
                    });
                }
                Some(match anchor {
                    Anchor::Local(base) => Place::LocalIndexed { base, len },
                    Anchor::Global(base) => Place::GlobalIndexed { base, len },
                })
            }
            _ => None,
        }
    }

    /// The statically known base of a variable, member or nested member.
    fn anchor(&mut self, expr: &Expr) -> Option<Anchor> {
        match self.place(expr)? {
            Place::Local(offset) => Some(Anchor::Local(offset)),
            Place::Global(slot) => Some(Anchor::Global(slot)),
            _ => None,
        }
    }

    fn store(&mut self, place: Place) {
        match place {
            Place::Local(offset) => self.emit(Op::StoreLocal(offset)),
            Place::Global(slot) => self.emit(Op::StoreSlot(slot)),
            Place::Address(index) => self.emit(Op::StoreAddress(index)),
            Place::LocalIndexed { base, len } => {
                self.emit(Op::StoreIndexedLocal { base, len, low: 0 });
            }
            Place::GlobalIndexed { base, len } => {
                self.emit(Op::StoreIndexed { base, len, low: 0 });
            }
        }
    }

    fn load(&mut self, place: Place) {
        match place {
            Place::Local(offset) => self.emit(Op::LoadLocal(offset)),
            Place::Global(slot) => self.emit(Op::LoadSlot(slot)),
            Place::Address(index) => self.emit(Op::LoadAddress(index)),
            Place::LocalIndexed { base, len } => {
                self.emit(Op::LoadIndexedLocal { base, len, low: 0 });
            }
            Place::GlobalIndexed { base, len } => {
                self.emit(Op::LoadIndexed { base, len, low: 0 });
            }
        }
    }

    // -- functions have no memory ----------------------------------------

    /// Re-initialises a `FUNCTION`'s own variables, on every call.
    ///
    /// A function has one static frame, because salman rejects recursion — but
    /// a frame that is never cleared is a frame that remembers. IEC 61131-3
    /// gives a function no state between invocations, and salman's own
    /// diagnostic for an unbound parameter says so in as many words, so a
    /// `VAR` that carried its last value into the next call was salman
    /// contradicting itself: the same call with the same arguments answered
    /// differently on the second scan.
    ///
    /// `VAR_INPUT` and `VAR_IN_OUT` are left alone: the caller has just written
    /// them. Everything else — `VAR`, `VAR_TEMP` and the result — starts from
    /// its declared initial value, or from its type's default when it has none.
    fn function_prologue(&mut self, pou: u32) {
        let Some(base) = self.compiler.function_bases.get(&pou).copied() else {
            return;
        };
        let symbols = self
            .compiler
            .checked
            .pous
            .get(pou as usize)
            .map(|p| p.symbols.clone())
            .unwrap_or_default();
        let layout = self
            .compiler
            .layouts
            .get(pou as usize)
            .cloned()
            .unwrap_or_default();
        for (position, symbol) in symbols.iter().enumerate() {
            if !matches!(symbol.section, VarSection::Local | VarSection::Temp) {
                continue;
            }
            let Some(offset) = layout.offsets.get(position).copied() else {
                continue;
            };
            let width = self.compiler.slot_size(symbol.ty, 0);
            let mut declared_defaults = Vec::new();
            self.compiler
                .declared_defaults(symbol.ty, 0, &mut declared_defaults);
            for step in 0..width {
                let slot = offset.saturating_add(step);
                let declared = if step == 0 { symbol.init.clone() } else { None };
                // A subrange or an enumeration whose declaration excludes the
                // elementary default gets the declaration's own default, so a
                // function's local starts where its type permits on every call
                // rather than at a value the next read of it would fault on.
                let value = declared
                    .or_else(|| declared_defaults.get(step as usize).cloned().flatten())
                    .unwrap_or_else(|| self.default_of(base, slot));
                let index = self.compiler.constant(value);
                self.emit(Op::Const(index));
                self.emit(Op::StoreLocal(slot));
            }
        }
        if let Some(result) = layout.result {
            let return_type = self
                .compiler
                .checked
                .pous
                .get(pou as usize)
                .and_then(|p| p.return_type);
            let value = return_type
                .and_then(|ty| self.compiler.declared_default(ty))
                .unwrap_or_else(|| self.default_of(base, result));
            let index = self.compiler.constant(value);
            self.emit(Op::Const(index));
            self.emit(Op::StoreLocal(result));
        }
    }

    /// The initial value of one slot of a static frame.
    fn default_of(&self, base: u32, offset: u32) -> Value {
        self.compiler
            .slot_types
            .get(base.saturating_add(offset) as usize)
            .copied()
            .unwrap_or(ElementaryType::Dint)
            .default_value()
    }

    // -- wide values -----------------------------------------------------

    /// How many slots an expression's value occupies.
    ///
    /// One for everything elementary; more for a structure, an array or a
    /// function block instance, which are the values that cannot travel on the
    /// operand stack.
    fn width(&self, expr: &Expr) -> u32 {
        self.compiler
            .checked
            .type_of(expr.id)
            .map_or(1, |ty| self.compiler.slot_size(ty, 0))
    }

    /// Copies a whole structure, array or instance from one place to another.
    ///
    /// Both sides must be statically addressable. The two types are identical —
    /// the checker permits an aggregate assignment only between one type and
    /// itself, and types are interned by structure — so slot `k` of the source
    /// is slot `k` of the target.
    ///
    /// # Why this does not call [`Body::coerce`], and why that is sound
    ///
    /// This is the one place a value is stored into a declared destination
    /// without going through `coerce`, so it owes an argument. Slot `k` of both
    /// sides has the **same declared type**, so there is no conversion to
    /// perform; and slot `k` of the source already satisfies that type's
    /// constraints, because the only two ways a slot acquires a value are its
    /// initial value — which
    /// [`Compiler::declared_default`] keeps inside the declaration — and a
    /// scalar store, which does go through `coerce`. Re-checking here would
    /// therefore cost a check per slot of every structure copy to discover
    /// something already true.
    ///
    /// That argument is an induction, and it breaks the moment either premise
    /// does. `an_aggregate_copy_cannot_carry_a_value_its_element_type_excludes`
    /// in `crates/salman-cli/tests/constraints.rs` is what holds it up.
    fn copy_wide(&mut self, destination: Anchor, source: Anchor, width: u32) {
        for offset in 0..width {
            self.load(anchor_place(source, offset));
            self.store(anchor_place(destination, offset));
        }
    }

    /// Copies a whole aggregate out of whatever `source` names.
    fn copy_wide_from(&mut self, destination: Anchor, source: &Expr, width: u32, what: &str) {
        let Some(anchor) = self.anchor(source) else {
            self.unsupported(
                source.span,
                &format!("{what} from something that is not a variable, a field or a global"),
            );
            return;
        };
        self.copy_wide(destination, anchor, width);
    }

    // -- calls -----------------------------------------------------------

    /// The `EN` and `ENO` arguments of a call, if it has them.
    ///
    /// IEC 61131-3:2013 Table 18 "Execution control graphically using EN and
    /// ENO" (Ed 3.0): `EN` decides whether the call happens at all, and `ENO`
    /// reports whether it did. Neither is declared by the POU being called, so
    /// they are pulled out of the argument list before anything else looks at
    /// it.
    fn execution_control(args: &[Arg]) -> (Option<&Expr>, Option<&Expr>) {
        let mut enable = None;
        let mut enable_out = None;
        for arg in args {
            match arg {
                Arg::Input { name, value } if name.ident.eq_str("EN") => enable = Some(value),
                Arg::Output { name, target } if name.ident.eq_str("ENO") => {
                    enable_out = target.as_ref();
                }
                _ => {}
            }
        }
        (enable, enable_out)
    }

    /// Whether an argument is one of the execution-control parameters, which
    /// the call body must not try to bind as an ordinary one.
    fn is_execution_control(arg: &Arg) -> bool {
        match arg {
            Arg::Input { name, .. } | Arg::Output { name, .. } => {
                name.ident.eq_str("EN") || name.ident.eq_str("ENO")
            }
            Arg::Positional(_) => false,
        }
    }

    /// Compiles a call, honouring `EN` and `ENO`. `want_value` is true when
    /// the result is used.
    ///
    /// With `EN` present the whole call — binding its inputs included — sits
    /// inside a conditional, because a call that does not happen does not
    /// write its inputs either.
    fn call(&mut self, expr: &Expr, want_value: bool) {
        let ExprKind::Call { args, .. } = &expr.kind else {
            return;
        };
        let (enable, enable_out) = Self::execution_control(args);

        let Some(enable) = enable else {
            self.call_body(expr, want_value);
            // Without EN the call always happens, so ENO is simply true.
            if let Some(target) = enable_out {
                self.set_bool(target, true);
            }
            return;
        };

        if want_value {
            // A function's result is the value of the expression. With EN false
            // there is no call and therefore no result, and salman will not
            // invent one.
            self.unsupported(
                expr.span,
                "`EN` on a call whose result is used; there would be no value when the call \
                 does not happen. Call it as a statement and read the result separately",
            );
            return;
        }

        self.expr(enable);
        let skip = self.emit_patch(Op::JumpIfFalse(0));
        self.call_body(expr, want_value);
        if let Some(target) = enable_out {
            self.set_bool(target, true);
        }
        let end = self.emit_patch(Op::Jump(0));
        self.patch_to_here(skip);
        if let Some(target) = enable_out {
            self.set_bool(target, false);
        }
        self.patch_to_here(end);
    }

    /// Stores a boolean constant into a place.
    fn set_bool(&mut self, target: &Expr, value: bool) {
        let Some(place) = self.place(target) else {
            self.error(
                target.span,
                "this cannot be assigned to",
                "salman could not resolve this to a variable",
            );
            return;
        };
        let index = self.compiler.constant(Value::Bool(value));
        self.emit(Op::Const(index));
        self.store(place);
    }

    fn call_body(&mut self, expr: &Expr, want_value: bool) {
        let ExprKind::Call { callee, args } = &expr.kind else {
            return;
        };

        // A function block instance: the callee is a variable of a function
        // block type, and the arguments name its inputs and outputs.
        if let Some(ty) = self.compiler.checked.type_of(callee.id)
            && let TypeData::FunctionBlock { native, pou, .. } =
                self.compiler.checked.types.get(ty).clone()
        {
            let Some(anchor) = self.anchor(callee) else {
                self.error(
                    callee.span,
                    "this function block instance has no address",
                    "salman could not resolve this instance",
                );
                return;
            };
            // Each records the slot the value comes out of, the elementary
            // type it comes out **as**, and where it goes. The source type is
            // the block's, not the caller's: taking it from the destination —
            // which is what this used to do — meant `coerce` compared a type
            // with itself, saw no difference, and emitted no conversion, so an
            // `INT` output bound into a `DINT` variable left an `INT` value in
            // a `DINT` slot.
            let mut outputs: Vec<(u32, Option<ElementaryType>, &Expr)> = Vec::new();
            // A VAR_IN_OUT is written at the call site like an input; salman
            // passes it by value and copies it back after the call, which is
            // what these record.
            let mut in_outs: Vec<(u32, Option<ElementaryType>, &Expr)> = Vec::new();
            for arg in args {
                // EN and ENO are the calling convention's, not the block's;
                // `call` has already dealt with them.
                if Self::is_execution_control(arg) {
                    continue;
                }
                match arg {
                    Arg::Input { name, value } => {
                        let Some(offset) = self.field_offset(ty, name.as_str()) else {
                            continue;
                        };
                        let field = self
                            .field_type_id(ty, name.as_str())
                            .and_then(|t| self.compiler.checked.types.as_elementary(t));
                        let width = self.width(value);
                        if width > 1 {
                            self.copy_wide_from(
                                shift(anchor, offset),
                                value,
                                width,
                                "passing a whole structure or array to a function block",
                            );
                        } else {
                            self.expr(value);
                            let declared = self.field_type_id(ty, name.as_str());
                            let source = self.elementary(value);
                            let label = name.as_str().to_string();
                            self.coerce(declared, source, &label);
                            self.store(anchor_place(anchor, offset));
                        }
                        if self.field_section(ty, name.as_str()) == Some(VarSection::InOut) {
                            in_outs.push((offset, field, value));
                        }
                    }
                    Arg::Output { name, target } => {
                        let Some(offset) = self.field_offset(ty, name.as_str()) else {
                            continue;
                        };
                        let field = self
                            .field_type_id(ty, name.as_str())
                            .and_then(|t| self.compiler.checked.types.as_elementary(t));
                        if let Some(target) = target {
                            outputs.push((offset, field, target));
                        }
                    }
                    Arg::Positional(value) => {
                        // IEC 61131-3:2013 Table 42 "Function block call" offers
                        // no positional form. The checker reports this; here we
                        // simply have nowhere to put the value.
                        self.unsupported(
                            value.span,
                            "positional arguments to a function block instance",
                        );
                    }
                }
            }

            match (native, pou) {
                (Some(block), _) => match anchor {
                    Anchor::Local(offset) => {
                        self.emit(Op::CallNativeLocal { block, offset });
                    }
                    Anchor::Global(base) => self.emit(Op::CallNative { block, base }),
                },
                (None, Some(index)) => {
                    let Some(routine) = self.compiler.routines.get(&index).copied() else {
                        return;
                    };
                    match anchor {
                        Anchor::Local(offset) => self.emit(Op::CallLocal { routine, offset }),
                        Anchor::Global(base) => self.emit(Op::Call { routine, base }),
                    }
                }
                (None, None) => self.error(
                    callee.span,
                    "this function block type was not resolved",
                    "salman could not find this function block",
                ),
            }

            for (offset, source, target) in outputs.into_iter().chain(in_outs) {
                let width = self.width(target);
                if width > 1 {
                    let Some(destination) = self.anchor(target) else {
                        self.unsupported(
                            target.span,
                            "binding a whole structure or array out of a function block through a \
                             subscript or a direct address",
                        );
                        continue;
                    };
                    self.copy_wide(destination, shift(anchor, offset), width);
                    continue;
                }
                let Some(place) = self.place(target) else {
                    continue;
                };
                self.load(anchor_place(anchor, offset));
                // The value is coming *out* of the block into a variable the
                // caller declared, so it is the caller's declaration whose
                // constraints apply — and the block's field whose type the
                // value currently has.
                let declared = self.compiler.checked.type_of(target.id);
                let name = Self::target_name(target);
                self.coerce(declared, source, &name);
                self.store(place);
            }

            if want_value {
                self.error(
                    expr.span,
                    "a function block call produces no value",
                    "read an output with dotted notation instead, as in `Timer.Q`",
                );
            }
            return;
        }

        // A function.
        let Some(Resolution::Function { pou }) = self.compiler.checked.resolution(callee.id) else {
            self.error(
                callee.span,
                "this is not something salman can call",
                "not callable",
            );
            return;
        };
        let Some(base) = self.compiler.function_bases.get(&pou).copied() else {
            return;
        };
        let Some(routine) = self.compiler.routines.get(&pou).copied() else {
            return;
        };
        // The parameters a positional call fills, in declaration order. This
        // must be the same list the checker counted its arguments against —
        // which includes VAR_IN_OUT — or an argument written for one parameter
        // lands in another, or in nothing at all.
        // The declared TypeId, not the flattened elementary type: flattening
        // is exactly what loses a subrange bound or a string length.
        let inputs: Vec<(u32, TypeId, VarSection, String)> = self
            .compiler
            .checked
            .pous
            .get(pou as usize)
            .map(|p| {
                p.symbols
                    .iter()
                    .enumerate()
                    .filter(|(_, s)| matches!(s.section, VarSection::Input | VarSection::InOut))
                    .filter_map(|(i, s)| {
                        let offset = self.compiler.layouts.get(pou as usize)?.offsets.get(i)?;
                        Some((*offset, s.ty, s.section, s.name.as_str().to_string()))
                    })
                    .collect()
            })
            .unwrap_or_default();

        // The slot the value comes back out of, the elementary type the
        // parameter holds it as, and the caller's variable it goes into.
        let mut in_outs: Vec<(u32, Option<ElementaryType>, &Expr)> = Vec::new();
        let mut positional = 0usize;
        for arg in args {
            // EN and ENO are the calling convention's, not the function's.
            if Self::is_execution_control(arg) {
                continue;
            }
            match arg {
                Arg::Positional(value) => {
                    let Some((offset, ty, section, name)) = inputs.get(positional).cloned() else {
                        positional += 1;
                        continue;
                    };
                    positional += 1;
                    self.pass_to_function(base, offset, ty, &name, value);
                    if section == VarSection::InOut {
                        in_outs.push((
                            offset,
                            self.compiler.checked.types.as_elementary(ty),
                            value,
                        ));
                    }
                }
                Arg::Input { name, value } => {
                    let Some(offset) = self.function_input_offset(pou, name.as_str()) else {
                        continue;
                    };
                    let Some(ty) = self.function_input_type_id(pou, name.as_str()) else {
                        continue;
                    };
                    let label = name.as_str().to_string();
                    self.pass_to_function(base, offset, ty, &label, value);
                    if self.function_input_section(pou, name.as_str()) == Some(VarSection::InOut) {
                        in_outs.push((
                            offset,
                            self.compiler.checked.types.as_elementary(ty),
                            value,
                        ));
                    }
                }
                Arg::Output { name, .. } => {
                    self.unsupported(
                        name.span,
                        "binding an output of a FUNCTION; only function blocks have outputs",
                    );
                }
            }
        }

        self.emit(Op::Call { routine, base });

        // A VAR_IN_OUT is copied back into whatever the caller named.
        for (offset, source, target) in in_outs {
            let width = self.width(target);
            let frame = Anchor::Global(base.saturating_add(offset));
            if width > 1 {
                let Some(destination) = self.anchor(target) else {
                    self.unsupported(
                        target.span,
                        "copying a whole structure or array back out of a VAR_IN_OUT parameter",
                    );
                    continue;
                };
                self.copy_wide(destination, frame, width);
                continue;
            }
            let Some(place) = self.place(target) else {
                continue;
            };
            self.load(anchor_place(frame, 0));
            let declared = self.compiler.checked.type_of(target.id);
            let name = Self::target_name(target);
            self.coerce(declared, source, &name);
            self.store(place);
        }

        if want_value {
            let result = self
                .compiler
                .layouts
                .get(pou as usize)
                .and_then(|l| l.result)
                .unwrap_or(0);
            self.emit(Op::LoadSlot(base.saturating_add(result)));
        }
    }

    /// Stores one argument into a function's static frame.
    fn pass_to_function(&mut self, base: u32, offset: u32, ty: TypeId, name: &str, value: &Expr) {
        let width = self.width(value);
        if width > 1 {
            self.copy_wide_from(
                Anchor::Global(base.saturating_add(offset)),
                value,
                width,
                "passing a whole structure or array to a FUNCTION",
            );
            return;
        }
        self.expr(value);
        let source = self.elementary(value);
        self.coerce(Some(ty), source, name);
        self.emit(Op::StoreSlot(base.saturating_add(offset)));
    }

    fn field_offset(&self, ty: TypeId, name: &str) -> Option<u32> {
        match self.compiler.checked.types.get(ty) {
            TypeData::FunctionBlock {
                native: Some(block),
                ..
            } => stdlib::field_offset(*block, name),
            TypeData::FunctionBlock {
                pou: Some(index), ..
            } => {
                let pou = self.compiler.checked.pous.get(*index as usize)?;
                let (position, _) = pou.symbol(name)?;
                self.compiler.field_slot_offset(ty, position)
            }
            _ => None,
        }
    }

    /// The **declared** type of a function block field, keeping any subrange or
    /// string length the declaration carried.
    ///
    /// `field_type` flattens to an elementary type, which is what the
    /// interpreter's instructions need and is exactly what loses the constraint.
    fn field_type_id(&self, ty: TypeId, name: &str) -> Option<TypeId> {
        match self.compiler.checked.types.get(ty) {
            TypeData::FunctionBlock {
                native: Some(block),
                ..
            } => {
                let field = stdlib::layout(*block)
                    .iter()
                    .find(|f| f.name.eq_ignore_ascii_case(name))?;
                Some(self.compiler.checked.types.elementary(field.ty))
            }
            TypeData::FunctionBlock {
                pou: Some(index), ..
            } => {
                let pou = self.compiler.checked.pous.get(*index as usize)?;
                let (_, symbol) = pou.symbol(name)?;
                Some(symbol.ty)
            }
            _ => None,
        }
    }

    /// Which section a named parameter of a function block instance is in.
    ///
    /// A standard block has no `VAR_IN_OUT`, so one is never a native block's.
    fn field_section(&self, ty: TypeId, name: &str) -> Option<VarSection> {
        match self.compiler.checked.types.get(ty) {
            TypeData::FunctionBlock {
                pou: Some(index), ..
            } => {
                let pou = self.compiler.checked.pous.get(*index as usize)?;
                let (_, symbol) = pou.symbol(name)?;
                Some(symbol.section)
            }
            _ => None,
        }
    }

    fn function_input_section(&self, pou: u32, name: &str) -> Option<VarSection> {
        let symbols = self.compiler.checked.pous.get(pou as usize)?;
        let (_, symbol) = symbols.symbol(name)?;
        Some(symbol.section)
    }

    fn function_input_offset(&self, pou: u32, name: &str) -> Option<u32> {
        let symbols = self.compiler.checked.pous.get(pou as usize)?;
        let (position, _) = symbols.symbol(name)?;
        self.compiler
            .layouts
            .get(pou as usize)?
            .offsets
            .get(position as usize)
            .copied()
    }

    /// The **declared** type of a function parameter, keeping any subrange or
    /// string length.
    fn function_input_type_id(&self, pou: u32, name: &str) -> Option<TypeId> {
        let symbols = self.compiler.checked.pous.get(pou as usize)?;
        let (_, symbol) = symbols.symbol(name)?;
        Some(symbol.ty)
    }

    // -- statements ------------------------------------------------------

    fn statements(&mut self, body: &[Stmt]) {
        for statement in body {
            self.statement(statement);
        }
    }

    fn statement(&mut self, statement: &Stmt) {
        match &statement.kind {
            StmtKind::Empty | StmtKind::Error => {}
            StmtKind::Assign { target, value } => {
                // A structure, an array or a function block instance is more
                // than one slot, and one load and one store would copy only its
                // first field and leave the rest of the target as it was. That
                // is a wrong answer nobody sees, so it is copied slot by slot.
                let width = self.width(target);
                if width > 1 {
                    let Some(destination) = self.anchor(target) else {
                        self.unsupported(
                            target.span,
                            "assigning a whole structure, array or function block instance \
                             through a subscript or a direct address",
                        );
                        return;
                    };
                    self.copy_wide_from(
                        destination,
                        value,
                        width,
                        "assigning a whole structure, array or function block instance",
                    );
                    return;
                }
                let Some(place) = self.place(target) else {
                    self.error(
                        target.span,
                        "this cannot be assigned to",
                        "salman could not resolve this to a variable",
                    );
                    return;
                };
                self.expr(value);
                let declared = self.compiler.checked.type_of(target.id);
                let have = self.elementary(value);
                let name = Self::target_name(target);
                self.coerce(declared, have, &name);
                self.store(place);
            }
            StmtKind::AssignAttempt { .. } => {
                self.unsupported(statement.span, "the assignment attempt `?=`");
            }
            StmtKind::Call(call) => self.call(call, false),
            StmtKind::If {
                branches,
                else_body,
            } => {
                let mut ends = Vec::new();
                for branch in branches {
                    self.expr(&branch.condition);
                    let skip = self.emit_patch(Op::JumpIfFalse(0));
                    self.statements(&branch.body);
                    ends.push(self.emit_patch(Op::Jump(0)));
                    self.patch_to_here(skip);
                }
                if let Some(body) = else_body {
                    self.statements(body);
                }
                for end in ends {
                    self.patch_to_here(end);
                }
            }
            StmtKind::Case {
                selector,
                arms,
                else_body,
            } => {
                let slot = self.temp();
                self.expr(selector);
                let ty = self.elementary(selector).unwrap_or(ElementaryType::Dint);
                self.emit(Op::StoreLocal(slot));

                let mut ends = Vec::new();
                for arm in arms {
                    let mut taken = Vec::new();
                    for label in &arm.labels {
                        match label {
                            CaseLabel::Single(value) => {
                                self.emit(Op::LoadLocal(slot));
                                self.expr(value);
                                self.emit(Op::Binary { op: BinOp::Eq, ty });
                                taken.push(self.emit_patch(Op::JumpIfTrue(0)));
                            }
                            CaseLabel::Range { low, high } => {
                                self.emit(Op::LoadLocal(slot));
                                self.expr(low);
                                self.emit(Op::Binary { op: BinOp::Ge, ty });
                                let below = self.emit_patch(Op::JumpIfFalse(0));
                                self.emit(Op::LoadLocal(slot));
                                self.expr(high);
                                self.emit(Op::Binary { op: BinOp::Le, ty });
                                taken.push(self.emit_patch(Op::JumpIfTrue(0)));
                                self.patch_to_here(below);
                            }
                        }
                    }
                    let skip = self.emit_patch(Op::Jump(0));
                    for jump in taken {
                        self.patch_to_here(jump);
                    }
                    self.statements(&arm.body);
                    ends.push(self.emit_patch(Op::Jump(0)));
                    self.patch_to_here(skip);
                }
                if let Some(body) = else_body {
                    self.statements(body);
                }
                for end in ends {
                    self.patch_to_here(end);
                }
            }
            StmtKind::While { condition, body } => {
                let top = self.here();
                self.expr(condition);
                let exit = self.emit_patch(Op::JumpIfFalse(0));
                self.loops.push(LoopFrame::default());
                self.statements(body);
                let frame = self.loops.pop().unwrap_or_default();
                for jump in frame.continues {
                    self.patch(jump, top);
                }
                self.emit(Op::Jump(top));
                self.patch_to_here(exit);
                for jump in frame.exits {
                    self.patch_to_here(jump);
                }
            }
            StmtKind::Repeat { body, until } => {
                let top = self.here();
                self.loops.push(LoopFrame::default());
                self.statements(body);
                let frame = self.loops.pop().unwrap_or_default();
                let test = self.here();
                for jump in frame.continues {
                    self.patch(jump, test);
                }
                self.expr(until);
                self.emit(Op::JumpIfFalse(top));
                for jump in frame.exits {
                    self.patch_to_here(jump);
                }
            }
            StmtKind::For {
                variable,
                from,
                to,
                by,
                body,
            } => {
                self.for_loop(statement, variable, from, to, by.as_ref(), body);
            }
            StmtKind::Exit => match self.loops.last_mut() {
                Some(_) => {
                    let jump = self.emit_patch(Op::Jump(0));
                    if let Some(frame) = self.loops.last_mut() {
                        frame.exits.push(jump);
                    }
                }
                None => self.error(
                    statement.span,
                    "EXIT outside a loop",
                    "there is no loop here",
                ),
            },
            StmtKind::Continue => match self.loops.last_mut() {
                Some(_) => {
                    let jump = self.emit_patch(Op::Jump(0));
                    if let Some(frame) = self.loops.last_mut() {
                        frame.continues.push(jump);
                    }
                }
                None => {
                    self.error(
                        statement.span,
                        "CONTINUE outside a loop",
                        "there is no loop here",
                    );
                }
            },
            StmtKind::Return => self.emit(Op::Return),
        }
    }

    /// `FOR v := a TO b BY c DO ... END_FOR`.
    ///
    /// salman policy: `TO` and `BY` are evaluated **once**, at loop entry, into
    /// temporaries. IEC 61131-3 does not say whether they are re-evaluated, and
    /// no public source settles it; salman chose the reading that makes a
    /// loop's length knowable when it starts.
    ///
    /// The direction test is dynamic, because the step may be a variable: the
    /// loop continues while `(step >= 0 AND v <= limit) OR (step < 0 AND
    /// v >= limit)`.
    fn for_loop(
        &mut self,
        statement: &Stmt,
        variable: &salman_lang::ast::Name,
        from: &Expr,
        to: &Expr,
        by: Option<&Expr>,
        body: &[Stmt],
    ) {
        let Some((symbol, _)) = self
            .compiler
            .checked
            .pous
            .get(self.pou as usize)
            .and_then(|p| p.symbol(variable.as_str()))
        else {
            self.error(
                variable.span,
                "the control variable is not declared",
                "salman could not find this variable",
            );
            return;
        };
        let Some(control) = self.local_offset(symbol) else {
            return;
        };
        let ty = self
            .compiler
            .checked
            .pous
            .get(self.pou as usize)
            .and_then(|p| p.symbol(variable.as_str()))
            .map_or(ElementaryType::Dint, |(_, s)| self.compiler.slot_type(s.ty));

        let limit = self.temp();
        let step = self.temp();
        // Where the next value goes before anything decides whether the loop
        // wants it. See the loop shape below.
        let next = self.temp();

        // The control variable's own declaration governs its initial value.
        // The limit, the step and the candidate are salman's temporaries, of
        // the control variable's base type, and carry no constraint of their
        // own.
        let declared = self
            .compiler
            .checked
            .pous
            .get(self.pou as usize)
            .and_then(|p| p.symbol(variable.as_str()))
            .map(|(_, s)| s.ty);
        let label = variable.as_str().to_string();

        self.expr(from);
        let source = self.elementary(from);
        self.coerce(declared, source, &label);
        self.emit(Op::StoreLocal(control));

        self.expr(to);
        if self.elementary(to) != Some(ty) {
            self.emit(Op::Convert { to: ty });
        }
        self.emit(Op::StoreLocal(limit));

        if let Some(expression) = by {
            self.expr(expression);
            if self.elementary(expression) != Some(ty) {
                self.emit(Op::Convert { to: ty });
            }
        } else {
            let one = self.compiler.constant(integer_value(1, ty));
            self.emit(Op::Const(one));
        }
        self.emit(Op::StoreLocal(step));

        // The test guards the value **before** it reaches the control
        // variable, which is why it is emitted twice: once on the initial
        // value and once on each candidate the increment produces.
        //
        //     control := from            (checked: the body will see it)
        //     if not wanted -> exit
        //   body:
        //     <body>
        //     next := control + step
        //     if not wanted -> exit      (the sentinel stops here)
        //     control := next            (checked: the body will see it)
        //     jump body
        //
        // Storing the incremented value first and checking it afterwards —
        // which is what this used to do — faulted on `FOR I := 0 TO 3` over
        // `I : INT (0..3)`: the value that ends the loop is one past its end
        // by construction, so an ordinary loop over exactly the range its
        // control variable declares could not run. Nothing reads that value,
        // and after this loop the control variable holds the last value the
        // body was given rather than one past it.
        let entry = self.emit_loop_test(control, limit, step, ty);
        let body_start = self.here();

        self.loops.push(LoopFrame::default());
        self.statements(body);
        let frame = self.loops.pop().unwrap_or_default();

        let increment = self.here();
        for jump in frame.continues {
            self.patch(jump, increment);
        }
        self.emit(Op::LoadLocal(control));
        self.emit(Op::LoadLocal(step));
        self.emit(Op::Binary { op: BinOp::Add, ty });
        self.emit(Op::StoreLocal(next));
        let stepped = self.emit_loop_test(next, limit, step, ty);
        // The increment writes the control variable, so its declared range
        // applies here too: a loop whose step carries it past the end of a
        // subrange it will still be used within is exactly what a bound exists
        // to catch.
        self.emit(Op::LoadLocal(next));
        self.coerce(declared, Some(ty), &label);
        self.emit(Op::StoreLocal(control));
        self.emit(Op::Jump(body_start));

        self.patch_to_here(entry);
        self.patch_to_here(stepped);
        for jump in frame.exits {
            self.patch_to_here(jump);
        }
        let _ = statement;
    }

    /// Emits `(step >= 0 AND candidate <= limit) OR (step < 0 AND candidate >=
    /// limit)` and a jump taken when the loop does **not** want `candidate`,
    /// returning that jump for patching.
    fn emit_loop_test(
        &mut self,
        candidate: u32,
        limit: u32,
        step: u32,
        ty: ElementaryType,
    ) -> usize {
        let zero = self.compiler.constant(integer_value(0, ty));
        self.emit(Op::LoadLocal(step));
        self.emit(Op::Const(zero));
        self.emit(Op::Binary { op: BinOp::Ge, ty });
        self.emit(Op::LoadLocal(candidate));
        self.emit(Op::LoadLocal(limit));
        self.emit(Op::Binary { op: BinOp::Le, ty });
        self.emit(Op::Binary {
            op: BinOp::And,
            ty: ElementaryType::Bool,
        });

        self.emit(Op::LoadLocal(step));
        self.emit(Op::Const(zero));
        self.emit(Op::Binary { op: BinOp::Lt, ty });
        self.emit(Op::LoadLocal(candidate));
        self.emit(Op::LoadLocal(limit));
        self.emit(Op::Binary { op: BinOp::Ge, ty });
        self.emit(Op::Binary {
            op: BinOp::And,
            ty: ElementaryType::Bool,
        });

        self.emit(Op::Binary {
            op: BinOp::Or,
            ty: ElementaryType::Bool,
        });
        self.emit_patch(Op::JumpIfFalse(0))
    }
}

/// The anchor `offset` slots further on, for reaching into an aggregate.
const fn shift(anchor: Anchor, offset: u32) -> Anchor {
    match anchor {
        Anchor::Local(base) => Anchor::Local(base.saturating_add(offset)),
        Anchor::Global(base) => Anchor::Global(base.saturating_add(offset)),
    }
}

fn anchor_place(anchor: Anchor, offset: u32) -> Place {
    match anchor {
        Anchor::Local(base) => Place::Local(base.saturating_add(offset)),
        Anchor::Global(base) => Place::Global(base.saturating_add(offset)),
    }
}

const fn is_duration(ty: ElementaryType) -> bool {
    matches!(ty, ElementaryType::Time | ElementaryType::LTime)
}

const fn map_binary(op: BinaryOp) -> BinOp {
    match op {
        BinaryOp::Pow => BinOp::Pow,
        BinaryOp::Mul => BinOp::Mul,
        BinaryOp::Div => BinOp::Div,
        BinaryOp::Mod => BinOp::Mod,
        BinaryOp::Add => BinOp::Add,
        BinaryOp::Sub => BinOp::Sub,
        BinaryOp::Lt => BinOp::Lt,
        BinaryOp::Gt => BinOp::Gt,
        BinaryOp::Le => BinOp::Le,
        BinaryOp::Ge => BinOp::Ge,
        BinaryOp::Eq => BinOp::Eq,
        BinaryOp::Ne => BinOp::Ne,
        BinaryOp::And => BinOp::And,
        BinaryOp::Xor => BinOp::Xor,
        BinaryOp::Or => BinOp::Or,
    }
}

/// The instruction a unary operator compiles to, if it needs one.
///
/// Unary plus is the identity: `+X` is `X`. It has no instruction, and giving
/// it the negation instruction — which is what this function used to do —
/// silently returned the wrong sign for every operand it was written on.
const fn map_unary(op: UnaryOp) -> Option<UnOp> {
    match op {
        UnaryOp::Plus => None,
        UnaryOp::Neg => Some(UnOp::Neg),
        UnaryOp::Not => Some(UnOp::Not),
    }
}

/// How much an instruction changes the operand stack depth.
const fn stack_delta(op: Op) -> i64 {
    match op {
        Op::Const(_) | Op::Dup | Op::LoadSlot(_) | Op::LoadLocal(_) | Op::LoadAddress(_) => 1,
        Op::Pop
        | Op::StoreSlot(_)
        | Op::StoreLocal(_)
        | Op::StoreAddress(_)
        | Op::Binary { .. }
        | Op::JumpIfFalse(_)
        | Op::JumpIfTrue(_) => -1,
        Op::StoreIndexed { .. } | Op::StoreIndexedLocal { .. } => -2,
        Op::LoadIndexed { .. }
        | Op::LoadIndexedLocal { .. }
        | Op::Unary { .. }
        | Op::Convert { .. }
        | Op::BoundsCheck { .. }
        | Op::CheckRange { .. }
        | Op::CheckEnum { .. }
        | Op::TruncateString { .. }
        | Op::Jump(_)
        | Op::Call { .. }
        | Op::CallLocal { .. }
        | Op::CallNative { .. }
        | Op::CallNativeLocal { .. }
        | Op::Return => 0,
    }
}

/// An integer constant of a given type.
fn integer_value(value: i128, ty: ElementaryType) -> Value {
    use ElementaryType as E;
    match ty {
        E::Sint => Value::Sint(value as i8),
        E::Int => Value::Int(value as i16),
        E::Lint => Value::Lint(value as i64),
        E::Usint => Value::Usint(value as u8),
        E::Uint => Value::Uint(value as u16),
        E::Udint => Value::Udint(value as u32),
        E::Ulint => Value::Ulint(value as u64),
        E::Byte => Value::Byte(value as u8),
        E::Word => Value::Word(value as u16),
        E::Dword => Value::Dword(value as u32),
        E::Lword => Value::Lword(value as u64),
        E::Bool => Value::Bool(value != 0),
        E::Real => Value::real(value as f32),
        E::Lreal => Value::lreal(value as f64),
        E::Time => Value::Time(Duration::from_nanos(value as i64)),
        E::LTime => Value::LTime(Duration::from_nanos(value as i64)),
        _ => Value::Dint(value as i32),
    }
}

/// The runtime value of a literal token, given the type the checker assigned.
fn literal_value(literal: &salman_lang::token::LiteralValue, ty: ElementaryType) -> Option<Value> {
    use salman_lang::token::LiteralValue as L;
    Some(match literal {
        L::Bool(value) => Value::Bool(*value),
        L::Int {
            magnitude,
            negative,
            ..
        } => {
            let magnitude = i128::try_from(*magnitude).ok()?;
            let value = if *negative { -magnitude } else { magnitude };
            match ty {
                ElementaryType::Real => Value::real(value as f32),
                ElementaryType::Lreal => Value::lreal(value as f64),
                other => integer_value(value, other),
            }
        }
        L::Real { value, .. } => {
            if ty == ElementaryType::Real {
                Value::real(*value as f32)
            } else {
                Value::lreal(*value)
            }
        }
        L::Duration { value, long } => {
            if *long || ty == ElementaryType::LTime {
                Value::LTime(*value)
            } else {
                Value::Time(*value)
            }
        }
        L::Date(date) => Value::Date(*date),
        L::TimeOfDay(tod) => Value::TimeOfDay(*tod),
        L::DateAndTime(dt) => Value::DateAndTime(*dt),
        L::String(bytes) => Value::string(bytes),
        L::WString(units) => Value::wstring(units),
    })
}
