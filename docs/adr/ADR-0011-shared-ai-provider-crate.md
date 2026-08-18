# ADR-0011: The AI provider abstraction is a shared crate

- **Status**: Accepted; not yet implemented
- **Date**: 2026-08-18
- **Deciders**: the salman authors

## Context

salman has no AI layer. At 0.0.1 there is no provider abstraction, no adapter, no
key store and no crate in this workspace that talks to any model. This document records a
decision about how that work will be structured when it is done, and nothing here
describes code that exists.

The project owner also maintains a flight-control platform, and both projects need the
same thing. The list is not vague:

* one internal message and tool-call schema, independent of any vendor's wire format;
* an adapter per provider, translating that schema both ways;
* streaming, so a long response is visible as it arrives rather than after it finishes;
* budgets, so a run cannot spend without a stated ceiling;
* an audit log, so what was asked and what came back can be reconstructed afterwards;
* a secret store backed by the platform keychain, so keys are not in a dotfile.

The hard part is the tool-call adapter. Every vendor encodes tool calls differently — in
the shape of the declaration, in how a call is emitted mid-stream, in how a result is
returned, in what happens when the model calls two tools at once, and in what an error
looks like. That work is intricate, it is where the bugs live, and it is worth doing once.

Doing it twice means two sets of those bugs, found at different times, fixed differently,
in two repositories that will not benefit from each other's findings.

## Decision

The multi-provider AI abstraction will be a **shared crate**, consumed both by salman and
by the owner's flight-control platform, rather than duplicated in each repository. The
internal schema, the per-provider adapters, streaming, budgets, the audit log and the
keychain-backed secret store live there.

The crate must **not** know about PLCs and must not know about flight control. It has no
type named for a controller, a scan, a tag, an airframe or a waypoint. Its vocabulary is
messages, tools, streams, budgets and secrets. The moment a domain concept leaks in, the
crate stops being shareable and becomes one project's helper that another project has an
awkward dependency on.

Each consuming project keeps its own domain layer: salman decides which tools an engineer
may expose, what an audit record has to contain for industrial work, and what the safety
posture permits (see ADR-0002). None of that belongs in the shared crate.

## Consequences

Two young designs become coupled to one release cadence. Neither project's AI layer has
been written, so the shared interface will be designed against requirements that are still
guesses, and it will be wrong in places. Changing it then means changing it for both.

A change needed by one project can destabilise the other. A refactor that the
flight-control platform needs urgently arrives in salman on the flight-control platform's
schedule, and vice versa. This is the ordinary cost of sharing code, and it is worth
stating rather than discovering: the coupling is the price of writing the tool-call
adapter once.

The crate must be versioned and published, or vendored, rather than shared by a filesystem
path. A `path` dependency pointing at a sibling directory builds on exactly one machine —
the owner's — and neither repository would build for anyone else, including CI. Whichever
route is chosen, it has to be chosen before the first commit that depends on the crate.

The domain-free rule needs an enforcement mechanism it does not have. A dependency
direction check, or simply a review rule that the crate's public API mentions no domain
noun, would do; without one, the leak happens gradually and is noticed only when the
second consumer tries to upgrade.

Until the crate exists, salman's capability registry has no entry for any of this, and
should not get one. A `Planned` entry is the right home for it once the crate has a name,
because a registry entry is a claim about a named thing.

## Open questions

The crate's **name and location are not yet fixed**. This document deliberately does not
invent either.

The flight-control repository's URL is **not recorded here, because it has not been
supplied**. It is not omitted for brevity and it should not be filled in from memory: a
URL that does not resolve fails `.github/workflows/docs-links.yml`, and an invented one
would be worse than the gap it fills. When the URL is supplied, it belongs in this section.

Also unresolved: which providers the first version supports, whether the audit log format
is shared or per-project, and whether the secret store is part of the same crate or a
second one — a keychain integration has a very different platform-support surface from a
protocol adapter and may not want the same release cadence.

## Alternatives considered

**Duplicate now, extract later.** The lowest-coupling option, and the honest one about how
little is known today: each project writes what it needs, the shared shape becomes obvious
from two working implementations, and the extraction is then informed rather than guessed.
It is a genuinely good argument. It lost because the usual outcome is that the extraction
never happens — by the time both implementations work, extracting them is a refactor with
no user-visible benefit, competing against features, and the tool-call adapter has been
debugged twice by then anyway.

**A git submodule.** Solves the sharing problem without publishing anything, and keeps the
code visible in both trees. Rejected: submodules are a persistent tax on everyone who
clones, and the tax is paid by contributors who did not choose it — a clone without
`--recursive` fails confusingly, updates need a second command that is easy to forget, and
the pinned commit shows up in diffs as an opaque hash.

**Copying the file between repositories.** Immediate, needs no infrastructure, and works
perfectly for about a month. Rejected: divergence is guaranteed. The first urgent fix goes
into whichever copy is in front of the person making it, and after that the two files are
similar rather than identical, which is the worst state to be in when debugging a
vendor-specific tool-call bug.

## How this is enforced

Nothing enforces this. There is no crate, no dependency, no test and no CI job. This
document is a record of intent, and until the shared crate exists and both repositories
depend on it, the only thing standing between this decision and a duplicated
implementation is that somebody reads this file first.
