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

#[test]
fn rar_volume_gap_is_reported_from_a_non_first_volume() {
    let sandbox = tempfile::tempdir().expect("create test sandbox");
    let first = sandbox.path().join("bundle.part1.rar");
    let selected = sandbox.path().join("bundle.part3.rar");
    let missing = sandbox.path().join("bundle.part2.rar");
    let seven_zip = sandbox.path().join("7zz");
    std::fs::write(&first, b"first volume").expect("create first volume");
    std::fs::write(&selected, b"third volume").expect("create selected volume");
    std::fs::write(&seven_zip, b"engine placeholder").expect("create engine placeholder");

    let result = ExtractionWorkflow::new(seven_zip).extract(&selected);

    assert_eq!(result, Err(ExtractionError::MissingVolume(missing)));
    assert!(first.is_file());
    assert!(selected.is_file());
}

#[test]
fn zip_volume_gap_is_reported_from_a_non_first_volume() {
    let sandbox = tempfile::tempdir().expect("create test sandbox");
    let first = sandbox.path().join("bundle.z01");
    let selected = sandbox.path().join("bundle.z03");
    let final_volume = sandbox.path().join("bundle.zip");
    let missing = sandbox.path().join("bundle.z02");
    let seven_zip = sandbox.path().join("7zz");
    std::fs::write(&first, b"first volume").expect("create first volume");
    std::fs::write(&selected, b"third volume").expect("create selected volume");
    std::fs::write(&final_volume, b"final volume").expect("create final volume");
    std::fs::write(&seven_zip, b"engine placeholder").expect("create engine placeholder");

    let result = ExtractionWorkflow::new(seven_zip).extract(&selected);

    assert_eq!(result, Err(ExtractionError::MissingVolume(missing)));
    assert!(first.is_file());
    assert!(selected.is_file());
    assert!(final_volume.is_file());
}
