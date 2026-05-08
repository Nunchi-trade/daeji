// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

import {Script, console} from "forge-std/Script.sol";
import {InsightBoard} from "../src/InsightBoard.sol";
import {PheromoneRegistry} from "../src/PheromoneRegistry.sol";

/// @title DeployHDC
/// @notice Foundry deployment script for InsightBoard and PheromoneRegistry.
contract DeployHDC is Script {
    function run() external {
        uint256 deployerKey = vm.envUint("DEPLOYER_PRIVATE_KEY");
        vm.startBroadcast(deployerKey);

        InsightBoard insightBoard = new InsightBoard();
        console.log("InsightBoard deployed at:", address(insightBoard));

        PheromoneRegistry pheromoneRegistry = new PheromoneRegistry();
        console.log("PheromoneRegistry deployed at:", address(pheromoneRegistry));

        vm.stopBroadcast();

        console.log("---");
        console.log("DUPLICATE_THRESHOLD:", insightBoard.DUPLICATE_THRESHOLD());
        console.log("MIN_STAKE:", insightBoard.MIN_STAKE());
        console.log("MIN_DEPOSIT:", pheromoneRegistry.MIN_DEPOSIT());
    }
}
