//! Non-interactive `run` subcommand.

#[cfg(not(test))]
use std::sync::Arc;

#[cfg(not(test))]
use agent::AutoApproveHandler;
#[cfg(not(test))]
use proto::{ChannelId, SessionId};
#[cfg(not(test))]
use skills::SkillLoader;

#[cfg(not(test))]
use crate::config::Config;
#[cfg(not(test))]
use crate::startup::build_runtime;

#[cfg(not(test))]
/// Executes one command against the agent and exits.
pub(crate) async fn cmd_run(config: Config, exec: String) -> anyhow::Result<()> {
    let runtime = build_runtime(&config, Arc::new(AutoApproveHandler)).await?;
    let skill_loader = SkillLoader::new(&config.skills.workspace);
    let skills_ctx = skill_loader.load_context().await;

    let channel_id = ChannelId::new("cli", "run");
    let session_id = SessionId::new();

    println!("{}", crate::format_run_header(&exec));

    let result = runtime
        .process(&channel_id, &session_id, &exec, Some(&skills_ctx))
        .await;

    match result {
        Ok((text, _usage)) => {
            println!("{text}");
        }
        Err(e) => {
            eprintln!("Error: {e}");
            std::process::exit(1);
        }
    }

    Ok(())
}
