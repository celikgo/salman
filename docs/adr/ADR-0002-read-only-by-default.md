# ADR-0002: Read-only by default — the posture model

- **Status**: Accepted
- **Date**: 2026-08-18
- **Deciders**: the salman authors

## Context

A tool that can read a controller is one small step away from being a tool that can write
to one. The step is small in code and enormous in consequence: the same function that
reads holding register 40001 writes it if you pass a value. Engineering tools in this
domain have historically treated that distinction as a matter of which menu item the user
clicked.

salman will eventually need to write. Commissioning, forcing a variable during a test, and
downloading a corrected value are all legitimate things an engineer does, so the question
is not whether the capability exists but what stands between it and an accident.

Two forces make the usual answer inadequate. First, salman is designed to be driven
headless, from a CLI and from scripts; anything that can be automated will be. A
permission model that consists of a flag in a configuration file is a permission model that
a shell script sets once and forgets. Second, some effects have no legitimate place in an
engineering tool at all, and treating them as merely privileged rather than absent would
make salman an attack tool with a warning dialog.

At version 0.0.1 there is no code path in salman that opens a socket, writes to a device or
changes a controller mode. Nothing calls this model yet. It was written first on purpose:
once a write path exists, the cost of retrofitting a gate in front of it is paid in
arguments, and the gate ends up with exceptions in it.

## Decision

salman has three postures and no fourth, defined in `crates/salman-core/src/posture.rs`.
`Observe` is the default and permits reads only. `Simulate` additionally permits writes to
devices simulated inside salman. `Armed` additionally permits writes to real devices, and
even then only with per-call human confirmation. `Posture::default()` is `Observe`, and
`PostureState::disarm` returns to it immediately.

Specifics:

- Every outward-facing operation classifies itself as exactly one `Effect`:
  `ReadLocalFile`, `ReadDevice`, `WriteSimulated`, `WriteLiveDevice`,
  `ChangeControllerMode`, `NetworkDiscovery`, `FirmwareOperation`, `CredentialGuessing`,
  `DenialOfService`. `PostureState::permits` matches on that enum exhaustively, so the
  check is total rather than a list of special cases somebody remembered to write.
- `permits` returns `Permit::Allowed`, `Permit::RequiresConfirmation` or
  `Permit::Denied(DenialReason)`. Being armed yields `RequiresConfirmation` for live writes,
  mode changes and discovery: arming is permission to be asked, not permission to act.
- Reaching `Armed` requires a `UserConfirmation`. That type has a private field and no
  public constructor. The only way to obtain one is `ConfirmationRequest::ask`, which takes
  a `&mut dyn ConfirmationPrompt` — something that can actually put the question to a
  person. An automated caller cannot manufacture consent, because it cannot construct the
  proof that consent happened. An agent must be given a prompt; it cannot be one.
- Arming expires. `PostureState::arm` takes a `now_ms` and a `ttl_ms` and records an
  `armed_until_ms`; `PostureState::posture(now_ms)` reports `Observe` once that instant
  passes. There is no way to arm indefinitely. Time is passed in rather than read, which
  keeps the expiry rule testable and keeps `salman-core` free of the wall-clock reads
  [ADR-0005](ADR-0005-determinism.md) forbids.
- `FirmwareOperation`, `CredentialGuessing` and `DenialOfService` are refused at every
  posture by `Effect::is_categorically_refused`. They are refused in code and are not
  configuration options. Enabling them would be a change of purpose, not a feature.

## Consequences

Every future outward-facing operation must classify itself as exactly one `Effect` before
it can be written. That is deliberate friction and it will be felt: adding a protocol means
deciding, in advance and in public, what each of its operations does to the plant. Some
operations will not fit an existing variant cleanly, and the honest response to that is a
new variant and a reviewed change to `permits`, not a convenient reuse of `ReadDevice`.

The confirmation dialog has an expensive shape. `ConfirmationRequest` carries the device
identity, the address, the current value, the proposed new value and the caller's declared
intent. Every one of those fields costs the caller work — the current value in particular
usually means an extra read before the write. It is required anyway, because a confirmation
that omits the current value is one nobody can act on: an engineer cannot approve setting a
setpoint to 40 without knowing whether it is currently 39 or 4.

Expiry will annoy people. An engineer part way through a commissioning session will be
returned to `Observe` and have to arm again, and the temptation to configure a very long
TTL will be real.

The categorical refusals close off legitimate uses. A firmware integrity check is a
reasonable thing for a diagnostic tool to want, and salman cannot do it, because
`FirmwareOperation` covers reading firmware as well as writing it. The line is drawn wide
on purpose; a narrower line would need a judgement about intent that code cannot make.

The model is currently unexercised. Nothing calls `permits`, so its tests prove that the
logic is right, not that it is reachable. Until a write path exists, the guarantee this
ADR describes is a guarantee about a module, not about the program.

## Alternatives considered

**A boolean `allow_writes` flag.** The conventional answer, and much less code. It was
rejected for three reasons, each fatal on its own: it has no expiry, so it is set once and
stays set; it has no per-call confirmation, so approving one write approves every
subsequent one; and it cannot distinguish a write to a simulated device from a write to a
real one, which collapses exactly the distinction an engineer relies on when testing.

**Capability tokens issued per device.** A token naming one device and one address range,
handed to the code that may act on it. More precise than a posture, and not rejected on the
merits: it is more machinery than v0.1 needs, and building token issuance, scoping and
revocation before there is a single device to issue a token for would be designing against
an imagined API. Worth revisiting when the protocol layer arrives.

**Documentation and code review.** Write the rule down, review against it. This is the
default in most projects and it is the reason the rule is worth structuring instead. Review
catches what a reviewer thinks to look for, and the failure mode here — a write path
nobody noticed was a write path — is precisely the one review is worst at. This is the
class of thing that has to be structural, or it is nothing.

**Refusing to write at all, ever.** Tempting, and it would make this ADR much shorter. It
was rejected because it does not remove the risk, it relocates it: engineers who need to
write would use a vendor tool with no posture model at all for that step, and salman would
have improved nothing except its own conscience.

## How this is enforced

The tests in `crates/salman-core/src/posture.rs`, cited as evidence under the capability
identifier `core.posture` in `crates/salman-core/src/capability.rs`. The registry names
three of them:

- `the_default_posture_is_observe`
- `firmware_credential_and_dos_effects_are_refused_at_every_posture`
- `armed_still_requires_per_call_confirmation_for_live_writes`

Two tests in `crates/salman-core/src/capability.rs` make those citations real:
`tested_capabilities_must_cite_evidence` and
`every_cited_test_exists_in_the_source_tree`. Deleting or renaming one of the posture tests
fails the build of `salman-core`, not merely a CI job.

Four further tests in `posture.rs` cover the rest of the model:
`observe_permits_reads_and_nothing_else`,
`simulate_permits_simulated_writes_but_never_live_ones`, `arming_expires_back_to_observe`,
and `refused_confirmation_yields_no_proof_and_so_cannot_arm`.

The `UserConfirmation` constructor rule is enforced by the language rather than by a test:
the type's only field is private, so no code outside `posture.rs` can build one. That the
test module has to go through an `AlwaysApprove` prompt to get one is the demonstration.

Nothing enforces the classification rule for code that does not exist yet. When the first
outward-facing operation is written, the enforcement that it calls `permits` will be review
plus the fact that there is no other way to obtain a `UserConfirmation`.
