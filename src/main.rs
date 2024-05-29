#![warn(clippy::pedantic)]
#![feature(iter_repeat_n)]
// REMEMBER: lsof | cut -d " " -f 1 | sort | uniq -c | sort -n -r | head
use anyhow::{bail, Result};
use itertools::Itertools;
use lsof::{buf_stdout, fmap, Data, FMap, Filetype, ProcInfo, StrLeakExt};
use tracing::info_span;
use tracing::level_filters::LevelFilter;
use tracing_subscriber::fmt::format::FmtSpan;
use tracing_subscriber::prelude::*;

use colored::Colorize;
use std::fmt::Display;
use std::hash::Hash;
use std::io::Write;
use std::iter::repeat_n;
use std::marker::PhantomData;
use std::path::PathBuf;
use std::str::FromStr;
use std::{cmp::Reverse, io::BufWriter};

use clap::{Parser, ValueEnum};

#[derive(Parser, Debug)] // requires `derive` feature
#[command(term_width = 0)] // Just to make testing across clap features easier
struct Args {
    /// Sort the entries of lsof,
    /// if not given then it is inferred based on `group_by`
    /// TODO: we should be able to sort by multiple
    #[arg(short, long)]
    sort_by: Option<Sorting>,

    #[arg(short, long, default_value_t)]
    order: Ordering,

    #[arg(short, long, default_value_t)]
    group_by: GroupBy,

    #[arg(short = 'G', long, default_value_t, requires = "group_by")]
    group_fold: GroupFold,

    #[arg(short = 'c', long)]
    total_count: bool,

    /// Exclude listing empty groups
    #[arg(short, long)]
    exclude_empty: bool,

    #[arg(short, long, group = "filter")]
    file: Option<PathBuf>,
    #[arg(short = 'F', long, group = "filter")]
    file_regex: Option<String>,
    #[arg(short, long, group = "filter")]
    pid: Option<u64>,
    #[arg(short = 'P', long, group = "filter")]
    proc_regex: Option<String>,

    #[arg(short = 't', long, default_value = "",  value_parser = Filetype::from_str)]
    filetype: Filetype,

    #[arg(skip)]
    invalidate: PhantomData<Box<()>>,

    #[arg(default_value = "1", long)]
    bench: usize,
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
    None,
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
    ProcName,
}
#[derive(Default, Clone, Copy, PartialEq, Eq, Debug, Hash, ValueEnum)]
enum GroupFold {
    #[default]
    Count,
    // TODO: List, // Comma separated list of entries
}

fn main() -> Result<()> {
    #[cfg(not(feature = "coz"))]
    tracing_subscriber();
    #[cfg(feature = "coz")]
    tracing_coz::TracingCozBridge::new().init();

    let args = Args::parse();

    let arg_proc_span = info_span!("argument processing");
    let order = args.order;
    let group_by = args.group_by;
    let sort_by = args.sort_by.unwrap_or(match group_by {
        GroupBy::None => Sorting::Filename,
        GroupBy::File => Sorting::NPids,
        GroupBy::Pid | GroupBy::Filetype | GroupBy::ProcName => Sorting::NFiles,
    });
    let group_fold = args.group_fold;
    let filetypes = args.filetype;
    let filename = args.file.map_or(String::new(), |p| {
        p.into_os_string().into_string().expect("")
    });
    let lsof_all = filetypes == Filetype::All && filename.is_empty();
    let exclude_empty = args.exclude_empty;
    let total_count = args.total_count;
    let o = OutputArgs {
        sort_by,
        order,
        group_fold,
        total_count,
        exclude_empty,
    };

    // No longer access Cli Args
    #[allow(clippy::no_effect_underscore_binding)]
    let _invalidate = args.invalidate;
    drop(arg_proc_span);

    for i in 0..args.bench {
        let lsof = if lsof_all {
            Data::lsof_all()?
        } else {
            let mut data = Data::lsof(filetypes)?;
            if !filename.is_empty() {
                data.invert_pid_to_files(&filename);
            }
            data
        };

        let _g = info_span!("output");
        // PERF: all the time is in the printing
        match group_by {
            GroupBy::None => output(lsof, o)?,
            GroupBy::File => group_by_file(lsof, o)?,
            GroupBy::Pid => group_by_pid(lsof, o)?,
            GroupBy::Filetype => group_by_filetype(lsof, o)?,
            GroupBy::ProcName => group_by_proc_name(lsof, o)?,
        }
    }

    Ok(())
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
struct OutputArgs {
    sort_by: Sorting,
    order: Ordering,
    group_fold: GroupFold,
    total_count: bool,
    exclude_empty: bool,
}

fn tracing_subscriber() {
    tracing_subscriber::registry()
        .with(LevelFilter::INFO)
        .with(tracing_subscriber::fmt::layer().with_span_events(FmtSpan::ACTIVE))
        // .with(TracingCozBridge::new())
        .init();
}

#[tracing::instrument(skip(lsof), level = "info")]
fn output(lsof: Data, OutputArgs { sort_by, order, .. }: OutputArgs) -> Result<()> {
    match sort_by {
        Sorting::NPids => bail!("Sorting by npids not implemented"),
        Sorting::NFiles => bail!("Sorting by nfiles not implemented"),
        _ => {}
    }
    let mut all = lsof.flattened().collect_vec();
    match (sort_by, order) {
        (Sorting::Filename, Ordering::Ascending) => all.sort_unstable_by_key(|e| e.file),
        (Sorting::Filename, Ordering::Descending) => all.sort_unstable_by_key(|e| Reverse(e.file)),
        (Sorting::Pid, Ordering::Ascending) => all.sort_unstable_by_key(|e| e.pid),
        (Sorting::Pid, Ordering::Descending) => all.sort_unstable_by_key(|e| Reverse(e.pid)),
        (Sorting::Filetype, Ordering::Ascending) => {
            all.sort_unstable_by_key(|e| e.get_ext());
        }
        (Sorting::Filetype, Ordering::Descending) => {
            all.sort_unstable_by_key(|e| Reverse(e.get_ext()));
        }
        (Sorting::ProcName, Ordering::Ascending) => all.sort_unstable_by_key(|e| e.proc),
        (Sorting::ProcName, Ordering::Descending) => all.sort_unstable_by_key(|e| Reverse(e.proc)),
        (Sorting::NPids | Sorting::NFiles, _) => {
            unreachable!("Handled above because we need to group")
        }
        (Sorting::None, _) => {}
    }
    let mut stdout = buf_stdout(all.iter());
    for lsof::Entry { pid, proc, file } in all {
        // TODO: prettify
        let file = if sort_by == Sorting::Filename {
            file.bold()
        } else {
            file.into()
        };
        let proc = if sort_by == Sorting::ProcName {
            proc.bold()
        } else {
            proc.into()
        };
        let pid = if sort_by == Sorting::Pid {
            pid.to_string().bold()
        } else {
            pid.to_string().into()
        };
        writeln!(stdout, "{pid} {proc} {file}")?;
    }

    Ok(())
}

#[tracing::instrument(skip(lsof), level = "info")]
fn group_by_file(
    lsof: Data,
    OutputArgs {
        sort_by,
        order,
        group_fold,
        total_count,
        exclude_empty,
    }: OutputArgs,
) -> Result<()> {
    let map = lsof.files_to_pid();
    match sort_by {
        Sorting::Filename => todo!(),
        Sorting::Pid => bail!("Can't sort by pid when grouping by file (the pids are folded)"),
        Sorting::Filetype => todo!(),
        Sorting::ProcName => todo!(),
        Sorting::NPids => todo!(),
        Sorting::NFiles => bail!("Can't sort by # of files when grouping by file (its 1)"),
        Sorting::None => todo!(),
    }
}

// #[tracing::instrument(skip(lsof), level = "info")]
fn group_by_pid(
    lsof: Data,
    OutputArgs {
        sort_by,
        order,
        group_fold,
        total_count,
        exclude_empty,
    }: OutputArgs,
) -> Result<()> {
    let map = lsof.into_pid_to_files();
    let map = fold_by_pid_w_count(map, |_, info| info.name);
    let mut stdout = buf_stdout(repeat_n((), 1024));
    let map = match sort_by {
        Sorting::Filename => {
            bail!("Can't sort by filename when grouping by pid (the filenames are folded)")
        }
        Sorting::NPids => bail!("Can't sort by # of pids when grouping by pid (its 1)"),
        Sorting::Filetype => {
            bail!("Can't sort by filetype when grouping by pid (the files are folded)")
        }
        Sorting::Pid => match group_fold {
            GroupFold::Count => map.sorted_unstable_by_key(|(pid, _, _)| *pid),
        },
        Sorting::ProcName => match group_fold {
            GroupFold::Count => map.sorted_unstable_by_key(|&(_, pname, _)| pname),
        },
        Sorting::NFiles => match group_fold {
            GroupFold::Count => map.sorted_unstable_by_key(|(_, _, nfiles)| *nfiles),
        },
        Sorting::None => match group_fold {
            GroupFold::Count => {
                for (pid, pname, nfiles) in map {
                    let pname = pname.unwrap_or("<noname>");
                    writeln!(stdout, "{pid} {pname} {nfiles}")?;
                }
                return Ok(());
            }
        },
    };
    print_map(order, map, |(pid, pname, nfiles)| {
        let pname = pname.unwrap_or("<noname>");
        writeln!(stdout, "{pid} {pname} {nfiles}")
    })?;
    Ok(())
}

// #[tracing::instrument(skip(lsof), level = "info")]
fn group_by_proc_name(
    lsof: Data,
    OutputArgs {
        sort_by,
        order,
        group_fold,
        total_count,
        exclude_empty,
    }: OutputArgs,
) -> Result<()> {
    let map = lsof.into_pid_to_files();
    let capacity = map.len();
    let mut stdout = buf_stdout(repeat_n((), 1024));

    match sort_by {
        Sorting::Filename => {
            bail!("Can't sort by filename when grouping by pid (the filenames are folded)")
        }
        Sorting::Filetype => {
            bail!("Can't sort by filetype when grouping by pid (the files are folded)")
        }
        Sorting::Pid => match group_fold {
            GroupFold::Count => {
                let map = fold_by_proc_name_w_count(
                    map,
                    capacity,
                    |pid, _| pid,
                    |minpid, pid, _| minpid.min(pid),
                );
                let map = map.into_iter().sorted_unstable_by_key(|(_, (pid, _))| *pid);
                print_map(order, map, |(pname, (pid, nfiles))| {
                    writeln!(stdout, "{pname} {pid} {nfiles}")
                })?;
            }
        },
        Sorting::ProcName => match group_fold {
            GroupFold::Count => {
                let map = fold_by_proc_name_w_count(map, capacity, |_, _| (), |(), _, _| ());
                let map = map.into_iter().sorted_unstable_by_key(|&(pname, _)| pname);
                print_map(order, map, |(pname, ((), nfiles))| {
                    writeln!(stdout, "{pname} {nfiles}")
                })?;
            }
        },
        Sorting::NPids => match group_fold {
            GroupFold::Count => {
                let map = fold_by_proc_name_w_count(map, capacity, |_, _| 1, |i, _, _| i + 1);
                let map = map
                    .into_iter()
                    .sorted_unstable_by_key(|&(_, (pids, nfiles))| (pids, nfiles));
                print_map(order, map, |(pname, (pids, nfiles))| {
                    writeln!(stdout, "{pname} {pids} {nfiles}")
                })?;
            }
        },
        Sorting::NFiles => match group_fold {
            GroupFold::Count => {
                let map = fold_by_proc_name_w_count(map, capacity, |_, _| (), |(), _, _| ());
                let map = map
                    .into_iter()
                    .sorted_unstable_by_key(|(_, nfiles)| *nfiles);
                print_map(order, map, |(pname, ((), nfiles))| {
                    writeln!(stdout, "{pname} {nfiles}")
                })?;
            }
        },
        Sorting::None => match group_fold {
            GroupFold::Count => {
                let map = fold_by_proc_name_w_count(map, capacity, |_, _| (), |(), _, _| ());
                for (pname, ((), nfiles)) in map {
                    writeln!(stdout, "{pname} {nfiles}")?;
                }
            }
        },
    };

    Ok(())
}

#[tracing::instrument(skip(lsof), level = "info")]
fn group_by_filetype(
    lsof: Data,
    OutputArgs {
        sort_by,
        order,
        group_fold,
        total_count,
        exclude_empty,
    }: OutputArgs,
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

fn fold_by_proc_name_w_count<T: Copy>(
    map: impl IntoIterator<Item = (u64, ProcInfo)>,
    capacity: usize,
    init: impl Fn(u64, ProcInfo) -> T,
    fold: impl Fn(T, u64, ProcInfo) -> T,
) -> FMap<&'static str, (T, usize)> {
    fold_pid_to_files_w_count(
        map,
        |pid, info| info.name.unwrap_or_else(|| pid.to_string().leak_str()),
        capacity,
        init,
        fold,
    )
}
fn fold_by_pid_w_count<T: Copy>(
    map: impl IntoIterator<Item = (u64, ProcInfo)>,
    f: impl Fn(u64, ProcInfo) -> T,
) -> impl Iterator<Item = (u64, T, usize)> {
    map.into_iter().map(move |(pid, info)| {
        let len = info.files.len();
        (pid, f(pid, info), len)
    })
}
fn fold_pid_to_files_w_count<T: Copy, K: Eq + Hash>(
    map: impl IntoIterator<Item = (u64, ProcInfo)>,
    key: impl Fn(u64, &ProcInfo) -> K,
    capacity: usize,
    init: impl Fn(u64, ProcInfo) -> T,
    fold: impl Fn(T, u64, ProcInfo) -> T,
) -> FMap<K, (T, usize)> {
    let map = map
        .into_iter()
        .fold(fmap(capacity), |mut proc_to, (pid, info)| {
            // TODO: if !info.files.is_empty() {
            match proc_to.entry(key(pid, &info)) {
                std::collections::hash_map::Entry::Occupied(o) => {
                    let (acc, count) = o.into_mut();
                    *count += info.files.len();
                    *acc = fold(*acc, pid, info);
                }
                std::collections::hash_map::Entry::Vacant(v) => {
                    let len = info.files.len();
                    v.insert((init(pid, info), len));
                }
            }
            // }
            proc_to
        });
    map
}
fn print_map<T>(
    order: Ordering,
    mut map: std::vec::IntoIter<T>,
    f: impl FnMut(T) -> std::io::Result<()>,
) -> Result<()> {
    match order {
        Ordering::Ascending => map.try_for_each(f)?,
        Ordering::Descending => map.rev().try_for_each(f)?,
    }
    Ok(())
}
