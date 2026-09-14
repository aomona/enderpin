//! Minecraft package management, independent of a terminal or launcher GUI.
pub mod model;
pub mod registry;
pub mod resolver;
pub mod storage;
pub mod workspace;

pub use model::{Kind, Manifest, Package, Side};
pub use workspace::{SyncOptions, SyncReport, Workspace};
