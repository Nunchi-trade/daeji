// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

/// @title HdcLib
/// @notice Library for calling the HDC precompile at address 0x09.
/// @dev The v1 precompile uses a 1-byte raw-opcode dispatch:
///      0x01 = hamming, 0x02 = bind, 0x03 = bundle,
///      0x04 = permute, 0x05 = vectorId, 0x06 = isSimilar.
///      Stateful store/search/delete opcodes are not part of v1.
///      These will be added when consensus-backed stateful precompile
///      support is implemented.
///      All vectors are 1,280 bytes (10,240 bits / 8).
library HdcLib {
    address internal constant HDC_PRECOMPILE = address(0x09);

    /// @notice Compute the Hamming distance between two raw vectors.
    /// @param a First 1,280-byte vector.
    /// @param b Second 1,280-byte vector.
    /// @return dist The number of differing bits (0..10240).
    function hamming(bytes memory a, bytes memory b) internal view returns (uint32 dist) {
        require(a.length == 1280 && b.length == 1280, "HdcLib: invalid vector size");
        bytes memory payload = abi.encodePacked(uint8(0x01), a, b);
        (bool ok, bytes memory ret) = HDC_PRECOMPILE.staticcall(payload);
        require(ok && ret.length == 32, "HdcLib: hamming failed");
        dist = abi.decode(ret, (uint32));
    }

    /// @notice XOR-bind two vectors. Result is quasi-orthogonal to both inputs.
    /// @param a First 1,280-byte vector.
    /// @param b Second 1,280-byte vector.
    /// @return result The XOR of a and b (1,280 bytes).
    function bind(bytes memory a, bytes memory b) internal view returns (bytes memory result) {
        require(a.length == 1280 && b.length == 1280, "HdcLib: invalid vector size");
        bytes memory payload = abi.encodePacked(uint8(0x02), a, b);
        (bool ok, bytes memory ret) = HDC_PRECOMPILE.staticcall(payload);
        require(ok && ret.length == 1280, "HdcLib: bind failed");
        result = ret;
    }

    /// @notice Majority-vote bundle of N vectors.
    /// @param vectors Array of 1,280-byte vectors (1 to 256 inclusive).
    /// @return result The bundled vector (1,280 bytes).
    function bundle(bytes[] memory vectors) internal view returns (bytes memory result) {
        require(vectors.length > 0, "HdcLib: empty bundle");
        require(vectors.length <= 256, "HdcLib: too many vectors (max 256)");
        bytes memory payload = abi.encodePacked(uint8(0x03), uint32(vectors.length));
        for (uint256 i = 0; i < vectors.length; i++) {
            require(vectors[i].length == 1280, "HdcLib: invalid vector size");
            payload = abi.encodePacked(payload, vectors[i]);
        }
        (bool ok, bytes memory ret) = HDC_PRECOMPILE.staticcall(payload);
        require(ok && ret.length == 1280, "HdcLib: bundle failed");
        result = ret;
    }

    /// @notice Cyclic left-rotation of a vector by n bit positions.
    /// @param v The 1,280-byte vector to permute.
    /// @param n Number of bit positions to rotate left.
    /// @return result The permuted vector (1,280 bytes).
    function permute(bytes memory v, uint32 n) internal view returns (bytes memory result) {
        require(v.length == 1280, "HdcLib: invalid vector size");
        bytes memory payload = abi.encodePacked(uint8(0x04), v, n);
        (bool ok, bytes memory ret) = HDC_PRECOMPILE.staticcall(payload);
        require(ok && ret.length == 1280, "HdcLib: permute failed");
        result = ret;
    }

    /// @notice Compute the keccak-256 content address of a vector.
    /// @param v The 1,280-byte vector.
    /// @return id The 32-byte content hash.
    function vectorId(bytes memory v) internal view returns (bytes32 id) {
        require(v.length == 1280, "HdcLib: invalid vector size");
        bytes memory payload = abi.encodePacked(uint8(0x05), v);
        (bool ok, bytes memory ret) = HDC_PRECOMPILE.staticcall(payload);
        require(ok && ret.length == 32, "HdcLib: vectorId failed");
        id = bytes32(ret);
    }

    /// @notice Check whether two vectors are similar (Hamming distance <= threshold).
    /// @param a First 1,280-byte vector.
    /// @param b Second 1,280-byte vector.
    /// @return similar True if the vectors are within the similarity threshold.
    function isSimilar(bytes memory a, bytes memory b) internal view returns (bool similar) {
        require(a.length == 1280 && b.length == 1280, "HdcLib: invalid vector size");
        bytes memory payload = abi.encodePacked(uint8(0x06), a, b);
        (bool ok, bytes memory ret) = HDC_PRECOMPILE.staticcall(payload);
        require(ok && ret.length == 32, "HdcLib: isSimilar failed");
        similar = abi.decode(ret, (bool));
    }
}
