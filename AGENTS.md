# SuperCode 开发约定

- 所有文本文件使用 UTF-8 无 BOM 读写。发现非 UTF-8 文本时先告知用户。
- 运行时优先低内存：agent 按需启动，输出限量缓存，重型界面按需加载。
- 桌面核心使用 Rust，前端使用 React + TypeScript，通信走 Tauri IPC。
- 不把模拟事件或浏览器预览称为真实 agent 运行。
- Windows 子进程不打开额外终端窗口。禁止把用户输入拼接成 shell 命令。
- 外部 agent 的审批与提问必须传递给用户，不自动批准。
- 修改协议、进程生命周期和持久化逻辑时，运行相应测试。

验证：`npm run build`、`npm test`、`cargo test --manifest-path src-tauri/Cargo.toml`。
