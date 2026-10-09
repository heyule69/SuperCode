# 自动化工具：一键安装、测试与 Agent 接入

设置 → 自动化工具。浏览器、电脑自动化各有“安装并测试”按钮。安装成功且实际测试通过后，写入 SuperCode MCP 配置并启用。后续可停用、重新测试、修复安装。重新测试保留已有启用选择。

## 安装与运行

- 浏览器：私有 Node.js 22.23.3、`@playwright/mcp` 0.0.83，以及对应的 Chromium。使用独立浏览器会话。
- 电脑：私有 uv 0.12.23、受管理 Python 3.13、私有虚拟环境和 `windows-mcp` 0.7.5。直接运行该环境的服务程序。
- 不依赖用户提前安装 Node、npm、uv 或 Python；不修改系统 PATH，不设置开机服务，不写原生 Agent 配置。
- 便携运行环境来自官方发布，下载后检查固定 SHA-256。npm/PyPI 安装使用官方仓库和固定 MCP 版本。
- 安装与测试串行进行，支持取消，输出缓存限量。中途失败不覆盖原有有效安装；已完成下载的待测试环境可复用。安装记录与 MCP 配置通过同一 SQLite 事务提交。
- 当前安装器支持 Windows x64 / ARM64。ARM64 的下载地址与校验值已覆盖，实际功能测试在 x64 上完成。

## 测试内容

浏览器通过 MCP 初始化、读取工具列表，然后在无头 Chromium 中打开本机临时网页。通过实际可访问性引用点击按钮，读取页面确认标识发生变化。临时 HTTP 服务只绑定回环地址。电脑通过 MCP 获取工具列表、执行只读 Snapshot，并验证桌面窗口状态。电脑测试不会执行点击、输入、滚动；这些操作工具的可用性由工具列表确认。

测试结束后释放进程及其子进程。安装完成的工具随 Agent 任务按需启动。重新测试与安装期间禁止修改配置，提交前再次核对是否存在运行中的任务。

## 实际验证

2026-10-08：Windows x64 实际安装成功；浏览器返回 25 个工具并完成页面导航、点击和读取；电脑返回 18 个工具并完成桌面状态读取。额外启动隔离 Codex app-server 和 Claude CLI，均确认连接到两个服务，分别列出 25 / 18 个工具。只查询工具连接，没有发送模型对话。

最终发布程序通过同一集成验证，NSIS 安装包 4.05 MiB。软件内两项“重新测试”按钮实际点击通过；电脑工具停用后重新测试仍保持停用，随后恢复启用。测试结束后确认没有残留自动化进程。前端 111 项、Rust 100 项测试通过；168 个项目文本文件均为 UTF-8 无 BOM。桌面实拍保存在 `.supercode/automation-desktop.png`。

验证入口：`src-tauri/target/release/supercode.exe --automation-smoke-test`。该入口明确执行安装/测试，使用当前软件数据目录；请在任务空闲时运行。结果写入 `.supercode/automation-smoke-report.json`。普通启动不会执行验证。

## 官方资料与版本依据

- [Playwright MCP](https://github.com/microsoft/playwright-mcp)；[浏览器安装与独立缓存目录](https://playwright.dev/docs/browsers)。
- [Windows MCP](https://github.com/CursorTouch/Windows-MCP)；[0.7.5 包元数据](https://pypi.org/pypi/windows-mcp/0.7.5/json)。
- [Node.js 22.23.3 校验值](https://nodejs.org/dist/v22.23.3/SHASUMS256.txt)。
- [uv 0.12.23 发布文件](https://github.com/astral-sh/uv/releases/tag/0.12.23)；[受管理 Python](https://docs.astral.sh/uv/concepts/python-versions/)。
- [Claude SDK 的 MCP 状态控制请求](https://github.com/anthropics/claude-agent-sdk-python/blob/main/src/claude_agent_sdk/_internal/query.py)。
