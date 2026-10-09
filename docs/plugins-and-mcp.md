# 插件与 MCP

原页面只显示 SuperCode 手动保存的 MCP，插件列表需要手动读取，而且调用了当前本机 Codex CLI 不支持的 `plugin/list`。现在进入页面即读取本机插件和连接，不启动 Agent、不连接 MCP，也不自动下载插件。

## 页面

- 插件、MCP 两个标签，显示真实条目数量。
- 按 Codex / Claude Code 筛选，名称与描述搜索。
- 已添加插件展示名称、描述、Agent、组件、启用状态及本机图标；详情提供来源、版本和文件夹。
- 浏览目录展示本机缓存中尚未添加的插件，支持添加到 SuperCode；另提供官方目录链接。
- 导入包含 `plugin.json` 的插件文件夹，使用原文件，不复制或覆盖本机配置。
- MCP 展示所属 Agent、来源、传输与启用状态；支持自定义 stdio / HTTP 服务及现有浏览器、电脑预设。
- 插件关闭后，其 MCP 也关闭。开关保存在 SuperCode 数据库，下一次任务生效，任务运行期间不能修改。

## 实际接入

Codex 读取用户 `config.toml` 的插件与 MCP、插件缓存，以及当前项目的插件设置。Claude Code 读取安装登记、用户和项目设置、用户 / 项目 / 本地范围的 MCP；支持 `CODEX_HOME` 和 `CLAUDE_CONFIG_DIR`。

Codex 启动时生成单独的启用状态覆盖参数；本机配置里的 transport 与认证配置仍由原生 CLI 读取。当前项目的插件选择也参与覆盖，切换项目时不会复用其他项目的插件进程环境。注意该版本 CLI 的覆盖参数按点号拆分键路径，给服务名添加 TOML 引号会使引号变成名字的一部分，已用原生 CLI 连接测试验证并修正。

Claude Code 保留 strict MCP 模式，将启用的原生及插件服务显式传入；插件开关通过本次进程的设置传入，导入的插件通过 `--plugin-dir` 加载。插件路径变量会展开，自动执行的 Hooks 保持关闭，不从导入插件复制工具自动批准策略。技能列表同步插件开关，并加入启用的导入插件技能。

目录、manifest、图片及 MCP 文件读取有限额；插件内文件须留在插件根目录内。无效 UTF-8、BOM、JSON、TOML 明确显示读取提示，不转换源文件编码。MCP 的完整配置、环境变量和认证头不发送到前端，HTTP 地址只显示 origin。

依赖 Codex 桌面内部管道或 SDK 宿主的 MCP 明确显示不可用；仅有 Hooks 的插件也标注不可用。ChatGPT 云端应用没有可移植的本机服务器配置时，不把它们伪造为可运行的 SuperCode 插件。参考 [OpenAI 插件格式](https://developers.openai.com/plugins/build/plugins)、[Claude Code 插件](https://code.claude.com/docs/en/plugins-reference) 与 [Claude Code MCP](https://code.claude.com/docs/en/mcp)。

## 验证

- 构建、108 项前端测试、94 项 Rust 测试通过。
- 新测试覆盖真实清单解析、版本排序、逐个启用、父插件关闭联动、配置与密钥不泄漏、目录越界、编码错误、启动参数与审批策略边界。
- `.supercode/verify-extension-mcp.mjs` 使用隔离的临时 Codex 配置，与真实本机 Codex CLI 和 stdio MCP 测试进程通信，读取到一个工具。没有发送模型对话，没有修改用户的 Codex 配置。

发布版已构建并实际打开，自动读取到本机 14 个完整插件、14 条 MCP 配置；不完整的 rust-analyzer-lsp 缓存显示读取提示。真实桌面验证了插件搜索、Agent 筛选、标签切换、列表读取及官方客户端专属连接的限制标记，未修改用户的插件开关或认证配置。截图保存为 `.supercode/plugins-desktop.jpg` 与 `.supercode/mcp-desktop.jpg`。蓝色开关与常规设置页面保持一致。
