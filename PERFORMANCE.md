# 性能验证

`npm run benchmark` 使用固定合成数据测量消息合并、轮次分组及服务端静态渲染，报告保存到 `.supercode/benchmarks/conversation.json`。它不代表真实 agent 或 WebView 的帧率。每次比较使用相同 Node 版本、硬件和负载，保留前后报告；结果恶化超过 20% 时检查原因，避免以一次波动直接判定失败。

原生进程测量只观察用户已经打开的 SuperCode，不自动启动应用、模型或工具：

```powershell
pwsh -NoProfile -File scripts/measure-performance.ps1 -SuperCodeProcessId <现有进程ID> -Scenario idle -OutputPath .supercode/idle.json
```

分别记录 `startup`（打开后开始采样）、`idle`、`parallel`（固定 4 个任务）、`long-chat`（固定历史规模）四种场景。脚本汇总整个进程树，输出内存峰值、P95、CPU 核占用及进程数量；使用 `-CompareWith` 比较同场景。任务内容和模型响应时间应另行固定、记录；启动耗时与 WebView 帧率需要专项测量，不能从内存采样推导。

先积累同机基线，再设置产品预算。流式交互的 CPU 成本与历史长度、存储写入次数一起检查，完成轮次尽量复用对象、缓存 diff；中途检查点以限频写入为主，结束时保存最终记录。
