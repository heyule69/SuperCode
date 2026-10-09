# SuperCode Windows 安装器

安装界面沿用已确认的透明 Logo 与深色设计，仅显示安装目录、必要图标按钮和安装中的小百分比。Logo 区域按窗口高度伸缩，无主题切换按钮和页面滚动条。安装前、安装中、完成后是实际状态；正式界面没有演示计时器。

安装核心使用 Rust，界面通过 Tauri IPC 接收实际文件写入进度。所有嵌入文件在覆盖前和安装后进行 SHA-256 校验；旧程序、卸载器、注册信息和快捷方式保存在安装目录的 `.supercode-backups/`。提交失败会回滚；发现上次中断的提交会先恢复。安装程序只处理自身文件，不写入聊天数据库、账号、Agent 配置或媒体目录。

安装器检测目标文件占用，要求先从系统托盘退出正在运行的旧版。安装时锁定目录和关闭按钮。完成按钮实际启动所选目录中的 SuperCode。新版支持同目录更新；更换已有安装位置时，应先卸载旧版。卸载保留聊天数据和版本备份。

外层 NSIS 只负责准备界面运行环境并启动原生安装界面。存在 WebView2 时直接复用系统运行时；缺少时从微软官方下载并安装。因此缺少 WebView2 的电脑首次准备需要网络。注册和卸载由独立 NSIS 工具完成，子进程不打开终端。

## 构建

Windows 需要项目现有的 Node、Python、Rust/MSVC 工具链和 NSIS。`npm run desktop:build` 可准备 Tauri 使用的 NSIS 工具，也可以通过 `SUPERCODE_NSIS` 指定 `makensis.exe`。验证脚本使用 PowerShell 7（`pwsh`），保证 UTF-8 无 BOM 脚本按 UTF-8 读取。

```powershell
npm ci
npm run installer:build
```

输出为 `release/SuperCode_0.1.0_x64-setup.exe` 和对应 SHA-256 文件。生成的 UI、嵌入安装载荷及可执行文件都不提交到 Git。界面源码为 `ui-template.html`，Logo 来源为 `resources/branding/supercode-mark-transparent-v1.png`。

开发时已有可信的最新 release 主程序，可以使用 `node scripts/build-installer.mjs --reuse-app`。`--prepare-only --reuse-app` 只准备测试所需载荷与界面，不生成最终包。

## 验证

```powershell
npm run build
npm test
cargo test --manifest-path src-tauri/Cargo.toml
npm run installer:test
npm run installer:test:ui
pwsh -NoProfile -File scripts/verify-installer.ps1
npm run check:encoding
```

Rust 测试覆盖文件校验、备份、回滚、安装互斥、路径边界和中断恢复。UI 检查使用模拟 IPC 与 Canvas 验证状态绑定，不代表真实视觉验收。隔离安装验证使用生成的完整 EXE、实际文件解压和实际 NSIS 卸载器，安装到项目内包含中文与空格的目录，跳过正式注册与快捷方式，不覆盖已安装的程序或配置。

原生安装界面另外通过 `--verify-ui <绝对报告路径>` 做启动检查：实际 WebView2 解码 Logo、生成 40 块 Canvas 纹理并连通 Tauri IPC 后，写入 UTF-8 报告并退出；此检查不自动安装，也不代替人工检查动画观感。
