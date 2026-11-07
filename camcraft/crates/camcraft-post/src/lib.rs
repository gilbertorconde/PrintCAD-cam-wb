//! Post processors for camcraft.
//!
//! Position in the dependency DAG: depends on `camcraft-core` alone.
//!
//! The engine never emits text. This crate converts a `Toolpath` into a
//! machine-independent instruction stream, and a **profile** renders that stream
//! into a dialect. A profile is a TOML file, not Rust: adding a machine must never
//! mean recompiling.
//!
//! The instruction stream carries structure, not just moves — section boundaries
//! naming the operation, tool, strategy and work offset — because posts need that
//! to write headers, tool-change blocks and setup sheets.
//!
//! What the text does is checked by reading it back as a controller would
//! ([`readback`]) and replaying it against the path it was posted from.
//!
//! Most profiles write G-code; one whose `dialect` is `heidenhain` writes
//! Heidenhain's Klartext instead ([`heidenhain`]), and is read back into the
//! same steps, so the same checks run on it.

pub mod custom;
pub mod heidenhain;
pub mod op;
pub mod placeholder;
pub mod profile;
pub mod readback;
pub mod render;
pub mod rotary;
