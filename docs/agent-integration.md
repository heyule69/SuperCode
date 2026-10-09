# 本机 Agent 与独立安装

设置 → Agent 默认显示 Codex、Claude Code、OpenCode、Pi 的安装状态、版本和测试入口。CLI 路径与 Codex 全局 MCP 选项收在“高级选项”，主页面不再显示长路径表单。

启动只做有界的本机路径检查，不在后台下载软件或启动模型任务。查找 PATH、常用 npm / pnpm / Bun / 原生安装目录，优先使用本机版本。手动指定程序优先于自动检测；找不到本机程序时使用 SuperCode 的独立安装记录。

旧版 SuperCode 明确选择过的 Codex 路径仍被保留，避免升级后改用 PATH 上的旧版本。“恢复自动”会忽略旧版手动选择。

“安装并测试”使用官方 npm 包和固定版本：

| Agent | 包 | 当前安装版本 | 执行协议 |
| --- | --- | --- | --- |
| Codex | `@openai/codex` | 0.161.0 | app-server JSON-RPC |
| Claude Code | `@anthropic-ai/claude-code` | 2.1.293 | 双向 stream-json |
| OpenCode | `opencode-ai` | 1.18.35 | ACP JSON-RPC |
| Pi | `@earendil-works/pi-coding-agent` | 1.0.4 | RPC JSONL |

旧的 `@mariozechner/pi-coding-agent` 安装可被发现。低于 1.0 的版本不具备本次使用的原生 MCP 接口，连接测试会提示修复安装。

## 安装与恢复

每次安装创建独立版本目录，使用随安装准备的 Node 22.23.3、官方 registry 和 npm 包完整性检查。下载、解压、安装和连接测试支持取消。Windows 子进程没有额外终端窗口。

安装不修改全局 npm、PATH、原有 CLI 配置或账号文件。版本检查和协议连接测试成功后，才在一个 SQLite 事务中切换安装记录。失败或取消保留原选择；修复安装保留原版本，并在 `previous-installation.json` 中记录旧程序路径及本次安装版本。高级选项可重新选择旧程序，或恢复自动检测。

npm 包中的 `.cmd` 包装脚本和 Claude 的文本 `.exe` 占位文件不会直接启动。Windows 原生候选程序检查 PE 文件头及架构；Pi 用原生 Node 程序加独立脚本参数启动。提示词和外部输入经 JSON stdin 传输，不拼成 shell 命令。

## OpenCode 与 Pi

两个引擎均接入真实流式文本、公开思考摘要、工具输入与结果、文件变更、会话恢复、停止、原生命令、模型连接和上下文压缩。配置的 API 可使用 Anthropic、Chat Completions 或 Responses 协议；未选 SuperCode API 连接时沿用本机模型与登录配置。

SuperCode 注入临时进程配置：OpenCode 使用 `OPENCODE_CONFIG_CONTENT`，Pi 使用私有运行扩展注册供应商和 MCP。不会覆盖用户已有配置。原生插件、扩展、技能仍由对应 CLI 管理。

OpenCode 的权限请求、Pi 的确认与选择 / 输入问题传回用户界面。只发送用户明确选择的回复；OpenCode 的允许操作仅选择 `allow_once`，不自动授予永久权限。停止任务会清除待处理请求和子进程。

同一连接内选择模型保留原生会话；切换连接按现有上下文长度选择携带历史或生成摘要。`/compact` 使用当前引擎另起受限会话生成摘要，失败保留原历史。摘要过程不执行工具或批准请求。

Anthropic 接口实际请求会去掉 Claude CLI 的 `[1M]` / `[256K]` 注记，列表保留用户配置的名称。OpenCode 的 AI SDK base URL 按其 `/messages` 规则处理。

Pi Token 用量累加每轮实际助手消息；自定义 API 没有可靠价格时不显示费用。OpenCode ACP 的上下文占用不当成计费用量，未返回的数据不补成零。平台额度仍由独立额度页面的供应商查询决定。

本机 Pi 模型列表读取真实默认模型和当前模型支持的推理强度。供应商自填模型未返回能力信息时不虚构推理选项。交互终端专用命令不保证能在桌面 RPC 中执行。

## 验证

常规检查：`npm run build`、`npm test`、`cargo test --manifest-path src-tauri/Cargo.toml`、`npm run check:encoding`。

显式开发测试入口创建独立数据库和带空格 / 中文的测试路径，仅只读复制本机连接供测试使用，测试报告不含密钥：

- `supercode.exe --agents-smoke-test`：OpenCode、Pi 的真实模型回复、跨进程续聊、读取文件、转交写入审批、未批准时停止和摘要压缩。
- 加 `--install-all-agents`：四个官方包的隔离安装与真实协议连接检查。
- `supercode.exe --agents-mcp-test`：两个新引擎经已有 Playwright MCP 访问本地测试网页，验证模型实际取得页面标识；需要先安装并启用浏览器自动化。

这些入口不会把浏览器预览或模拟事件当成真实 Agent 运行。报告写入工作区的 `.supercode/agents-smoke-report.json`。

2026-10-08 的 Windows 实测通过四个固定版本的隔离安装与连接，OpenCode / Pi 的真实回复、续聊、读文件、审批转交、停止、摘要压缩，以及两个引擎通过 Playwright MCP 打开本地网页。完整运行报告和 MCP 报告分别另存为 `.supercode/agents-full-smoke-report.json`、`.supercode/agents-browser-mcp-report.json`。不把这次使用的 Kimi 连接测试扩展成所有供应商均已逐一验证。

## 官方来源

- [OpenCode ACP](https://opencode.ai/docs/acp/) 与 [配置](https://opencode.ai/docs/config/)。
- [Pi RPC](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/docs/rpc.md)、[扩展](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/docs/extensions.md)、[MCP](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/docs/mcp.md)。
- OpenCode 本地图标取自 [opencode.ai/favicon.svg](https://opencode.ai/favicon.svg)，Pi 图标取自 [pi.dev/logo-auto.svg](https://pi.dev/logo-auto.svg)，均保存到 `public/brands/`，运行时无需远程加载。
