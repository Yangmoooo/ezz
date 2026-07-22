# ezz

ezz 是一个无主窗口的桌面解压工具。它从 Finder 或 Windows 资源管理器接收文件，使用随应用发布的固定版本 7-Zip，依次完成格式识别、密码尝试、事务式解压、目录整理和原归档清理。

v3 不提供命令行接口、主窗口、任务列表或持久化设置。

## 支持平台

| 平台 | 架构 | 最低版本 | 发布格式 |
| --- | --- | --- | --- |
| macOS | Apple Silicon (`arm64`) | macOS 11 | ZIP 中的 `ezz.app` |
| Windows | x64 | Windows 10 | Portable ZIP |

Linux、macOS Intel、Windows ARM 和 macOS 10.x 不属于 v3 支持范围。v2 的 tag 和历史发布会保留，但不再维护。

## 主要能力

- 通过内容而非文件扩展名识别 7-Zip 支持的归档，修改过后缀的归档也可通过文件选择器打开。
- 支持 Steganographier 生成的 MP4/MKV；普通视频只读探测后会被拒绝，不会产生输出或清理源文件。
- 支持一次打开多个文件并严格顺序处理，单个失败不会中断后续文件。
- 可从任意数字分卷、`.partN.rar` 或 `.zNN` 分卷开始，自动定位首卷并在成功后清理完整分卷集合。
- 支持无密码、内容加密和文件名加密归档，并可在原生密码弹窗中重试。
- 只在归档旁的隐藏临时目录中解压；验证完整结果后才提交，不覆盖或合并已有文件。
- 成功后将原归档移入废纸篓或回收站，绝不永久删除。清理失败只产生警告，不撤销已提交结果。

## 安装

### macOS

1. 下载 `ezz-<版本>-macos-arm64.zip` 并解压。
2. 将 `ezz.app` 移到“应用程序”目录。
3. 首次运行时在 Finder 中右键点击 `ezz.app`，选择“打开”，再确认打开。

首发版本使用 ad-hoc 签名，没有 Apple Developer ID 签名和公证。如果右键打开仍被拦截，可在“系统设置 > 隐私与安全性”中选择“仍要打开”。最后的手动方案是：

```sh
xattr -dr com.apple.quarantine /Applications/ezz.app
```

放行只需要完成一次。请只对从本项目 GitHub Release 下载并自行确认来源的应用执行该命令。

### Windows

1. 下载并完整解压 `ezz-<版本>-windows-x64.zip`。
2. 保持 `ezz.exe` 与 `7zz.exe` 位于同一目录。
3. 用 `ezz.exe` 打开归档，或直接启动 `ezz.exe` 后选择文件。

ezz 不提供安装器，也不会修改注册表或抢占默认文件关联。需要右键菜单时，可自行使用 [Custom Context Menu](https://github.com/ikas-mc/ContextMenuForWindows11) 等工具配置“用 ezz 打开”。

## 使用方式

- 在 Finder 或 Windows 资源管理器中选择一个或多个文件并用 ezz 打开。
- 直接启动 ezz 时会显示允许多选、允许选择任意文件的系统文件选择器。
- macOS 注册常见压缩扩展名以及 Steganographier 的 `mp4`、`mkv`；未注册或修改过后缀的文件请通过文件选择器打开。
- 队列完成后会显示汇总通知并退出，程序不会常驻后台。

当空密码和已保存密码都失败时，密码弹窗会显示：

- `Remember this password`：默认勾选，仅在完整解压成功后保存密码。
- `Keep the original archive`：默认不勾选，只影响当前归档及其分卷。

密码错误时可以继续重试；取消只会让当前文件失败，批处理仍会继续。

## 输出与冲突

- 只有一个有效顶层文件或目录时，直接提交该项。
- 有多个顶层项时，提交到以逻辑归档名命名的目录。
- 顶层 `.DS_Store` 和 `__MACOSX` 会被丢弃，其他隐藏文件会保留。
- 文件冲突使用 `name (1).ext`，目录冲突使用 `name (1)`；不会覆盖或合并现有内容。
- 普通归档只解压一层，不会递归解压其中的内层归档。

## 分卷归档

可以打开分卷集合中的任意一卷：

- 数字分卷：`.001`、`.002`、`.003` 等。
- RAR 分卷：`.part1.rar`、`.part2.rar` 等，也支持带前导零的编号。
- ZIP 分卷：`.z01`、`.z02` 等，自动定位对应的 `.zip`。

缺少首卷或中间卷时，当前输入会失败并保留全部分卷。只有完整解压和提交成功后，确认属于该集合的所有分卷才会一起移入废纸篓或回收站。

## 数据位置

ezz 没有设置文件，也不读取或迁移 v2 的 `.ezz.pw` 和程序目录日志。

| 数据 | macOS | Windows |
| --- | --- | --- |
| 密码库 | `~/Library/Application Support/ezz/passwords.json` | `%APPDATA%\ezz\passwords.json` |
| 日志 | `~/Library/Logs/ezz/ezz.log` | `%LOCALAPPDATA%\ezz\logs\ezz.log` |

密码库是仅当前用户可访问的结构化明文文件，不使用 Keychain 或 Windows Credential Manager。日志不会记录密码或完整的 7-Zip 密码参数。

## 构建与测试

普通 `cargo build` 不访问网络，也不会自动下载 7-Zip。首次开发或运行真实端到端测试前执行：

```sh
cargo xtask prepare
```

该命令下载固定的 7zz-bin 26.02 平台资产、校验 SHA-256，并缓存到 `target/ezz-tools/26.02/`。需要代理时只对当前命令设置环境变量即可：

```sh
HTTPS_PROXY=http://127.0.0.1:PORT \
HTTP_PROXY=http://127.0.0.1:PORT \
cargo xtask prepare
```

本机验证命令：

```sh
cargo fmt --all -- --check
cargo test --workspace --all-targets
cargo test --lib -- --ignored
cargo clippy --workspace --all-targets -- -D warnings
```

生成当前平台发布物：

```sh
cargo xtask package
```

输出位于 `target/dist/`。macOS 打包会生成 plist、裁剪 arm64 7zz、依次 ad-hoc 签名 7zz 和应用包并验证签名；Windows 打包会生成包含完整运行文件和许可证的 Portable ZIP。

## 许可证

ezz 使用 LGPL-2.1-or-later。发布物同时包含 7-Zip、unRAR 相关许可证原文；详情见 [`assets/7zip`](./assets/7zip)。

感谢 [7-Zip](https://7-zip.org/) 提供解压引擎，以及 [Steganographier](https://github.com/cenglin123/SteganographierGUI) 对特殊视频封装格式的探索。
