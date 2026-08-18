// SPDX-License-Identifier: Apache-2.0
//! Running declarative tests against a compiled program.
//!
//! Each test gets a **fresh copy** of the program's memory and a fresh clock,
//! so tests cannot leak state into each other and their order cannot change
//! their results. That costs a memory clone per test and buys the property that
//! makes a suite trustworthy.

use std::collections::BTreeMap;

use salman_core::time::Duration;
use salman_core::value::{ElementaryType, Value};
use salman_vm::bytecode::Program;
use salman_vm::clock::Clock;
use salman_vm::compile::Compiled;
use salman_vm::memory::SlotId;
use salman_vm::task::Runtime;
use salman_vm::trace::{Signal, Trace};

use crate::spec::{Step, TestCase};
use crate::value::ValueSpec;

/// How a test ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// Every expectation held.
    Passed,
    /// An expectation did not hold.
    Failed,
    /// The test could not be run: a name did not resolve, a value did not fit,
    /// or the program faulted.
    Errored,
    /// The test declared a reason to skip.
    Skipped,
}

/// One thing that went wrong.
#[derive(Debug, Clone, PartialEq)]
pub struct Problem {
    /// Which step, counting from one. `None` for a problem in `given`.
    pub step: Option<usize>,
    /// What went wrong, in an engineer's words.
    pub message: String,
}

/// What running one test produced.
#[derive(Debug, Clone)]
pub struct Outcome {
    /// The test's name.
    pub name: String,
    /// How it ended.
    pub status: Status,
    /// Everything that went wrong, in order.
    pub problems: Vec<Problem>,
    /// Scans run in total.
    pub scans: u64,
    /// Simulation time at the end.
    pub elapsed: Duration,
    /// The recorded trace, when the test asked for one.
    pub trace: Option<Trace>,
    /// The golden file the trace should be compared against.
    pub golden: Option<String>,
}

impl Outcome {
    /// Whether the test passed or was skipped.
    #[must_use]
    pub const fn is_ok(&self) -> bool {
        matches!(self.status, Status::Passed | Status::Skipped)
    }
}

/// Runs one test.
#[must_use]
pub fn run(compiled: &Compiled, case: &TestCase) -> Outcome {
    let mut outcome = Outcome {
        name: case.test.clone(),
        status: Status::Passed,
        problems: Vec::new(),
        scans: 0,
        elapsed: Duration::ZERO,
        trace: None,
        golden: case.golden.clone(),
    };

    if let Some(reason) = &case.skip {
        outcome.status = Status::Skipped;
        outcome.problems.push(Problem { step: None, message: reason.clone() });
        return outcome;
    }

    let mut runtime = Runtime::new(
        compiled.program.clone(),
        compiled.memory.clone(),
        Clock::virtual_default(),
        compiled.tasks.clone(),
    )
    .with_seed(case.seed.unwrap_or(0));

    if !case.record.is_empty() {
        let mut signals = Vec::new();
        for name in &case.record {
            match resolve(&compiled.program, case.pou.as_deref(), name) {
                Ok(slot) => signals.push(Signal { slot, name: name.clone() }),
                Err(message) => {
                    outcome.problems.push(Problem { step: None, message });
                    outcome.status = Status::Errored;
                }
            }
        }
        runtime.record(signals);
    }

    apply(&mut runtime, compiled, case, None, &case.given, &mut outcome, Application::Set);

    for (index, step) in case.steps.iter().enumerate() {
        let number = index + 1;
        apply(&mut runtime, compiled, case, Some(number), &step.set, &mut outcome, Application::Set);
        apply(
            &mut runtime,
            compiled,
            case,
            Some(number),
            &step.force,
            &mut outcome,
            Application::Force,
        );
        for name in &step.release {
            match resolve(&compiled.program, case.pou.as_deref(), name) {
                Ok(slot) => {
                    runtime.memory_mut().release(slot);
                }
                Err(message) => problem(&mut outcome, Some(number), message),
            }
        }

        run_step(&mut runtime, step, &mut outcome, number);
        check(&runtime, compiled, case, step, number, &mut outcome);

        if runtime.has_faulted() {
            for fault in runtime.faults() {
                problem(&mut outcome, Some(number), format!("the program faulted: {fault}"));
            }
            outcome.status = Status::Errored;
            break;
        }
    }

    outcome.scans = runtime.scan_count();
    outcome.elapsed = runtime.clock().elapsed();
    outcome.trace = runtime.trace().cloned();
    outcome
}

/// Runs every test in a file against one compiled program.
#[must_use]
pub fn run_all(compiled: &Compiled, cases: &[TestCase]) -> Vec<Outcome> {
    cases.iter().map(|case| run(compiled, case)).collect()
}

enum Application {
    Set,
    Force,
}

fn problem(outcome: &mut Outcome, step: Option<usize>, message: impl Into<String>) {
    outcome.problems.push(Problem { step, message: message.into() });
    if outcome.status == Status::Passed {
        outcome.status = Status::Errored;
    }
}

fn apply(
    runtime: &mut Runtime,
    compiled: &Compiled,
    case: &TestCase,
    step: Option<usize>,
    values: &BTreeMap<String, ValueSpec>,
    outcome: &mut Outcome,
    how: Application,
) {
    for (name, spec) in values {
        let slot = match resolve(&compiled.program, case.pou.as_deref(), name) {
            Ok(slot) => slot,
            Err(message) => {
                problem(outcome, step, message);
                continue;
            }
        };
        let ty = slot_type(&compiled.program, slot);
        match spec.to_value(ty) {
            Ok(value) => match how {
                Application::Set => {
                    runtime.memory_mut().write_slot(slot, value);
                }
                Application::Force => {
                    runtime.memory_mut().force(slot, value);
                }
            },
            Err(error) => problem(outcome, step, format!("{name}: {error}")),
        }
    }
}

fn run_step(runtime: &mut Runtime, step: &Step, outcome: &mut Outcome, number: usize) {
    if let Some(text) = &step.advance {
        match ValueSpec::Text(text.clone()).to_value(ElementaryType::Time) {
            Ok(value) => {
                let by = value.as_duration().unwrap_or(Duration::ZERO);
                if by.is_negative() {
                    problem(outcome, Some(number), format!("cannot advance by {text}: time only moves forward"));
                } else {
                    let target = runtime.clock().elapsed().saturating_add(by);
                    runtime.run_until(target);
                }
            }
            Err(error) => problem(outcome, Some(number), format!("advance: {error}")),
        }
    }
    if let Some(scans) = step.scans {
        runtime.run_scans(scans);
    }
    // A step that neither advances nor scans still runs one scan, so that
    // `- { set: { Start: true }, expect: { Motor: true } }` means what it looks
    // like it means.
    if step.advance.is_none() && step.scans.is_none() && !step.expect.is_empty() {
        runtime.run_scans(1);
    }
}

fn check(
    runtime: &Runtime,
    compiled: &Compiled,
    case: &TestCase,
    step: &Step,
    number: usize,
    outcome: &mut Outcome,
) {
    for (name, spec) in &step.expect {
        let slot = match resolve(&compiled.program, case.pou.as_deref(), name) {
            Ok(slot) => slot,
            Err(message) => {
                problem(outcome, Some(number), message);
                continue;
            }
        };
        let ty = slot_type(&compiled.program, slot);
        let wanted = match spec.to_value(ty) {
            Ok(value) => value,
            Err(error) => {
                problem(outcome, Some(number), format!("{name}: {error}"));
                continue;
            }
        };
        let found = runtime.memory().read_slot(slot).cloned().unwrap_or(Value::Bool(false));
        if found != wanted {
            let note = step.note.as_ref().map_or(String::new(), |n| format!(" ({n})"));
            outcome.problems.push(Problem {
                step: Some(number),
                message: format!(
                    "{name} is {} at {}, expected {}{note}",
                    found.to_trace_string(),
                    runtime.clock().elapsed().to_iec_literal(),
                    wanted.to_trace_string()
                ),
            });
            if outcome.status == Status::Passed {
                outcome.status = Status::Failed;
            }
        }
    }
}

fn slot_type(program: &Program, slot: SlotId) -> ElementaryType {
    program.slot_types.get(slot.index()).copied().unwrap_or(ElementaryType::Dint)
}

/// Finds the slot a test file's name refers to.
///
/// A test writes `Motor_Run`, and the program's slots are named
/// `Conveyor_Ctrl.Motor_Run`. A bare name matches on its last dotted segment;
/// a name that would match several slots is an **error listing them**, never a
/// guess — a test that silently asserted about the wrong instance would be
/// worse than one that failed to run.
fn resolve(program: &Program, pou: Option<&str>, name: &str) -> Result<SlotId, String> {
    let mut exact = Vec::new();
    let mut suffix = Vec::new();
    for (index, candidate) in program.slot_names.iter().enumerate() {
        if candidate.eq_ignore_ascii_case(name) {
            exact.push(index);
            continue;
        }
        let tail_len = name.len() + 1;
        if candidate.len() > tail_len
            && candidate
                .get(candidate.len() - tail_len..)
                .is_some_and(|tail| tail.eq_ignore_ascii_case(&format!(".{name}")))
        {
            suffix.push(index);
        }
    }

    let mut candidates = if exact.is_empty() { suffix } else { exact };
    if let Some(pou) = pou
        && candidates.len() > 1
    {
        let prefix = format!("{pou}.");
        let narrowed: Vec<usize> = candidates
            .iter()
            .copied()
            .filter(|index| {
                program
                    .slot_names
                    .get(*index)
                    .is_some_and(|n| n.len() > prefix.len() && n.get(..prefix.len()).is_some_and(|p| p.eq_ignore_ascii_case(&prefix)))
            })
            .collect();
        if !narrowed.is_empty() {
            candidates = narrowed;
        }
    }

    match candidates.len() {
        0 => Err(format!(
            "no variable called {name}. The program has: {}",
            summarise(&program.slot_names)
        )),
        1 => candidates
            .first()
            .and_then(|index| u32::try_from(*index).ok())
            .map(SlotId)
            .ok_or_else(|| format!("{name} could not be addressed")),
        _ => {
            let names: Vec<&str> = candidates
                .iter()
                .filter_map(|index| program.slot_names.get(*index).map(String::as_str))
                .collect();
            Err(format!(
                "{name} is ambiguous: it could be {}. Write the full name, or set `pou:`",
                names.join(", ")
            ))
        }
    }
}

/// A short list of what exists, for a "no such variable" message.
fn summarise(names: &[String]) -> String {
    const SHOWN: usize = 12;
    let shown: Vec<&str> = names.iter().take(SHOWN).map(String::as_str).collect();
    if names.len() > SHOWN {
        format!("{}, and {} more", shown.join(", "), names.len() - SHOWN)
    } else {
        shown.join(", ")
    }
}
