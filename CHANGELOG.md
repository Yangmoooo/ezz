# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/zh-CN/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- 在密码库中未找到密码时，弹窗提示输入密码

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
