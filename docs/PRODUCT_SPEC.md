# Product specification

## Problem

Developers need a reproducible answer to a narrow question: after an MCP stdio
server starts, does initialization succeed and does the server exit when its
input closes? Existing operating-system inspection shows too much unrelated
state and creates a risk of terminating unrelated processes.

## Contract

MCP Process Guard launches exactly one explicit command without a shell, creates
an isolated child process group, optionally performs the newline-delimited MCP
`initialize` request and `notifications/initialized` exchange, closes stdin,
and waits for a configured grace period. A
timeout triggers cleanup of that owned group. If the root exits first, the tool
still queries its Unix process group or Windows Job Object for owned descendants.
Reports contain outcome, handshake status, elapsed time, exit code, descendant
detection, and structured cleanup proof.

On Unix, `cleanup: succeeded` proves that the group-directed termination signal
was accepted and the direct child was reaped within the cleanup deadline. A
killed descendant can remain briefly as a non-running zombie until its new
parent reaps it; the tool does not scan the machine to inspect that unrelated
parent. On Windows, success additionally requires Job Object active-process
accounting to reach zero.

## Non-goals

- Running as a daemon or general process reaper.
- Discovering, scanning, or killing unrelated processes.
- Forwarding arbitrary MCP traffic or validating full protocol conformance.
- Capturing payloads, logs, arguments, or environment values.
- Sandboxing untrusted executables.

## Acceptance criteria

- Clean exit, non-zero exit, handshake failure, and timeout are distinguishable.
- Waiting is bounded and timeout cleanup targets only the owned process group.
- Human and JSON output never echo the launched command or its arguments.
- Handshake frames and all waits have validated upper bounds.
- Linux, macOS, and Windows builds are covered in CI; platform limitations remain documented.

On Unix, descendants that deliberately create a new session with `setsid` leave
the owned process group and cannot be attributed or cleaned by this tool. On
Windows, ownership and accounting use a private Job Object with
`KILL_ON_JOB_CLOSE`.
