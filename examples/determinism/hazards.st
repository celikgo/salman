(* The determinism reference.

   Every kind of value salman can put in a trace that could plausibly differ
   between two machines, in one program that drives itself and needs no inputs.
   `.github/workflows/determinism.yml` runs this on Linux, macOS and Windows and
   compares the trace fingerprints; `docs/adr/ADR-0005-determinism.md` is the
   decision it enforces.

   This is a CI fixture, not a tutorial. If you want to learn Structured Text,
   read examples/conveyor/ instead — it is a machine rather than a list.

   Every variable earns its place by being a hazard ADR-0005 names. Nothing here
   is arbitrary, and a value that stops being a hazard should be deleted rather
   than kept for the sake of a longer trace.

   Run it:  salman run examples/determinism/hazards.st --scans 1000 \
                --record Scan,Wrapping,Drift,Third,NotANumber,Infinite,NegZero,Elapsed,Phase,Alternating,Second

   salman is not a safety tool. Nothing here is a safety function. *)

PROGRAM Hazards
VAR
    (* A scan counter, so that no column is constant by accident and a
       truncated trace is obvious rather than plausible. *)
    Scan        : DINT;

    (* Integer overflow wraps. That is a salman policy, not a standard
       requirement — docs/CONFORMANCE.md policy 13 — so it is exactly the kind
       of behaviour that must not drift between platforms unnoticed. *)
    Wrapping    : SINT;

    (* A REAL accumulating a value with no exact binary representation. This is
       the column that moves if anything about 32-bit float arithmetic, or the
       shortest-round-trip formatting salman renders it with, ever changes. *)
    Drift       : REAL;

    (* LREAL division. IEC 61131-3:2013 references IEEE 754 normatively for the
       real types, and Rust guarantees the exact semantics of `/`, so this is a
       value both standards fix and neither platform may argue with. *)
    Ticks       : LREAL;
    Third       : LREAL;

    (* The three bit patterns ADR-0005 was written about.

       All three render identically on every architecture, so the rendered
       trace cannot tell you whether they are right. The fingerprint can: it is
       SHA-256 over a canonical binary encoding rather than over this text, so a
       NaN that stopped being canonicalised would change the fingerprint while
       the column still read `NaN`. That asymmetry is the whole reason the gate
       compares fingerprints and shows text only to a human. *)
    NotANumber  : LREAL;
    Infinite    : LREAL;
    NegZero     : LREAL;

    (* Duration arithmetic, rendered back as an IEC literal. *)
    Elapsed     : TIME;

    (* A little control flow, so the trace is not one straight line of
       assignments. *)
    Phase       : INT;
    Alternating : BOOL;

    (* Left at its initial value, so that `Zero / Zero` below is a division the
       checker cannot fold away before the program ever runs. *)
    Zero        : LREAL;
END_VAR
    Scan := Scan + 1;
    Wrapping := Wrapping + 100;

    Drift := Drift + 0.1;
    Ticks := Ticks + 1.0;
    Third := Ticks / 3.0;

    NotANumber := Zero / Zero;
    Infinite := 1.0E308 * 10.0;
    NegZero := -0.0;

    Elapsed := Elapsed + T#1s;
    Alternating := NOT Alternating;

    CASE Phase OF
        0:  Phase := 1;
        1:  Phase := 2;
        2:  Phase := 0;
    ELSE
        Phase := 0;
    END_CASE;

END_PROGRAM

(* A second program, so that the trace has two tasks in it.

   A file with no CONFIGURATION gives every PROGRAM one freewheeling task of its
   own, in declaration order — docs/CONFORMANCE.md policy 18 — so these two are
   released at the same instant on every tick and the trace carries two rows per
   instant. Which of them comes first is the whole point: the tie is broken by
   `(next_release_ns, priority, index)` in crates/salman-vm/src/task.rs, where
   `index` is declaration order. If that order ever came from iterating a
   collection instead, these rows would swap and the fingerprint would move.

   This is the hazard the `HashMap` ban in clippy.toml exists to prevent, and it
   is the only one of ADR-0005's hazards that a single-task trace cannot show. *)
PROGRAM Ordering
VAR
    Second : DINT;
END_VAR
    Second := Second + 1;
END_PROGRAM
