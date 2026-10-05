//! 工作流级测试：真实引擎 + 真实文件系统。
//!
//! 每个用例都从 `ExtractionWorkflow::extract` 进入，只断言可观察结果；纯函数与单模块的用例
//! 留在各自模块里。

use std::collections::VecDeque;
use std::io::Write;
use std::process::Command;
use std::sync::Mutex;

use super::*;

struct RemoveSource;

impl SourceCleaner for RemoveSource {
    fn clean(&self, sources: &[PathBuf]) -> Result<(), String> {
        for source in sources {
            std::fs::remove_file(source).map_err(|error| error.to_string())?;
        }
        Ok(())
    }
}

struct FailingSourceCleaner;

impl SourceCleaner for FailingSourceCleaner {
    fn clean(&self, _sources: &[PathBuf]) -> Result<(), String> {
        Err("cleanup unavailable".to_owned())
    }
}

struct ScriptedPasswordPrompt {
    responses: Mutex<VecDeque<PasswordResponse>>,
}

impl ScriptedPasswordPrompt {
    fn new(responses: impl IntoIterator<Item = PasswordResponse>) -> Self {
        Self {
            responses: Mutex::new(responses.into_iter().collect()),
        }
    }
}

impl PasswordPrompt for ScriptedPasswordPrompt {
    fn request_password(&self, _previous_attempt_failed: bool) -> Option<PasswordResponse> {
        self.responses.lock().unwrap().pop_front()
    }
}

struct NoResponsePrompt;

impl PasswordPrompt for NoResponsePrompt {
    fn request_password(&self, _previous_attempt_failed: bool) -> Option<PasswordResponse> {
        None
    }
}

/// 大多数用例只需要替换清理器：`RemoveSource` 直接删文件（不发回收站），也不弹密码。
fn workflow(seven_zip: impl Into<PathBuf>) -> ExtractionWorkflow {
    workflow_with_cleaner(seven_zip, RemoveSource)
}

/// 替换清理器（例如让它失败，观察 `SourceCleanupFailed` 警告）。
fn workflow_with_cleaner(
    seven_zip: impl Into<PathBuf>,
    source_cleaner: impl SourceCleaner + 'static,
) -> ExtractionWorkflow {
    ExtractionWorkflow::from_parts(WorkflowParts {
        seven_zip: seven_zip.into(),
        source_cleaner: Some(Box::new(source_cleaner)),
        ..WorkflowParts::default()
    })
}

/// 额外替换密码弹窗。
fn workflow_with(
    seven_zip: impl Into<PathBuf>,
    source_cleaner: impl SourceCleaner + 'static,
    password_prompt: impl PasswordPrompt + 'static,
) -> ExtractionWorkflow {
    ExtractionWorkflow::from_parts(WorkflowParts {
        seven_zip: seven_zip.into(),
        source_cleaner: Some(Box::new(source_cleaner)),
        password_prompt: Some(Box::new(password_prompt)),
        ..WorkflowParts::default()
    })
}

/// 额外接上密码库。
fn workflow_with_store(
    seven_zip: impl Into<PathBuf>,
    source_cleaner: impl SourceCleaner + 'static,
    password_prompt: impl PasswordPrompt + 'static,
    password_store: impl Into<PathBuf>,
) -> ExtractionWorkflow {
    ExtractionWorkflow::from_parts(WorkflowParts {
        seven_zip: seven_zip.into(),
        source_cleaner: Some(Box::new(source_cleaner)),
        password_prompt: Some(Box::new(password_prompt)),
        password_store: Some(password_store.into()),
    })
}

#[test]
#[ignore = "requires cargo xtask prepare"]
fn real_archive_extracts_and_commits_its_single_top_level_file() {
    let seven_zip = prepared_seven_zip();
    assert!(
        seven_zip.is_file(),
        "run `cargo xtask prepare` before this test"
    );

    let sandbox = tempfile::tempdir().expect("create test sandbox");
    let payload = sandbox.path().join("payload.txt");
    let archive = sandbox.path().join("archive.7z");
    std::fs::write(&payload, b"ezz v3 payload").expect("create payload");

    create_archive(&seven_zip, sandbox.path(), &archive, &["payload.txt"]);
    std::fs::remove_file(&payload).expect("remove source payload");

    let outcome = workflow(&seven_zip)
        .extract(&archive)
        .expect("extract archive");

    assert_eq!(
        outcome,
        ExtractionOutcome {
            input: archive.clone(),
            output: payload.clone(),
            warnings: Vec::new(),
        }
    );
    assert_eq!(
        std::fs::read(&payload).expect("read extracted payload"),
        b"ezz v3 payload"
    );
    assert!(
        !archive.exists(),
        "successful extraction must clean the source"
    );
}

#[test]
#[ignore = "requires cargo xtask prepare"]
fn cleanup_failure_is_reported_as_a_success_warning() {
    let seven_zip = prepared_seven_zip();
    assert!(
        seven_zip.is_file(),
        "run `cargo xtask prepare` before this test"
    );

    let sandbox = tempfile::tempdir().expect("create test sandbox");
    let payload = sandbox.path().join("payload.txt");
    let archive = sandbox.path().join("archive.7z");
    std::fs::write(&payload, b"ezz v3 payload").expect("create payload");
    create_archive(&seven_zip, sandbox.path(), &archive, &["payload.txt"]);
    std::fs::remove_file(&payload).expect("remove source payload");

    let outcome = workflow_with_cleaner(&seven_zip, FailingSourceCleaner)
        .extract(&archive)
        .expect("cleanup failure must not fail extraction");

    assert_eq!(
        outcome.warnings,
        vec![ExtractionWarning::SourceCleanupFailed {
            sources: vec![archive.clone()],
            message: "cleanup unavailable".to_owned(),
        }]
    );
    assert!(payload.is_file(), "extracted output must stay committed");
    assert!(archive.is_file(), "failed cleanup must preserve the source");
}

#[test]
#[ignore = "requires cargo xtask prepare"]
fn damaged_archive_does_not_commit_partial_output_or_clean_the_source() {
    let seven_zip = prepared_seven_zip();
    assert!(
        seven_zip.is_file(),
        "run `cargo xtask prepare` before this test"
    );

    let sandbox = tempfile::tempdir().expect("create test sandbox");
    let first = sandbox.path().join("first.txt");
    let second = sandbox.path().join("second.txt");
    let archive = sandbox.path().join("damaged.7z");
    std::fs::write(&first, vec![b'a'; 4 * 1024]).expect("create first payload");
    std::fs::write(&second, vec![b'b'; 4 * 1024]).expect("create second payload");
    create_archive(
        &seven_zip,
        sandbox.path(),
        &archive,
        &["first.txt", "second.txt"],
    );
    std::fs::remove_file(&first).expect("remove first source payload");
    std::fs::remove_file(&second).expect("remove second source payload");
    let archive_file = std::fs::OpenOptions::new()
        .write(true)
        .open(&archive)
        .expect("open archive for truncation");
    let length = archive_file.metadata().unwrap().len();
    archive_file
        .set_len(length / 2)
        .expect("truncate test archive");

    let result = workflow(&seven_zip).extract(&archive);

    assert!(result.is_err(), "damaged archive must fail");
    assert!(archive.is_file(), "damaged archive must be preserved");
    assert!(
        !first.exists(),
        "partial first output must not be committed"
    );
    assert!(
        !second.exists(),
        "partial second output must not be committed"
    );
    assert_eq!(
        std::fs::read_dir(sandbox.path()).unwrap().count(),
        1,
        "damaged archive must not leave a workspace"
    );
}

#[test]
#[ignore = "requires cargo xtask prepare"]
fn multiple_top_level_entries_are_committed_in_an_archive_named_directory() {
    let seven_zip = prepared_seven_zip();
    assert!(
        seven_zip.is_file(),
        "run `cargo xtask prepare` before this test"
    );

    let sandbox = tempfile::tempdir().expect("create test sandbox");
    let first = sandbox.path().join("first.txt");
    let second = sandbox.path().join("second.txt");
    let archive = sandbox.path().join("bundle.7z");
    std::fs::write(&first, b"first").expect("create first payload");
    std::fs::write(&second, b"second").expect("create second payload");
    create_archive(
        &seven_zip,
        sandbox.path(),
        &archive,
        &["first.txt", "second.txt"],
    );
    std::fs::remove_file(&first).expect("remove first source payload");
    std::fs::remove_file(&second).expect("remove second source payload");

    let outcome = workflow(&seven_zip)
        .extract(&archive)
        .expect("extract archive");

    let output = sandbox.path().join("bundle");
    assert_eq!(outcome.output, output);
    assert_eq!(std::fs::read(output.join("first.txt")).unwrap(), b"first");
    assert_eq!(std::fs::read(output.join("second.txt")).unwrap(), b"second");
}

#[test]
#[ignore = "requires cargo xtask prepare"]
fn existing_file_is_preserved_and_new_output_gets_a_sequence_suffix() {
    let seven_zip = prepared_seven_zip();
    assert!(
        seven_zip.is_file(),
        "run `cargo xtask prepare` before this test"
    );

    let sandbox = tempfile::tempdir().expect("create test sandbox");
    let payload = sandbox.path().join("payload.txt");
    let archive = sandbox.path().join("archive.7z");
    std::fs::write(&payload, b"new content").expect("create payload");
    create_archive(&seven_zip, sandbox.path(), &archive, &["payload.txt"]);
    std::fs::write(&payload, b"existing content").expect("replace existing payload");

    let outcome = workflow(&seven_zip)
        .extract(&archive)
        .expect("extract archive without overwriting");

    let sequenced = sandbox.path().join("payload (1).txt");
    assert_eq!(outcome.output, sequenced);
    assert_eq!(std::fs::read(&payload).unwrap(), b"existing content");
    assert_eq!(std::fs::read(&sequenced).unwrap(), b"new content");
}

#[test]
#[ignore = "requires cargo xtask prepare"]
fn existing_directory_is_preserved_and_new_output_directory_gets_a_sequence_suffix() {
    let seven_zip = prepared_seven_zip();
    assert!(
        seven_zip.is_file(),
        "run `cargo xtask prepare` before this test"
    );

    let sandbox = tempfile::tempdir().expect("create test sandbox");
    let first = sandbox.path().join("first.txt");
    let second = sandbox.path().join("second.txt");
    let archive = sandbox.path().join("bundle.7z");
    let existing = sandbox.path().join("bundle");
    std::fs::write(&first, b"first").expect("create first payload");
    std::fs::write(&second, b"second").expect("create second payload");
    create_archive(
        &seven_zip,
        sandbox.path(),
        &archive,
        &["first.txt", "second.txt"],
    );
    std::fs::remove_file(&first).expect("remove first source payload");
    std::fs::remove_file(&second).expect("remove second source payload");
    std::fs::create_dir(&existing).expect("create existing directory");
    std::fs::write(existing.join("marker.txt"), b"existing").expect("create marker");

    let outcome = workflow(&seven_zip)
        .extract(&archive)
        .expect("extract archive without merging directories");

    let sequenced = sandbox.path().join("bundle (1)");
    assert_eq!(outcome.output, sequenced);
    assert_eq!(
        std::fs::read(existing.join("marker.txt")).unwrap(),
        b"existing"
    );
    assert_eq!(
        std::fs::read(sequenced.join("first.txt")).unwrap(),
        b"first"
    );
    assert_eq!(
        std::fs::read(sequenced.join("second.txt")).unwrap(),
        b"second"
    );
}

#[test]
#[ignore = "requires cargo xtask prepare"]
fn platform_metadata_does_not_change_the_top_level_layout() {
    let seven_zip = prepared_seven_zip();
    assert!(
        seven_zip.is_file(),
        "run `cargo xtask prepare` before this test"
    );

    let sandbox = tempfile::tempdir().expect("create test sandbox");
    let payload = sandbox.path().join("payload.txt");
    let ds_store = sandbox.path().join(".DS_Store");
    let metadata = sandbox.path().join("__MACOSX");
    let archive = sandbox.path().join("archive.7z");
    std::fs::write(&payload, b"payload").expect("create payload");
    std::fs::write(&ds_store, b"metadata").expect("create DS_Store");
    std::fs::create_dir(&metadata).expect("create metadata directory");
    std::fs::write(metadata.join("entry"), b"metadata").expect("create metadata entry");
    create_archive(
        &seven_zip,
        sandbox.path(),
        &archive,
        &["payload.txt", ".DS_Store", "__MACOSX"],
    );
    std::fs::remove_file(&payload).expect("remove source payload");
    std::fs::remove_file(&ds_store).expect("remove source DS_Store");
    std::fs::remove_dir_all(&metadata).expect("remove source metadata directory");

    let outcome = workflow(&seven_zip)
        .extract(&archive)
        .expect("extract archive");

    assert_eq!(outcome.output, payload);
    assert!(!sandbox.path().join(".DS_Store").exists());
    assert!(!sandbox.path().join("__MACOSX").exists());
    assert!(!sandbox.path().join("archive").exists());
}

#[cfg(unix)]
#[test]
#[ignore = "requires cargo xtask prepare"]
fn symbolic_link_that_escapes_the_result_is_sanitized_and_reported() {
    use std::os::unix::fs::symlink;

    let seven_zip = prepared_seven_zip();
    assert!(
        seven_zip.is_file(),
        "run `cargo xtask prepare` before this test"
    );

    let sandbox = tempfile::tempdir().expect("create test sandbox");
    let link = sandbox.path().join("escape");
    let archive = sandbox.path().join("archive.7z");
    let outside_name = format!("ezz-escape-outside-{}", std::process::id());
    let outside = sandbox
        .path()
        .parent()
        .expect("sandbox parent")
        .join(&outside_name);
    symlink(format!("../{outside_name}"), &link).expect("create escaping symlink");
    create_archive(&seven_zip, sandbox.path(), &archive, &["escape"]);
    std::fs::remove_file(&link).expect("remove source symlink");

    // 不安全条目不得否决整个输入。
    let outcome = workflow(&seven_zip)
        .extract(&archive)
        .expect("an escaping link must not fail the whole input");

    let reported = outcome
        .warnings
        .iter()
        .find_map(|warning| match warning {
            ExtractionWarning::UnsafeEntriesSkipped {
                discarded,
                sanitized,
            } => Some((discarded, sanitized)),
            _ => None,
        })
        .expect("the sanitized entry must be reported");
    // 薄封装：引擎留下的占位普通文件照常提交，只登记为“消毒”。
    assert!(
        reported.1.iter().any(|entry| entry.contains("escape")),
        "the escaping entry must be reported as sanitized: {reported:?}"
    );
    let committed =
        std::fs::symlink_metadata(&outcome.output).expect("the sanitized entry must be committed");
    assert!(
        committed.file_type().is_file(),
        "the committed entry must be a regular file, not a link"
    );
    assert!(
        std::fs::symlink_metadata(&outside).is_err(),
        "the escaping link must not create anything outside the result"
    );
}

#[test]
#[ignore = "requires cargo xtask prepare"]
fn parent_directory_entry_is_sanitized_and_reported() {
    let seven_zip = prepared_seven_zip();
    assert!(
        seven_zip.is_file(),
        "run `cargo xtask prepare` before this test"
    );

    let sandbox = tempfile::tempdir().expect("create test sandbox");
    let archive = sandbox.path().join("unsafe.zip");
    let escaped_name = format!("ezz-escaped-{}.txt", std::process::id());
    let escaped = sandbox
        .path()
        .parent()
        .expect("sandbox parent")
        .join(&escaped_name);
    write_zip(
        &archive,
        &[(&format!("../{escaped_name}"), b"must not escape")],
    );

    // 路径需要消毒的条目保留（数据不得丢失），但必须报告。
    let outcome = workflow(&seven_zip)
        .extract(&archive)
        .expect("a sanitized path must not fail the whole input");

    assert!(
        !escaped.exists(),
        "archive entry must not escape the workspace"
    );
    assert!(
        outcome.warnings.iter().any(|warning| matches!(
            warning,
            ExtractionWarning::UnsafeEntriesSkipped { sanitized, .. }
                if sanitized.iter().any(|entry| entry.contains(&escaped_name))
        )),
        "the sanitized entry must be named in the report: {:?}",
        outcome.warnings
    );
    assert_eq!(
        outcome.output,
        sandbox.path().join(&escaped_name),
        "the sanitized entry must be committed inside the archive directory"
    );
    assert_eq!(
        std::fs::read_to_string(&outcome.output).expect("read committed entry"),
        "must not escape"
    );
}

#[test]
#[ignore = "requires cargo xtask prepare"]
fn encrypted_archive_uses_prompted_password_and_honors_keep_source() {
    let seven_zip = prepared_seven_zip();
    assert!(
        seven_zip.is_file(),
        "run `cargo xtask prepare` before this test"
    );

    let sandbox = tempfile::tempdir().expect("create test sandbox");
    let payload = sandbox.path().join("secret.txt");
    let archive = sandbox.path().join("secret.7z");
    std::fs::write(&payload, b"classified").expect("create secret payload");
    create_encrypted_archive(
        &seven_zip,
        sandbox.path(),
        &archive,
        "secret.txt",
        "correct horse",
    );
    std::fs::remove_file(&payload).expect("remove source payload");
    let prompt = ScriptedPasswordPrompt::new([PasswordResponse {
        password: "correct horse".to_owned(),
        remember: false,
        keep_original: true,
    }]);

    let outcome = workflow_with(&seven_zip, FailingSourceCleaner, prompt)
        .extract(&archive)
        .expect("extract encrypted archive");

    assert_eq!(std::fs::read(&payload).unwrap(), b"classified");
    assert!(archive.is_file(), "keep source must preserve the archive");
    assert!(outcome.warnings.is_empty(), "cleaner must not be called");
}

#[test]
#[ignore = "requires cargo xtask prepare"]
fn content_encrypted_archive_uses_the_prompted_password() {
    let seven_zip = prepared_seven_zip();
    assert!(
        seven_zip.is_file(),
        "run `cargo xtask prepare` before this test"
    );

    let sandbox = tempfile::tempdir().expect("create test sandbox");
    let payload = sandbox.path().join("visible-name.txt");
    let archive = sandbox.path().join("content-encrypted.7z");
    std::fs::write(&payload, b"encrypted content").expect("create secret payload");
    create_content_encrypted_archive(
        &seven_zip,
        sandbox.path(),
        &archive,
        "visible-name.txt",
        "content password",
    );
    std::fs::remove_file(&payload).expect("remove source payload");
    let prompt = ScriptedPasswordPrompt::new([PasswordResponse {
        password: "content password".to_owned(),
        remember: false,
        keep_original: false,
    }]);

    let outcome = workflow_with(&seven_zip, RemoveSource, prompt)
        .extract(&archive)
        .expect("extract content-encrypted archive");

    assert_eq!(outcome.output, payload);
    assert_eq!(std::fs::read(&payload).unwrap(), b"encrypted content");
    assert!(
        !archive.exists(),
        "successful extraction must clean the source"
    );
}

#[test]
#[ignore = "requires cargo xtask prepare"]
fn password_prompt_can_retry_after_an_incorrect_password() {
    let seven_zip = prepared_seven_zip();
    assert!(
        seven_zip.is_file(),
        "run `cargo xtask prepare` before this test"
    );

    let sandbox = tempfile::tempdir().expect("create test sandbox");
    let payload = sandbox.path().join("secret.txt");
    let archive = sandbox.path().join("secret.7z");
    std::fs::write(&payload, b"classified").expect("create secret payload");
    create_encrypted_archive(
        &seven_zip,
        sandbox.path(),
        &archive,
        "secret.txt",
        "correct horse",
    );
    std::fs::remove_file(&payload).expect("remove source payload");
    let prompt = ScriptedPasswordPrompt::new([
        PasswordResponse {
            password: "wrong".to_owned(),
            remember: false,
            keep_original: false,
        },
        PasswordResponse {
            password: "correct horse".to_owned(),
            remember: false,
            keep_original: false,
        },
    ]);

    workflow_with(&seven_zip, RemoveSource, prompt)
        .extract(&archive)
        .expect("retry with the correct password");

    assert_eq!(std::fs::read(&payload).unwrap(), b"classified");
    assert!(!archive.exists(), "successful retry must clean the source");
}

#[test]
#[ignore = "requires cargo xtask prepare"]
fn cancelling_the_password_prompt_preserves_the_archive() {
    let seven_zip = prepared_seven_zip();
    assert!(
        seven_zip.is_file(),
        "run `cargo xtask prepare` before this test"
    );

    let sandbox = tempfile::tempdir().expect("create test sandbox");
    let payload = sandbox.path().join("cancelled-secret.txt");
    let archive = sandbox.path().join("cancelled.7z");
    std::fs::write(&payload, b"cancelled secret").expect("create secret payload");
    create_encrypted_archive(
        &seven_zip,
        sandbox.path(),
        &archive,
        "cancelled-secret.txt",
        "not entered",
    );
    std::fs::remove_file(&payload).expect("remove source payload");

    let result = workflow_with(&seven_zip, RemoveSource, NoResponsePrompt).extract(&archive);

    assert_eq!(
        result,
        Err(ExtractionError::PasswordRequired(archive.clone()))
    );
    assert!(archive.is_file(), "cancelled archive must be preserved");
    assert!(
        !payload.exists(),
        "cancelled archive must not commit output"
    );
}

#[test]
#[ignore = "requires cargo xtask prepare"]
fn remembered_password_is_used_for_the_next_archive() {
    let seven_zip = prepared_seven_zip();
    assert!(
        seven_zip.is_file(),
        "run `cargo xtask prepare` before this test"
    );

    let sandbox = tempfile::tempdir().expect("create test sandbox");
    let password_store = sandbox.path().join("passwords.json");
    let first_payload = sandbox.path().join("first-secret.txt");
    let first_archive = sandbox.path().join("first.7z");
    std::fs::write(&first_payload, b"first secret").expect("create first payload");
    create_encrypted_archive(
        &seven_zip,
        sandbox.path(),
        &first_archive,
        "first-secret.txt",
        "shared password",
    );
    std::fs::remove_file(&first_payload).expect("remove first source payload");

    let first_prompt = ScriptedPasswordPrompt::new([PasswordResponse {
        password: "shared password".to_owned(),
        remember: true,
        keep_original: false,
    }]);
    workflow_with_store(&seven_zip, RemoveSource, first_prompt, &password_store)
        .extract(&first_archive)
        .expect("extract and remember first password");

    let second_payload = sandbox.path().join("second-secret.txt");
    let second_archive = sandbox.path().join("second.7z");
    std::fs::write(&second_payload, b"second secret").expect("create second payload");
    create_encrypted_archive(
        &seven_zip,
        sandbox.path(),
        &second_archive,
        "second-secret.txt",
        "shared password",
    );
    std::fs::remove_file(&second_payload).expect("remove second source payload");

    workflow_with_store(&seven_zip, RemoveSource, NoResponsePrompt, &password_store)
        .extract(&second_archive)
        .expect("reuse remembered password without a prompt");

    assert_eq!(std::fs::read(&first_payload).unwrap(), b"first secret");
    assert_eq!(std::fs::read(&second_payload).unwrap(), b"second secret");
    assert!(
        password_store.is_file(),
        "remembered password must be persisted"
    );
}

#[test]
#[ignore = "requires cargo xtask prepare"]
fn numeric_volume_input_finds_the_first_volume_and_cleans_the_complete_set() {
    let seven_zip = prepared_seven_zip();
    assert!(
        seven_zip.is_file(),
        "run `cargo xtask prepare` before this test"
    );

    let sandbox = tempfile::tempdir().expect("create test sandbox");
    let payload = sandbox.path().join("payload.bin");
    let archive = sandbox.path().join("bundle.7z");
    std::fs::write(&payload, vec![0x5a; 8 * 1024]).expect("create volume payload");
    create_split_archive(&seven_zip, sandbox.path(), &archive, "payload.bin");
    std::fs::remove_file(&payload).expect("remove source payload");
    let second_volume = sandbox.path().join("bundle.7z.002");
    assert!(
        second_volume.is_file(),
        "fixture must contain a second volume"
    );

    let outcome = workflow(&seven_zip)
        .extract(&second_volume)
        .expect("extract from a non-first numeric volume");

    assert_eq!(outcome.output, payload);
    assert_eq!(std::fs::read(&payload).unwrap(), vec![0x5a; 8 * 1024]);
    assert!(
        !sandbox.path().join("bundle.7z.001").exists(),
        "first volume must be cleaned"
    );
    assert!(!second_volume.exists(), "selected volume must be cleaned");
    assert!(
        !sandbox.path().join("bundle.7z.003").exists(),
        "remaining volumes must be cleaned"
    );
}

#[test]
#[ignore = "requires cargo xtask prepare"]
fn numeric_volume_uses_the_logical_archive_name_for_multiple_outputs() {
    let seven_zip = prepared_seven_zip();
    assert!(
        seven_zip.is_file(),
        "run `cargo xtask prepare` before this test"
    );

    let sandbox = tempfile::tempdir().expect("create test sandbox");
    let first_payload = sandbox.path().join("first.bin");
    let second_payload = sandbox.path().join("second.bin");
    let archive = sandbox.path().join("bundle.7z");
    std::fs::write(&first_payload, vec![0x31; 2 * 1024]).expect("create first payload");
    std::fs::write(&second_payload, vec![0x32; 2 * 1024]).expect("create second payload");
    create_split_archive_with_inputs(
        &seven_zip,
        sandbox.path(),
        &archive,
        &["first.bin", "second.bin"],
    );
    std::fs::remove_file(&first_payload).expect("remove first source payload");
    std::fs::remove_file(&second_payload).expect("remove second source payload");
    let selected = sandbox.path().join("bundle.7z.002");

    let outcome = workflow(&seven_zip)
        .extract(&selected)
        .expect("extract multiple files from a non-first volume");

    let output = sandbox.path().join("bundle");
    assert_eq!(outcome.output, output);
    assert_eq!(
        std::fs::read(output.join("first.bin")).unwrap(),
        vec![0x31; 2 * 1024]
    );
    assert_eq!(
        std::fs::read(output.join("second.bin")).unwrap(),
        vec![0x32; 2 * 1024]
    );
}

#[test]
#[ignore = "requires cargo xtask prepare"]
fn steganographier_mp4_extracts_its_embedded_zip() {
    let seven_zip = prepared_seven_zip();
    assert!(
        seven_zip.is_file(),
        "run `cargo xtask prepare` before this test"
    );

    let sandbox = tempfile::tempdir().expect("create test sandbox");
    let payload = sandbox.path().join("hidden.txt");
    let embedded = sandbox.path().join("embedded.zip");
    let video = sandbox.path().join("carrier.mp4");
    std::fs::write(&payload, b"hidden payload").expect("create hidden payload");
    create_zip_archive(&seven_zip, sandbox.path(), &embedded, "hidden.txt");
    std::fs::remove_file(&payload).expect("remove source payload");

    let mut carrier = minimal_mp4();
    carrier.extend(std::fs::read(&embedded).expect("read embedded ZIP"));
    std::fs::write(&video, carrier).expect("create Steganographier MP4");
    std::fs::remove_file(&embedded).expect("remove standalone embedded ZIP");

    let outcome = workflow(&seven_zip)
        .extract(&video)
        .expect("extract Steganographier MP4");

    assert_eq!(outcome.output, payload);
    assert_eq!(std::fs::read(&payload).unwrap(), b"hidden payload");
    assert!(
        !video.exists(),
        "successful extraction must clean the video"
    );
}

#[test]
#[ignore = "requires cargo xtask prepare"]
fn ordinary_mp4_is_rejected_without_modifying_the_source() {
    let seven_zip = prepared_seven_zip();
    assert!(
        seven_zip.is_file(),
        "run `cargo xtask prepare` before this test"
    );

    let sandbox = tempfile::tempdir().expect("create test sandbox");
    let video = sandbox.path().join("ordinary.mp4");
    std::fs::write(&video, minimal_mp4()).expect("create ordinary MP4");

    let result = workflow(&seven_zip).extract(&video);

    assert_eq!(
        result,
        Err(ExtractionError::UnsupportedInput(video.clone()))
    );
    assert!(video.is_file(), "ordinary video must be preserved");
    assert_eq!(
        std::fs::read_dir(sandbox.path()).unwrap().count(),
        1,
        "ordinary video must not create output or leave a workspace"
    );
}

#[test]
#[ignore = "requires cargo xtask prepare"]
fn archive_with_an_mp4_extension_is_detected_by_content() {
    let seven_zip = prepared_seven_zip();
    assert!(
        seven_zip.is_file(),
        "run `cargo xtask prepare` before this test"
    );

    let sandbox = tempfile::tempdir().expect("create test sandbox");
    let payload = sandbox.path().join("renamed.txt");
    let archive = sandbox.path().join("renamed.mp4");
    std::fs::write(&payload, b"renamed archive").expect("create payload");
    create_zip_archive(&seven_zip, sandbox.path(), &archive, "renamed.txt");
    std::fs::remove_file(&payload).expect("remove source payload");

    let outcome = workflow(&seven_zip)
        .extract(&archive)
        .expect("extract renamed ZIP");

    assert_eq!(outcome.output, payload);
    assert_eq!(std::fs::read(&payload).unwrap(), b"renamed archive");
    assert!(
        !archive.exists(),
        "successful extraction must clean the source"
    );
}

#[test]
#[ignore = "requires cargo xtask prepare"]
fn tar_gzip_and_xz_archives_extract_through_the_shared_workflow() {
    let seven_zip = prepared_seven_zip();
    assert!(
        seven_zip.is_file(),
        "run `cargo xtask prepare` before this test"
    );

    for (archive_type, extension) in [("tar", "tar"), ("gzip", "gz"), ("xz", "xz")] {
        let sandbox = tempfile::tempdir().expect("create format sandbox");
        let payload = sandbox.path().join(format!("payload-{archive_type}.txt"));
        let archive = sandbox.path().join(format!("archive.{extension}"));
        let content = format!("{archive_type} payload");
        std::fs::write(&payload, &content).expect("create format payload");
        create_typed_archive(
            &seven_zip,
            sandbox.path(),
            &archive,
            payload.file_name().unwrap().to_str().unwrap(),
            archive_type,
        );
        std::fs::remove_file(&payload).expect("remove source payload");

        let outcome = workflow(&seven_zip)
            .extract(&archive)
            .expect("extract archive format");

        let expected_output = if archive_type == "xz" {
            sandbox.path().join("archive")
        } else {
            payload
        };
        assert_eq!(outcome.output, expected_output);
        assert_eq!(std::fs::read_to_string(&expected_output).unwrap(), content);
        assert!(
            !archive.exists(),
            "successful extraction must clean the source"
        );
    }
}

#[test]
#[ignore = "requires cargo xtask prepare"]
fn steganographier_mkv_extracts_its_embedded_zip() {
    let seven_zip = prepared_seven_zip();
    assert!(
        seven_zip.is_file(),
        "run `cargo xtask prepare` before this test"
    );

    let sandbox = tempfile::tempdir().expect("create test sandbox");
    let payload = sandbox.path().join("mkv-hidden.txt");
    let embedded = sandbox.path().join("mkv-embedded.zip");
    let video = sandbox.path().join("carrier.mkv");
    std::fs::write(&payload, b"MKV hidden payload").expect("create hidden payload");
    create_zip_archive(&seven_zip, sandbox.path(), &embedded, "mkv-hidden.txt");
    std::fs::remove_file(&payload).expect("remove source payload");

    let mut carrier = minimal_mkv();
    carrier.extend(std::fs::read(&embedded).expect("read embedded ZIP"));
    std::fs::write(&video, carrier).expect("create Steganographier MKV");
    std::fs::remove_file(&embedded).expect("remove standalone embedded ZIP");

    let outcome = workflow(&seven_zip)
        .extract(&video)
        .expect("extract Steganographier MKV");

    assert_eq!(outcome.output, payload);
    assert_eq!(std::fs::read(&payload).unwrap(), b"MKV hidden payload");
    assert!(
        !video.exists(),
        "successful extraction must clean the video"
    );
}

#[test]
#[ignore = "requires cargo xtask prepare"]
fn rar_non_first_volume_extracts_and_cleans_the_complete_set() {
    let seven_zip = prepared_seven_zip();
    assert!(
        seven_zip.is_file(),
        "run `cargo xtask prepare` before this test"
    );

    let sandbox = tempfile::tempdir().expect("create test sandbox");
    let mut volumes = Vec::new();
    for sequence in 1..=3 {
        let name = format!("rar-multivolume.part{sequence}.rar");
        let source = fixture(&name);
        let destination = sandbox.path().join(&name);
        std::fs::copy(source, &destination).expect("copy RAR volume fixture");
        volumes.push(destination);
    }

    let outcome = workflow(&seven_zip)
        .extract(&volumes[1])
        .expect("extract from second RAR volume");

    let output = sandbox.path().join("LibarchiveAddingTest.html");
    assert_eq!(outcome.output, output);
    let content = std::fs::read(&output).expect("read extracted RAR content");
    assert_eq!(content.len(), 20_111);
    assert!(content.ends_with(b"</BODY>\n</HTML>"));
    assert!(
        volumes.iter().all(|volume| !volume.exists()),
        "successful extraction must clean every RAR volume"
    );
}

#[test]
#[ignore = "requires cargo xtask prepare"]
fn zip_non_first_volume_extracts_and_cleans_the_complete_set() {
    let seven_zip = prepared_seven_zip();
    assert!(
        seven_zip.is_file(),
        "run `cargo xtask prepare` before this test"
    );

    let sandbox = tempfile::tempdir().expect("create test sandbox");
    let first = sandbox.path().join("zip-multivolume.z01");
    let final_volume = sandbox.path().join("zip-multivolume.zip");
    std::fs::copy(fixture("zip-multivolume.z01"), &first).expect("copy first ZIP volume fixture");
    std::fs::copy(fixture("zip-multivolume.zip"), &final_volume)
        .expect("copy final ZIP volume fixture");

    let outcome = workflow(&seven_zip)
        .extract(&first)
        .expect("extract from first ZIP split volume");

    let output = sandbox.path().join("zip-volume-payload.txt");
    assert_eq!(outcome.output, output);
    let content = std::fs::read(&output).expect("read extracted ZIP content");
    assert_eq!(content.len(), 70_000);
    assert!(content.starts_with(b"ezz zip volume payload\n"));
    assert!(!first.exists(), "first ZIP volume must be cleaned");
    assert!(!final_volume.exists(), "final ZIP volume must be cleaned");
}

/// 符号链接条目：逃逸的必须被报告且不得以链接形态提交，内部的保留。
#[test]
#[ignore = "requires cargo xtask prepare"]
fn symbolic_link_entries_do_not_fail_the_input() {
    let seven_zip = prepared_seven_zip();
    assert!(
        seven_zip.is_file(),
        "run `cargo xtask prepare` before this test"
    );

    let sandbox = tempfile::tempdir().expect("create test sandbox");
    let archive = sandbox.path().join("links.zip");
    let file = std::fs::File::create(&archive).expect("create ZIP");
    let mut writer = zip::ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default();
    writer
        .add_symlink("escape-link", "../outside.txt", options)
        .expect("add escaping symlink");
    writer
        .add_symlink("inner-link", "target.txt", options)
        .expect("add inner symlink");
    writer
        .start_file("target.txt", options)
        .expect("start entry");
    writer.write_all(b"payload\n").expect("write entry");
    writer.finish().expect("finish ZIP");

    let outcome = workflow(&seven_zip)
        .extract(&archive)
        .expect("symbolic links must not fail the whole input");

    let reported = outcome
        .warnings
        .iter()
        .find_map(|warning| match warning {
            ExtractionWarning::UnsafeEntriesSkipped {
                discarded,
                sanitized,
            } => Some((discarded, sanitized)),
            _ => None,
        })
        .expect("the escaping link must be reported");
    assert!(
        reported
            .0
            .iter()
            .any(|path| path.to_string_lossy().contains("escape-link"))
            || reported.1.iter().any(|name| name.contains("escape-link")),
        "the escaping link must be named: discarded={:?} sanitized={:?}",
        reported.0,
        reported.1
    );

    let committed_escape = outcome.output.join("escape-link");
    if committed_escape.exists() {
        assert!(
            !std::fs::symlink_metadata(&committed_escape)
                .expect("inspect committed entry")
                .file_type()
                .is_symlink(),
            "an escaping link must not be committed as a link"
        );
    }

    let committed_inner = outcome.output.join("inner-link");
    let is_link = std::fs::symlink_metadata(&committed_inner)
        .map(|metadata| metadata.file_type().is_symlink())
        .unwrap_or(false);
    if is_link {
        assert_eq!(
            std::fs::read_link(&committed_inner).expect("read committed link"),
            Path::new("target.txt"),
            "a link inside the result must be preserved as-is"
        );
    }
}

/// 剔除平台元数据后没有内容：降级成功 + 报告。
#[test]
#[ignore = "requires cargo xtask prepare"]
fn archive_with_only_platform_metadata_is_a_reported_degraded_success() {
    let seven_zip = prepared_seven_zip();
    assert!(
        seven_zip.is_file(),
        "run `cargo xtask prepare` before this test"
    );

    let sandbox = tempfile::tempdir().expect("create test sandbox");
    let archive = sandbox.path().join("meta.zip");
    write_zip(
        &archive,
        &[("__MACOSX/junk", b"junk\n"), (".DS_Store", b"ds\n")],
    );

    let outcome = workflow(&seven_zip)
        .extract(&archive)
        .expect("an archive with only platform metadata must not fail");

    assert!(
        outcome.output.is_dir(),
        "an empty result must still have a final path: {:?}",
        outcome.output
    );
    assert_eq!(
        std::fs::read_dir(&outcome.output)
            .expect("read empty result")
            .count(),
        0,
        "the committed result must be empty"
    );
    assert!(
        outcome.warnings.iter().any(|warning| matches!(
            warning,
            ExtractionWarning::EmptyAfterMetadataRemoval { removed } if !removed.is_empty()
        )),
        "the empty result must be reported: {:?}",
        outcome.warnings
    );
}

/// 手写一个最小 tar：`zip` crate 会把反斜杠改写成下划线，所以盘符前缀条目只能这样造。
fn write_tar(path: &Path, entries: &[(&str, &[u8])]) {
    let mut bytes = Vec::new();
    for (name, data) in entries {
        let mut header = [0_u8; 512];
        header[..name.len()].copy_from_slice(name.as_bytes());
        header[100..108].copy_from_slice(b"0000644\0");
        header[108..116].copy_from_slice(b"0000000\0");
        header[116..124].copy_from_slice(b"0000000\0");
        header[124..136].copy_from_slice(format!("{:011o}\0", data.len()).as_bytes());
        header[136..148].copy_from_slice(b"00000000000\0");
        header[156] = b'0';
        header[148..156].copy_from_slice(b"        ");
        let checksum: u32 = header.iter().map(|byte| u32::from(*byte)).sum();
        header[148..156].copy_from_slice(format!("{checksum:06o}\0 ").as_bytes());
        bytes.extend_from_slice(&header);
        bytes.extend_from_slice(data);
        bytes.extend(std::iter::repeat_n(0_u8, (512 - data.len() % 512) % 512));
    }
    bytes.extend(std::iter::repeat_n(0_u8, 1024));
    std::fs::write(path, &bytes).expect("write tar");
}

/// 盘符前缀条目（`C:\drive.txt`）：7-Zip 读取时把它改写成 `C:_drive.txt`，提取时再把
/// 非法字符换成 `_`。数据必须保留且必须报告。
///
/// 用 tar 而不是 zip：`zip` crate 会在写入时就把反斜杠换成下划线，造不出真的盘符条目。
#[test]
#[ignore = "requires cargo xtask prepare"]
fn drive_prefixed_entries_are_sanitized_and_reported() {
    let seven_zip = prepared_seven_zip();
    assert!(
        seven_zip.is_file(),
        "run `cargo xtask prepare` before this test"
    );

    let sandbox = tempfile::tempdir().expect("create test sandbox");
    let archive = sandbox.path().join("drive.tar");
    write_tar(
        &archive,
        &[("C:\\drive.txt", b"drive payload"), ("keep.txt", b"keep")],
    );

    let outcome = workflow(&seven_zip)
        .extract(&archive)
        .expect("a drive-prefixed entry must not fail the whole input");

    let sanitized = outcome
        .warnings
        .iter()
        .find_map(|warning| match warning {
            ExtractionWarning::UnsafeEntriesSkipped { sanitized, .. } => Some(sanitized),
            _ => None,
        })
        .expect("the drive-prefixed entry must be reported");
    assert!(
        sanitized.iter().any(|name| name.contains("drive.txt")),
        "the entry must be named in the report: {sanitized:?}"
    );

    // 数据不得丢失：必须落在结果内（名字会被消毒成合法文件名）。
    assert!(outcome.output.is_dir(), "{:?}", outcome.output);
    let mut found_payload = false;
    for entry in std::fs::read_dir(&outcome.output).expect("read result") {
        let entry = entry.expect("result entry");
        if entry.file_type().expect("file type").is_file()
            && std::fs::read(entry.path()).expect("read entry") == b"drive payload"
        {
            found_payload = true;
        }
    }
    assert!(found_payload, "the sanitized entry must keep its data");
    assert!(outcome.output.join("keep.txt").is_file());
}

/// 绝对路径条目（`/absolute.txt`）：7-Zip 把它重写进工作目录（提交后为 `absolute.txt`）。
#[test]
#[ignore = "requires cargo xtask prepare"]
fn absolute_path_entries_are_sanitized_and_reported() {
    let seven_zip = prepared_seven_zip();
    assert!(
        seven_zip.is_file(),
        "run `cargo xtask prepare` before this test"
    );

    let sandbox = tempfile::tempdir().expect("create test sandbox");
    let archive = sandbox.path().join("absolute.zip");
    write_zip(
        &archive,
        &[
            ("/absolute.txt", b"/absolute.txt"),
            ("keep.txt", b"keep.txt"),
        ],
    );

    let outcome = workflow(&seven_zip)
        .extract(&archive)
        .expect("an absolute entry path must not fail the whole input");

    let sanitized = outcome
        .warnings
        .iter()
        .find_map(|warning| match warning {
            ExtractionWarning::UnsafeEntriesSkipped { sanitized, .. } => Some(sanitized),
            _ => None,
        })
        .expect("the absolute entry path must be reported");
    assert!(
        sanitized.iter().any(|name| name.contains("absolute.txt")),
        "the entry must be named in the report: {sanitized:?}"
    );

    assert!(outcome.output.is_dir(), "{:?}", outcome.output);
    assert!(
        outcome.output.join("absolute.txt").is_file(),
        "the sanitized entry must be kept"
    );
    assert!(outcome.output.join("keep.txt").is_file());
}

/// 把最后一个条目的压缩数据末字节改坏（中央目录紧跟在数据之后）。
fn corrupt_last_entry_byte(bytes: &mut [u8]) {
    let eocd = bytes
        .windows(4)
        .rposition(|window| window == b"PK\x05\x06")
        .expect("find end of central directory");
    let directory_offset = u32::from_le_bytes(
        bytes[eocd + 16..eocd + 20]
            .try_into()
            .expect("offset field"),
    ) as usize;
    bytes[directory_offset - 1] ^= 0xFF;
}

/// 单个条目损坏 → 降级成功：其余条目照常提交，损坏条目也照旧提交，只点名报告。
#[test]
#[ignore = "requires cargo xtask prepare"]
fn a_corrupted_entry_is_committed_and_reported_while_the_rest_is_kept() {
    let seven_zip = prepared_seven_zip();
    assert!(
        seven_zip.is_file(),
        "run `cargo xtask prepare` before this test"
    );

    let sandbox = tempfile::tempdir().expect("create test sandbox");
    let archive = sandbox.path().join("corrupted.zip");
    let file = std::fs::File::create(&archive).expect("create ZIP");
    let mut writer = zip::ZipWriter::new(file);
    let options =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    writer.start_file("good.txt", options).expect("start entry");
    writer.write_all(b"good content").expect("write entry");
    writer
        .start_file("third.txt", options)
        .expect("start entry");
    writer.write_all(b"third content").expect("write entry");
    // 故意写在最后：`corrupt_last_entry_byte` 改坏的正是最后一个条目的数据。
    writer.start_file("bad.txt", options).expect("start entry");
    writer
        .write_all(b"payload-to-corrupt")
        .expect("write entry");
    writer.finish().expect("finish ZIP");

    let mut bytes = std::fs::read(&archive).expect("read ZIP");
    corrupt_last_entry_byte(&mut bytes);
    std::fs::write(&archive, &bytes).expect("write corrupted ZIP");

    let outcome = workflow(&seven_zip)
        .extract(&archive)
        .expect("a corrupted entry must not fail the whole input");

    let reported = outcome
        .warnings
        .iter()
        .find_map(|warning| match warning {
            ExtractionWarning::FailedEntries { entries } => Some(entries),
            _ => None,
        })
        .expect("the corrupted entry must be reported");
    assert!(
        reported.iter().any(|entry| entry.contains("bad.txt")),
        "the corrupted entry must be named: {reported:?}"
    );

    // 三个顶层项 → 结果是归档名命名的目录。
    assert!(outcome.output.is_dir(), "{:?}", outcome.output);
    assert!(
        outcome.output.join("good.txt").is_file() && outcome.output.join("third.txt").is_file(),
        "healthy entries must still be committed"
    );
    // 7-Zip 把损坏的条目也写出来了，这里不改它的输出，只把条目名写进警告。
    assert!(
        outcome.output.join("bad.txt").is_file(),
        "the corrupted entry is written by 7-Zip and must not be removed by ezz"
    );
}

/// Unicode 与空格文件名：提交名原样保留，冲突时递增序号。
#[test]
#[ignore = "requires cargo xtask prepare"]
fn unicode_and_space_names_are_committed_unchanged() {
    let seven_zip = prepared_seven_zip();
    assert!(
        seven_zip.is_file(),
        "run `cargo xtask prepare` before this test"
    );

    let sandbox = tempfile::tempdir().expect("create test sandbox");
    let source = sandbox.path().join("source");
    std::fs::create_dir(&source).expect("create source directory");
    let name = "报告 汇总 (最终).txt";
    std::fs::write(source.join(name), b"content").expect("write source file");
    let archive = sandbox.path().join("unicode.zip");
    create_archive(&seven_zip, &source, &archive, &[name]);

    let workflow = workflow(&seven_zip);
    let first = workflow.extract(&archive).expect("first extraction");
    assert_eq!(first.output.file_name().unwrap().to_string_lossy(), name);
    assert_eq!(
        std::fs::read_to_string(&first.output).expect("read committed file"),
        "content"
    );

    // 原归档被 RemoveSource 删除，重建一次以验证冲突命名。
    create_archive(&seven_zip, &source, &archive, &[name]);
    let second = workflow.extract(&archive).expect("second extraction");
    assert_eq!(
        second.output.file_name().unwrap().to_string_lossy(),
        "报告 汇总 (最终) (1).txt"
    );
}

/// 特殊文件（FIFO）必须被丢弃并报告。Windows 上无法构造 FIFO，所以只在 Unix 跑。
#[cfg(unix)]
#[test]
fn special_files_are_discarded_and_reported() {
    let sandbox = tempfile::tempdir().expect("create test sandbox");
    let root = sandbox.path().join("extracted");
    std::fs::create_dir(&root).expect("create root");
    std::fs::write(root.join("keep.txt"), b"keep").expect("write kept file");

    let fifo = root.join("pipe");
    let status = std::process::Command::new("mkfifo")
        .arg(&fifo)
        .status()
        .expect("run mkfifo");
    assert!(status.success(), "mkfifo must create the FIFO");

    let discarded = discard_unsafe_entries(&root).expect("discard unsafe entries");

    assert_eq!(
        discarded,
        vec![PathBuf::from("pipe")],
        "the FIFO must be discarded and named"
    );
    assert!(root.join("keep.txt").is_file(), "safe content must be kept");
    assert!(!fifo.exists(), "the FIFO must be removed");
}

/// 硬链接不可跨越文件系统，因此它不会变成逃逸向量；归档里的硬链接必须当普通文件处理。
#[test]
#[ignore = "requires cargo xtask prepare"]
fn hard_links_are_extracted_as_regular_files() {
    let seven_zip = prepared_seven_zip();
    assert!(
        seven_zip.is_file(),
        "run `cargo xtask prepare` before this test"
    );

    let sandbox = tempfile::tempdir().expect("create test sandbox");
    let source = sandbox.path().join("source");
    std::fs::create_dir(&source).expect("create source directory");
    std::fs::write(source.join("original.txt"), b"shared").expect("write original");
    std::fs::hard_link(source.join("original.txt"), source.join("linked.txt"))
        .expect("create hard link");

    let archive = sandbox.path().join("hardlinks.7z");
    let status = std::process::Command::new(&seven_zip)
        .current_dir(&source)
        .args(["a", "-t7z", "-snh", "-mx=1", "-bso0", "-bsp0"])
        .arg(&archive)
        .args(["original.txt", "linked.txt"])
        .status()
        .expect("create archive with 7-Zip");
    assert!(status.success(), "7-Zip must create the hard-link archive");

    let outcome = workflow(&seven_zip)
        .extract(&archive)
        .expect("hard links must not fail the input");

    assert!(
        !outcome
            .warnings
            .iter()
            .any(|warning| matches!(warning, ExtractionWarning::UnsafeEntriesSkipped { .. })),
        "hard links must not be reported as unsafe: {:?}",
        outcome.warnings
    );
    for name in ["original.txt", "linked.txt"] {
        let path = if outcome.output.is_dir() {
            outcome.output.join(name)
        } else {
            outcome.output.clone()
        };
        assert_eq!(
            std::fs::read_to_string(&path).expect("read extracted file"),
            "shared"
        );
    }
}

fn create_archive(seven_zip: &Path, directory: &Path, archive: &Path, inputs: &[&str]) {
    let mut command = Command::new(seven_zip);
    command
        .current_dir(directory)
        .args(["a", "-t7z"])
        .arg(archive)
        .args(inputs)
        .args(["-mx=1", "-snl", "-bso0", "-bsp0"]);
    let status = command.status().expect("create archive with 7-Zip");
    assert!(status.success(), "7-Zip must create the test archive");
}

fn create_encrypted_archive(
    seven_zip: &Path,
    directory: &Path,
    archive: &Path,
    input: &str,
    password: &str,
) {
    let status = Command::new(seven_zip)
        .current_dir(directory)
        .args(["a", "-t7z"])
        .arg(archive)
        .arg(input)
        .arg(format!("-p{password}"))
        .args(["-mhe=on", "-mx=1", "-bso0", "-bsp0"])
        .status()
        .expect("create encrypted archive with 7-Zip");
    assert!(status.success(), "7-Zip must create encrypted test archive");
}

fn create_content_encrypted_archive(
    seven_zip: &Path,
    directory: &Path,
    archive: &Path,
    input: &str,
    password: &str,
) {
    let status = Command::new(seven_zip)
        .current_dir(directory)
        .args(["a", "-t7z"])
        .arg(archive)
        .arg(input)
        .arg(format!("-p{password}"))
        .args(["-mhe=off", "-mx=1", "-bso0", "-bsp0"])
        .status()
        .expect("create content-encrypted archive with 7-Zip");
    assert!(status.success(), "7-Zip must create encrypted test archive");
}

fn create_zip_archive(seven_zip: &Path, directory: &Path, archive: &Path, input: &str) {
    let status = Command::new(seven_zip)
        .current_dir(directory)
        .args(["a", "-tzip"])
        .arg(archive)
        .arg(input)
        .args(["-mx=1", "-bso0", "-bsp0"])
        .status()
        .expect("create ZIP with 7-Zip");
    assert!(status.success(), "7-Zip must create the embedded ZIP");
}

fn create_typed_archive(
    seven_zip: &Path,
    directory: &Path,
    archive: &Path,
    input: &str,
    archive_type: &str,
) {
    let status = Command::new(seven_zip)
        .current_dir(directory)
        .arg("a")
        .arg(format!("-t{archive_type}"))
        .arg(archive)
        .arg(input)
        .args(["-mx=1", "-bso0", "-bsp0"])
        .status()
        .expect("create typed archive with 7-Zip");
    assert!(status.success(), "7-Zip must create {archive_type}");
}

fn minimal_mp4() -> Vec<u8> {
    vec![
        0, 0, 0, 24, b'f', b't', b'y', b'p', b'i', b's', b'o', b'm', 0, 0, 2, 0, b'i', b's', b'o',
        b'm', b'm', b'p', b'4', b'2', 0, 0, 0, 8, b'f', b'r', b'e', b'e',
    ]
}

fn minimal_mkv() -> Vec<u8> {
    vec![
        0x1a, 0x45, 0xdf, 0xa3, 0x8f, 0x42, 0x86, 0x81, 0x01, 0x42, 0xf7, 0x81, 0x01, 0x42, 0xf2,
        0x81, 0x04,
    ]
}

fn create_split_archive(seven_zip: &Path, directory: &Path, archive: &Path, input: &str) {
    create_split_archive_with_inputs(seven_zip, directory, archive, &[input]);
}

fn create_split_archive_with_inputs(
    seven_zip: &Path,
    directory: &Path,
    archive: &Path,
    inputs: &[&str],
) {
    let status = Command::new(seven_zip)
        .current_dir(directory)
        .args(["a", "-t7z"])
        .arg(archive)
        .args(inputs)
        .args(["-v1k", "-mx=0", "-bso0", "-bsp0"])
        .status()
        .expect("create split archive with 7-Zip");
    assert!(status.success(), "7-Zip must create split test archive");
}

/// 用一个或多个条目造一个 ZIP：名字与内容由调用方给定，压缩方式用默认值。
fn write_zip(archive: &Path, entries: &[(&str, &[u8])]) {
    let file = std::fs::File::create(archive).expect("create ZIP");
    let mut writer = zip::ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default();
    for (name, content) in entries {
        writer.start_file(*name, options).expect("start entry");
        writer.write_all(content).expect("write entry");
    }
    writer.finish().expect("finish ZIP");
}

fn prepared_seven_zip() -> PathBuf {
    let binary_name = if cfg!(target_os = "windows") {
        "7zz.exe"
    } else {
        "7zz"
    };
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join("ezz-tools")
        .join("26.02")
        .join(binary_name)
}

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name)
}
