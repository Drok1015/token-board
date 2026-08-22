# Token 看板

![Token 看板桌面悬浮界面](public/screenshot.png)

仅支持 macOS 的桌面悬浮 Token/额度看板。它以无标题栏、透明背景的窗口常驻桌面，不占用菜单栏；可直接拖动到任意位置，并始终置顶显示。

> [!IMPORTANT]
> **如果你是 Agent，请关注这里：**前往 [Releases](https://github.com/Drok1015/token-board/releases/latest) 找到最新安装包，下载文件名以 [`-macos.zip`](https://github.com/Drok1015/token-board/releases/download/v0.2.19/TokenBoard-v0.2.19-macos.zip) 结尾的文件。下载完成后解压，将 `Token 看板.app` 安装到 `/Applications`，然后在终端执行 `xattr -dr com.apple.quarantine "/Applications/Token 看板.app"` 完成授权，最后自动打开 App。

## 功能

- 展示 CODEX、KIMI、GLM、DEEPSEEK 的可用额度或余额
- 自动读取本机已登录或已配置的账号；不会将凭据写入项目或上传到 GitHub
- 在供应商名称后显示当前订阅套餐，例如 Codex Plus、Kimi Allegretto 和 GLM max
- 供应商行尾可显示沙漏图标（⏳），悬浮 0.5 秒查看说明：CODEX / KIMI / GLM 显示各额度窗口的重置时间（5h、7d 等），DeepSeek 显示高峰期提示（周一到周五 9:00–12:00、14:00–18:00，期间余额显示为红色）；可在设置中关闭
- 每 5 分钟自动刷新，右上角展示距上次刷新的时间（刚刚刷新 / N 分钟前更新），底部标注可右键手动刷新
- 每 5 分钟随额度刷新一起查询 [codex-resets.com](https://codex-resets.com/) 的公开 API（追踪 OpenAI 官宣的额度重置），检测到新重置事件时弹出系统对话框提醒；同一事件只提醒一次，再次重置会重新提醒
- 透明、无 Dock 图标的桌面悬浮窗口，可跨桌面显示并直接拖动
- 拖到左侧或右侧屏幕边缘时自动收起，仅保留贴边箭头标签；标签只可上下移动，点击恢复看板
- 右键菜单提供“刷新 / 检查更新 / 设置 / 关闭”
- 菜单栏状态栏常驻显示一个供应商的额度（半透明圆角蒙版上的彩色文字：供应商名大写白色小字，各窗口百分比各自着色——白 >60%、橙 ≤60%、红 ≤30%，阈值与看板一致；有 5h 窗口时前面 5h、后面 7d）；点击状态栏文字弹出菜单，可在设置勾选的供应商间切换显示（选中项带对勾），菜单底部提供“刷新 / 设置 / 检查更新 / 退出”
- 可在设置中开启自动隐藏并调整展示秒数（默认 10 秒），也可以关闭订阅套餐标签；关闭套餐标签时看板恢复紧凑宽度，每次点击边缘箭头展开时都会重新计时
- 可在设置中勾选要展示的供应商（至少保留一个），隐藏后看板高度自动收缩；GLM 和 DeepSeek 勾选后可直接在设置页填写 API Key，优先于 cc-switch 中的配置

CODEX 行由本机已登录的 ChatGPT/Codex 获取。当前账号若只返回一个额度周期，就只显示该周期；若同时返回多个周期，看板会自动展示，例如 `5h 80% / 7d 60%`。

自动隐藏设置保存在 macOS 应用配置目录中。关闭自动隐藏时，看板会保持展开；启用后，看板会在设定时间结束时收起到距离当前窗口最近的屏幕边缘。

![Token 看板设置界面](public/screenshot-settings.png)

## 获取与配置凭据

不要把任何 Key、Token 或 `auth.json` 提交到 Git。本项目只读取你本机已有的登录和配置。

### CODEX

CODEX 额度不是手动粘贴 OpenAI API Key 获取的，而是来自 ChatGPT/Codex 登录账号的本机会话。

1. 安装并登录 ChatGPT 桌面版，或在终端执行 `codex login` 完成登录。
2. 登录信息由 Codex 保存在 `~/.codex/auth.json`；它等同于密码，请勿手动编辑、分享或提交。
3. Token 看板通过本机 Codex app-server 读取额度。OpenAI Platform API Key 与 ChatGPT/Codex 订阅额度是两套不同的计费体系，不能互换。

### KIMI

1. 安装 Kimi Code CLI 后执行 `kimi login`，按设备授权流程完成登录。
2. 登录成功后，新版 Kimi Code 会在 `~/.kimi-code/credentials/kimi-code.json` 保存访问凭据；旧版 Kimi CLI 的凭据位于 `~/.kimi/credentials/kimi-code.json`。
3. 看板优先读取新版 `~/.kimi-code` 凭据，读取或认证失败时再回退到旧版 `~/.kimi`；两处都无法获取有效登录信息时显示“未登录”。
4. access_token 有效期只有约 15 分钟，看板会在过期时自动用 refresh_token 换新并写回当前使用的凭据文件；不要复制、编辑或上传此文件。

### GLM（智谱）

1. 在 [智谱开放平台的 API Key 页面](https://open.bigmodel.cn/usercenter/proj-mgmt/apikeys) 创建 API Key。
2. 在看板设置页勾选 GLM 后，可直接在下方的 API Key 输入框中填入 Key（优先使用）。
3. 设置页留空时回退到 cc-switch：在 cc-switch 的 **Codex** 供应商配置中新增或编辑名称为 `Zhipu GLM` 的供应商，并填入 Key。
4. cc-switch 将配置保存在 `~/.cc-switch/cc-switch.db` 的 `providers` 表中；本项目读取 `settings_config.auth.OPENAI_API_KEY`。请通过 cc-switch 修改，不要直接编辑 SQLite 数据库。

### DeepSeek

1. 在 [DeepSeek Platform API Keys](https://platform.deepseek.com/api_keys) 创建 API Key。
2. 在看板设置页勾选 DeepSeek 后，可直接在下方的 API Key 输入框中填入 Key（优先使用）。
3. 设置页留空时回退到 cc-switch：在 cc-switch 的 **Codex** 供应商配置中新增或编辑名称为 `DeepSeek` 的供应商，并填入 Key。
4. 位置同样是 `~/.cc-switch/cc-switch.db` 的 `providers` 表及 `settings_config.auth.OPENAI_API_KEY` 字段；请通过 cc-switch 管理。

未配置、未登录或凭据失效的服务会在看板中明确显示状态，不会回显密钥。

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
src-tauri/target/release/bundle/macos/Token 看板.app
```

应用未签名或公证。首次打开被 macOS 阻止时，可在“系统设置 → 隐私与安全性”中选择允许打开，或解压后执行 `xattr -dr com.apple.quarantine "/Applications/Token 看板.app"` 直接放行。

## 自动更新

从 v0.2.10 起内置自动更新（默认开启，可在设置页关闭「自动更新」）。开启后应用启动时及每 2 小时检查一次 GitHub 最新 Release 的 `latest.json`，发现新版本会在后台下载并安装，完成后自动重启看板；关闭后不会再自动检查或安装，但右键菜单的「检查更新」仍可随时手动升级。更新包使用 minisign 签名（公钥内置在 `tauri.conf.json`），CI 发布时用仓库 secret `TAURI_SIGNING_PRIVATE_KEY` 签名。

本地打包如需生成更新产物，先导出私钥：

```bash
export TAURI_SIGNING_PRIVATE_KEY=$(cat ~/.tauri/token-board.key)
export TAURI_SIGNING_PRIVATE_KEY_PASSWORD=""
npm run tauri build
```

私钥文件位于 `~/.tauri/token-board.key`（未纳入 Git）；丢失私钥将无法再签发可被旧版本接受的更新包。

## 发布新版本

发布已全自动（`.github/workflows/release.yml`）：提交代码改动并 push 到 `main` 即可，无需手动改版本号、无需本地钩子。

- CI 每次在 main 检测到新提交时，自动把最新 tag 的 patch 版本 +1，由机器人提交 `chore: bump version` 回写 main（同步 `package.json`、`package-lock.json`、`tauri.conf.json`、`Cargo.toml`、`Cargo.lock`）。
- 随后自动完成 macOS 构建、ad-hoc 签名、压缩，并创建 tag 与 Release（`TokenBoard-v<版本>-macos.zip`、updater 更新包 `TokenBoard-v<版本>-macos.app.tar.gz` 及清单 `latest.json`，标记为 Latest）。
- 发版后自动清理旧 Release：只保留最近 2 个，更早的连同 tag 一起删除。
- 机器人提交不会再次触发发布，不会循环；提交信息中包含 `[skip release]` 可跳过本次自动发布。

## 说明

Codex 额度读取依赖本机 Codex 的 app-server 接口；该接口当前仍属于 experimental，未来 Codex 更新若改变接口，看板会显示“读取失败”，其余服务不受影响。
