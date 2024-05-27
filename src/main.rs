use anyhow::{anyhow, Result};
use lsof_rs::*;
use tracing::level_filters::LevelFilter;
use tracing_subscriber::prelude::*;

use std::path::PathBuf;

use clap::builder::TypedValueParser as _;
use clap::{arg, command, value_parser, ArgAction, Command, Parser, ValueEnum};

#[derive(Parser, Debug)] // requires `derive` feature
#[command(term_width = 0)] // Just to make testing across clap features easier
struct Args {
    sort: Sorting,

    #[clap(group = "filter")]
    file: PathBuf,
}

// https://github.com/clap-rs/clap/issues/2621
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash, ValueEnum)]
enum Sorting {
    Filename,
    Pid,
    Filetype,
    ProcName,
    NPids,
    NFiles,
}

fn main() -> Result<()> {
    // TODO: timing, tracy and coz
    tracing_subscriber::registry()
        .with(LevelFilter::INFO)
        .with(tracing_subscriber::fmt::layer())
        .init();

    let args = Args::parse();

    Ok(())
}
// REMEMBER: lsof | cut -d " " -f 1 | sort | uniq -c | sort -n -r | head
