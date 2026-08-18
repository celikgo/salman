#!/usr/bin/env python3
"""Checks what salman's client printed against what pymodbus was seeded with.

Reads the client's output on stdin.
"""
import sys

EXPECTED = {
    "holding": [0x1000 + i for i in range(5)],
    "input": [0x2000 + i for i in range(5)],
    "coils": [0, 1, 1, 0, 1, 0, 0, 1],
    "wrote-then-read": [0xBEEF],
    "wrote-then-read-many": [1, 2, 3],
}

def main():
    seen = {}
    out_of_range = None
    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue
        key, _, rest = line.partition(" ")
        if key == "out-of-range":
            out_of_range = rest
            continue
        if key in EXPECTED:
            seen[key] = [int(v) for v in rest.strip("[]").split(",") if v.strip()]
        print(f"     {line}")

    failures = []
    for key, expected in EXPECTED.items():
        if key not in seen:
            failures.append(f"{key}: salman printed nothing")
        elif seen[key] != expected:
            failures.append(f"{key}: salman read {seen[key]}, pymodbus holds {expected}")
        else:
            print(f"ok   {key}: {seen[key]}")

    if out_of_range is None:
        failures.append("out of range: salman printed nothing")
    elif "Illegal Data Address" not in out_of_range:
        failures.append(f"out of range: expected exception 2, salman said {out_of_range!r}")
    else:
        print("ok   out of range: exception 2, Illegal Data Address")

    for failure in failures:
        print(f"FAIL {failure}", file=sys.stderr)
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
