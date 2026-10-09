# 官方额度读取与项目单击

更新：2026-10-07。

旧实现只向匹配连接的运行中 Codex 客户端读取额度。刚打开软件或当前运行 Claude 时，官方额度直接返回空。

现在优先复用匹配的 Codex 客户端；否则使用独立、短暂的账号连接，完成 `initialize` / `initialized` 后调用 `account/rateLimits/read`。连接使用本机 CLI 已保存的 ChatGPT 官方登录，覆盖为 OpenAI 服务商并排除 API Key 环境变量。不会创建、恢复或发送聊天，也不会替换当前运行客户端或改动会话状态。

独立查询有 15 秒总超时，结束主动释放进程；取消与异常由 `kill_on_drop` 及 Windows JobGuard 清理。禁止在账号连接执行工具或批准请求。并发账号读取串行，沿用 60 秒有效数据缓存和 20 秒无数据缓存；刷新可跳过缓存。

展示平台返回的窗口剩余百分比、重置时间及套餐字段。支持多额度桶和旧版单桶；空多桶字段可回退到单桶。5 小时和周额度分别标注，缺失或不合法数据不推算。

协议出处：[OpenAI Codex App Server — Rate limits](https://learn.chatgpt.com/docs/app-server#6-rate-limits-chatgpt)。

项目名称与文件夹图标都单击展开或收起，名称不再依赖双击；右侧新聊天和菜单保持独立。

验证：前端 105 项、Rust 87 项测试通过。新增账号协议测试覆盖初始化、读取、通知、非账号请求拒绝、断连及远端错误；额度解析测试覆盖空多桶回退、真实 0% / 100%、周周期与重置时间。

发布构建、NSIS 打包与 UTF-8 检查（154 个文本文件）通过。实际打开重新生成的桌面程序，在没有运行中聊天、没有匹配 Codex 客户端的情况下，额度页读取到当前 ChatGPT Pro 官方登录的周额度剩余 91%，显示平台返回的重置时间。本次平台只返回了一个有效额度窗口，不增加缺失的周期。

真实桌面操作验证项目名称单击收起、再单击展开，原聊天内容保持显示。额度读取后进程清单没有 SuperCode 的 Codex 子进程，当前活动聊天数为 0。实拍：`.supercode/screenshots/codex-official-quota.png`、`project-single-click-collapsed.png`、`project-single-click-expanded.png`。本轮不发送新的模型对话。
