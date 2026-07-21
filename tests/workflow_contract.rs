use ezz::{ExtractionError, ExtractionWorkflow};

#[test]
fn missing_input_is_rejected_with_its_original_path() {
    let sandbox = tempfile::tempdir().expect("create test sandbox");
    let missing = sandbox.path().join("missing.zip");
    let seven_zip = sandbox.path().join("7zz");

    let result = ExtractionWorkflow::new(&seven_zip).extract(&missing);

    assert_eq!(result, Err(ExtractionError::InputNotFound(missing)));
}

#[test]
fn directory_input_is_rejected_with_its_original_path() {
    let sandbox = tempfile::tempdir().expect("create test sandbox");
    let directory = sandbox.path().join("archive.zip");
    let seven_zip = sandbox.path().join("7zz");
    std::fs::create_dir(&directory).expect("create directory input");

    let result = ExtractionWorkflow::new(&seven_zip).extract(&directory);

    assert_eq!(result, Err(ExtractionError::InputNotFile(directory)));
}

#[test]
fn missing_seven_zip_is_reported_for_an_existing_input() {
    let sandbox = tempfile::tempdir().expect("create test sandbox");
    let input = sandbox.path().join("archive.zip");
    let seven_zip = sandbox.path().join("7zz");
    std::fs::write(&input, b"not yet inspected").expect("create input");

    let result = ExtractionWorkflow::new(&seven_zip).extract(&input);

    assert_eq!(result, Err(ExtractionError::EngineNotFound(seven_zip)));
}
