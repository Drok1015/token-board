# 额度脉搏

![额度脉搏桌面悬浮看板](public/screenshot.png)

仅支持 macOS 的桌面悬浮额度看板。它以无标题栏、透明背景的窗口常驻桌面，不占用菜单栏；可直接拖动到任意位置，并始终置顶显示。

## 功能

- 展示 CODEX、KIMI、GLM、DEEPSEEK 的可用额度或余额
- 自动读取本机已登录/已配置的账号信息；不将凭据写入项目或上传到 GitHub
- 每 5 分钟自动刷新
- 透明、无 Dock 图标的桌面悬浮窗口，可跨桌面显示并直接拖动

CODEX 行由本机已登录的 ChatGPT/Codex 获取。当前账号若只返回一个额度周期，就只显示该周期；若同时返回多个周期，看板会自动展示，例如 `5h 80% / 7d 60%`。

## 使用前提

- macOS 11 或更高版本
- 已安装并登录 ChatGPT 桌面版或 Codex CLI（用于 CODEX 额度）
- 已登录 Kimi Code（用于 KIMI 额度）
- 在 cc-switch 的 Codex 配置中设置了 GLM 与 DeepSeek 的 API Key（用于相应行）

未配置或登录失效的服务会在看板中明确显示状态，不会回显任何密钥。

## 从源码运行

```bash
npm install
npm run tauri dev
```

## 打包

```bash
npm run tauri build
```

构建结果位于：

```text
src-tauri/target/release/bundle/macos/额度脉搏.app
```

应用未签名或公证。首次打开被 macOS 阻止时，可在“系统设置 → 隐私与安全性”中选择允许打开。

## 说明

Codex 额度读取依赖本机 Codex 的 app-server 接口；该接口当前仍属于 experimental，未来 Codex 更新若改变接口，看板会显示“读取失败”，其余服务不受影响。
