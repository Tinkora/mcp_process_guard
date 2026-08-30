# MCP Process Guard

[简体中文](README.zh-CN.md)

A local, offline CLI for checking whether one explicitly selected MCP stdio
server initializes and exits cleanly after its input closes.

## Why

MCP clients can leave a stdio server running when initialization fails or the
client disappears. Reproducing that lifecycle problem should not require a
machine-wide process scanner or an always-running reaper. MCP Process Guard
launches one command, optionally performs the MCP `initialize` exchange, closes
stdin, waits for a bounded grace period, and cleans up only the process group it
created.

## Install and use

```bash
cargo install --path .
mcp-process-guard -- your-mcp-server --stdio
mcp-process-guard --no-handshake --grace-ms 1000 --output json -- your-command
```

Exit codes are `0` for a clean zero exit, `1` for a child failure, `3` for a
grace timeout, `4` for an initialization failure, `5` for a launch/ownership
failure, `6` when owned descendants survived the root, and `7` when cleanup
could not be proved within its deadline.

`cleanup: unverified` means termination was requested and the direct child was
reaped, but the owned group remained observable until the cleanup deadline.

## Privacy and boundaries

- No network access, background service, machine-wide process scan, or config edits.
- The command, arguments, environment, protocol payloads, stdout, and stderr are
  not included in reports. This suppresses secret-like arguments by default.
- Unix uses the child-owned process group/session. A descendant that deliberately
  creates a new session with `setsid` escapes that containment and is outside
  this tool's ownership boundary.
- Windows uses a private Job Object with `KILL_ON_JOB_CLOSE`; no `taskkill` or
  machine-wide PID lookup is used.
- The handshake currently expects one newline-delimited JSON response.
- Unix behavior is exercised locally and in CI. Windows Job Object root and
  descendant scenarios execute on the Windows CI runner.
- This is a lifecycle diagnostic, not a sandbox or malware containment system.

See [the product specification](docs/PRODUCT_SPEC.md), [security policy](SECURITY.md),
and [contribution guide](CONTRIBUTING.md).

If this saves you time, you can [support Tinkora on Ko-fi](https://ko-fi.com/tinkora).

## License

MIT
