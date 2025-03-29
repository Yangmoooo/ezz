# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/zh-CN/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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
