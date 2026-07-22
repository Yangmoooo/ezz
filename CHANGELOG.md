# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/zh-CN/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- 新增 macOS 11+ Apple Silicon 原生 AppKit 桌面支持、文件关联、文件选择器和 ad-hoc 签名应用包
- 新增 Windows 10/11 x64 原生桌面适配、后续实例路径转发和 Portable ZIP
- 支持一次打开多个输入并严格顺序处理，单个失败不再阻塞后续文件
- 新增事务式解压、路径安全验证、无冲突提交和结构化成功警告
- 支持从任意数字分卷、RAR 分卷或 ZIP 分卷自动定位首卷并清理完整集合
- 支持通过内容识别 Steganographier MP4/MKV，同时拒绝普通视频
- 新增原生密码重试弹窗、结构化明文密码库和最近成功密码复用
- 新增固定 7zz-bin 26.02 版本、SHA-256 校验以及 `cargo xtask prepare/package`
- 新增 macOS arm64 与 Windows x64 的真实 7-Zip CI 测试和发布物构建

### Changed

- Cargo 包名、程序名和显示名统一为 `ezz`，版本升级为 3.0.0
- 普通归档改为按内容探测，不再依赖扩展名判断是否支持
- 原归档只在完整结果提交后移入系统废纸篓或回收站，清理失败不再使解压结果失败
- 密码库和日志迁移到平台标准用户数据目录，且不自动读取或迁移 v2 数据
- macOS 和 Windows 使用各自原生桌面交互，核心解压行为由共享 Rust library 提供

### Removed

- 移除 Linux、macOS Intel、Windows ARM 和 macOS 10.x 支持
- 移除 `add`、`extract` 子命令以及所有正式 CLI 契约
- 移除主窗口、持久化设置和后台常驻能力
- 移除构建脚本自动联网下载 7-Zip 的行为

### Security

- 7-Zip 只能向归档旁的随机隐藏工作目录解压，失败时不提交部分结果或清理源归档
- 拒绝逃逸路径、绝对路径、设备文件、FIFO 和指向结果目录外部的符号链接
- 密码库采用原子写入并设置严格文件权限，日志不记录密码

## [2.0.6] - 2025-11-08

### Changed

- 替换了下载到 `assets/` 的 7zz 二进制文件，改为使用由 [7zz-bin](https://github.com/Yangmoooo/7zz-bin) 提供的自动构建版本
- Linux 平台改为 musl 版本，内置的 `7zz` 也使用了静态编译的 `7zzs`
- 修改了 package 名称，`easy_unzip` -> `easy-unzip`

### Removed

- 移除了 `assets/` 中的 `7zz.exe` 和 `7zz`

## [2.0.5] - 2025-08-22

### Changed

- 密码库由 `ezz.vault` 变更为 `.ezz.pw`，需手动修改文件名
- 调整了 Windows 平台的密码输入弹窗，Toast 通知变更为英文
- 完善了 `ezz.exe` 在 Windows 上的程序信息

## [2.0.4] - 2025-08-15

### Changed

- 更新 7zip 到 25.01

### Removed

- 清除有关 `aletheia` 的内容

## [2.0.3] - 2025-07-28

### Changed

- 更新 7zip 到 25.00

## [2.0.2] - 2025-04-09

### Changed

- 内嵌的 7-Zip 不再释放到程序目录，而是释放到系统临时目录
- 提取出的文件（夹）名在冲突时会重命名现有文件（夹），而非直接覆盖

### Security

- 引入独占机制，避免多个 `ezz` 实例同时运行导致并发冲突

## [2.0.1] - 2025-03-30

### Changed

- `aletheia` 不再修改目标文件后缀名，后续应该不需要更新 `aletheia` 了

### Fixed

- 修复了密码库的缓存密码未正确更新的问题

## [2.0.0] - 2025-03-29

### Added

- 完全支持还原 `apate` 格式，但作为单独的 `aletheia` 二进制文件发布
*注意，`apate` 的处理是破坏性过程，故 `aletheia` 也是如此。请确保目标文件是由 `apate` 伪装过的，否则请及时备份。*

### Changed

- 清理解压完成的压缩包时，现在会将其移动至回收站而非直接删除
- 密码库格式变更，在开头添加了最近使用过的缓存密码
*请在旧版本的密码库开头添加一行 `0 0 0`，否则将丢失第一个密码项*

### Fixed

- 修复了由压缩包生成的文件夹名称不准确的问题
- 修复了解压无密码的压缩包时会将次数统计到第一个密码的问题

## [1.3.0] - 2025-03-23

### Added

- 添加了 [apate](https://github.com/rippod/apate) 的默认格式（一键伪装）支持

### Changed

- 在日志中记录日期信息，格式为 `YYYY-MM-DD HH:mm:ss`
- 测试密码时只会选择压缩包中的一个文件，显著缩短了处理时间（但对单个大文件仍无能为力）
- 用方法重构了大部分函数，提升可读性

## [1.2.0] - 2025-02-14

### Added

- 在密码库中未找到密码时，弹窗提示输入密码
- 新增 `add` 子命令，用于向密码库中添加密码

### Changed

- 原有的命令行参数移动至 `extract` 子命令中
- 若不使用子命令，现在仅接受一个参数作为压缩文件路径进行提取

### Fixed

- 修复了当压缩包中文件较多时解压耗时过长的问题

### Deprecated

- 由于我不再使用 KDE，因此后续（包括本次）的功能更新将不再支持 Linux 桌面环境

## [1.1.2] - 2024-12-24

### Changed

- 现在仅当压缩包内只含一个文件时，才直接提取至当前文件夹
- 桌面通知在完成时会展示提取出的文件（夹）名，而非原压缩包名

### Fixed

- 修复了 zip 分卷的残余（`.zip`、`.z01`、`.z02`...）文件未被清理的问题
- 修复了提取隐写文件后，桌面通知的文件名为 `2.zip` 的问题
- 修复了在 Windows 上当压缩包名以空格结尾时，创建的解压文件夹无法访问的问题（这是由于 NTFS 不允许文件名以空格结尾，但强行创建了该文件夹导致的）

## [1.1.1] - 2024-12-06

### Added

- 给 Windows 上的程序添加了图标

### Changed

- 更新内置的 7-Zip 版本至 24.09

### Fixed

- 修复了在提取 7z 和 rar 分卷压缩包时不会清理剩余分卷的问题

## [1.1.0] - 2024-11-22

### Added

- 正式添加对 Linux 平台（KDE）的支持
