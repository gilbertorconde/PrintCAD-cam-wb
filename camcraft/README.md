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

And a machine taps as its controller does. In `camcraft-post`, a profile's
`tap` (`TapStyle`) names how: `"g84"` (`G84` and `G74`, as before, the
default for a profile with `drill_cycles`; in Klartext cycle 207, which
`"cycle"` also names), `"m29"` (those after `M29 S..`, Fanuc's rigid
tapping, `Op::RigidMode`), `"g33.1"` (rigid tapping, as LinuxCNC has it)
or `"none"`; `Profile::tapping` says which, and a Heidenhain profile may
ask only for its cycle or none. `DrillCycles::tap_style` carries it to
lowering. With `"g33.1"` each tapped hole is written on its own: across at
the clearance height, down to its R plane, `Op::RigidTap` (`G33.1 Z..
K<pitch>`, the pitch to three more places than an axis word,
`Profile::pitch`) and back up, no canned cycle round it. `readback` reads
`G33.1`, down Z only, fed at `K` times the spindle's speed both ways and
back to where it began, refusing it with the spindle stopped, with no `K`,
or with an `I`, and checks it as it checks a `G84`; and `M29 S..`
(`Action::RigidMode`), which must be at the speed the spindle turns, with
nothing moving before its tapping cycle and no other cycle after it until
`G80`, and without which a Fanuc's tap is refused. The LinuxCNC profiles
take the cycles and tap with `G33.1` (LinuxCNC's `G84` is for a floating
holder, not synchronized); the Centroid ones take the cycles and tap with
`G84`; Masso's take the cycles, `dwell_milliseconds` for its `G82`, and do
not tap; Fanuc's tap with `"m29"`, Haas's with `"g84"` and Heidenhain's
with cycle 207, their other cycles still off; GRBL's and Mach's say
`"none"`. In `camcraft-session`, `program_spec` posts a tap however the
machine taps, cycles or not, and refuses one for a machine that cannot
(`LowerError::CannotTap`), naming the built-in machines that can
(`Profile::builtin_tapping`). Golden programs: `linuxcnc-tap.nc`,
`linuxcnc-inch-tap-left.nc`, `linuxcnc-chip-break.nc`,
`centroid-avid-tap.nc`, `centroid-avid-inch-tap-left.nc`, `fanuc-tap.nc`,
`fanuc-inch-tap-left.nc`, `haas-tap.nc` and `haas-inch-tap-left.nc`;
`masso-tap.nc` and `masso-tap-left.nc` are gone, as Masso cannot tap, and
`masso-dwell.nc` pauses in milliseconds.

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

And patterns, Fusion's Pattern and Mirror. In `camcraft-core`, an
`Operation` has a `transform` (`job::Pattern`; not `pattern`, which a
slot's own setting is called beside it: `linear`, rows along X and Y;
`polar`, a count over an angle about a centre; `mirror`, about a line
along X or Y through a point, the original kept; at most 1000 copies),
which `path::pattern::repeat` applies to the strategy's path before it is
ordered, tabbed and linked (`run/operation.rs`), so the linker links every
copy into the material and the simulation, the checks and the program see
them all. A mirrored copy's passes are turned round where that is safe:
one all at one level after its link backwards whole, else each stretch
of cutting that closes on itself backwards in place; adaptive clearing
and drilling never are, and passes left cutting the other way are a
warning, as is a copy further outside the stock than the original. In
`camcraft-session`, the finishing job's operation has no pattern.

And a machine's own blocks, its travel, and Custom G-code. In
`camcraft-post`, `placeholder` fills `@name@` placeholders (the program,
the operation, its strategy, the tool and its description, the speed, the
spindle's way, the coolant, the units, the work offset and the date) in a
profile's `header`, `footer`, `safe_retract`, `end_home` and new
`tool_change_lines` (written in place of the `tool_change` style's own,
naming `@tool@`), `@@` an `@` and `{name}` as before; a name that is not
one, or an `@` never closed, makes the profile invalid
(`ProfileError::BadValue`, with the block and the line). `render_dated`
renders with a date for `@date@`; `render` leaves it empty. A profile's
optional `[travel]` gives each of `x`, `y` and `z` as `[lowest, highest]`
in the program's coordinates and units, which `Profile::travel_problems`
checks a program's extents against (`op::extents`, an arc's bulge
included); `camcraft_session::travel_problems` (now `travel_check`, below) checks an operation's, and
`plan_posting` refuses a setup that goes past it. In `camcraft-core`,
`StrategySpec::CustomGcode` is an operation of lines for the program
(`Operation::gcode`): it needs no tool, is not worked out or cut, and its
result is `Made::Text` with `StrategyDetail::CustomGcode`. In
`camcraft-post`, `ProgramSpec::before` and `after` hold `op::CustomGcode`
blocks, lowered as `Op::Custom`, which the renderer writes between two
comments (`render::custom_begin`, `custom_end`), placeholders filled, and
forgets where the machine is after; lowering stops the coolant after the
blocks before an operation, and the spindle and coolant after those at the
end. `custom::concern` says what a line may not do: an axis word, a G code
but `G4`, `M6`, `M60`, `M2`, `M30` or `M99` refused, a subprogram or macro
call (an `O` word, `M98`, `G65`, `G66`, `M100` up) not read. `readback`
skips the lines between those comments (`custom_blocks`, `read_around`),
refusing what `concern` refuses, counts the blocks against the spec, and
after each (`Action::Custom`) takes the position, spindle, coolant, tool
and work offset as not known, refusing one in the middle of the path. In
`camcraft-session`, `custom_gcode` puts each Custom G-code operation
before the next operation with a program, or after the last, and
`plan_posting` and `program_with` post them; the checks add `CC108` (a
line that moves or changes a mode, or a placeholder that is not one:
fails), `CC216` (a call not read) and `CC217` (custom G-code in a setup
with no program to go into). A golden program with all of it
(`linuxcnc-custom.nc`).

And travel in machine coordinates, and work offsets past G59. In
`camcraft-post`, a profile's `[travel]` takes `coordinates = "machine"`
(`Coordinates`; left out it is `work`, as before), and
`Profile::travel_check` (in place of `travel_problems`) places a program on
the machine by a `Placement`: where its work zero is in machine
coordinates, and the tool's length from the gauge line, added to Z when the
profile takes length offsets. It returns a `TravelCheck`: each axis past
the travel, how far under or over, and what was not checked (no work
offset position; with length offsets, no tool length). A `G53` move in the
profile's own blocks outside a machine travel makes it invalid. A profile's
`extended_offsets` lists the work offsets it has past `G59` as it writes
them; a work offset from 60 is one of them (`Profile::wcs_word`,
`work_offsets`), `ProgramSpec::extended_offsets` says how many lowering
lets through, and the renderer and `@wcs@` write the word. `readback`
reads `G59.1` to `G59.3`. The LinuxCNC profiles list those three. In
`camcraft-core`, `Setup::work_offset` is where its work zero is in machine
coordinates, a `wcs` may be up to `LAST_WCS` (G59 and 99 more),
`ToolSpec::gauge_length` is a tool's stickout (or overall length) and its
holder's segments, and `OperationResult::tool_length` carries it. In
`camcraft-session`, `travel_check` checks an operation of a setup, and
`program_spec` copies the profile's extended offsets. A golden program on
`G59.2` (`linuxcnc-g59-2.nc`).

And posts for industrial controllers. In `camcraft-post`, profiles for
Fanuc (`fanuc.toml`, `fanuc-inch.toml`), Haas's NGC (`haas.toml`,
`haas-inch.toml`), Mach3 and Mach4 (`mach.toml`, `mach-inch.toml`) and
Heidenhain's TNC (`heidenhain.toml`, `heidenhain-inch.toml`), each with
golden programs. A profile has a `dialect`, `profile::Dialect`: `iso`,
G-code, unless it says `heidenhain`, which `render_dated` hands to the new
`heidenhain` module to write Klartext, and `readback::check` to read it back
(`heidenhain::read_around`, `custom_blocks` and `concern`, for custom lines)
into the same `Step`s G-code is read into, so the same checks run. Its
programs are numbered from 0 by 1 with `;` comments, which `validate`
insists on; its `M91` moves are kept inside a machine travel as `G53`'s
are. `preselect_tool` calls up the next program's tool after the
change, `Op::PreSelect` (`T<n>`, `TOOL DEF <n>`), from
`ProgramSpec::next_tool`; `ProgramSpec::stock` is lowered as `Op::Stock`,
which only Klartext writes (`BLK FORM`); G-code writes neither unless asked.
`dwell_milliseconds` writes a pause's `P` in milliseconds, as Fanuc reads
it in `G82` and `G4`, and `readback::read_dwelling` reads it so. `readback`
also reads an `O` program number, Mach's `G91.1` (the arc centres it
reads), Haas's `G103` and `G187` (with its `E`), the work offsets Fanuc's
and Haas's profiles list past G59 (`G54.1 P1` to `P48`, `G154 P1` to
`P99`, a setup's 60 on), and takes up to 4096 moves
for an arc written as straight ones, as Klartext writes a helix about X or
Y. `Profile::number` is public, for the dialect's own numbers. In
`camcraft-session`, `next_tools` and `stock_box` give each program its
next tool and the stock, which `plan_posting` and the helper pass on.

And a setup posted as one program, as Fusion posts one. In
`camcraft-core`, a `Setup` has `programs` (`Programs`: `setup`, the
default, or `operation`, one an operation as before) and `number` (1001
unless set). In `camcraft-post`, `setup::lower_setup` lowers its
operations, each a `Section` (its path and its `ProgramSpec`), into one
stream: the units once; each operation's custom G-code, its section and its
moves; the tool changed only where it changes (or after custom G-code, or
an optional stop), the same tool kept turning, its speed and coolant set
again only where they change; between two, the profile's safe retract, or,
with none, rapids straight up as high as either goes and across, so the
next starts straight down; the end and home once. `Joining` says, from the
profile, whether it takes the tool up itself and whether it stops between
operations: a profile's new `optional_stop` writes `Op::OptionalStop`
(`M1`) between every two, the spindle and coolant stopped and the tool clear
first. It says of each instruction which section it is of and how many of
that section's moves are made once it has run. `op::refuse` is what
`lower_counted` refused, split out so both refuse alike. The renderer fills
each operation's lines with its own values (an operation runs from just
after the one before ends, its custom G-code with it), the header with the
first's and the footer with the last's; what one operation does not set
again, as a tool kept turning, is the one before's. `render_program` renders
with a `Title`, giving `@number@` and `@number5@` (five digits), new
placeholders, which the Fanuc and Haas profiles' `O` lines now take;
`render_dated` gives 1001. A Klartext program, which `render_program` hands
to `heidenhain::render`, fills each operation's lines as G-code's do, and
calls each tool at its operation's speed. In a joined program the stock
(`Op::Stock`) and the work offset are written once (the offset again only
after custom G-code), and after each tool change the next other tool an
operation takes is called up (`Op::PreSelect`), where the profile does.
`readback::check_setup` reads a setup's program, G-code or Klartext, back
against every section in turn, each checked as `check` checks one (its tool, work
offset, length offset, spindle and coolant in force before it cuts, its
moves or holes), and between two allows only rapids up, across as high as
the one before went and the next begins, and down where the next begins.
In `camcraft-session`, `setup_specs` and `setup_program` post a setup as one
(`setup_program_name`, after the setup), saying which operation and move
each instruction is of, and `plan_posting` takes `programs` to post every
setup one way or the other, else as each says; programs one an operation are
numbered on from the setup's. `setup_specs` gives each the setup's stock.
Golden programs:
`linuxcnc-setup.nc`, `grbl-setup.nc`, `masso-setup-stop.nc`,
`fanuc-setup.nc`, `haas-setup.nc` and `heidenhain-setup.nc`; and with two
tapping operations, each in its post's form (`fanuc-setup-taps.nc`, rigid
mode taken again for each, as it ends at `G80`; `linuxcnc-setup-taps.nc`,
`G33.1`).

And one test: ogeom is 0.11.1 here, which printCAD's shapes need, and it
meshes `round_edge_loose_fit.step`'s seam that 0.11.0 drew off the torus, so
`a_loose_fit_is_refused_for_ogeoms_seam` became
`a_loose_fit_reads_within_the_tolerance`, as its comment asked.

To take a newer camcraft, copy those crates over these, then make the same
changes again; `diff -r` against the commit above shows them.
