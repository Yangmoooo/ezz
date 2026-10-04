# Ezz 开发命令。不带参数运行 just 会列出全部配方。

_default:
    @just --list

alias b  := build
alias t  := test
alias ti := test-ignored
alias l  := lint
alias p  := package
alias v  := verify

# 下载并校验发布所用的 7-Zip 引擎，缓存到 target/ezz-tools/
prepare:
    cargo xtask prepare

# 生成当前平台的发布物，输出到 target/dist/
package:
    cargo xtask package

build:
    cargo build

test:
    cargo test --workspace --all-targets --locked

# 需要引擎与真实 7-Zip 的测试，以及需要交互式桌面会话的 Windows 对话框测试
test-ignored:
    cargo test --workspace --all-targets --locked -- --ignored

check:
    cargo check --workspace --all-targets --locked

lint:
    cargo clippy --workspace --all-targets --locked -- -D warnings

fmt:
    cargo fmt --all

fmt-check:
    cargo fmt --all -- --check

# 提交前跑这个
verify: fmt-check lint test test-ignored

clean:
    cargo clean

update:
    cargo update
