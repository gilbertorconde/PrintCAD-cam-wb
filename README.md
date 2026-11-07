# CAM: 3-axis CNC milling for printCAD, with camcraft

A workbench package for [printCAD](https://github.com/gilbertorconde/printCAD)
that machines the document's solids: make a setup on the bodies, add
operations, and each setup's toolpaths are worked out, simulated against the
material, checked, and saved as G-code. The machining is
[camcraft](https://github.com/dan335/camcraft)'s, run natively on every core.

![Adaptive clearing played back in printCAD: the material cut so far, the toolpath, and the operation's heights](docs/screenshots/playback.png)

> [!WARNING]
> **Use at your own risk.** camcraft is pre-alpha and has not been proven on a
> real machine. Its G-code can be wrong in ways that crash the machine, break
> tools, ruin the part or hurt someone. Check every program yourself before it
> cuts: in a simulator you trust, then as an air cut above the stock, with a
> hand near the e-stop. The software comes with no warranty (see the licences).

## Based on camcraft

The machining in this workbench is
[camcraft](https://github.com/dan335/camcraft) by
[dan335](https://github.com/dan335): open-source CAM for 3-axis CNC routers,
written in Rust, whose operations, settings and programs follow Fusion's. Its
engine is copied into `camcraft/` (from camcraft 0.2.0; `camcraft/README.md`
says which commit and what is changed here), under its MIT or Apache-2.0
licences. Bugs in the toolpaths, the simulation or the posts are best fixed,
and reported, there; this package is the printCAD side: the document, the
panels, the drawing and the helper that runs camcraft.

## What it does

- **Setups.** A setup (`io.github.gilbertorconde.cam.setup`) names the bodies
  it machines, its stock (a box round them with margins, or one of a fixed
  size, or a cylinder: round bar up the tool, as a disc, or lying along the
  machine's X or Y, round the bodies with a margin all round and at each end,
  or of a fixed diameter and length centred on them), how the part sits (the
  tool axis along any of the six directions, or tilted, below),
  its work zero (a corner or the middle of the stock or of the bodies, or the
  document's origin, plus an offset), its post and its tolerance. New setup on
  the toolbar makes one on the selected body, or on every shown one.
- **Tilted setups (3+2).** A setup's tool axis can be any direction: out of
  a flat face picked on the part, along a straight edge picked on it (either
  way), the document's Z tilted by angles about its X and then its Y, or a
  direction typed in; each number takes a formula. Machine X is a document
  axis made square to the tool (X unless the tool lies nearly along it), or
  a straight edge picked on the part. Everything then happens square to the
  tool: the toolpaths, the heights, the simulation and the checks, and the
  stock, whose box round the bodies is round them as the tool sees them,
  its margins along the setup's axes, as a fixed box's size and a cylinder's
  axis are. A post with rotary axes stands the part up for each program,
  positioning only: `G0 A.. B..` before it cuts, with the tool sent clear
  first. Without them the part is fixtured tilted by hand, and the program's
  header names the tilt. Either way the program's coordinates are in the
  tilted frame: **set the work offset in the tilted frame**, or use the
  controller's tilted-plane feature where it has one. A tilt the rotaries
  cannot reach, or only past their limits, is a failed check that says
  so.
- **Operations**, after Fusion's: adaptive and pocket clearing; parallel,
  scallop, contour, flat, spiral, radial and morphed spiral finishing; 2D
  profiles and pockets; facing; drilling: at once, in pecks backing out,
  breaking the chip, or tapping, right- or left-hand; engraving along curves
  and edges, open or closed, at a depth (0.5 mm unless set), each ramped
  down along itself, round a closed one and back and forth along an open
  one, so it never lifts between its levels, its arcs written as `G2` and
  `G3`; slots along the same curves and edges, as wide as the tool, 3 mm
  deep unless set and a millimetre a level, both ways (back and forth,
  ramping down as an engraving does) or one way (each level from the
  start, lifting back, entered by a ramp along the slot or a plunge),
  their arcs arcs at every level; deburring and chamfering with a chamfer
  mill or V-bit along picked edges, a pocket's rim or a part's outline,
  each from its own height: as wide as asked (0.5 mm unless set), the
  tip a tip offset below the chamfer so its flank cuts it, the tool's
  centre beside the edge away from the material, as the part either side
  says, or flipped, and a cone too small for it a failed check; and
  V-carving closed areas, letters among them, with a V-bit down
  their middle, as deep as each is wide: past its deepest it cuts the walls
  that deep and leaves the flat floor between, and says so, for a flat end
  mill to clear. And Custom G-code, Fusion's Manual NC: lines of your own,
  typed a row at a time, written into the program where the operation
  stands in its setup: before the next operation's start (its stop, tool
  change and spindle), or, last in the setup, after the one before ends.
  It has no tool and no toolpath, and the simulation does not see it.
  Its lines take the same `@name@` placeholders as a profile's blocks
  (below), filled from the program it goes into. They may not move the
  machine or change a mode: an axis word, any G code but `G4` (a dwell),
  `M6`, `M2` or `M30` is a failed check, so a `G91` cannot make the rest of
  the program cut somewhere else; a subprogram or macro call (`O`, `M98`,
  `G65`, `M100` up) is a warning, as nothing reads what it does. After the
  lines the program stops the spindle and the coolant, and takes where the
  machine is as not known, as after a tool change. An operation
  (`io.github.gilbertorconde.cam.operation`) belongs to a setup and runs in the
  document's order, each against what the one before left. Drag one in the
  tree to run it sooner or later, or under another setup.
- **Patterns.** Any operation's toolpath can be repeated before it is
  linked: in rows along X and Y (so many, so far apart), round a centre
  (so many, over an angle, a whole turn by default), and mirrored about a
  line along the setup's X or Y through a point, the original kept; given
  together, the rows are copied round the centre and all of them
  mirrored. Each number takes a formula. Every copy is linked, cut into
  the material, simulated and checked, and drawn. A mirrored copy would
  cut conventional where the original climbs, so each of its rings, and
  each pass all at one level, is run the other way round; what cannot be
  (a ramp, an adaptive pass, steered by the material ahead of it) is said.
  A copy that goes further outside the stock than the original is a
  warning.
- **Picking in the view,** as camcraft's: a pick row in the panel arms it,
  and while it is armed, hovering shows what a click takes. An operation
  takes faces, and edges by the loop they go round at their height (an edge
  that is not level, alone; an engraving's, each alone, and those end to
  end followed as one); a click on a picked one takes it out. It can take
  the document's sketches too, ticked in its panel: what their closed loops
  enclose, or the loops themselves for an engraving. A sketch's text is its
  letters' outlines. Its open curves close nothing, and a package cannot
  read them, so only picked edges give an engraving an open curve. A setup's
  work zero can be a corner or a round edge's centre picked on the part,
  its tool axis a flat face or a straight edge, and its machine X a
  straight edge.
- **What an operation machines, drawn** while it is edited: its boundary as
  chosen, where the tool's centre may go against it, and, hatched, what the
  tool cuts outside the part or beside a 2D outline.
- **Heights**, as Fusion's Heights tab: the clearance, the retract, and the top
  and bottom it cuts between, each measured from the stock, the part, the work
  zero or the picked faces and edges. The view shows them as planes across the
  stock while the operation is edited.

  ![An adaptive clearing open for editing: its toolpath, its heights as planes, and its settings](docs/screenshots/operation.png)

- **Linking**, as Fusion's Linking tab: retract feed, retraction, safe and
  stay-down distances, ramps, the entry (helix, plunge or ramp), leads on and
  off each ring, and reordering. A value left unset is the strategy's own, and
  the panel says what that comes to for the tool.
- **Passes**, as Fusion's Passes tab: besides the stepover, depths, feeds and
  stock to leave, each strategy's own settings (slopes, order, centre and
  radii, spokes, pass extension, flat areas; a drill's peck, chip break
  distance and dwell, or a tap's pitch and hand), the deepest step,
  tabs to hold a profiled part, and finishing passes along a pocket's walls.
  Hovering the stepover, stepdown, stock to leave or a height shows a sketch
  of what it measures, as camcraft's hints do.

  ![The stepover's hint: two passes side by side, the stepover between their centres](docs/screenshots/hint.png)
- **Tools.** Preferences › CAM holds the tool library, which starts as
  camcraft's twenty common router bits, each with presets for the materials
  it suits. The chosen tool's kind, dimensions and presets can be edited,
  and a Fusion tool library exported as CSV imports into it. A setup takes a
  copy of a tool when an operation first cuts with it, so a document never
  depends on the machine it is opened on.
- **Status in the tree.** Each setup and operation shows how its run went:
  running (with how far), done, warnings, failed, or out of date.
- **Checked against the material.** Every setup is simulated: a gouge, a rapid
  through material and the other failures camcraft checks for are listed in
  the setup's and the operation's panels, and a setup that fails is not
  posted (Save anyway exists, and says what it is).
- **Playback,** as camcraft's: while a setup or one of its operations is
  open, a transport under the view plays its run back. A timeline has a span
  per operation as long as it takes to run, with failed checks marked filled
  and warnings hollow; click or drag on it to go anywhere. Buttons go to the
  start, the operation before, a step back, play backwards, play, a step on,
  the next operation and the end, at ¼× to 10,000×. Playing stops by itself
  at problems (on unless turned off) or after each operation. The clock is
  machining time: moves at their feeds, rapids at 5000 mm/min. Keys: Space
  plays and pauses; J, K and L shuttle back, pause and forward (again for
  faster); Left and Right step (Shift: ten); Up and Down go to the operation
  before and after; Home and End; Minus and Equals change the speed.
  Once playing, the material is swept again at full resolution by the helper
  kept running for it (up to 600 points a side), and steps go a move at a
  time; until then, and where it cannot, the run's coarse moments play.
  The helper runs the setup from the same request as the run shown, even
  when the document has changed since without the setup running again,
  and its moves must end when the run's do; when they do not, playback
  stays coarse and says so, rather than step and seek to the wrong moves.
  The panel lists the program being played (the operation's own, or in a
  setup's panel the one under way): the lines round the playhead's,
  numbered, with its line marked, following it as it plays. Click a line's
  number to take the playhead to its move.

  ![3D finishing half way: the passes still to run, over the material roughing left](docs/screenshots/finishing.png)

- **Drawn over the model.** The stock's box or cylinder, the work zero, each operation's
  toolpath (rapids dashed while it is edited), and the material left.
- **G-code through the save dialog,** one program for the whole setup, as
  Fusion posts one, or one program per operation (every one of a setup into
  a folder at once), as the setup's Programs says, for GRBL,
  Masso, Centroid CNC12 (Avid), LinuxCNC, Fanuc (0i, 31i), Haas (NGC) or
  Mach3 and Mach4, or in Klartext for a Heidenhain TNC, each in millimetres
  or inches, or through a
  machine profile of your own in camcraft's TOML, which a setup carries a
  copy of. A setup's program has one header and one footer, and each
  operation in order under a comment naming it: the tool is changed only
  where it changes, the same tool kept turning, at the next operation's
  speed where that is another, and the coolant changed only where it
  changes. Between operations the tool goes up clear as the profile's
  `safe_retract` takes it, or, with none, straight up as high as either
  operation goes, across, and down. The work offset is the setup's, and
  written once. Custom G-code between two operations
  is written with the spindle stopped and the tool clear, and the next
  starts again after it, its tool changed. A profile with
  `optional_stop = true` stops between every two operations (`M1`), the
  spindle stopped first. The program is named after the setup and numbered
  as its Program number says (`@number@`, 1001 unless set); one per
  operation is named `setup-01-operation` and numbered on from it, one a
  program. The listing beside playback lists the setup's program whole,
  each line followed to the operation and the move it makes.
  Fanuc's and Haas's programs are tapes, `%` round them and an `O`
  number first, the setup's (`O1001`, Haas's `O01001`), and call up the
  next tool after a change (`T5` after `T3 M6`), where the profile says
  `preselect_tool = true`. A
  Heidenhain program (`.h`) is Klartext, not G-code (`dialect =
  "heidenhain"`): numbered blocks from `BEGIN PGM` to `END PGM`, named as
  its file is, which the control insists on, once however many operations
  the setup has, the stock once
  as `BLK FORM`, `TOOL CALL 3 Z S18000`, `L X+10 Y+5 R0 F1200` and `FMAX`,
  arcs by their centre (`CC`, then `C .. DR+`), a helix by its angle (`CP
  IPA+360 IZ-1`), the work offset as a preset row (`CYCL DEF 247`, G54 its
  first), feeds in tenths of an inch a minute in inches, and holes as
  `CYCL DEF 200`, `203` and `207` called at each by `M99`. It is read back,
  as every program is, and the same checks run on it. A profile can give the spindle's range (`spindle = [6000, 24000]`),
  and an operation whose tool turns outside it is warned about. It can give
  its travel, in the program's units, in the machine's own coordinates:

  ```toml
  [travel]
  coordinates = "machine"   # as G53 moves; left out, from the work zero
  x = [0, 600]              # any of x, y and z; an axis left out goes anywhere
  y = [-400, 0]
  z = [-120, 0]
  ```

  A setup then says where its work zero is on the machine, what its work
  offset holds (Machine X, Y and Z in its panel, each taking a formula), and
  its programs are checked there. With tool length offsets (`G43`, a
  profile's `length_offset`), Machine Z is the spindle's gauge line at the
  work zero, and each tool's length from the gauge line is added: what
  sticks out of its holder (its stickout, else its overall length) and the
  holder's segments, when it has one; without a holder the gauge line is
  taken at the collet. A tool of no known length leaves Z unchecked, and a
  note says so. Without length offsets, Machine Z is the tip's, touched off.
  A setup that does not say where its work offset is is not checked, and a
  note says so. A `G53` move of the profile's own (`safe_retract = ["G53 G0
  Z0"]`, a tool change, the park at the end) must be inside the travel, or
  the profile does not load. A tilted setup is checked along its own axes,
  which are the machine's once its rotaries have stood the part up, or as
  it is fixtured: its work offset is the one set with the part turned.
  The older form, without `coordinates`, still loads and means what it did:
  ranges from the work zero as you set it, whatever the work offset.
  A program that goes past the travel (its rapids and arcs counted) is a
  failed check on the operation that does, saying which axis, how far under
  or over, and in what coordinates: past its limits a machine stops part-way
  through the cut, or crashes into them, so it is refused before it gets
  there, like the simulator's failures (Save anyway still writes it).
  A setup picks its work offset from those its post has: `G54` to `G59`,
  and any past them the profile lists (`extended_offsets = ["G59.1",
  "G59.2", "G59.3"]` on LinuxCNC, Fanuc's `G54.1 P1` to `P48`, Haas's
  `G154 P1` to `P99`), which its programs select. The lines a profile gives, `header`, `footer`,
  `safe_retract`, `end_home` and its own tool change, `tool_change_lines`
  (written in place of `T.. M6` or the pause, and naming `@tool@`), take
  placeholders: `@name@` (the program), `@number@` (its number),
  `@operation@`, `@strategy@`,
  `@tool@`, `@tool_comment@`, `@rpm@`, `@spindle@` (`M3` or `M4`),
  `@coolant@` (`flood`, `mist`, `off`), `@coolant_code@` (`M8`, `M7`, `M9`),
  `@units@`, `@units_code@` (`G21`, `G20`), `@wcs@` (`G54`…) and `@date@`
  (the day it was posted); `@@` is an `@`, and `{name}` still the program's
  name. In a setup's program each operation's lines are filled with its own,
  the header with the first's and the footer with the last's. A placeholder that is not one makes the profile invalid: adding it
  says which and on which line, and a setup carrying a copy that no longer
  loads says so in its panel. A profile
  that takes canned cycles (`drill_cycles = true`) writes each hole as one:
  G81, G82 with a dwell, G83 pecking, G73 breaking the chip (backing off as
  far as the controller is set to), and G84, or G74 left-hand, tapping at the
  pitch times the spindle's speed; without them holes go as plain moves. The
  LinuxCNC, Centroid CNC12 and Masso posts write them (Masso's G82 pause in
  milliseconds, `dwell_milliseconds = true`, as Fanuc's); the others write
  plain moves, GRBL having no cycles. A tap goes only as the controller
  taps, since no plain move keeps the feed to the spindle as it reverses at
  the bottom, and `tap` says how, whether or not the other cycles are on:
  `"g84"` (G84 and G74, the default with cycles; in Klartext, cycle 207,
  which `"cycle"` also names), `"m29"` (G84 and G74 after `M29 S..`, Fanuc's
  rigid tapping), `"g33.1"` (rigid tapping a hole at a time, the spindle
  reversing itself: `G33.1 Z.. K` with the pitch) or `"none"`. LinuxCNC taps
  with G33.1, which needs a spindle encoder; Fanuc in rigid mode after
  `M29`; Haas with G84 and G74, which it taps rigidly by itself; Heidenhain
  with cycle 207; CNC12 with G84 and G74, which needs its rigid tapping (an
  encoder and the Pro licence on an Acorn) or a floating tap holder. Masso,
  GRBL and Mach cannot (Mach4's G84 is for a floating holder, not kept to
  the spindle): a tapping operation posted for them is a failed check that
  says so, and names the machines that can. A tap is kept in the tool
  library as a flat end mill its size, and turns the way its thread does.
  A profile can name its rotary axes, for 3+2 machining:

  ```toml
  [rotary]
  axes = ["A", "B"]     # from the machine's base out: A carries B, B the part
  [rotary.A]
  about = "x"           # the machine axis it turns about: x for A, y for B unless said
  sign = -1             # 1: a positive angle turns the part right-handed about it
  limits = [-30, 110]   # degrees; a full turn either way unless said
  ```

  With every rotary at zero the part sits as modelled. With two, the setup's
  machine X must lie where they leave the machine's X; a check says which
  way that is when it does not. A profile with rotaries needs a
  `safe_retract`, since the part turns under the tool.
- **Formulas.** Every number of a setup and of an operation takes a formula.
- **Script commands**, for scripts and AI agents:

```lua
local cam = pc.io.github.gilbertorconde.cam
local setup = cam.setup{bodies = {"<body id>"}, margin = 3, post = "grbl"}
local each = cam.setup{bodies = {"<body id>"}, programs = "operation", number = 2000}
                                        -- one program an operation, O2000 on
local disc = cam.setup{bodies = {"<body id>"}, stock = "cylinder", axis = "tool",
                       diameter = 80, length = 25}  -- or margin = 3 round the bodies
local side = cam.setup{bodies = {"<body id>"}, tool_axis = "-y", machine_x = "+x"}
local placed = cam.setup{bodies = {"<body id>"}, post = "linuxcnc", wcs = "G59.1",
                        work_offset = {x = 250, y = -180, z = -95}}  -- in machine coordinates
local tilted = cam.setup{bodies = {"<body id>"}, tilt = {x = 30, y = 0}}
                                        -- or tool_axis = {x = 0, y = -0.5, z = 0.866}
local rough = cam.operation{setup = setup.feature, strategy = "adaptive", tool = "1/4 upcut",
                           heights = {bottom_from = "model-top", bottom = -6}}
local name = cam.operation{setup = setup.feature, strategy = "engrave", tool = "60° V-bit",
                           sketches = {"<sketch id>"}, passes = {depth = 0.4}}
local keyway = cam.operation{setup = setup.feature, strategy = "slot", tool = "1/4 upcut",
                             sketches = {"<sketch id>"}, pattern = "directional",
                             passes = {depth = 4}, stepdown = 1}
local three = cam.operation{setup = setup.feature, strategy = "pocket", tool = "1/4 upcut",
                            transform = {linear = true, x_count = 3, x_spacing = 25,
                                       mirror = true, mirror_axis = "x", through_y = 0}}
local deburr = cam.operation{setup = setup.feature, strategy = "chamfer", tool = "90° V-bit",
                             passes = {chamfer_width = 0.8, tip_offset = 0.3}}
local letters = cam.operation{setup = setup.feature, strategy = "v-carve", tool = "60° V-bit",
                              sketches = {"<sketch id>"}, passes = {max_depth = {value = 2, given = true}}}
local clamps = cam.operation{setup = setup.feature, strategy = "custom-gcode",
                             gcode = "M0 (move the clamps for @operation@)\nM7"}
cam.status{id = setup.feature}          -- queued, running, ready (with its checks), failed
cam.import_tools{text = fusion_csv}     -- a Fusion tool library, exported as CSV
cam.add_post{text = profile_toml}       -- a machine profile of your own
local program = cam.gcode{id = setup.feature}  -- the setup's; an operation's id gives it too
local own = cam.gcode{id = each_op}             -- one program an operation: the operation's
```

## How it works

The package itself (WebAssembly, in printCAD's sandbox) keeps the setups and
operations, draws and shows what comes back, and saves the programs. A setup
runs as a job that hands it to the package's native helper,
`camcraft-helper`: a camcraft job with each body's B-rep as printCAD keeps it
(ogeom's native format). The helper tessellates the solids, runs camcraft's
one engine path (toolpaths, linking, simulation, checks, post) and answers
with the toolpaths, the checks and the programs. One setup runs at a time;
Stop kills the helper.

The package asks for the `helper` capability, to run that program outside the
sandbox. In a browser printCAD runs no helpers, so the package does nothing
useful there.

## What it does not do yet

- Rotary axes only position the part, before each program cuts: nothing
  moves on more than three axes at once, and no tilted-plane code (`G68.2`
  and the like) is written.
- A setup's run is not kept in the document. The helper keeps the newest 64
  runs in the user's cache folder (`printcad-cam`; `CAM_HELPER_CACHE` names
  another, or `off`), so a document opened again on the same machine shows
  its runs at once; elsewhere they run again.
- Picked faces and edges follow camcraft's rules: a pick is a point on the
  face, found again on the solid by that point.
- Custom G-code is checked word by word, not run: what a subprogram or
  macro it calls does is not known, only warned about. A profile's own
  blocks are written as it gives them, and only the built-in posts' are
  read back in the tests.
- The Fanuc, Haas, Mach and Heidenhain posts are written from their
  manuals and from how Fusion's posts write for them, and read back in the
  tests; none has been run on its machine yet. Klartext's arcs out of the
  XY plane turn as G-code's `G18` and `G19` do, a helix about X or Y is
  written as straight moves, and cycle 203 backs off 0.25 mm to break the
  chip, whatever the operation asks: `G73` says no distance either. There
  is no Siemens Sinumerik post yet.
- Travel is checked as a box of the program's moves, in the setup's
  frame: where the rotaries carry the part while they turn, the holder and
  the spindle against the part or the table, and a controller's own
  tool-change position (outside a `G53` move in the profile) are not
  checked.

## Install it

It needs printCAD 0.11 or later.

- From a release: download the `.pcbench` file from the
  [releases](https://github.com/gilbertorconde/PrintCAD-cam-wb/releases)
  and use Preferences › Workbench packages › Install from a file….
- From this repository: type its address,
  `https://github.com/gilbertorconde/PrintCAD-cam-wb`, on the same page and
  press Install. printCAD takes the latest release, checks for newer ones and
  offers to update.

A release carries the helper for Linux (x86_64, aarch64), macOS (Apple
silicon, Intel) and Windows (x86_64).

## Build it

With [rustup](https://rustup.rs), the toolchain and the `wasm32-wasip2`
target come from `rust-toolchain.toml`:

```sh
scripts/pack.sh                        # dist/io.github.gilbertorconde.cam-<version>.pcbench, with this machine's helper
cargo test --release -p cam -p camcraft-helper
cargo test --release --workspace       # camcraft's own tests too
```

To see what the package asks of the helper, start printCAD with
`CAM_HELPER_DUMP=/tmp/request.json`: the helper keeps the last request there,
and `camcraft-helper < /tmp/request.json` runs it again by hand.

## Release it

1. Set the same version in `bench.toml` and `Cargo.toml`.
2. Commit, tag and push: `git tag v0.2.0 && git push origin v0.2.0`.

The Release workflow builds the helper on each system, runs the tests, packs
them all with the package, checks that the tag names that version, and
publishes a GitHub release with the `.pcbench` attached. A tag with a suffix
(`v0.3.0-beta.1`) becomes a prerelease, which printCAD does not offer as an
update. Every push and pull request runs the CI workflow: format, clippy,
tests, the helpers, and the package as a build artifact.

## Layout

| Path | What it is |
| --- | --- |
| `bench.toml` | the manifest: id, version, feature kinds, capabilities |
| `src/lib.rs` | the workbench: tools, events, commands, the job |
| `src/data.rs` | a setup's and an operation's data, and the camcraft job they make |
| `src/panel.rs` | the task panels and the preferences page |
| `src/draw.rs` | what the view draws |
| `src/runs.rs` | the setups being run, one at a time |
| `protocol/` | what the package and the helper say to each other |
| `helper/` | `camcraft-helper`, the native program that machines |
| `camcraft/` | camcraft's engine, copied in (its README says from where) |
| `scripts/pack.sh` | build and pack a `.pcbench` |
| `.github/workflows/` | CI, the helpers on each system, a release for every `v*` tag |

## License

MIT or Apache-2.0, at your option. camcraft is dan335's, under the same
licences; `camcraft/NOTICE` says what else it holds.
