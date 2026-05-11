//! Symphony-class agent-coordination protocol primitives + service entry point.
//!
//! Library-only crate. Bolted into the `kora` chain binary via
//! [`service::run_chat`] when `--enable-chat` is set; not run as a separate
//! process. See `crates/network/daeji-chat/README.md` for the architecture
//! rationale (canonical-plan §13).

#![allow(missing_docs)]

pub mod card;
pub mod chain;
pub mod lobby;
pub mod messages;
pub mod registry;
pub mod room;
pub mod service;
pub mod supervisor;
pub mod transport;
