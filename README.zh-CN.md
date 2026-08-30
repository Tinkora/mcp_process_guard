# MCP Process Guard

[English](README.md)

一个本地、离线的 CLI，用于检查用户明确指定的 MCP stdio 服务是否能完成初始化，
并在输入关闭后正常退出。

## 为什么需要它

当初始化失败或客户端消失时，MCP stdio 服务可能继续驻留。复现这类生命周期问题
不应依赖全机进程扫描或常驻清理程序。本工具只启动一个命令，可选执行 MCP
`initialize` 交换，随后关闭 stdin、在有限宽限期内等待，并只清理由本次调用创建的
进程组。

## 安装与使用

```bash
cargo install --path .
mcp-process-guard -- your-mcp-server --stdio
mcp-process-guard --no-handshake --grace-ms 1000 --output json -- your-command
```

退出码：正常零退出为 `0`，子进程失败为 `1`，超时为 `3`，初始化失败为 `4`，
启动或所有权边界失败为 `5`，根进程退出后仍有后代为 `6`，无法在期限内证明清理
完成为 `7`。

## 隐私与边界

- 不联网、不常驻、不扫描全机进程，也不修改配置。
- 报告不包含命令、参数、环境变量、协议载荷、stdout 或 stderr，默认避免泄露密钥参数。
- Unix 使用子进程所属进程组/会话；主动调用 `setsid` 创建新会话的后代会逃逸，超出
  本工具的所有权边界。
- Windows 使用带 `KILL_ON_JOB_CLOSE` 的私有 Job Object，不调用 `taskkill`，也不做
  全机 PID 查询。
- 握手目前只接受一行 JSON 响应。
- Unix 行为在本地和 CI 中执行；Windows CI 执行 Job Object 根进程和后代场景。
- 这是生命周期诊断工具，不是沙箱或恶意程序隔离系统。

更多信息见[产品规格](docs/PRODUCT_SPEC.zh-CN.md)、[安全策略](SECURITY.zh-CN.md)
和[贡献指南](CONTRIBUTING.zh-CN.md)。

如果它帮你节省了时间，可以在 [Ko-fi](https://ko-fi.com/tinkora) 支持 Tinkora。

## 许可证

MIT
