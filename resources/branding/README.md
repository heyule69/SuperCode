# SuperCode 应用图标

当前图标（2026-10-09）：使用 `supercode-mark-transparent-v1.png` 的独立橙色徽记，移除浅色底板。软件、托盘、通知和安装器使用同一图标。原始素材保留，处理过程与完整提示词见 `supercode-mark-transparent-v1.notes.md`。

当前资源重新生成命令：

```powershell
npm exec tauri icon -- 'resources/branding/supercode-mark-transparent-v1.png' --output 'src-tauri/icons'
Copy-Item -LiteralPath 'src-tauri/icons/128x128.png' -Destination 'public/app-icon.png'
Copy-Item -LiteralPath 'src-tauri/icons/32x32.png' -Destination 'public/favicon.png'
```

`public/favicon.svg` 嵌入同一张 32 × 32 透明 PNG。设计演示位于 `docs/designs/installer.html`；已确认的正式安装器源码位于 `installer/`，安装包输出到 `release/`。正式界面使用同一透明徽记和真实安装进度，软件、原生安装器 EXE 的六个内嵌图标尺寸均与源 ICO 一致。

以下记录保留图标的历史选择与验证过程。

2026-10-08：用户选择方案 2，浅色圆角方块与橙色放射徽记，作为软件图标。

- 原始素材：`supercode-icon-ai-v3-radial.png`，1254 × 1254，RGBA，来自内置 ImageGen。保留用户选定的原始生成图。
- 全部设计方案和提示词：`supercode-icon-variants-v2.prompts.md`。
- 桌面与安装包：`src-tauri/icons/`，使用 Tauri CLI 转换所需尺寸、ICO 与 ICNS。
- 软件内“关于”和通知示例：`public/app-icon.png`。
- 浏览器标签：`public/favicon.png`。
- Windows 通知：嵌入 `src-tauri/icons/128x128.png`；注册通知时比较现有缓存并按需更新，使升级后的通知使用新图标。

重新生成打包图标：

```powershell
npm exec tauri icon -- 'resources/branding/supercode-icon-ai-v3-radial.png' --output 'src-tauri/icons'
Copy-Item -LiteralPath 'src-tauri/icons/128x128.png' -Destination 'public/app-icon.png'
Copy-Item -LiteralPath 'src-tauri/icons/32x32.png' -Destination 'public/favicon.png'
```

仅做应用发布所需的尺寸和格式转换，没有重新设计用户选中的徽记。

验证与安装：前端构建成功，155 项前端测试与 132 项 Rust 测试通过。新的 NSIS 安装包已安装（退出码 0），安装目录中的程序已打开；原生界面确认“关于”和通知示例使用所选图标，本机测试通知成功提交系统，通知缓存已更新为本次 128 × 128 图标。安装 EXE 与 release EXE 仅相差 NSIS 包类型的三个标记字节，其余一致。

更新前备份：`.supercode/backups/branding-20261008023450`，包括旧程序、图标、SQLite 一致性备份、窗口状态、WebView 偏好、附件和媒体。更新前后保持 2 个项目、11 个会话、291 条消息，崩溃日志仍为原有 4 条。

后续修正：上述界面验证没有覆盖 Windows EXE 内嵌资源；旧编译缓存仍保留闪电图标。`src-tauri/build.rs` 已显式监听图标资源，重新编译并安装后，EXE 的六个图标尺寸全部与源 ICO 一致。针对本机旧快捷方式缓存，安装目录另存内容哈希命名的 ICO，并更新 SuperCode 自己的桌面和开始菜单快捷方式；资源管理器刷新后已实际确认显示所选橙色徽记。

最终验证：156 项前端测试、135 项 Rust 测试通过，发布版原生窗口与托盘测试通过。安装程序为 `src-tauri/target/release/bundle/nsis/SuperCode_0.1.0_x64-setup.exe`，安装版 EXE SHA-256 为 `c077e1c2ff088df1a93b8087d97cabe8a4436dcbd3d64bd9452c03bf8e885367`。本次更新前备份位于 `.supercode/backups/tray-icon-20261008`。
