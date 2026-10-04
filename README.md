# Ezz

Ezz 是一个无主窗口的桌面解压工具。它从 Finder 或 Windows 资源管理器接收文件，使用随应用发布的固定版本 7-Zip，依次完成格式识别、密码尝试、事务式解压、目录整理和原归档清理。

v3 不提供命令行接口、主窗口、任务列表或持久化设置。

## 支持平台

| 平台 | 架构 | 最低版本 | 发布格式 |
| --- | --- | --- | --- |
| macOS | Apple Silicon (`arm64`) | macOS 11 | DMG 中的 `ezz.app` |
| Windows | x64 | Windows 10 | Portable ZIP |

Linux、macOS Intel、Windows ARM 和 macOS 10.x 不属于 v3 支持范围。v2 的 tag 和历史发布会保留，但不再维护。

## 主要能力

- 通过内容而非文件扩展名识别 7-Zip 支持的归档，修改过后缀的归档也可通过文件选择器打开。
- 支持 Steganographier 生成的 MP4/MKV；普通视频只读探测后会被拒绝，不会产生输出或清理源文件。
- 可从任意数字分卷、`.partN.rar` 或 `.zNN` 分卷开始，自动定位首卷并在成功后清理完整分卷集合。
- 支持无密码、内容加密和文件名加密归档，并可在原生密码弹窗中重试。
- 只在归档旁的隐藏临时目录中解压；验证完整结果后才提交，不覆盖或合并已有文件。
- 成功后将原归档移入废纸篓或回收站，绝不永久删除。清理失败只产生警告，不撤销已提交结果。

## 安装

### macOS

1. 下载 `ezz-macos-arm64.dmg` 并打开。
2. 把 `ezz.app` 拖进“应用程序”目录（可以直接拖到 DMG 里那个 `/Applications` 链接上）。
3. 首次运行时在 Finder 中右键点击 `ezz.app`，选择“打开”，再确认打开。

首发版本使用 ad-hoc 签名，没有 Apple Developer ID 签名和公证。如果右键打开仍被拦截，可在“系统设置 > 隐私与安全性”中选择“仍要打开”。最后的手动方案是：

```sh
xattr -dr com.apple.quarantine /Applications/ezz.app
```

放行只需要完成一次。请只对从本项目 GitHub Release 下载并自行确认来源的应用执行该命令。

### Windows

1. 下载并完整解压 `ezz-windows-x64.zip`。
2. 保持 `ezz.exe` 与 `7zz.exe` 位于同一目录。
3. 用 `ezz.exe` 打开归档，或直接启动 `ezz.exe` 后选择文件。

Ezz 不提供安装器，也不会修改注册表或抢占默认文件关联。需要右键菜单时，可自行使用 [Custom Context Menu](https://github.com/ikas-mc/ContextMenuForWindows11) 等工具；仓库里的 [`assets/用 ezz 提取.json`](./assets/用%20ezz%20提取.json) 是一份可直接导入的配置，导入前请把其中的 `exe` 与 `icon` 路径改成你解压后的 `ezz.exe`。

导入时建议把 `acceptMultipleFilesFlag` 设为 `1`（即一次把选中的全部路径交给同一个 Ezz 进程）。保持 `0` 时每个文件会各起一个进程，只有第一个能提取，其余的会被跳过并各自弹出一条通知——需要重新提取一次。

## 使用方式

- 在 Finder 或 Windows 资源管理器中选择文件并用 Ezz 打开。
- 直接启动 Ezz 时会显示允许多选、允许选择任意文件的系统文件选择器。
- macOS 注册常见压缩扩展名以及 Steganographier 的 `mp4`、`mkv`；未注册或修改过后缀的文件请通过文件选择器打开。
- 每个文件处理完成后都会显示一条通知并报告最终路径（含警告数量）；全部完成后程序退出，不会常驻后台。

当空密码和已保存密码都失败时，密码弹窗会显示：

- `Remember this password`：默认勾选，仅在完整解压成功后保存密码。
- `Keep the original archive`：默认不勾选，只影响当前归档及其分卷。

密码错误时可以继续重试；取消只会让当前文件失败。

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

Ezz 没有设置文件。密码库与日志放在同一个应用数据目录：

| 数据 | macOS | Windows |
| --- | --- | --- |
| 密码库 | `~/Library/Application Support/ezz/passwords.json` | `%LOCALAPPDATA%\ezz\passwords.json` |
| 日志 | `~/Library/Application Support/ezz/ezz.log` | `%LOCALAPPDATA%\ezz\ezz.log` |

密码库是仅当前用户可访问的结构化明文文件，不使用 Keychain 或 Windows Credential Manager。日志不会记录密码或完整的 7-Zip 密码参数。

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

- `passwords` 的元素可以是字符串（等价于 `uses` 与 `last_used` 为 0），也可以是对象。
- `version` 缺失时按 `1` 处理；`uses` 与 `last_used` 缺失时按 `0` 处理；未知字段忽略。手工编辑后无需保持排序。
- 文件读不出来时 Ezz 会忽略它并继续允许手动输入密码，不会让提取失败；下一次成功保存前会把原文件改名为 `passwords.json.corrupt-<时间戳>` 保留。

### 从 v2 迁移密码

v2 的 `.ezz.pw` 是文本文件（在旁边可执行文件同目录，或用户主目录）：首行是三个缓存行号，其余每行是 `<使用次数>,<密码>`。把每行转成 `passwords` 里的一个对象即可，例如：

```text
0 0 0
3,hunter2
1,correct horse
```

```json
{
  "version": 1,
  "passwords": [
    { "password": "hunter2", "uses": 3, "last_used": 0 },
    { "password": "correct horse", "uses": 1, "last_used": 0 }
  ]
}
```

密码里如果包含逗号，以**第一个**逗号为分隔（其余部分都属于密码）。

## 构建与测试

普通 `cargo build` 不访问网络，也不会自动下载 7-Zip。仓库里有一个 [`justfile`](./justfile)，常用命令都可以用 `just` 调用（直接运行 `just` 会列出全部配方）：

```sh
just prepare      # 首次开发前执行一次：下载并校验固定版本的 7-Zip 引擎
just test         # 单元测试与契约测试
just test-ignored # 需要真实 7-Zip 的端到端测试，以及 Windows 对话框测试
just verify       # 提交前跑这个：格式、两个平台的 clippy、全部测试
```

`just prepare` 下载固定的 7zz-bin 26.02 平台资产、校验 SHA-256，并缓存到 `target/ezz-tools/26.02/`。需要代理时只对当前命令设置环境变量即可：

```sh
HTTPS_PROXY=http://127.0.0.1:PORT \
HTTP_PROXY=http://127.0.0.1:PORT \
just prepare
```

不装 `just` 时，等价的原生命令是：

```sh
cargo xtask prepare
cargo fmt --all -- --check
cargo test --workspace --all-targets
cargo test --workspace --all-targets -- --ignored
cargo clippy --workspace --all-targets -- -D warnings
```

macOS 代码可以在 Windows 或 Linux 上做真实的编译与 lint 检查（只需先 `rustup target add aarch64-apple-darwin`）：

```sh
just check-mac
# 等价于：cargo check -p ezz --all-targets --target aarch64-apple-darwin
```

只检查 `ezz` 本体：`xtask` 依赖 `xz2`（liblzma）这类 C 代码，无法在非 macOS 主机上交叉编译，它由 macOS 上的 CI 原生构建。

### 发布物

```sh
just package      # 等价于 cargo xtask package
```

输出位于 `target/dist/`，文件名不含版本号：

| 平台 | 产物 | 内容 |
| --- | --- | --- |
| Windows | `ezz-windows-x64.zip` | `ezz-windows-x64/`：`ezz.exe`、`7zz.exe`、原样拷贝的 `README.md` 与 `CHANGELOG.md`、`licenses/` |
| macOS | `ezz-macos-arm64.dmg` | 卷标 `Ezz`：`ezz.app`（内含 `7zz`、图标与 `licenses/`）、原样拷贝的 `README.md` 与 `CHANGELOG.md`、指向 `/Applications` 的符号链接 |

版本只有一个来源：`Cargo.toml` 的 `package.version`。Windows 写在 exe 的 `VERSIONINFO` 里，macOS 写在 `Info.plist` 里。

macOS 打包会裁剪 arm64 的 `7zz`、先生成 plist、再依次 ad-hoc 签名 `7zz` 与应用包并验证签名，最后用 `hdiutil` 生成 DMG；Windows 打包会生成包含完整运行文件、文档与许可证的 Portable ZIP。

### 发布流程

1. 把 `CHANGELOG.md` 里的 `[Unreleased]` 改名为即将发布的版本号并写上日期。
2. 同步 `Cargo.toml` 的 `package.version`，提交。
3. 打 tag 并推送：`git tag v3.0.0 && git push origin v3.0.0`。

推 tag 会触发 `.github/workflows/release.yml`：两个平台各自打包并检查产物内容（DMG 会真实挂载校验布局与签名），然后在同一个 Release 里附上 `SHA256SUMS` 和从 CHANGELOG 解析出的发行说明。CI 在每次推送到 `main` 时也会走同一条打包路径，所以发布流程不会在发布当天才第一次运行。

## 许可证

Ezz 使用 LGPL-2.1-or-later。发布物同时包含 7-Zip、unRAR 相关许可证原文；详情见 [`assets/7zip`](./assets/7zip)。

感谢 [7-Zip](https://7-zip.org/) 提供解压引擎，以及 [Steganographier](https://github.com/cenglin123/SteganographierGUI) 对特殊视频封装格式的探索。
