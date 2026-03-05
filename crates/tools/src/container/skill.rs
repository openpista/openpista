//! Skill loading and runtime mode resolution.

use skills::{SkillExecutionMode, SkillLoader};

use super::{ContainerArgs, RuntimeMode, non_empty};

/// Determine whether the provided container arguments should run as a Docker container or as a WASM skill.
///
/// If `skill_name` is not provided or is empty, this returns `RuntimeMode::Docker`.
/// When `skill_name` is present, `workspace_dir` must also be provided; the function loads the skill's metadata
/// from the workspace and returns `RuntimeMode::Wasm` when the skill's execution mode is WASM, otherwise
/// it returns `RuntimeMode::Docker`.
///
/// # Errors
///
/// Returns `Err` if `skill_name` is present but `workspace_dir` is missing or empty, or if the skill metadata
/// (SKILL.md) cannot be found for the given `skill_name`.
///
/// # Examples
///
/// ```ignore
/// # use crate::ContainerArgs;
/// # use crate::RuntimeMode;
/// # tokio_test::block_on(async {
/// let args = ContainerArgs {
///     skill_name: None,
///     workspace_dir: None,
///     image: None,
///     command: None,
///     skill_image: None,
///     skill_args: None,
///     timeout_secs: None,
///     working_dir: None,
///     env: None,
///     allow_network: None,
///     workspace_dir: None,
///     memory_mb: None,
///     cpu_millis: None,
///     pull: None,
///     inject_task_token: None,
///     token_ttl_secs: None,
///     token_env_name: None,
///     allow_subprocess_fallback: None,
/// };
///
/// let mode = crate::resolve_runtime_mode(&args).await.unwrap();
/// assert_eq!(mode, RuntimeMode::Docker);
/// # });
/// ```
pub(super) async fn resolve_runtime_mode(args: &ContainerArgs) -> Result<RuntimeMode, String> {
    let Some(skill_name) = args.skill_name.as_deref().and_then(non_empty) else {
        return Ok(RuntimeMode::Docker);
    };

    let workspace = args
        .workspace_dir
        .as_deref()
        .and_then(non_empty)
        .ok_or_else(|| "workspace_dir is required when skill_name is provided".to_string())?;

    let loader = SkillLoader::new(workspace);
    let metadata = loader
        .load_skill_metadata(skill_name)
        .await
        .ok_or_else(|| format!("SKILL.md not found for skill '{skill_name}'"))?;

    if metadata.mode == SkillExecutionMode::Wasm {
        Ok(RuntimeMode::Wasm)
    } else {
        Ok(RuntimeMode::Docker)
    }
}
