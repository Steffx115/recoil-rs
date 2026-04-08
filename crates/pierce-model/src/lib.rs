//! Common model types for the Pierce RTS engine.
//!
//! This crate provides format-independent types for 3D model data:
//! vertices, piece trees, and transforms. It has no wgpu dependency.

pub mod piece_tree;
pub mod vertex;

pub use piece_tree::{flatten_with_transforms, ModelLoader, PieceNode, PieceTransform, PieceTree};
pub use vertex::ModelVertex;
