# The conveyor example

A conveyor with a start-stop station, a motor starter with a run-on delay, a
part counter and a jam detector. Small enough to read in one sitting, and not a
toy: it uses a user-written function block, four of the IEC standard function
blocks, a function, and the three kinds of statement a real program is made of.

```
salman check examples/conveyor/conveyor.st
salman run   examples/conveyor/conveyor.st --until T#30s --record Motor,Jam_Lamp
salman test  examples/conveyor/conveyor.st examples/conveyor/
```

`salman test` runs `conveyor.salman-test.yaml`. Eight tests, including a
golden-trace test whose expected output is `conveyor.trace` — a text file you
can read, and whose diff in a pull request tells you what changed about the
machine's behaviour.

## What the tests are actually checking

The interesting ones are the ones about time, because they are the ones a
hardware test rig makes expensive:

- the run-on timer holds the motor for exactly two seconds after the stop
  button, checked at one millisecond either side of the boundary;
- a jam is flagged after exactly ten seconds, checked the same way;
- a part passing the sensor restarts the jam timer;
- a sensor held high counts one part, not one per scan.

Each of those runs in microseconds on a virtual clock, produces the same answer
on every machine, and needs no PLC.

## Not a safety function

Nothing here is a safety function, and a real conveyor needs a hard-wired
emergency stop that no program can override. salman is not certified under any
functional safety standard; see `LEGAL.md`.
