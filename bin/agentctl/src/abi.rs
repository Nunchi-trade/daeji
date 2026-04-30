//! `sol!` ABIs for the contracts agentctl talks to. Mirrors the canonical
//! interfaces in contracts-core; signatures must stay in sync.

use alloy::sol;

sol! {
    #[sol(rpc)]
    interface IAgentRegistry {
        function register(string calldata capabilities, bytes32 passportHash) external;
        function isActive(address agent) external view returns (bool);
    }

    #[sol(rpc)]
    interface IRoleRegistry {
        function OPERATOR_ROLE() external view returns (bytes32);
        function MANAGER_ROLE() external view returns (bytes32);
        function hasRole(bytes32 role, address account) external view returns (bool);
        function grantRole(bytes32 role, address account) external;
    }

    #[sol(rpc)]
    interface IWorkerRegistry {
        function register(uint256 amount) external;
        function tier(address worker) external view returns (uint8);
        function reputationOf(address worker) external view returns (uint256);
        function updateReputation(address worker, bool outcome) external;
        function MIN_BOND() external view returns (uint256);
    }

    #[sol(rpc)]
    interface IMockERC20 {
        function mint(address to, uint256 amount) external;
        function approve(address spender, uint256 amount) external returns (bool);
        function balanceOf(address account) external view returns (uint256);
    }

    #[sol(rpc)]
    interface IMultiAgentMarket {
        function postMultiJob(
            bytes32 specHash,
            uint256 bounty,
            uint64 deadline,
            uint64 requiredCapabilities,
            uint8 numAgents
        ) external returns (uint256 id);

        function bid(uint256 id, uint256 askPrice, uint64 etaBlocks) external;

        function awardJob(uint256 id, address[] calldata winners) external;

        function submitMulti(uint256 id, bytes32 resultHash, bytes[] calldata signatures) external;

        function resolve(uint256 id, bool accepted) external;

        function stateOf(uint256 id) external view returns (uint8);
        function getWinners(uint256 id) external view returns (address[] memory);
    }

    #[sol(rpc)]
    interface IJobTypeRegistry {
        function register(
            bytes32 jobType,
            string calldata description,
            uint8 minTier,
            uint256 minBounty,
            uint64 maxDeadlineOffset,
            bytes calldata metadata
        ) external;
    }
}
