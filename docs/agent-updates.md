# Agent 安装与更新

Agent 设置页同时显示本机版本和官方发布的最新稳定版本。进入页面时分别查询本机安装与发布源，网络请求不会阻塞本机检测；可以手动点击“检查更新”。本机版本较旧时提供“更新”，未安装时提供“安装”。本机版本比最新稳定版更新时不提供降级按钮。

四个 Agent 使用各自维护者在 npm 的官方包：

| Agent | 发布源 |
| --- | --- |
| Codex | [@openai/codex](https://registry.npmjs.org/@openai/codex/latest) |
| Claude Code | [@anthropic-ai/claude-code](https://registry.npmjs.org/@anthropic-ai/claude-code/latest) |
| OpenCode | [opencode-ai](https://registry.npmjs.org/opencode-ai/latest) |
| Pi | [@earendil-works/pi-coding-agent](https://registry.npmjs.org/@earendil-works/pi-coding-agent/latest) |

成功查询在本次运行中缓存十五分钟，失败缓存三十秒；手动检查可刷新。断网时显示查询失败或上次检测的版本，本机 Agent 仍可使用与测试，不把未知状态当成“已是最新”。请求有大小和时间限制，不常驻轮询，也不会为检查版本启动模型任务。

安装固定到用户点击时显示的版本。下载使用软件自己的独立目录、Node 运行时、npm 配置和缓存；保留已有登录、本机全局安装与旧程序。先校验程序版本，再执行原生协议连接测试，全部通过后才原子切换选择。更新失败或取消时继续使用旧选择，原版本的安装记录和解析后的程序路径保存在新版目录的 `previous-installation.json`，不写入账号密钥。

运行任务期间不切换程序。安装、配置和任务启动共享操作门禁，数据库事务再次检查活动任务；上下文压缩和模型切换也遵守门禁。更新成功后释放旧的空闲连接，下一轮使用新程序。

## 2026-10-09 验证

隔离数据目录中真实下载并安装 Codex 0.162.0、Claude Code 2.1.295、OpenCode 1.18.35、Pi 1.1.0。四个程序均通过实际版本检查、原生协议握手、保留旧程序、失败更新不覆盖旧选择；Codex 另验证更新后旧的空闲 app-server 被替换。Pi 另从未安装状态测试普通“安装”入口。报告为 `.supercode/agent-updates-report.json`，只读正式数据，不自动升级正式 Agent 选择。

这里验证的是下载、安装和原生连接，未把握手称作真实模型回复。既有输入协议测试与版本边界见 [Agent 输入协议审计](agent-input-capabilities.md)；OpenCode 的原生引导仍只对已验证的 1.18.23 开启，其他版本使用停止后引导。

前端组件覆盖更新与安装参数、网络查询独立加载、断网重试、避免降级、活动任务禁用和更新失败后继续使用。Rust 测试覆盖官方包白名单、固定稳定版本及活动任务时的原子切换保护。
