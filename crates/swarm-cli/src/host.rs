use anyhow::Result;
use swarm_core::DataPaths;
use swarm_storage::Storage;

pub use swarm_cli::migration::HostOptions;

pub async fn run(paths: &DataPaths, storage: &Storage, options: HostOptions) -> Result<()> {
    swarm_cli::migration::run_authority_runtime(paths, storage, options).await
}
