<img src="public/app-icon.png" width="72" alt="SuperCode">

# SuperCode

聊天与设置可在“设置 → 常规 → 数据与空间”保存、恢复备份，并查看空间占用、清理旧 Agent 安装。恢复在重新打开应用后生效，自动保留恢复前的数据库；加密凭据仅可在原电脑的 Windows 用户下恢复。项目文件、附件原文件和 Agent 程序需单独保管。Agent 设置显示当前兼容资料覆盖，并可回滚已有的上一安装版本。

开发验证包括 `npm run check:encoding`、`npm run check:catalogs`、`npm run build`、`npm test`、`npm run test:rust`、`npm run installer:test` 和 `npm run release:test`。性能基准和现有进程的测量方法见 [PERFORMANCE.md](PERFORMANCE.md)。

Windows CI 还运行 `npm audit`，并用固定版本 cargo-audit 检查主程序与安装器的两份 Cargo.lock。已发布漏洞会使验证失败；停止维护、内存安全等 informational 提示保留在日志中，不添加忽略公告列表。

把 Codex、Claude Code、OpenCode、Pi 和 Gemini CLI 放进一个桌面工作台，管理项目、模型连接和工具，直接用对话完成开发任务。

当前版本 **0.2.4**，主要支持 **Windows**。基于 Tauri 2、Rust、React + TypeScript 和 SQLite。

[下载 Windows 安装包](https://github.com/heyule69/SuperCode/releases/latest) · [版本记录](https://github.com/heyule69/SuperCode/releases)

![SuperCode 工作台](.github/assets/workbench.jpg)

## 功能

- **Agent 管理**：自动发现本机 Agent，显示已安装和最新版本，支持一键安装、更新与连接测试。
- **模型连接**：Codex、Claude 官方多账号登录，按 Agent 配置 API 协议，支持从 CC Switch 导入连接。OpenCode / Pi 可更改已有连接的协议；自定义连接可在保存前获取模型，列表支持单项或多选删除。任务运行时可新增连接；主任务或子任务正在使用的连接暂不能编辑或删除。
- **对话工作台**：流式回复、工具记录、文件差异、历史会话和侧边聊天；任务运行中可以追加消息、排队或引导。
- **计划与子代理**：显示原生计划文档并选择执行或继续修改；子代理有独立图标和详情，完整任务与步骤摘要持续可见，具体工具输出按需展开。能力以所选引擎支持为准。
- **图片与媒体**：上传、粘贴图片，在对话中查看图片、播放视频和音频。
- **技能、插件与 MCP**：发现本机技能，管理 Codex / Claude 插件和 MCP；内置浏览器、电脑操作，支持连接测试和随时接管。
- **内置浏览器与电脑操作**：在右侧查看网页操作过程，浏览器与文件变更独立显示；电脑操作带屏幕光效和 AI 指针，按 Esc 或直接操作即可接管。接管只撤销电脑控制，聊天任务可以继续；聊天的停止按钮另行停止任务。
- **桌面体验**：深浅主题、系统通知、托盘和多窗口。聊天保存在本机，Agent 按需启动，空闲后释放。
- **上下文管理**：按模型、接口与引擎提供的实际限额显示上下文，支持引擎原生自动压缩。
- **软件更新**：启动约 10 秒后检查 GitHub Releases，之后每 6 小时检查；恢复窗口焦点时补查到期更新。新版本弹窗提醒，同次启动同版本只提醒一次；可稍后处理，任务完成后重启安装，保留聊天与配置。

Codex、Claude Code、OpenCode、Pi、Gemini CLI 均使用独立的聊天运行通道，同一种 Agent 可开启多个会话，没有应用设置的 Agent 数量或任务并发上限。各聊天独立处理停止、审批和输出；向同一个活动聊天追加消息时，按原生能力引导或按该聊天的顺序发送。Gemini CLI 本轮以 0.63.0 官方源码和受控 ACP 测试验证，未进行真实模型运行验证。

## 开始使用

先下载安装包，选择安装目录。之后可直接在软件中更新。

1. 在「设置 → Agent」检测已有安装，缺少时点击「安装」。
2. 在「模型供应商」选择官方账号登录，或添加 API 连接。
3. 选择 Agent 和模型，输入任务即可。项目可选；未选择时，聊天文件保存在文档目录的 `SuperCode` 文件夹中。

Codex、Claude Code、OpenCode、Pi 的已有登录与配置按各引擎能力接入。Gemini CLI 需在模型供应商页添加 Gemini 原生 API 连接，支持 API Key 和兼容网关。下载与更新使用独立目录，测试通过后再启用。

## 界面截图

**Agent 管理**

![Agent 管理：版本检查、安装、更新与测试](.github/assets/agents.jpg)

## 排查异常

软件启动后自动保存诊断日志，无需手动开启。在「查看 → 运行日志」或聊天顶部的终端图标打开日志面板，点击「打开文件夹」查看文件，或「导出诊断日志」收集最近 30 天的记录。「清空显示」只清空面板，不删除文件。

Windows 日志目录为 `%APPDATA%\dev.supercode.desktop\logs`。日志按日期保存为 UTF-8 JSONL，每个分段最多 4 MiB，启动时及运行期间每小时清理超过 30 天的日志。记录包含版本、时间、会话 ID、Agent、任务状态、进程退出、错误输出、上下文压缩和浏览器/界面异常；API 密钥、认证令牌等会在写入前脱敏，不主动记录对话正文。

如果另一台设备出现自动停止，请在该设备导出日志，并告知出现问题的大致时间，以便对应任务和进程记录。

## 从源码运行

准备 Node.js 22+、Rust、Windows C++ Build Tools 和 WebView2。

```powershell
git clone https://github.com/heyule69/SuperCode.git
cd SuperCode
npm ci
npm run desktop
```

```powershell
npm run build                                      # 构建前端
npm test                                           # 前端测试
cargo test --manifest-path src-tauri/Cargo.toml      # Rust 测试
npm run desktop:build                              # 打包桌面端
npm run installer:build                            # 构建 Logo 动画安装器
```

安装器构建还需要 Python 与 NSIS，详见 [安装器说明](installer/README.md)。

依赖安全检查可在项目目录运行；cargo-audit 安装到被 Git 忽略的本地工具目录，无需更改 PATH：

```powershell
npm audit --registry=https://registry.npmjs.org
cargo install --locked --version 0.22.2 --root .supercode/tools cargo-audit
& '.supercode/tools/bin/cargo-audit.exe' audit --file src-tauri/Cargo.lock --db .supercode/tools/advisory-db --url https://github.com/RustSec/advisory-db.git
& '.supercode/tools/bin/cargo-audit.exe' audit --file installer/Cargo.lock --db .supercode/tools/advisory-db --no-fetch
```

审计使用 [RustSec 公告数据库](https://rustsec.org/)。Cargo.lock 包含跨平台依赖，告警需结合实际构建平台的依赖树判断，例如 `cargo tree --locked --manifest-path src-tauri/Cargo.toml --target x86_64-pc-windows-msvc --edges normal,build`。审计只覆盖已发布依赖公告，不代表外部 Agent 或系统组件已完成安全验证。
