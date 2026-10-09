# Windows 安装器

Rust + Tauri 安装界面，Logo 动画显示实际安装进度。覆盖旧版本前自动备份，失败时回滚，保留聊天与 Agent 配置。支持软件内更新和卸载。

## 构建

需要 Node.js 22+、Rust/MSVC、Python 和 NSIS。先运行 `npm run desktop:build` 准备 Tauri 使用的 NSIS，或通过 `SUPERCODE_NSIS` 指定 `makensis.exe`。

在项目根目录运行：

```powershell
npm ci
npm run installer:build
```

安装包输出到 `release/`。界面源码为 `ui-template.html`，Logo 为 `resources/branding/logo.png`。生成的界面、载荷和安装包不提交到 Git。

## 发布

1. 同步根目录 `package.json`、两个 Cargo 项目和两个 Tauri 配置中的版本号。
2. 在 `.github/release-notes/<版本>.md` 中填写版本说明。
3. 设置 `TAURI_SIGNING_PRIVATE_KEY_PATH`，构建并验证安装包。
4. 提交源码，运行 `npm run release:publish`。

发布脚本通过 Git SSH 上传临时构建产物，GitHub Actions 校验后发布安装包和 `latest.json`，并清理临时分支。更新签名和校验值包含在 `latest.json` 中。签名私钥单独保管，后续版本使用同一密钥。

## 验证

```powershell
npm run installer:test
npm run installer:test:ui
npm run installer:test:update-ui
pwsh -NoProfile -File scripts/verify-installer.ps1
```

Rust 测试覆盖文件校验、备份、回滚和中断恢复。界面测试使用模拟 IPC；安装验证使用实际安装包和卸载器，在隔离目录中运行。
