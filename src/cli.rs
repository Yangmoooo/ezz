use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(name = "ezz", author = "Yangmoooo")]
#[command(version, propagate_version = true)]
#[command(about = "A very light wrapper around 7-Zip")]
pub struct Args {
    #[command(subcommand)]
    pub action: Action,
}

#[derive(Subcommand, Debug)]
pub enum Action {
    #[command(alias = "x")]
    #[command(about = "e[X]tract an archive")]
    Extract {
        /// path to input file
        #[arg(index = 1, value_name = "FILE")]
        archive: PathBuf,

        /// specify password
        #[arg(short, long, value_name = "PASSWORD")]
        pwd: Option<String>,

        /// path to password db
        #[arg(short, long, value_name = "FILE")]
        db: Option<PathBuf>,
    },

    #[command(alias = "a")]
    #[command(about = "[A]dd a password to the db")]
    Add {
        /// password to add
        #[arg(index = 1, value_name = "PASSWORD")]
        pwd: String,

        /// path to password db
        #[arg(short, long, value_name = "FILE")]
        db: Option<PathBuf>,
    },
}
