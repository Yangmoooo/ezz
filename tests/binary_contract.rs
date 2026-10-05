//! 真实 `ezz` 二进制的契约测试：全局互斥体、退出码、多输入循环。
//!
//! 只在 Windows 上跑：macOS 的输入只走 Apple Event。
//!
//! 注意：真实二进制会把原归档送进回收站。

#![cfg(windows)]

use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus};
use std::sync::{Mutex, MutexGuard};

use windows::Win32::Foundation::{CloseHandle, HANDLE, WAIT_ABANDONED, WAIT_OBJECT_0};
use windows::Win32::System::Threading::{CreateMutexW, ReleaseMutex, WaitForSingleObject};
use windows::core::PCWSTR;

/// 故意写死字面量：实现改了名字，这些用例就该失败。
const EXTRACT_MUTEX: &str = "Local\\io.github.yangmoooo.ezz.extract";

/// 这些用例必须串行：其中一个会按住全局互斥体，别的用例一旦并发就会被"已跳过"。
static SERIAL: Mutex<()> = Mutex::new(());

fn serialize() -> MutexGuard<'static, ()> {
    SERIAL
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn prepared_seven_zip() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join("ezz-tools")
        .join("7zz.exe")
}

/// 造一个只含单个顶层文件的归档；源文件放在 `source/` 里，提取结果不会与它同名。
fn create_archive(engine: &Path, directory: &Path, name: &str) -> PathBuf {
    let source = directory.join("source");
    std::fs::create_dir_all(&source).expect("create source directory");
    std::fs::write(source.join("payload.txt"), b"payload\n").expect("write payload");
    let archive = directory.join(name);
    let status = Command::new(engine)
        .current_dir(&source)
        .args(["a", "-bso0", "-bsp0"])
        .arg(&archive)
        .arg("payload.txt")
        .status()
        .expect("create archive with 7-Zip");
    assert!(status.success(), "7-Zip must create the test archive");
    archive
}

/// 跑真实二进制；`EZZ_7ZZ` 指向准备好的引擎，避免依赖"引擎与二进制同目录"。
fn run_ezz(engine: &Path, working_directory: &Path, inputs: &[&Path]) -> ExitStatus {
    let mut command = Command::new(env!("CARGO_BIN_EXE_ezz"));
    command
        .current_dir(working_directory)
        .env("EZZ_7ZZ", engine);
    for input in inputs {
        command.arg(input);
    }
    command.output().expect("run ezz").status
}

fn open_mutex() -> HANDLE {
    let name: Vec<u16> = EXTRACT_MUTEX
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    unsafe { CreateMutexW(None, false, PCWSTR(name.as_ptr())) }.expect("create extraction mutex")
}

/// 非阻塞获取；`WAIT_OBJECT_0` 与 `WAIT_ABANDONED` 都算拿到（与实现同一套判据）。
fn try_acquire(handle: HANDLE) -> bool {
    let status = unsafe { WaitForSingleObject(handle, 0) };
    status == WAIT_OBJECT_0 || status == WAIT_ABANDONED
}

fn release(handle: HANDLE) {
    unsafe {
        let _ = ReleaseMutex(handle);
        let _ = CloseHandle(handle);
    }
}

#[test]
#[ignore = "requires cargo xtask prepare"]
fn multiple_inputs_are_processed_and_a_failed_one_does_not_stop_the_rest() {
    let _guard = serialize();
    let engine = prepared_seven_zip();
    assert!(
        engine.is_file(),
        "run `cargo xtask prepare` before this test"
    );

    let sandbox = tempfile::tempdir().expect("create test sandbox");
    let first = create_archive(&engine, sandbox.path(), "first.zip");
    let second = create_archive(&engine, sandbox.path(), "second.zip");
    let missing = sandbox.path().join("missing.zip");

    let status = run_ezz(&engine, sandbox.path(), &[&first, &missing, &second]);

    assert_eq!(status.code(), Some(1), "任一输入失败 → 退出码 1");
    // 两个成功输入都必须提交：单顶层文件直接提交，第二个因重名而递增序号。
    assert!(
        sandbox.path().join("payload.txt").is_file(),
        "the first input must be committed"
    );
    assert!(
        sandbox.path().join("payload (1).txt").is_file(),
        "an input after a failed one must still be committed"
    );
    // 成功的原归档被清理，失败的那次没有碰过任何东西。
    assert!(
        std::fs::symlink_metadata(&first).is_err(),
        "a successful input must clean its source"
    );
    assert!(
        std::fs::symlink_metadata(&second).is_err(),
        "a successful input after a failure must clean its source"
    );
}

#[test]
#[ignore = "requires cargo xtask prepare"]
fn a_call_is_skipped_while_another_ezz_holds_the_lock() {
    let _guard = serialize();
    let engine = prepared_seven_zip();
    assert!(
        engine.is_file(),
        "run `cargo xtask prepare` before this test"
    );

    let sandbox = tempfile::tempdir().expect("create test sandbox");
    let archive = create_archive(&engine, sandbox.path(), "locked.zip");

    let holder = open_mutex();
    assert!(try_acquire(holder), "the test must own the mutex");

    let status = run_ezz(&engine, sandbox.path(), &[&archive]);

    assert_eq!(status.code(), Some(0), "被跳过不是失败");
    assert!(archive.is_file(), "被跳过时不得清理原归档");
    assert!(
        !sandbox.path().join("payload.txt").exists(),
        "被跳过时不得产生任何输出"
    );

    release(holder);
}

#[test]
#[ignore = "requires cargo xtask prepare"]
fn an_abandoned_lock_is_acquired_by_the_next_call() {
    let _guard = serialize();
    let engine = prepared_seven_zip();
    assert!(
        engine.is_file(),
        "run `cargo xtask prepare` before this test"
    );

    let sandbox = tempfile::tempdir().expect("create test sandbox");
    let archive = create_archive(&engine, sandbox.path(), "abandoned.zip");

    // 本进程保留一个句柄，使内核对象在持有者线程结束后仍然存在。
    let keeper = open_mutex();

    // 让一个线程拿到锁后直接结束（不释放）：互斥体进入 abandoned 状态。
    std::thread::spawn(|| {
        let handle = open_mutex();
        assert!(try_acquire(handle), "the holder thread must own the mutex");
    })
    .join()
    .expect("holder thread must finish");

    let status = run_ezz(&engine, sandbox.path(), &[&archive]);

    assert_eq!(status.code(), Some(0), "WAIT_ABANDONED 必须视为已获得");
    assert!(
        sandbox.path().join("payload.txt").is_file(),
        "取得 abandoned 锁之后必须正常提取"
    );
    assert!(
        std::fs::symlink_metadata(&archive).is_err(),
        "正常提取后应当清理原归档"
    );

    unsafe {
        let _ = CloseHandle(keeper);
    }
}
