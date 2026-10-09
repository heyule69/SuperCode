# 权限菜单对齐

更新：2026-10-08。

权限选项随 Agent 实际能力展示。Codex 使用用户截图中的三项，不在新菜单中额外加入只读。输入区与常规设置复用菜单，顶部保留说明入口、两行选项说明、当前选择勾选及橙色完全访问。完全访问仍经过明确启用确认；关闭菜单、Esc、点击外部或开始任务不会提交未确认的选择。

尺寸在 2026-10-08 收紧为 360px 宽度、52px 最小行高、12px 圆角，默认标题 12px、说明 11px。字号继续随应用界面字号调整；较长说明可以换行，不截断审批含义。默认字号下，Claude 六项菜单为 360 × 355px。

## 实际审批规则

| Agent | 显示选项 | 实际执行规则 |
| --- | --- | --- |
| Codex | 请求批准 / 帮我批准 / 完全访问权限 | 前两项都是 workspace-write + on-request，分别传递 user / auto_review；完全访问使用 danger-full-access + never |
| Claude Code | 计划模式（只读）/ 请求批准 / 自动接受编辑 / 自动模式 / 不询问 / 完全访问权限 | plan / manual / acceptEdits / auto / dontAsk / bypassPermissions |
| OpenCode | 只读 / 请求批准 / 完全访问权限 | 允许读取并拒绝其他工具 / 工具请求批准 / 工具允许执行 |
| Pi | 只读 / 逐项批准 / 读取自动批准 / 完全访问权限 | 限定读取 / 每项工具确认 / 读取直接执行而其他工具确认 / 跳过工具确认 |

Codex 的原生自动审查仅更换审批者，保留工作区和网络边界。参数在 thread/start、thread/resume 和 turn/start 中同步传递，界面原生命令也采用同一规则。旧 strict 迁移为用户审批语义，不再传递已弃用的 untrusted。没有添加由 SuperCode 自动点击或自动回复原生审批的逻辑。[官方权限说明](https://learn.chatgpt.com/docs/sandboxing)、[自动审查说明](https://learn.chatgpt.com/docs/sandboxing/auto-review)。

Claude 映射已核对本机 2.1.287 的 CLI 参数。自动模式受 Claude 版本、账户和平台策略约束；失败会显示原生错误，不改为完全访问。OpenCode 不再重复展示两个实际相同的审批选项。Pi 的逐项确认与读取自动批准使用不同工具规则，名称直接描述其能力。

## 保存与兼容

使用 supercode.permissions.v2 分别保存各 Agent 的选择；旧单值仅在首次迁移时应用于当前 Agent。切换或重启不会把其他 Agent 的完全访问复制给未配置的 Agent。不同 Agent 不支持的模式回到请求批准。已有只读记录仍按只读执行，Codex 菜单会说明当前保留的旧选择，不因升级或打开菜单而增加权限。

## 验证

- npm run build 成功；npm test：155 项通过；cargo test：132 项通过。
- React DOM 验证三项 Codex、六项 Claude、三项 OpenCode、四项 Pi，以及当前选择、原生 ID、全访问确认、只读兼容、键盘操作、回焦与任务启动关闭。
- 持久化检查覆盖 Agent 独立选择、旧审批迁移、无效数据和不支持的模式，避免隐式开启自动审查或扩大权限。
- 本机 Codex 0.160.1 app-server 的无模型任务检查：两个 ephemeral thread/start 分别返回 user 与 auto_review，approvalPolicy 都为 on-request，sandbox 为 workspaceWrite，networkAccess 为 false。没有发送 turn/start，没有更改本机全局配置。报告位于 .supercode/research/permission-protocol-report.json。
- 浏览器预览确认 Claude 六项及 Codex 三项的实际界面；预览不代表真实 Agent 执行。

已生成并安装新的 NSIS 包，安装程序退出码为 0；更新后的 SuperCode 已重新打开。已安装 EXE 与本次 release 构建大小相同（16,328,704 字节），除 NSIS 的三个字节包类型标记外逐字节一致。实际桌面确认 Claude 六项菜单、顶部说明入口、对应图标与完全访问橙色勾选；原会话和完全访问选择保留，验证菜单已关闭。

更新前备份：.supercode/backups/permissions-20261008-101453，包含 SQLite 一致性备份、偏好、窗口状态、附件和媒体。更新前后都是 2 个项目、11 个会话、291 条消息；没有运行中的任务，崩溃日志仍为 4 条。Codex 菜单已检查暗色、亮色，Claude 菜单已在 760 × 580 最小窗口尺寸下确认全部选项可见；预览已关闭，临时服务已停止。207 个项目文本文件通过 UTF-8 无 BOM 检查。

2026-10-08 尺寸调整验证：默认 Claude 菜单为 360 × 355px，六项行高均为 52px，说明没有截断。`npm test` 161 项、Rust 测试 137 项通过，前端和 NSIS 发布构建成功。新版已安装并打开；正式桌面确认六项选项、顶部说明入口及当前完全访问橙色勾选完整显示，检查结束后收起菜单。此次只调整尺寸和排版，没有改变审批映射或用户权限选择。更新前备份位于 `.supercode/backups/composer-menu-20261008-1439`。
