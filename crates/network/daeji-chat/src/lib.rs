//! Symphony-class agent-coordination protocol primitives + service entry point.
//!
//! Library-only crate. Bolted into the `kora` chain binary via
//! [`service::run_chat`] when `--enable-chat` is set; not run as a separate
//! process. See `crates/network/daeji-chat/README.md` for the architecture
//! rationale (canonical-plan §13).

pub mod card;
pub mod chain;
pub mod lobby;
/// Wire format for symphony-room messages — Hello / Status / PartialResult / Vote / Final.
pub mod messages;
pub mod registry;
/// Deterministic room id + slot derivation + ChaCha20Poly1305 AEAD with room id bound as AAD.
pub mod room;
pub mod service;
pub mod supervisor;
