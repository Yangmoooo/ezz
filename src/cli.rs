use clap::Parser;
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(
    name = "ezz",
    author = "Yangmoooo",
    version,
    about = "A very light wrapper around 7-Zip"
)]
pub struct Args {
    /// path to input file
    #[arg(index = 1, value_name = "FILE")]
    pub archive: PathBuf,

    /// specify password
    #[arg(short, long, value_name = "PASSWORD")]
    pub pw: Option<String>,

    /// path to password db
    #[arg(short, long, value_name = "FILE")]
    pub db: Option<PathBuf>,
}
