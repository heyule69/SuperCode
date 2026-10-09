# 模型供应商的显示条件

模型供应商页只列出已保存的供应商连接和已确认登录的官方账号，不把底层 `@local` 路由包装成“本机 CLI 配置”供应商。没有配置的 OpenCode / Pi 分类显示 0 个连接。账号检查未完成时先显示已有 API 连接，不先补出官方账号。登录或刷新失败时隐藏未确认的账号，保留 API 连接和重试入口。

官方账号通过本机 CLI 的只读状态查询确认：Codex 使用独立短连接执行 `account/read`（`refreshToken: false`），Claude 使用 `auth status --json`。API Key 登录不作为 ChatGPT / Claude 官方订阅账号。检测不创建模型任务、不切换主任务连接，结果在本次程序运行中缓存六十秒，手动“刷新连接”重新检测。

在 Codex / Claude 的“添加连接”中，OpenAI / Anthropic 主入口分别显示为“ChatGPT 官方账号”和“Claude 官方账号”，未登录时直接调用对应 CLI 的浏览器登录，不打开 API Key、Base URL 或模型 ID 表单。已确认本机登录时直接使用该账号。完成登录并检查成功后，官方账号才进入供应商列表。需要 Key 的用户可点击“使用 API Key”，再选择明确命名的“OpenAI API / Anthropic API”。OpenCode / Pi 的 API 连接保留自己的引擎，不借用其他 Agent 的官方凭据。

模型菜单不再无条件补出 Claude Sonnet / Opus / Haiku；读取官方模型前先确认官方登录。账号不可用时返回空模型列表，并标记来源不可用，避免从保存的旧模型名称恢复出可选择项。

界面隐藏与数据库删除分开：已有连接配置、历史会话绑定和 CLI 路由保留。排序时把可见连接排在前面，保留隐藏路由，第一项成为新聊天默认；初始列表中不把被过滤后排到第一位的连接误标为默认。

## 2026-10-09 验证

187 项前端测试、156 项 Rust 测试和生产前端构建通过。组件测试覆盖空分类计数、已配置连接立即显示、官方登录确认、失败重试、默认标记、保留隐藏路由排序，以及添加页官方入口启动登录、复用本机账号、登录失败、API Key 独立入口和 OpenCode / Pi 引擎保留；模型选择测试覆盖移除凭空官方分组和拒绝恢复不可用目录。

真实 CLI 测试使用隔离软件数据库，确认本机 Codex 是 ChatGPT 登录，Claude 未登录；未登录 Claude 的官方模型目录为空，检测结束后主任务连接仍未启动。报告为 `.supercode/provider-accounts-report.json`。这是账号状态与目录验证，没有把它称为真实模型回复或 Windows 界面操作测试。

协议依据为 [Codex 官方账号协议](https://github.com/openai/codex/blob/main/codex-rs/app-server-protocol/src/protocol/v2/account.rs)；Claude 状态以本机 CLI 的 JSON 为准。
