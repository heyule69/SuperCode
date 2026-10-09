# Agent 输入与生命周期审计

调查日期：2026-10-09。范围是运行中补充输入、排队、取消和连接复用；不是所有客户端功能的兼容性认证。

## 已核实的能力

| Agent / 本机版本 | 运行中补充输入 | 原生后续消息 | SuperCode 的接入 |
| --- | --- | --- | --- |
| Codex CLI 0.160.1 | app-server `turn/steer`，带 `threadId` 和 `expectedTurnId` | 应用端维护下一轮消息，未提交时可编辑、删除 | 原生 steer 校验活动任务及已发送的权限 / effort；继续复用 app-server 和已加载会话 |
| Claude Code 2.1.287 | 双向 stream-json 可写入带 UUID 的用户消息；工具完成边界可在当前轮消费，纯文字生成期间可能等到下一轮 | CLI 本身有队列及消息生命周期能力 | “引导”写入原进程，以 `priority: "next"` 等待原生消费；不用中断来模拟补充 |
| OpenCode 1.18.23 | 此版本 ACP 允许第二个 `session/prompt` 加入正在运行的会话循环，两个请求都等待循环结束 | ACP 没有单独的 steer / follow-up 标准协商能力 | 初始化返回版本恰为 1.18.23 时使用并行 prompt；其他版本显示“停止并引导” |
| Pi 1.0.4 | `steer` 在当前工具执行完、下一次模型调用前进入；`prompt` 的 `streamingBehavior: "steer"` 同时覆盖运行中和刚转为空闲的情况 | `follow_up` 在没有工具调用、没有 steering 待发消息时处理；可配置一次一条或全部处理 | “引导”使用原生 prompt / steer 行为；应用维护可编辑、可持久化的下一轮队列 |

Claude 的接收时机依据 [官方交互文档](https://code.claude.com/docs/en/interactive-mode#when-claude-code-sends-what-you-queued)、[官方 Agent SDK Python 客户端](https://github.com/anthropics/claude-agent-sdk-python/blob/main/src/claude_agent_sdk/client.py)，以及官方 npm 包 `@anthropic-ai/claude-agent-sdk` 0.3.295 的 SDKUserMessage / SDKControlInterruptRequest 类型。CLI 2.1.287 初始化实际返回了 `msg_lifecycle_v1`、`interrupt_cancel_queued_v1` 等能力。补充消息提交成功不代表当前正在生成的文字立即终止；界面说明为“在 Agent 下一可接收的位置生效”。

OpenCode 的依据是 [v1.18.23 ACP service](https://github.com/anomalyco/opencode/blob/v1.18.23/packages/opencode/src/acp/service.ts)、[同版本 ACP event](https://github.com/anomalyco/opencode/blob/v1.18.23/packages/opencode/src/acp/event.ts)、[同版本 session prompt](https://github.com/anomalyco/opencode/blob/v1.18.23/packages/opencode/src/session/prompt.ts) 及本机协议运行。创建用户消息发生在加入已有会话循环之前。需要特别保留版本边界：[最新 v2 ACP 文档](https://opencode.ai/v2/docs/cli/acp/) 明确只允许一个活动 prompt，因此不能把 1.18.23 的行为推广到更新版。未知版本不自动尝试并行补充，也不会在远程报错后偷偷中断原任务。

Pi 的依据是 [官方 RPC 命令](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/docs/rpc-commands.md)、[官方 RPC 概览](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/docs/rpc.md) 和本机 1.0.4 协议。`success` 只确认接收；`disposition: "handled"` 可能由输入扩展直接处理，不产生模型轮次。`agent_end` 后可能还有重试、压缩或内部队列，应用等待 `agent_settled`，再确认原生状态确实空闲。

## 接入修复

Codex 的协议依据是 [官方 App Server 文档](https://learn.chatgpt.com/docs/app-server) 和本机 0.160.1 生成的 JSON Schema（`.supercode/research/agent-input-audit/codex-schema-0.160.1`）。`turn/start` 返回任务 ID，`turn/started` 通知实际启动；`turn/steer` 要求 expectedTurnId 匹配，不能传入新的任务级配置；`turn/interrupt` 成功后等待原生 interrupted 完成事件。

- Codex 的真实测试发现启动回复与活动任务建立之间有时序间隙：发送返回后立刻停止可能得到 `no active turn to interrupt`。应用保留 starting 状态直到原生启动通知，停止等待相同任务 ID 就绪；对这个明确的暂态错误限时重试。已完成的任务直接结束取消流程，任务 ID 变化则拒绝，避免误停后续轮次。迟到的启动回复不覆盖已经完成的状态。
- Claude 开启 `--replay-user-messages`，按 UUID 确认补充输入消费；尚有未消费输入时，第一份 result 不会提前把应用任务设为空闲。纯文字场景的多个原生 result 聚合为一次应用完成事件。Token 分段累计，累计原生成本转为当前应用轮次的增量；恢复旧会话时未知初始成本不猜测。
- Claude 停止使用 `cancel_queued: true`。旧 CLI 未声明能力且仍有补充消息时关闭进程，避免停止后队列自行续跑；未消费消息暂停保留。
- OpenCode / Pi 的原始 prompt 写入先于活动任务对外可见，避免发送返回后立即点击停止，却把取消命令写在 prompt 前面。准备阶段的停止标为 interrupted；OpenCode 尚未进入会话循环时采用关闭进程的取消方式。
- Pi 等待初始 prompt 和已经提交的引导完成接收，先 clear_queue 再 abort，最后再次清理取消钩子的待发内容；abort 自身会继续尚存的队列，因此不能只在 abort 后清理。清理完成前不把连接交给下一轮。正常取消后，已经确认空闲的 OpenCode / Pi 连接继续复用；不再一律杀进程。
- 引导校验当前任务 ID。四种 Agent 的引导还校验活动权限和推理设置；不同配置的消息保留在队列，并提示下一轮发送，避免界面设置与实际执行不一致。Codex 的 steer 只传入补充输入，不能把 UI 里改过的权限或 effort 当作已在当前轮生效。
- 原生提交保留待发记录直到确认。Claude 确认消费；OpenCode 确认 prompt 结束；Pi 确认接受或处理。Pi 接受不等于模型已执行，生成结束另由 settled 状态判断。错误时保留并暂停消息，用户可以继续排队。

## 应用队列与原生队列

输入框上方的普通排队由 SuperCode 数据库管理，上一轮完成后按顺序发送。提交前支持编辑、删除、暂停和侧边打开，重启保留并暂停，避免重复执行。即使 Agent 支持原生 follow-up，也不把所有可编辑消息提前交给 Agent 的内部队列。点击“引导”才把该条消息提交到当前任务，提交过程中锁定编辑和删除。

这个选择保留了用户要求的交互与持久化行为，不能把它描述为每个 Agent 的原生队列 UI。当前仍是全局同时运行一个 Agent 任务；侧边聊天也受这个调度限制，尚未对齐 Codex 的多任务并行。

## 验证证据

使用真实安装的 CLI 和模拟模型服务器做了可复现的协议测试，不把这些测试称为真实模型执行：

- Claude：工具边界的补充出现在同一原生 result；纯文字生成补充出现在第二个 result；两种情况下进程不变。脚本 `.supercode/research/probe-claude-input.py`。
- Claude 图片补充：约 5.6 MiB 的测试 PNG 经过 CLI 规范化后，最大的回传 JSONL 帧为 655,987 字节；补充保持双轮消费且进程不变。这个检查使用模拟模型，验证图像传输与生命周期，不证明所有供应商的视觉理解能力。
- OpenCode 1.18.23：两个并行 ACP prompt 均正常完成，补充消息进入模型输入。脚本 `.supercode/research/probe-opencode-input.py`。
- Pi 1.0.4：steer / follow_up 返回 queued，输入扩展返回 handled；回复顺序为原始 → steer → follow-up，一次 settled。脚本 `.supercode/research/probe-pi-input.py`。
- Pi 1.0.4 队列取消：当前输出被测试服务器暂挂，原生 steering / follow-up 已接收；先 clear_queue 再 abort 后，内部待发量为零，模型请求仍只有最初一次，没有执行被取消的补充。命令 `python -X utf8 .supercode/research/probe-pi-input.py --cancel`。

另用现有官方账号和 API 连接运行四种真实 Agent，测试数据全部在隔离数据库，正式会话不参与测试。连续两轮 PID 不变，客户端 FIFO 排队两轮按序回复，原生引导收到 `STEER-OK`，补充用户消息只保存一次，应用完成事件仅一次。报告为 `.supercode/agent-input-report.json`，每个适配器的 `isolatedData` 指向测试数据库。附加测试覆盖过期任务 ID 拒绝、配置变化拒绝、运行中停止、立即停止及停止后续聊；Pi 针对取消顺序另行复测。四种适配器均通过；Codex 修复后正常停止、立即停止和续聊保留同一 PID。OpenCode 的立即停止允许关闭尚未开始工作的进程，并从原会话恢复。

Claude / OpenCode / Pi 测试使用只读设置；Codex 使用请求批准模式，全部测试提示均明确禁止工具调用和文件操作。不自动批准审批或提问；触发请求将使测试停止。模拟服务器测试不使用真实凭据；真实测试从已有 DPAPI 存储读取配置，不打印凭据。

本轮前端 168 项、Rust 150 项测试通过；新增回归覆盖原生启动等待、通知先于 RPC 回复、迟到回复不复活已完成任务、任务 ID 变化拒绝及已完成任务状态缓存上限。前端发布构建和 UTF-8 无 BOM 检查通过。

修改前备份：`.supercode/backups/agent-input-audit-20261009-072939`，包含源文件、旧程序和数据库快照（2 项目 / 12 会话 / 378 消息）。更新安装前另行生成数据快照。

NSIS 发布构建及静默更新安装成功，更新快照为 `.supercode/backups/agent-input-install-20261009-081135`。已安装 EXE 为 17,413,632 字节，SHA-256 为 `a2bbf7448aac8b7a1c801863f1eec2a69fc2672de4c8a28d75c464fc04edf06e`；与 release 文件仅有三个 NSIS 标记字节不同。安装核验确认原项目、会话、消息 ID 与消息文本全部保留，数量仍为 2 / 12 / 378，活动任务及待发消息为零，崩溃日志未变。安装记录为 `.supercode/agent-input-install-report.json`。
