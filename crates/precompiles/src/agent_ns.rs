//! Agent-namespace precompile page at `0xA10–0xA1F`.
//!
//! Reserved for ERC-8004 lookups against on-chain `AgentRegistry`,
//! `WorkerRegistry`, and `RoleRegistry` contracts in `~/contracts-core/`.
//! Concrete handlers land in a follow-up PR once the canonical contract
//! addresses are pinned in `kora_config`.
//!
//! Reserved selectors (per D-PR1 plan):
//!
//! - `0xA10` passport_lookup(address agent) -> (bool, uint64, uint8, bytes32)
//! - `0xA11` capability_check(address agent, uint8 capability_bit) -> bool
//! - `0xA12` tier_check(address agent, uint8 min_tier) -> bool
//! - `0xA13` reputation_min(address agent, bytes32 domain, uint16 min_score_bps) -> bool

use alloy_primitives::{Address, address};

/// Reserved precompile addresses for the agent namespace `0xA10–0xA1F`.
pub const AGENT_NS_RESERVED: [Address; 16] = [
    address!("0x0000000000000000000000000000000000000A10"),
    address!("0x0000000000000000000000000000000000000A11"),
    address!("0x0000000000000000000000000000000000000A12"),
    address!("0x0000000000000000000000000000000000000A13"),
    address!("0x0000000000000000000000000000000000000A14"),
    address!("0x0000000000000000000000000000000000000A15"),
    address!("0x0000000000000000000000000000000000000A16"),
    address!("0x0000000000000000000000000000000000000A17"),
    address!("0x0000000000000000000000000000000000000A18"),
    address!("0x0000000000000000000000000000000000000A19"),
    address!("0x0000000000000000000000000000000000000A1A"),
    address!("0x0000000000000000000000000000000000000A1B"),
    address!("0x0000000000000000000000000000000000000A1C"),
    address!("0x0000000000000000000000000000000000000A1D"),
    address!("0x0000000000000000000000000000000000000A1E"),
    address!("0x0000000000000000000000000000000000000A1F"),
];
