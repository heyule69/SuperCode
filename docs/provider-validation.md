# 模型供应商、权限与命令验证

日期：2026-10-06，Windows x64。测试对象是 SuperCode 的实际接入路径。

| 验证内容 | 结果与证据 |
| --- | --- |
| 新建 Kimi 连接、保存并启用、真实调用 | 本机 Claude Code 2.1.287 返回结果；连接测试实际返回 `k3[1m]` |
| 模型名称 | 导入 / 新建连接菜单为 `k3[1M]`、`k3`，默认模型保存到会话，没有 Opus / Sonnet / Haiku 冒充 |
| Claude 会话 / 工具 | 中文流式回复、进程释放后恢复验证码、Read 读取 README、停止生成通过 |
| Claude 原生命令 | `/context` 有可显示结果；读取 CLI 的 `slash_commands` 清单通过 |
| 严格审批 | Write 请求进入 waiting，未经批准未写入；停止后仍未创建文件 |
| Codex 真实连接 | 项目内 CLI 0.160.1，通过 CC Switch 当前官方连接收到回复、恢复上下文、读取 README 和停止 |
| Codex goal | 实际 `thread/goal/set/get/clear` 设置、查询、暂停、恢复和清除通过；原生目标会自行启动任务 |
| 标准 Chat API | 真实 Claude CLI + 本地协议测试服务通过模型获取、模型 ID 保留、思考流、真实 Read 工具结果回传、会话恢复和中断 |
| 连接存储 | Windows DPAPI 新增 / 删除 / 选择 / 官方账号切换、旧明文连接迁移均有 Rust 测试 |
| 页面 | 按桌面窗口重新设计；固定导航与保存栏，连接 / 官方账号 / 添加 / 导入分别显示；760 × 700、1024 × 600 整体无溢出，见 [设置页](settings-design.md) |

原始报告保存在忽略目录：`.supercode/claude-provider-command-report.json`、`.supercode/codex-provider-goal-report.json`、`.supercode/claude-chat-protocol-report.json`。Chat 测试服务是协议 fixture，不是真实模型供应商；真实外部验证覆盖本机已有 Kimi 和 Codex 连接。

设置重做后的发布版 Codex 实测再次通过回复、工具读取、恢复、停止与 goal 操作，报告为 `.supercode/codex-final-release-report.json`。此次测试前后 Claude 设置、Codex 配置与登录文件的哈希一致。CC Switch 本身同时运行，源数据库整体哈希发生变化，未追溯写入进程；不能以整体哈希证明本轮导入只读。导入实现以 `SQLITE_OPEN_READ_ONLY` 打开源库，隔离 SQLite 测试校验读取前后的源文件字节一致。

## 接入方式

- 提供 23 个供应商 / 本地服务预设以及自定义兼容 API。预设地址与候选模型均可编辑，可手动添加模型或通过 API 获取。
- Claude 接收 Anthropic Messages；Chat Completions 经本地适配器进入 Claude 原生工具循环。Codex 需要 Responses。
- Coding Plan 的测试使用真正的 Claude CLI；不会伪造官方客户端标识。标准 API 发送简短测试请求，按供应商规则可能计费。
- API Key 只在用户填写时发送给 Rust，不保存在浏览器存储。Windows SQLite 配置字段采用当前用户 DPAPI 加密，保存的密钥不会重新发回界面。其他系统的系统钥匙串实现和打包验证仍待后续工作。
- CC Switch 是供应商页内的导入来源，可选择导入或一键导入全部，结果加入统一连接列表；不会改写源数据库或全局 CLI 配置。
- 导入的官方连接明确显示使用本机登录；编辑名称与模型保留官方鉴权方式，不改为 API Key 接入。此行为有 Rust 回归测试。

## 参考资料

预设和适配依据官方接口资料以及 CC Switch 的公开连接定义，未复制界面或推广链接。主要来源：

- [Kimi Code 文档](https://www.kimi.com/code/docs/)
- [MiniMax Anthropic 接口](https://platform.minimax.cn/docs/api-reference/text-anthropic-api)
- [Z.AI Claude Code 接入](https://docs.z.ai/devpack/tool/claude)
- [阿里云百炼 Base URL](https://help.aliyun.com/zh/model-studio/base-url)
- [Gemini OpenAI 兼容接口](https://ai.google.dev/gemini-api/docs/openai)
- [OpenRouter Claude Code 接入](https://openrouter.ai/docs/cookbook/coding-agents/claude-code-integration)
- [Ollama OpenAI 兼容接口](https://docs.ollama.com/api/openai-compatibility)
- [Claude SDK 原生命令发现与执行](https://code.claude.com/docs/en/agent-sdk/slash-commands)
- [CC Switch 开源项目](https://github.com/farion1231/cc-switch)

## 验证边界

预设不等于每家供应商已实测。模型权限、套餐 Key 和模型部署 ID 由对应账号决定；专有工具签名、非标准内容块、模型不支持工具调用等情况可能需要供应商专门适配。官方登录入口与本机账号状态接口已实现，新的网页授权全过程需用户本人完成，本轮没有代用户登录或自动点击审批。

OpenCode / Pi 完整执行、macOS / Linux 打包仍属后续阶段。当前同时运行一个任务；连接独立绑定到聊天，同一聊天切换供应商或模型前先压缩上下文，保留原始历史，失败保持原连接。最新界面与平台用量范围见 [Codex 对齐任务](codex-alignment-tasks.md)。没有宣称完整复制 Codex 桌面端全部功能。
