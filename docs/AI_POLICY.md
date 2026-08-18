# AI policy

Two separate subjects, kept separate on purpose.

**Part 1** is about how salman is developed. It describes something that is true today.

**Part 2** is about the AI layer salman intends to ship. That layer does not exist at 0.0.1
— there is no agent, no model client, no prompt, and no network code of any kind in this
repository — so Part 2 is written entirely in the future tense. It is a statement of
policy, not of capability.

---

## Part 1 — How salman is developed

**salman is developed with AI assistance.** That is stated plainly at the top rather than
buried, because a reader is entitled to know it before they decide how much of this project
to trust.

What follows is not a defence of the method. It is a description of the checks that exist in
this repository today, on the grounds that a method is only as defensible as what a stranger
can verify without taking anybody's word for it.

### A capability cannot call itself tested

`crates/salman-core/src/capability.rs` holds the registry every status claim salman
publishes is generated from — the README table, the status document, the CLI's status
output. A capability marked *implemented and tested* must name the tests that test it, and
two tests in that module enforce it:

- `tested_capabilities_must_cite_evidence` — a capability claiming the tested status
  with an empty evidence list fails the build.
- `every_cited_test_exists_in_the_source_tree` — every cited test must exist, at the named
  path, spelled exactly that way.

The consequence is the useful part: deleting a test fails the build of the crate that claims
it. A status table cannot drift away from the code, because it is not written by hand.

The registry also records what is *not* claimed. Lexer fuzzing sits at *implemented,
untested* with an empty evidence list and a note saying why: a fuzzing run demonstrates that
nothing was found, which is not the same as demonstrating that anything is right. That row
is a small thing, and it is the clearest example of the rule working, because the easy move
was to tick it.

### Uncertainty about the standard is a field, not a footnote

Where a language behaviour comes from IEC 61131-3, the test that asserts it cites the
clause, through a `ClauseRef` declared in `crates/salman-core/src/clause.rs` rather than a
string typed into a test somewhere.

IEC 61131-3 is paywalled. Clause numbers have been cross-checked against public sources —
the publisher's own free front-matter preview, which carries the contents and the lists of
tables and figures, and vendor implementer compliance statements, which enumerate the
feature tables. Where a number could not be checked, the citation carries
`Provenance::NumberUnconfirmed`, and `Display` appends `[clause number unconfirmed]` to it
wherever it is rendered. The generated `IEC_CITATIONS.md` lists which is which, and a test
fails if the committed document and the registry disagree.

The design decision worth naming: **how far a citation could be verified is a field on the
citation type**, so it travels with the citation into every diagnostic, document and test
name. It is not a caveat in a preface that a reader might not reach. A confidently wrong
citation is worse than no citation, and this is the mechanism that stops one being produced.

The same type carries the `requirement` field, which is a paraphrase in salman's own words
of the behaviour being tested. No IEC text is reproduced. See `../LEGAL.md` for the full
citation policy.

### The awkward cases are asserted, not hidden

`crates/salman-vm/src/stdfb.rs` implements the standard function blocks, and the tests
assert the edge cases that a tidier implementation would quietly smooth over.

- `a_fresh_f_trig_emits_one_spurious_pulse_with_its_clock_low`. A fresh `F_TRIG` instance,
  called with its clock low, reports a falling edge that never happened, because its
  internal memory has no initialiser and the output is a function of that memory. The test
  asserts the pulse. The comment records that this is Edition 2 text believed unchanged in
  Edition 3, that salman could not read the Edition 3 page, that a technical report is
  reported to recommend the opposite behaviour, and that at least one vendor implements the
  report instead. salman follows IEC 61131-3 and says so, and marks the point unverified.
- `a_fresh_tof_with_its_input_low_does_not_start_an_off_delay`. `TOF`'s analogue of the same
  trap, in the other direction. Getting it wrong makes every program using a `TOF` emit a
  phantom pulse at start-up.

The module header states the larger uncomfortable fact rather than leaving it to be
discovered: the standard supplies no body at all for the timers — they are defined by
timing diagrams — so every timer test is a trace of inputs against outputs, not a
comparison against a body salman does not have. It also states that `SEMA` is not a
standard function block, is shipped because existing code uses it, and is never described
as standard.

### Parsers are fuzzed

Rule 7 of the project is that untrusted input is treated as hostile. Four libFuzzer targets
in `fuzz/fuzz_targets` run against the Structured Text lexer daily in CI, and they assert
postconditions — exactly one end-of-file token, non-decreasing spans inside the source,
every literal and address index resolving — rather than only that nothing panicked.

Coverage today is the lexer. The parser and every decoder salman later grows are not fuzzed
yet. The sentence "every parser will be fuzzed in CI" is a commitment with a workflow behind
it, and at 0.0.1 it is partly kept, which is what the capability registry says.

### What this adds up to

AI assistance is a method. It is not a licence to claim more than the tree supports, and it
is not an excuse when a claim turns out to be wrong.

The check on it is that **a stranger can verify the output**. Every one of the mechanisms
above is a way of making a claim falsifiable by somebody who does not trust us: the status
table is generated from tests that must exist, the citations carry their own provenance, the
edge cases are asserted where they can be read, the fuzzers assert properties rather than
absence of crashes, and the determinism gate means a result that cannot be reproduced is a
failure rather than a variation.

None of that is a claim that AI-assisted code is correct. It is the arrangement that makes
incorrectness visible, which is the only property that survives contact with a reader who is
right to be sceptical.

---

## Part 2 — The AI layer salman will ship

**None of this exists at 0.0.1.** There is no agent, no model client, no key handling, no
audit log, and no outbound network code. What follows is the policy the layer will be built
under, recorded now so that it constrains the first implementation rather than being written
around it afterwards.

### Models and keys

- **Bring your own key, or your own local model.** salman will not ship a hosted service, a
  proxy, or a bundled key. The user supplies either a provider API key or the endpoint of a
  model they run themselves.
- **Secrets live in the operating system keychain.** Never in a configuration file, never in
  a project file, never in an environment variable committed anywhere, and never in a log
  line. A key that appears in a support bundle is a key that has been disclosed.
- **Offline mode hard-fails.** In offline mode, any outbound request to anything other than
  the configured local endpoint fails, loudly, rather than being retried, downgraded, or
  silently permitted. "Hard-fails" is the operative word: a mode that degrades to working is
  not a mode.

### Auditability

- **Every tool call is written to a structured audit log**: what was called, with what
  arguments, at what time, against what target, and what came back. Structured rather than
  prose, so that it can be queried and diffed instead of read hopefully.
- Model-generated output is recorded as model-generated. See the honesty note below.

### The posture model applies to the agent exactly as it applies to a human

The agent gets no privileged path. It is subject to the same `Effect` classification and the
same `PostureState` checks described in `../SECURITY.md`, with no exemption and no separate
code path.

**The agent can never arm itself.** This is already structural in
`crates/salman-core/src/posture.rs` rather than something that will need to be added:
`PostureState::arm` requires a `UserConfirmation`, a type with no public constructor, and
the only way to obtain one is to put a fully described request to a human through a
`ConfirmationPrompt` and have them approve it. An agent must be *given* a prompt; it cannot
*be* one. An automated caller cannot manufacture consent by constructing the value.

The categorical refusals apply to the agent identically. There is no posture, no
configuration and no prompt that lets an agent perform a firmware operation, guess a
credential, or degrade a device.

### The terms-of-service rule

salman will never implement, document, or link to a path that:

- drives a consumer chat subscription through unofficial or reverse-engineered endpoints;
- scrapes a web session belonging to a provider's consumer product; or
- replays browser cookies to authenticate against a provider.

Where a provider publishes no third-party OAuth flow, integration with that provider is
**bring-your-own-key only**, and if they publish no API key route either, salman does not
integrate with them.

This is not a hedge about enforcement risk. A tool that tells industrial engineers to
respect the boundaries of systems they do not own has no business routing around somebody
else's terms of service to save a user a subscription line item.

---

## Honesty note: the opposite position is legitimate

At least one open-source PLC static-analysis project has staked the opposite ground
deliberately: fully deterministic, explicitly free of language models, and aimed squarely at
regulated industries where every finding has to be explainable and reproducible from rules a
reviewer can read.

That is a legitimate position and salman should say so rather than argue past it. In a
validated environment, a finding you cannot trace to a rule is a finding you cannot defend
in an audit, and "the model said so" is not a rule. A project that refuses model-generated
analysis outright is buying a property that salman, by shipping an agent at all, will not
have in full.

(No project is named here. We could not confirm from a primary source which project holds
that stated position, and attributing a stance to a named project on the strength of a
recollection is the same error this document spends Part 1 building machinery against.)

What salman commits to instead, when an agent ships: **every finding will state whether it
is deterministic or model-generated, per finding**, in the same output, without the reader
having to ask. A deterministic finding names the rule and the clause it came from. A
model-generated finding is labelled as model-generated, and is a suggestion to be checked
rather than a result to be relied on.

A tool that mixes the two without labelling them has taken its deterministic findings' one
genuine advantage — that they are checkable — and spent it.
