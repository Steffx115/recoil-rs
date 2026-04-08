# Pierce RTS Engine — Project Instructions

## Build & Test

```bash
cargo test --workspace                          # run all tests
cargo clippy --workspace --all-targets          # lint (CI uses -D warnings via RUSTFLAGS)
cargo fmt --all --check                         # format check
cargo build --release -p bar-game               # full release build
```

Use `--message-format=short` on cargo build/clippy during iterative fix loops.

## Commit Discipline

- **Always commit when a batch of work is done.** Never leave large amounts of work uncommitted. A "batch" is any coherent set of changes that passes tests — typically one Jira story, one feature, or one bug fix.
- Commit before switching branches, before running worktree agents, and before any destructive git operations.
- If a session involves multiple stories, commit after each one completes (don't batch them all at the end).
- Worktree agents must not check out branches in the main worktree.

## Testing

- **Headless tests with every change:** Always extend or adapt the headless game tests in `bar-game-lib` when changing game logic, building, economy, AI, or commands. These tests simulate the full game loop without rendering.
- **Headless UI tests:** Always extend or adapt the UI interaction tests (prefixed `ui_`) in `bar-game-lib/src/game.rs` when changing input handling, selection, placement, or factory queuing. These tests use `click_select`, `click_move`, `handle_place`, etc. to simulate player actions and verify outcomes.

## Binary Sync

- **Keep the game binary up to date.** `crates/bar-game/src/main.rs` must always use `bar-game-lib::GameState` — never duplicate game logic in the binary. When game logic changes in `bar-game-lib`, update the binary in the same commit. The binary is a thin shell: window, renderer, input dispatch, egui overlay. All game state, tick loop, AI, building, economy, and selection live in `bar-game-lib`.

## Architecture Rules

- All simulation math MUST use `SimFloat` from `pierce-math`. Never use f32/f64 in sim code.
- ECS components must derive `Serialize, Deserialize` for checksum/replay support.
- `pierce-sim` must NOT depend on `pierce-render` or `pierce-ui`. Sim is headless-capable.
- Systems declare data access via ECS query signatures. No global mutable state.
- **Determinism**: no `HashMap` iteration in sim (use `BTreeMap` or sorted `Vec`). No thread-local RNG. No system time in sim.

## Code Style

- Rust 2021 edition, stable toolchain.
- No `unsafe` unless justified in a comment.
- Error handling: `anyhow` for application code, `thiserror` for library errors.
- Tests alongside code in `#[cfg(test)]` modules, not separate files.

## Code Navigation

- **Always use LSP when possible.** Prefer `documentSymbol`, `goToDefinition`, `findReferences`, `hover` over grep/read for understanding code structure. LSP gives accurate, type-aware results.

## Crate Boundaries

| Crate | Depends on | Purpose |
|-------|-----------|---------|
| `pierce-math` | serde | SimFloat, SimVec2/3. No other deps. |
| `pierce-model` | anyhow, bytemuck | ModelVertex, PieceTree, PieceTransform. No wgpu. |
| `pierce-cob` | pierce-model | COB animation parser, VM, CobAnimationDriver |
| `pierce-s3o` | pierce-model | S3O model loader (flat + hierarchical) |
| `pierce-sim` | pierce-math, bevy_ecs | ECS, game systems, spatial grid, pathfinding |
| `pierce-net` | pierce-sim, tokio | Lockstep protocol, replay |
| `pierce-render` | pierce-sim, pierce-model, pierce-cob, pierce-s3o, wgpu | Rendering pipeline |
| `pierce-ui` | pierce-render, pierce-sim, egui | UI framework |
| `pierce-audio` | pierce-sim, kira | Spatial audio (reads positions from sim) |
| `bar-game` | all crates | Game binary: unit defs, factions, game logic |

## Agent Workflow (RR-61)

### Workflow Per Story

1. Pull story from sprint. Write brief in Jira description with: acceptance criteria, relevant crate, API boundaries, design decisions.
2. **Transition the Jira issue to "In Progress"** before starting any work.
3. Launch agent in a worktree (`isolation: "worktree"`) with the brief.
4. Agent implements, writes tests, runs `cargo test` and `cargo clippy`.
5. Agent completes — **transition the Jira issue to "Ready for Merge"**.
6. Review the diff. If good: merge worktree branch into main. Close story.
7. If issues: send feedback via `SendMessage`, agent fixes (issue stays "In Progress").

### Jira Status Updates

Agents MUST keep Jira issue status current using `mcp__mcp-atlassian__jira_transition_issue`:
- **Starting work**: Transition to "In Progress" (id `21`) before writing any code.
- **Work complete**: Transition to "Ready for Merge" once tests pass and implementation is done.
- **Merged**: Transition to "Done" (id `31`) after the branch is merged to main.

Use `mcp__mcp-atlassian__jira_get_transitions` to discover available transition IDs if they change.

### Parallel Execution Rules

- Agents can run simultaneously on **different crates**.
- Two agents NEVER work on the **same crate** at the same time.
- Shared types (in `pierce-math`, `pierce-model`) must be merged before dependent agents start.

## Testing Strategy (RR-67)

### Test Layers

1. **Unit tests** (`cargo test`): Every system, component, algorithm. Run on every push.
2. **Determinism tests** (CI-blocking): Run two headless sims, compare checksums. The most critical test.
3. **Integration tests**: Full sim scenarios run headless, assert end state.
