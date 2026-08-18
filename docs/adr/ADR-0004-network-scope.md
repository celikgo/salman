# ADR-0004: The network layer's scope boundary

- **Status**: Accepted (not yet implemented)
- **Date**: 2026-08-18
- **Deciders**: the salman authors

## Context

Nothing described in this ADR exists at version 0.0.1. salman has no network model, no
channel model, no profile files and no federation interface. This document draws a line
before the work starts, because the line is much harder to draw afterwards.

The question engineers actually bring to a network model is narrow and practical. A control
loop that closes over a wireless link behaves differently from one that closes over a
cable, and the difference is not a single number. Latency has a distribution; loss comes in
bursts; packets arrive out of order; a queue that is full adds delay that depends on what
else is on the link. An engineer wants to know whether their loop still works, and they
want to know within a coffee break.

The pressure, once a project has any network model at all, is to keep adding fidelity until
the model becomes a protocol implementation. That path ends with a project that has spent
years reimplementing a 5G stack badly, alongside people who do it properly, and whose
control-engineering value has stalled. It also ends with a tool whose output people mistake
for a conformance result.

There is also a claim boundary that matters legally and professionally. A number produced
by a model parameterised from a published profile is not a measurement of the user's
network, and it is not a statement about anyone's conformance to anything.

## Decision

salman models the *effect* of a network on a control loop. It does not implement a radio
access network, a 5G core, or any protocol stack below the modelled channel.

Specifics:

- Channel models are stochastic. The parameters are one-way delay distribution, jitter,
  loss probability, burst correlation, reordering, duplication, and bandwidth with a
  queueing model. That set is chosen because it is what determines whether a control loop
  survives; it is not chosen to resemble any particular technology's internals.
- Channel models are parameterised from published profiles. Every profile file states its
  source document and explicitly marks which numbers are citations and which are
  assumptions made by salman's authors. A profile with an uncited number that is not
  labelled as an assumption is a defect in the profile.
- For packet-level fidelity, salman federates with established simulators — such as
  [ns-3](https://www.nsnam.org/) or [OMNeT++](https://omnetpp.org/) — rather than
  reimplementing what they already do properly.
- Every network result carries a banner stating that it is a model parameterised from a
  cited profile: not a measurement of the user's network, and not a conformance statement.
  The banner is part of the result, not a footnote in the documentation.

The line, stated so that it can be quoted: salman answers "what does this network do to my
control loop", not "is this stack conformant".

## Consequences

salman can never certify anything about a network. Not conformance, not compliance, not
suitability for a particular application. If a user needs a certifiable answer they need a
laboratory, and salman's output must not be presented in a document that implies otherwise.

A user who needs packet-level truth has to federate or measure. Federation means installing
and configuring an external simulator, which is a substantially higher barrier than running
salman, and it means the fast answer and the accurate answer come from different tools with
different setup costs. Some users will take the fast answer when they needed the accurate
one, and the banner is the only thing standing between them and that mistake.

Profiles age. Published sources are revised, technologies deploy differently in practice
than in their specifications, and a profile that was well-sourced in 2026 may be misleading
in 2030. Every profile therefore carries its source and the date it was compiled, and a
profile whose source has been superseded is a maintenance obligation rather than a stable
asset. Bodies such as [5G-ACIA](https://5g-acia.org/) publish the kind of industrial
profile material salman will draw on, and that material is revised.

Marking assumptions explicitly will make the profiles look weaker than a competitor's
undocumented numbers. A profile that honestly says "burst correlation: assumed, no public
source" reads worse than one that simply states a figure. That is the correct trade and it
will cost salman credibility with readers who do not look closely.

The stochastic model cannot reproduce effects that depend on protocol state — a
retransmission timer interacting with a scan period, or a handover that stalls a specific
flow. Those are real control problems and salman will get them wrong. Federation is the
answer, and it is a partial one.

## Alternatives considered

**Implementing a 5G stack.** Rejected as absurd for this project. It is years of work by
specialists, it would be wrong in ways nobody in this project could check, and a
plausible-looking wrong stack is worse than no stack because it produces numbers people
believe. The organisations that do this properly already exist, and salman's contribution
is not a second-rate copy of their work.

**Shipping only a fixed-delay model.** By far the simplest, easy to explain, and impossible
to misuse for a conformance claim. It lost because it is too weak to answer the question
anyone actually asks. A fixed delay tells an engineer nothing about the tail, and the tail
is where control loops fail. A model that cannot represent a burst of loss cannot represent
the failure mode it exists to predict.

**Always requiring an external simulator.** The most defensible position technically:
salman would never produce a network number of its own, and federation would be the only
path. It lost on time to answer. The value of this feature is that an engineer can ask
"does my loop survive this link?" and get an answer in ten seconds while they are still
thinking about the question. A workflow that starts with installing ns-3 is a workflow most
users will not start.

**Measuring the user's real network instead of modelling it.** Genuinely useful, and much
more truthful when it applies. It lost because it answers a different question: it tells you
what your network did, not what it would do under a link you have not built yet, which is
the question asked during design. It is also outside the read-only-by-default posture for
anything beyond passive capture; see [ADR-0002](ADR-0002-read-only-by-default.md). This
remains a reasonable future capability alongside the model, not instead of it.

## How this is enforced

Nothing enforces this today. There is no network code, no profile format and no CI job that
could check a profile that does not exist. The boundary is currently held by this document,
by the "What this is NOT" section of `README.md`, and by review.

When the network layer arrives, the enforcement will be:

- Every profile file carries a `citation` field naming its source document and date, and a
  per-parameter marker distinguishing cited values from assumptions.
- A CI check that fails when any profile's `citation` field is empty or missing, and when
  any parameter is neither cited nor explicitly marked as an assumption.
- A test that every rendered network result carries the model banner, so that removing the
  banner fails the build rather than passing quietly.

Until those exist, salman must publish no network result of any kind.
