use anyhow::Result;
use crate::models::RawEvent;

/// Every data source implements this trait.
/// Official connectors: git, beads, timeforged, beacon.
/// Community connectors can be separate crates.
pub trait Connector: Send + Sync {
    fn name(&self) -> &str;
    fn pull(&self) -> Result<Vec<RawEvent>>;
}
