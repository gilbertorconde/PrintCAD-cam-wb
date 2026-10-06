# CAM: pocket toolpaths for printCAD

A workbench package for [printCAD](https://github.com/gilbertorconde/printCAD)
that mills pockets: pick a sketch with a closed outline, set the tool and
the depths, and save the toolpath as G-code.

## What it does

- **Pocket operations.** A pocket is a feature of its own kind
  (`io.github.gilbertorconde.cam.pocket`) that builds no solid: the outline it
  clears, the stock's top and the depth below it, the tool and how it cuts.
  New pocket on the toolbar makes one from the selected sketch; its tree
  entry has **Edit pocket**, and a double click opens it too.
- **Zig-zag clearing.** The toolpath runs back and forth in rows no further
  apart than the stepover. The tool's whole disc stays inside the outline
  and clear of any island (a loop inside the outline), and the tool lifts
  only where the step to the next row would cut what it should not. The
  rows are cut again at every depth, top down, no pass deeper than the
  depth per pass.
- **A job with progress and Stop.** The toolpath is worked out away from
  the window; the task panel shows its progress and a Stop button, and it
  is worked out again whenever a number it depends on changes.
- **Drawn over the model.** The outline draws dashed at the stock's top and
  the toolpath at the floor, the edited operation's in the accent colour.
- **G-code through the save dialog.** Save G-code… writes millimetres,
  absolute coordinates, rapids at the clearance over the stock, plunges at
  the plunge rate and cuts at the feed rate.
- **A tool table.** Preferences › CAM keeps the tools (name, diameter,
  feed, plunge, spindle speed) and the clearance. An operation takes a copy
  of its tool's numbers when the tool is picked, so a document never
  depends on the machine it is opened on.
- **Formulas.** Every number of an operation (top, depth, depth per pass,
  tool diameter, stepover, feed, plunge, spindle, clearance) takes a
  formula, as printCAD's own fields do.
- **Script commands**, for scripts and AI agents:

```lua
local op = pc.io.github.gilbertorconde.cam.pocket{sketch = "<sketch id>", depth = 3, tool_diameter = 6}
local op2 = pc.io.github.gilbertorconde.cam.pocket{outline = {{{0, 0}, {40, 0}, {40, 30}, {0, 30}}}, depth = 3}
pc.io.github.gilbertorconde.cam.toolpath{id = op.feature}   -- where the toolpath stands
local program = pc.io.github.gilbertorconde.cam.gcode{id = op.feature}
```

## What it does not do yet

- Only zig-zag clearing: no passes that follow the outline, and no
  finishing pass along the walls.
- One G-code file per operation; operations are not chained into one
  program.
- The outline's sketch must lie square to Z: the tool comes down from
  above.
- Outlines with splines are refused; lines, arcs, circles and ellipses are
  followed.

## Install it

- From the store: Preferences › Workbench packages › Browse lists CAM.
- From a release: download the `.pcbench` file from the
  [releases](https://github.com/gilbertorconde/PrintCAD-cam-wb/releases)
  and use Install from a file… on the same page.
- From this repository: type its address,
  `https://github.com/gilbertorconde/PrintCAD-cam-wb`, on the same page and
  press Install. printCAD takes the latest release, checks for newer ones
  and offers to update.

The workbench loads at once. It asks for the save dialog, to write G-code
where you choose.

## Build it

With [rustup](https://rustup.rs), the toolchain and the `wasm32-wasip2`
target come from `rust-toolchain.toml`:

```sh
scripts/pack.sh                                  # dist/io.github.gilbertorconde.cam-<version>.pcbench
cargo test --target x86_64-unknown-linux-gnu     # the toolpath and G-code logic
```

## Release it

1. Set the same version in `bench.toml` and `Cargo.toml`.
2. Commit, tag and push: `git tag v0.2.0 && git push origin v0.2.0`.

The Release workflow runs the tests, builds the package, checks that the
tag names that version, and publishes a GitHub release with the `.pcbench`
attached. A tag with a suffix (`v0.3.0-beta.1`) becomes a prerelease,
which printCAD does not offer as an update. Every push and pull request
runs the CI workflow: format, clippy, tests, and the package as a build
artifact.

## The SDK

The package builds on
[`printcad-bench-sdk`](https://crates.io/crates/printcad-bench-sdk) from
crates.io, `0.1` for the `printcad:workbench@0.1` contract. Its guide is
[docs/PLUGINS.md](https://github.com/gilbertorconde/printCAD/blob/master/docs/PLUGINS.md).

## Layout

| Path | What it is |
| --- | --- |
| `bench.toml` | the manifest: id, version, feature kind, capabilities |
| `src/lib.rs` | the workbench: operations, task panel, commands, settings, the job |
| `src/path.rs` | the toolpath, worked out in the plane |
| `src/gcode.rs` | the toolpath as a G-code program |
| `icons/` | its icon |
| `scripts/pack.sh` | build and pack a `.pcbench` |
| `.github/workflows/ci.yml` | checks on every push |
| `.github/workflows/release.yml` | a release for every `v*` tag |

## License

MIT or Apache-2.0, at your option.
