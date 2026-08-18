# ADR-0014: salman reads live equipment and does not drive it

- **Status**: Accepted
- **Date**: 2026-08-18
- **Deciders**: the salman authors

## Context

v0.2 joins two things that had not met before: a Structured Text program running on salman's
scan runtime, and a Modbus device on a real socket. An IO mapping binds a device's registers
to the process image, so a program can declare `Level AT %IW0` and read what the device
holds.

Once inputs flow in, outputs flowing out is a small step in code and a very large one in
meaning. A program that reads `%IW0` and writes `%QX0.0` is a controller. If `%QX0.0` reaches
a real coil, salman is controlling a plant.

The posture model already answers the question it was designed for. An **engineering write**
is one value, to one register, once, because a person decided to: `Client::write` requires
the ARMED posture and takes a `UserConfirmation` by value, so it authorises exactly that
call.

A **control loop** is a different thing wearing the same clothes. It writes its outputs every
scan, for ever. There is no way to confirm each one, and a tool that asked once and then
wrote ten thousand times would have converted a per-call confirmation into a session-wide
licence to drive a plant — while still being able to say, truthfully and uselessly, that a
human approved.

## Decision

**Output mappings run against a simulated device only. Against live equipment, salman reads
and does not write.**

`salman_link::Link` takes an explicit `Peer` — `Simulated` or `Live` — and refuses at
construction to hold an output mapping against a live peer. The refusal is repeated at the
call that would perform the write, because the check that matters is the one next to the
action. It is `LinkError::WouldDriveALiveDevice`, and its message says what it says here:
there is no posture, flag or configuration key that enables it.

`Peer` is a value the caller supplies, because **there is nothing in a socket that says
whether the thing on the other end is real**. salman cannot detect this and does not pretend
to; it makes the caller state it, and the project file will have to carry it explicitly when
the command line grows a `run` that uses devices.

## Consequences

**salman cannot be used as a soft PLC, and that is the intent.** A tool that drives a plant's
outputs needs a watchdog, a defined failsafe state, deterministic scheduling under fault, and
an assessment against IEC 61508 or IEC 62061. salman has none of these and is not seeking
them — `LEGAL.md` says so and this decision is what makes that statement true in code rather
than only in prose.

**Hardware-in-the-loop testing is asymmetric**, and the asymmetry is deliberate: salman may
read a real device's inputs into a simulated program, which is the useful and safe half. It
may not close the loop back onto real outputs.

**The simulator is what output mappings are for.** A test drives both ends — a program, a
device salman is pretending to be, and the mapping between them — and every part of it runs
in CI with no hardware anywhere. That is the case v0.2 set out to make work, and it is
unaffected by this decision.

**If this is ever revisited**, it is not a configuration change. It is a change of what
salman is, and it needs a new ADR that supersedes this one, an answer to the watchdog and
failsafe questions, and a reason better than "a user asked".
