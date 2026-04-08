# Start Story

Work on a Jira story: transition it, create a worktree agent, and implement.

## Usage
`/start-story RR-123`

## Steps

1. **Fetch the Jira issue** using `mcp__mcp-atlassian__jira_get_issue` with the key from $ARGUMENTS.
2. **Transition to "In Progress"** using `mcp__mcp-atlassian__jira_transition_issue` with transition_id `21`.
3. **Read the issue description** — extract acceptance criteria, relevant crate, and design decisions.
4. **Launch a worktree agent** (`isolation: "worktree"`) with a prompt that includes:
   - The Jira issue key and summary
   - Acceptance criteria from the description
   - Relevant crate(s) to modify
   - Instructions to read CLAUDE.md, write tests, run `cargo test` and `cargo clippy`
   - Instruction to commit with the Jira key in the message
5. **Report** the agent ID and worktree path so the user can track progress.

## Parallel Execution Rules

- Two agents NEVER work on the same crate at the same time.
- Shared types in `pierce-math` or `pierce-model` must be merged before dependent agents start.

## Jira Transition IDs

- "In Progress": `21`
- "Done": `31`
- Use `mcp__mcp-atlassian__jira_get_transitions` if IDs change.
