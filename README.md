# Pierce RTS Engine (Rust)

A deterministic real-time strategy engine written in Rust, inspired by the Spring/Pierce RTS engine. Designed for competitive multiplayer with lockstep networking and full replay support.

## Architecture

The engine is organized as a Cargo workspace with strict crate boundaries:

| Crate | Purpose |
|-------|---------|
| `pierce-math` | Deterministic fixed-point math (SimFloat, vectors, matrices) |
| `pierce-sim` | ECS-based simulation: units, combat, economy, pathfinding |
| `pierce-net` | Lockstep networking protocol, replay recording/playback |
| `pierce-render` | wgpu-based rendering pipeline, terrain, models |
| `pierce-ui` | In-game UI framework (egui-based) |
| `pierce-audio` | Spatial audio via kira |
| `bar-game` | Game binary: unit definitions, factions, game-specific logic |

## Build & Test

```bash
cargo test --workspace                    # run all tests
cargo clippy -- -D warnings               # lint
cargo fmt --check                         # format check
cargo build --release -p bar-game         # release build
```

## License

Dual-licensed under MIT or Apache-2.0, at your option.
