//! Registry of loaded 3D models, keyed by unit type ID.
//!
//! The [`ModelRegistry`] stores parsed mesh data and provides a fallback
//! octahedron for any unit type that hasn't had a model loaded.

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::Result;

use crate::obj_loader;
use crate::s3o_loader;
use crate::unit_mesh::{generate_unit_mesh, UnitVertex};

/// A loaded model's vertex and index data, ready for GPU upload.
pub struct LoadedModel {
    pub vertices: Vec<UnitVertex>,
    pub indices: Vec<u16>,
}

/// Stores models keyed by `unit_type_id`, with a fallback default mesh.
pub struct ModelRegistry {
    models: BTreeMap<u32, LoadedModel>,
    default_mesh: LoadedModel,
}

impl ModelRegistry {
    /// Create a new registry with the default octahedron as the fallback mesh.
    pub fn new() -> Self {
        let (vertices, indices) = generate_unit_mesh();
        Self {
            models: BTreeMap::new(),
            default_mesh: LoadedModel { vertices, indices },
        }
    }

    /// Parse an OBJ string and register it under the given unit type ID.
    pub fn load_model(&mut self, unit_type_id: u32, obj_data: &str) -> Result<()> {
        let (vertices, indices) = obj_loader::load_obj(obj_data)?;
        self.models
            .insert(unit_type_id, LoadedModel { vertices, indices });
        Ok(())
    }

    /// Load an OBJ file from disk and register it under the given unit type ID.
    pub fn load_model_file(&mut self, unit_type_id: u32, path: &Path) -> Result<()> {
        let (vertices, indices) = obj_loader::load_obj_file(path)?;
        self.models
            .insert(unit_type_id, LoadedModel { vertices, indices });
        Ok(())
    }

    /// Parse an s3o byte slice and register it under the given unit type ID.
    pub fn load_s3o_model(&mut self, unit_type_id: u32, data: &[u8]) -> Result<()> {
        let (vertices, indices) = s3o_loader::load_s3o(data)?;
        self.models
            .insert(unit_type_id, LoadedModel { vertices, indices });
        Ok(())
    }

    /// Load an s3o file from disk and register it under the given unit type ID.
    pub fn load_s3o_file(&mut self, unit_type_id: u32, path: &Path) -> Result<()> {
        let (vertices, indices) = s3o_loader::load_s3o_file(path)?;
        self.models
            .insert(unit_type_id, LoadedModel { vertices, indices });
        Ok(())
    }

    /// Get the model for a unit type. Returns the default mesh if no model has
    /// been loaded for the given ID.
    pub fn get(&self, unit_type_id: u32) -> &LoadedModel {
        self.models.get(&unit_type_id).unwrap_or(&self.default_mesh)
    }
}

impl Default for ModelRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_mesh_is_octahedron() {
        let registry = ModelRegistry::new();
        let model = registry.get(999);
        // Octahedron: 8 faces * 3 verts = 24
        assert_eq!(model.vertices.len(), 24);
        assert_eq!(model.indices.len(), 24);
    }

    #[test]
    fn unknown_type_returns_default() {
        let registry = ModelRegistry::new();
        let a = registry.get(0);
        let b = registry.get(42);
        // Both should be the default octahedron
        assert_eq!(a.vertices.len(), b.vertices.len());
        assert_eq!(a.indices.len(), b.indices.len());
    }

    #[test]
    fn loaded_model_returned_for_known_type() {
        let mut registry = ModelRegistry::new();
        let obj = "\
v 0.0 0.0 0.0
v 1.0 0.0 0.0
v 0.0 1.0 0.0
vn 0.0 0.0 1.0
f 1//1 2//1 3//1
";
        registry.load_model(7, obj).unwrap();
        let model = registry.get(7);
        assert_eq!(model.vertices.len(), 3);
        assert_eq!(model.indices.len(), 3);
    }

    #[test]
    fn load_replaces_existing() {
        let mut registry = ModelRegistry::new();
        let tri = "\
v 0.0 0.0 0.0
v 1.0 0.0 0.0
v 0.0 1.0 0.0
vn 0.0 0.0 1.0
f 1//1 2//1 3//1
";
        registry.load_model(1, tri).unwrap();
        assert_eq!(registry.get(1).vertices.len(), 3);

        // Load a different model for the same ID
        let quad = "\
v 0.0 0.0 0.0
v 1.0 0.0 0.0
v 1.0 1.0 0.0
v 0.0 1.0 0.0
vn 0.0 0.0 1.0
f 1//1 2//1 3//1 4//1
";
        registry.load_model(1, quad).unwrap();
        assert_eq!(registry.get(1).vertices.len(), 4);
    }
}
