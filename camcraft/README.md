# camcraft's engine, copied in

These crates are [camcraft](https://github.com/dan335/camcraft)'s engine, by
dan335, MIT OR Apache-2.0 (`LICENSE-MIT`, `LICENSE-APACHE`, `NOTICE`). They
build the package's native helper (`../helper`), which runs on every core, and
lend the package (WebAssembly) camcraft's job types, tool library and job
file; the package does no machining itself.

Copied from camcraft 0.2.0, commit `b53326c` ("Release 0.2.0"):
`crates/camcraft-core`, `camcraft-geom`, `camcraft-sim`, `camcraft-post`,
`camcraft-session`, `camcraft-import` and `step-check`, and `fixtures/` and
`examples/rail-jig.camcraft`, which their tests read. The app (`camcraft-gui`), the command line (`camcraft-cli`)
and `vendor/` (OpenCascade, LGPL) are left out.

## What is changed here

In `camcraft-import`:

- The `opencascade` feature is gone (its code, `build.rs` and
  `tests/opencascade.rs`), and with it every trace of OpenCascade and the LGPL.
- `native` reads a solid in ogeom's native format, as printCAD keeps a body's
  shape, and tessellates it as a STEP file's solids are: `step::mesh_bodies`
  and `step::weld_faces` are what `step` did inline, split out so both use
  them.

And in `camcraft-post`: a LinuxCNC profile (`profiles/linuxcnc.toml`, and
`linuxcnc-inch.toml`), and a profile's optional `spindle = [slowest, fastest]`
in rev/min, which `Profile::spindle_problem` checks a speed against; the
helper warns about an operation asked to turn outside it.

And one fix, in `camcraft-core`'s adaptive clearing
(`strategy/adaptive/clearing.rs`): a pass that circles a speck it touches
every lap but never takes used to go round until the level's steps ran out;
now a pass that goes a tool's radius taking less than a sliver, and ends up
close to where it last took some, ends there. The test
`a_pass_that_takes_nothing_ends` replays one such pass from
`strategy/adaptive/circling.txt`.

And in `camcraft-post`, so a program's lines can be traced to the moves
they make, for the listing beside playback: `op::lower_counted` is `lower`
that also says, for each instruction, how many of the path's moves are made
once it has run (a canned cycle counting the moves that drill its hole), and
`RenderReport::op_lines` says how many lines are written once each
instruction is. What is written is unchanged.

And drilling breaks the chip and taps, as Fusion's drilling cycles do. In
`camcraft-core`, `StrategySpec::Drill` has `chip_break` (with a peck, each
backs off only that far: Fusion's chip break distance) and `pitch` and
`hand` (each hole tapped, fed at the pitch times the spindle's speed both
ways; a left-hand tap turns counter-clockwise, whatever its tool says).
There is no tap kind of tool: a tap is a flat end mill or a drill its size,
which is the hole it leaves. In `camcraft-post`, the cycles are `G73`,
`G84` and `G74`, a tap's feed written to three more places than other feeds;
`ProgramSpec::taps` makes lowering refuse a tap as plain moves
(`LowerError::TapsAsMoves`), and one turning against its thread; and
`readback` reads the three cycles and checks a tap is fed at its pitch a
turn, both ways, with the spindle turning its way.

And one test: ogeom is 0.11.1 here, which printCAD's shapes need, and it
meshes `round_edge_loose_fit.step`'s seam that 0.11.0 drew off the torus, so
`a_loose_fit_is_refused_for_ogeoms_seam` became
`a_loose_fit_reads_within_the_tolerance`, as its comment asked.

To take a newer camcraft, copy those crates over these, then make the same
changes again; `diff -r` against the commit above shows them.
