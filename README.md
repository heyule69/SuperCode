# SuperCode

一个轻量、本地优先的多 Agent 编程桌面工作台。Windows 优先，采用 Tauri 2、Rust、React 和 SQLite。

## 当前版本：0.1.0

- 项目侧栏、历史会话、中文聊天、五种外观选项、搜索、重命名和归档。
- **真实 Codex 接入**：`app-server` stdio JSON-RPC、模型列表、流式输出、会话恢复、停止任务。
- **真实 Claude Code 接入**：本机 CLI 的双向 JSONL、中文流式输出、原生会话恢复、工具记录、审批/提问与停止任务。每轮结束后立即释放 Claude 进程。
- 从 CC Switch 的 SQLite 数据库读取并选择导入 Claude / Codex 的连接、模型和凭证；在设置中切换要使用的连接配置。
- 在“模型供应商”中直接新增、编辑、删除和切换 API 连接；23 个供应商 / 本地服务预设与自定义接入，Coding Plan 与标准 API 分开。
- 模型菜单显示真实模型 ID，Kimi 等第三方模型不会统一显示成 Opus。旧导入配置的 CLI 别名会解析到实际供应商模型。
- 官方 Claude / ChatGPT 账号的状态检查、浏览器登录入口和本机账号选择；授权由用户本人完成。
- 权限可切换默认权限、严格确认、只读和完全访问，使用 Agent 原生权限模式；Codex 命令请求可按原生能力允许本次或当前会话。
- `/` 命令菜单：界面操作、Claude 原生命令发现 / 执行，以及 Codex 原生 goal 与上下文压缩。
- 展示 Agent 实际返回的思考摘要、计划、工具参数、命令流式输出、文件差异、MCP 进度、审批与用户提问。运行、完成、失败和停止状态分别显示，执行过程默认展开并保存到历史。
- 输入框自动增高，各会话草稿独立保存，重启后恢复上次会话；支持命令键盘补全、回到最新消息、项目折叠、代码复制和 Ctrl+N / Ctrl+K / Ctrl+, / Ctrl+B 快捷键。
- 输入框“＋”添加文件、图片、目录和技能；支持 Ctrl+V 粘贴图片、缩略图、大图预览与独立图片草稿；原图传给 Agent，UTF-8 文件内容随消息发送。见 [图片输入](docs/image-input.md)。
- 每轮文件变更汇总与差异行号；Claude 的 Edit / Write 记录限定在项目内的文本文件，Codex 使用原生 turn diff。
- 真实 Token、缓存、费用和历史用量；上下文占用与原生压缩入口。API 连接和官方账号额度分别处理。
- 白色、暗黑、石墨、暖色和跟随系统；界面 / 代码字体、字号、减少动画。
- 任务完成 / 失败 / 等待确认通知、提醒时机、名称隐私、系统提示音与点击返回；默认 Agent / 权限、发送方式和自定义指令。通知设置及验证见 [系统通知](docs/system-notifications.md)。语音不在实现范围。
- 本机技能发现、创建、查看与添加到对话；Codex 原生插件列表、安装 / 移除及应用独立的 MCP 配置。
- 浏览器 Playwright MCP、电脑 Windows MCP 预设；默认关闭，启用后随 Agent 按需启动，可编辑命令和参数。
- Git 变更和 diff、项目内 UTF-8 文本文件预览。
- 本地 SQLite 持久化，每次读取最多 100 条历史消息。
- Agent 按需启动，空闲 5 分钟后释放，可手动立即释放。
- 可指定独立的 Codex CLI 路径；默认关闭全局配置的 MCP 服务，需要时在设置中开启。
- Windows Job Object 在应用退出或释放运行时后清理 agent 进程树。
- 单实例运行，重复启动会聚焦已有窗口，避免重复后台进程。
- 点击关闭默认隐藏到系统托盘，保留窗口和任务；托盘右键可恢复正在运行的窗口、新建窗口或退出。“文件”菜单也提供新窗口和退出。见 [桌面窗口行为](docs/desktop-lifecycle.md)。
- 资源面板分别显示应用/WebView 和 agent/工具进程的工作集内存。

OpenCode、Pi 已通过原生协议接入真实聊天、工具执行、审批、停止和恢复会话。四种 Agent 均可在“设置 → Agent”中自动检测、独立安装和测试，程序路径收在高级选项中。接入范围与验证见 [Agent 接入说明](docs/agent-integration.md)。

## 快速开始

需要 Node.js 22+、Rust stable、Windows C++ Build Tools 和 WebView2。此项目开发时使用 Node.js；发布后的桌面应用使用 Rust 和系统 WebView2，应用本身无需常驻 Node 服务。

```powershell
npm install
npm run desktop
```

使用前安装并登录 Codex CLI：

```powershell
npm install -g @openai/codex
codex login
```

应用会自动查找本机安装。没有 Agent 时，在“设置 → Agent”点击“安装并测试”；旧版本不兼容时可在高级选项中修复安装。下载到独立版本目录，不改全局安装、已有登录或配置。只有需要手动指定程序时才展开高级选项。

在桌面版点击“添加项目”，输入已有代码文件夹的完整路径，或点击“选择文件夹”。添加后输入消息即可。新会话默认选择 Codex 模型列表中标记为默认的模型；后续沿用该会话的模型。选择“加载可用模型…”可手动切换。模型列表不代表账号一定有权限使用对应模型。

Windows 中如果 PowerShell 禁止执行 npm.ps1，可以使用 `npm.cmd` 和 `codex.cmd`。

使用 Claude 时，需要先安装本机 Claude Code，在 SuperCode 中选择官方账号或配置 API。输入框下方分别选择 Agent、模型连接、真实模型 ID 和审批规则。只读模式仅提供 Read / Glob / Grep 文件工具，工作区编辑模式的权限请求由用户确认。

OpenCode、Pi 也可从输入框选择。默认读取本机模型配置；也可以在模型供应商页面的对应分类中添加 API 连接。SuperCode 管理的 MCP 服务会传给两个引擎，原生 CLI 的技能和扩展继续由各自管理。

### 从 CC Switch 导入

打开左下角“设置 → 模型供应商 → CC Switch 导入”。可以导入全部，也可以搜索、勾选并导入所选连接。默认查找用户目录的 `.cc-switch/cc-switch.db`，也可输入数据库文件或目录路径。导入结果加入统一的连接列表，可继续编辑、测试或“保存并使用”。导入本身不自动更换当前连接；再次导入会更新对应连接。

导入只复制连接、模型和凭证，不复制 hooks、权限绕过、插件或 MCP。源数据库及全局 Claude / Codex 配置不被改写。Windows 下连接配置使用当前用户 DPAPI 加密后存入 SQLite，旧明文连接启动时迁移；保存的密钥不返回前端。选择“本机配置”可沿用 CLI 原有设置。Codex 当前支持 Responses 类型供应商；官方登录沿用本机登录态，不复制 CC Switch 中的 OAuth 令牌。

### 直接接入 API 与命令

供应商页的“添加连接”提供 Kimi、OpenAI、Anthropic、MiniMax、GLM、百炼、豆包、混元、文心、DeepSeek、硅基流动、OpenRouter、Gemini、Grok 等预设，以及 Ollama / LM Studio 和自定义服务。预设是可编辑模板，不保证每个账号都能使用列出的候选模型；可从 API 获取模型或手动填写真实 ID。不同套餐需要对应的 Key。

Claude Code 使用 Anthropic 原生接口；标准 Chat Completions API 经按需启动的 Rust 本地适配器处理，工具仍由 Claude CLI 执行。Codex 使用 Responses API。模型需要支持工具调用；供应商专有内容块、工具签名等能力仍可能需要专门适配。

输入 `/` 查看当前引擎可用命令。Claude 启动后读取 CLI 公开的命令清单，通过原生消息执行 `/compact`、`/context` 等；只适用于交互终端的命令不列入。Codex 支持 `/goal 目标内容`、`/goal status`、`/goal pause`、`/goal resume`、`/goal clear`、`/goal-status`、`/compact`。目标由 Codex 原生调度，SuperCode 展示目标状态与 Token 用量；停止任务会先暂停活动目标。权限规则遵循底层 Agent，并非每个只读操作都会出现确认。

最新的 Codex 样式、对话刻度、设置返回、官方 Agent 图标和平台额度范围见 [27 项对齐清单](docs/codex-alignment-tasks.md)。客户端其它功能见 [客户端功能清单](docs/client-parity.md)，接入验证见 [供应商与命令验证](docs/provider-validation.md)。

## 执行过程

工具卡片显示实际工具名称、文件路径或命令，展开可查看输入、结果、退出码和耗时。思考区只展示 Agent 主动公开的内容：不支持摘要的模型仅显示运行状态，不生成虚构思考。Claude 工具输出通常在工具结束时返回；Codex 命令输出可逐段显示。设置的“通用”可关闭默认展开。

切换到其他会话不会停止当前任务。输入区提供返回运行中会话的入口；返回后会恢复尚未完成的摘要和工具输出。停止后未完成的工具会标记“已停止”，不会显示成功标记。最近 20 份草稿和附件路径保存在本机 WebView 存储中，每份文字最多 32000 个字符、附件最多 12 项，重启后可继续编辑。

文件预览显示行号；消息中的项目文件链接可定位到指定行，外部 HTTP / HTTPS / 邮件链接交给系统应用打开。工作区和每轮完成卡片可逐个查看文件差异，包括删除的文件。长代码、表格和日志在各自区域内滚动。

## 浏览器预览与桌面运行

```powershell
npm run dev
```

访问 `http://127.0.0.1:1420`。浏览器预览用于检查界面，项目和会话元数据保存在浏览器中，**不会运行本地 agent 或读取本地文件**。真实功能请使用桌面版。

## 验证与构建

```powershell
npm run build
npm test
npm run test:rust
npm run desktop:build
```

Windows 产物位于 `src-tauri/target/release/`，安装包位于 `src-tauri/target/release/bundle/nsis/`。

本次测试与内存样本见 [验证记录](docs/verification.md)。编码检查使用 `npm run check:encoding`。

可选真实集成测试：

```powershell
npm run test:integration
```

此命令通过 SuperCode 自身的命令、存储和运行时调用真实本机 Agent，会使用账号额度。测试使用 `.supercode/smoke/` 中的独立数据库，检查真实流式输出、释放进程后的上下文恢复和停止任务。结果保存为 `.supercode/native-smoke-report.json`。

设置 `SUPERCODE_CODEX_PATH` 可指定测试用 CLI；设置 `SUPERCODE_TEST_MODEL` 可选择测试模型。额外设置 `SUPERCODE_TEST_TOOLS=1` 会测试通过内置工具读取本项目 README，保持只读且不自动处理审批。

设置 `SUPERCODE_TEST_AGENT=claude` 先验证 Claude Code；`SUPERCODE_TEST_CC_IMPORT=1` 会在测试数据库中导入 CC Switch 当前对应 Agent 的配置并验证运行。`SUPERCODE_TEST_APPROVAL=1` 额外验证 Claude 写入请求进入等待审批状态，再停止任务；不自动批准，也不创建请求中的文件。详细结果见 [Claude 与配置导入验证](docs/claude-validation.md)。

### 客户端功能集成验证

额外设置 `SUPERCODE_TEST_CLIENT=1`，使用隔离测试目录验证附件、技能、完全访问下的文件修改、每轮差异、实际用量和原生上下文压缩；不会修改用户项目文件。Codex 测试再设置 `SUPERCODE_TEST_MCP=1`，会启用测试库中的两个自动化连接并检查实际工具注册。它不执行桌面自动化动作，也不批准外部请求。每个 Agent 的报告为 `.supercode/client-<agent>-report.json`。

## 数据与权限

- 桌面数据保存在系统应用数据目录下的 `dev.supercode.desktop/supercode.db`，具体路径由 Tauri 按操作系统解析。
- 登录和模型配置交由已安装的 Codex CLI 管理。SuperCode 不复制账号密钥到自己的数据库。
- 如果本机 Codex 配置仍使用旧档位名称 `service_tier="priority"`，SuperCode 仅对自己的子进程映射成当前 CLI 的 `fast`，不改写全局配置文件。
- 为减少内存，默认对全局 `mcp_servers` 中的服务传递 `enabled=false` 子进程参数。设置中启用后沿用原配置。项目目录中额外定义的 MCP 配置以及插件仍可能引入其他进程。
- 只读模式使用 Codex 的 `readOnly` sandbox；工作区编辑使用 `workspaceWrite`。审批策略为 `on-request`，由 Codex 产生审批请求并显示到界面。
- 项目文件预览会验证解析后的文件仍位于所选项目内，不支持项目外路径或跳出项目的符号链接。
- 所有项目文本使用 UTF-8 无 BOM。预览遇到 BOM、非 UTF-8 或二进制文件时会提示，不自动转换文件编码。
- 工作集内存统计会包含进程的共享内存，不能视为系统新增的独占内存。当前不承诺固定内存数值，以实际机器测量为准。

## 项目结构

```text
src/                       React 界面、事件合并与浏览器预览
src-tauri/src/commands.rs   桌面 IPC、项目、Git 与文件预览
src-tauri/src/runtime.rs    Codex 协议、进程生命周期与空闲释放
src-tauri/src/process.rs    程序发现、Windows 隐藏子进程与 Job Object
src-tauri/src/storage.rs    SQLite 与分页历史
src-tauri/src/protocol.rs   事件归一化、输出限制
src-tauri/src/smoke.rs      显式运行的真实集成测试
docs/                      架构与后续计划
```

## 开源参考

架构参考 [Codexia](https://github.com/milisp/codexia) 的 Tauri/Rust 桌面及 Codex app-server 接入方向；交互参考 [AionUI](https://github.com/iOfficeAI/AionUi)、[Opcode](https://github.com/winfunc/opcode)、[CloudCLI](https://github.com/siteboon/claudecodeui)。当前实现为本项目编写，未直接复制这些项目的源码。

协议以 [Codex 官方 app-server 文档](https://developers.openai.com/codex/app-server) 与本机 CLI 生成的 JSON Schema 为准。协议兼容性检查使用 `0.154.0`，真实对话验证使用 `0.160.1`。新版模型目录变更见 [官方更新记录](https://learn.chatgpt.com/docs/changelog)；协议仍可能随 CLI 版本变化。

项目尚未指定整体发布许可证；依赖包遵循各自许可证。对外发布前应确定项目许可证并生成依赖声明。
