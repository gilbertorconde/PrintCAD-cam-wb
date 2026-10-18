//! Solids in ogeom's native format, as printCAD keeps a body's shape.
//!
//! printCAD hands a package each body's B-rep as ogeom's native text, already in
//! millimetres and where the document places it. It is tessellated here the way a
//! STEP file's solids are ([`crate::step`]): every face meshed to within the
//! tolerance and measured against its surface, the faces welded into one
//! [`Body`] whose faces are the shape's own, in ogeom's order of faces. There is no
//! file to check the mesh against: the shape is the kernel's own, not something
//! read from a file another kernel wrote.

use camcraft_geom::topology::Body;

use crate::step::{WeldError, Welded, mesh_bodies, weld_faces};

/// A native shape that could not be turned into a trustworthy mesh.
#[derive(Debug, thiserror::Error)]
pub enum NativeError {
    /// The text is not ogeom's native format, or not a version this reads.
    #[error("the shape does not read: {0}")]
    Read(String),
    /// A face would not mesh, or lies further off its surface than asked.
    #[error("the shape would not mesh: {0}")]
    Mesh(String),
    /// The triangles could not be made one body.
    #[error("the shape's triangles are not a usable body: {0}")]
    Body(String),
    /// Nothing was left to machine.
    #[error("the shape produced no triangles")]
    Empty,
    /// The requested tessellation tolerance was not usable.
    #[error("tessellation tolerance must be finite and greater than zero, got {0}")]
    BadTolerance(f64),
}

/// What loading found, beyond the body itself.
#[derive(Debug, Clone)]
pub struct NativeReport {
    /// Faces the shape has.
    pub faces: u64,
    /// Triangles the kernel produced.
    pub triangles: u64,
    /// Triangles dropped because corners coincided.
    pub degenerate: u64,
    /// Vertices after welding.
    pub vertices: u64,
}

/// Read a shape in ogeom's native format and tessellate it to within `tolerance`
/// millimetres, keeping the face each triangle came from.
pub fn load_native(text: &str, tolerance: f64) -> Result<(Body, NativeReport), NativeError> {
    if !(tolerance.is_finite() && tolerance > 0.0) {
        return Err(NativeError::BadTolerance(tolerance));
    }
    let (model, roots) =
        ogeom_io::native::read(text).map_err(|e| NativeError::Read(e.to_string()))?;
    let soup =
        mesh_bodies(&model, &roots, tolerance).map_err(|e| NativeError::Mesh(e.to_string()))?;
    let faces = soup.face_triangles.len() as u64;
    let Welded {
        body,
        triangles,
        degenerate,
        vertices,
    } = weld_faces(&soup).map_err(|e| match e {
        WeldError::Faces(detail) => NativeError::Body(detail),
        WeldError::Empty => NativeError::Empty,
        WeldError::Mesh(source) => NativeError::Body(source.to_string()),
    })?;
    Ok((
        body,
        NativeReport {
            faces,
            triangles,
            degenerate,
            vertices,
        },
    ))
}
