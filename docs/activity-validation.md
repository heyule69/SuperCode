# 执行过程与桌面交互验证

日期：2026-10-06。此次重点是把真实 Agent 执行过程接入 SuperCode，并完善对应交互。

## 运行过程

Claude Code 2.1.287 使用 CC Switch 当前 kimi 配置，通过 SuperCode 原生 JSONL 运行时完成验证。收到 38 次思考摘要更新、24 次工具参数更新、5 次文本更新；Read 工具的路径、参数和结果保存到历史。CLI 使用的供应商映射为 kimi，并不代表 Anthropic 官方模型验证。

Codex CLI 0.160.1 使用 CC Switch 官方配置及 gpt-5.6-sol，通过 SuperCode app-server 运行时完成验证。收到 42 次文本更新、4 次命令输出更新。该轮没有返回思考摘要；空摘要不会显示为虚构内容。对应协议路径通过分段事件测试覆盖。

两者均通过真实首轮输出、进程释放后原生会话恢复、内置工具读取 README 和中断验证。Claude 另通过 Write 请求进入审批等待后停止：不自动批准、不创建目标文件。历史中没有残留运行中的活动。报告和原始事件位于 `.supercode/claude-activity-smoke-report.json`、`.supercode/codex-activity-smoke-report.json` 和 `activity-*-events.json`。

发布版重复真实验证通过：Claude 34 次思考摘要更新、75 次工具参数更新、16 次文本更新；Codex 37 次文本更新和 4 次命令输出更新。报告为 `.supercode/claude-release-activity-report.json` 和 `.supercode/codex-release-activity-report.json`。8 个前端测试与 12 个 Rust 测试通过。新增大数据测试确认 4 MiB 输入参数被有界保存，Unicode 截断不留下半个 emoji。CC Switch、Claude settings、Codex config/auth 文件的 SHA-256 均与验证前一致。

## 展示与持久化

- 思考摘要与回复分开，工具按实际名称显示读取、搜索、编辑或命令。
- 工具输入参数生成、执行、完成、失败与停止分别显示。
- 命令输出、文件差异、MCP 参数/结果/进度、执行计划、网络搜索和协作调用使用对应卡片。
- 活动条目按回合分组，可逐项展开或收起。完成后显示耗时与工具调用数量。
- 开始和完成事件落盘；流式片段保存在有界缓存中，切回会话时读取快照。每个活动正文最多 128 KiB、元数据最多约 128 KiB，缓存最多 200 项且序列化内容不超过 2 MiB。
- 应用异常退出后，未完成条目会恢复为“已停止”。

MCP、文件编辑与协作卡片的协议合并路径已实现，真实本轮主要验证内置文件/命令工具；不把所有外部工具都称为已实测。AskUserQuestion 支持多选与自定义输入，真实用户回答链路尚未在本轮完整交互验证。

## 界面检查

真实测试产生的历史和事件在独立开发页面中使用与桌面版相同的 Conversation、Markdown 和事件合并代码回放；页面明确标注“真实本地 Agent 事件回放”，不会再次调用模型，也不打入发布包。生成方式：`node scripts/prepare-activity-preview.mjs`，开发服务器路径 `/.supercode/activity-preview.html`。

已检查工具展开/收起、输入参数与结果、停止标记、浅色/深色样式、会话草稿隔离。760 px 窗口无水平溢出，变更面板默认隐藏，可主动展开和关闭。设置改为常规、Agent 连接、CC Switch 导入、资源使用四类，并固定标题与分类栏。输入框自动增高，滚动查看旧消息时不抢回底部；新建/搜索/设置/侧栏提供快捷键。浏览器预览不调用本地 Agent。

当前实现对齐 Codex 桌面端的主要会话和执行反馈；不声称具备它的全部工作树、自动化、插件或多人任务能力。

最终原生 exe 与 NSIS 安装包成功生成并启动；原有 Claude 会话、选用模型和 CC Switch 配置保留。exe 约 8.61 MiB，安装包约 2.44 MiB。此次应用及 WebView 进程组样本：工作集 740.3 MiB、私有工作集 391.5 MiB，空闲无 Agent 进程。这是单次样本，不代表固定值或相对旧版本的内存下降；WebView 占用仍需持续测量。

参考：[Codex app-server 官方事件文档](https://learn.chatgpt.com/docs/app-server)、[Claude 官方流式事件文档](https://platform.claude.com/docs/en/build-with-claude/streaming)，以及本机 CLI 生成的 JSON Schema。
