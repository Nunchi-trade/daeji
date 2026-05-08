//! HDC Precompile at address `0x09`.
//!
//! Opcodes:
//! - 0x01: hamming_distance(a, b) -> u32
//! - 0x02: bind(a, b) -> HdcVector
//! - 0x03: bundle(vectors) -> HdcVector
//! - 0x04: permute(v, n) -> HdcVector
//! - 0x05: vector_id(v) -> bytes32
//! - 0x06: is_similar(a, b) -> bool

use alloy_primitives::Address;

/// HDC precompile address. This is a sovereign chain fork choice:
/// daeji intentionally replaces EIP-152 BLAKE2F with HDC at address 0x09.
/// This chain does not need BLAKE2F compatibility.
pub const HDC_PRECOMPILE_ADDRESS: Address =
    Address::new([0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x09]);

/// Gas costs per opcode.
///
/// These values reflect the computational cost of 10,240-bit (160-word)
/// vector operations. Each word operation is roughly equivalent to a
/// 64-bit ALU instruction.
mod gas {
    /// Hamming distance: 160-word XOR + popcount.
    pub(super) const HAMMING_DISTANCE: u64 = 1_500;
    /// Bind: 160-word XOR.
    pub(super) const BIND: u64 = 500;
    /// Bundle base cost (setup + final threshold pass).
    pub(super) const BUNDLE_BASE: u64 = 500;
    /// Bundle per-vector cost (accumulate 160 words).
    pub(super) const BUNDLE_PER_VECTOR: u64 = 300;
    /// Permute: cyclic rotation of 160 words.
    pub(super) const PERMUTE: u64 = 500;
    /// Vector ID: keccak256 over 1,280 bytes.
    pub(super) const VECTOR_ID: u64 = 3_000;
    /// Is-similar: hamming distance + threshold compare.
    pub(super) const IS_SIMILAR: u64 = 1_500;
}

/// Maximum number of vectors in a single bundle operation.
const MAX_BUNDLE_VECTORS: usize = 256;

/// Opcode identifiers for the HDC precompile.
#[repr(u8)]
enum Opcode {
    /// Hamming distance between two vectors.
    HammingDistance = 0x01,
    /// XOR-bind two vectors.
    Bind = 0x02,
    /// Majority-vote bundle of N vectors.
    Bundle = 0x03,
    /// Cyclic rotation of a vector.
    Permute = 0x04,
    /// Keccak-256 content address of a vector.
    VectorId = 0x05,
    /// Integer threshold similarity check.
    IsSimilar = 0x06,
}

impl Opcode {
    fn from_byte(b: u8) -> Option<Self> {
        match b {
            0x01 => Some(Self::HammingDistance),
            0x02 => Some(Self::Bind),
            0x03 => Some(Self::Bundle),
            0x04 => Some(Self::Permute),
            0x05 => Some(Self::VectorId),
            0x06 => Some(Self::IsSimilar),
            _ => None,
        }
    }
}

use kora_hdc::{
    BYTES, HdcVector, THRESHOLD_HAMMING, bind, bundle, deserialize, hamming_distance, permute,
    serialize, vector_id,
};

/// Execute the HDC precompile. Returns `(gas_used, output_bytes)` or an error.
pub fn hdc_precompile(input: &[u8], gas_limit: u64) -> Result<(u64, Vec<u8>), PrecompileError> {
    if input.is_empty() {
        return Err(PrecompileError::InvalidInput("empty input".into()));
    }

    let opcode = Opcode::from_byte(input[0]).ok_or(PrecompileError::InvalidOpcode(input[0]))?;

    let data = &input[1..];

    match opcode {
        Opcode::HammingDistance => exec_hamming_distance(data, gas_limit),
        Opcode::Bind => exec_bind(data, gas_limit),
        Opcode::Bundle => exec_bundle(data, gas_limit),
        Opcode::Permute => exec_permute(data, gas_limit),
        Opcode::VectorId => exec_vector_id(data, gas_limit),
        Opcode::IsSimilar => exec_is_similar(data, gas_limit),
    }
}

fn read_vector(data: &[u8], offset: usize) -> Result<HdcVector, PrecompileError> {
    if data.len() < offset + BYTES {
        return Err(PrecompileError::InvalidInput(format!(
            "need {} bytes at offset {}, got {}",
            BYTES,
            offset,
            data.len()
        )));
    }
    let bytes: &[u8; BYTES] = data[offset..offset + BYTES]
        .try_into()
        .map_err(|_| PrecompileError::InvalidInput("slice conversion failed".into()))?;
    Ok(deserialize(bytes))
}

/// Expected input length: exactly 2 * BYTES (two vectors, opcode already stripped).
fn exec_hamming_distance(data: &[u8], gas_limit: u64) -> Result<(u64, Vec<u8>), PrecompileError> {
    let expected_len = 2 * BYTES;
    if data.len() != expected_len {
        return Err(PrecompileError::InvalidInput(format!(
            "hamming: expected {} bytes, got {}",
            expected_len,
            data.len()
        )));
    }
    if gas_limit < gas::HAMMING_DISTANCE {
        return Err(PrecompileError::OutOfGas);
    }
    let a = read_vector(data, 0)?;
    let b = read_vector(data, BYTES)?;
    let dist = hamming_distance(&a, &b);
    // ABI-encoded uint32: 32 bytes, big-endian, left-padded
    let mut out = vec![0u8; 32];
    out[28..32].copy_from_slice(&dist.to_be_bytes());
    Ok((gas::HAMMING_DISTANCE, out))
}

/// Expected input length: exactly 2 * BYTES (two vectors).
fn exec_bind(data: &[u8], gas_limit: u64) -> Result<(u64, Vec<u8>), PrecompileError> {
    let expected_len = 2 * BYTES;
    if data.len() != expected_len {
        return Err(PrecompileError::InvalidInput(format!(
            "bind: expected {} bytes, got {}",
            expected_len,
            data.len()
        )));
    }
    if gas_limit < gas::BIND {
        return Err(PrecompileError::OutOfGas);
    }
    let a = read_vector(data, 0)?;
    let b = read_vector(data, BYTES)?;
    let result = bind(&a, &b);
    Ok((gas::BIND, serialize(&result).to_vec()))
}

/// Expected input length: exactly 4 + count * BYTES.
fn exec_bundle(data: &[u8], gas_limit: u64) -> Result<(u64, Vec<u8>), PrecompileError> {
    if data.len() < 4 {
        return Err(PrecompileError::InvalidInput("bundle: need 4-byte count prefix".into()));
    }
    let count = u32::from_be_bytes([data[0], data[1], data[2], data[3]]) as usize;
    if count == 0 {
        return Err(PrecompileError::InvalidInput("bundle: count must be >= 1".into()));
    }
    if count > MAX_BUNDLE_VECTORS {
        return Err(PrecompileError::InvalidInput(format!(
            "bundle: count {} exceeds maximum {}",
            count, MAX_BUNDLE_VECTORS
        )));
    }
    let expected_len = 4 + count * BYTES;
    if data.len() != expected_len {
        return Err(PrecompileError::InvalidInput(format!(
            "bundle: expected {} bytes, got {}",
            expected_len,
            data.len()
        )));
    }
    let gas_cost = gas::BUNDLE_BASE + (count as u64) * gas::BUNDLE_PER_VECTOR;
    if gas_limit < gas_cost {
        return Err(PrecompileError::OutOfGas);
    }
    let vec_data = &data[4..];
    let mut vectors = Vec::with_capacity(count);
    for i in 0..count {
        vectors.push(read_vector(vec_data, i * BYTES)?);
    }
    let refs: Vec<&HdcVector> = vectors.iter().collect();
    let result = bundle(&refs);
    Ok((gas_cost, serialize(&result).to_vec()))
}

/// Expected input length: exactly BYTES + 4 (vector + u32 rotation).
fn exec_permute(data: &[u8], gas_limit: u64) -> Result<(u64, Vec<u8>), PrecompileError> {
    let expected_len = BYTES + 4;
    if data.len() != expected_len {
        return Err(PrecompileError::InvalidInput(format!(
            "permute: expected {} bytes, got {}",
            expected_len,
            data.len()
        )));
    }
    if gas_limit < gas::PERMUTE {
        return Err(PrecompileError::OutOfGas);
    }
    let v = read_vector(data, 0)?;
    let n_bytes: [u8; 4] = data[BYTES..BYTES + 4].try_into().map_err(|_| {
        PrecompileError::InvalidInput("permute: failed to read rotation amount".into())
    })?;
    let n = u32::from_be_bytes(n_bytes) as usize;
    let result = permute(&v, n);
    Ok((gas::PERMUTE, serialize(&result).to_vec()))
}

/// Expected input length: exactly BYTES (one vector).
fn exec_vector_id(data: &[u8], gas_limit: u64) -> Result<(u64, Vec<u8>), PrecompileError> {
    if data.len() != BYTES {
        return Err(PrecompileError::InvalidInput(format!(
            "vector_id: expected {} bytes, got {}",
            BYTES,
            data.len()
        )));
    }
    if gas_limit < gas::VECTOR_ID {
        return Err(PrecompileError::OutOfGas);
    }
    let v = read_vector(data, 0)?;
    let id = vector_id(&v);
    Ok((gas::VECTOR_ID, id.to_vec()))
}

/// Expected input length: exactly 2 * BYTES (two vectors).
fn exec_is_similar(data: &[u8], gas_limit: u64) -> Result<(u64, Vec<u8>), PrecompileError> {
    let expected_len = 2 * BYTES;
    if data.len() != expected_len {
        return Err(PrecompileError::InvalidInput(format!(
            "is_similar: expected {} bytes, got {}",
            expected_len,
            data.len()
        )));
    }
    if gas_limit < gas::IS_SIMILAR {
        return Err(PrecompileError::OutOfGas);
    }
    let a = read_vector(data, 0)?;
    let b = read_vector(data, BYTES)?;
    let dist = hamming_distance(&a, &b);
    let similar = dist <= THRESHOLD_HAMMING;
    // ABI-encoded bool: 32 bytes, 0 or 1, left-padded
    let mut out = vec![0u8; 32];
    out[31] = u8::from(similar);
    Ok((gas::IS_SIMILAR, out))
}

/// Errors from the HDC precompile.
#[derive(Debug, thiserror::Error)]
pub enum PrecompileError {
    /// Invalid opcode byte.
    #[error("invalid opcode: 0x{0:02x}")]
    InvalidOpcode(u8),
    /// Invalid input data.
    #[error("invalid input: {0}")]
    InvalidInput(String),
    /// Insufficient gas.
    #[error("out of gas")]
    OutOfGas,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_input(opcode: u8, data: &[u8]) -> Vec<u8> {
        let mut input = vec![opcode];
        input.extend_from_slice(data);
        input
    }

    #[test]
    fn test_hamming_distance_precompile() {
        let a = HdcVector::random(1);
        let b = HdcVector::random(2);
        let mut data = Vec::new();
        data.extend_from_slice(&serialize(&a));
        data.extend_from_slice(&serialize(&b));

        let input = make_input(0x01, &data);
        let (gas, output) = hdc_precompile(&input, 10_000).expect("hamming should succeed");
        assert_eq!(gas, gas::HAMMING_DISTANCE);

        let dist = u32::from_be_bytes(output[28..32].try_into().expect("slice len"));
        assert_eq!(dist, hamming_distance(&a, &b));
    }

    #[test]
    fn test_bind_precompile() {
        let a = HdcVector::random(10);
        let b = HdcVector::random(11);
        let mut data = Vec::new();
        data.extend_from_slice(&serialize(&a));
        data.extend_from_slice(&serialize(&b));

        let input = make_input(0x02, &data);
        let (gas, output) = hdc_precompile(&input, 10_000).expect("bind should succeed");
        assert_eq!(gas, gas::BIND);

        let result_bytes: &[u8; BYTES] = output.as_slice().try_into().expect("output len");
        let result = deserialize(result_bytes);
        assert_eq!(result, bind(&a, &b));
    }

    #[test]
    fn test_bundle_precompile() {
        let a = HdcVector::random(20);
        let b = HdcVector::random(21);
        let c = HdcVector::random(22);
        let count: u32 = 3;
        let mut data = count.to_be_bytes().to_vec();
        data.extend_from_slice(&serialize(&a));
        data.extend_from_slice(&serialize(&b));
        data.extend_from_slice(&serialize(&c));

        let input = make_input(0x03, &data);
        let (gas, output) = hdc_precompile(&input, 100_000).expect("bundle should succeed");
        assert_eq!(gas, gas::BUNDLE_BASE + 3 * gas::BUNDLE_PER_VECTOR);

        let result_bytes: &[u8; BYTES] = output.as_slice().try_into().expect("output len");
        let result = deserialize(result_bytes);
        let expected = bundle(&[&a, &b, &c]);
        assert_eq!(result, expected);
    }

    #[test]
    fn test_bundle_empty_rejected() {
        let count: u32 = 0;
        let data = count.to_be_bytes().to_vec();
        let input = make_input(0x03, &data);
        let result = hdc_precompile(&input, 100_000);
        assert!(matches!(result, Err(PrecompileError::InvalidInput(_))));
    }

    #[test]
    fn test_bundle_oversized_rejected() {
        let count: u32 = 257;
        let data = count.to_be_bytes().to_vec();
        let input = make_input(0x03, &data);
        let result = hdc_precompile(&input, 1_000_000);
        assert!(matches!(result, Err(PrecompileError::InvalidInput(_))));
    }

    #[test]
    fn test_permute_precompile() {
        let v = HdcVector::random(30);
        let n: u32 = 37;
        let mut data = Vec::new();
        data.extend_from_slice(&serialize(&v));
        data.extend_from_slice(&n.to_be_bytes());

        let input = make_input(0x04, &data);
        let (gas, output) = hdc_precompile(&input, 10_000).expect("permute should succeed");
        assert_eq!(gas, gas::PERMUTE);

        let result_bytes: &[u8; BYTES] = output.as_slice().try_into().expect("output len");
        let result = deserialize(result_bytes);
        assert_eq!(result, permute(&v, 37));
    }

    #[test]
    fn test_vector_id_precompile() {
        let v = HdcVector::random(40);
        let input = make_input(0x05, &serialize(&v));
        let (gas, output) = hdc_precompile(&input, 10_000).expect("vector_id should succeed");
        assert_eq!(gas, gas::VECTOR_ID);
        assert_eq!(output, vector_id(&v).to_vec());
    }

    #[test]
    fn test_is_similar_precompile() {
        let a = HdcVector::random(50);
        let b = a.clone(); // identical = similar
        let mut data = Vec::new();
        data.extend_from_slice(&serialize(&a));
        data.extend_from_slice(&serialize(&b));

        let input = make_input(0x06, &data);
        let (_, output) = hdc_precompile(&input, 10_000).expect("is_similar should succeed");
        assert_eq!(output[31], 1); // should be similar
    }

    #[test]
    fn test_invalid_opcode() {
        let result = hdc_precompile(&[0xFF], 10_000);
        assert!(result.is_err());
    }

    #[test]
    fn test_empty_input() {
        let result = hdc_precompile(&[], 10_000);
        assert!(result.is_err());
    }

    #[test]
    fn test_out_of_gas() {
        let a = HdcVector::random(60);
        let b = HdcVector::random(61);
        let mut data = Vec::new();
        data.extend_from_slice(&serialize(&a));
        data.extend_from_slice(&serialize(&b));
        let input = make_input(0x01, &data);
        let result = hdc_precompile(&input, 10); // too little gas
        assert!(matches!(result, Err(PrecompileError::OutOfGas)));
    }

    #[test]
    fn test_wrong_input_length_hamming() {
        let a = HdcVector::random(70);
        // Only one vector instead of two
        let input = make_input(0x01, &serialize(&a));
        let result = hdc_precompile(&input, 10_000);
        assert!(matches!(result, Err(PrecompileError::InvalidInput(_))));
    }

    #[test]
    fn test_wrong_input_length_bind() {
        // Extra trailing byte
        let a = HdcVector::random(71);
        let b = HdcVector::random(72);
        let mut data = Vec::new();
        data.extend_from_slice(&serialize(&a));
        data.extend_from_slice(&serialize(&b));
        data.push(0xFF);
        let input = make_input(0x02, &data);
        let result = hdc_precompile(&input, 10_000);
        assert!(matches!(result, Err(PrecompileError::InvalidInput(_))));
    }

    #[test]
    fn test_wrong_input_length_permute() {
        // Missing rotation bytes
        let v = HdcVector::random(73);
        let input = make_input(0x04, &serialize(&v));
        let result = hdc_precompile(&input, 10_000);
        assert!(matches!(result, Err(PrecompileError::InvalidInput(_))));
    }

    #[test]
    fn test_bundle_wrong_payload_length() {
        let count: u32 = 2;
        let mut data = count.to_be_bytes().to_vec();
        // Only provide 1 vector instead of 2
        data.extend_from_slice(&serialize(&HdcVector::random(74)));
        let input = make_input(0x03, &data);
        let result = hdc_precompile(&input, 100_000);
        assert!(matches!(result, Err(PrecompileError::InvalidInput(_))));
    }
}
