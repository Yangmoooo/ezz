# Ezz 的开发命令。直接运行 just 会列出全部配方。
#
# 首次开发前执行一次 "just prepare"（下载并校验固定版本的 7-Zip 引擎，之后离线可用）。

_default:
    @just --list

alias b  := build
alias br := build-release
alias c  := check
alias cm := check-mac
alias t  := test
alias ti := test-ignored
alias l  := lint
alias p  := package
alias v  := verify

# 下载并校验固定版本的 7-Zip 引擎，缓存到 target/ezz-tools/<版本>/
prepare:
    cargo xtask prepare

# 生成当前平台的发布物（Windows ZIP / macOS DMG），输出到 target/dist/
package:
    cargo xtask package

# ── 构建 ──────────────────────────────────────────────────────────────────────

# 开发构建（Windows 上保留控制台子系统，便于直接看子进程输出）
build:
    cargo build

# 发布构建
build-release:
    cargo build --release

# 手动跑一次，例如：just run ~/Downloads/foo.zip
run *args:
    cargo run -- {{args}}

# ── 测试 ──────────────────────────────────────────────────────────────────────

# 单元测试与契约测试（不需要引擎）
test:
    cargo test --workspace --all-targets --locked

# 需要引擎与真实 7-Zip 的测试，以及 Windows 对话框测试（需要交互式桌面会话）
test-ignored:
    cargo test --workspace --all-targets --locked -- --ignored

# ── 检查 ──────────────────────────────────────────────────────────────────────

check:
    cargo check --workspace --all-targets --locked

# macOS 代码可以在 Windows/Linux 上做真实的编译与 lint 检查。
#
# 只检查 ezz 本体：xtask 依赖 xz2( liblzma ) 这类 C 代码，无法在非 macOS 主机上交叉编译；
# 它在 CI 的 macOS 任务里原生构建并单独验证。
check-mac:
    cargo check -p ezz --all-targets --locked --target aarch64-apple-darwin

lint:
    cargo clippy --workspace --all-targets --locked -- -D warnings

lint-mac:
    cargo clippy -p ezz --all-targets --locked --target aarch64-apple-darwin -- -D warnings

fmt:
    cargo fmt --all

fmt-check:
    cargo fmt --all -- --check

# 提交前跑这个：格式、静态检查（两个平台）、全部测试
verify: fmt-check lint lint-mac test test-ignored

# ── 其它 ──────────────────────────────────────────────────────────────────────

# 查看 release exe 的版本资源（显示名是 Ezz，标识符仍是 ezz.exe）
[windows]
version-info:
    powershell -NoProfile -Command "(Get-Item 'target/release/ezz.exe').VersionInfo | Format-List ProductName,FileDescription,CompanyName,ProductVersion,OriginalFilename"

# 挂载 DMG 检查内容（看完用 hdiutil detach /Volumes/Ezz 卸载）
[macos]
inspect-dmg:
    hdiutil attach -nobrowse -readonly target/dist/ezz-macos-arm64.dmg

clean:
    cargo clean

update:
    cargo update
