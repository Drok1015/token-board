# 汇兑小猪

![汇兑小猪运行截图](public/screenshot.png)

仅支持 macOS 的桌面悬浮宠物。点击小猪左键，头顶会弹出汇兑、人人视频和 Parallels Desktop 图标；点击图标后，原生调用 macOS `open -a` 打开对应应用。

## 功能

- 始终置顶的透明桌面小猪
- 短按展开应用快捷图标，直接拖动改变位置
- 汇兑、人人视频 for Mac、Parallels Desktop 三个原生启动入口

## 安装

从 Release 下载并解压 `HuiDuiPet-v0.1.0-macos.zip`，将“汇兑小猪.app”拖入“应用程序”。

该版本未签名和公证；如果 macOS 阻止打开，请在“系统设置 → 隐私与安全性”中允许打开。

## 开发

```bash
npm install
npm run tauri dev
```

## 打包

```bash
npm run tauri build
```

短按小猪显示快捷入口；按住后直接移动即可拖动，不需要等待。图标区域始终使用固定的透明辅助空间，因此展开和收起时小猪位置不会发生位移。窗口始终置顶、无标题栏、不出现在 Dock。

小猪素材为本项目生成的原创形象，不复刻任何已有角色。
