use ezz::{DesktopApplication, ExtractionError, FileOutcome};

#[test]
fn every_input_produces_an_outcome_in_original_order() {
    let sandbox = tempfile::tempdir().expect("create test sandbox");
    let unsupported = sandbox.path().join("plain.txt");
    let missing = sandbox.path().join("missing.zip");
    let directory = sandbox.path().join("directory.7z");
    std::fs::write(&unsupported, b"not an archive").expect("create unsupported input");
    std::fs::create_dir(&directory).expect("create directory input");

    let report = DesktopApplication::new().process_files([
        unsupported.clone(),
        missing.clone(),
        directory.clone(),
    ]);

    assert_eq!(
        report.files,
        vec![
            FileOutcome {
                input: unsupported.clone(),
                result: Err(ExtractionError::UnsupportedInput(unsupported)),
            },
            FileOutcome {
                input: missing.clone(),
                result: Err(ExtractionError::InputNotFound(missing)),
            },
            FileOutcome {
                input: directory.clone(),
                result: Err(ExtractionError::InputNotFile(directory)),
            },
        ]
    );
}
