# Pierce RTS Engine — Project Instructions

## VCS & GitHub

- **Use GitHub MCP** (`mcp__github__*`) for all GitHub operations (PRs, issues, branches, CI status, code search).
- **No jj in this repo.** This is a git-only repo — use `git` commands for local VCS operations.
- **Always use `git -C <path>`** for direct git commands. Never `cd && git`.

## Jira

- **Use Docker MCP** (`mcp__MCP_DOCKER__*`) for all Jira operations. Do NOT use `mcp__mcp-atlassian__*` (broken API).
- **Cloud ID:** `aa7c9251-e0d9-46d7-a7cf-5324859b4b7f` — required for all Docker MCP Jira calls.
- **Fallback:** `scripts/jira.sh` provides curl-based Jira access (requires `JIRA_API_TOKEN` env var).
- Key tools: `searchJiraIssuesUsingJql`, `getJiraIssue`, `transitionJiraIssue`, `editJiraIssue`, `createJiraIssue`, `addCommentToJiraIssue`.
- **Large results:** When MCP output exceeds inline limits and is saved to a file, parse it with `scripts/jira-parse.cmd <file>` (PowerShell, no node).

## Build & Test

```bash
cargo test --release --workspace -- --test-threads=4  # run all tests (release, capped threads)
cargo clippy --workspace --all-targets                # lint (CI uses -D warnings via RUSTFLAGS)
cargo fmt --all --check                               # format check
cargo build --release -p bar-game                     # full release build
```

- **Always run tests in release mode** (`--release`). Debug tests are 5-10x slower.
- Build parallelism is capped to 4 jobs via `.cargo/config.toml`.
- Always pass `--test-threads=4` (or set `RUST_TEST_THREADS=4`) to limit test parallelism.
- Use `--message-format=short` on cargo build/clippy during iterative fix loops.

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

## Agent Workflow

Use `/start-story RR-123` to begin work on a Jira story (transitions issue, launches worktree agent).
Use `/merge-story RR-123` to review, merge, and close a completed story.
Use `/check` to run the full test + clippy pipeline.

## Testing Strategy (RR-67)

### Test Layers

1. **Unit tests** (`cargo test`): Every system, component, algorithm. Run on every push.
2. **Determinism tests** (CI-blocking): Run two headless sims, compare checksums. The most critical test.
3. **Integration tests**: Full sim scenarios run headless, assert end state.
