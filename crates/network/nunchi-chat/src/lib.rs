//! Symphony-class agent-coordination protocol primitives + service entry point.
//!
//! Can be embedded as a library via [`service::run_chat`] or run as the
//! standalone `nunchi-chat` binary for local/client deployments.

#![allow(missing_docs)]

pub mod card;
pub mod chain;
pub mod identity;
pub mod lobby;
pub mod messages;
pub mod registry;
pub mod room;
pub mod service;
pub mod supervisor;
pub mod transport;
