use crate::error::Result;
use crate::watchdog::updater::{UpdatePlan, run_one_shot};
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct UpdateArgs {
    pub config: String,
    pub dev: bool,
    pub dry_run: bool,
    pub rollback: bool,
    pub project_dir: Option<PathBuf>,
}

pub async fn run(args: UpdateArgs) -> Result<()> {
    run_one_shot(UpdatePlan {
        config_path: args.config,
        dev: args.dev,
        dry_run: args.dry_run,
        rollback: args.rollback,
        project_dir: args.project_dir,
    })
    .await
}
