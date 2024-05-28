#![warn(clippy::pedantic)]
// REMEMBER: lsof | cut -d " " -f 1 | sort | uniq -c | sort -n -r | head
use anyhow::{anyhow, bail, Result};
use itertools::Itertools;
use lsof_rs::{Data, Filetype, StrLeakExt};
use tracing::info_span;
use tracing::level_filters::LevelFilter;
use tracing_coz::TracingCozBridge;
use tracing_subscriber::fmt::format::FmtSpan;
use tracing_subscriber::prelude::*;

use colored::Colorize;
use std::fmt::Display;
use std::marker::PhantomData;
use std::path::PathBuf;
use std::str::FromStr;

use clap::builder::TypedValueParser as _;
use clap::{arg, command, value_parser, ArgAction, Command, Parser, ValueEnum};

#[derive(Parser, Debug)] // requires `derive` feature
#[command(term_width = 0)] // Just to make testing across clap features easier
struct Args {
    /// Sort the entries of lsof,
    /// if not given then it is inferred based on group_by
    #[arg(short, long)]
    sort_by: Option<Sorting>,

    #[arg(short, long, default_value_t)]
    order: Ordering,

    #[arg(short, long, default_value_t)]
    group_by: GroupBy,

    #[arg(short = 'G', long, default_value_t, requires = "group_by")]
    group_fold: GroupFold,

    #[arg(short, long, group = "filter")]
    file: Option<PathBuf>,
    #[arg(short = 'F', long, group = "filter")]
    file_regex: Option<String>,
    #[arg(short, long, group = "filter")]
    pid: Option<u64>,
    #[arg(short = 'P', long, group = "filter")]
    proc_regex: Option<String>,

    #[clap( value_parser = Filetype::from_str)]
    #[arg(short = 't', long, default_value = "")]
    filetype: Filetype,

    #[clap(skip)]
    invalidate: PhantomData<Box<()>>,
}

// These should be
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

#[derive(Default, Clone, Copy, PartialEq, Eq, Debug, Hash, ValueEnum)]
enum Ordering {
    #[default]
    Descending,
    Ascending,
}

#[derive(Default, Clone, Copy, PartialEq, Eq, Debug, Hash, ValueEnum)]
enum GroupBy {
    #[default]
    None,
    File,
    Pid,
    Filetype,
}
#[derive(Default, Clone, Copy, PartialEq, Eq, Debug, Hash, ValueEnum)]
enum GroupFold {
    #[default]
    Count,
    // TODO: what else?
}

fn main() -> Result<()> {
    // TODO: timing, tracy and coz
    tracing_subscriber::registry()
        .with(LevelFilter::INFO)
        .with(tracing_subscriber::fmt::layer().with_span_events(FmtSpan::ACTIVE))
        // .with(TracingCozBridge::new())
        .init();

    let args = Args::parse();

    let _g = info_span!("argument processing");
    let order = args.order;
    let group_by = args.group_by;
    let sort_by = args.sort_by.unwrap_or(match group_by {
        GroupBy::None => Sorting::Filename,
        GroupBy::File => Sorting::NPids,
        GroupBy::Pid | GroupBy::Filetype => Sorting::NFiles,
    });
    let group_fold = args.group_fold;
    let filetypes = args.filetype;
    let filename = args.file.map_or(String::new(), |p| {
        p.into_os_string().into_string().expect("")
    });
    let lsof_all = filetypes == Filetype::All && filename.is_empty();

    // No longer access Cli Args
    #[allow(clippy::no_effect_underscore_binding)]
    let _invalidate = args.invalidate;
    drop(_g);

    let lsof = if lsof_all {
        Data::lsof_all()?
    } else {
        Data::lsof(filetypes, filename)?
    };

    let _g = info_span!("output");
    match group_by {
        GroupBy::None => output(lsof, sort_by, order)?,
        GroupBy::File => group_by_file(lsof, sort_by, order, group_fold)?,
        GroupBy::Pid => group_by_pid(lsof, sort_by, order, group_fold)?,
        GroupBy::Filetype => group_by_filetype(lsof, sort_by, order, group_fold)?,
    }

    Ok(())
}

#[tracing::instrument(skip(lsof), level = "info")]
fn output(lsof: Data, sort_by: Sorting, order: Ordering) -> Result<()> {
    match sort_by {
        Sorting::NPids => bail!("Sorting by npids not implemented"),
        Sorting::NFiles => bail!("Sorting by nfiles not implemented"),
        _ => {}
    }
    let mut all = lsof
        .all()
        .map(|(a, b, c)| (a, b, c.leak_str()))
        .collect_vec();
    match sort_by {
        Sorting::Filename => all.sort_unstable_by_key(|(_, _, f)| *f),
        Sorting::Pid => all.sort_unstable_by_key(|(p, _, _)| *p),
        Sorting::Filetype => {
            all.sort_unstable_by_key(|(_, _, f)| f.rsplit_once('.').unwrap_or((f, "")).1);
        }
        Sorting::ProcName => all.sort_unstable_by_key(|(_, name, _)| *name),
        Sorting::NPids => unimplemented!(),
        Sorting::NFiles => unimplemented!(),
    }

    Ok(())
}

#[tracing::instrument(skip(lsof), level = "info")]
fn group_by_file(
    lsof: Data,
    sort_by: Sorting,
    order: Ordering,
    group_fold: GroupFold,
) -> Result<()> {
    let map = lsof.files_to_pid();
    match sort_by {
        Sorting::Filename => todo!(),
        Sorting::Pid => bail!("Can't sort by pid when grouping by file (the pids are folded)"),
        Sorting::Filetype => todo!(),
        Sorting::ProcName => todo!(),
        Sorting::NPids => todo!(),
        Sorting::NFiles => bail!("Can't sort by # of files when grouping by file (its 1)"),
    }
}

#[tracing::instrument(skip(lsof), level = "info")]
fn group_by_pid(
    lsof: Data,
    sort_by: Sorting,
    order: Ordering,
    group_fold: GroupFold,
) -> Result<()> {
    let map = lsof.pid_to_files();
    match sort_by {
        Sorting::Filename => {
            bail!("Can't sort by filename when grouping by pid (the filenames are folded)")
        }
        Sorting::Pid => todo!(),
        Sorting::Filetype => todo!(),
        Sorting::ProcName => todo!(),
        Sorting::NPids => bail!("Can't sort by # of pids when grouping by pid (its 1)"),
        Sorting::NFiles => todo!(),
    }
}

#[tracing::instrument(skip(lsof), level = "info")]
fn group_by_filetype(
    lsof: Data,
    sort_by: Sorting,
    order: Ordering,
    group_fold: GroupFold,
) -> Result<()> {
    todo!()
}
impl Display for Sorting {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{n}", n = self.to_possible_value().unwrap().get_name())
    }
}
impl Display for GroupBy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{n}", n = self.to_possible_value().unwrap().get_name())
    }
}
impl Display for Ordering {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{n}", n = self.to_possible_value().unwrap().get_name())
    }
}
impl Display for GroupFold {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{n}", n = self.to_possible_value().unwrap().get_name())
    }
}
