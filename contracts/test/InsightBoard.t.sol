// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

import {Test, console} from "forge-std/Test.sol";
import {InsightBoard} from "../src/InsightBoard.sol";
import {IInsightBoard} from "../src/IInsightBoard.sol";

/// @title TestableInsightBoard
/// @notice A test-only subclass that overrides precompile-dependent functions
///         so tests can run without an actual HDC precompile at 0x09.
contract TestableInsightBoard is InsightBoard {
    /// @dev Override submit -- identical to parent but skip any precompile calls.
    function submit(
        Kind kind,
        bytes calldata vector,
        bytes calldata content
    ) external payable override returns (bytes32 insightId) {
        require(vector.length == 1280, "InsightBoard: invalid vector size");
        require(msg.value >= MIN_STAKE, "InsightBoard: insufficient stake");

        bytes32 vectorHash = keccak256(vector);

        insightId = keccak256(
            abi.encodePacked(vectorHash, msg.sender, block.number)
        );

        require(
            insights[insightId].anchor.publishBlock == 0,
            "InsightBoard: ID collision"
        );

        insights[insightId] = Insight({
            anchor: InsightAnchor({
                vectorHash: vectorHash,
                contentHash: keccak256(content),
                author: msg.sender,
                publishBlock: uint64(block.number),
                kind: uint8(kind),
                tier: uint8(Tier.TRANSIENT),
                state: uint8(State.SUBMITTED)
            }),
            confirmations: 0,
            lastConfirmedBlock: uint64(block.number),
            confirmsSinceChallenge: 0,
            stakedAmount: msg.value,
            originalStake: msg.value,
            unlockedAmount: 0
        });

        emit InsightPublished(
            insightId, vectorHash, msg.sender,
            vector, content,
            uint8(kind), uint8(Tier.TRANSIENT)
        );

        emit InsightStateChanged(
            insightId, type(uint8).max, uint8(State.SUBMITTED)
        );

        emit StakeDeposited(insightId, msg.sender, msg.value);
    }

    /// @dev Override purge -- skip precompile deleteVector call.
    function purge(bytes32 insightId) external override {
        Insight storage insight = insights[insightId];

        require(
            insight.anchor.publishBlock != 0,
            "InsightBoard: insight does not exist"
        );
        require(
            computeState(insightId) == State.PURGED,
            "InsightBoard: not yet purgeable"
        );

        address author = insight.anchor.author;
        uint256 staked = insight.stakedAmount;
        uint256 authorLegacy = staked / 10;
        uint256 cleanupReward = staked / 20;

        if (authorLegacy > 0) {
            pendingWithdrawals[author] += authorLegacy;
        }
        if (cleanupReward > 0) {
            pendingWithdrawals[msg.sender] += cleanupReward;
        }

        delete insights[insightId];

        emit InsightPurged(insightId, msg.sender);
    }
}

contract InsightBoardTest is Test {
    TestableInsightBoard board;
    address alice = makeAddr("alice");
    address bob = makeAddr("bob");
    address charlie = makeAddr("charlie");
    address dave = makeAddr("dave");

    struct InsightView {
        bytes32 vectorHash;
        bytes32 contentHash;
        address author;
        uint64 publishBlock;
        uint8 kind;
        uint8 tier;
        uint8 state;
        uint64 confirmations;
        uint64 lastConfirmedBlock;
        uint64 confirmsSinceChallenge;
        uint256 stakedAmount;
        uint256 originalStake;
        uint256 unlockedAmount;
    }

    function setUp() public {
        board = new TestableInsightBoard();
        vm.deal(alice, 100 ether);
        vm.deal(bob, 100 ether);
        vm.deal(charlie, 100 ether);
        vm.deal(dave, 100 ether);
    }

    function _makeVector(uint8 seed) internal pure returns (bytes memory) {
        bytes memory v = new bytes(1280);
        for (uint256 i = 0; i < 1280; i++) {
            v[i] = bytes1(seed);
        }
        return v;
    }

    function _submitInsight(address sender, uint8 seed) internal returns (bytes32) {
        bytes memory vector = _makeVector(seed);
        bytes memory content = abi.encodePacked("content-", seed);
        vm.prank(sender);
        return board.submit{value: 0.01 ether}(
            IInsightBoard.Kind.INSIGHT, vector, content
        );
    }

    function _submitInsightWithStake(address sender, uint8 seed, uint256 stake) internal returns (bytes32) {
        bytes memory vector = _makeVector(seed);
        bytes memory content = abi.encodePacked("content-", seed);
        vm.prank(sender);
        return board.submit{value: stake}(
            IInsightBoard.Kind.INSIGHT, vector, content
        );
    }

    function _submitAntiKnowledge(address sender, uint8 seed) internal returns (bytes32) {
        bytes memory vector = _makeVector(seed);
        bytes memory content = abi.encodePacked("anti-", seed);
        vm.prank(sender);
        return board.submit{value: 0.01 ether}(
            IInsightBoard.Kind.ANTI_KNOWLEDGE, vector, content
        );
    }

    function _getInsight(bytes32 id) internal view returns (InsightView memory v) {
        (
            InsightBoard.InsightAnchor memory anchor,
            uint64 confs,
            uint64 lastConf,
            uint64 confsSinceChall,
            uint256 staked,
            uint256 origStake,
            uint256 unlocked
        ) = board.insights(id);
        v = InsightView({
            vectorHash: anchor.vectorHash, contentHash: anchor.contentHash,
            author: anchor.author, publishBlock: anchor.publishBlock,
            kind: anchor.kind, tier: anchor.tier, state: anchor.state,
            confirmations: confs, lastConfirmedBlock: lastConf,
            confirmsSinceChallenge: confsSinceChall, stakedAmount: staked,
            originalStake: origStake, unlockedAmount: unlocked
        });
    }

    // ================================================================
    // test_submit_with_stake
    // ================================================================

    function test_submit_with_stake() public {
        bytes32 id = _submitInsightWithStake(alice, 0x01, 0.05 ether);
        assertTrue(id != bytes32(0));

        InsightView memory v = _getInsight(id);
        assertEq(v.author, alice);
        assertEq(v.state, uint8(IInsightBoard.State.SUBMITTED));
        assertEq(v.tier, uint8(IInsightBoard.Tier.TRANSIENT));
        assertEq(v.kind, uint8(IInsightBoard.Kind.INSIGHT));
        assertEq(v.confirmations, 0);
        assertEq(v.stakedAmount, 0.05 ether);
        assertEq(v.originalStake, 0.05 ether);
        assertEq(v.unlockedAmount, 0);
    }

    // ================================================================
    // test_submit_below_min_stake_reverts
    // ================================================================

    function test_submit_below_min_stake_reverts() public {
        bytes memory vector = _makeVector(0x01);
        vm.prank(alice);
        vm.expectRevert("InsightBoard: insufficient stake");
        board.submit{value: 0.001 ether}(IInsightBoard.Kind.INSIGHT, vector, "content");
    }

    // ================================================================
    // test_confirm_promotes_tier
    // ================================================================

    function test_confirm_promotes_tier() public {
        bytes32 id = _submitInsightWithStake(alice, 0x01, 1 ether);

        // 3 confirmations -> WORKING tier, 20% unlocked
        for (uint256 i = 0; i < 3; i++) {
            address confirmer = makeAddr(string(abi.encodePacked("confirmer", i)));
            vm.deal(confirmer, 1 ether);
            vm.prank(confirmer);
            board.confirm(id);
        }
        InsightView memory v = _getInsight(id);
        assertEq(v.tier, uint8(IInsightBoard.Tier.WORKING));
        assertEq(v.unlockedAmount, 0.2 ether); // 20% of 1 ether
        assertEq(board.pendingWithdrawals(alice), 0.2 ether);

        // 10 confirmations -> CONSOLIDATED tier, 40% cumulative
        for (uint256 i = 3; i < 10; i++) {
            address confirmer = makeAddr(string(abi.encodePacked("c", i)));
            vm.deal(confirmer, 1 ether);
            vm.prank(confirmer);
            board.confirm(id);
        }
        v = _getInsight(id);
        assertEq(v.tier, uint8(IInsightBoard.Tier.CONSOLIDATED));
        assertEq(v.unlockedAmount, 0.4 ether);
        assertEq(board.pendingWithdrawals(alice), 0.4 ether);

        // 25 confirmations -> PERSISTENT tier, 60% cumulative
        for (uint256 i = 10; i < 25; i++) {
            address confirmer = makeAddr(string(abi.encodePacked("p", i)));
            vm.deal(confirmer, 1 ether);
            vm.prank(confirmer);
            board.confirm(id);
        }
        v = _getInsight(id);
        assertEq(v.tier, uint8(IInsightBoard.Tier.PERSISTENT));
        assertEq(v.unlockedAmount, 0.6 ether);
        assertEq(board.pendingWithdrawals(alice), 0.6 ether);
        assertEq(v.stakedAmount, 0.4 ether); // 40% remains staked
    }

    // ================================================================
    // test_challenge_slashes_stake
    // ================================================================

    function test_challenge_slashes_stake() public {
        bytes32 targetId = _submitInsightWithStake(alice, 0x01, 1 ether);

        // Make it ACTIVE
        vm.prank(bob);
        board.confirm(targetId);

        bytes32 antiId = _submitAntiKnowledge(charlie, 0x02);

        uint256 charlieWithdrawalBefore = board.pendingWithdrawals(charlie);

        vm.prank(charlie);
        board.challenge(targetId, antiId);

        InsightView memory v = _getInsight(targetId);
        assertEq(v.state, uint8(IInsightBoard.State.CHALLENGED));

        // 50% of 1 ether slashed = 0.5 ether
        // Challenger gets 25% (0.25 ether), protocol gets 25% (0.25 ether)
        assertEq(v.stakedAmount, 0.5 ether);
        assertEq(
            board.pendingWithdrawals(charlie) - charlieWithdrawalBefore,
            0.25 ether
        );
        assertEq(
            board.pendingWithdrawals(board.PROTOCOL_FEE_ADDRESS()),
            0.25 ether
        );
    }

    // ================================================================
    // test_state_transitions
    // ================================================================

    function test_state_transitions() public {
        // INSIGHT kind: halfLife = 10,000 blocks, TRANSIENT tier mult = 1x
        // effectiveHL = 10,000
        bytes32 id = _submitInsight(alice, 0x01);
        assertEq(uint8(board.computeState(id)), uint8(IInsightBoard.State.SUBMITTED));

        // First confirm -> ACTIVE
        vm.prank(bob);
        board.confirm(id);
        assertEq(uint8(board.computeState(id)), uint8(IInsightBoard.State.ACTIVE));

        // age > 1x HL -> DECAYING
        vm.roll(block.number + 10_001);
        assertEq(uint8(board.computeState(id)), uint8(IInsightBoard.State.DECAYING));

        // age > 5x HL -> ARCHIVED
        vm.roll(block.number + 40_000); // total > 50,000
        assertEq(uint8(board.computeState(id)), uint8(IInsightBoard.State.ARCHIVED));

        // age > 10x HL -> PURGED
        vm.roll(block.number + 50_000); // total > 100,000
        assertEq(uint8(board.computeState(id)), uint8(IInsightBoard.State.PURGED));
    }

    // ================================================================
    // test_withdraw_pull_pattern
    // ================================================================

    function test_withdraw_pull_pattern() public {
        bytes32 id = _submitInsightWithStake(alice, 0x01, 1 ether);

        // Get 3 confirmations to promote to WORKING and unlock 20%
        for (uint256 i = 0; i < 3; i++) {
            address confirmer = makeAddr(string(abi.encodePacked("w", i)));
            vm.deal(confirmer, 1 ether);
            vm.prank(confirmer);
            board.confirm(id);
        }

        assertEq(board.pendingWithdrawals(alice), 0.2 ether);

        uint256 aliceBefore = alice.balance;
        vm.prank(alice);
        board.withdraw();
        assertEq(alice.balance - aliceBefore, 0.2 ether);
        assertEq(board.pendingWithdrawals(alice), 0);
    }

    function test_withdraw_nothing_reverts() public {
        vm.prank(alice);
        vm.expectRevert("InsightBoard: nothing to withdraw");
        board.withdraw();
    }

    // ================================================================
    // test_purge_returns_partial_stake
    // ================================================================

    function test_purge_returns_partial_stake() public {
        bytes32 id = _submitInsightWithStake(alice, 0x01, 1 ether);
        vm.prank(bob);
        board.confirm(id);

        // Roll past 10x half-life (INSIGHT=10,000, TRANSIENT=1x, so >100,000 blocks)
        vm.roll(block.number + 100_001);
        assertEq(uint8(board.computeState(id)), uint8(IInsightBoard.State.PURGED));

        uint256 aliceWithdrawalBefore = board.pendingWithdrawals(alice);
        uint256 daveWithdrawalBefore = board.pendingWithdrawals(dave);

        vm.prank(dave);
        board.purge(id);

        // 10% of 1 ether -> alice
        assertEq(board.pendingWithdrawals(alice) - aliceWithdrawalBefore, 0.1 ether);
        // 5% of 1 ether -> cleanup caller (dave)
        assertEq(board.pendingWithdrawals(dave) - daveWithdrawalBefore, 0.05 ether);

        // Storage cleared
        InsightView memory v = _getInsight(id);
        assertEq(v.publishBlock, 0);
        assertEq(v.author, address(0));
        assertEq(v.stakedAmount, 0);

        // Alice can withdraw
        uint256 aliceBefore = alice.balance;
        vm.prank(alice);
        board.withdraw();
        assertEq(alice.balance - aliceBefore, 0.1 ether);
    }

    // ================================================================
    // test_renew_archived_insight
    // ================================================================

    function test_renew_nonArchivedReverts() public {
        bytes32 id = _submitInsight(alice, 0x01);
        vm.prank(bob);
        board.confirm(id);
        vm.prank(alice);
        vm.expectRevert("InsightBoard: only ARCHIVED insights can be renewed");
        board.renew{value: 0.01 ether}(id);
    }

    // ================================================================
    // Additional tests
    // ================================================================

    function test_submit_happyPath() public {
        bytes32 id = _submitInsight(alice, 0x01);
        assertTrue(id != bytes32(0));

        InsightView memory v = _getInsight(id);
        assertEq(v.author, alice);
        assertEq(v.state, uint8(IInsightBoard.State.SUBMITTED));
        assertEq(v.tier, uint8(IInsightBoard.Tier.TRANSIENT));
        assertEq(v.kind, uint8(IInsightBoard.Kind.INSIGHT));
        assertEq(v.confirmations, 0);
        assertEq(v.stakedAmount, 0.01 ether);
    }

    function test_submit_invalidVectorSize() public {
        bytes memory shortVector = new bytes(100);
        vm.prank(alice);
        vm.expectRevert("InsightBoard: invalid vector size");
        board.submit{value: 0.01 ether}(IInsightBoard.Kind.INSIGHT, shortVector, "content");
    }

    function test_confirm_submittedToActive() public {
        bytes32 id = _submitInsight(alice, 0x01);
        vm.prank(bob);
        board.confirm(id);

        InsightView memory v = _getInsight(id);
        assertEq(v.state, uint8(IInsightBoard.State.ACTIVE));
        assertEq(v.confirmations, 1);
    }

    function test_confirm_doubleConfirmReverts() public {
        bytes32 id = _submitInsight(alice, 0x01);
        vm.prank(bob);
        board.confirm(id);
        vm.prank(bob);
        vm.expectRevert("InsightBoard: already confirmed");
        board.confirm(id);
    }

    function test_confirm_nonExistentReverts() public {
        vm.prank(bob);
        vm.expectRevert("InsightBoard: insight does not exist");
        board.confirm(bytes32(uint256(0xdead)));
    }

    function test_challenge_activeToChallenge() public {
        bytes32 targetId = _submitInsight(alice, 0x01);
        vm.prank(bob);
        board.confirm(targetId);

        bytes32 antiId = _submitAntiKnowledge(charlie, 0x02);
        vm.prank(charlie);
        board.challenge(targetId, antiId);

        InsightView memory v = _getInsight(targetId);
        assertEq(v.state, uint8(IInsightBoard.State.CHALLENGED));
        assertEq(v.confirmsSinceChallenge, 0);
    }

    function test_challenge_nonActiveReverts() public {
        bytes32 targetId = _submitInsight(alice, 0x01);
        bytes32 antiId = _submitAntiKnowledge(charlie, 0x02);
        vm.prank(charlie);
        vm.expectRevert("InsightBoard: target not ACTIVE");
        board.challenge(targetId, antiId);
    }

    function test_challenge_wrongKindReverts() public {
        bytes32 targetId = _submitInsight(alice, 0x01);
        vm.prank(bob);
        board.confirm(targetId);
        bytes32 notAntiId = _submitInsight(charlie, 0x02);
        vm.prank(charlie);
        vm.expectRevert("InsightBoard: challenger is not ANTI_KNOWLEDGE");
        board.challenge(targetId, notAntiId);
    }

    function test_confirm_challengedResolved() public {
        bytes32 targetId = _submitInsight(alice, 0x01);
        vm.prank(bob);
        board.confirm(targetId);
        bytes32 antiId = _submitAntiKnowledge(charlie, 0x02);
        vm.prank(charlie);
        board.challenge(targetId, antiId);

        for (uint256 i = 0; i < 5; i++) {
            address confirmer = makeAddr(string(abi.encodePacked("res", i)));
            vm.deal(confirmer, 1 ether);
            vm.prank(confirmer);
            board.confirm(targetId);
        }

        InsightView memory v = _getInsight(targetId);
        assertEq(v.state, uint8(IInsightBoard.State.ACTIVE));
    }

    function test_computeState_withinHalfLife() public {
        bytes32 id = _submitInsight(alice, 0x01);
        vm.prank(bob);
        board.confirm(id);
        // INSIGHT HL = 10,000. Stay within.
        vm.roll(block.number + 5_000);
        assertEq(uint8(board.computeState(id)), uint8(IInsightBoard.State.ACTIVE));
    }

    function test_computeState_decaying() public {
        bytes32 id = _submitInsight(alice, 0x01);
        vm.prank(bob);
        board.confirm(id);
        // INSIGHT HL = 10,000. age > 1x HL -> DECAYING
        vm.roll(block.number + 10_001);
        assertEq(uint8(board.computeState(id)), uint8(IInsightBoard.State.DECAYING));
    }

    function test_computeState_archived() public {
        bytes32 id = _submitInsight(alice, 0x01);
        vm.prank(bob);
        board.confirm(id);
        // age > 5x HL = 50,000 -> ARCHIVED
        vm.roll(block.number + 50_001);
        assertEq(uint8(board.computeState(id)), uint8(IInsightBoard.State.ARCHIVED));
    }

    function test_computeState_purged() public {
        bytes32 id = _submitInsight(alice, 0x01);
        vm.prank(bob);
        board.confirm(id);
        // age > 10x HL = 100,000 -> PURGED
        vm.roll(block.number + 100_001);
        assertEq(uint8(board.computeState(id)), uint8(IInsightBoard.State.PURGED));
    }

    function test_computeState_nonExistent() public view {
        assertEq(
            uint8(board.computeState(bytes32(uint256(0xbeef)))),
            uint8(IInsightBoard.State.PURGED)
        );
    }

    function test_computeState_tierMultiplier() public {
        bytes32 id = _submitInsight(alice, 0x01);
        for (uint256 i = 0; i < 3; i++) {
            address confirmer = makeAddr(string(abi.encodePacked("tm", i)));
            vm.deal(confirmer, 1 ether);
            vm.prank(confirmer);
            board.confirm(id);
        }
        // WORKING tier: effective HL = 10,000 * 3 = 30,000
        // At 15,000 blocks should still be ACTIVE
        vm.roll(block.number + 15_000);
        assertEq(uint8(board.computeState(id)), uint8(IInsightBoard.State.ACTIVE));
    }

    function test_purge_notPurgeableReverts() public {
        bytes32 id = _submitInsight(alice, 0x01);
        vm.prank(bob);
        board.confirm(id);
        vm.expectRevert("InsightBoard: not yet purgeable");
        board.purge(id);
    }

    function test_purge_clearsStorage() public {
        bytes32 id = _submitInsight(alice, 0x01);
        vm.prank(bob);
        board.confirm(id);
        vm.roll(block.number + 100_001);
        board.purge(id);

        InsightView memory v = _getInsight(id);
        assertEq(v.publishBlock, 0);
        assertEq(v.author, address(0));
        assertEq(v.stakedAmount, 0);
    }
}
