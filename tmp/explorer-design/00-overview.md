# Kora Explorer — Design Overview

A cinematic block explorer for kora, built in the ROSEDUST design language.
Not a table of transactions — a living instrument that renders the chain as a physical, atmospheric space.

## Documents

| File | Contents |
|------|----------|
| [01-rpc-enablement](./01-rpc-enablement.md) | What new RPC/WS capabilities unlock for the explorer |
| [02-visual-architecture](./02-visual-architecture.md) | Core scenes, layout, and atmospheric system |
| [03-scenes](./03-scenes.md) | Detailed spec for each visual scene |
| [04-data-flows](./04-data-flows.md) | How chain data feeds into visual elements |
| [05-interaction](./05-interaction.md) | Mouse, keyboard, and touch interaction model |
| [06-tech-stack](./06-tech-stack.md) | Framework, dependencies, build config, deployment |
| [07-performance](./07-performance.md) | Frame budgets, progressive enhancement, optimization |
| [08-consensus-data](./08-consensus-data.md) | Consensus event streaming, safety violations, round lifecycle |

## Premise

The chain is not an API to query. It is a space to inhabit.

Blocks are not rows. They are terrain.
Transactions are not lists. They are light moving between bodies.
State roots are not hex strings. They are the shape of everything that exists.

The explorer should feel like mission control crossed with a James Turrell installation —
precise data rendered as atmosphere.
