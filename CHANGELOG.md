# Changelog

All notable changes are documented here.

## [0.1.0-alpha.1] - 2026-08-31

- Add bounded MCP stdio initialization and shutdown checks.
- Detect descendants that survive their root process and report structured cleanup proof.
- Use owned Unix process groups and Windows Job Objects for bounded cleanup.
- Bound handshake frames, grace periods, cleanup, and every child wait.
- Add redacted human and JSON reports with distinct launch, ownership, wait, and cleanup outcomes.
- Report bounded cleanup uncertainty explicitly instead of claiming success.
- Add bilingual documentation, tests, CI, and release governance.
