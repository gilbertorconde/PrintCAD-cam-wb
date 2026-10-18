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

Only `camcraft-import`:

- The `opencascade` feature is gone (its code, `build.rs` and
  `tests/opencascade.rs`), and with it every trace of OpenCascade and the LGPL.
- `native` reads a solid in ogeom's native format, as printCAD keeps a body's
  shape, and tessellates it as a STEP file's solids are: `step::mesh_bodies`
  and `step::weld_faces` are what `step` did inline, split out so both use
  them.

And one test: ogeom is 0.11.1 here, which printCAD's shapes need, and it
meshes `round_edge_loose_fit.step`'s seam that 0.11.0 drew off the torus, so
`a_loose_fit_is_refused_for_ogeoms_seam` became
`a_loose_fit_reads_within_the_tolerance`, as its comment asked.

To take a newer camcraft, copy those crates over these, then make the same
changes again; `diff -r` against the commit above shows them.
