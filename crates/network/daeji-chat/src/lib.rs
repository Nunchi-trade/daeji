//! Phase-1/2 POC for the Daeji × commonware-chat agent-coordination spec.
//!
//! Library crate so the agent binary (`daeji-chat`) and indexer binary
//! (`daeji-indexer`) can share the registry / room / messages modules.

pub mod card;
pub mod messages;
pub mod registry;
pub mod room;
