# ezz

A very light wrapper around [7-Zip](https://7-zip.org/), only supporting one-click extraction

## ⭐ Features

- 开箱即用，无多余操作
- 一键无感运行，完成后显示桌面通知
- 支持 7-Zip 的所有压缩格式，以及 [隐写者](https://github.com/cenglin123/SteganographierGUI) 和 [apate](https://github.com/rippod/apate) 格式
- 提取至当前目录，自动整理 [目录结构](#关于目录结构)，并清理压缩包
- 跨平台，支持 x86_64 架构 Windows 和 Linux

<img src="./assets/whatever.jpg" alt="我管你这的那的" width="60%" />

## 💡 Usage

完整组件包括：

1. 主程序 `ezz.exe`（如无说明，下文中均指该程序）
2. 密码库文件 `.ezz.pw`，未指定路径时将依次在程序目录和用户家目录下寻找
3. 日志文件保存在程序目录下的 `ezz.log`

### 解手模式

右键点击待处理的文件，选择用本程序打开即可，配合 [Custom Context Menu](https://github.com/ikas-mc/ContextMenuForWindows11) 效果更佳。由于某些技术问题，仅支持同时运行一个实例。

该模式使用默认密码库中的密码，若无匹配项则会弹出密码输入框（仅 Windows 平台）

- 密码库的第一行为缓存，包含了最近使用过的密码的行号
- 其后的每一行表示一个密码条目
- 密码条目由 `频率`、`分隔符` 和 `密码` 三部分组成
  1. `频率` 为该密码被使用的次数，由程序自动统计并排序
  2. `分隔符` 为**英文逗号**
  3. `密码` 为一串字符

密码库示例如下：

```txt
4 2 3
23,Ao82s9jNk
12,6$hu!,4
9,i5l.6?rt07
0,klsidu9
```

若要给密码库添加新密码，只需在文件末尾添加一行，注意此时 `频率` 应该为 0

也可在命令行中添加密码：

```sh
ezz a <PASSWORD>
```

### 终端模式

程序包含两个子命令：`extract` 和 `add`，分别用于提取压缩文件和向密码库中添加密码

如果不指定子命令，默认会将传入的参数作为压缩文件路径执行 `extract`

参数说明如下：

```sh
Usage: ezz [FILE] [COMMAND]

Commands:
  extract  e[X]tract an archive
  add      [A]dd a password to the wordlist
  help     Print this message or the help of the given subcommand(s)

Arguments:
  [FILE]  path to input file (when no subcommand is given, extract it)

Options:
  -h, --help     Print help
  -V, --version  Print version

# 子命令 extract (x)
Usage: ezz extract [OPTIONS] <FILE>

Arguments:
  <FILE>  path to input file

Options:
  -p, --password <PASSWORD>  specify password
      --wordlist <FILE>      path to password wordlist
  -h, --help                 Print help
  -V, --version              Print version

# 子命令 add (a)
Usage: ezz add [OPTIONS] <PASSWORD>

Arguments:
  <PASSWORD>  password to add

Options:
      --wordlist <FILE>    path to password wordlist
  -h, --help               Print help
  -V, --version            Print version
```

由于 Windows 平台的模式设为了桌面程序（不会弹出终端窗口），导致其在终端不会有输出，包括 `--help` 和 `--version`，但程序可以正常接受参数并运行

## 🔔 Notice

### 关于分卷压缩包

本程序支持标准风格的分卷：

- 形如 `.001`、`.002`、`.003` 的分卷（一般由 7-Zip 生成）
- 形如 `.part1.rar`、`.part2.rar` 的分卷
- 形如 `.zip`、`.z01`、`.z02` 的分卷

使用时请打开第一个分卷（但 zip 是最后一个），即 `.001`、`.part1.rar`、**`.zip`**，否则无法完全清理分卷文件

### 关于目录结构

- 若压缩包中只包含 1 个文件（夹），则直接提取至当前目录
- 否则将提取至与压缩包同名的文件夹中，并排除重复的根目录

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
