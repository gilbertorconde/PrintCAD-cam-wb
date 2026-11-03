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

And stock can be a round bar. In `camcraft-core`, `StockSpec` has
`CylinderAroundModels` (along a model axis through the middle of the
models' box, its radius a margin past the farthest of them from it, and
longer at each end) and `FixedCylinder` (a diameter and a length, centred
on the models across its axis and placed along it as a fixed box is). In
`camcraft-sim`, a `Block` is a box or a `Bar` along X, Y or Z, and
`Stock::from_blocks` starts the height field as one: a disc up the tool is
material over its circle only, a bar lying across it stands to its curve.
`Prepared::stocks` and `SetupResult::stocks` are blocks; what needs only a
bound (the stock's top and bottom, facing, linking, verifying, the work
zero) takes the box round them, and adaptive clearing clears what each
stands above a level, a disc's circle or a bar's width there. Facing still
faces the box round round stock, over air at its corners.

And two strategies, Fusion's Trace and Engrave, and V-carving. In
`camcraft-core`, a `Selection` has `curves` beside its picks: lines and
three-point arcs drawn in the document, a sketch's, on no model, so not
found again. `setup::features` reads them, in the setup's frame, its level
arcs kept as arcs, among the selection's points (for a height measured
from them) and, closed, among the loops its area is read from, even-odd,
so a letter's counter is a hole. `StrategySpec::Engrave` (`depth`,
`direction`: climb, conventional or as drawn, `reverse`) follows the
curves and the picked edges joined end to end, open or closed, a round
edge lying level as arcs (`strategy::engrave`): each ramps down along
itself at the ramp angle, round a closed one and back and forth along an
open one, a stepdown at most each time, then once at the depth; its arcs
are arc moves, helixes going down, so the post writes `G2` and `G3`.
`StrategySpec::VCarve` (`max_depth`) cuts what the picks enclose with a
cone only (`Operation::cannot_use`): round each ring of it, the tool goes
along the centres of the largest circles inside it touching the ring
(`camcraft_geom::medial`, new), as deep as each radius reaches on the V,
from the shallowest; past the deepest it may go it cuts the walls that
deep, and the floor left is a warning, since an operation has one tool.
`Uses` has `traces` and `carves`, and the run's targets with them. What
they cut into the part is meant: the part they are verified against is cut
as deep as they cut it (`PartSurface::cut_into` in `camcraft-sim`), as
negative stock to leave already was. In `camcraft-post`, a golden
engraving (`grbl-engrave.nc`).

And a setup's tool axis can be any direction, for 3+2 machining. In
`camcraft-core`, an `Orientation`'s `z` and `x` are each a `Direction`: a
model axis as before, a vector, a flat face's outward normal or a straight
edge's direction (`toward` a rough one, as a view gives), picked on a model
and found again by its point, either way (`reverse`). X is the one given
made square to Z. `Orientation::resolve` works it out as a `Turn`:
`Square`, axes given as axes, which turns as exactly as before, or
`Tilted`, a rotation; a `Frame` holds a `Turn` where it held an
`Orientation`. A tilted setup lays its stock out in its own axes
(`setup::laid`): a box round the models is round them as it sees them, its
margins, sizes and a bar's axis along its axes. In `camcraft-post`, a
profile's `[rotary]` names its rotary axes, from the base out, each turning
about the machine's X or Y with a sign and limits (`rotary.rs`);
`Rotary::position` finds where they stand a tool axis up the spindle, within
their limits, and refuses one they cannot reach, one past their limits, and
one that leaves the setup's X off the machine's (`RotaryError`, a
`LowerError`). `ProgramSpec` has `tilt` and `rotary`; lowering writes the
tilt in the header (`Op::Comment`) and positions the rotaries
(`Op::Rotary`, `G0 A.. B..`, the tool sent clear first) before the spindle
starts; `readback` reads A, B and C at rapid alone in their block, and
checks they are where the program needs them before it cuts, turned only
with the tool clear and never while cutting. A profile with rotaries needs
a `safe_retract`. `camcraft_session::program_spec` and `program` take the
setup's `Frame`. Golden programs: `masso-rotary-a.nc` and
`linuxcnc-rotary-ab.nc`.

And a slot, Fusion's Slot. In `camcraft-core`, `StrategySpec::Slot`
(`depth`, 3 mm unless set; `pattern`, `SlotPattern`: bidirectional or
directional; `reverse`) is an engraving deeper, with an end mill its width
(a cone is refused, `Operation::cannot_use`), along the same curves and
edges, as drawn, a millimetre a level unless set. Both ways it is
`engrave`'s own ramping back and forth; one way, `EngraveParams::levels`
is `Levels::OneWay`: each level, evenly a stepdown at most apart, a pass
of its own from the curve's start, which the linker lifts out of (it never
stays down, `run/operation/link.rs`) and enters as the linking's entry
says, a ramp along the slot or a plunge, never a helix beside it. So
`Operation::entry` with none asked is the first entry a strategy offers,
not always a helix. Its arcs are arcs at every level. A golden slot
(`grbl-slot.nc`).

And deburring, Fusion's 2D Chamfer. In `camcraft-core`,
`StrategySpec::Chamfer` (`width`, 0.5 mm; `tip_offset`, 0.2 mm; `side`,
`ChamferSide`: auto or flipped) runs a cone along the curves and picked
edges an engraving takes (`strategy::chamfer`). `Place` puts the cone:
its tip `width / tan(half angle)` and the tip offset below each edge,
its centre the cone's radius at the chamfer's foot off the edge; a cone
with too little flank for the width is refused (`Operation::cannot_use`).
The side away from the material is where the part, dropped onto either
side of each curve, is lower. Each curve is moved that far to that side,
its lines and arcs square to themselves, and run to climb, then engraved
from its own height, the edge's: it has no top or bottom.
`run::chamfer_centres` gives the curves, for drawing. In `camcraft-geom`,
`ring::offset_open` moves open spans as `Ring::offset` moves a ring's,
which it now shares, and `Ring::new` makes a ring of spans. What a
chamfer cuts into the part is meant, as an engraving's is.

And one test: ogeom is 0.11.1 here, which printCAD's shapes need, and it
meshes `round_edge_loose_fit.step`'s seam that 0.11.0 drew off the torus, so
`a_loose_fit_is_refused_for_ogeoms_seam` became
`a_loose_fit_reads_within_the_tolerance`, as its comment asked.

To take a newer camcraft, copy those crates over these, then make the same
changes again; `diff -r` against the commit above shows them.
