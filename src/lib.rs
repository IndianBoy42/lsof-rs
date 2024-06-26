#![warn(clippy::pedantic)]
#![feature(const_trait_impl)]
#![feature(hash_raw_entry)]
#![feature(iter_collect_into)]
#![feature(anonymous_lifetime_in_impl_trait)]
use anyhow::{anyhow, Context, Result};
use glob::glob;
use itertools::{chain, Itertools};
use rayon::iter::{
    IntoParallelIterator, IntoParallelRefIterator, ParallelBridge, ParallelIterator,
};
use std::error::Error;
use std::fmt::Display;
use std::fs::read_to_string;
use std::str::FromStr;
use std::{fs, path::Component};

mod utils;
pub use utils::*;
mod procstat;
pub use procstat::*;

// PERF: leak all the Strings for fun and profits
// No more String

#[derive(Clone)]
pub struct Data {
    // pid => info
    pid_to_files: FMap<u64, ProcInfo>,
    // file => pid
    files_to_pid: Option<FMap<String, FdInfo>>, // PERF: leak this
}

#[derive(Default, Debug, Clone)]
pub struct Proc {
    pub pid: u64,
    pub info: ProcInfo,
}
#[derive(Default, Debug, Clone)]
pub struct ProcInfo {
    pub name: Option<&'static str>,
    pub files: FSet<&'static str>,
}
#[derive(Default, Debug, Clone)]
pub struct Fd {
    pub info: FdInfo,
    pub name: String,
}
#[derive(Default, Debug, Clone)]
pub struct FdInfo {
    pub pids: FSet<u64>, // PERF: can be Vec because small?
}
// TODO: bitflags
#[derive(Default, PartialEq, Clone, Copy, Eq, Debug)]
pub enum Filetype {
    #[default]
    All,
    Mem,
    Socket, // TODO: Not really used
    File,   // TODO: Not really used
    Extension(&'static str),
}

///get all infomation
#[tracing::instrument(level = "info")]
pub fn lsof() -> Result<Data> {
    Data::lsof(Filetype::All)
}
///get target info
#[tracing::instrument(level = "info")]
pub fn lsof_file(path: String) -> Result<Vec<Result<Proc, u64>>> {
    let metadata = fs::metadata(&path);
    //to do judge file type
    if let Ok(metadata) = metadata {
        let file_type = metadata.file_type();
        println!("File type {file_type:?}");
    }

    let data = Data::lsof(Filetype::All)?;

    data.find(&path)
}
///get socket port used by process
#[tracing::instrument(level = "info")]
pub fn lsof_port(port: String) -> Result<Vec<Result<Proc, u64>>> {
    let path = format!("socket:[{port}]");
    let data = Data::lsof(Filetype::Socket)?;
    data.find(&path)
}

impl From<(u64, ProcInfo)> for Proc {
    fn from((pid, info): (u64, ProcInfo)) -> Self {
        Self { pid, info }
    }
}
impl From<Proc> for ProcInfo {
    fn from(Proc { pid: _, info }: Proc) -> Self {
        info
    }
}
impl std::ops::Deref for Proc {
    type Target = ProcInfo;
    fn deref(&self) -> &Self::Target {
        &self.info
    }
}
impl From<(String, FdInfo)> for Fd {
    fn from((name, info): (String, FdInfo)) -> Self {
        Self { info, name }
    }
}
impl From<Fd> for FdInfo {
    fn from(Fd { name: _, info }: Fd) -> Self {
        info
    }
}
impl std::ops::Deref for Fd {
    type Target = FdInfo;
    fn deref(&self) -> &Self::Target {
        &self.info
    }
}

impl Default for Data {
    fn default() -> Self {
        Self::new()
    }
}

impl Filetype {
    const fn includes_mem(self) -> bool {
        matches!(self, Filetype::Mem) || matches!(self, Filetype::All)
    }
    const fn includes_socket(self) -> bool {
        matches!(self, Filetype::Socket) || matches!(self, Filetype::All)
    }
    const fn includes_file(self) -> bool {
        matches!(self, Filetype::File) || matches!(self, Filetype::All)
    }
}

impl Display for Filetype {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Filetype::Extension(s) => write!(f, ".{s}"),
            ft => write!(f, "{ft:?}"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BadFiletypeStr;
impl FromStr for Filetype {
    type Err = BadFiletypeStr;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s {
            "" | "all" => Filetype::All,
            "mem" => Filetype::Mem,
            "socket" => Filetype::Socket,
            "file" => Filetype::File,
            s if s.starts_with('.') => Filetype::Extension(s.trim_start_matches('.').leak_str()),
            _ => Err(BadFiletypeStr)?,
        })
    }
}
impl Error for BadFiletypeStr {}
impl Display for BadFiletypeStr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Bad filetype arg")
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Entry {
    pub pid: u64,
    pub proc: &'static str,
    pub file: &'static str,
}
impl Entry {
    fn from((pid, proc): (u64, ProcInfo)) -> impl Iterator<Item = Self> {
        let name = proc.name.map_or("<noname>", StrLeakExt::leak_str);
        proc.files.into_iter().map(move |f| Self {
            pid,
            proc: name,
            file: f.leak_str(),
        })
    }
    #[must_use]
    pub fn get_ext(&self) -> &'static str {
        self.file.rsplit_once('.').unwrap_or((self.file, "")).1
    }
}

impl Data {
    pub fn debug_fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Data")
            .field("pid_to_files", &self.pid_to_files)
            .field("files_to_pid", &self.files_to_pid)
            .finish()
    }

    fn new() -> Data {
        Data {
            pid_to_files: fmap(0),
            files_to_pid: None,
        }
    }

    pub fn flattened(self) -> impl Iterator<Item = Entry> {
        self.pid_to_files.into_iter().flat_map(Entry::from)
    }

    fn files_to_pid_mut(&mut self) -> &mut FMap<String, FdInfo> {
        self.as_mut().1
    }
    fn as_mut(&mut self) -> (&mut FMap<u64, ProcInfo>, &mut FMap<String, FdInfo>) {
        (
            &mut self.pid_to_files,
            self.files_to_pid.get_or_insert_with(|| fmap(0)),
        )
    }
    fn file_to_pid_insert(&mut self, fname: &str, pid: u64) {
        file_to_pid_insert(self.files_to_pid_mut(), fname, pid);
    }
    // #[tracing::instrument(skip(self, i), level = "trace")]
    fn file_to_pid_extend(&mut self, i: impl IntoIterator<Item = (&str, u64)>) {
        file_to_pid_extend(self.files_to_pid_mut(), i);
    }

    #[tracing::instrument(level = "info")]
    pub fn lsof_all() -> Result<Data> {
        // This should be inlined so that the target_* behaviour is completely removed
        Self::lsof(Filetype::All)
    }
    // #[tracing::instrument(level = "info")]
    pub fn lsof(target_filetype: Filetype) -> Result<Data> {
        let mut data = Data::new();
        // PERF: this glob can just be a read_dir
        let proc_paths = glob("/proc/*")?;
        // PERF: parallelize
        data.pid_to_files = proc_paths
            // .collect_vec()
            // .into_par_iter()
            .par_bridge()
            .filter_map(|proc| {
                let proc = proc.ok()?;
                let (_pid_str, pid) = extract_pid_from_path(&proc);
                Some((pid.ok()?, proc.into_os_string().into_string().unwrap()))
            })
            .map(|(pid, proc_path_str)| {
                //get process other info
                let name = get_pid_name(proc_path_str.clone());

                let (cap, files) = get_files_info(target_filetype, proc_path_str);
                let mut fileset = fset(cap.min(1));
                files.collect_into(&mut fileset);

                (
                    pid,
                    ProcInfo {
                        name,
                        files: fileset,
                    },
                )
            })
            .collect();
        Ok(data)
    }

    /// Description.
    /// Find a certain file in the lsof data and return the Info of the processes
    ///
    /// # Arguments
    ///
    /// * `path`: The file path to search for
    /// * `argument_name` - type and description.
    ///
    /// # Returns
    /// List of Processes opening this file, or an error if the path is not found
    ///
    /// # Errors
    /// anyhow: path not found
    ///
    /// # Examples
    /// ```rust
    /// write me later
    /// ```
    pub fn find(&self, path: &str) -> Result<Vec<Result<Proc, u64>>> {
        let files_to_pid = self
            .files_to_pid()
            .context("did not construct files_to_pid yet")?;
        let t = files_to_pid
            .get(path)
            .ok_or_else(|| anyhow!("{path} not found in lsof"))?;
        let result = t
            .pids
            .iter()
            .map(|s| {
                self.pid_to_files
                    .get(s)
                    .map(|p| (*s, p.clone()).into())
                    .ok_or(*s)
            })
            .collect();
        Ok(result)
    }

    #[must_use]
    pub fn pid_to_files(&self) -> &FMap<u64, ProcInfo> {
        &self.pid_to_files
    }

    #[must_use]
    pub fn files_to_pid(&self) -> Option<&FMap<String, FdInfo>> {
        self.files_to_pid.as_ref()
    }

    #[must_use]
    pub fn proc_to_files(&self) -> FMap<&'static str, (Vec<u64>, FSet<&'static str>)> {
        self.clone().into_proc_to_files()
    }

    #[must_use]
    pub fn into_pid_to_files(self) -> FMap<u64, ProcInfo> {
        self.pid_to_files
    }

    #[must_use]
    pub fn into_files_to_pid(mut self) -> FMap<String, FdInfo> {
        self.invert_pid_to_files("");
        self.files_to_pid.expect("We just constructed it")
    }

    #[must_use]
    pub fn into_proc_to_files(self) -> FMap<&'static str, (Vec<u64>, FSet<&'static str>)> {
        let map = self.into_pid_to_files();
        let mut proc_to_files = fmap(map.len());
        for (pid, ProcInfo { name, files }) in map {
            let (pids, fileset) = proc_to_files
                .entry(name.unwrap_or_else(|| pid.to_string().leak_str()))
                .or_insert_with(|| (vec![], fset(files.len())));
            fileset.extend(files);
            pids.push(pid);
        }
        proc_to_files
    }
    pub fn invert_pid_to_files(&mut self, target_filename: &str) {
        let (pid_to_files, files_to_pid) = self.as_mut();
        for (pid, info) in pid_to_files {
            let files = &info.files;
            if target_filename.is_empty() {
                file_to_pid_extend(files_to_pid, files.iter().map(|file| (*file, *pid)));
            } else {
                file_to_pid_extend(
                    files_to_pid,
                    files
                        .iter()
                        .filter(|&&file| target_filename == file)
                        .map(|file| (*file, *pid)),
                );
            }
        }
    }
}

fn file_to_pid_extend(
    files_to_pid: &mut FMap<String, FdInfo>,
    i: impl IntoIterator<Item = (&str, u64)>,
) {
    let i = i.into_iter();
    files_to_pid.reserve(i.size_hint().0);
    i.for_each(|(name, pid)| file_to_pid_insert(files_to_pid, name, pid));
}

fn file_to_pid_insert(files_to_pid: &mut FMap<String, FdInfo>, fname: &str, pid: u64) {
    #[allow(clippy::enum_glob_use)]
    use std::collections::hash_map::RawEntryMut::*;
    // PERF: hashing perf
    let entry = files_to_pid.raw_entry_mut().from_key(fname);
    match entry {
        Occupied(o) => {
            o.into_mut().pids.insert(pid);
        }
        Vacant(v) => {
            v.insert(
                fname.to_owned(), // PERF: hotspot
                FdInfo {
                    pids: [pid].into_iter().collect(), // PERF: hotspot
                },
            );
        }
    }
}

fn extract_pid_from_path(
    proc_path_r: &std::path::Path,
) -> (&str, std::result::Result<u64, std::num::ParseIntError>) {
    // let pid = proc_path_str.split('/').last().unwrap();
    let Component::Normal(pid) = proc_path_r.components().last().unwrap() else {
        unreachable!("As constructed this is a normal component");
    };
    let pid_str = pid.to_str().unwrap();
    // This is an assert
    let pid = pid_str.parse::<u64>();
    (pid_str, pid)
}

#[tracing::instrument(level = "trace")]
fn get_files_info(
    target_filetype: Filetype,
    proc_path_str: String,
) -> (usize, impl Iterator<Item = &'static str> + 'static) {
    let meminfo = target_filetype
        .includes_mem()
        .then(|| get_mem_info(proc_path_str.clone() + "/maps"))
        .into_iter()
        .flatten();
    // PERF: this glob can just be a read_dir, half the time is spent here
    let file = glob((proc_path_str + "/fd/*").as_str())
        .unwrap()
        .filter_map(std::result::Result::ok)
        .map(|p| {
            fs::read_link(&p) // PERF: almost half of the time, do it lazy
                .unwrap_or(p)
                .into_os_string()
                .into_string()
                .unwrap()
                .leak_str()
        }); // PERF: don't clone
    let cap = meminfo.size_hint().0 + file.size_hint().0;
    let file = chain!(meminfo, file);
    (cap, file)
}

#[tracing::instrument(level = "trace")]
fn get_mem_info(proc_path_str: String) -> Vec<&'static str> {
    let path = proc_path_str + "/maps";
    let Ok(content) = read_to_string(path) else {
        return Vec::new();
    };
    content
        .lines()
        .filter_map(|line| {
            let (i, Some(last)) = line
                .split(' ')
                .fold((0, None), |(i, _), b| (i + 1, Some(b)))
            else {
                return None;
            };
            (i >= 6 && !last.is_empty()).then(|| last.leak_str())
        })
        .collect()
}
// #[test]
// fn test_port() {
//     let port = "43869".to_owned();
//     let mut d = LsofData::new();
//     let result = d.port_ls(port);
//     println!("{:?}", result);
// }
#[cfg(test)]
mod tests;
