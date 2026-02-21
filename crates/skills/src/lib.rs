//! Skill discovery/loading helpers.

pub mod loader;

/// Skill loader for workspace-based skills.
pub use loader::{SkillExecutionMode, SkillLoader, SkillMetadata};
