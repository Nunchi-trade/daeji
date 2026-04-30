use chacha20poly1305::{
    aead::{Aead, KeyInit, Payload},
    ChaCha20Poly1305, Key, Nonce,
};
use rand::RngCore;
use sha3::{Digest, Keccak256};

/// Domain-separation tag matching `keccak256("DAEJI_ROOM_V1" || job_id)` in the spec (D2).
const ROOM_DOMAIN: &[u8] = b"DAEJI_ROOM_V1";

/// Domain-separation tag for the well-known lobby channel id (per canonical-plan §14).
const LOBBY_DOMAIN: &[u8] = b"DAEJI_LOBBY_V1";

/// Domain-separation tag for the slot pool base (per canonical-plan §14).
const JOB_SLOT_DOMAIN: &[u8] = b"DAEJI_JOB_SLOT_V1";

/// Number of pre-registered slot channels per agent (canonical-plan §14).
/// Locked at 64 — birthday-paradox math: ~50% collision probability at ~8 concurrent
/// jobs per agent. AEAD with the room key disambiguates colliding jobs at the message
/// layer; bandwidth is shared per slot.
pub const POOL_SIZE: u32 = 64;

/// Well-known lobby channel id, derived once from `LOBBY_DOMAIN`.
/// All agents register this channel at startup and listen for `LobbyMessage` traffic.
pub fn lobby_channel_id() -> u64 {
    let mut hasher = Keccak256::new();
    hasher.update(LOBBY_DOMAIN);
    let h: [u8; 32] = hasher.finalize().into();
    let mut buf = [0u8; 8];
    buf.copy_from_slice(&h[..8]);
    u64::from_le_bytes(buf)
}

/// Base channel id for the slot pool. Slot `i` lives at `slot_pool_base() + i as u64`.
/// Derived once from `JOB_SLOT_DOMAIN` so all agents agree without coordination.
pub fn slot_pool_base() -> u64 {
    let mut hasher = Keccak256::new();
    hasher.update(JOB_SLOT_DOMAIN);
    let h: [u8; 32] = hasher.finalize().into();
    let mut buf = [0u8; 8];
    buf.copy_from_slice(&h[..8]);
    u64::from_le_bytes(buf)
}

/// Deterministically map a chain job id to a slot index in `0..POOL_SIZE`.
/// Same value computed by every agent — no coordination needed.
pub fn slot_for_chain_job(job_id: u64) -> u32 {
    let r = room_id_for_chain_job(job_id);
    let mut buf = [0u8; 4];
    buf.copy_from_slice(&r[..4]);
    u32::from_le_bytes(buf) % POOL_SIZE
}

/// Channel id for a given slot index. Combine with `slot_pool_base()`.
pub fn channel_id_for_slot(slot: u32) -> u64 {
    slot_pool_base().wrapping_add(slot as u64)
}

/// 32-byte room id, deterministic from a free-form job id (Phase 1/2 demo CLI).
pub fn room_id(job_id: &[u8]) -> [u8; 32] {
    let mut hasher = Keccak256::new();
    hasher.update(ROOM_DOMAIN);
    hasher.update(job_id);
    hasher.finalize().into()
}

/// 32-byte room id matching the on-chain `MultiAgentMarket.computeRoomId(uint256)` derivation.
/// Encoded as `keccak256(abi.encodePacked(bytes("DAEJI_ROOM_V1"), uint256(jobId)))`.
pub fn room_id_for_chain_job(job_id: u64) -> [u8; 32] {
    let mut hasher = Keccak256::new();
    hasher.update(ROOM_DOMAIN);
    let mut id_be = [0u8; 32];
    id_be[24..].copy_from_slice(&job_id.to_be_bytes());
    hasher.update(id_be);
    hasher.finalize().into()
}

/// Derive the commonware channel id from a room id (first 8 bytes, little-endian).
pub fn channel_id_from_room(room: &[u8; 32]) -> u64 {
    let mut buf = [0u8; 8];
    buf.copy_from_slice(&room[..8]);
    u64::from_le_bytes(buf)
}

/// AEAD-encrypt a message with the room key. Layout: nonce(12) || ciphertext.
/// The room id is bound as additional authenticated data.
pub fn encrypt(room_key: &[u8; 32], room: &[u8; 32], plaintext: &[u8]) -> Vec<u8> {
    let cipher = ChaCha20Poly1305::new(Key::from_slice(room_key));
    let mut nonce_bytes = [0u8; 12];
    rand::thread_rng().fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);

    let ct = cipher
        .encrypt(
            nonce,
            Payload {
                msg: plaintext,
                aad: room,
            },
        )
        .expect("AEAD seal");

    let mut out = Vec::with_capacity(12 + ct.len());
    out.extend_from_slice(&nonce_bytes);
    out.extend_from_slice(&ct);
    out
}

/// Inverse of `encrypt`. Returns None on tag mismatch (wrong key, tampering, or
/// a peer using a different room key for the same channel).
pub fn decrypt(room_key: &[u8; 32], room: &[u8; 32], wire: &[u8]) -> Option<Vec<u8>> {
    if wire.len() < 12 + 16 {
        return None;
    }
    let cipher = ChaCha20Poly1305::new(Key::from_slice(room_key));
    let nonce = Nonce::from_slice(&wire[..12]);
    cipher
        .decrypt(
            nonce,
            Payload {
                msg: &wire[12..],
                aad: room,
            },
        )
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let key = [7u8; 32];
        let room = room_id(b"job-42");
        let ct = encrypt(&key, &room, b"hello world");
        let pt = decrypt(&key, &room, &ct).unwrap();
        assert_eq!(pt, b"hello world");
    }

    #[test]
    fn tag_rejects_wrong_room() {
        let key = [7u8; 32];
        let room_a = room_id(b"job-42");
        let room_b = room_id(b"job-99");
        let ct = encrypt(&key, &room_a, b"hello");
        assert!(decrypt(&key, &room_b, &ct).is_none());
    }

    #[test]
    fn tag_rejects_wrong_key() {
        let room = room_id(b"job-42");
        let ct = encrypt(&[7u8; 32], &room, b"hello");
        assert!(decrypt(&[8u8; 32], &room, &ct).is_none());
    }

    #[test]
    fn channel_id_is_deterministic() {
        let r1 = room_id(b"job-42");
        let r2 = room_id(b"job-42");
        assert_eq!(channel_id_from_room(&r1), channel_id_from_room(&r2));
        let r3 = room_id(b"job-99");
        assert_ne!(channel_id_from_room(&r1), channel_id_from_room(&r3));
    }

    /// Cross-language parity: `room_id_for_chain_job(7)` must match
    /// `MultiAgentMarket.computeRoomId(7)` from the Solidity side.
    /// Expected value from `cast keccak 0x{ROOM_DOMAIN}{uint256(7)_be}`.
    #[test]
    fn room_id_for_chain_job_matches_solidity() {
        let r = room_id_for_chain_job(7);
        let expected_hex = "349edc281e8962dc4cdcb608f603ff0947711607f3d3412cea0b7e07e2899a90";
        assert_eq!(hex::encode(r), expected_hex);
    }

    #[test]
    fn lobby_and_slot_pool_constants_are_deterministic() {
        // All agents must compute the same lobby/slot ids without coordination.
        let l1 = lobby_channel_id();
        let l2 = lobby_channel_id();
        assert_eq!(l1, l2);
        let s1 = slot_pool_base();
        let s2 = slot_pool_base();
        assert_eq!(s1, s2);
        // Lobby and slot-pool bases must differ — otherwise lobby traffic would
        // collide with slot-0 traffic at the channel layer.
        assert_ne!(l1, s1);
    }

    #[test]
    fn slot_for_chain_job_in_pool_range() {
        for job_id in [0u64, 1, 7, 42, 1_000, u64::MAX / 2, u64::MAX] {
            let s = slot_for_chain_job(job_id);
            assert!(s < POOL_SIZE, "slot {} out of pool range for job_id {}", s, job_id);
        }
    }

    #[test]
    fn slot_for_chain_job_is_deterministic() {
        // Same job_id → same slot, every time.
        assert_eq!(slot_for_chain_job(42), slot_for_chain_job(42));
        assert_eq!(slot_for_chain_job(0), slot_for_chain_job(0));
    }

    #[test]
    fn slot_distribution_is_roughly_uniform() {
        // Sanity check for the hash mod 64 mapping. Across 6400 sequential job ids,
        // each slot should see roughly 100 ± noise. Tight bound at ±50% of mean.
        let mut counts = [0u32; POOL_SIZE as usize];
        for job_id in 0u64..6400 {
            let s = slot_for_chain_job(job_id) as usize;
            counts[s] += 1;
        }
        let mean = 6400 / POOL_SIZE;
        for (i, &c) in counts.iter().enumerate() {
            assert!(
                c > mean / 2 && c < mean * 3 / 2,
                "slot {} got {} jobs (mean {}); distribution not uniform",
                i,
                c,
                mean,
            );
        }
    }

    #[test]
    fn channel_id_for_slot_is_offset_from_base() {
        let base = slot_pool_base();
        for slot in [0u32, 1, 17, POOL_SIZE - 1] {
            assert_eq!(channel_id_for_slot(slot), base.wrapping_add(slot as u64));
        }
    }
}
