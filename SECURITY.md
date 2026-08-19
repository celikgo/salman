# Security policy

Version 0.1.0. Read the first two sections before the reporting section: what salman refuses
to do is a larger part of its security posture than what it does.

---

## What salman is for

salman is an engineering and diagnostic tool. It is built for equipment and networks that
the person running it owns, or is authorised by the owner to work on.

That is a statement about purpose, and purpose is what the design is built around. salman
does not attempt to determine whether you are authorised — it cannot, and a tool that
claimed to would be lying. What it does instead is decline to carry the capabilities that
are only useful without authorisation, so that the tool is not a convenient starting point
for somebody who lacks it.

---

## Capabilities that are deliberately absent

The following are not implemented, and will not be implemented:

- **Network discovery outside user-declared ranges.** Discovery is permitted only inside
  address ranges a person has explicitly declared as theirs. An address outside them is
  refused with `OutsideDeclaredScope`.
- **Credential guessing.** Trying candidate credentials against a device.
- **Exploitation.** salman has no exploit code and takes none as input.
- **Fuzzing of live equipment.** salman fuzzes its own parsers, on its own inputs, in CI.
  It does not point a fuzzer at a controller.
- **Denial of service.** Anything whose purpose or predictable effect is to degrade a device
  or a network: flooding, malformed-frame storms, resource exhaustion.
- **Firmware manipulation.** Reading, writing or erasing device firmware.

### These are refused in code, not behind a flag

The refusal lives in `crates/salman-core/src/posture.rs`. Firmware operations, credential
guessing and denial of service are classified by `Effect::is_categorically_refused`, and
`PostureState::permits` returns `Denied(CategoricallyRefused)` for them at **every**
posture, including the highest one. There is no configuration option, no environment
variable and no build feature that changes that answer. A test named
`firmware_credential_and_dos_effects_are_refused_at_every_posture` fails if one is ever
added.

This is a deliberate difference from "off by default". A default can be flipped by a
configuration file somebody else wrote. Adding a switch here would not be adding a feature;
it would be changing what the tool is.

### The rest of the posture model

Everything that can reach outside salman's own process is classified as exactly one
`Effect`, so the check is total rather than a list of special cases somebody remembered to
write. Three postures govern them, and there is no fourth:

| Posture | May affect |
|---|---|
| `Observe` | Nothing outside the process. Reads only. **The default, and the state everything returns to.** |
| `Simulate` | Devices simulated inside salman. Never a real one. |
| `Armed` | Real devices, and only with per-call confirmation. |

Two properties are worth stating explicitly because they are what make the model more than
a label:

- **Arming expires.** `PostureState::arm` takes a time-to-live and there is no way to arm
  indefinitely. Once the grant lapses the effective posture is `Observe` again, without
  anybody having to remember to disarm.
- **Arming cannot be self-granted.** `arm` requires a `UserConfirmation`, a type with no
  public constructor. The only way to obtain one is to put a described request to a human
  through a `ConfirmationPrompt` and have them approve it. An automated caller —
  including a future agent — must be given a prompt; it cannot be one.

That is no longer hypothetical. **salman opens sockets and can write to a device.**
`salman-modbus-net` connects with `TcpStream::connect_timeout`, listens with
`TcpListener::bind` for its simulator, and `Client::write` performs a live write. It is the
first caller of `permits`, and it went through the posture model because the model was there
before it — which was the point of writing the model first.

What that path does, precisely: `Client::write` calls
`posture.permits(Effect::WriteLiveDevice, ..)` and refuses at anything below ARMED, and it
takes a `UserConfirmation` **by value**, so one confirmation authorises exactly one write
and cannot be retained for the next. `UserConfirmation` has no public constructor and can
only be obtained by asking a person, so an automated caller cannot manufacture one. Reads
need no permission: that is what read-only by default means.

`Client::write_simulated` is deliberately *not* posture-gated, because nothing on the other
end is real. Nothing in a socket says whether a peer is simulated, so the caller carries that
knowledge: `salman_link::Link` takes an explicit `Peer` and refuses to run output mappings
against a live one. **No controller mode is changed by any code path here**, and no firmware
operation, credential guess or denial of service exists at any posture.

---

## Reporting a vulnerability

**salman has no published security contact address.**

That is the honest answer and it is stated rather than papered over. Inventing an address
that nobody monitors would be worse than having none: it would convert a reporter's good
faith into silence, and they would have no way to tell the difference between a mailbox
nobody reads and a report under consideration.

A contact route, and the disclosure timeline that goes with it, will be published here
before the first release. Until then, treat this project as having no coordinated
disclosure process. If you have found something and want to tell somebody now, use the
repository's public issue tracker and use your judgement about what to put in it.

---

## Supported versions

**None.**

| Version | Supported |
|---|---|
| 0.1.0 | No. Pre-alpha. |

0.1.0 is pre-alpha. There is no support commitment, no patch stream, and no backport
policy. Nothing in this repository should be deployed anywhere that a security response
would matter, and the absence of a support row is not an oversight to be read around.

---

## Captures, process data and redaction

When salman gains the ability to capture traffic, those captures will contain process data
and may contain credentials: industrial protocols commonly carry authentication material in
the clear, and a capture taken to debug a timing problem will pick it up along with
everything else. An exported bundle is exactly the artefact that gets attached to a ticket
and forwarded on.

The rule, for when that code exists: **redaction is on by default in exported bundles**, the
redaction rules are written down, and they are testable rather than asserted.

**Capture code now exists**, in `salman-capture`: it reads classic pcap files and decodes
Ethernet, IP and TCP. It is a *reader* — salman does not put an interface into promiscuous
mode and has no live-capture path — so the bytes it handles are ones the user already has.
There is still no export-bundle format, so there is nothing yet for the redaction rule above
to apply to; it is written down here so that the default is settled before the first bundle
is exported rather than afterwards.

---

## Supply chain

The dependency graph gets its own gate, because forbidding `unsafe` in salman's own code —
which the workspace does, with `unsafe_code = "forbid"` — says nothing at all about the
crates underneath it.

- **`cargo-deny` runs in CI**, on every push and pull request and on a daily schedule, from
  `.github/workflows/supply-chain.yml`. An advisory published today makes a graph that was
  clean yesterday dirty without a single commit, which is why it cannot only run on push.
  See <https://embarkstudios.github.io/cargo-deny/> and the advisory database at
  <https://rustsec.org/>.
- **The policy is an allowlist.** `deny.toml` lists the licences salman has considered; one
  it has not stops the build rather than entering quietly. Unmaintained crates are flagged
  anywhere in the graph, not just at the top level. Wildcard version requirements are
  denied. Unknown registries and git dependencies are denied.
- **Ignored advisories must carry a reason.** A bare advisory id records that somebody made
  a decision but not why, which makes the decision impossible to revisit. A CI step parses
  `deny.toml` and fails on any ignore entry without both an id and a stated reason.
- **`Cargo.lock` is committed**, and `.gitignore` says so explicitly so that nobody
  helpfully adds it. salman gates on bit-identical output across machines, which is only
  meaningful with a locked graph.
- **The toolchain is pinned** in `rust-toolchain.toml` to an exact version, with the same
  reasoning: a byte-for-byte trace comparison across Linux, macOS and Windows means nothing
  if the machines run different compilers. Bumping the pin is a reviewed change.
- **Parsers are fuzzed.** Six libFuzzer targets in `fuzz/fuzz_targets` run daily against the
  Structured Text front end, asserting postconditions rather than only that nothing panicked.
  Four cover the lexer — valid UTF-8, raw bytes, the strict dialect, and a differential run
  of both dialects — one covers the parser, and one covers lexing, parsing and semantic
  analysis together. The declarative test-file reader in `salman-test` is not fuzzed, and
  neither is any future protocol decoder. The capability registry records that coverage
  honestly rather than as a tick.

---

## IEC 62443, and what salman does and does not do about it

salman is **not assessed against IEC 62443**, by anyone, and no assessment is planned.
Nothing below is a conformance claim. It is a map of which concepts from that series the
architecture actually engages with, written so that a reader can see the size of the gap
rather than infer it.

The reference is `IEC 62443-3-3:2013 "Industrial communication networks - Network and system
security - Part 3-3: System security requirements and security levels" (Ed 1.0)`,
<https://webstore.iec.ch/en/publication/7033>.

**Concepts salman engages with, as a tool:**

- *Least functionality.* The categorical refusals above remove whole classes of capability
  from the tool rather than disabling them.
- *Use control, applied to the tool itself.* Nothing reaches a real device without an
  explicit, expiring, human-granted posture and a per-call confirmation that names the
  device, the address, the current value and the value to be written.
- *Restricted data flow.* Discovery is confined to ranges a person declared. There is no
  implicit scope.
- *Auditability.* Every effect is classified, every denial carries a reason fit to show a
  user, and traces are deterministic and fingerprinted, so a run can be reproduced and
  compared byte for byte.
- *Resource availability, as a discipline on salman's own code.* Untrusted input is treated
  as hostile: parsers are bounded and fuzzed, diagnostics are capped so hostile input cannot
  exhaust memory, oversized sources are refused rather than loaded, and a scan has an
  instruction budget so a runaway loop stops the task instead of hanging the process.

**Concepts salman does not address at all:**

- Identification and authentication of users. salman has no user model, no accounts and no
  roles.
- Anything cryptographic: confidentiality, integrity or authenticity of communications.
  salman implements no cryptography.
- Zone and conduit definition, network segmentation, or any control over the network it
  will eventually observe.
- Security levels, target or achieved, in the IEC 62443 sense. salman has no basis to claim
  one.
- Patch management, backup and restore, or any operational lifecycle requirement. Those are
  properties of a deployed system, and salman is not one.
- Compensating countermeasures, risk assessment, and every process requirement in the
  series. salman is a tool used by people who do that work; it does not do it.

The short version: salman borrows a handful of ideas from IEC 62443 about how a tool that
touches control systems ought to behave, and it addresses none of the requirements the
series places on a system. Those are different things, and conflating them would be the
first step towards a conformance claim nobody could support.
