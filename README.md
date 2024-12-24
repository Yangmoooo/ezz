# ezz

A very light wrapper around [7-Zip](https://7-zip.org/), only supporting one-click extraction

## ⭐ Features

- 开箱即用，无多余操作
- 一键无感运行，完成后显示桌面通知
- 支持几乎所有的压缩格式，以及 [隐写者](https://github.com/cenglin123/SteganographierGUI) 文件
- 提取至当前目录，自动整理 [目录结构](#关于目录结构)，并清理压缩包
- 跨平台，支持 x86_64 架构 Windows 和 Linux

<img src="./assets/whatever.jpg" alt="我管你这的那的" width="60%" />

## 💡 Usage

完整组件包括：

1. 可执行文件 `ezz.exe`（Linux 上为 `ezz`）
2. 密码库文件 `ezz.db.txt`，未指定路径时将依次在程序目录和用户家目录下寻找
3. 日志文件保存在程序目录下的 `ezz.log`（会自动创建）

### 解手模式

右键点击待处理的文件，选择用本程序打开即可，配合 [Custom Context Menu](https://github.com/ikas-mc/ContextMenuForWindows11) 效果更佳

该模式使用默认密码库中的密码，需要先配置密码库

- 密码库中每行表示一个密码条目
- 一行由 `频率`、`分隔符` 和 `密码` 三部分组成
  1. `频率` 为该密码被使用的次数，由程序自动统计并排序
  2. `分隔符` 为**英文逗号**
  3. `密码` 为一串字符

密码库示例如下：

```txt
23,Ao82s9jNk
12,6$hu!,4
9,i5l.6?rt07
```

若要给密码库添加新密码，只需在文件末尾添加一行，注意此时 `频率` 应该为 0

也可在命令行中添加密码：

```pwsh
ac DB_PATH "0,password"
```

```sh
echo "0,password" >> DB_PATH
```

### 终端模式

由于 Windows 平台的模式设为了桌面程序（不会弹出终端窗口），导致其在终端不会有输出，包括 `--help` 和 `--version`，但程序可以接受参数并正确运行，参数如下：

```pwsh
Usage: ezz.exe [OPTIONS] <FILE>

Arguments:
  <FILE>  指定输入文件路径

Options:
  -p, --pw <PASSWORD>  指定密码
  -d, --db <FILE>      指定密码库路径
  -h, --help           Print help
  -V, --version        Print version
```

## 🔔 Notice

### 关于目录结构

- 若压缩包中只包含 1 个文件（夹），则直接提取至当前目录
- 否则将提取至与压缩包同名的文件夹中，并排除重复的根目录

### 关于分卷压缩包

本程序支持标准风格的分卷：

- 形如 `.001`、`.002`、`.003` 的分卷（一般由 7-Zip 生成）
- 形如 `.part1.rar`、`.part2.rar` 的分卷
- 形如 `.zip`、`.z01`、`.z02` 的分卷

使用时请打开第一个分卷（但 zip 是最后一个），即 `.001`、`.part1.rar`、**`.zip`**，否则无法完全清理分卷文件

### 关于 Custom Context Menu

作为一个 Portable App，本程序不会添加至 Windows 右键菜单

但可以通过 [Custom Context Menu](https://github.com/ikas-mc/ContextMenuForWindows11) 来实现。具体用法请参考其 [Wiki](https://github.com/ikas-mc/ContextMenuForWindows11/wiki/Help)，或直接导入自用 [配置文件](./assets/用%20ezz%20提取.json)，然后修改其中 `ezz` 的路径即可

请注意，尽管 Custom Context Menu 提供了选中多个文件后批量操作的功能，但本程序并不支持。如果将其 Match Files 设为 Each 模式，**似乎**能够工作（会出现错误通知），但不建议这样做

## ❤️ Thanks

- 感谢 [7-Zip](https://www.7-zip.org/) 提供了强大的开源压缩工具
- 感谢 [@cenglin123](https://github.com/cenglin123) 为探索可行的网盘保存方式所做出的大量实践和考证

## 📄 License

7-Zip 的许可证构成较为复杂，详见附件 [Lisence1](./assets/License1.txt) 和 [Lisence2](./assets/License2.txt)

其主要的许可证是 [LGPL](https://www.gnu.org/licenses/lgpl-2.1.html)，而在本项目中：

- Windows 版封装了通过 7-Zip [仓库](https://github.com/ip7z/7zip) 编译的 `7zz.exe`
- Linux 版封装了 7-Zip [官网](https://7-zip.org/) 分发的 `7zz`

因此本项目也遵循 LGPL 许可
