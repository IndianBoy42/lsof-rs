#![warn(clippy::pedantic)]
#![feature(const_trait_impl)]
#![feature(hash_raw_entry)]
use anyhow::{anyhow, Result};
use glob::glob;
use itertools::chain;
use std::error::Error;
use std::fmt::Display;
use std::fs::read_to_string;
use std::str::FromStr;
use std::{fs, path::Component};

pub mod utils;
pub use utils::*;

// PERF: leak all the Strings for fun and profits
// No more String

#[derive(Clone)]
pub struct Data {
    // pid => info
    pid_to_files: FMap<u64, ProcInfo>,
    // file => pid
    files_to_pid: FMap<String, FdInfo>,
}

#[derive(Default, Debug, Clone)]
pub struct Proc {
    pub pid: u64,
    pub info: ProcInfo,
}
#[derive(Default, Debug, Clone)]
pub struct ProcInfo {
    pub name: Option<String>,
    pub files: FSet<String>,
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
    Data::lsof(Filetype::All, String::new())
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

    let data = Data::lsof(Filetype::All, path.clone())?;

    data.find(&path)
}
///get socket port used by process
#[tracing::instrument(level = "info")]
pub fn lsof_port(port: String) -> Result<Vec<Result<Proc, u64>>> {
    let path = format!("socket:[{port}]");
    let data = Data::lsof(Filetype::Socket, path.clone())?;
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
        Self { name, info }
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
            files_to_pid: fmap(0),
        }
    }

    pub fn all(self) -> impl Iterator<Item = (u64, &'static str, String)> {
        self.pid_to_files.into_iter().flat_map(|(pid, info)| {
            let name = info.name.map_or("<noname>", StrLeakExt::leak_str);
            info.files.into_iter().map(move |f| (pid, name, f))
        })
    }

    #[tracing::instrument(skip(self), level = "trace")]
    fn file_to_pid_insert(&mut self, fname: &str, pid: u64) {
        #[allow(clippy::enum_glob_use)]
        use std::collections::hash_map::RawEntryMut::*;
        let entry = self.files_to_pid.raw_entry_mut().from_key(fname);
        match entry {
            Occupied(o) => {
                o.into_mut().pids.insert(pid);
            }
            Vacant(v) => {
                v.insert(
                    fname.to_owned(),
                    FdInfo {
                        pids: [pid].into_iter().collect(),
                    },
                );
            }
        }
    }

    #[tracing::instrument(level = "info")]
    pub fn lsof_all() -> Result<Data> {
        // This should be inlined so that the target_* behaviour is completely removed
        Self::lsof(Filetype::All, String::new())
    }
    #[tracing::instrument(level = "info")]
    pub fn lsof(target_filetype: Filetype, target_filename: String) -> Result<Data> {
        let mut data = Data::new();
        // PERF: this glob can just be a read_dir
        let proc_paths = glob("/proc/*")?;
        // PERF: parallelize
        for proc in proc_paths {
            let proc = proc?;
            let (_pid_str, pid) = extract_pid_from_path(&proc);
            let Ok(pid) = pid else {
                continue;
            };

            // ISSUE: These osstring conversions with unwrap are bad
            let proc_path_str = proc.into_os_string().into_string().unwrap();

            //get process other info
            let other_info = get_pid_info(proc_path_str.clone());
            let name = other_info.get("Name").cloned();

            let (cap, files) = get_files_info(target_filetype, proc_path_str);
            let mut fileset = fset(cap.min(1));
            for file in files {
                // PERF: this shared map maybe a blocker to parallelism
                // Just extract it in a separate loop?
                if !target_filename.is_empty() && target_filename == file {
                    data.file_to_pid_insert(&target_filename, pid);
                } else {
                    data.file_to_pid_insert(&file, pid);
                }
                fileset.insert(file);
            }

            data.pid_to_files.insert(
                pid,
                ProcInfo {
                    name,
                    files: fileset,
                },
            );
        }
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
        let t = self
            .files_to_pid
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
    pub fn files_to_pid(&self) -> &FMap<String, FdInfo> {
        &self.files_to_pid
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
) -> (usize, impl Iterator<Item = String> + 'static) {
    let meminfo = target_filetype
        .includes_mem()
        .then(|| get_mem_info(proc_path_str.clone() + "/maps"))
        .into_iter()
        .flatten();
    let file = glob((proc_path_str + "/fd/*").as_str())
        .unwrap()
        .filter_map(std::result::Result::ok)
        .filter_map(|p| fs::read_link(p).ok())
        .map(|file| file.into_os_string().into_string().unwrap());
    let cap = meminfo.size_hint().0 + file.size_hint().0;
    let file = chain!(meminfo, file);
    (cap, file)
}

#[tracing::instrument(level = "trace")]
fn get_pid_info(path: String) -> FMap<String, String> {
    let path = path + "/status";
    // TODO: better parser for this
    // can make this lazy because we only need "Name"
    // Also, parse to a struct
    // also should use /stat/ because parsing
    let mut map: FMap<String, String> = fmap(0);
    if path.is_empty() {
        return map;
    }
    match read_to_string(path) {
        Ok(content) => {
            for line in content.lines() {
                if let Some((key, value)) = line.split_once(':') {
                    map.insert(key.trim().to_owned(), value.trim().to_owned());
                }
            }
        }
        Err(_) => {
            return map;
            // println!("Error reading file: {}", e);
        }
    }
    map
}

#[tracing::instrument(level = "trace")]
fn get_mem_info(proc_path_str: String) -> Vec<String> {
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
            (i >= 6 && !last.is_empty()).then(|| last.to_owned())
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
