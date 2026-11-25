//! 3MF import: the 3D Manufacturing Format, the file PrusaSlicer, Cura, Bambu
//! Studio and Windows 3D Builder save, now one more format the importer reads.
//!
//! The whole plugin is one registration. The importer converts every format to
//! a GLB and runs one pipeline on that (preview, `.material` extraction, the
//! stored `.rvg`), so a format only has to produce the GLB; see
//! `renzora::register_import_format`.
//!
//! | module | what it is |
//! |---|---|
//! | [`model`] | the package and its XML: objects, components, build, materials |
//! | [`convert`] | the build flattened into meshes, normals made, GLB written |
//! | [`glb`] | the binary glTF writer |

use bevy::prelude::*;

mod convert;
mod glb;
mod model;

pub struct Format3mfPlugin;

impl Plugin for Format3mfPlugin {
    fn build(&self, _app: &mut App) {
        renzora::register_import_format(renzora::ImportFormat {
            name: "3MF (3D Manufacturing Format)",
            extensions: &["3mf"],
            convert: convert::convert,
        });
    }
}

// Editor scope: importing is the editor's. A game loads the `.rvg` the import
// stored and never sees a 3MF.
renzora::plugin!(Format3mfPlugin);
