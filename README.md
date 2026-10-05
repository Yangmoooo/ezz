# Ezz

Ezz 是一个轻量解压工具，它封装了 7-Zip 作为引擎，自动尝试密码、事务式解压，并整理目录和清理归档

## 主要能力

- 自学习解压密码，使用过的密码会被记录并在后续自动尝试
- 通过内容识别 7-Zip 支持的归档，修改过后缀的归档也可通过文件选择器打开
- 支持 Steganographier 生成的 MP4/MKV；普通视频只读探测后会被拒绝
- 支持常见加密、分卷归档
- 默认将原归档移入废纸篓或回收站

## 安装

### macOS

1. 下载 `ezz-macos-arm64.dmg` 并打开。
2. 把 `Ezz.app` 拖进“应用程序”目录（可以直接拖到 DMG 里那个 `/Applications` 链接上）。
3. 首次运行时在 Finder 中右键点击 `Ezz.app`，选择“打开”，再确认打开。

首发版本使用 ad-hoc 签名，没有 Apple Developer ID 签名和公证。如果右键打开仍被拦截，可在“系统设置 > 隐私与安全性”中选择“仍要打开”。最后的手动方案是：

```sh
xattr -dr com.apple.quarantine /Applications/Ezz.app
```

放行只需要完成一次。请只对从本项目 GitHub Release 下载并自行确认来源的应用执行该命令。

### Windows

1. 下载并完整解压 `ezz-windows-x64.zip`
2. 保持 `ezz.exe` 与 `7zz.exe` 位于同一目录
3. 用 `ezz.exe` 打开归档，或直接启动 `ezz.exe` 后选择文件

Ezz 不提供安装器，也不会修改注册表或抢占默认文件关联。需要右键菜单时，可自行使用 [Custom Context Menu](https://github.com/ikas-mc/ContextMenuForWindows11) 等工具；仓库里的 [`assets/用 Ezz 提取.json`](./assets/用%20Ezz%20提取.json) 是一份可直接导入的配置，导入前请把其中的 `ezz.exe` 路径改为实际位置

## 使用方式

- 在 Finder 或 Windows 资源管理器中选择文件并用 Ezz 打开
- 直接启动 Ezz 时会显示系统文件选择器
- 归档处理完成后会显示通知并报告最终路径

当空密码和已保存密码都失败时，密码弹窗会显示：

- `Remember this password`：默认勾选，仅在完整解压成功后保存密码
- `Keep the original archive`：默认不勾选，只影响当前归档及其分卷

密码错误时可以继续重试；取消会让当前文件失败

## 输出与冲突

- 只有一个有效顶层文件或目录时，直接提交该项
- 有多个顶层项时，提交到以逻辑归档名命名的目录
- 顶层 `.DS_Store` 和 `__MACOSX` 会被丢弃，其他隐藏文件会保留
- 文件冲突使用 `name (1).ext`，目录冲突使用 `name (1)`；不会覆盖或合并现有内容
- 普通归档只解压一层，不会递归解压其中的内层归档

## 分卷归档

可以打开分卷集合中的任意一卷：

- 数字分卷：`.001`、`.002`、`.003` 等
- RAR 分卷：`.part1.rar`、`.part2.rar` 等，也支持带前导零的编号
- ZIP 分卷：`.z01`、`.z02` 等，自动定位对应的 `.zip`

缺少首卷或中间卷时，当前输入会失败并保留全部分卷。只有完整解压和提交成功后，确认属于该集合的所有分卷才会一起移入废纸篓或回收站

## 数据位置

Ezz 没有设置文件。密码库与日志放在同一个应用数据目录：

| 数据 | macOS | Windows |
| --- | --- | --- |
| 密码库 | `~/Library/Application Support/ezz/passwords.json` | `%LOCALAPPDATA%\ezz\passwords.json` |
| 日志 | `~/Library/Application Support/ezz/ezz.log` | `%LOCALAPPDATA%\ezz\ezz.log` |

### 密码库格式

```json
{
  "version": 1,
  "passwords": [
    "short-form-password",
    { "password": "hunter2", "uses": 3, "last_used": 1784786364 }
  ]
}
```

- `passwords` 的元素可以是字符串（等价于 `uses` 与 `last_used` 为 0），也可以是对象
- `version` 缺失时按 `1` 处理；`uses` 与 `last_used` 缺失时按 `0` 处理；未知字段忽略。手工编辑后无需保持排序
- 文件无法读取时 Ezz 会忽略它并继续允许手动输入密码，不会让提取失败；下一次成功保存前会把原文件改名为 `passwords.json.corrupt-<时间戳>` 保留

## 构建与测试

普通 `cargo build` 不访问网络，也不会自动下载 7-Zip。常用命令都写在 [`justfile`](./justfile) 里（直接运行 `just` 会列出全部）：

```sh
just prepare      # 下载并校验固定版本的 7-Zip 引擎，首次开发前执行一次
just test         # 单元测试与契约测试
just test-ignored # 需要真实 7-Zip 的端到端测试，以及 Windows 对话框测试
just verify       # 提交前跑这个：格式、clippy、全部测试
```

### 发布物

```sh
just package      # 等价于 cargo xtask package
```

输出位于 `target/dist/`，文件名不含版本号：

| 平台 | 产物 | 内容 |
| --- | --- | --- |
| Windows | `ezz-windows-x64.zip` | `ezz-windows-x64/`：`ezz.exe`、`7zz.exe`、原样拷贝的 `README.md` 与 `CHANGELOG.md`、`licenses/` |
| macOS | `ezz-macos-arm64.dmg` | 卷标 `Ezz`：`Ezz.app`（内含 `7zz`、图标与 `licenses/`）、原样拷贝的 `README.md` 与 `CHANGELOG.md`、指向 `/Applications` 的符号链接 |

版本只有一个来源：`Cargo.toml` 的 `package.version`（Windows 写在 exe 的 `VERSIONINFO` 里，macOS 写在 `Info.plist` 里）

## 许可证

Ezz 使用 LGPL-2.1-or-later。发布物同时包含 7-Zip、unRAR 相关许可证原文；详情见 [`assets/7zip`](./assets/7zip)

感谢 [7-Zip](https://7-zip.org/) 提供解压引擎，以及 [Steganographier](https://github.com/cenglin123/SteganographierGUI) 对特殊视频封装格式的探索
