# SuperCode 架构与迭代计划

## 运行结构

React 通过 Tauri IPC 调用 Rust 核心。Rust 负责 SQLite、项目路径验证、Git 和 Codex 子进程。前端不能直接执行 shell。

Claude Code 使用本机原生 CLI 的双向 `stream-json` 协议，由 Rust 直接管理，无常驻 Node 桥接。每轮启动 CLI，通过原生 session ID 恢复对话，并在结束后释放进程。流式文本、工具结果和权限请求映射为前端共同的事件格式；停止操作等待输出读取任务完成状态落库后再返回。

CC Switch 导入以只读模式打开 `providers` 表，只保留 Claude 环境变量/模型和 Codex Responses 连接/模型。凭证保存在本机 `agent_profiles` 表，前端只接收名称、模型、是否含凭证和启用状态。选中的配置仅作用于 SuperCode 子进程，不改写全局文件；不复制审批绕过、hooks、MCP 或插件。Codex API Key 通过子进程环境传递，避免出现在启动参数中。

供应商连接统一使用该配置仓库：手动配置与 CC Switch 导入结果可在同一页面管理。模型 ID 不改成 Claude 别名；旧配置仅在读取时解析已有别名对应的真实模型。Windows 配置字段使用 DPAPI 保护，启动迁移旧 JSON，密钥不经过前端读取接口。

Chat Completions 接入通过 Rust / axum / reqwest 本地适配器转换 Anthropic 请求和流事件。每轮绑定随机 loopback 端口与短期令牌，转发模型 ID、工具历史和思考内容，停止时取消请求并关闭监听；没有常驻 Node 转发服务。Codex Responses 保持原生协议。

官方登录由安装的 CLI 管理：Claude `auth login` / `auth status`，Codex app-server `account/login/start` / `account/read`。SuperCode 只打开官方浏览器入口，不接收 OAuth 令牌。Claude 从 `system/init.slash_commands` 发现可用命令。Codex goal 使用 `thread/goal/*` 原生调度，设置目标时不会额外重复发送用户任务；运行中允许查询、暂停和清除，停止操作先暂停活动目标。目标创建 / 恢复遵守界面选定的沙箱和审批规则。

Codex 使用官方 `app-server --listen stdio://`。初始化后通过数字 ID 关联请求与响应，审批与提问属于 server-initiated requests，由前端显示并将用户回答传回。未知请求显式返回“不支持”，不默认批准。

一个应用运行时共享一个 Codex app-server。0.1.0 同时只允许一个运行任务，其余会话保留索引和历史。会话的原生 thread ID 存储在 SQLite 中，释放运行时后通过 `thread/resume` 恢复上下文。

会话状态为 `idle`、`starting`、`running`、`waiting`、`failed`、`interrupted`。重启应用时将未完成任务标记为 interrupted，避免历史记录误显示仍在运行。进程世代编号用于避免旧进程退出事件影响新进程。

## 内存控制

- 应用启动不拉起 agent，读取模型列表或发送消息才启动。
- 默认不启动 Codex 全局配置中的 MCP 服务，设置中可启用；应用不会改写原始配置。
- 非运行状态下空闲 300 秒释放进程；审批中的会话不会被空闲策略中断。
- Windows Job Object 包含 agent 和其工具进程，释放后清理进程树。
- 前端每 50 ms 合并文本流更新；历史消息每页 100 条，工具输出默认折叠。
- Markdown 和审批界面按需加载，未引入完整 IDE、Monaco 或 Office 预览库。
- 单条协议事件最多 4 MB，持久化文本最多 128 KB，Git 输出读取最多 2 MB，diff 展示最多 256 KB，文件预览最多 1 MB。
- 日志界面最多保留 200 条，每条最多 4 KB。

资源面板按进程父子关系分类工作集内存。发布版的指标应单独测试，不能使用开发版和开发服务器占用代表最终结果。

## 下一阶段

1. Claude Code：补充模型发现、更多工具展示和不同 CLI 版本的兼容测试。
2. OpenCode：按需启动 server，通过 HTTP 和 SSE 归一化为统一事件。
3. Pi Coding Agent：接入 `--mode rpc`，按 LF 分割 JSONL，不将 Unicode 分隔符当成消息边界。
4. 将当前 Codex 事件映射抽象成 AgentAdapter，声明模型、停止、恢复、审批等能力。保留底层原始事件以便诊断协议变化。
5. Git worktree、受控并行和更完整的快捷键，再扩展 macOS/Linux。

跨 agent 交接创建新原生会话并传递摘要与文件引用；不假定不同 agent 的工具状态和上下文可以无损互换。
