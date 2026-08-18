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
        self.compute_layouts();
        self.report_located_variables();
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

    fn compute_layouts(&mut self) {
        // Function block layouts must exist before the POUs that instantiate
        // them are laid out. The checker has already rejected cycles, so a
        // fixed number of passes converges; two is enough for any acyclic
        // nesting salman accepts, and a third is cheap insurance.
        self.layouts = vec![PouLayout::default(); self.checked.pous.len()];
        for _ in 0..3 {
            for index in 0..self.checked.pous.len() {
                let layout = self.layout_for(index);
                if let Some(slot) = self.layouts.get_mut(index) {
                    *slot = layout;
                }
            }
        }
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
                    self.emit_slots(&name, symbol.ty, persistence, depth + 1);
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
                self.push_slot(prefix, elementary, persistence);
            }
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

/// Two temporary slots per `FOR` statement: its limit and its step.
///
/// salman policy: `TO` and `BY` are evaluated exactly once, at loop entry.
/// IEC 61131-3 does not say whether they are re-evaluated, and no public source
/// settles it, so salman chose the reading that makes a loop's length knowable
/// when it starts and reserved the memory to implement it.
fn count_loop_temps(body: &[Stmt]) -> u32 {
    let mut total = 0u32;
    for statement in body {
        total = total.saturating_add(match &statement.kind {
            StmtKind::For { body, .. } => 2 + count_loop_temps(body),
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
                self.emit(Op::Unary {
                    op: map_unary(*op),
                    ty,
                });
            }
            ExprKind::Binary { op, lhs, rhs } => self.binary(expr, *op, lhs, rhs),
            ExprKind::Call { .. } => {
                self.call(expr, true);
            }
            ExprKind::EnumValue { .. } => {
                let ty = self.elementary(expr).unwrap_or(ElementaryType::Dint);
                match self.compiler.checked.resolution(expr.id) {
                    Some(Resolution::EnumValue { value, .. }) => {
                        let literal = integer_value(i128::from(value), ty);
                        let index = self.compiler.constant(literal);
                        self.emit(Op::Const(index));
                    }
                    _ => self.error(
                        expr.span,
                        "this enumeration value was not resolved",
                        "salman could not find this value",
                    ),
                }
            }
            ExprKind::Deref(_) => self.unsupported(expr.span, "references"),
            ExprKind::Error => {}
        }
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

    // -- calls -----------------------------------------------------------

    /// Compiles a call. `want_value` is true when the result is used.
    fn call(&mut self, expr: &Expr, want_value: bool) {
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
            let mut outputs: Vec<(u32, &Expr)> = Vec::new();
            for arg in args {
                match arg {
                    Arg::Input { name, value } => {
                        let Some(offset) = self.field_offset(ty, name.as_str()) else {
                            continue;
                        };
                        self.expr(value);
                        let target = self.field_type(ty, name.as_str());
                        let source = self.elementary(value);
                        if let (Some(target), Some(source)) = (target, source)
                            && target != source
                        {
                            self.emit(Op::Convert { to: target });
                        }
                        self.store(anchor_place(anchor, offset));
                    }
                    Arg::Output { name, target } => {
                        let Some(offset) = self.field_offset(ty, name.as_str()) else {
                            continue;
                        };
                        if let Some(target) = target {
                            outputs.push((offset, target));
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

            for (offset, target) in outputs {
                let Some(place) = self.place(target) else {
                    continue;
                };
                self.load(anchor_place(anchor, offset));
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
        let inputs: Vec<(u32, ElementaryType)> = self
            .compiler
            .checked
            .pous
            .get(pou as usize)
            .map(|p| {
                p.symbols
                    .iter()
                    .enumerate()
                    .filter(|(_, s)| s.section == VarSection::Input)
                    .filter_map(|(i, s)| {
                        let offset = self.compiler.layouts.get(pou as usize)?.offsets.get(i)?;
                        Some((*offset, self.compiler.slot_type(s.ty)))
                    })
                    .collect()
            })
            .unwrap_or_default();

        let mut positional = 0usize;
        for arg in args {
            match arg {
                Arg::Positional(value) => {
                    let Some((offset, ty)) = inputs.get(positional).copied() else {
                        positional += 1;
                        continue;
                    };
                    positional += 1;
                    self.expr(value);
                    if self.elementary(value) != Some(ty) {
                        self.emit(Op::Convert { to: ty });
                    }
                    self.emit(Op::StoreSlot(base.saturating_add(offset)));
                }
                Arg::Input { name, value } => {
                    let Some(offset) = self.function_input_offset(pou, name.as_str()) else {
                        continue;
                    };
                    let ty = self.function_input_type(pou, name.as_str());
                    self.expr(value);
                    if let Some(ty) = ty
                        && self.elementary(value) != Some(ty)
                    {
                        self.emit(Op::Convert { to: ty });
                    }
                    self.emit(Op::StoreSlot(base.saturating_add(offset)));
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

    fn field_type(&self, ty: TypeId, name: &str) -> Option<ElementaryType> {
        match self.compiler.checked.types.get(ty) {
            TypeData::FunctionBlock {
                native: Some(block),
                ..
            } => stdlib::layout(*block)
                .iter()
                .find(|f| f.name.eq_ignore_ascii_case(name))
                .map(|f| f.ty),
            TypeData::FunctionBlock {
                pou: Some(index), ..
            } => {
                let pou = self.compiler.checked.pous.get(*index as usize)?;
                let (_, symbol) = pou.symbol(name)?;
                Some(self.compiler.slot_type(symbol.ty))
            }
            _ => None,
        }
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

    fn function_input_type(&self, pou: u32, name: &str) -> Option<ElementaryType> {
        let symbols = self.compiler.checked.pous.get(pou as usize)?;
        let (_, symbol) = symbols.symbol(name)?;
        Some(self.compiler.slot_type(symbol.ty))
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
                let Some(place) = self.place(target) else {
                    self.error(
                        target.span,
                        "this cannot be assigned to",
                        "salman could not resolve this to a variable",
                    );
                    return;
                };
                self.expr(value);
                let want = self.elementary(target);
                let have = self.elementary(value);
                if let (Some(want), Some(have)) = (want, have)
                    && want != have
                {
                    self.emit(Op::Convert { to: want });
                }
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

        self.expr(from);
        if self.elementary(from) != Some(ty) {
            self.emit(Op::Convert { to: ty });
        }
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

        let top = self.here();
        // (step >= 0 AND control <= limit) OR (step < 0 AND control >= limit)
        let zero = self.compiler.constant(integer_value(0, ty));
        self.emit(Op::LoadLocal(step));
        self.emit(Op::Const(zero));
        self.emit(Op::Binary { op: BinOp::Ge, ty });
        self.emit(Op::LoadLocal(control));
        self.emit(Op::LoadLocal(limit));
        self.emit(Op::Binary { op: BinOp::Le, ty });
        self.emit(Op::Binary {
            op: BinOp::And,
            ty: ElementaryType::Bool,
        });

        self.emit(Op::LoadLocal(step));
        self.emit(Op::Const(zero));
        self.emit(Op::Binary { op: BinOp::Lt, ty });
        self.emit(Op::LoadLocal(control));
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
        let exit = self.emit_patch(Op::JumpIfFalse(0));

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
        self.emit(Op::StoreLocal(control));
        self.emit(Op::Jump(top));

        self.patch_to_here(exit);
        for jump in frame.exits {
            self.patch_to_here(jump);
        }
        let _ = statement;
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

const fn map_unary(op: UnaryOp) -> UnOp {
    match op {
        UnaryOp::Neg | UnaryOp::Plus => UnOp::Neg,
        UnaryOp::Not => UnOp::Not,
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
