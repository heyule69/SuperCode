# 设置页

2026-10-06，根据用户要求重新设计。

- 固定设置窗口与左侧分类：通用、模型供应商、Agent、资源。
- 供应商页分别显示连接、官方账号、添加和 CC Switch 导入。
- 连接、供应商、模型和导入列表分别滚动，表单保存栏固定在底部。
- 只保留字段、状态和操作名称；套餐切换、密钥保留、测试请求等必要信息放在提示或字段占位符中。
- 导入页进入时读取本机配置，支持搜索、勾选、全选、清空和导入全部。导入后加入供应商连接列表，不自动启用。

## 验证

浏览器仅验证布局，不冒充真实 Agent：

| 窗口 | 结果 |
| --- | --- |
| 760 × 700 | 设置容器高 650 px，内容区高 598 px，整体无垂直或水平溢出；配置与模型栏并排，保存栏在窗口内 |
| 1024 × 600 | 内容区高 498 px，滚动高度同为 498 px；较矮窗口中表单局部滚动 94 px，保存栏底部 555 px，保持可见 |

前端构建、10 个现有测试与 UTF-8 无 BOM 检查通过。供应商页按需加载，未增加前端依赖。

发布版桌面检查：读取本机 CC Switch 的 20 项配置，勾选 DeepSeek 后选中数从 2 变为 3；返回后仍是原来的 2 个已保存连接，勾选不会自动导入。官方连接编辑页只显示本机账号与模型，不显示 API Key / Base URL 字段。

桌面截图：`.supercode/screenshots/settings-provider-final.png`、`.supercode/screenshots/settings-import-final.png`。

发布文件：`src-tauri/target/release/supercode.exe`、`src-tauri/target/release/bundle/nsis/SuperCode_0.1.0_x64-setup.exe`（约 3.38 MiB）。

## 2026-10-07 控件调整

- 自定义 / 兼容 API 位于添加连接目录第一项。
- 文本框与下拉框统一为 36 px；焦点边框放在控件内部，避免外侧轮廓被滚动容器裁切。
- 下拉框箭头、文字内边距与禁用状态统一；URL 和模型 ID 使用等宽字体。
- 自定义连接只保留一处 API 协议选择，默认名称为“自定义连接”。
- 无模型时隐藏搜索框，空列表缩为 96 px，新增模型输入框使用独立占位文字。

构建、前端 10 项、Rust 22 项测试和 UTF-8 无 BOM 检查通过。

发布版桌面检查通过：自定义连接位于第一项，输入框与下拉框的焦点边框完整显示，API 协议菜单正常展开。检查未保存新连接或调用模型。截图：`.supercode/screenshots/settings-custom-first.png`、`.supercode/screenshots/settings-custom-inputs.png`。安装包已更新。
