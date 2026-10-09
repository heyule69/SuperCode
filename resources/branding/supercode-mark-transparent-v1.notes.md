# SuperCode 透明徽记

2026-10-09：按用户要求，软件图标和安装器共用独立的橙色放射徽记，移除原来的浅色圆角底板。原始选定图保留在 `supercode-icon-ai-v3-radial.png`。

- 当前素材：`supercode-mark-transparent-v1.png`，1254 × 1254，RGBA，背景使用真实 Alpha 透明。
- 处理工具：内置 `image_gen.imagegen`，使用原图编辑，`transparent_background=true`。
- 桌面 ICO、ICNS、各尺寸 PNG 由 Tauri CLI 转换，未使用 Python 编辑图片。
- 安装器当前用 Canvas 将原始徽记拆为 40 片，随进度翻折、汇聚和拼合；空白区域保持透明。早期从下往上填充的 HTML 已保存在本机备份中。

首次背景处理的完整提示词：

```text
Use case: background-extraction. Asset type: transparent foreground logo for SuperCode installer HTML. Input image 1 is the exact edit target, NOT a style reference. Remove the entire cream/white rounded-square tile, its edge and shadow. Preserve ONLY the existing eight orange radial petals/rays, preserving their exact count, geometry, arrangement, proportions, orientation, orange color and smooth appearance. Keep the central hole and all gaps between rays fully transparent, and make everything outside the orange rays actual alpha transparency. Center the standalone orange emblem on a square transparent canvas with modest padding. Do not redesign the emblem. No white plate, no rounded-square background, no stroke around the emblem, no extra symbols, no lettering, no watermark, no rendered checkerboard. This is a precise background removal of the given logo.
```

透明边缘清理的完整提示词：

```text
Use case: background-extraction cleanup. Edit target: the provided transparent SuperCode orange eight-ray emblem. Change only the alpha cutout cleanup: eliminate ALL scattered orange/red speckles, low-opacity smears, colored halo and stray detached pixels outside the eight ray shapes and in their gaps. Preserve the exact eight existing foreground rays, their placement, smooth shape, orange appearance and proportions. Each ray must have a crisp clean antialiased contour like a professionally exported vector logo. Central hole and spaces between rays must be truly alpha zero. Keep a transparent canvas everywhere outside the emblem. No drop shadow, no backdrop, no white tile, no border, no new objects, no text, no checkerboard. Keep all original foreground logo geometry unchanged.
```

生成文件：`exec-aad92255-d8fe-4f73-969f-893c6171b1df.png`、`exec-a6e9fd2d-b149-4777-86fc-697f3244bf00.png`；当前素材来自第二次生成。
