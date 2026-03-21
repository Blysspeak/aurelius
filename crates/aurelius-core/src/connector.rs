use crate::models::RawEvent;
use anyhow::Result;
use async_trait::async_trait;

/// Every data source implements this trait.
/// Official connectors: git, beads, timeforged, beacon.
/// Community connectors can be separate crates.
#[async_trait]
pub trait Connector: Send + Sync {
    fn name(&self) -> &str;
    async fn pull(&self) -> Result<Vec<RawEvent>>;
}
