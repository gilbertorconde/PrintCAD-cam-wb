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
A tap was then a flat end mill or a drill its size, which is the hole it
leaves; it is now a kind of its own (below), and those still tap. In
`camcraft-post`, the cycles are `G73`,
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
of cutting that closes on itself backwards in place; drilling's never
are, and passes left cutting the other way are a warning, as is a copy
further outside the stock than the original. In `camcraft-session`, the
finishing job's operation has no pattern.

And a pattern of a strategy the material steers, adaptive clearing and
pocket clearing, is worked out copy by copy (`run/operation/copies.rs`),
not moved whole: the material where each copy is, brought to the
original's place through the copy undone (`pattern::Copy::inverse` and
`Copy::region`), is cleared from the original's levels, and the passes
moved out again (`Copy::moves`, `pattern::push_copy`); a mirrored copy is
cleared the other way round, so it climbs as the original does. Each
copy's passes are cut into a copy of the material before a copy whose
footprint (its area, the tool and a helix round it) overlaps them is
worked out; copies that overlap none not yet cut are worked out together,
in parallel, with the same result on any number of cores. To work out
many from one, `strategy::adaptive` has `AdaptiveLevels` (the levels and
where the tool may be at each, from the part, once; `clear` takes the
material at each level as an `Uncut` closure, the blocks' for
`adaptive_rough` as before) and `strategy::pocket_clearing` has
`ClearingLevels` (likewise, its material a `Standing` closure, the
stock's grid for `pocket_clearing` as before), and each report has `and`.
Patterned, an adaptive clearing's material is the blocks less where the
grid stands lower than where its copies start from, taken the tolerance
wider; a copy that finds nothing is a warning.

And rest machining, Fusion's, on adaptive clearing.
`StrategySpec::Adaptive` has `rest` (`job::Rest`: `source`, a
`RestSource`, only from previous operations so far, and `adjustment`, in
mm, at least zero; none, off, the default). On, the operation is worked
out as a pattern's one copy is, its material the blocks less what the
operations before it cut, read from the grid as it stands; off, a
pattern's copies start from the blocks, so they see only what each other
cut and the original is what it is alone. Each piece of what is left
within the tool's reach at a level (`AdaptiveLevels::allowed`, grown by
its radius) that is thinner than the adjustment everywhere is taken as
cleared. Pocket clearing already reads the material as it stands, so has
no such choice.

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

And holes milled, as Fusion's Bore, Circular and Thread, and picked as
its Select Same Diameter picks them. In `camcraft-core`, `StrategySpec`
has `Bore` (`pitch`, a millimetre a turn; `conventional`), `Circular`
(`conventional`; `boss_width`, how far out from a boss its material
goes) and `Thread` (`pitch`, `hand`, `diameter`, its major one, the
hole's plus the thread's depth or the boss's own unless given;
`radial_passes`, not `passes`, which the operation's own are called;
`spring_pass`; `conventional`), each with its holes as a drill's are
(`Uses::holes`), worked out in `strategy::holes`: a bore down a helix
about each hole, round once at the bottom, then finishing passes
(`Operation::finishing`) there; circles a stepover apart, a hole's from
its middle out and a boss's in to it, a stepdown a level, each level
entered down a helix at the ramp angle; a thread round a helix at its
pitch, the ISO profile's depth (`thread_depth`) in radial passes, up or
down as the hand and climbing say, a multi-form mill a turn for each
span its teeth cover, its end no lower than the bottom. Into and out of
a finishing circle or a thread it goes along half circles tangent to
it; every move round a hole is an arc, a helix where it goes down, which
the post writes as `G2` or `G3` with a `Z`, split as the profile's arcs
are. Each hole is one pass, linked as a drilled one is. A thread's
groove past the wall is meant: the part it is verified against is cut
where it cut (`run::cut_past`). A `ToolKind::ThreadMill` (`tip_length`,
`pitch_range` and `teeth` on `ToolSpec`) cuts as a flat end mill its
cutting diameter, cuts only threads, and a thread only it
(`Operation::cannot_use`, which also refuses a pitch outside its
range); a Fusion library's thread mill imports as one, its pitch its
range. `drilling::rounds` finds holes and round bosses (`Round`, `Rim`)
from rims, a hole and a boss on one axis two, and `drilling::hole_order`
orders them nearest first, by X then Y back and forth, or as picked. An
`Operation` has `holes` (`HolePicking`: `same_diameter`, `order`, a
`HoleOrder`). `run/operation/holes.rs` takes a picked face as a hole's
wall (curved, every corner on one cylinder along the tool, its rims at
two heights or more), a boss's when it faces out, and with the same
diameter every such wall of the setup's models as wide; drilling and
boring refuse a boss (`HoleError::Boss`), and milling what does not fit
is a `RunError::Milling`. `setup::features::chain_of` makes an edge's
chain. `StrategyDetail::Holes` reports the holes. Golden programs:
`grbl-holes.nc`, `linuxcnc-holes.nc`, `fanuc-holes.nc` and
`heidenhain-holes.nc` (each helix a Klartext `CP IPA.. IZ.. DR±`), read
back in every profile, alone and in a setup's one program.

And corner relief, dogbones and T-bones, in `camcraft-core`. `Passes` has
`relief` (`job::Relief`: a `style`, `ReliefStyle`, `dogbone` or `t-bone-x`,
`t-bone-y`, `t-bone-long`, `t-bone-short`, and `sharper_than`, degrees, to
leave blunter corners), which `Uses::relief` gives a 2D profile, a pocket
and a slot, and `Operation::relief` reads. The new `path::relief` finds the
corners from the areas' rings, or a slot's curves (`corners`, `ring_edges`,
`trace_edges`): the sharp corners of the tool centre's path whose outside
is the material kept, each reaching as far as takes the tool's side to the
walls' point; and `relieve` goes out to each and back, level, every time a
cut or a ramp reaches one. `run_operation` relieves the strategy's path
before the pattern repeats it, and `OperationResult::reliefs` says where
each relief went, in every copy, for `cut_past` to take what the notches
cut past the part as meant, not as a gouge. Tests in `path/relief/tests.rs`
and `a_square_hole_has_its_corners_relieved`; in `camcraft-post`, the
golden program `grbl-dogbone.nc`.

And machining time, for the setup sheet. In `camcraft-post`, a profile's
`[rapid]` (`Rapid`) gives each axis's rapid rate, `x`, `y` and `z` in
mm/min whatever the program's units (`ASSUMED_RAPID`, 5000, for one left
out), and `tool_change_seconds` how long a change takes, chip to chip;
`Profile::rapids` gives them, `Rapid::seconds` times a rapid as its slowest
axis goes, and `Rapid::feed_seconds` a feed move at its feed but never
quicker than its axes go at rapid. Neither speeds up or slows down. A rate
of nothing, or a time that is not one, makes the profile invalid. Every
built-in profile has them, typical of its kind of machine, its comments
saying where the machine's own are kept. `op::tilt_notes` is public, for
the sheet to say what the program's header says of a tilt. In
`camcraft-session`, `SetupProgram::tool_changes` says how many times a
setup's program changes the tool.

And holder and shank collisions. In `camcraft-sim`, `stock::reach`: a
tool's `Body` above its flutes as `Ring`s (heights above the tip, a radius
at each end, a cone checked as four cylinders no narrower than it), `Reach`
to find what material stands into each over a straight move (exact for a
cylinder, the lowest its foot comes while within reach of a cell), `Tops`,
each 8 by 8 block's tallest cell, so a ring well above the material skips
it (a block cut into is read again only when a ring comes down to it), and
`Stock::sweep_watching`, which checks each move before sweeping it.
`PartSurface::field` is public. In `camcraft-core`, `HolderSegment` has an
optional `top_diameter` (a cone, as Fusion's upper diameter) and
`HolderSegment::cylinder`; `ToolSpec::above_flutes` gives the rings the
tool's lengths place, each a `Piece` (body, shank, holder segment), and
what they leave unchecked (a thread mill's neck, behind its teeth, is not
checked; its shank is, from its shoulder); `Passes::use_holder` is Fusion's Use Shank and
Holder. A run makes the part's `PartSurface` before its operations, not
after, and each operation (and each kept one) is cut watching its tool
(`run/operation/holder.rs`), its `OperationResult::clashes` saying what met
material, from which move, how far, whether the part, and the stickout
that clears the holder; a clash fails `is_clean` and is a warning. With
`use_holder`, a 3D finish's raw path loses the cuts where the tool above
its flutes meets the part, a link left where it goes on, before ordering
and linking. In `camcraft-session`, the check `CC111` (a failure at the
first move, or a pass, or a note of what was not checked) and `CC218` (cuts
left out); `library::system::holders`, a dozen common holders; and
`library::fusion_json`, Fusion's JSON tool library, which carries holders,
read as `fusion_csv` reads CSV (`fusion_csv::name_for` is shared).

And a setup may carry on from the material an earlier one left, as
Fusion's setups take their stock from the one before. In `camcraft-sim`, a
`Stock` may have an underside (`Stock::under`): each column's bottom, where
the material stands clear of the floor, shared by its copies as it never
changes. A tool taking a column down to its bottom empties it (to the
floor), and only what stood above the bottom is cut, so a rapid through it
after is clear; the volume is measured from the bottoms. `Stock::with_bottoms`
gives a stock its underside, `Block::bottom_near` saying where a block's
starts. `Carry` (a rigid map from one frame to another, and the box the
material is within) carries a stock into another frame:
`Stock::carried_bounds` is the box round it there, and `Stock::carried`
makes it a stock on a grid there. Down the same line, the other way up or
turned about it, each column stays one, a cell taking every earlier column
whose cell overlaps its own; across it, each new column is looked down for
the first material from above and from below and held as everything
between, grown a cell all round: never less material, only more.
`PackedStock` keeps the underside, and `to_bytes` and `from_bytes` keep a
stock in a file. In `camcraft-core`, `setup::Carried` is what a setup left
(its stock with an underside, its box, and its frame to model coordinates),
`SetupResult::left` gives it, and `setup::prepare_from` lays a setup out on
it in place of its own stock, one block the box round it as the setup sees
it (`Prepared::carried`); `run_prepared_planned` then starts from it
carried into the setup's frame. `setup::affine_of` makes a turn and a move
an affine map. In `camcraft-session`, `run_setup_from` runs a setup so.

And probing, and cut depths corrected to what was probed. In
`camcraft-core`, `ToolKind::Probe` is a touch probe, its ball `diameter`
across, built as a ball; it is not turned (an operation's `rpm` is 0),
probes only, and only it probes (`Operation::cannot_use`). A Fusion
library's probe imports as one. `StrategySpec::Probe` (`x_count`,
`y_count`, `x_spacing` and `y_spacing`, 10 mm; `inset`, 2 mm;
`probe_feed`, 50 mm/min; `max_depth`, 3 mm) is a grid over its boundary's
box (the stock's footprint unless kept to less), `inset` in from it, so
many columns and rows or as few as keep them no further apart than the
spacing (`probe::grid_lines`); `probe::probe_path` touches each, back and
forth along the rows, from the retract height down to the top as a
`Plunge` at the probe feed and back up at rapid. `run_prepared_planned`
makes it apart from the cutting operations (`run::probe`), cuts nothing
with it, and refuses heights that are not clearance above retract above
top (`StrategyError::ProbeHeights`); `StrategyDetail::Probe` reports its
grid (`probe::ProbeReport`). A `Setup` has `z_correction`
(`job::ZCorrection`: the probed `points`, or a `file` to read them from,
their `units` and a `segment`, 1 mm). `probe::parse` reads a LinuxCNC
probe log (x y z and the other axes a line) or a CSV of x, y, z, a
heading and comments passed over and anything else refused;
`probe::ProbeMap` makes them a grid in any order (columns and rows within
0.01 mm one), refusing points that are not one (`ProbeMapError`), and
interpolates it bilinearly, outside it at its nearest edge, a grid of one
column or row level across it. `probe::correct` moves every move by the
map: a straight feeding move split no longer than the segment seen from
above, an arc cut into chords and split likewise unless the map is level
under it, when it stays an arc, and a rapid at its end; it counts the
points outside the map. With a correction, an operation is planned as
before, against the stock as modelled, and once every one is, each is
corrected (but a probe) and cut again, in order, into a stock whose top is
moved by the map (`Stock::warped`, in `camcraft-sim`); the keyframes are
that stock's, the part is moved too (`PartSurface::warped`) before it is
verified, a drill's holes for its cycles (`probe::correct_holes`), and an
operation that goes outside the map says how far. A kept operation is
worked out again. The holder is watched again on the corrected paths, against the part
moved likewise (`cut_kept`), and a setup that carries on from an earlier
one's material starts the corrected pass from that material, moved.
`SetupResult::probe_map` is the map, and in `camcraft-session` the CC102
check lowers the cutter onto the part where the map puts it (but for a
probe's own rapids, which are not corrected), CC112 fails a probe posted
in one program with the cuts corrected to probe results, which would
follow what was probed before, not what it probes; `read_probe_results` reads a setup's `file` relative to
the job (`SessionError::ProbeIo`), and the run's notes say what the map
is. `JobError::BadProbeMap` refuses results that are no map, or missing.
In `camcraft-post`, a profile's `probe` (`ProbeStyle`) says how its
controller probes: `"g38.2"` (`G38.2`, the point reported by the
controller, as GRBL does over its serial line), `"g38.2-probeopen"`
(`G38.2`, each point logged by the controller to the file `(PROBEOPEN
name)` opens, until `(PROBECLOSE)`, as LinuxCNC does) or `"none"`, the
default, which a Heidenhain profile must be. GRBL's profile probes, and
LinuxCNC's log. `ProgramSpec::probe` (`op::Probing`: `depth`, and the
`log` file) makes lowering write each plunge as `Op::Probe`, `depth`
below where the path expects to touch, between `Op::ProbeOpen` and
`Op::ProbeClose`, and no spindle or coolant, in a program of its own or
a setup's. `readback` reads `G38.2` (`Motion::probe`), and `check` takes
a probe's plunge as a straight-down `G38.2` from above it to below it at
its feed, the spindle stopped, and the move after it from where it
touched, which is not known, straight up. In `camcraft-session`,
`program_spec` posts a probe operation as its profile probes, its log
named after the operation (`probe_log_name`), and refuses one for a
profile that cannot (`LowerError::CannotProbe`, naming those that can,
`Profile::builtin_probing`). Golden programs: `linuxcnc-probe.nc`,
`grbl-probe.nc` and `linuxcnc-setup-probe.nc`, the probe before an
engraving in one program.

And holes found by a diameter with nothing picked, so a template's
drilling holds on another part. In `camcraft-core`, `HolePicking` has
`diameter`: every hole of the setup's models that wide along the tool is
taken, picked or not (`hole_targets`), and none that wide is
`HoleError::NoneAsWide`. `HolePicking` is no longer `Eq`, as it holds a
number.

And a setup can be turned on a rotary as it cuts (4-axis), as Fusion's
Wrap toolpath. In `camcraft-core`, `Setup::wrap` (`job::Wrap`, with an
optional `diameter`) puts it on one: its stock must be one cylinder lying
along the setup's X or Y, and it may not carry on from an earlier setup,
be corrected to a probed surface, pick faces or edges of the part, or have
operations other than engraving, slots, 2D profiles and pockets, V-carving
and Custom G-code (`JobError::Wrap`, `PrepareError::Wrap`). A `Frame` has
`wrap`, a `frame::Unrolled` (the axis the bar lies along and the radius it
is unrolled at, the wrap diameter's half or the bar's): the work zero is
moved onto the bar's axis across it (`setup::round`), the part is unrolled
about it (along, round at the radius, out from the axis), each triangle
split until no piece goes further round than keeps the straight line
between its corners within half the tolerance of the curve, the pieces
across the turn's seam put in on both sides of it; and the stock is the
bar unrolled, a box once round, from the axis out to its radius.
`Frame::shown` and `unshown` take a point of the paths to the model and
back, rolled round. Sketch curves are not unrolled: they are the flat
design, wrapped as drawn. The paths on a rotary are straight moves
(`Toolpath::straightened`, the strategy's arcs before linking, the
linker's after); `Toolpath::shifted` moves one. In `camcraft-sim`, a
`Stock` can be `rounded` (`stock::Round`: which of its grid's axes goes
round, and the radius), its grid a whole number of cells a turn
(`round_pitch`), and the run's pitch made so: a tool's tip at a point of
the unrolled frame stands on a radius, and each column round it, a ray
out from the axis at an angle to it, is cut down to where the tool's
underside first meets the ray (`stamp_round`), every move sampled as a
straight line in the unrolled frame, the seam no edge; a column the
tool's side leaves before its top keeps its material. Its volume is the
wedges'. `PackedStock` keeps the round, in its bytes too (after
everything else, so a stock kept before reads as it did). On a rotary
`sweep_watching` only sweeps: the shank and holder are not watched. In
`camcraft-post`, `[rotary]` has `feed` (`rotary::RotaryFeed`: inverse
time, the default, or degrees) and `unwind`, and a rotary
`continuous`; `wrap::Wrapping` is how a setup's unrolled path goes round
the profile's rotary about the bar's axis (`Wrapping::of`, refused as
`LowerError::NoRotaryFor` for a machine without one): A is the angle
round, by its sign, B minus that, the across axis at zero. `ProgramSpec`
has `wrap`; lowering then writes the moves as `Op::Turning` (X, Y, Z and
the rotary at once, its `F` worked out for the move: one over its minutes
along the material at the tool's tip, or units a minute with the degrees
counted as lengths), between `Op::InverseTime(true)` (`G93`) and `(false)`
(`G94`) when fed in inverse time, and the header says where the work
zero is (`tilt_notes`). Every block in inverse time says its `F`, to four
more places than a feed. `wrap::wound` moves a path round by whole turns
to start near a distance round. `readback::read_wrapped` reads a program
round a rotary back into the bar unrolled: the rotary with X, Y and Z in
`G0` and `G1` blocks, the across axis kept at zero, `G93`, and feeds
worked back to mm/min along the material, a move in inverse time without
its own `F` refused; `check` and `check_setup` read it so for a
`ProgramSpec` with `wrap`. In `camcraft-session`, `program_spec` gives a
setup's spec its wrap (Klartext refused, `LowerError::WrapNotWritten`),
leaving the rotary that turns it out of those positioned; `posted_path`
and `setup_paths` wind each operation's path to start within half a turn
of where the one before ended, or of zero with `unwind`, as it is posted;
and `travel_check` takes the frame, checks the linear axes along and out
from the bar, and the rotary's angles, as posted alone, against its
limits unless it is continuous. Golden programs: `linuxcnc-wrap-a.nc` and
`mach-wrap-a-degrees.nc`.

And on a rotary, parallel finishing is rotary finishing
(`strategy::rotary`): `StrategySpec::Parallel` on a wrapped setup goes
along the bar in passes a stepover apart round it, back and forth, for an
angle within 45 degrees of along it, else in one helix a stepover along
it a turn (`RotaryPattern`), from end to end of the part, at levels a
stepdown apart from the stock's top when one is given, the last on the
part: the first level on the part everywhere ends it. Each point's tip
is as near the axis as the tool comes without meeting the part on the simulator's grid (`rotary::Unrolled::tip`): each
column round from the tool is a ray out from the axis, met by the tool's
side `ρ sin dθ` across, so its tip may come no nearer than `ρ cos dθ` less
the tool's height there, the column taken no higher than where the ray
leaves the tool's side, rows round from the tool's taken while they could
still be met higher than the best yet. Points are sampled a quarter of
the tool's radius apart, again between two where the tip strays from
the line between them, in parallel, and simplified. `JobError::Wrap`
lets parallel finishing through, and `StrategyError::RotaryGrid` refuses a
part on a grid that does not go round in whole cells.

And the tool kinds a machinist's library has, each cut with its own
shape. In `camcraft-core`'s `cutter::tool`, two cutters more:
`Tapered`, a flat tip, a round corner and a straight flank tangent to it,
an angle from the axis, out to a radius (a tapered or tapered ball end
mill, a round-tipped V-bit, a face mill with a lead), its `h(d)` convex
as the cone's, so `DropTest for Tapered` is the cone's (a vertex
directly, an edge by the shared concave search, a facet where the
profile's slope is the plane's, `touches_slope`); and `Undercut`, a
head (a flat or ball `Bullnose`) on a narrower neck, a slot cutter's
disc or a lollipop's ball, not monotonic, dropped as its head.
`Cutter::grown` grows either by stock to leave as a bullnose grows, the
tapered end's corner by the stock and its flat by the radial stock past
it, which `EffectiveTool::of` uses for every cutter but a cone; and
`Cutter::swept_height_at` and `swept_max_radius`, which the simulator
sweeps (`path::simadapt`), take an undercutting head as only its neck's
width, since a height field cannot hold an undercut. `ToolError` has
`BadTaper` and `BadNeck`. In `job`, `ToolKind` has `Tapered`,
`FaceMill`, `Lollipop`, `SlotCutter`, `SpotDrill`, `Countersink` and
`Tap` (`name`, `is_cone`, `undercuts`, `is_pointed`), and `ToolSpec`
`taper_angle` (a taper or a lead, from the axis), `neck_diameter`, and a
tap's `pitch`, `hand` and `chamfer_length`, its `corner` now also a
tapered mill's, a face mill's inserts' and a V-bit's round tip;
`build` makes each its profile (a tapered mill and a face mill from
their diameter at the tip up their flute length; a tap a cone from its
thread's minor diameter, `5/8 H` a side, up its chamfer; a spot drill
and a countersink cones at 90° unless given), `check` asks for what
each needs, and `flat_diameter`, `cutting_diameter`, `head_height`,
`point_angle`, `spot_depth` and `shape_words` say what their names do.
Facing runs its passes off the stock only as far as the real tool's edge
reaches as deep as a level cuts (`LevelsAsked::deepest`, the radius there),
not by its widest: a face mill's at its face and its lead up that depth, a
flat end mill's its radius as before.
`above_flutes` checks an undercutting head's neck from the head up, and
a tapered mill's body as wide as its flank's top. `Operation::cannot_use`
(its kinds now in `cannot_use_kind` and `cannot_drill`) takes a face
mill only in facing and flat finishing, a tapered mill only in 3D
finishing and engraving, a lollipop or slot cutter only in a 2D profile
or a slot with no stepdown, a spot drill and a countersink in drilling,
chamfering and engraving, a round-tipped V-bit neither V-carving nor
chamfering, and a tap only tapping, at its pitch and hand; a flat end
mill or a drill still taps, and the run warns to make it a tap.
`StrategySpec::Drill` has `spot`, a diameter: each hole goes only as deep
as leaves it that wide at its top, with a pointed tool; `run::cut_past`
takes what a tap cuts past its hole, its thread, and a spot's or a
countersink's seat, as meant. On a rotary a tapered end is met radially
as any profile is, and a lollipop or slot cutter is refused
(`JobError::Wrap`). `passes` reads a
tool's corner and flat through `corner_radius` and `flat_diameter`. In
`camcraft-session`, `library::fusion_kind` and `fusion_shape`, shared by
the CSV and the JSON import, map Fusion's `tapered mill`, `face mill`,
`lollipop mill`, `slot mill`, `spot drill`, `center drill`, `counter
sink` and `tap right hand` and `tap left hand` to these, with their
taper (`TA`), neck (`DN`, `tool_shoulderDiameter`), pitch (`TP`) and
chamfer (`tip-length`), a spot drill no longer a plain cone; and
`finish` names a kind by `ToolKind::name`. Tests in
`cutter/tool/tests.rs`, `cutter/dropcutter/tests.rs`, `job/tests.rs`
(`kinds`), `run/tests/kinds.rs` and the two imports'.

And stock from a solid, as Fusion's "From solid". In `camcraft-core`,
`StockSpec::Solid` names a model of the job that is the stock: a setup's
only one, never one of its models (`JobError::SolidStock`). Anything that
needs only a bound takes its box; `prepare_from` lays it out in the
setup's frame (`Prepared::solid`), and `Prepared::fresh_stock` starts the
run, and a pattern's or rest machining's copies, from it, as from what an
earlier setup left. In `camcraft-sim`, `Stock::from_solid` samples a mesh
into columns, each from the lowest it comes over the column's middle to
the highest, the bottom kept as the underside (`part::extremes` rasterises
both). An adaptive clearing whose setup starts from such material, a
solid's or what an earlier setup left (`Prepared::starts_shaped`), is
worked out as a pattern's one copy is, its material at each level where
that stands above it rather than its blocks, so it does not clear a cast
pocket, or what the side before cut, again. In `camcraft-session`, the
run's notes name the solid.

And fixtures, which are never cut. In `camcraft-core`, a `Setup` has
`fixtures`, models of the job that are neither its parts nor its stock
(`JobError::BadFixture`), laid out in its frame (`Prepared::fixtures`).
`run::fixtures` lays each one's top on a grid once, grown a cell, no finer
than 0.2 mm (`Fixture::of`), and checks every move of every operation with
paths, after any probe correction, against each, at each placement of the
part (`Setup::placement_offsets`), with the tool whole (`whole_tool`: the
cutter as cylinders up its tip, then what stands above its flutes) through
`camcraft_sim::stock::Reach`, as the holder is checked against the
material. What meets one is an `OperationResult::fixtures` entry
(`FixtureHit`: the fixture, the placement, what of the tool, the first
move, how many, how deep, whether at rapid), which fails `is_clean`.
`Passes::avoid_fixtures` makes a 3D finish leave out the cuts where the
tool would meet a fixture, as `use_holder` does the part (`leave_out`, which
both now share). In `camcraft-session`, the checks add `CC113` (a fixture
met: fails) and `CC219` (cuts left out to avoid one).

And several parts in one setup, each at its own work offset. In
`camcraft-core`, a `Setup` has `placements` (`Placement`: a `wcs`, `at`,
how far its part is from the job's along the setup's axes, and its
`work_offset` in machine coordinates, else the first's moved by `at`;
`Setup::placed` gives them, or the one part at `wcs`; two on one offset are
`JobError::SameWorkOffset`) and `placement_order` (`PlacementOrder`:
`by-tool` or `by-part`). Each part is cut the same in its own coordinates,
so the run is the first's; the fixtures are checked at each placement's
`at`. In `camcraft-post`, `lower_setup` writes the work offset again where
two sections' differ, the tool kept turning: sent clear by the profile's
`safe_retract` (`Op::SafeRetract`) first, the next section starting from a
place not known; a profile without one is refused
(`LowerError::OffsetsWithoutRetract`). `readback::check_setup` reads it as
it is. In `camcraft-session`, `setup_specs` gives each operation at every
placement in the order asked (by tool, each run of operations with one tool
at every placement), `placed_specs` and `placed_program` post one operation
at each, `travel_check` checks each placement where its offset is, saying
which, and a run warns of placements whose stocks overlap. A golden program
of two vises by tool (`linuxcnc-setup-vises.nc`).

And one test: ogeom is 0.11.1 here, which printCAD's shapes need, and it
meshes `round_edge_loose_fit.step`'s seam that 0.11.0 drew off the torus, so
`a_loose_fit_is_refused_for_ogeoms_seam` became
`a_loose_fit_reads_within_the_tolerance`, as its comment asked.

To take a newer camcraft, copy those crates over these, then make the same
changes again; `diff -r` against the commit above shows them.
