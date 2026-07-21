use ezz::{DesktopApplication, ExtractionError, ExtractionWorkflow, FileOutcome};

#[test]
fn every_input_produces_an_outcome_in_original_order() {
    let sandbox = tempfile::tempdir().expect("create test sandbox");
    let first_missing = sandbox.path().join("first-missing.zip");
    let directory = sandbox.path().join("directory.7z");
    let second_missing = sandbox.path().join("second-missing.rar");
    let seven_zip = sandbox.path().join("7zz");
    std::fs::create_dir(&directory).expect("create directory input");

    let workflow = ExtractionWorkflow::new(seven_zip);
    let report = DesktopApplication::new(workflow).process_files([
        first_missing.clone(),
        directory.clone(),
        second_missing.clone(),
    ]);

    assert_eq!(
        report.files,
        vec![
            FileOutcome {
                input: first_missing.clone(),
                result: Err(ExtractionError::InputNotFound(first_missing)),
            },
            FileOutcome {
                input: directory.clone(),
                result: Err(ExtractionError::InputNotFile(directory)),
            },
            FileOutcome {
                input: second_missing.clone(),
                result: Err(ExtractionError::InputNotFound(second_missing)),
            },
        ]
    );
}
