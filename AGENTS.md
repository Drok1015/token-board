# Token 看板

macOS 桌面悬浮 Token/额度看板（Tauri 2 + Vite），展示 CODEX、KIMI、GLM、DEEPSEEK 的可用额度或余额。功能与凭据配置详见 README.md。

## 常用命令

- `npm run tauri dev` 本地运行调试
- `npm run tauri build` 本地打包（产物在 `src-tauri/target/release/bundle/macos/`）
- `cd src-tauri && cargo test` 运行 Rust 测试

## 发布流程（全自动）

- 代码改动 push 到 `main` 即自动发版：CI（`.github/workflows/release.yml`）把最新 tag 的 patch 版本 +1，由机器人提交 `chore: bump version` 回写 main（同步 5 个版本文件），随后完成 macOS 构建、ad-hoc 签名并发布 Release（`TokenBoard-v<版本>-macos.zip` 及 updater 更新包 `.app.tar.gz` + `latest.json`，Latest）。发版后自动清理旧 Release：只保留最近 2 个，更早的连同 tag 一起删除。
- 自动更新基于 tauri-plugin-updater：应用启动及每 6 小时读取 `releases/latest/download/latest.json`，有新版即后台安装并重启。签名私钥存于仓库 secret `TAURI_SIGNING_PRIVATE_KEY`（本地备份 `~/.tauri/token-board.key`，未入 Git），公钥内置 `tauri.conf.json`；本地打包需同时导出 `TAURI_SIGNING_PRIVATE_KEY` 和 `TAURI_SIGNING_PRIVATE_KEY_PASSWORD=""`。
- 提交信息包含 `[skip release]` 可跳过本次自动发布（纯文档/配置改动时使用）。
- 不要手动修改任何文件里的版本号；版本以 Releases 最新 tag 为准。

## AI 助手协作约定

- 每次 push 到 main 后：在后台用 `gh run watch` 盯本次 workflow 运行结束，然后 `git pull --ff-only` 同步机器人的 bump 提交，最后把新 Release 的链接汇报给用户。若 run 失败，查看日志（`gh run view --log`）并汇报原因。
- 本仓库已配置 `http.proxy=http://127.0.0.1:7897` 应对直连 GitHub 不通的情况；若代理也不通，先检查网络再重试，不要修改该配置。
