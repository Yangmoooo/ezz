use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::fs;
use std::path::{Path, PathBuf};

use thiserror::Error;

use crate::password_store::PasswordStore;
use crate::seven_zip::SevenZip;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtractionOutcome {
    pub input: PathBuf,
    pub output: PathBuf,
    pub warnings: Vec<ExtractionWarning>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExtractionWarning {
    SourceCleanupFailed {
        sources: Vec<PathBuf>,
        message: String,
    },
    PasswordStoreUpdateFailed {
        path: PathBuf,
        message: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PasswordResponse {
    pub password: String,
    pub remember: bool,
    pub keep_original: bool,
}

pub trait PasswordPrompt {
    fn request_password(
        &self,
        input: &Path,
        previous_attempt_failed: bool,
    ) -> Option<PasswordResponse>;
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum ExtractionError {
    #[error("Input does not exist: {0}")]
    InputNotFound(PathBuf),

    #[error("Input is not a file: {0}")]
    InputNotFile(PathBuf),

    #[error("7-Zip executable does not exist: {0}")]
    EngineNotFound(PathBuf),

    #[error("Could not start 7-Zip at {path}: {message}")]
    EngineLaunch { path: PathBuf, message: String },

    #[error("7-Zip failed to {operation} with exit code {exit_code:?}: {message}")]
    EngineFailed {
        operation: &'static str,
        exit_code: Option<i32>,
        message: String,
    },

    #[error("Input is not a supported archive: {0}")]
    UnsupportedInput(PathBuf),

    #[error("Archive volume is missing: {0}")]
    MissingVolume(PathBuf),

    #[error("Archive password is incorrect")]
    WrongPassword,

    #[error("Archive password was not provided: {0}")]
    PasswordRequired(PathBuf),

    #[error("Could not read password database {path}: {message}")]
    PasswordStore { path: PathBuf, message: String },

    #[error("Archive produced no output: {0}")]
    EmptyArchive(PathBuf),

    #[error("Could not {operation} {path}: {message}")]
    FileSystem {
        operation: &'static str,
        path: PathBuf,
        message: String,
    },

    #[error("Unsafe extracted output at {path}: {reason}")]
    UnsafeOutput { path: PathBuf, reason: String },
}

pub struct ExtractionWorkflow {
    seven_zip: PathBuf,
    source_cleaner: Box<dyn SourceCleaner>,
    password_prompt: Box<dyn PasswordPrompt>,
    password_store: Option<PasswordStore>,
}

impl ExtractionWorkflow {
    pub fn new(seven_zip: impl Into<PathBuf>) -> Self {
        Self {
            seven_zip: seven_zip.into(),
            source_cleaner: Box::new(TrashCleaner),
            password_prompt: Box::new(NoPasswordPrompt),
            password_store: None,
        }
    }

    pub fn with_password_support(
        seven_zip: impl Into<PathBuf>,
        password_store: impl Into<PathBuf>,
        password_prompt: impl PasswordPrompt + 'static,
    ) -> Self {
        Self {
            seven_zip: seven_zip.into(),
            source_cleaner: Box::new(TrashCleaner),
            password_prompt: Box::new(password_prompt),
            password_store: Some(PasswordStore::new(password_store)),
        }
    }

    #[cfg(test)]
    fn with_source_cleaner(
        seven_zip: impl Into<PathBuf>,
        source_cleaner: impl SourceCleaner + 'static,
    ) -> Self {
        Self {
            seven_zip: seven_zip.into(),
            source_cleaner: Box::new(source_cleaner),
            password_prompt: Box::new(NoPasswordPrompt),
            password_store: None,
        }
    }

    #[cfg(test)]
    fn with_adapters(
        seven_zip: impl Into<PathBuf>,
        source_cleaner: impl SourceCleaner + 'static,
        password_prompt: impl PasswordPrompt + 'static,
    ) -> Self {
        Self {
            seven_zip: seven_zip.into(),
            source_cleaner: Box::new(source_cleaner),
            password_prompt: Box::new(password_prompt),
            password_store: None,
        }
    }

    #[cfg(test)]
    fn with_adapters_and_password_store(
        seven_zip: impl Into<PathBuf>,
        source_cleaner: impl SourceCleaner + 'static,
        password_prompt: impl PasswordPrompt + 'static,
        password_store: impl Into<PathBuf>,
    ) -> Self {
        Self {
            seven_zip: seven_zip.into(),
            source_cleaner: Box::new(source_cleaner),
            password_prompt: Box::new(password_prompt),
            password_store: Some(PasswordStore::new(password_store)),
        }
    }

    pub fn extract(&self, input: impl AsRef<Path>) -> Result<ExtractionOutcome, ExtractionError> {
        let input = input.as_ref();
        if !input.exists() {
            return Err(ExtractionError::InputNotFound(input.to_path_buf()));
        }
        if !input.is_file() {
            return Err(ExtractionError::InputNotFile(input.to_path_buf()));
        }
        if !self.seven_zip.is_file() {
            return Err(ExtractionError::EngineNotFound(self.seven_zip.clone()));
        }

        let selected_input = absolute_path(input)?;
        let archive_set = resolve_archive_set(&selected_input)?;
        let input = &archive_set.primary;
        let seven_zip = SevenZip::new(&self.seven_zip);
        match seven_zip.probe(input) {
            Ok(()) | Err(ExtractionError::WrongPassword) => {}
            Err(error) => return Err(error),
        }
        let password = self.resolve_password(&seven_zip, input)?;

        let parent = input.parent().ok_or_else(|| ExtractionError::FileSystem {
            operation: "resolve parent of",
            path: input.to_path_buf(),
            message: "input has no parent directory".to_owned(),
        })?;
        let workspace = tempfile::Builder::new()
            .prefix(".ezz-work-")
            .tempdir_in(parent)
            .map_err(|error| file_system_error("create workspace for", input, error))?;
        let extracted = workspace.path().join("extracted");
        fs::create_dir(&extracted)
            .map_err(|error| file_system_error("create extraction directory", &extracted, error))?;

        seven_zip.extract(input, &extracted, &password.value)?;
        validate_extracted_output(&extracted)?;
        let output = commit_output(input, &extracted)?;
        let sources = archive_set.sources;
        let mut warnings = Vec::new();
        if password.remember
            && !password.value.is_empty()
            && let Some(store) = &self.password_store
            && let Err(message) = store.record_success(&password.value)
        {
            warnings.push(ExtractionWarning::PasswordStoreUpdateFailed {
                path: store.path().to_path_buf(),
                message,
            });
        }
        if !password.keep_original
            && let Some(message) = self.source_cleaner.clean(&sources).err()
        {
            warnings.push(ExtractionWarning::SourceCleanupFailed { sources, message });
        }

        Ok(ExtractionOutcome {
            input: selected_input,
            output,
            warnings,
        })
    }

    fn resolve_password(
        &self,
        seven_zip: &SevenZip,
        input: &Path,
    ) -> Result<ResolvedPassword, ExtractionError> {
        match seven_zip.test_password(input, "") {
            Ok(()) => return Ok(ResolvedPassword::empty()),
            Err(ExtractionError::WrongPassword) => {}
            Err(error) => return Err(error),
        }

        if let Some(store) = &self.password_store {
            let candidates =
                store
                    .candidates()
                    .map_err(|message| ExtractionError::PasswordStore {
                        path: store.path().to_path_buf(),
                        message,
                    })?;
            for password in candidates {
                match seven_zip.test_password(input, &password) {
                    Ok(()) => {
                        return Ok(ResolvedPassword {
                            value: password,
                            remember: true,
                            keep_original: false,
                        });
                    }
                    Err(ExtractionError::WrongPassword) => {}
                    Err(error) => return Err(error),
                }
            }
        }

        let mut previous_attempt_failed = false;
        loop {
            let Some(response) = self
                .password_prompt
                .request_password(input, previous_attempt_failed)
            else {
                return Err(ExtractionError::PasswordRequired(input.to_path_buf()));
            };

            match seven_zip.test_password(input, &response.password) {
                Ok(()) => {
                    return Ok(ResolvedPassword {
                        value: response.password,
                        remember: response.remember,
                        keep_original: response.keep_original,
                    });
                }
                Err(ExtractionError::WrongPassword) => previous_attempt_failed = true,
                Err(error) => return Err(error),
            }
        }
    }
}

struct ArchiveSet {
    primary: PathBuf,
    sources: Vec<PathBuf>,
}

fn resolve_archive_set(selected: &Path) -> Result<ArchiveSet, ExtractionError> {
    let Some(sequence) = numeric_extension(selected) else {
        return Ok(ArchiveSet {
            primary: selected.to_path_buf(),
            sources: vec![selected.to_path_buf()],
        });
    };

    let first = selected.with_extension("001");
    if !first.is_file() {
        return Err(ExtractionError::MissingVolume(first));
    }

    let parent = selected.parent().expect("absolute input parent");
    let prefix = selected.file_stem().expect("volume file stem");
    let mut volumes = BTreeMap::new();
    let entries = fs::read_dir(parent)
        .map_err(|error| file_system_error("scan archive volumes in", parent, error))?;
    for entry in entries {
        let entry =
            entry.map_err(|error| file_system_error("scan archive volume in", parent, error))?;
        let path = entry.path();
        if path.file_stem() == Some(prefix)
            && let Some(number) = numeric_extension(&path)
        {
            volumes.insert(number, path);
        }
    }

    let last = volumes.keys().next_back().copied().unwrap_or(sequence);
    for number in 1..=last {
        if !volumes.contains_key(&number) {
            return Err(ExtractionError::MissingVolume(
                selected.with_extension(format!("{number:03}")),
            ));
        }
    }

    Ok(ArchiveSet {
        primary: first,
        sources: volumes.into_values().collect(),
    })
}

fn numeric_extension(path: &Path) -> Option<u32> {
    let extension = path.extension()?.to_str()?;
    (extension.len() == 3 && extension.bytes().all(|byte| byte.is_ascii_digit()))
        .then(|| extension.parse().ok())
        .flatten()
}

struct ResolvedPassword {
    value: String,
    remember: bool,
    keep_original: bool,
}

impl ResolvedPassword {
    fn empty() -> Self {
        Self {
            value: String::new(),
            remember: false,
            keep_original: false,
        }
    }
}

struct NoPasswordPrompt;

impl PasswordPrompt for NoPasswordPrompt {
    fn request_password(
        &self,
        _input: &Path,
        _previous_attempt_failed: bool,
    ) -> Option<PasswordResponse> {
        None
    }
}

fn validate_extracted_output(root: &Path) -> Result<(), ExtractionError> {
    let canonical_root = fs::canonicalize(root)
        .map_err(|error| file_system_error("resolve extraction directory", root, error))?;
    let mut directories = vec![root.to_path_buf()];

    while let Some(directory) = directories.pop() {
        let entries = fs::read_dir(&directory)
            .map_err(|error| file_system_error("inspect extracted directory", &directory, error))?;
        for entry in entries {
            let entry = entry.map_err(|error| {
                file_system_error("inspect extracted entry in", &directory, error)
            })?;
            let path = entry.path();
            let metadata = fs::symlink_metadata(&path)
                .map_err(|error| file_system_error("inspect extracted entry", &path, error))?;
            let file_type = metadata.file_type();

            if file_type.is_symlink() {
                let resolved =
                    fs::canonicalize(&path).map_err(|error| ExtractionError::UnsafeOutput {
                        path: path.clone(),
                        reason: format!("symbolic link cannot be resolved: {error}"),
                    })?;
                if !resolved.starts_with(&canonical_root) {
                    return Err(ExtractionError::UnsafeOutput {
                        path,
                        reason: "symbolic link escapes the extraction directory".to_owned(),
                    });
                }
            } else if file_type.is_dir() {
                directories.push(path);
            } else if !file_type.is_file() {
                return Err(ExtractionError::UnsafeOutput {
                    path,
                    reason: "special files are not supported".to_owned(),
                });
            }
        }
    }

    Ok(())
}

fn absolute_path(path: &Path) -> Result<PathBuf, ExtractionError> {
    if path.is_absolute() {
        return Ok(path.to_path_buf());
    }

    std::env::current_dir()
        .map(|current| current.join(path))
        .map_err(|error| file_system_error("resolve absolute path for", path, error))
}

fn commit_output(input: &Path, extracted: &Path) -> Result<PathBuf, ExtractionError> {
    remove_platform_metadata(extracted)?;
    let mut entries = fs::read_dir(extracted)
        .map_err(|error| file_system_error("read extracted contents from", extracted, error))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| file_system_error("read extracted entry from", extracted, error))?;

    match entries.len() {
        0 => Err(ExtractionError::EmptyArchive(input.to_path_buf())),
        1 => {
            let entry = entries.pop().expect("one extracted entry");
            let source = entry.path();
            let parent = input.parent().expect("validated input parent");
            let target = unique_file_destination(parent, &entry.file_name());
            fs::rename(&source, &target)
                .map_err(|error| file_system_error("commit extracted output to", &target, error))?;
            Ok(target)
        }
        _ => {
            let parent = input.parent().expect("validated input parent");
            let name = input
                .file_stem()
                .unwrap_or_else(|| std::ffi::OsStr::new("archive"));
            let target = unique_directory_destination(parent, name);
            fs::rename(extracted, &target)
                .map_err(|error| file_system_error("commit extracted output to", &target, error))?;
            Ok(target)
        }
    }
}

fn remove_platform_metadata(extracted: &Path) -> Result<(), ExtractionError> {
    for name in ["__MACOSX", ".DS_Store"] {
        let path = extracted.join(name);
        if path.is_dir() {
            fs::remove_dir_all(&path)
                .map_err(|error| file_system_error("remove platform metadata", &path, error))?;
        } else if path.exists() {
            fs::remove_file(&path)
                .map_err(|error| file_system_error("remove platform metadata", &path, error))?;
        }
    }
    Ok(())
}

fn unique_file_destination(parent: &Path, name: &OsStr) -> PathBuf {
    let initial = parent.join(name);
    if !initial.exists() {
        return initial;
    }

    let name_path = Path::new(name);
    let stem = name_path.file_stem().unwrap_or(name);
    let extension = name_path.extension();
    for sequence in 1_u64.. {
        let mut candidate = OsString::from(stem);
        candidate.push(format!(" ({sequence})"));
        if let Some(extension) = extension {
            candidate.push(".");
            candidate.push(extension);
        }
        let candidate = parent.join(candidate);
        if !candidate.exists() {
            return candidate;
        }
    }

    unreachable!("u64 destination sequence exhausted")
}

fn unique_directory_destination(parent: &Path, name: &OsStr) -> PathBuf {
    let initial = parent.join(name);
    if !initial.exists() {
        return initial;
    }

    for sequence in 1_u64.. {
        let mut candidate = OsString::from(name);
        candidate.push(format!(" ({sequence})"));
        let candidate = parent.join(candidate);
        if !candidate.exists() {
            return candidate;
        }
    }

    unreachable!("u64 destination sequence exhausted")
}

fn file_system_error(
    operation: &'static str,
    path: &Path,
    error: std::io::Error,
) -> ExtractionError {
    ExtractionError::FileSystem {
        operation,
        path: path.to_path_buf(),
        message: error.to_string(),
    }
}

trait SourceCleaner {
    fn clean(&self, sources: &[PathBuf]) -> Result<(), String>;
}

struct TrashCleaner;

impl SourceCleaner for TrashCleaner {
    fn clean(&self, sources: &[PathBuf]) -> Result<(), String> {
        trash::delete_all(sources).map_err(|error| error.to_string())
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
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
        fn request_password(
            &self,
            _input: &Path,
            _previous_attempt_failed: bool,
        ) -> Option<PasswordResponse> {
            self.responses.lock().unwrap().pop_front()
        }
    }

    struct NoResponsePrompt;

    impl PasswordPrompt for NoResponsePrompt {
        fn request_password(
            &self,
            _input: &Path,
            _previous_attempt_failed: bool,
        ) -> Option<PasswordResponse> {
            None
        }
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

        let outcome = ExtractionWorkflow::with_source_cleaner(&seven_zip, RemoveSource)
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

        let outcome = ExtractionWorkflow::with_source_cleaner(&seven_zip, FailingSourceCleaner)
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

        let outcome = ExtractionWorkflow::with_source_cleaner(&seven_zip, RemoveSource)
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

        let outcome = ExtractionWorkflow::with_source_cleaner(&seven_zip, RemoveSource)
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

        let outcome = ExtractionWorkflow::with_source_cleaner(&seven_zip, RemoveSource)
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

        let outcome = ExtractionWorkflow::with_source_cleaner(&seven_zip, RemoveSource)
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
    fn symbolic_link_that_escapes_the_result_is_rejected() {
        use std::os::unix::fs::symlink;

        let seven_zip = prepared_seven_zip();
        assert!(
            seven_zip.is_file(),
            "run `cargo xtask prepare` before this test"
        );

        let sandbox = tempfile::tempdir().expect("create test sandbox");
        let link = sandbox.path().join("escape");
        let archive = sandbox.path().join("archive.7z");
        symlink("../outside", &link).expect("create escaping symlink");
        create_archive(&seven_zip, sandbox.path(), &archive, &["escape"]);
        std::fs::remove_file(&link).expect("remove source symlink");

        let result =
            ExtractionWorkflow::with_source_cleaner(&seven_zip, RemoveSource).extract(&archive);

        assert!(
            matches!(result, Err(ExtractionError::UnsafeOutput { .. })),
            "unexpected result: {result:?}"
        );
        assert!(archive.is_file(), "unsafe archive must be preserved");
        assert!(
            std::fs::symlink_metadata(&link).is_err(),
            "unsafe output must not be committed"
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

        let outcome = ExtractionWorkflow::with_adapters(&seven_zip, FailingSourceCleaner, prompt)
            .extract(&archive)
            .expect("extract encrypted archive");

        assert_eq!(std::fs::read(&payload).unwrap(), b"classified");
        assert!(archive.is_file(), "keep source must preserve the archive");
        assert!(outcome.warnings.is_empty(), "cleaner must not be called");
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

        ExtractionWorkflow::with_adapters(&seven_zip, RemoveSource, prompt)
            .extract(&archive)
            .expect("retry with the correct password");

        assert_eq!(std::fs::read(&payload).unwrap(), b"classified");
        assert!(!archive.exists(), "successful retry must clean the source");
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
        ExtractionWorkflow::with_adapters_and_password_store(
            &seven_zip,
            RemoveSource,
            first_prompt,
            &password_store,
        )
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

        ExtractionWorkflow::with_adapters_and_password_store(
            &seven_zip,
            RemoveSource,
            NoResponsePrompt,
            &password_store,
        )
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

        let outcome = ExtractionWorkflow::with_source_cleaner(&seven_zip, RemoveSource)
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

    fn create_split_archive(seven_zip: &Path, directory: &Path, archive: &Path, input: &str) {
        let status = Command::new(seven_zip)
            .current_dir(directory)
            .args(["a", "-t7z"])
            .arg(archive)
            .arg(input)
            .args(["-v1k", "-mx=0", "-bso0", "-bsp0"])
            .status()
            .expect("create split archive with 7-Zip");
        assert!(status.success(), "7-Zip must create split test archive");
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
}
