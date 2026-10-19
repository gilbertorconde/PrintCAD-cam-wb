# CAM: 3-axis CNC milling for printCAD, with camcraft

A workbench package for [printCAD](https://github.com/gilbertorconde/printCAD)
that machines the document's solids: make a setup on the bodies, add
operations, and each setup's toolpaths are worked out, simulated against the
material, checked, and saved as G-code. The machining is
[camcraft](https://github.com/dan335/camcraft)'s, run natively on every core.

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
  size), how the part sits (the tool axis along any of the six directions),
  its work zero (a corner or the middle of the stock or of the bodies, or the
  document's origin, plus an offset), its post and its tolerance. New setup on
  the toolbar makes one on the selected body, or on every shown one.
- **Operations**, after Fusion's: adaptive and pocket clearing; parallel,
  scallop, contour, flat, spiral, radial and morphed spiral finishing; 2D
  profiles and pockets; facing; drilling. An operation
  (`io.github.gilbertorconde.cam.operation`) belongs to a setup and runs in the
  document's order, each against what the one before left.
- **Heights**, as Fusion's Heights tab: the clearance, the retract, and the top
  and bottom it cuts between, each measured from the stock, the part, the work
  zero or the picked faces and edges. The view shows them as planes across the
  stock while the operation is edited.
- **Linking**, as Fusion's Linking tab: retract feed, retraction, safe and
  stay-down distances, ramps, the entry (helix, plunge or ramp), leads on and
  off each ring, and reordering. A value left unset is the strategy's own, and
  the panel says what that comes to for the tool.
- **Passes**, as Fusion's Passes tab: besides the stepover, depths, feeds and
  stock to leave, each strategy's own settings (slopes, order, centre and
  radii, spokes, pass extension, peck and dwell, flat areas), the deepest step,
  tabs to hold a profiled part, and finishing passes along a pocket's walls.
- **Tools.** Preferences › CAM holds the tool library, which starts as
  camcraft's twenty common router bits, each with presets for the materials
  it suits. The chosen tool's kind, dimensions and presets can be edited,
  and a Fusion tool library exported as CSV imports into it. A setup takes a
  copy of a tool when an operation first cuts with it, so a document never
  depends on the machine it is opened on.
- **Checked against the material.** Every setup is simulated: a gouge, a rapid
  through material and the other failures camcraft checks for are listed in
  the setup's and the operation's panels, and a setup that fails is not
  posted (Save anyway exists, and says what it is).
- **Playback.** An open setup plays its run back: the material coming off,
  the tool where it is, and the toolpath as far as it has gone, with a slider
  to stop at any moment.
- **Drawn over the model.** The stock's box, the work zero, each operation's
  toolpath (rapids dashed while it is edited), and the material left.
- **G-code through the save dialog,** one program per operation, for GRBL,
  Masso or Centroid CNC12 (Avid), in millimetres or inches, or through a
  machine profile of your own in camcraft's TOML, which a setup carries a
  copy of.
- **Formulas.** Every number of a setup and of an operation takes a formula.
- **Script commands**, for scripts and AI agents:

```lua
local cam = pc.io.github.gilbertorconde.cam
local setup = cam.setup{bodies = {"<body id>"}, margin = 3, post = "grbl"}
local rough = cam.operation{setup = setup.feature, strategy = "adaptive", tool = "1/4 upcut",
                           heights = {bottom_from = "model-top", bottom = -6}}
cam.status{id = setup.feature}          -- queued, running, ready (with its checks), failed
cam.import_tools{text = fusion_csv}     -- a Fusion tool library, exported as CSV
cam.add_post{text = profile_toml}       -- a machine profile of your own
local program = cam.gcode{id = rough.feature}
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

- A setup's run is not kept in the document. The helper keeps the newest 64
  runs in the user's cache folder (`printcad-cam`; `CAM_HELPER_CACHE` names
  another, or `off`), so a document opened again on the same machine shows
  its runs at once; elsewhere they run again.
- The helper reports how far a run is (`progress <done> <total>` on its
  standard error), but printCAD does not read a helper's output until it
  ends, so the panel shows only that it is running.
- Picked faces and edges follow camcraft's rules: a pick is a point on the
  face, found again on the solid by that point.
- printCAD offers a package no dialog to open a file, so a Fusion CSV or a
  machine profile is named by its path, and the helper reads it.

## Install it

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
