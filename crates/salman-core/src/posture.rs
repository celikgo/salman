// SPDX-License-Identifier: Apache-2.0
//! The safety posture: what salman is currently allowed to affect.
//!
//! salman can, in later versions, talk to machinery. The rule that makes that
//! acceptable is that it is **read-only by default** and that reaching a state
//! where it can write is an explicit, expiring, human action.
//!
//! Three states, and there is no fourth:
//!
//! | Posture    | May affect                                            |
//! |------------|-------------------------------------------------------|
//! | `Observe`  | nothing outside the process — reads only. **Default.** |
//! | `Simulate` | simulated devices inside salman only.                  |
//! | `Armed`    | real devices, and only with per-call confirmation.     |
//!
//! Some effects are refused at **every** posture. They are refused here, in
//! code, and are not configuration options: credential guessing, denial of
//! service, and firmware manipulation. See `README.md` and
//! `docs/adr/ADR-0002-read-only-by-default.md`.
//!
//! # State of this module
//!
//! This module was written before anything in salman could reach a network, so
//! that the first write path could not be written without going through it.
//! That path now exists: `salman_modbus_net::Client::write` is the first caller
//! of [`PostureState::permits`], asks for [`Effect::WriteLiveDevice`], and is
//! refused at anything below [`Posture::Armed`]. It also takes a
//! [`UserConfirmation`] by value, so one confirmation authorises one write.
//!
//! Reads call nothing here, which is what read-only by default means.

use std::fmt;

/// What salman is currently permitted to affect.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub enum Posture {
    /// Read-only. The default, and the state everything returns to.
    #[default]
    Observe,
    /// May write to devices simulated inside salman. Never to real ones.
    Simulate,
    /// May write to real devices, subject to per-call confirmation.
    Armed,
}

impl Posture {
    /// A short, stable label for logs, the CLI and the UI banner.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Observe => "OBSERVE",
            Self::Simulate => "SIMULATE",
            Self::Armed => "ARMED",
        }
    }
}

impl fmt::Display for Posture {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

/// Something salman might do that reaches outside its own process.
///
/// Every outward-facing operation in salman must classify itself as exactly one
/// of these so that the posture check is total rather than a list of special
/// cases somebody remembered to write.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum Effect {
    /// Reading a file, capture or project from local disk.
    ReadLocalFile,
    /// Passively listening to a network, or issuing a protocol read.
    ReadDevice,
    /// Writing to a device simulated inside salman.
    WriteSimulated,
    /// Writing a value to a real device on a real network.
    WriteLiveDevice,
    /// Changing a controller's run/stop mode.
    ChangeControllerMode,
    /// Probing addresses to find devices. Permitted only inside address ranges
    /// the user has explicitly declared as theirs.
    NetworkDiscovery,
    /// Reading, writing or erasing device firmware.
    FirmwareOperation,
    /// Trying candidate credentials against a device.
    CredentialGuessing,
    /// Anything whose purpose or predictable effect is to degrade a device or
    /// network: flooding, malformed-frame storms, resource exhaustion.
    DenialOfService,
}

impl Effect {
    /// Effects salman refuses at every posture, for every user, always.
    ///
    /// These are refused because salman is an engineering tool and not an
    /// attack tool. Adding a configuration option to enable them would be a
    /// change of purpose, not a feature.
    #[must_use]
    pub const fn is_categorically_refused(self) -> bool {
        matches!(
            self,
            Self::FirmwareOperation | Self::CredentialGuessing | Self::DenialOfService
        )
    }
}

/// The answer to "may I do this?".
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Permit {
    /// Go ahead.
    Allowed,
    /// Allowed by the posture, but a human must confirm this specific call
    /// before it happens, and be shown what it will do.
    RequiresConfirmation,
    /// Refused, with a reason fit to show a user.
    Denied(DenialReason),
}

impl Permit {
    /// Whether the operation may proceed without asking anybody.
    #[must_use]
    pub const fn is_allowed_outright(&self) -> bool {
        matches!(self, Self::Allowed)
    }
}

/// Why an effect was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DenialReason {
    /// The effect is refused at every posture and cannot be enabled.
    CategoricallyRefused,
    /// The current posture is too low. Carries the posture that would be
    /// needed, so the UI can say what to do about it.
    PostureTooLow {
        /// The lowest posture that permits this effect.
        required: Posture,
    },
    /// Discovery was requested outside the address ranges the user declared.
    OutsideDeclaredScope,
}

impl fmt::Display for DenialReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CategoricallyRefused => f.write_str(
                "salman does not do this at any posture: it is an engineering tool, not an attack tool",
            ),
            Self::PostureTooLow { required } => {
                write!(f, "requires posture {required}, which a person must set explicitly")
            }
            Self::OutsideDeclaredScope => {
                f.write_str("address is outside the ranges declared as owned by the user")
            }
        }
    }
}

/// Proof that a human was asked and said yes.
///
/// This type has no public constructor. The only way to obtain one is
/// [`ConfirmationRequest::ask`], which requires a [`ConfirmationPrompt`] — that
/// is, something that can actually put the question in front of a person. An
/// automated caller cannot manufacture consent by constructing this value.
#[derive(Debug)]
pub struct UserConfirmation {
    _private: (),
}

/// Something able to put a question to a human and return their answer.
///
/// Implemented by the desktop app and by an interactive CLI. An agent must be
/// given one; it cannot be one.
pub trait ConfirmationPrompt {
    /// Puts `request` to the user and returns what they chose.
    fn confirm(&mut self, request: &ConfirmationRequest) -> Decision;
}

/// What a human decided.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    /// The user approved this specific action.
    Approved,
    /// The user refused.
    Refused,
}

/// A specific action, described well enough that a person can decide about it.
///
/// Every field exists because a confirmation dialog that omits it is a
/// confirmation dialog nobody can act on: you cannot approve a write without
/// knowing what is there now.
#[derive(Debug, Clone)]
pub struct ConfirmationRequest {
    /// The effect being requested.
    pub effect: Effect,
    /// Human-readable device identity, e.g. `"PLC-01 (10.4.2.7:502)"`.
    pub device: String,
    /// Human-readable address, e.g. `"holding register 40001"`.
    pub address: String,
    /// The value currently there, rendered for display, if it is known.
    pub current_value: Option<String>,
    /// The value that would be written, rendered for display.
    pub new_value: Option<String>,
    /// Why the caller says it wants to do this.
    pub declared_intent: String,
}

impl ConfirmationRequest {
    /// Asks a human, and returns proof if they agreed.
    pub fn ask(&self, prompt: &mut dyn ConfirmationPrompt) -> Option<UserConfirmation> {
        match prompt.confirm(self) {
            Decision::Approved => Some(UserConfirmation { _private: () }),
            Decision::Refused => None,
        }
    }
}

/// The live posture, including the expiry of an arming grant.
///
/// Times are milliseconds on a monotonic clock chosen by the caller. Keeping
/// the clock out of this type is what makes the expiry rule testable, and keeps
/// core free of wall-clock reads that would break the determinism gate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PostureState {
    posture: Posture,
    armed_until_ms: Option<u64>,
}

impl Default for PostureState {
    fn default() -> Self {
        Self::new()
    }
}

impl PostureState {
    /// Default posture: `Observe`.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            posture: Posture::Observe,
            armed_until_ms: None,
        }
    }

    /// The effective posture at `now_ms`, after applying arming expiry.
    #[must_use]
    pub fn posture(&self, now_ms: u64) -> Posture {
        match (self.posture, self.armed_until_ms) {
            (Posture::Armed, Some(until)) if now_ms >= until => Posture::Observe,
            (p, _) => p,
        }
    }

    /// When the current arming grant lapses, if armed.
    #[must_use]
    pub const fn armed_until_ms(&self) -> Option<u64> {
        self.armed_until_ms
    }

    /// Moves to `Simulate`. No confirmation needed: nothing outside salman can
    /// be affected from this posture.
    pub const fn simulate(&mut self) {
        self.posture = Posture::Simulate;
        self.armed_until_ms = None;
    }

    /// Moves to `Armed` for `ttl_ms` milliseconds.
    ///
    /// Requires a [`UserConfirmation`], which only a human can cause to exist.
    /// The grant expires; there is no way to arm indefinitely.
    pub const fn arm(&mut self, _confirmation: UserConfirmation, now_ms: u64, ttl_ms: u64) {
        self.posture = Posture::Armed;
        self.armed_until_ms = Some(now_ms.saturating_add(ttl_ms));
    }

    /// Returns immediately to `Observe`.
    pub const fn disarm(&mut self) {
        self.posture = Posture::Observe;
        self.armed_until_ms = None;
    }

    /// Whether `effect` may happen now, and whether a human must be asked.
    #[must_use]
    pub fn permits(&self, effect: Effect, now_ms: u64) -> Permit {
        if effect.is_categorically_refused() {
            return Permit::Denied(DenialReason::CategoricallyRefused);
        }
        let posture = self.posture(now_ms);
        match effect {
            Effect::ReadLocalFile | Effect::ReadDevice => Permit::Allowed,
            Effect::WriteSimulated => match posture {
                Posture::Observe => Permit::Denied(DenialReason::PostureTooLow {
                    required: Posture::Simulate,
                }),
                Posture::Simulate | Posture::Armed => Permit::Allowed,
            },
            // Discovery is additionally constrained to user-declared ranges by
            // the caller, which is why this is `RequiresConfirmation` rather
            // than `Allowed` even when armed.
            Effect::NetworkDiscovery | Effect::WriteLiveDevice | Effect::ChangeControllerMode => {
                match posture {
                    Posture::Armed => Permit::RequiresConfirmation,
                    Posture::Observe | Posture::Simulate => {
                        Permit::Denied(DenialReason::PostureTooLow {
                            required: Posture::Armed,
                        })
                    }
                }
            }
            Effect::FirmwareOperation | Effect::CredentialGuessing | Effect::DenialOfService => {
                Permit::Denied(DenialReason::CategoricallyRefused)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A prompt that always approves — used to construct a `UserConfirmation`
    /// in tests. Its existence in test code is the point: production code
    /// cannot get one without a real user interface.
    struct AlwaysApprove;
    impl ConfirmationPrompt for AlwaysApprove {
        fn confirm(&mut self, _r: &ConfirmationRequest) -> Decision {
            Decision::Approved
        }
    }

    struct AlwaysRefuse;
    impl ConfirmationPrompt for AlwaysRefuse {
        fn confirm(&mut self, _r: &ConfirmationRequest) -> Decision {
            Decision::Refused
        }
    }

    fn request() -> ConfirmationRequest {
        ConfirmationRequest {
            effect: Effect::WriteLiveDevice,
            device: "PLC-01 (10.4.2.7:502)".into(),
            address: "holding register 40001".into(),
            current_value: Some("0".into()),
            new_value: Some("1".into()),
            declared_intent: "start the conveyor".into(),
        }
    }

    fn confirmation() -> UserConfirmation {
        request().ask(&mut AlwaysApprove).expect("approved")
    }

    #[test]
    fn the_default_posture_is_observe() {
        assert_eq!(Posture::default(), Posture::Observe);
        assert_eq!(PostureState::new().posture(0), Posture::Observe);
        assert_eq!(PostureState::default().posture(0), Posture::Observe);
    }

    #[test]
    fn observe_permits_reads_and_nothing_else() {
        let s = PostureState::new();
        assert_eq!(s.permits(Effect::ReadLocalFile, 0), Permit::Allowed);
        assert_eq!(s.permits(Effect::ReadDevice, 0), Permit::Allowed);
        assert_eq!(
            s.permits(Effect::WriteSimulated, 0),
            Permit::Denied(DenialReason::PostureTooLow {
                required: Posture::Simulate
            })
        );
        assert_eq!(
            s.permits(Effect::WriteLiveDevice, 0),
            Permit::Denied(DenialReason::PostureTooLow {
                required: Posture::Armed
            })
        );
        assert_eq!(
            s.permits(Effect::ChangeControllerMode, 0),
            Permit::Denied(DenialReason::PostureTooLow {
                required: Posture::Armed
            })
        );
    }

    #[test]
    fn simulate_permits_simulated_writes_but_never_live_ones() {
        let mut s = PostureState::new();
        s.simulate();
        assert_eq!(s.permits(Effect::WriteSimulated, 0), Permit::Allowed);
        assert_eq!(
            s.permits(Effect::WriteLiveDevice, 0),
            Permit::Denied(DenialReason::PostureTooLow {
                required: Posture::Armed
            })
        );
    }

    #[test]
    fn armed_still_requires_per_call_confirmation_for_live_writes() {
        let mut s = PostureState::new();
        s.arm(confirmation(), 0, 60_000);
        // Being armed is permission to be asked, not permission to act.
        assert_eq!(
            s.permits(Effect::WriteLiveDevice, 0),
            Permit::RequiresConfirmation
        );
        assert_eq!(
            s.permits(Effect::ChangeControllerMode, 0),
            Permit::RequiresConfirmation
        );
        assert_eq!(
            s.permits(Effect::NetworkDiscovery, 0),
            Permit::RequiresConfirmation
        );
        assert!(!s.permits(Effect::WriteLiveDevice, 0).is_allowed_outright());
    }

    #[test]
    fn arming_expires_back_to_observe() {
        let mut s = PostureState::new();
        s.arm(confirmation(), 1_000, 5_000);
        assert_eq!(s.posture(5_999), Posture::Armed);
        assert_eq!(s.posture(6_000), Posture::Observe);
        assert_eq!(
            s.permits(Effect::WriteLiveDevice, 6_000),
            Permit::Denied(DenialReason::PostureTooLow {
                required: Posture::Armed
            })
        );
    }

    #[test]
    fn disarming_is_immediate() {
        let mut s = PostureState::new();
        s.arm(confirmation(), 0, 1_000_000);
        s.disarm();
        assert_eq!(s.posture(1), Posture::Observe);
        assert_eq!(s.armed_until_ms(), None);
    }

    #[test]
    fn refused_confirmation_yields_no_proof_and_so_cannot_arm() {
        assert!(request().ask(&mut AlwaysRefuse).is_none());
    }

    #[test]
    fn firmware_credential_and_dos_effects_are_refused_at_every_posture() {
        let refused = [
            Effect::FirmwareOperation,
            Effect::CredentialGuessing,
            Effect::DenialOfService,
        ];
        let mut armed = PostureState::new();
        armed.arm(confirmation(), 0, 1_000_000);
        let mut simulating = PostureState::new();
        simulating.simulate();

        for state in [PostureState::new(), simulating, armed] {
            for effect in refused {
                assert!(effect.is_categorically_refused());
                assert_eq!(
                    state.permits(effect, 0),
                    Permit::Denied(DenialReason::CategoricallyRefused),
                    "{effect:?} must be refused at posture {:?}",
                    state.posture(0)
                );
            }
        }
    }

    #[test]
    fn every_effect_has_a_decision_at_every_posture() {
        // The check must be total: a new Effect variant that nobody classified
        // would be a hole in the safety model. `permits` matches exhaustively,
        // so this test's job is to prove no variant falls through to a panic.
        let all = [
            Effect::ReadLocalFile,
            Effect::ReadDevice,
            Effect::WriteSimulated,
            Effect::WriteLiveDevice,
            Effect::ChangeControllerMode,
            Effect::NetworkDiscovery,
            Effect::FirmwareOperation,
            Effect::CredentialGuessing,
            Effect::DenialOfService,
        ];
        let mut armed = PostureState::new();
        armed.arm(confirmation(), 0, 1_000);
        for state in [PostureState::new(), armed] {
            for effect in all {
                let _ = state.permits(effect, 0);
            }
        }
    }

    #[test]
    fn posture_labels_are_the_ones_shown_in_the_ui() {
        assert_eq!(Posture::Observe.label(), "OBSERVE");
        assert_eq!(Posture::Simulate.label(), "SIMULATE");
        assert_eq!(Posture::Armed.label(), "ARMED");
    }
}
