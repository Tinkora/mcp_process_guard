# Repository guide for AI agents

## Product boundary

This repository provides a local lifecycle diagnostic for one explicitly
launched MCP stdio server. It must never scan the machine, become a background
reaper, log commands or secrets, or terminate PIDs it did not create.

Unix containment is the child-created session/process group. Windows containment
is a private Job Object configured with `KILL_ON_JOB_CLOSE`. Never replace these
with PID-name matching, `taskkill`, or a machine-wide process scan.

## Conventions

- Use English Conventional Commits and English code comments.
- Keep the default README in English with a Chinese entry point.
- Preserve the narrow, offline product boundary.

## Required checks

```bash
cargo fmt --all -- --check
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
```
