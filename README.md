# ezz

A very light wrapper around [7-Zip](https://7-zip.org/), only supporting one-click extraction

封装了从 [7zip](https://github.com/ip7z/7zip) 项目中编译的可执行文件 `7zz` 用于解压

## ⭐ Features

- 开箱即用，无多余操作
- 一键无感运行，完成后显示桌面通知
- 支持几乎所有的压缩格式，以及[隐写者](https://github.com/cenglin123/SteganographierGUI)文件
- 跨平台，支持 x86_64 架构 Windows 和 Linux

## 💡 Usage

完整组件包括：

1. 可执行文件 `ezz.exe`（Linux 上为 `ezz`）
2. 密码库文件 `ezz.db.txt`，未指定路径时将依次在程序目录和用户家目录下寻找
3. 日志文件保存在程序目录下的 `ezz.log`（会自动创建）

### 解手模式

右键点击待处理的文件，选择用本程序打开即可

**Notice**: 解手模式需要使用密码库

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

## ❤️ Thanks

- 感谢 [7-Zip](https://www.7-zip.org/) 提供了强大的开源压缩工具
- 感谢 [@cenglin123](https://github.com/cenglin123) 为探索可行的网盘保存方式所做出的大量实践和考证

## 📝 License

7-Zip 的许可证由多种许可证构成，主要内容是 [LGPL](https://www.gnu.org/licenses/lgpl-2.1.html)，本项目使用并分发了其二进制文件，因此也遵循 LGPL 许可证
