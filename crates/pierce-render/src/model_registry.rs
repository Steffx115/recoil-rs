//! Registry of loaded 3D models, keyed by unit type ID.
//!
//! The [`ModelRegistry`] stores parsed mesh data and provides a fallback
//! octahedron for any unit type that hasn't had a model loaded.

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::Result;

use crate::obj_loader;
use crate::unit_mesh::{generate_unit_mesh, UnitVertex};

use pierce_model::PieceTree;
use pierce_s3o;

/// A loaded model's vertex and index data, ready for GPU upload.
pub struct LoadedModel {
    pub vertices: Vec<UnitVertex>,
    pub indices: Vec<u16>,
}

/// Stores models keyed by `unit_type_id`, with a fallback default mesh.
pub struct ModelRegistry {
    models: BTreeMap<u32, LoadedModel>,
    trees: BTreeMap<u32, PieceTree>,
    default_mesh: LoadedModel,
}

impl ModelRegistry {
    /// Create a new registry with the default octahedron as the fallback mesh.
    pub fn new() -> Self {
        let (vertices, indices) = generate_unit_mesh();
        Self {
            models: BTreeMap::new(),
            trees: BTreeMap::new(),
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
        let (vertices, indices) = pierce_s3o::load_s3o(data)?;
        self.models
            .insert(unit_type_id, LoadedModel { vertices, indices });
        Ok(())
    }

    /// Load an s3o file from disk and register it under the given unit type ID.
    pub fn load_s3o_file(&mut self, unit_type_id: u32, path: &Path) -> Result<()> {
        let (vertices, indices) = pierce_s3o::load_s3o_file(path)?;
        self.models
            .insert(unit_type_id, LoadedModel { vertices, indices });
        Ok(())
    }

    /// Parse an s3o byte slice as a piece tree and register it.
    pub fn load_s3o_tree(&mut self, unit_type_id: u32, data: &[u8]) -> Result<()> {
        let tree = pierce_s3o::load_s3o_tree(data)?;
        self.trees.insert(unit_type_id, tree);
        Ok(())
    }

    /// Get the piece tree for a unit type (if loaded as a tree).
    pub fn get_tree(&self, unit_type_id: u32) -> Option<&PieceTree> {
        self.trees.get(&unit_type_id)
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
#[path = "tests/model_registry_tests.rs"]
mod tests;
