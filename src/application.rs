use std::path::PathBuf;

use crate::{ExtractionError, ExtractionOutcome, ExtractionWorkflow};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileOutcome {
    pub input: PathBuf,
    pub result: Result<ExtractionOutcome, ExtractionError>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BatchReport {
    pub files: Vec<FileOutcome>,
}

#[derive(Debug, Default)]
pub struct DesktopApplication {
    workflow: ExtractionWorkflow,
}

impl DesktopApplication {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn process_files<I, P>(&self, inputs: I) -> BatchReport
    where
        I: IntoIterator<Item = P>,
        P: Into<PathBuf>,
    {
        let files = inputs
            .into_iter()
            .map(|input| {
                let input = input.into();
                let result = self.workflow.extract(&input);
                FileOutcome { input, result }
            })
            .collect();

        BatchReport { files }
    }
}
