# Merge Story

Review a completed worktree agent's work, merge to master, and close the Jira issue.

## Usage
`/merge-story RR-123`

## Steps

1. **Find the worktree branch** — check `git worktree list` and `git branch` for the agent's branch.
2. **Review the diff** — `git log --oneline master..BRANCH` and `git diff --stat master..BRANCH`.
3. **Verify tests pass** — run `cargo test --workspace --message-format=short` in the worktree if not already verified.
4. **Merge into master** — `git merge BRANCH --no-edit` from the main worktree. If conflicts exist, merge master INTO the branch first (per CLAUDE.md: merge, don't rebase).
5. **Clean up** — remove the worktree (`git worktree remove PATH --force`) and delete the branch (`git branch -D BRANCH`).
6. **Sync with remote** — `git push` to push the merged master to origin.
7. **Transition Jira to "Done"** — `mcp__mcp-atlassian__jira_transition_issue` with transition_id `31`.
8. **Add a comment** to the Jira issue summarizing what was implemented.

## Jira Transition IDs

- "Done": `31`
- Use `mcp__mcp-atlassian__jira_get_transitions` if IDs change.
