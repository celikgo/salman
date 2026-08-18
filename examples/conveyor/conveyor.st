(* A conveyor with a start-stop station, a motor starter with a run-on delay,
   a part counter and a jam detector.

   This is salman's worked example. It is deliberately small enough to read in
   one sitting and deliberately not a toy: it uses a user-written function
   block, four of the IEC standard function blocks, a function, and the three
   kinds of statement a real program is made of.

   Run it:      salman run examples/conveyor/conveyor.st --until T#30s
   Test it:     salman test examples/conveyor/conveyor.st examples/conveyor/

   salman is not a safety tool. Nothing here is a safety function, and a real
   conveyor needs a hard-wired emergency stop that no program can override. *)

(* Latches a start-stop station and holds the motor on for a run-on time after
   the stop button, so that the belt clears before it halts. *)
FUNCTION_BLOCK Motor_Starter
VAR_INPUT
    Start   : BOOL;
    Stop    : BOOL;
    Run_On  : TIME;
END_VAR
VAR_OUTPUT
    Running : BOOL;
END_VAR
VAR
    Latch   : RS;
    Run_Off : TOF;
END_VAR
    (* RS is reset dominant: with both buttons pressed, the belt stops. *)
    Latch(S := Start, R1 := Stop);
    Run_Off(IN := Latch.Q1, PT := Run_On);
    Running := Run_Off.Q;
END_FUNCTION_BLOCK

(* Percentage of a batch, as whole percent. *)
FUNCTION Percent_Of : INT
VAR_INPUT
    Count : INT;
    Batch : INT;
END_VAR
    IF Batch <= 0 THEN
        Percent_Of := 0;
    ELSE
        Percent_Of := Count * 100 / Batch;
    END_IF;
END_FUNCTION

PROGRAM Conveyor
VAR
    (* Field inputs. In a real project these are located; salman 0.0.1 has no
       IO mapping layer, so the example drives them as plain variables and the
       tests write them directly. *)
    Start_PB    : BOOL;
    Stop_PB     : BOOL;
    Part_Sensor : BOOL;

    (* Field outputs. *)
    Motor       : BOOL;
    Jam_Lamp    : BOOL;
    Batch_Done  : BOOL;

    (* Internals. *)
    Starter     : Motor_Starter;
    Part_Edge   : R_TRIG;
    Parts       : CTU;
    Jam_Timer   : TON;
    Progress    : INT;
    State       : INT;
END_VAR
VAR CONSTANT
    Batch_Size  : INT := 10;
    Run_On_Time : TIME := T#2s;
    Jam_Timeout : TIME := T#10s;
END_VAR

    (* The belt runs while the starter says so. *)
    Starter(Start := Start_PB, Stop := Stop_PB, Run_On := Run_On_Time);
    Motor := Starter.Running;

    (* One count per part, on the leading edge of the sensor. *)
    Part_Edge(CLK := Part_Sensor);
    Parts(CU := Part_Edge.Q, R := Stop_PB, PV := Batch_Size);
    Batch_Done := Parts.Q;
    Progress := Percent_Of(Count := Parts.CV, Batch := Batch_Size);

    (* A jam is the belt running with no part seen for the timeout. Each part
       restarts the timer, because the edge output is low again the next scan. *)
    Jam_Timer(IN := Motor AND NOT Part_Edge.Q, PT := Jam_Timeout);
    Jam_Lamp := Jam_Timer.Q;

    (* A small state machine, to show CASE. *)
    CASE State OF
        0:  IF Motor THEN State := 1; END_IF;
        1:  IF Jam_Lamp THEN
                State := 2;
            ELSIF Batch_Done THEN
                State := 3;
            ELSIF NOT Motor THEN
                State := 0;
            END_IF;
        2:  IF NOT Jam_Lamp THEN State := 1; END_IF;
        3:  IF Stop_PB THEN State := 0; END_IF;
    ELSE
        State := 0;
    END_CASE;

END_PROGRAM
