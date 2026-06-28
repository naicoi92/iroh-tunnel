//! Access role: consume remote services to local.
// TODO: implement in T-07 (access run) / T-11 (config_cmd) / T-12 (service).

use std::path::Path;

use anyhow::Result;

/// Placeholder until T-07 implements the real access loop.
pub async fn run(_config_path: &Path) -> Result<()> {
    anyhow::bail!("access run is not implemented yet (T-07)")
}
