use ezz::{ExtractionError, ExtractionWorkflow};

#[test]
fn missing_input_is_rejected_with_its_original_path() {
    let sandbox = tempfile::tempdir().expect("create test sandbox");
    let missing = sandbox.path().join("missing.zip");

    let result = ExtractionWorkflow::new().extract(&missing);

    assert_eq!(result, Err(ExtractionError::InputNotFound(missing)));
}

#[test]
fn directory_input_is_rejected_with_its_original_path() {
    let sandbox = tempfile::tempdir().expect("create test sandbox");
    let directory = sandbox.path().join("archive.zip");
    std::fs::create_dir(&directory).expect("create directory input");

    let result = ExtractionWorkflow::new().extract(&directory);

    assert_eq!(result, Err(ExtractionError::InputNotFile(directory)));
}
