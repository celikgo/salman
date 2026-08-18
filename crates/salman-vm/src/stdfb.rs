// SPDX-License-Identifier: Apache-2.0
//! The standard function blocks.
//!
//! Citation policy: salman cites IEC 61131-3:2013 (Edition 3.0) clause, table
//! and figure numbers as references only. **No IEC text is reproduced**, and
//! none of these implementations is a transcription: each is written from a
//! description of the observable behaviour, in salman's own code, and the
//! tests assert behaviour rather than body equivalence.
//!
//! # What salman could and could not verify
//!
//! The pages of Edition 3.0 that define these blocks are behind a paywall and
//! appear in no publisher preview. The behaviour below is **Edition 2 text,
//! believed unchanged in Edition 3**, reconstructed from vendor documentation
//! that cites both editions for identical behaviour, from open implementations,
//! and from a peer-reviewed formal analysis of the standard's own definitions.
//! Every place that matters says so.
//!
//! Two consequences worth reading before trusting anything here:
//!
//! * **The standard supplies no body for the timers at all.** `TP`, `TON` and
//!   `TOF` are defined only by timing diagrams — IEC 61131-3:2013 Figure 15
//!   "Standard timer function blocks - timing diagrams (Rules)". Every timer
//!   test here is therefore a trace of `(t, IN, PT)` against `(Q, ET)`, not a
//!   comparison against a body salman does not have.
//! * **`SEMA` is not a standard function block.** It is in neither Edition 2
//!   Table 34 nor Edition 3 Table 43, which between them contain every standard
//!   bistable. salman ships it because existing code uses it, and never
//!   describes it as standard. See [`NativeBlock::is_iec_standard`].

use salman_core::time::Duration;
use salman_core::value::{ElementaryType, Value};

use crate::bytecode::NativeBlock;
use crate::clock::Clock;
use crate::exec::FaultKind;
use crate::memory::{Memory, SlotId};

/// What a field of a function block instance is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldRole {
    /// Written by the caller, read by the block.
    Input,
    /// Written by the block, read by the caller.
    Output,
    /// The block's own state. Visible in a watch list, because a timer whose
    /// internals you cannot see is a timer you cannot debug.
    Internal,
}

/// One field of a function block instance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BlockField {
    /// The name, as written in Structured Text for inputs and outputs.
    pub name: &'static str,
    /// Its type.
    pub ty: ElementaryType,
    /// What it is for.
    pub role: FieldRole,
}

/// A timer or counter's internal phase.
///
/// Stored in a `BYTE` field so it appears in a watch list alongside everything
/// else rather than living somewhere only the runtime can see.
mod phase {
    /// Not timing, and nothing to show.
    pub(super) const IDLE: u8 = 0;
    /// Timing now.
    pub(super) const TIMING: u8 = 1;
    /// The interval finished; the elapsed time holds at `PT`.
    pub(super) const COMPLETE: u8 = 2;
}

/// Builds one field descriptor.
const fn f(name: &'static str, ty: ElementaryType, role: FieldRole) -> BlockField {
    BlockField { name, ty, role }
}

use ElementaryType as E;
use FieldRole::{Input, Internal, Output};

/// IEC 61131-3:2013 Table 43 "Standard bistable function blocks".
///
/// The dominant input carries the digit: `SR` is set dominant, so its set
/// input is `S1`. That mnemonic is the one thing that prevents the classic
/// slip of wiring the pair the wrong way round.
const SR_FIELDS: &[BlockField] = &[
    f("S1", E::Bool, Input),
    f("R", E::Bool, Input),
    f("Q1", E::Bool, Output),
];

/// IEC 61131-3:2013 Table 43. `RS` is reset dominant, so its reset is `R1`.
const RS_FIELDS: &[BlockField] = &[
    f("S", E::Bool, Input),
    f("R1", E::Bool, Input),
    f("Q1", E::Bool, Output),
];

/// IEC 61131-3:2013 Table 44 "Standard edge detection function blocks".
const TRIG_FIELDS: &[BlockField] = &[
    f("CLK", E::Bool, Input),
    f("Q", E::Bool, Output),
    f("M", E::Bool, Internal),
];

/// IEC 61131-3:2013 Table 45 "Standard counter function blocks".
///
/// `CU` is declared `BOOL R_EDGE` in the standard, so edge detection is part of
/// the parameter itself: a level that was already high when the instance
/// started does not count. `CU_M` is what remembers the previous level.
const CTU_FIELDS: &[BlockField] = &[
    f("CU", E::Bool, Input),
    f("R", E::Bool, Input),
    f("PV", E::Int, Input),
    f("Q", E::Bool, Output),
    f("CV", E::Int, Output),
    f("CU_M", E::Bool, Internal),
];

/// IEC 61131-3:2013 Table 45. `CTD` has no reset input; `LD` loads the preset.
const CTD_FIELDS: &[BlockField] = &[
    f("CD", E::Bool, Input),
    f("LD", E::Bool, Input),
    f("PV", E::Int, Input),
    f("Q", E::Bool, Output),
    f("CV", E::Int, Output),
    f("CD_M", E::Bool, Internal),
];

/// IEC 61131-3:2013 Table 45.
const CTUD_FIELDS: &[BlockField] = &[
    f("CU", E::Bool, Input),
    f("CD", E::Bool, Input),
    f("R", E::Bool, Input),
    f("LD", E::Bool, Input),
    f("PV", E::Int, Input),
    f("QU", E::Bool, Output),
    f("QD", E::Bool, Output),
    f("CV", E::Int, Output),
    f("CU_M", E::Bool, Internal),
    f("CD_M", E::Bool, Internal),
];

/// IEC 61131-3:2013 Table 46 "Standard timer function blocks" and Figure 15
/// "Standard timer function blocks - timing diagrams (Rules)".
///
/// The internal phase and start instant are ordinary fields rather than hidden
/// runtime state, because a timer whose internals you cannot watch is a timer
/// you cannot debug at three in the morning.
const TIMER_FIELDS: &[BlockField] = &[
    f("IN", E::Bool, Input),
    f("PT", E::Time, Input),
    f("Q", E::Bool, Output),
    f("ET", E::Time, Output),
    f("PHASE", E::Byte, Internal),
    f("START", E::LTime, Internal),
    f("PREV_IN", E::Bool, Internal),
];

/// Not standard. See the module documentation.
const SEMA_FIELDS: &[BlockField] = &[
    f("CLAIM", E::Bool, Input),
    f("RELEASE", E::Bool, Input),
    f("BUSY", E::Bool, Output),
    f("X", E::Bool, Internal),
];

/// The fields of every natively implemented block, in slot order.
///
/// The compiler allocates exactly this many consecutive slots for each
/// instance, so this list is the contract between the compiler and the runtime.
#[must_use]
pub const fn layout(block: NativeBlock) -> &'static [BlockField] {
    match block {
        NativeBlock::Sr => SR_FIELDS,
        NativeBlock::Rs => RS_FIELDS,
        NativeBlock::RTrig | NativeBlock::FTrig => TRIG_FIELDS,
        NativeBlock::Ctu => CTU_FIELDS,
        NativeBlock::Ctd => CTD_FIELDS,
        NativeBlock::Ctud => CTUD_FIELDS,
        NativeBlock::Tp | NativeBlock::Ton | NativeBlock::Tof => TIMER_FIELDS,
        NativeBlock::Sema => SEMA_FIELDS,
    }
}

/// How many slots one instance of a block occupies.
#[must_use]
pub fn slot_count(block: NativeBlock) -> u32 {
    layout(block).len() as u32
}

/// The slot offset of a named field, for the compiler and for tests.
#[must_use]
pub fn field_offset(block: NativeBlock, name: &str) -> Option<u32> {
    layout(block)
        .iter()
        .position(|f| f.name.eq_ignore_ascii_case(name))
        .and_then(|i| u32::try_from(i).ok())
}

/// Runs one invocation of a standard function block instance.
///
/// `base` is the instance's first slot; the fields follow in [`layout`] order.
///
/// # Errors
///
/// Returns a [`FaultKind`] if the instance's slots are missing or hold the
/// wrong types — which would be a compiler bug — or if a timer is given a
/// negative preset.
pub fn step(
    block: NativeBlock,
    base: SlotId,
    memory: &mut Memory,
    clock: &Clock,
) -> Result<(), FaultKind> {
    let mut instance = Instance {
        block,
        base,
        memory,
    };
    match block {
        NativeBlock::Sr => sr(&mut instance),
        NativeBlock::Rs => rs(&mut instance),
        NativeBlock::RTrig => r_trig(&mut instance),
        NativeBlock::FTrig => f_trig(&mut instance),
        NativeBlock::Ctu => ctu(&mut instance),
        NativeBlock::Ctd => ctd(&mut instance),
        NativeBlock::Ctud => ctud(&mut instance),
        NativeBlock::Tp => timer(&mut instance, TimerKind::Pulse, clock),
        NativeBlock::Ton => timer(&mut instance, TimerKind::OnDelay, clock),
        NativeBlock::Tof => timer(&mut instance, TimerKind::OffDelay, clock),
        NativeBlock::Sema => sema(&mut instance),
    }
}

/// A view of one instance's slots.
struct Instance<'a> {
    block: NativeBlock,
    base: SlotId,
    memory: &'a mut Memory,
}

impl Instance<'_> {
    fn slot(&self, name: &str) -> Result<SlotId, FaultKind> {
        let offset = field_offset(self.block, name).ok_or_else(|| {
            FaultKind::Unsupported(format!("field {name} of {}", self.block.name()))
        })?;
        Ok(SlotId(self.base.0.saturating_add(offset)))
    }

    fn read(&self, name: &str) -> Result<Value, FaultKind> {
        let slot = self.slot(name)?;
        self.memory
            .read_slot(slot)
            .cloned()
            .ok_or(FaultKind::SlotOutOfRange(slot.0))
    }

    fn write(&mut self, name: &str, value: Value) -> Result<(), FaultKind> {
        let slot = self.slot(name)?;
        if self.memory.write_slot(slot, value) {
            Ok(())
        } else {
            Err(FaultKind::SlotOutOfRange(slot.0))
        }
    }

    fn bool(&self, name: &str) -> Result<bool, FaultKind> {
        let value = self.read(name)?;
        value.as_bool().ok_or(FaultKind::TypeMismatch {
            expected: ElementaryType::Bool,
            found: value.type_of(),
        })
    }

    fn int(&self, name: &str) -> Result<i64, FaultKind> {
        let value = self.read(name)?;
        value.as_i64().ok_or(FaultKind::TypeMismatch {
            expected: ElementaryType::Int,
            found: value.type_of(),
        })
    }

    fn duration(&self, name: &str) -> Result<Duration, FaultKind> {
        let value = self.read(name)?;
        value.as_duration().ok_or(FaultKind::TypeMismatch {
            expected: ElementaryType::Time,
            found: value.type_of(),
        })
    }

    fn byte(&self, name: &str) -> Result<u8, FaultKind> {
        match self.read(name)? {
            Value::Byte(b) => Ok(b),
            other => Err(FaultKind::TypeMismatch {
                expected: ElementaryType::Byte,
                found: other.type_of(),
            }),
        }
    }

    fn set_bool(&mut self, name: &str, value: bool) -> Result<(), FaultKind> {
        self.write(name, Value::Bool(value))
    }
}

// ---------------------------------------------------------------------------
// Bistables — IEC 61131-3:2013 Table 43
// ---------------------------------------------------------------------------

/// `SR`, the **set dominant** bistable.
///
/// With both inputs true the output is set. The mnemonic that prevents the
/// classic slip: the dominant input is the one carrying the digit, so `SR` has
/// `S1` and `RS` has `R1`.
fn sr(fb: &mut Instance) -> Result<(), FaultKind> {
    let set = fb.bool("S1")?;
    let reset = fb.bool("R")?;
    let held = fb.bool("Q1")?;
    fb.set_bool("Q1", set || (!reset && held))
}

/// `RS`, the **reset dominant** bistable. With both inputs true it resets.
fn rs(fb: &mut Instance) -> Result<(), FaultKind> {
    let set = fb.bool("S")?;
    let reset = fb.bool("R1")?;
    let held = fb.bool("Q1")?;
    fb.set_bool("Q1", !reset && (set || held))
}

// ---------------------------------------------------------------------------
// Edge detection — IEC 61131-3:2013 Table 44
// ---------------------------------------------------------------------------

/// `R_TRIG`, rising edge.
///
/// The internal memory has no initialiser and therefore starts false, so a
/// fresh instance whose `CLK` is already true reports an edge on its first
/// call. That is what the definition says, and it is benign.
fn r_trig(fb: &mut Instance) -> Result<(), FaultKind> {
    let clk = fb.bool("CLK")?;
    let memory = fb.bool("M")?;
    fb.set_bool("Q", clk && !memory)?;
    fb.set_bool("M", clk)
}

/// `F_TRIG`, falling edge.
///
/// **A fresh instance called with `CLK` false emits one scan of `Q` true.**
/// The internal memory has no initialiser, so it starts false, and the block's
/// output is `NOT CLK AND NOT M`. This is specified behaviour, not a bug, and
/// there is a test that asserts the pulse rather than one that hides it.
///
/// This is **Edition 2 text believed unchanged in Edition 3** — salman could
/// not read the Edition 3 page, and relies on vendor documentation citing
/// Edition 2 Table 35.2 and Edition 3 Table 44.2 for identical behaviour.
/// IEC TR 61131-8 is reported to recommend the opposite, requiring `CLK` to
/// have been seen true first, and at least one vendor implements the technical
/// report's behaviour instead. salman follows IEC 61131-3.
/// **UNVERIFIED against the Edition 3 text.**
fn f_trig(fb: &mut Instance) -> Result<(), FaultKind> {
    let clk = fb.bool("CLK")?;
    let memory = fb.bool("M")?;
    fb.set_bool("Q", !clk && !memory)?;
    fb.set_bool("M", !clk)
}

// ---------------------------------------------------------------------------
// Counters — IEC 61131-3:2013 Table 45
// ---------------------------------------------------------------------------

/// The limits `CV` saturates at, from its declared type.
///
/// These are the counter type's own limits, **not** `PV`: a counter that has
/// reached its preset keeps counting. An implementation that stopped at `PV`
/// would disagree with the standard — and one widely used open implementation
/// does exactly that, which is worth knowing when comparing against it.
const CV_MIN: i64 = i16::MIN as i64;
/// The upper limit `CV` saturates at. See [`CV_MIN`].
const CV_MAX: i64 = i16::MAX as i64;

/// Detects a rising edge on an input whose previous level is kept in `memory`.
fn rising(fb: &mut Instance, input: &str, memory: &str) -> Result<bool, FaultKind> {
    let now = fb.bool(input)?;
    let before = fb.bool(memory)?;
    fb.set_bool(memory, now)?;
    Ok(now && !before)
}

/// `CTU`, count up.
///
/// `R` dominates `CU`. `CV` saturates at the counter type's maximum rather than
/// at `PV`. `Q` is `CV >= PV`, so if `PV` is above the type's maximum, `Q` can
/// never become true — the standard does not constrain `PV` against the type
/// limits, and salman does not invent a constraint it cannot cite.
fn ctu(fb: &mut Instance) -> Result<(), FaultKind> {
    let count_edge = rising(fb, "CU", "CU_M")?;
    let reset = fb.bool("R")?;
    let preset = fb.int("PV")?;
    let mut count = fb.int("CV")?;

    if reset {
        count = 0;
    } else if count_edge && count < CV_MAX {
        count += 1;
    }

    fb.write("CV", Value::Int(count as i16))?;
    fb.set_bool("Q", count >= preset)
}

/// `CTD`, count down.
///
/// `LD` dominates `CD`, and there is no reset input at all. `CV` saturates at
/// the counter type's minimum. `Q` is `CV <= 0`.
fn ctd(fb: &mut Instance) -> Result<(), FaultKind> {
    let count_edge = rising(fb, "CD", "CD_M")?;
    let load = fb.bool("LD")?;
    let preset = fb.int("PV")?;
    let mut count = fb.int("CV")?;

    if load {
        count = preset;
    } else if count_edge && count > CV_MIN {
        count -= 1;
    }

    fb.write("CV", Value::Int(count as i16))?;
    fb.set_bool("Q", count <= 0)
}

/// `CTUD`, count up and down.
///
/// Precedence is `R`, then `LD`, then counting. **Rising edges on `CU` and `CD`
/// in the same invocation cancel: the count does not change at all.**
fn ctud(fb: &mut Instance) -> Result<(), FaultKind> {
    let up_edge = rising(fb, "CU", "CU_M")?;
    let down_edge = rising(fb, "CD", "CD_M")?;
    let reset = fb.bool("R")?;
    let load = fb.bool("LD")?;
    let preset = fb.int("PV")?;
    let mut count = fb.int("CV")?;

    if reset {
        count = 0;
    } else if load {
        count = preset;
    } else if !(up_edge && down_edge) {
        if up_edge && count < CV_MAX {
            count += 1;
        } else if down_edge && count > CV_MIN {
            count -= 1;
        }
    }

    fb.write("CV", Value::Int(count as i16))?;
    fb.set_bool("QU", count >= preset)?;
    fb.set_bool("QD", count <= 0)
}

// ---------------------------------------------------------------------------
// Timers — IEC 61131-3:2013 Table 46 and Figure 15
// ---------------------------------------------------------------------------

/// Which of the three timers is running.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TimerKind {
    /// `TP`: a pulse of exactly `PT`, neither retriggerable nor truncatable.
    Pulse,
    /// `TON`: output rises `PT` after the input does, falls with it.
    OnDelay,
    /// `TOF`: output falls `PT` after the input does, rises with it.
    OffDelay,
}

/// The three timers.
///
/// # `PT` changed while the timer is running
///
/// The standard declines to define this: it says the effect of changing `PT`
/// during a timing operation is implementer-specific. **salman policy**: the
/// start instant is kept and `start + PT` is re-evaluated every invocation, so
/// shortening `PT` below the elapsed time ends the interval on the next scan
/// and setting `PT` to zero acts as a reset. That matches what the open and
/// vendor implementations salman could inspect do, and common practice is all
/// there is to go on here.
///
/// # `PT` negative
///
/// Also undefined by the standard. Negative duration literals are legal, so the
/// parser accepts `T#-250ms`, but a timer given one would produce
/// implementer-specific nonsense. **salman policy**: it is a runtime fault,
/// named as such.
fn timer(fb: &mut Instance, kind: TimerKind, clock: &Clock) -> Result<(), FaultKind> {
    let input = fb.bool("IN")?;
    let preset = fb.duration("PT")?;
    let previous_input = fb.bool("PREV_IN")?;
    let mut state = fb.byte("PHASE")?;
    let now = clock.elapsed();

    if preset.is_negative() {
        return Err(FaultKind::Unsupported(format!(
            "a negative preset time ({}) on a {} instance; salman refuses it rather than \
             producing an implementer-specific result",
            preset.to_iec_literal(),
            fb.block.name()
        )));
    }

    let rising_edge = input && !previous_input;
    let falling_edge = !input && previous_input;

    // Elapsed since the stored start instant. Recomputed from the start rather
    // than accumulated, which is what makes a change to PT part way through
    // take effect on the next invocation.
    let mut since = if state == phase::IDLE {
        Duration::ZERO
    } else {
        now.checked_sub(fb.duration("START")?)
            .unwrap_or(Duration::ZERO)
    };
    let restart = |fb: &mut Instance, state: &mut u8, since: &mut Duration| {
        *state = phase::TIMING;
        *since = Duration::ZERO;
        fb.write("START", Value::LTime(now))
    };

    let (output, elapsed) = match kind {
        TimerKind::OnDelay => {
            if input {
                if state == phase::IDLE {
                    restart(fb, &mut state, &mut since)?;
                }
                if since >= preset {
                    state = phase::COMPLETE;
                    (true, preset)
                } else {
                    state = phase::TIMING;
                    (false, since)
                }
            } else {
                // A falling input resets immediately. Elapsed time does not
                // accumulate across separate pulses of the input.
                state = phase::IDLE;
                (false, Duration::ZERO)
            }
        }
        TimerKind::OffDelay => {
            if input {
                state = phase::IDLE;
                (true, Duration::ZERO)
            } else {
                if falling_edge {
                    restart(fb, &mut state, &mut since)?;
                }
                if state == phase::IDLE {
                    // A fresh instance whose input starts low must NOT begin an
                    // off delay. Getting this wrong makes every program emit a
                    // phantom pulse at start-up.
                    (false, Duration::ZERO)
                } else if since >= preset {
                    state = phase::COMPLETE;
                    (false, preset)
                } else {
                    state = phase::TIMING;
                    (true, since)
                }
            }
        }
        TimerKind::Pulse => {
            // A pulse that has run its length is over, even if IN is still up.
            if state == phase::TIMING && since >= preset {
                state = phase::COMPLETE;
            }
            // PT lengthened after the pulse ended: it had not ended after all.
            if state == phase::COMPLETE && since < preset {
                state = phase::TIMING;
            }
            // The pulse is over and IN has gone low: ready for the next edge.
            if state == phase::COMPLETE && !input {
                state = phase::IDLE;
            }
            // A rising edge starts a pulse from any state but an active one —
            // which is what makes the pulse neither retriggerable nor
            // truncatable, and what allows a second pulse to begin in the very
            // invocation the first one ends.
            if state != phase::TIMING && rising_edge {
                restart(fb, &mut state, &mut since)?;
            }
            match state {
                phase::TIMING => (true, if since < preset { since } else { preset }),
                phase::COMPLETE => (false, preset),
                _ => (false, Duration::ZERO),
            }
        }
    };

    fb.write("PHASE", Value::Byte(state))?;
    fb.set_bool("PREV_IN", input)?;
    fb.set_bool("Q", output)?;
    fb.write("ET", Value::Time(elapsed))
}

// ---------------------------------------------------------------------------
// SEMA — not standard
// ---------------------------------------------------------------------------

/// `SEMA`, a semaphore. **Not an IEC 61131-3 standard function block.**
///
/// Two well-known implementations disagree observably. salman copies the one
/// that is published verbatim with a stated rationale: `BUSY` reports the state
/// as it was *before* this invocation, so the first caller to claim in a scan
/// sees `BUSY` false and wins. The other widely used implementation has no such
/// lag. Anyone porting between the two will see a one-scan difference, and this
/// comment is where they should find out why.
fn sema(fb: &mut Instance) -> Result<(), FaultKind> {
    let claim = fb.bool("CLAIM")?;
    let release = fb.bool("RELEASE")?;
    let held = fb.bool("X")?;

    let mut busy = held;
    let mut state = held;
    if claim {
        state = true;
    } else if release {
        busy = false;
        state = false;
    }

    fb.set_bool("BUSY", busy)?;
    fb.set_bool("X", state)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::ImageLayout;

    /// One function block instance, wired to memory and a clock.
    struct Fb {
        block: NativeBlock,
        memory: Memory,
        clock: Clock,
    }

    impl Fb {
        fn new(block: NativeBlock) -> Self {
            let types: Vec<ElementaryType> = layout(block).iter().map(|f| f.ty).collect();
            Self {
                block,
                memory: Memory::new(&types, 0, ImageLayout::default()),
                clock: Clock::virtual_default(),
            }
        }

        fn slot(&self, name: &str) -> SlotId {
            SlotId(field_offset(self.block, name).expect("field exists"))
        }

        fn set(&mut self, name: &str, value: Value) -> &mut Self {
            assert!(self.memory.write_slot(self.slot(name), value));
            self
        }

        fn set_bool(&mut self, name: &str, value: bool) -> &mut Self {
            self.set(name, Value::Bool(value))
        }

        fn get(&self, name: &str) -> Value {
            self.memory
                .read_slot(self.slot(name))
                .cloned()
                .expect("field exists")
        }

        fn q(&self) -> bool {
            self.get("Q").as_bool().expect("Q is BOOL")
        }

        fn et(&self) -> Duration {
            self.get("ET").as_duration().expect("ET is a duration")
        }

        fn cv(&self) -> i64 {
            self.get("CV").as_i64().expect("CV is an integer")
        }

        fn step(&mut self) -> &mut Self {
            step(self.block, SlotId(0), &mut self.memory, &self.clock).expect("no fault");
            self
        }

        fn try_step(&mut self) -> Result<(), FaultKind> {
            step(self.block, SlotId(0), &mut self.memory, &self.clock)
        }

        fn advance(&mut self, d: Duration) -> &mut Self {
            self.clock.advance(d);
            self
        }
    }

    fn ms(n: i64) -> Duration {
        Duration::from_nanos(n * 1_000_000)
    }

    fn s(n: i64) -> Duration {
        Duration::from_nanos(n * 1_000_000_000)
    }

    // -----------------------------------------------------------------
    // Layout
    // -----------------------------------------------------------------

    #[test]
    fn every_block_layout_has_unique_field_names_and_at_least_one_output() {
        for block in NativeBlock::all() {
            let fields = layout(*block);
            let mut names: Vec<&str> = fields.iter().map(|f| f.name).collect();
            let count = names.len();
            names.sort_unstable();
            names.dedup();
            assert_eq!(names.len(), count, "{} has a duplicate field", block.name());
            assert!(
                fields.iter().any(|f| f.role == FieldRole::Output),
                "{} has no output",
                block.name()
            );
            assert_eq!(slot_count(*block), fields.len() as u32);
        }
    }

    #[test]
    fn field_offsets_are_found_case_insensitively() {
        assert_eq!(field_offset(NativeBlock::Ton, "IN"), Some(0));
        assert_eq!(field_offset(NativeBlock::Ton, "pt"), Some(1));
        assert_eq!(field_offset(NativeBlock::Ton, "Q"), Some(2));
        assert_eq!(field_offset(NativeBlock::Ton, "ET"), Some(3));
        assert_eq!(field_offset(NativeBlock::Ton, "nope"), None);
    }

    #[test]
    fn internal_state_is_a_visible_field_so_a_timer_can_be_debugged() {
        // A timer whose internals only the runtime can see is a timer nobody
        // can diagnose at three in the morning.
        let internals: Vec<&str> = layout(NativeBlock::Ton)
            .iter()
            .filter(|f| f.role == FieldRole::Internal)
            .map(|f| f.name)
            .collect();
        assert!(internals.contains(&"PHASE"));
        assert!(internals.contains(&"START"));
    }

    // -----------------------------------------------------------------
    // Bistables — IEC 61131-3:2013 Table 43
    // -----------------------------------------------------------------

    #[test]
    fn sr_is_set_dominant_when_both_inputs_are_true() {
        let mut fb = Fb::new(NativeBlock::Sr);
        fb.set_bool("S1", true).set_bool("R", true).step();
        assert_eq!(fb.get("Q1"), Value::Bool(true));
    }

    #[test]
    fn rs_is_reset_dominant_when_both_inputs_are_true() {
        let mut fb = Fb::new(NativeBlock::Rs);
        fb.set_bool("S", true).set_bool("R1", true).step();
        assert_eq!(fb.get("Q1"), Value::Bool(false));
    }

    #[test]
    fn a_bistable_starts_reset_and_holds_its_state() {
        let mut fb = Fb::new(NativeBlock::Sr);
        fb.step();
        assert_eq!(fb.get("Q1"), Value::Bool(false));
        fb.set_bool("S1", true).step();
        assert_eq!(fb.get("Q1"), Value::Bool(true));
        fb.set_bool("S1", false).step();
        assert_eq!(fb.get("Q1"), Value::Bool(true), "SR did not hold");
        fb.set_bool("R", true).step();
        assert_eq!(fb.get("Q1"), Value::Bool(false));
    }

    #[test]
    fn rs_holds_its_state_too() {
        let mut fb = Fb::new(NativeBlock::Rs);
        fb.set_bool("S", true).step();
        assert_eq!(fb.get("Q1"), Value::Bool(true));
        fb.set_bool("S", false).step();
        assert_eq!(fb.get("Q1"), Value::Bool(true));
        fb.set_bool("R1", true).step();
        assert_eq!(fb.get("Q1"), Value::Bool(false));
        fb.set_bool("R1", false).step();
        assert_eq!(fb.get("Q1"), Value::Bool(false));
    }

    // -----------------------------------------------------------------
    // Edge detection — IEC 61131-3:2013 Table 44
    // -----------------------------------------------------------------

    #[test]
    fn r_trig_pulses_for_exactly_one_invocation_on_a_rising_edge() {
        let mut fb = Fb::new(NativeBlock::RTrig);
        fb.step();
        assert!(!fb.q(), "no edge yet");
        fb.set_bool("CLK", true).step();
        assert!(fb.q(), "rising edge missed");
        fb.step();
        assert!(!fb.q(), "R_TRIG pulsed for more than one invocation");
        fb.set_bool("CLK", false).step();
        assert!(!fb.q());
        fb.set_bool("CLK", true).step();
        assert!(fb.q(), "second rising edge missed");
    }

    #[test]
    fn a_fresh_r_trig_whose_clock_is_already_true_reports_an_edge() {
        // The internal memory has no initialiser and therefore starts false.
        // This follows from the definition and is benign.
        let mut fb = Fb::new(NativeBlock::RTrig);
        fb.set_bool("CLK", true).step();
        assert!(fb.q());
    }

    #[test]
    fn a_fresh_f_trig_emits_one_spurious_pulse_with_its_clock_low() {
        // ASSERTED DELIBERATELY. F_TRIG's internal memory has no initialiser,
        // so it starts false, and the output is NOT CLK AND NOT M. A fresh
        // instance called with CLK false therefore reports a falling edge that
        // never happened.
        //
        // This is Edition 2 text believed unchanged in Edition 3; salman could
        // not read the Edition 3 page. IEC TR 61131-8 is reported to recommend
        // the opposite, and at least one vendor implements the technical
        // report's behaviour instead. salman follows IEC 61131-3 and asserts
        // the pulse rather than hiding it. UNVERIFIED against Edition 3.
        let mut fb = Fb::new(NativeBlock::FTrig);
        fb.step();
        assert!(fb.q(), "the documented first-call pulse did not happen");
        fb.step();
        assert!(!fb.q(), "the pulse lasted more than one invocation");
    }

    #[test]
    fn f_trig_pulses_on_a_falling_edge() {
        let mut fb = Fb::new(NativeBlock::FTrig);
        fb.set_bool("CLK", true).step();
        assert!(!fb.q());
        fb.step();
        assert!(!fb.q());
        fb.set_bool("CLK", false).step();
        assert!(fb.q(), "falling edge missed");
        fb.step();
        assert!(!fb.q());
    }

    // -----------------------------------------------------------------
    // Counters — IEC 61131-3:2013 Table 45
    // -----------------------------------------------------------------

    #[test]
    fn ctu_counts_rising_edges_only_not_levels() {
        let mut fb = Fb::new(NativeBlock::Ctu);
        fb.set("PV", Value::Int(3));
        fb.set_bool("CU", true).step();
        assert_eq!(fb.cv(), 1);
        fb.step();
        assert_eq!(fb.cv(), 1, "a held level counted more than once");
        fb.set_bool("CU", false).step();
        fb.set_bool("CU", true).step();
        assert_eq!(fb.cv(), 2);
    }

    #[test]
    fn ctu_reset_dominates_the_count_input() {
        let mut fb = Fb::new(NativeBlock::Ctu);
        fb.set("PV", Value::Int(3));
        fb.set_bool("CU", true).step();
        fb.set_bool("CU", false).step();
        fb.set_bool("CU", true).set_bool("R", true).step();
        assert_eq!(fb.cv(), 0, "R must dominate CU");
    }

    #[test]
    fn ctu_output_is_true_from_the_moment_the_count_reaches_the_preset() {
        let mut fb = Fb::new(NativeBlock::Ctu);
        fb.set("PV", Value::Int(2));
        for expected in [(1, false), (2, true), (3, true)] {
            fb.set_bool("CU", false).step();
            fb.set_bool("CU", true).step();
            assert_eq!((fb.cv(), fb.q()), (expected.0, expected.1));
        }
    }

    #[test]
    fn ctu_keeps_counting_past_its_preset_and_saturates_at_the_type_limit() {
        // The count saturates at the counter TYPE's maximum, not at PV. One
        // widely used open implementation stops at PV instead; that is a real
        // and known disagreement, and the standard is what salman follows.
        let mut fb = Fb::new(NativeBlock::Ctu);
        fb.set("PV", Value::Int(1));
        fb.set("CV", Value::Int(i16::MAX - 1));
        fb.set_bool("CU", true).step();
        assert_eq!(fb.cv(), i64::from(i16::MAX));
        fb.set_bool("CU", false).step();
        fb.set_bool("CU", true).step();
        assert_eq!(
            fb.cv(),
            i64::from(i16::MAX),
            "the counter wrapped instead of saturating"
        );
    }

    #[test]
    fn a_preset_above_the_counter_type_can_never_be_reached_and_salman_does_not_pretend_otherwise()
    {
        // The standard does not constrain PV against the counter's own limits.
        // salman does not invent a constraint it cannot cite; it simply never
        // sets Q, which is what the definition produces.
        let mut fb = Fb::new(NativeBlock::Ctu);
        fb.set("PV", Value::Int(i16::MAX));
        fb.set("CV", Value::Int(i16::MAX - 1));
        fb.set_bool("CU", true).step();
        assert!(fb.q(), "reaching PV exactly must set Q");
    }

    #[test]
    fn ctd_loads_its_preset_and_load_dominates_the_count_input() {
        let mut fb = Fb::new(NativeBlock::Ctd);
        fb.set("PV", Value::Int(3));
        fb.set_bool("LD", true).set_bool("CD", true).step();
        assert_eq!(fb.cv(), 3, "LD must dominate CD");
        fb.set_bool("LD", false).set_bool("CD", false).step();
        fb.set_bool("CD", true).step();
        assert_eq!(fb.cv(), 2);
    }

    #[test]
    fn ctd_output_is_true_at_and_below_zero() {
        let mut fb = Fb::new(NativeBlock::Ctd);
        fb.set("PV", Value::Int(1));
        fb.set_bool("LD", true).step();
        assert_eq!((fb.cv(), fb.q()), (1, false));
        fb.set_bool("LD", false).set_bool("CD", true).step();
        assert_eq!((fb.cv(), fb.q()), (0, true));
    }

    #[test]
    fn ctd_saturates_at_the_type_minimum() {
        let mut fb = Fb::new(NativeBlock::Ctd);
        fb.set("CV", Value::Int(i16::MIN + 1));
        fb.set_bool("CD", true).step();
        assert_eq!(fb.cv(), i64::from(i16::MIN));
        fb.set_bool("CD", false).step();
        fb.set_bool("CD", true).step();
        assert_eq!(
            fb.cv(),
            i64::from(i16::MIN),
            "the counter wrapped instead of saturating"
        );
    }

    #[test]
    fn ctud_precedence_is_reset_then_load_then_counting() {
        let mut fb = Fb::new(NativeBlock::Ctud);
        fb.set("PV", Value::Int(5));
        fb.set_bool("R", true)
            .set_bool("LD", true)
            .set_bool("CU", true)
            .step();
        assert_eq!(fb.cv(), 0, "R must dominate LD");
        fb.set_bool("R", false).set_bool("CU", false).step();
        assert_eq!(fb.cv(), 5, "LD must dominate counting");
    }

    #[test]
    fn simultaneous_up_and_down_edges_leave_the_count_alone() {
        // The guard in the definition is `NOT (CU AND CD)`, so two edges in one
        // invocation cancel entirely rather than netting to zero by accident.
        let mut fb = Fb::new(NativeBlock::Ctud);
        fb.set("PV", Value::Int(10)).set("CV", Value::Int(4));
        fb.set_bool("CU", true).set_bool("CD", true).step();
        assert_eq!(fb.cv(), 4);
    }

    #[test]
    fn ctud_reports_both_of_its_outputs() {
        let mut fb = Fb::new(NativeBlock::Ctud);
        fb.set("PV", Value::Int(2));
        fb.step();
        assert_eq!(fb.get("QU"), Value::Bool(false));
        assert_eq!(
            fb.get("QD"),
            Value::Bool(true),
            "CV starts at 0, so QD is true"
        );
        fb.set_bool("CU", true).step();
        assert_eq!(fb.cv(), 1);
        assert_eq!(fb.get("QD"), Value::Bool(false));
        fb.set_bool("CU", false).step();
        fb.set_bool("CU", true).step();
        assert_eq!(fb.get("QU"), Value::Bool(true));
    }

    // -----------------------------------------------------------------
    // TON — IEC 61131-3:2013 Table 46 and Figure 15
    // -----------------------------------------------------------------

    #[test]
    fn ton_does_not_fire_early() {
        let mut fb = Fb::new(NativeBlock::Ton);
        fb.set("PT", Value::Time(s(5)));
        fb.set_bool("IN", true).step();
        assert!(!fb.q());
        fb.advance(ms(4_999)).step();
        assert!(!fb.q(), "TON fired 1 ms early");
        assert_eq!(fb.et(), ms(4_999));
        fb.advance(ms(1)).step();
        assert!(fb.q(), "TON did not fire at PT");
        assert_eq!(fb.et(), s(5));
    }

    #[test]
    fn ton_elapsed_time_is_clamped_at_the_preset() {
        let mut fb = Fb::new(NativeBlock::Ton);
        fb.set("PT", Value::Time(s(1)));
        fb.set_bool("IN", true).step();
        fb.advance(s(10)).step();
        assert_eq!(fb.et(), s(1), "ET ran past PT");
        assert!(fb.q());
    }

    #[test]
    fn ton_falls_immediately_when_its_input_does() {
        let mut fb = Fb::new(NativeBlock::Ton);
        fb.set("PT", Value::Time(s(1)));
        fb.set_bool("IN", true).step();
        fb.advance(s(2)).step();
        assert!(fb.q());
        fb.set_bool("IN", false).step();
        assert!(!fb.q());
        assert_eq!(fb.et(), Duration::ZERO);
    }

    #[test]
    fn ton_does_not_accumulate_elapsed_time_across_separate_input_pulses() {
        // Two three-second pulses do not add up to a five-second delay. Each
        // rising edge restarts from zero.
        let mut fb = Fb::new(NativeBlock::Ton);
        fb.set("PT", Value::Time(s(5)));
        fb.set_bool("IN", true).step();
        fb.advance(s(3)).step();
        assert!(!fb.q());
        fb.set_bool("IN", false).step();
        assert_eq!(fb.et(), Duration::ZERO);
        fb.set_bool("IN", true).step();
        fb.advance(s(3)).step();
        assert!(
            !fb.q(),
            "TON accumulated elapsed time across two input pulses"
        );
        assert_eq!(fb.et(), s(3));
    }

    #[test]
    fn shortening_the_preset_below_the_elapsed_time_ends_the_interval() {
        // salman policy, not a standard requirement: the standard says the
        // effect of changing PT while timing is implementer-specific. salman
        // keeps the start instant and re-evaluates start + PT every invocation.
        let mut fb = Fb::new(NativeBlock::Ton);
        fb.set("PT", Value::Time(s(10)));
        fb.set_bool("IN", true).step();
        fb.advance(s(3)).step();
        assert!(!fb.q());
        fb.set("PT", Value::Time(s(2))).step();
        assert!(
            fb.q(),
            "reducing PT below the elapsed time did not end the interval"
        );
    }

    #[test]
    fn lengthening_the_preset_after_completion_resumes_timing() {
        // The other half of the same policy: start + PT is re-evaluated, so a
        // completed interval can become incomplete again.
        let mut fb = Fb::new(NativeBlock::Ton);
        fb.set("PT", Value::Time(s(1)));
        fb.set_bool("IN", true).step();
        fb.advance(s(2)).step();
        assert!(fb.q());
        fb.set("PT", Value::Time(s(5))).step();
        assert!(!fb.q(), "lengthening PT did not resume timing");
        assert_eq!(fb.et(), s(2));
    }

    #[test]
    fn a_zero_preset_makes_ton_fire_at_once() {
        let mut fb = Fb::new(NativeBlock::Ton);
        fb.set("PT", Value::Time(Duration::ZERO));
        fb.set_bool("IN", true).step();
        assert!(fb.q());
    }

    #[test]
    fn a_negative_preset_is_a_fault_rather_than_an_implementer_specific_result() {
        // Negative duration literals are legal, so the parser accepts T#-250ms.
        // A timer given one is undefined by the standard, so salman refuses it
        // by name instead of inventing a behaviour. salman policy.
        for block in [NativeBlock::Ton, NativeBlock::Tof, NativeBlock::Tp] {
            let mut fb = Fb::new(block);
            fb.set("PT", Value::Time(ms(-250)));
            let err = fb.try_step().unwrap_err();
            assert!(
                matches!(&err, FaultKind::Unsupported(m) if m.contains("negative preset")),
                "{}: {err}",
                block.name()
            );
        }
    }

    // -----------------------------------------------------------------
    // TOF — IEC 61131-3:2013 Table 46 and Figure 15
    // -----------------------------------------------------------------

    #[test]
    fn a_fresh_tof_with_its_input_low_does_not_start_an_off_delay() {
        // TOF's analogue of the F_TRIG trap. Getting this wrong makes every
        // program that uses a TOF emit a phantom pulse at start-up.
        let mut fb = Fb::new(NativeBlock::Tof);
        fb.set("PT", Value::Time(s(5)));
        fb.step();
        assert!(!fb.q(), "a fresh TOF emitted a phantom pulse");
        assert_eq!(fb.et(), Duration::ZERO);
        fb.advance(s(10)).step();
        assert!(!fb.q());
    }

    #[test]
    fn tof_holds_its_output_for_the_preset_after_its_input_falls() {
        let mut fb = Fb::new(NativeBlock::Tof);
        fb.set("PT", Value::Time(s(5)));
        fb.set_bool("IN", true).step();
        assert!(fb.q(), "TOF follows its input up immediately");
        fb.set_bool("IN", false).step();
        assert!(fb.q(), "TOF dropped as soon as its input did");
        fb.advance(ms(4_999)).step();
        assert!(fb.q());
        assert_eq!(fb.et(), ms(4_999));
        fb.advance(ms(1)).step();
        assert!(!fb.q(), "TOF did not drop at PT");
        assert_eq!(fb.et(), s(5), "ET holds at PT after the delay ends");
    }

    #[test]
    fn a_rising_input_during_a_tof_off_delay_aborts_it() {
        let mut fb = Fb::new(NativeBlock::Tof);
        fb.set("PT", Value::Time(s(5)));
        fb.set_bool("IN", true).step();
        fb.set_bool("IN", false).step();
        fb.advance(s(2)).step();
        assert_eq!(fb.et(), s(2));
        fb.set_bool("IN", true).step();
        assert!(fb.q());
        assert_eq!(
            fb.et(),
            Duration::ZERO,
            "the aborted off delay left elapsed time behind"
        );
        fb.advance(s(10)).step();
        assert!(fb.q(), "the aborted off delay fired anyway");
    }

    // -----------------------------------------------------------------
    // TP — IEC 61131-3:2013 Table 46 and Figure 15
    // -----------------------------------------------------------------

    #[test]
    fn tp_produces_a_pulse_of_exactly_the_preset() {
        let mut fb = Fb::new(NativeBlock::Tp);
        fb.set("PT", Value::Time(s(3)));
        fb.set_bool("IN", true).step();
        assert!(fb.q());
        assert_eq!(fb.et(), Duration::ZERO);
        fb.advance(ms(2_999)).step();
        assert!(fb.q());
        assert_eq!(fb.et(), ms(2_999));
        fb.advance(ms(1)).step();
        assert!(!fb.q(), "the pulse outlasted PT");
        assert_eq!(fb.et(), s(3));
    }

    #[test]
    fn tp_is_not_retriggerable_during_its_pulse() {
        let mut fb = Fb::new(NativeBlock::Tp);
        fb.set("PT", Value::Time(s(3)));
        fb.set_bool("IN", true).step();
        fb.advance(s(1)).set_bool("IN", false).step();
        fb.set_bool("IN", true).step();
        assert_eq!(fb.et(), s(1), "a rising edge during the pulse restarted it");
        fb.advance(s(2)).step();
        assert!(!fb.q(), "the pulse was extended by a retrigger");
    }

    #[test]
    fn tp_is_not_truncatable_by_its_input_falling() {
        let mut fb = Fb::new(NativeBlock::Tp);
        fb.set("PT", Value::Time(s(3)));
        fb.set_bool("IN", true).step();
        fb.advance(ms(500)).set_bool("IN", false).step();
        assert!(fb.q(), "the pulse was cut short when IN fell");
        fb.advance(ms(2_499)).step();
        assert!(fb.q());
        fb.advance(ms(1)).step();
        assert!(!fb.q());
    }

    #[test]
    fn tp_holds_elapsed_at_the_preset_until_its_input_goes_low() {
        let mut fb = Fb::new(NativeBlock::Tp);
        fb.set("PT", Value::Time(s(1)));
        fb.set_bool("IN", true).step();
        fb.advance(s(2)).step();
        assert!(!fb.q());
        assert_eq!(fb.et(), s(1));
        fb.advance(s(5)).step();
        assert_eq!(fb.et(), s(1), "ET moved after the pulse ended");
        fb.set_bool("IN", false).step();
        assert_eq!(fb.et(), Duration::ZERO, "ET did not reset when IN went low");
    }

    #[test]
    fn a_rising_edge_in_the_invocation_a_pulse_ends_starts_the_next_one() {
        // salman policy: the pulse-active test is strict, so the invocation in
        // which elapsed reaches PT is already past the end of the pulse and a
        // rising edge in that same invocation begins a new one back to back.
        let mut fb = Fb::new(NativeBlock::Tp);
        fb.set("PT", Value::Time(s(2)));
        fb.set_bool("IN", true).step();
        // The input goes low during the pulse, which does not truncate it.
        fb.advance(ms(500)).set_bool("IN", false).step();
        assert!(fb.q());
        // It rises again exactly as the pulse completes.
        fb.advance(ms(1_500)).set_bool("IN", true).step();
        assert!(fb.q(), "the back-to-back pulse did not start");
        assert_eq!(
            fb.et(),
            Duration::ZERO,
            "the new pulse did not start from zero"
        );
    }

    // -----------------------------------------------------------------
    // SEMA — not standard
    // -----------------------------------------------------------------

    #[test]
    fn sema_reports_the_state_before_this_invocation_so_the_first_claimer_wins() {
        // SEMA is NOT an IEC 61131-3 standard function block, and the two
        // well-known implementations disagree observably. salman copies the one
        // published with a stated rationale and names the divergence.
        let mut fb = Fb::new(NativeBlock::Sema);
        fb.set_bool("CLAIM", true).step();
        assert_eq!(
            fb.get("BUSY"),
            Value::Bool(false),
            "the first claimer must win"
        );
        fb.step();
        assert_eq!(
            fb.get("BUSY"),
            Value::Bool(true),
            "a second claimer must be refused"
        );
        fb.set_bool("CLAIM", false).set_bool("RELEASE", true).step();
        assert_eq!(fb.get("BUSY"), Value::Bool(false));
        fb.set_bool("RELEASE", false).set_bool("CLAIM", true).step();
        assert_eq!(
            fb.get("BUSY"),
            Value::Bool(false),
            "the semaphore was not released"
        );
    }

    // -----------------------------------------------------------------
    // Determinism
    // -----------------------------------------------------------------

    #[test]
    fn every_block_produces_the_same_trace_from_the_same_inputs() {
        let run = |block: NativeBlock| {
            let mut fb = Fb::new(block);
            let mut trace = String::new();
            for scan in 0..40 {
                for field in layout(block).iter().filter(|f| f.role == FieldRole::Input) {
                    let value = match field.ty {
                        ElementaryType::Bool => Value::Bool(scan % 3 == 0),
                        ElementaryType::Int => Value::Int(3),
                        ElementaryType::Time => Value::Time(ms(250)),
                        other => other.default_value(),
                    };
                    fb.set(field.name, value);
                }
                fb.advance(ms(100)).step();
                for field in layout(block).iter().filter(|f| f.role == FieldRole::Output) {
                    trace.push_str(&fb.get(field.name).to_trace_string());
                    trace.push(';');
                }
            }
            trace
        };
        for block in NativeBlock::all() {
            assert_eq!(
                run(*block),
                run(*block),
                "{} is not deterministic",
                block.name()
            );
        }
    }
}
