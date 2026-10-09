# Claude Code 与 CC Switch 导入验证

验证日期：2026-10-06，Windows x64。测试对象是 SuperCode 自身的命令、Agent 运行时、事件持久化和桌面界面，不是让 Claude / Codex 审查源代码。

## 本机真实连接

Claude Code 版本为 2.1.287。先使用本机 CLI 配置验证，再通过 SuperCode 导入 CC Switch 当前 Claude 配置 `kimi` 后验证。当前版本已将旧别名解析为真实 `k3[1M]` / `k3`，并完成新增 Kimi 连接的真实运行验证；使用的是 Kimi 模型，不代表 Anthropic 模型。

| 测试 | 结果 |
| --- | --- |
| 中文消息通过 SuperCode 发给真实 Claude CLI，收到随机验证码 | 通过 |
| 接收真实流式增量事件，并保存最终输出 | 通过 |
| 结束并释放 CLI，重新启动后恢复原生会话、回忆验证码 | 通过 |
| 通过 Claude 内置 Read 工具读取项目 README | 通过 |
| 停止运行中的任务，状态进入 interrupted | 通过 |
| Write 请求进入 waiting，并可从运行时查询到待确认请求 | 通过 |
| 未经批准不创建文件，停止审批中的任务后仍不创建文件 | 通过 |
| 从 CC Switch 读取、导入并启用当前 Claude 配置后重复上述操作 | 通过 |

修复了停止操作返回早于读取任务结束的时序问题：现在停止请求等待任务状态和最终事件处理完成后返回。中断任务也不会额外保存一条“连接意外关闭”的错误提示。

原始报告位于 Git 忽略目录：`.supercode/claude-smoke-report.json`、`.supercode/claude-ccswitch-smoke-report.json`、`.supercode/claude-ccswitch-approval-report.json`。集成测试使用独立 SQLite 数据库，不写入正常用户会话。

## CC Switch 导入

本机发现 `.cc-switch/cc-switch.db` 中的 Claude 和 Codex 供应商配置。导入使用固定查询和 SQLite 只读连接，不执行 SQL 导出文件。前端仅收到名称、模型、Agent 类型、是否含凭证和当前选择状态。

- 导入 Claude 连接环境变量、模型和凭证；不复制 hooks、权限绕过、插件或 MCP。
- 导入 Codex Responses 供应商的连接、模型和 API Key；API Key 通过子进程环境传递，不写入启动参数。官方配置使用本机登录态，不复制 OAuth 令牌。
- 可分别为 Claude / Codex 选择连接，也可切回“沿用本机 CLI 配置”。
- Windows 连接配置使用当前用户 DPAPI 加密保存到 SQLite，旧明文连接启动时迁移；界面不返回保存的密钥。
- 导入后不自动测试所有供应商，以免调用未选择的账户。当前真实验证覆盖 `kimi` 与 `OpenAI Official`。

CC Switch 源数据库、全局 Claude settings.json、Codex config.toml 和 auth.json 在导入测试前后的 SHA-256 比较均保持不变。

发布版桌面设置页实际扫描得到 20 项配置，已通过界面导入 `kimi` / `OpenAI Official` 两项，并选择 Claude 使用 `kimi`。数据库检查确认这两项配置及当前选择已持久化。修复了切换连接/Agent 后同一项目的 Git 面板没有重新刷新的问题。集成测试还检查导入配置的默认模型是否真正保存到新会话。最终发布版 Claude 运行报告保存在 `.supercode/claude-release-smoke-report.json`。

Codex 回归使用本机独立 CLI 0.160.1。通过 SuperCode 导入 CC Switch 当前 `OpenAI Official` 配置，随机验证码、流式输出、进程释放与会话恢复、工具读取 README 和停止任务均通过。原始报告为 `.supercode/codex-ccswitch-smoke-report.json`。

## 自动检查

- TypeScript / Vite 发布构建通过。
- Vitest 2 项测试通过。
- Rust 9 项测试通过，覆盖 UTF-8、JSONL、Claude 事件映射、持久化分页、项目路径边界和 CC Switch 导入过滤/只读查询。
- 所有项目文本文件通过 UTF-8 无 BOM 检查。
- Windows 桌面程序和 NSIS 安装包重新构建。

最终发布版已重新打开，Agent 选择为 Claude Code，同一项目的 Git 面板恢复显示 master 和变更列表。未启动 Agent 时一次本机样本：应用及 WebView2 工作集合计 671.1 MiB、私有工作集合计 315.8 MiB，主进程工作集约 79.6 MiB；不同界面状态会影响数值。原始报告为 `.supercode/claude-ready-memory-report.json`。当前可执行文件约 8.51 MiB，安装包约 2.41 MiB。

复现核心测试：

```powershell
$env:SUPERCODE_TEST_AGENT = 'claude'
$env:SUPERCODE_TEST_TOOLS = '1'
$env:SUPERCODE_TEST_CC_IMPORT = '1'
$env:SUPERCODE_TEST_APPROVAL = '1'
npm.cmd run test:integration
```

这些请求会使用所选连接的额度。测试提出真实审批请求后停止任务，不自动批准。实际点击“允许本次”后的写入、用户提问的完整往返、所有供应商、MCP 和 macOS/Linux 尚未完成真实覆盖。

协议参考：[Claude Code 程序化运行](https://code.claude.com/docs/en/headless)、[官方 Agent SDK 控制协议实现](https://github.com/anthropics/claude-agent-sdk-python/blob/main/src/claude_agent_sdk/_internal/query.py)、[CC Switch providers 数据库结构](https://github.com/farion1231/cc-switch/blob/main/src-tauri/src/database/schema.rs)。
