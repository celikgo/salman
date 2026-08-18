// SPDX-License-Identifier: Apache-2.0
//! The scan scheduler: `CONFIGURATION`, `RESOURCE`, `TASK`, `PROGRAM`.
//!
//! IEC 61131-3:2013 §6.8 "Configuration elements" and §6.8.2 "Tasks".
//!
//! # The scan
//!
//! One scan of one task is: **latch the inputs, run the programs bound to the
//! task, publish the outputs.** The process image is what makes that meaningful
//! and it lives in [`crate::memory`]; this module decides *when* a scan happens
//! and *in what order*.
//!
//! # What salman models, and what it does not
//!
//! * **Modelled**: cyclic tasks with a period and a priority, event tasks
//!   released by a rising edge, freewheeling tasks, execution order by
//!   priority, and overrun detection.
//! * **Not modelled: pre-emption.** A task runs to completion. Real
//!   controllers let a higher-priority task interrupt a lower-priority one part
//!   way through, and modelling that faithfully needs an execution-cost model
//!   salman does not have. A scan here is atomic, and the consequence is that
//!   salman cannot reproduce a race that depends on being interrupted mid-scan.
//!   That is a real limitation, it is stated here and in `docs/CONFORMANCE.md`,
//!   and it is not hidden behind the word "deterministic".
//! * **Modelled optionally**: how long a scan takes. Zero by default, so
//!   virtual time only advances between releases. Give a task an execution time
//!   and overruns become visible.
//!
//! # Priority
//!
//! A lower number is more urgent. This is the convention across the dialect
//! documentation salman consulted, but the governing clause could not be
//! verified from a public source, so it is recorded as a salman decision.
//!
//! # Determinism
//!
//! When two tasks are released at the same instant they run in priority order,
//! and ties are broken by declaration order. The tie-break exists so that the
//! answer never depends on how a collection happened to iterate.

use salman_core::time::Duration;

use crate::bytecode::Program;
use crate::clock::Clock;
use crate::exec::{ExecLimits, Fault, execute};
use crate::memory::{Memory, SlotId};
use crate::trace::{Sample, Signal, Trace};

/// What releases a task.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TaskTrigger {
    /// Released every `interval`. IEC's `INTERVAL`.
    Cyclic {
        /// The period.
        interval: Duration,
    },
    /// Released by a rising edge on a `BOOL`. IEC's `SINGLE`.
    Event {
        /// The variable watched for a rising edge.
        slot: SlotId,
    },
    /// Runs again as soon as it finishes.
    ///
    /// Modelled as a cyclic task whose period is its own execution time, which
    /// is what freewheeling means. A freewheeling task with a zero execution
    /// time would never let the clock advance, so it is given a modelled scan
    /// time; [`FREEWHEEL_DEFAULT_SCAN`] is used when none is stated.
    Freewheeling,
}

/// The modelled scan time of a freewheeling task that states none.
///
/// Not a measurement and not a claim about any controller: a number had to be
/// chosen for virtual time to advance at all, and one millisecond is the order
/// of magnitude of a small program's scan.
pub const FREEWHEEL_DEFAULT_SCAN: Duration = Duration::from_nanos(1_000_000);

/// One program instance bound to a task.
///
/// The base is the instance's first slot: a POU is compiled once and run
/// against whichever block of memory belongs to the instance, exactly as a
/// controller keeps one copy of the code and one data block per instance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProgramBinding {
    /// The compiled routine.
    pub routine: u32,
    /// The instance's first slot.
    pub base: u32,
}

/// One task and the programs bound to it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskConfig {
    /// Its name, as written in the configuration.
    pub name: String,
    /// What releases it.
    pub trigger: TaskTrigger,
    /// Lower is more urgent. See the module documentation.
    pub priority: u16,
    /// The program instances to run, in the order they were declared.
    pub programs: Vec<ProgramBinding>,
    /// How long a scan is modelled to take. Zero unless stated.
    pub execution_time: Duration,
}

impl TaskConfig {
    /// A cyclic task.
    #[must_use]
    pub fn cyclic(name: impl Into<String>, interval: Duration, priority: u16) -> Self {
        Self {
            name: name.into(),
            trigger: TaskTrigger::Cyclic { interval },
            priority,
            programs: Vec::new(),
            execution_time: Duration::ZERO,
        }
    }

    /// The period between releases, for a task that has one.
    #[must_use]
    pub fn period(&self) -> Option<Duration> {
        match &self.trigger {
            TaskTrigger::Cyclic { interval } => Some(*interval),
            TaskTrigger::Freewheeling => Some(if self.execution_time.nanos() > 0 {
                self.execution_time
            } else {
                FREEWHEEL_DEFAULT_SCAN
            }),
            TaskTrigger::Event { .. } => None,
        }
    }
}

/// How a task has behaved so far.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TaskStats {
    /// Scans completed.
    pub scans: u64,
    /// Scans that finished after the next release was already due.
    pub overruns: u64,
    /// Fewest instructions any scan used.
    pub min_instructions: u64,
    /// Most instructions any scan used.
    pub max_instructions: u64,
    /// Instructions across every scan, for the mean.
    pub total_instructions: u64,
}

impl TaskStats {
    /// Mean instructions per scan, or zero before the first scan.
    #[must_use]
    pub const fn mean_instructions(&self) -> u64 {
        if self.scans == 0 {
            0
        } else {
            self.total_instructions / self.scans
        }
    }
}

/// A fault that stopped a task, and when.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskFault {
    /// The task's index.
    pub task: u16,
    /// The scan it happened on.
    pub scan: u64,
    /// The simulation time.
    pub time: Duration,
    /// What went wrong.
    pub fault: Fault,
}

impl std::fmt::Display for TaskFault {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "scan {} at {}: {}",
            self.scan,
            self.time.to_iec_literal(),
            self.fault
        )
    }
}

/// Per-task scheduling state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TaskState {
    next_release_ns: i64,
    previous_trigger: bool,
    stopped: bool,
}

/// Why [`Runtime::step`] stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepOutcome {
    /// A task ran.
    Ran {
        /// Which one.
        task: u16,
    },
    /// Every task has stopped, so nothing will ever run again.
    Idle,
}

/// A running configuration.
#[derive(Debug, Clone)]
pub struct Runtime {
    program: Program,
    memory: Memory,
    clock: Clock,
    tasks: Vec<TaskConfig>,
    state: Vec<TaskState>,
    stats: Vec<TaskStats>,
    limits: ExecLimits,
    scan: u64,
    faults: Vec<TaskFault>,
    trace: Option<Trace>,
    seed: u64,
}

impl Runtime {
    /// Builds a runtime around a compiled program.
    #[must_use]
    pub fn new(program: Program, memory: Memory, clock: Clock, tasks: Vec<TaskConfig>) -> Self {
        let state = vec![
            TaskState {
                next_release_ns: 0,
                previous_trigger: false,
                stopped: false
            };
            tasks.len()
        ];
        let stats = vec![TaskStats::default(); tasks.len()];
        Self {
            program,
            memory,
            clock,
            tasks,
            state,
            stats,
            limits: ExecLimits::default(),
            scan: 0,
            faults: Vec::new(),
            trace: None,
            seed: 0,
        }
    }

    /// Sets the execution limits, including the scan watchdog.
    #[must_use]
    pub const fn with_limits(mut self, limits: ExecLimits) -> Self {
        self.limits = limits;
        self
    }

    /// Records the seed this run used, which goes into every trace.
    #[must_use]
    pub const fn with_seed(mut self, seed: u64) -> Self {
        self.seed = seed;
        self
    }

    /// Starts recording the named signals.
    pub fn record(&mut self, signals: Vec<Signal>) {
        self.trace = Some(Trace::new(
            signals,
            self.seed,
            self.clock.is_deterministic(),
        ));
    }

    /// The trace so far, if recording.
    #[must_use]
    pub const fn trace(&self) -> Option<&Trace> {
        self.trace.as_ref()
    }

    /// The program's memory, for inspecting and forcing variables.
    #[must_use]
    pub const fn memory(&self) -> &Memory {
        &self.memory
    }

    /// The mutable memory, for driving inputs and setting forces.
    pub const fn memory_mut(&mut self) -> &mut Memory {
        &mut self.memory
    }

    /// The compiled program.
    #[must_use]
    pub const fn program(&self) -> &Program {
        &self.program
    }

    /// The simulation clock.
    #[must_use]
    pub const fn clock(&self) -> &Clock {
        &self.clock
    }

    /// How many scans have run in total, across every task.
    #[must_use]
    pub const fn scan_count(&self) -> u64 {
        self.scan
    }

    /// Statistics for each task, in declaration order.
    #[must_use]
    pub fn stats(&self) -> &[TaskStats] {
        &self.stats
    }

    /// The tasks, in declaration order.
    #[must_use]
    pub fn tasks(&self) -> &[TaskConfig] {
        &self.tasks
    }

    /// Every fault so far.
    #[must_use]
    pub fn faults(&self) -> &[TaskFault] {
        &self.faults
    }

    /// Whether any task has faulted.
    #[must_use]
    pub fn has_faulted(&self) -> bool {
        !self.faults.is_empty()
    }

    /// Runs one task, advancing the clock to its release if necessary.
    ///
    /// Returns what happened. When every task has stopped — because each has
    /// faulted, or because there are none — this returns [`StepOutcome::Idle`]
    /// and the clock does not move.
    pub fn step(&mut self) -> StepOutcome {
        // An event task with a pending rising edge runs now, without advancing
        // the clock, because the edge has already happened.
        if let Some(index) = self.pending_event() {
            self.run_task(index);
            return StepOutcome::Ran { task: index };
        }

        let Some((index, release_ns)) = self.next_periodic_release() else {
            return StepOutcome::Idle;
        };
        self.clock.advance_to_ns(release_ns);
        self.run_task(index);
        StepOutcome::Ran { task: index }
    }

    /// Runs until the simulation clock reaches `deadline`.
    ///
    /// Stops early if every task has stopped. Returns how many scans ran.
    pub fn run_until(&mut self, deadline: Duration) -> u64 {
        let start = self.scan;
        loop {
            let Some((_, release_ns)) = self.next_periodic_release() else {
                if self.pending_event().is_none() {
                    break;
                }
                self.step();
                continue;
            };
            if release_ns > deadline.nanos() && self.pending_event().is_none() {
                break;
            }
            if self.step() == StepOutcome::Idle {
                break;
            }
        }
        // Leave the clock at the deadline so that a run of a stated length has
        // a stated end, whether or not a task was due at the very last instant.
        self.clock.advance_to_ns(deadline.nanos());
        self.scan - start
    }

    /// Runs exactly `scans` scans, or fewer if everything stops.
    pub fn run_scans(&mut self, scans: u64) -> u64 {
        let start = self.scan;
        for _ in 0..scans {
            if self.step() == StepOutcome::Idle {
                break;
            }
        }
        self.scan - start
    }

    /// The event task whose trigger has just risen, if any, in priority order.
    fn pending_event(&mut self) -> Option<u16> {
        let mut best: Option<(u16, u16)> = None;
        for (index, task) in self.tasks.iter().enumerate() {
            let TaskTrigger::Event { slot } = task.trigger else {
                continue;
            };
            let Some(state) = self.state.get_mut(index) else {
                continue;
            };
            if state.stopped {
                continue;
            }
            let level = self
                .memory
                .read_slot(slot)
                .and_then(salman_core::value::Value::as_bool)
                .unwrap_or(false);
            let rose = level && !state.previous_trigger;
            state.previous_trigger = level;
            if rose {
                let index = u16::try_from(index).unwrap_or(u16::MAX);
                if best.is_none_or(|(p, _)| task.priority < p) {
                    best = Some((task.priority, index));
                }
            }
        }
        best.map(|(_, index)| index)
    }

    /// The next periodic task to run, and when.
    ///
    /// Ties are broken by priority and then by declaration order, so the answer
    /// never depends on iteration order.
    fn next_periodic_release(&self) -> Option<(u16, i64)> {
        let mut best: Option<(i64, u16, u16)> = None;
        for (index, task) in self.tasks.iter().enumerate() {
            if task.period().is_none() {
                continue;
            }
            let Some(state) = self.state.get(index) else {
                continue;
            };
            if state.stopped {
                continue;
            }
            let index = u16::try_from(index).unwrap_or(u16::MAX);
            let key = (state.next_release_ns, task.priority, index);
            if best.is_none_or(|current| key < current) {
                best = Some(key);
            }
        }
        best.map(|(release, _, index)| (index, release))
    }

    /// One scan of one task: latch, execute, publish.
    fn run_task(&mut self, index: u16) {
        let Some(task) = self.tasks.get(index as usize).cloned() else {
            return;
        };

        self.memory.latch_inputs();

        let mut instructions = 0u64;
        let mut fault = None;
        for binding in &task.programs {
            match execute(
                &self.program,
                &mut self.memory,
                &self.clock,
                binding.routine,
                binding.base,
                self.limits,
            ) {
                Ok(done) => instructions += done.instructions,
                Err(e) => {
                    fault = Some(e);
                    break;
                }
            }
        }

        self.memory.publish_outputs();

        if task.execution_time.nanos() > 0 {
            self.clock.advance(task.execution_time);
        }

        self.scan += 1;
        let now = self.clock.elapsed();

        if let Some(stats) = self.stats.get_mut(index as usize) {
            stats.scans += 1;
            stats.total_instructions = stats.total_instructions.saturating_add(instructions);
            if stats.scans == 1 || instructions < stats.min_instructions {
                stats.min_instructions = instructions;
            }
            if instructions > stats.max_instructions {
                stats.max_instructions = instructions;
            }
        }

        if let Some(period) = task.period()
            && let Some(state) = self.state.get_mut(index as usize)
        {
            let next = state.next_release_ns.saturating_add(period.nanos().max(1));
            // A scan that finishes after its own next release has overrun. The
            // schedule is not rewound: the task simply runs again at once,
            // which is what a controller does.
            if next <= now.nanos()
                && let Some(stats) = self.stats.get_mut(index as usize)
            {
                stats.overruns += 1;
            }
            state.next_release_ns = next.max(now.nanos());
        }

        if let Some(fault) = fault {
            if let Some(state) = self.state.get_mut(index as usize) {
                state.stopped = true;
            }
            self.faults.push(TaskFault {
                task: index,
                scan: self.scan,
                time: now,
                fault,
            });
        }

        self.sample(index, now);
    }

    fn sample(&mut self, task: u16, time: Duration) {
        let Some(trace) = self.trace.as_mut() else {
            return;
        };
        let values = trace
            .signals
            .iter()
            .map(|signal| {
                self.memory
                    .read_slot(signal.slot)
                    .cloned()
                    .unwrap_or(salman_core::value::Value::Bool(false))
            })
            .collect();
        trace.push(Sample {
            scan: self.scan,
            time,
            task,
            values,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bytecode::{BinOp, Op, Routine};
    use crate::memory::ImageLayout;
    use salman_core::value::{ElementaryType, Value};

    fn ms(n: i64) -> Duration {
        Duration::from_nanos(n * 1_000_000)
    }

    /// A program with one routine that adds one to slot 0 every scan.
    fn counting_program() -> (Program, Memory) {
        let program = Program {
            routines: vec![Routine {
                name: "Counter".into(),
                code: vec![
                    Op::LoadSlot(0),
                    Op::Const(0),
                    Op::Binary {
                        op: BinOp::Add,
                        ty: ElementaryType::Dint,
                    },
                    Op::StoreSlot(0),
                    Op::Return,
                ],
                result_slot: None,
                frame_size: 0,
                max_stack: 2,
            }],
            constants: vec![Value::Dint(1)],
            slot_types: vec![ElementaryType::Dint, ElementaryType::Bool],
            slot_names: vec!["Count".into(), "Trigger".into()],
            ..Program::new()
        };
        let memory = Memory::new(&program.slot_types, 0, ImageLayout::default());
        (program, memory)
    }

    fn runtime(tasks: Vec<TaskConfig>) -> Runtime {
        let (program, memory) = counting_program();
        Runtime::new(program, memory, Clock::virtual_default(), tasks)
    }

    fn count(rt: &Runtime) -> i64 {
        rt.memory()
            .read_slot(SlotId(0))
            .and_then(Value::as_i64)
            .unwrap_or(-1)
    }

    #[test]
    fn a_cyclic_task_runs_once_per_period() {
        let mut task = TaskConfig::cyclic("Main", ms(10), 1);
        task.programs = vec![ProgramBinding {
            routine: 0,
            base: 0,
        }];
        let mut rt = runtime(vec![task]);
        rt.run_until(ms(100));
        // Releases at 0, 10, 20 ... 100 inclusive is eleven scans.
        assert_eq!(count(&rt), 11);
        assert_eq!(rt.clock().elapsed(), ms(100));
    }

    #[test]
    fn the_clock_lands_exactly_on_each_release() {
        let mut task = TaskConfig::cyclic("Main", ms(10), 1);
        task.programs = vec![ProgramBinding {
            routine: 0,
            base: 0,
        }];
        let mut rt = runtime(vec![task]);
        rt.step();
        assert_eq!(rt.clock().elapsed(), Duration::ZERO);
        rt.step();
        assert_eq!(rt.clock().elapsed(), ms(10));
        rt.step();
        assert_eq!(rt.clock().elapsed(), ms(20));
    }

    #[test]
    fn tasks_released_together_run_in_priority_order_lower_number_first() {
        let program = Program {
            routines: vec![
                Routine {
                    name: "Fast".into(),
                    code: vec![Op::Const(0), Op::StoreSlot(0), Op::Return],
                    result_slot: None,
                    frame_size: 0,
                    max_stack: 1,
                },
                Routine {
                    name: "Slow".into(),
                    code: vec![Op::Const(1), Op::StoreSlot(0), Op::Return],
                    result_slot: None,
                    frame_size: 0,
                    max_stack: 1,
                },
            ],
            constants: vec![Value::Dint(1), Value::Dint(2)],
            slot_types: vec![ElementaryType::Dint],
            slot_names: vec!["Winner".into()],
            ..Program::new()
        };
        let memory = Memory::new(&program.slot_types, 0, ImageLayout::default());
        let mut high = TaskConfig::cyclic("High", ms(10), 1);
        high.programs = vec![ProgramBinding {
            routine: 0,
            base: 0,
        }];
        let mut low = TaskConfig::cyclic("Low", ms(10), 9);
        low.programs = vec![ProgramBinding {
            routine: 1,
            base: 0,
        }];
        // Declared low first, so only priority can produce the right order.
        let mut rt = Runtime::new(program, memory, Clock::virtual_default(), vec![low, high]);
        rt.step();
        assert_eq!(
            rt.memory().read_slot(SlotId(0)).and_then(Value::as_i64),
            Some(1),
            "the higher-priority task did not run first"
        );
    }

    #[test]
    fn an_event_task_runs_on_a_rising_edge_and_not_otherwise() {
        let mut event = TaskConfig {
            name: "OnAlarm".into(),
            trigger: TaskTrigger::Event { slot: SlotId(1) },
            priority: 1,
            programs: vec![ProgramBinding {
                routine: 0,
                base: 0,
            }],
            execution_time: Duration::ZERO,
        };
        event.programs = vec![ProgramBinding {
            routine: 0,
            base: 0,
        }];
        let mut rt = runtime(vec![event]);

        assert_eq!(
            rt.step(),
            StepOutcome::Idle,
            "an event task ran with no edge"
        );
        rt.memory_mut().write_slot(SlotId(1), Value::Bool(true));
        assert_eq!(rt.step(), StepOutcome::Ran { task: 0 });
        assert_eq!(count(&rt), 1);
        assert_eq!(
            rt.step(),
            StepOutcome::Idle,
            "a held level released the task again"
        );
        rt.memory_mut().write_slot(SlotId(1), Value::Bool(false));
        assert_eq!(rt.step(), StepOutcome::Idle);
        rt.memory_mut().write_slot(SlotId(1), Value::Bool(true));
        assert_eq!(rt.step(), StepOutcome::Ran { task: 0 });
        assert_eq!(count(&rt), 2);
    }

    #[test]
    fn a_freewheeling_task_advances_the_clock_by_its_modelled_scan_time() {
        // A freewheeling task with no modelled cost would never let virtual
        // time move, and the run would never end.
        let task = TaskConfig {
            name: "Free".into(),
            trigger: TaskTrigger::Freewheeling,
            priority: 1,
            programs: vec![ProgramBinding {
                routine: 0,
                base: 0,
            }],
            execution_time: Duration::ZERO,
        };
        let mut rt = runtime(vec![task]);
        rt.run_scans(5);
        assert_eq!(count(&rt), 5);
        assert!(rt.clock().elapsed().nanos() > 0, "the clock never advanced");
    }

    #[test]
    fn a_scan_that_outlasts_its_period_is_counted_as_an_overrun() {
        let mut task = TaskConfig::cyclic("Main", ms(10), 1);
        task.programs = vec![ProgramBinding {
            routine: 0,
            base: 0,
        }];
        task.execution_time = ms(15);
        let mut rt = runtime(vec![task]);
        rt.run_scans(3);
        assert_eq!(rt.stats().first().map(|s| s.overruns), Some(3));
    }

    #[test]
    fn a_scan_inside_its_period_is_not_an_overrun() {
        let mut task = TaskConfig::cyclic("Main", ms(10), 1);
        task.programs = vec![ProgramBinding {
            routine: 0,
            base: 0,
        }];
        task.execution_time = ms(2);
        let mut rt = runtime(vec![task]);
        rt.run_scans(5);
        assert_eq!(rt.stats().first().map(|s| s.overruns), Some(0));
    }

    #[test]
    fn a_faulted_task_stops_and_the_fault_says_where() {
        let program = Program {
            routines: vec![Routine {
                name: "Divider".into(),
                code: vec![
                    Op::Const(0),
                    Op::Const(1),
                    Op::Binary {
                        op: BinOp::Div,
                        ty: ElementaryType::Dint,
                    },
                    Op::StoreSlot(0),
                    Op::Return,
                ],
                result_slot: None,
                frame_size: 0,
                max_stack: 2,
            }],
            constants: vec![Value::Dint(1), Value::Dint(0)],
            slot_types: vec![ElementaryType::Dint],
            slot_names: vec!["X".into()],
            ..Program::new()
        };
        let memory = Memory::new(&program.slot_types, 0, ImageLayout::default());
        let mut task = TaskConfig::cyclic("Main", ms(10), 1);
        task.programs = vec![ProgramBinding {
            routine: 0,
            base: 0,
        }];
        let mut rt = Runtime::new(program, memory, Clock::virtual_default(), vec![task]);
        rt.run_scans(5);
        assert!(rt.has_faulted());
        assert_eq!(rt.faults().len(), 1, "a stopped task kept running");
        let fault = rt.faults().first().expect("one fault");
        assert!(fault.to_string().contains("division by zero"), "{fault}");
        assert_eq!(rt.step(), StepOutcome::Idle);
    }

    #[test]
    fn a_runtime_with_no_tasks_is_idle_rather_than_looping() {
        let mut rt = runtime(vec![]);
        assert_eq!(rt.step(), StepOutcome::Idle);
        assert_eq!(rt.run_until(ms(1000)), 0);
    }

    #[test]
    fn statistics_record_the_instruction_cost_of_a_scan() {
        let mut task = TaskConfig::cyclic("Main", ms(10), 1);
        task.programs = vec![ProgramBinding {
            routine: 0,
            base: 0,
        }];
        let mut rt = runtime(vec![task]);
        rt.run_scans(4);
        let stats = rt.stats().first().copied().expect("one task");
        assert_eq!(stats.scans, 4);
        assert!(stats.min_instructions > 0);
        assert_eq!(
            stats.min_instructions, stats.max_instructions,
            "the scan is not constant"
        );
        assert_eq!(stats.mean_instructions(), stats.min_instructions);
    }

    #[test]
    fn a_recorded_run_produces_one_trace_row_per_scan() {
        let mut task = TaskConfig::cyclic("Main", ms(10), 1);
        task.programs = vec![ProgramBinding {
            routine: 0,
            base: 0,
        }];
        let mut rt = runtime(vec![task]);
        rt.record(vec![Signal {
            slot: SlotId(0),
            name: "Count".into(),
        }]);
        rt.run_scans(3);
        let trace = rt.trace().expect("recording");
        assert_eq!(trace.len(), 3);
        assert_eq!(
            trace.samples.last().map(|s| s.values.clone()),
            Some(vec![Value::Dint(3)])
        );
    }

    #[test]
    fn the_same_configuration_run_twice_produces_the_same_trace_fingerprint() {
        let run = || {
            let mut task = TaskConfig::cyclic("Main", ms(10), 1);
            task.programs = vec![ProgramBinding {
                routine: 0,
                base: 0,
            }];
            let mut rt = runtime(vec![task]);
            rt.record(vec![Signal {
                slot: SlotId(0),
                name: "Count".into(),
            }]);
            rt.run_until(ms(200));
            rt.trace().map(crate::trace::Trace::fingerprint_hex)
        };
        assert_eq!(run(), run());
        assert!(run().is_some());
    }

    #[test]
    fn the_scan_watchdog_stops_a_program_that_never_ends() {
        // WHILE TRUE DO ; END_WHILE compiles to a jump to itself. Without a
        // budget this would hang the test run rather than fail it.
        let program = Program {
            routines: vec![Routine {
                name: "Spin".into(),
                code: vec![Op::Jump(0)],
                result_slot: None,
                frame_size: 0,
                max_stack: 0,
            }],
            slot_types: vec![ElementaryType::Bool],
            slot_names: vec!["X".into()],
            ..Program::new()
        };
        let memory = Memory::new(&program.slot_types, 0, ImageLayout::default());
        let mut task = TaskConfig::cyclic("Main", ms(10), 1);
        task.programs = vec![ProgramBinding {
            routine: 0,
            base: 0,
        }];
        let mut rt = Runtime::new(program, memory, Clock::virtual_default(), vec![task])
            .with_limits(ExecLimits {
                max_instructions: 10_000,
                ..ExecLimits::default()
            });
        rt.run_scans(1);
        assert!(rt.has_faulted());
        let fault = rt.faults().first().expect("one fault");
        assert!(fault.to_string().contains("watchdog"), "{fault}");
    }
}
