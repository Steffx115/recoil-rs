# Check

Run the full build verification pipeline.

## Usage
`/check`

## Steps

Run these in parallel:
1. `cargo test --workspace --message-format=short`
2. `cargo clippy --workspace --all-targets --message-format=short`

Report: total tests passed, any failures, any new clippy warnings.
