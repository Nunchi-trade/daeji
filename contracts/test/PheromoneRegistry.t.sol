// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

import {Test, console} from "forge-std/Test.sol";
import {PheromoneRegistry} from "../src/PheromoneRegistry.sol";
import {IPheromoneRegistry} from "../src/IPheromoneRegistry.sol";

contract PheromoneRegistryTest is Test {
    PheromoneRegistry registry;
    address alice = makeAddr("alice");
    address bob = makeAddr("bob");
    address charlie = makeAddr("charlie");
    address dave = makeAddr("dave");

    function setUp() public {
        registry = new PheromoneRegistry();

        vm.deal(alice, 100 ether);
        vm.deal(bob, 100 ether);
        vm.deal(charlie, 100 ether);
        vm.deal(dave, 100 ether);
    }

    function _makeLocation(uint8 seed) internal pure returns (bytes memory) {
        bytes memory v = new bytes(1280);
        for (uint256 i = 0; i < 1280; i++) {
            v[i] = bytes1(seed);
        }
        return v;
    }

    function _deposit(
        address sender,
        uint8 seed,
        IPheromoneRegistry.PheromoneType pType,
        uint64 intensity
    ) internal returns (bytes32) {
        return _depositWithValue(sender, seed, pType, intensity, 0.001 ether);
    }

    function _depositWithValue(
        address sender,
        uint8 seed,
        IPheromoneRegistry.PheromoneType pType,
        uint64 intensity,
        uint256 value
    ) internal returns (bytes32) {
        bytes memory location = _makeLocation(seed);
        bytes32 locationHash = keccak256(location);
        bytes32 pheromoneId = keccak256(
            abi.encodePacked(locationHash, sender, block.number, uint8(pType))
        );

        vm.prank(sender);
        registry.deposit{value: value}(location, pType, intensity);

        return pheromoneId;
    }

    // ----------------------------------------------------------------
    // test_deposit_creates_pheromone
    // ----------------------------------------------------------------

    function test_deposit_creates_pheromone() public {
        bytes32 id = _deposit(alice, 0x01, IPheromoneRegistry.PheromoneType.THREAT, 1000);

        (
            bytes32 locationHash,
            address depositor,
            uint64 intensity,
            uint64 depositBlock,
            uint16 confirmationCount,
            uint8 pType
        ) = registry.pheromones(id);

        assertEq(depositor, alice);
        assertEq(intensity, 1000);
        assertEq(depositBlock, block.number);
        assertEq(confirmationCount, 0);
        assertEq(pType, uint8(IPheromoneRegistry.PheromoneType.THREAT));
        assertTrue(locationHash != bytes32(0));

        // Deposit tracked
        assertEq(registry.deposits(id), 0.001 ether);
    }

    // ----------------------------------------------------------------
    // test_deposit_below_min_reverts
    // ----------------------------------------------------------------

    function test_deposit_below_min_reverts() public {
        bytes memory location = _makeLocation(0x01);
        vm.prank(alice);
        vm.expectRevert("PheromoneRegistry: insufficient deposit");
        registry.deposit{value: 0.0001 ether}(
            location,
            IPheromoneRegistry.PheromoneType.THREAT,
            1000
        );
    }

    // ----------------------------------------------------------------
    // test_alpha_paradox_decay
    // ----------------------------------------------------------------

    function test_alpha_paradox_decay() public {
        // Deposit THREAT pheromone (base HL = 100 blocks)
        bytes32 id = _deposit(alice, 0x01, IPheromoneRegistry.PheromoneType.THREAT, 10000);

        // Intensity at deposit = 10000
        assertEq(registry.currentIntensity(id), 10000);

        // No confirmations: after 100 blocks (1 half-life), intensity = 5000
        vm.roll(block.number + 100);
        assertEq(registry.currentIntensity(id), 5000);

        // Reset: new deposit
        vm.roll(1000);
        bytes32 id2 = _deposit(bob, 0x02, IPheromoneRegistry.PheromoneType.THREAT, 10000);

        // 1 confirmation: effective HL = 100/(1+1) = 50
        vm.prank(charlie);
        registry.confirm(id2);

        uint64 afterConfirm = registry.currentIntensity(id2);

        // After 50 blocks (1 effective half-life), should be ~half
        vm.roll(block.number + 50);
        uint64 afterDecay = registry.currentIntensity(id2);

        // afterDecay should be approximately afterConfirm / 2
        assertTrue(afterDecay <= afterConfirm / 2 + 1, "Alpha paradox: more confirms = faster decay");
        assertTrue(afterDecay >= afterConfirm / 2 - afterConfirm / 10, "Decay should be close to half");

        // 2 confirmations: effective HL = 100/(1+2) = 33
        vm.roll(2000);
        bytes32 id3 = _deposit(dave, 0x03, IPheromoneRegistry.PheromoneType.THREAT, 10000);
        vm.prank(alice);
        registry.confirm(id3);
        vm.prank(bob);
        registry.confirm(id3);

        // HL is now 33 blocks. After 33 blocks, intensity should halve
        uint64 baseline = registry.currentIntensity(id3);
        vm.roll(block.number + 33);
        uint64 decayed = registry.currentIntensity(id3);
        assertTrue(decayed <= baseline / 2 + 1, "2 confirms: even faster decay");
    }

    // ----------------------------------------------------------------
    // test_intensity_decay_over_blocks
    // ----------------------------------------------------------------

    function test_intensity_decay_over_blocks() public {
        // Set a known starting block
        vm.roll(1000);
        bytes32 id = _deposit(alice, 0x01, IPheromoneRegistry.PheromoneType.THREAT, 10000);

        // At deposit: full intensity
        assertEq(registry.currentIntensity(id), 10000);

        // 1 half-life (100 blocks from deposit): 5000
        vm.roll(1100);
        assertEq(registry.currentIntensity(id), 5000);

        // 2 half-lives (200 blocks from deposit): 2500
        vm.roll(1200);
        assertEq(registry.currentIntensity(id), 2500);

        // 10 half-lives (1000 blocks from deposit): 10000 >> 10 = 9 (near death)
        vm.roll(2000);
        assertTrue(registry.currentIntensity(id) < 10, "Should be near zero after 10 half-lives");

        // 14 half-lives (1400 blocks from deposit): effectively 0
        vm.roll(2400);
        assertEq(registry.currentIntensity(id), 0);
    }

    // ----------------------------------------------------------------
    // test_sinr_calculation
    // ----------------------------------------------------------------

    function test_sinr_calculation() public {
        // Single pheromone: SINR = signal / (0 + 1) = signal
        bytes32 id1 = _deposit(alice, 0x01, IPheromoneRegistry.PheromoneType.THREAT, 1000);
        uint64 sinr1 = registry.sinr(id1);
        assertEq(sinr1, 1000); // 1000 / (0 + 1)

        // Add interferer at same location, same type
        vm.roll(block.number + 1);
        bytes32 id2 = _deposit(bob, 0x01, IPheromoneRegistry.PheromoneType.THREAT, 1000);

        // id1 has decayed slightly (1 block, HL=100): ~995
        // id2 is fresh: 1000
        // SINR of id2 = 1000 / (995 + 1) = 1 (integer division)
        uint64 sinr2 = registry.sinr(id2);
        assertTrue(sinr2 >= 1, "SINR should be at least 1 with interferer");
        assertTrue(sinr2 <= 2, "SINR should be low with equal interferer");

        // Different type at same location: no interference
        vm.roll(block.number + 1);
        bytes32 id3 = _deposit(charlie, 0x01, IPheromoneRegistry.PheromoneType.OPPORTUNITY, 5000);
        uint64 sinr3 = registry.sinr(id3);
        assertEq(sinr3, 5000); // No same-type interferers: 5000 / (0 + 1)
    }

    // ----------------------------------------------------------------
    // test_cleanup_removes_dead_pheromones
    // ----------------------------------------------------------------

    function test_cleanup_removes_dead_pheromones() public {
        bytes32 id = _deposit(alice, 0x01, IPheromoneRegistry.PheromoneType.THREAT, 1000);

        // Kill it: advance past death (20+ half-lives)
        vm.roll(block.number + 2100);
        assertEq(registry.currentIntensity(id), 0);

        // Cleanup by charlie
        vm.prank(charlie);
        registry.cleanup(id);

        // Should be deleted
        (, address depositor,,,,) = registry.pheromones(id);
        assertEq(depositor, address(0));

        // Deposit tracking cleared
        assertEq(registry.deposits(id), 0);
    }

    function test_cleanup_alive_reverts() public {
        bytes32 id = _deposit(alice, 0x01, IPheromoneRegistry.PheromoneType.THREAT, 1000);
        vm.expectRevert("PheromoneRegistry: not dead yet");
        registry.cleanup(id);
    }

    function test_cleanup_rewards_caller_and_depositor() public {
        bytes32 id = _depositWithValue(
            alice, 0x01, IPheromoneRegistry.PheromoneType.THREAT, 1000, 1 ether
        );

        vm.roll(block.number + 2100);

        // Charlie cleans up
        vm.prank(charlie);
        registry.cleanup(id);

        // Charlie gets 5% of remaining deposit (1 ether)
        uint256 cleanupReward = 1 ether / 20; // 0.05 ether
        assertEq(registry.pendingWithdrawals(charlie), cleanupReward);

        // Alice gets remaining 95%
        uint256 depositorReturn = 1 ether - cleanupReward; // 0.95 ether
        assertEq(registry.pendingWithdrawals(alice), depositorReturn);
    }

    // ----------------------------------------------------------------
    // test_confirm_rewards_confirmer
    // ----------------------------------------------------------------

    function test_confirm_rewards_confirmer() public {
        bytes32 id = _depositWithValue(
            alice, 0x01, IPheromoneRegistry.PheromoneType.THREAT, 1000, 1 ether
        );

        // First confirmer (bob): gets 10% = 0.1 ether
        vm.prank(bob);
        registry.confirm(id);
        assertEq(registry.pendingWithdrawals(bob), 0.1 ether);

        // Second confirmer (charlie): gets 10%/2 = 0.05 ether
        vm.prank(charlie);
        registry.confirm(id);
        assertEq(registry.pendingWithdrawals(charlie), 0.05 ether);

        // Third confirmer (dave): gets 10%/3 = 0.0333... ether
        vm.prank(dave);
        registry.confirm(id);
        uint256 deposit_ = 1 ether;
        uint256 thirdReward = deposit_ / 30; // integer division
        assertEq(registry.pendingWithdrawals(dave), thirdReward);
    }

    // ----------------------------------------------------------------
    // test_withdraw_pull_pattern
    // ----------------------------------------------------------------

    function test_withdraw_pull_pattern() public {
        bytes32 id = _depositWithValue(
            alice, 0x01, IPheromoneRegistry.PheromoneType.THREAT, 1000, 1 ether
        );

        // Bob confirms -> gets reward
        vm.prank(bob);
        registry.confirm(id);

        uint256 expected = 0.1 ether;
        assertEq(registry.pendingWithdrawals(bob), expected);

        // Bob withdraws
        uint256 bobBalanceBefore = bob.balance;
        vm.prank(bob);
        registry.withdraw();

        assertEq(registry.pendingWithdrawals(bob), 0);
        assertEq(bob.balance, bobBalanceBefore + expected);
    }

    function test_withdraw_nothing_reverts() public {
        vm.prank(alice);
        vm.expectRevert("PheromoneRegistry: nothing to withdraw");
        registry.withdraw();
    }

    // ----------------------------------------------------------------
    // test_all_three_pheromone_types
    // ----------------------------------------------------------------

    function test_all_three_pheromone_types() public {
        // Deposit all three types
        bytes32 threatId = _deposit(alice, 0x01, IPheromoneRegistry.PheromoneType.THREAT, 1000);
        vm.roll(block.number + 1);
        bytes32 oppId = _deposit(bob, 0x02, IPheromoneRegistry.PheromoneType.OPPORTUNITY, 1000);
        vm.roll(block.number + 1);
        bytes32 wisdomId = _deposit(charlie, 0x03, IPheromoneRegistry.PheromoneType.WISDOM, 1000);

        // Verify types stored correctly
        (,,,,, uint8 threatType) = registry.pheromones(threatId);
        (,,,,, uint8 oppType) = registry.pheromones(oppId);
        (,,,,, uint8 wisdomType) = registry.pheromones(wisdomId);
        assertEq(threatType, 0); // THREAT
        assertEq(oppType, 1);    // OPPORTUNITY
        assertEq(wisdomType, 2); // WISDOM

        // After 100 blocks: THREAT should be ~halved, OPPORTUNITY/WISDOM barely decayed
        vm.roll(block.number + 100);

        uint64 threatInt = registry.currentIntensity(threatId);
        uint64 oppInt = registry.currentIntensity(oppId);
        uint64 wisdomInt = registry.currentIntensity(wisdomId);

        // THREAT HL=100: ~500 after 100 blocks from deposit (102 blocks total)
        assertTrue(threatInt <= 510, "THREAT should be about half");
        assertTrue(threatInt >= 450, "THREAT should be about half");

        // OPPORTUNITY HL=250: only ~40% through first halving (~101 blocks)
        assertTrue(oppInt > threatInt, "OPPORTUNITY decays slower");

        // WISDOM HL=1000: only ~10% through first halving (~100 blocks)
        assertTrue(wisdomInt > oppInt, "WISDOM decays slowest");
        assertTrue(wisdomInt >= 900, "WISDOM should still be near initial");
    }

    // ----------------------------------------------------------------
    // Additional tests for coverage
    // ----------------------------------------------------------------

    function test_deposit_emits_event() public {
        bytes memory location = _makeLocation(0x01);
        bytes32 locationHash = keccak256(location);

        vm.prank(alice);
        vm.expectEmit(false, true, true, false);
        emit IPheromoneRegistry.PheromoneDeposited(
            bytes32(0), locationHash, alice,
            uint8(IPheromoneRegistry.PheromoneType.THREAT),
            1000, uint64(block.number)
        );
        registry.deposit{value: 0.001 ether}(
            location,
            IPheromoneRegistry.PheromoneType.THREAT,
            1000
        );
    }

    function test_deposit_invalid_vector() public {
        bytes memory shortVector = new bytes(100);
        vm.prank(alice);
        vm.expectRevert("PheromoneRegistry: invalid vector size");
        registry.deposit{value: 0.001 ether}(
            shortVector,
            IPheromoneRegistry.PheromoneType.THREAT,
            1000
        );
    }

    function test_deposit_intensity_too_low() public {
        bytes memory location = _makeLocation(0x01);
        vm.prank(alice);
        vm.expectRevert("PheromoneRegistry: intensity out of range");
        registry.deposit{value: 0.001 ether}(
            location,
            IPheromoneRegistry.PheromoneType.THREAT,
            50
        );
    }

    function test_deposit_intensity_too_high() public {
        bytes memory location = _makeLocation(0x01);
        vm.prank(alice);
        vm.expectRevert("PheromoneRegistry: intensity out of range");
        registry.deposit{value: 0.001 ether}(
            location,
            IPheromoneRegistry.PheromoneType.THREAT,
            20_000
        );
    }

    function test_confirm_double_reverts() public {
        bytes32 id = _deposit(alice, 0x01, IPheromoneRegistry.PheromoneType.THREAT, 1000);

        vm.prank(bob);
        registry.confirm(id);

        vm.prank(bob);
        vm.expectRevert("PheromoneRegistry: already confirmed");
        registry.confirm(id);
    }

    function test_confirm_dead_reverts() public {
        bytes32 id = _deposit(alice, 0x01, IPheromoneRegistry.PheromoneType.THREAT, 1000);
        vm.roll(block.number + 2100);

        vm.prank(bob);
        vm.expectRevert("PheromoneRegistry: pheromone is dead");
        registry.confirm(id);
    }

    function test_read_returns_strongest_and_sinr() public {
        bytes memory location = _makeLocation(0x01);
        bytes32 locationHash = keccak256(location);

        // Deposit two THREATs at same location
        _deposit(alice, 0x01, IPheromoneRegistry.PheromoneType.THREAT, 5000);
        vm.roll(block.number + 1);
        _deposit(bob, 0x01, IPheromoneRegistry.PheromoneType.THREAT, 1000);

        (uint64 intensity, uint64 sinrValue) = registry.read(
            locationHash, IPheromoneRegistry.PheromoneType.THREAT
        );

        // Alice's pheromone is stronger (5000 vs 1000, both barely decayed)
        assertTrue(intensity >= 4900, "Should return strongest intensity");
        assertTrue(intensity <= 5000, "Should return strongest intensity");

        // SINR = ~4997 / (~997 + 1) = ~5
        assertTrue(sinrValue >= 3, "SINR should reflect interference");
        assertTrue(sinrValue <= 6, "SINR should reflect interference");
    }

    function test_read_no_pheromones_returns_zero() public {
        bytes32 locationHash = keccak256(_makeLocation(0xFF));
        (uint64 intensity, uint64 sinrValue) = registry.read(
            locationHash, IPheromoneRegistry.PheromoneType.THREAT
        );
        assertEq(intensity, 0);
        assertEq(sinrValue, 0);
    }

    function test_cleanup_batch_mixed() public {
        bytes32 id1 = _deposit(alice, 0x01, IPheromoneRegistry.PheromoneType.THREAT, 1000);
        vm.roll(block.number + 1);
        bytes32 id2 = _deposit(bob, 0x02, IPheromoneRegistry.PheromoneType.WISDOM, 1000);

        // Advance enough to kill THREAT (HL=100) but not WISDOM (HL=1000)
        vm.roll(block.number + 2100);

        bytes32[] memory ids = new bytes32[](2);
        ids[0] = id1;
        ids[1] = id2;

        registry.cleanupBatch(ids);

        // THREAT should be cleaned
        (, address dep1,,,,) = registry.pheromones(id1);
        assertEq(dep1, address(0), "THREAT should be cleaned up");

        // WISDOM should still exist (HL=1000, at ~2101 blocks: q=2, intensity = 1000>>2 = 250)
        (, address dep2,,,,) = registry.pheromones(id2);
        assertEq(dep2, bob, "WISDOM should still exist");
    }

    function test_decay_accuracy_exact_halving() public {
        vm.roll(1000);
        bytes32 id = _deposit(alice, 0x01, IPheromoneRegistry.PheromoneType.THREAT, 10000);

        vm.roll(1100); // 100 blocks = 1 HL
        assertEq(registry.currentIntensity(id), 5000);

        vm.roll(1200); // 200 blocks = 2 HL
        assertEq(registry.currentIntensity(id), 2500);
    }

    function test_confirm_preserves_intensity() public {
        bytes32 id = _deposit(alice, 0x01, IPheromoneRegistry.PheromoneType.THREAT, 1000);

        vm.roll(block.number + 50);
        uint64 before = registry.currentIntensity(id);

        vm.prank(bob);
        registry.confirm(id);

        uint64 after_ = registry.currentIntensity(id);
        assertEq(after_, before);
    }

    function test_cleanup_emits_decayed_event() public {
        bytes32 id = _deposit(alice, 0x01, IPheromoneRegistry.PheromoneType.THREAT, 1000);
        vm.roll(block.number + 2100);

        vm.expectEmit(true, false, false, false);
        emit IPheromoneRegistry.PheromoneDecayed(id);

        vm.prank(charlie);
        registry.cleanup(id);
    }

    function test_withdraw_emits_event() public {
        bytes32 id = _depositWithValue(
            alice, 0x01, IPheromoneRegistry.PheromoneType.THREAT, 1000, 1 ether
        );

        vm.prank(bob);
        registry.confirm(id);

        vm.expectEmit(true, false, false, true);
        emit IPheromoneRegistry.WithdrawalClaimed(bob, 0.1 ether);

        vm.prank(bob);
        registry.withdraw();
    }
}
