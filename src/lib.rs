#![feature(const_trait_impl)]
#![feature(hash_raw_entry)]
use anyhow::{anyhow, Result};
use glob::glob;
use itertools::chain;
use std::fs::read_to_string;
use std::{fs, path::Component};

pub type Filenames = FSet<String>;
pub type PidMap = FMap<u64, Procinfo>;

#[derive(Default, Debug, Clone)]
pub struct Fdinfo {
    pub pid: u64,
    pub name: Option<String>,
    pub files: Filenames,
}
#[derive(Default, Debug, Clone)]
pub struct Procinfo {
    name: Option<String>,
    files: Filenames,
}
#[derive(PartialEq, Clone, Copy, Eq, Debug)]
pub enum LsofFiletype {
    Mem,
    All,
    Socket,
}
#[derive(Debug, Clone)]
pub struct LsofData {
    // pid => info
    pid_to_files: PidMap,
    // file => pid
    files_to_pid: FMap<String, FSet<u64>>, // PERF: can be Vec because small?>
}

///get all infomation
#[tracing::instrument(level = "info")]
pub fn lsof() -> Result<LsofData> {
    LsofData::lsof(LsofFiletype::All, String::new())
}
///get target info
#[tracing::instrument(level = "info")]
pub fn lsof_file(path: String) -> Result<Vec<Fdinfo>> {
    let metadata = fs::metadata(&path);
    //to do judge file type
    if let Ok(metadata) = metadata {
        let file_type = metadata.file_type();
        println!("File type {:?}", file_type);
    }

    let data = LsofData::lsof(LsofFiletype::All, path.clone())?;

    data.find(path)
}
///get socket port used by process
#[tracing::instrument(level = "info")]
pub fn lsof_port(port: String) -> Result<Vec<Fdinfo>> {
    let path = format!("socket:[{}]", port);
    let data = LsofData::lsof(LsofFiletype::Socket, path.clone())?;
    data.find(path)
}

impl From<(u64, Procinfo)> for Fdinfo {
    fn from((pid, Procinfo { name, files }): (u64, Procinfo)) -> Self {
        Self { pid, name, files }
    }
}
impl From<Fdinfo> for Procinfo {
    fn from(
        Fdinfo {
            pid: _,
            name,
            files,
        }: Fdinfo,
    ) -> Self {
        Self { name, files }
    }
}

impl Default for LsofData {
    fn default() -> Self {
        Self::new()
    }
}

impl LsofFiletype {
    const fn includes_mem(self) -> bool {
        matches!(self, LsofFiletype::Mem) || matches!(self, LsofFiletype::All)
    }
}

impl LsofData {
    fn new() -> LsofData {
        LsofData {
            pid_to_files: fmap(0),
            files_to_pid: fmap(0),
        }
    }

    #[tracing::instrument(skip(self), level = "info")]
    fn file_to_pid(&mut self, fname: &str, pid: u64) {
        if true {
            use std::collections::hash_map::RawEntryMut::*;
            let entry = self.files_to_pid.raw_entry_mut().from_key(fname);
            match entry {
                Occupied(o) => {
                    o.into_mut().insert(pid);
                }
                Vacant(v) => {
                    v.insert(fname.to_owned(), [pid].into_iter().collect());
                }
            }
        } else {
            use std::collections::hash_map::Entry::*;
            let entry = self.files_to_pid.entry(fname.to_owned());
            match entry {
                Occupied(o) => {
                    o.into_mut().insert(pid);
                }
                Vacant(v) => {
                    v.insert([pid].into_iter().collect());
                }
            };
        }
    }

    #[tracing::instrument(level = "info")]
    pub fn lsof(target_filetype: LsofFiletype, target_filename: String) -> Result<LsofData> {
        let mut data = LsofData::new();
        let proc_paths = glob("/proc/*")?;
        // PERF: parallelize
        for proc in proc_paths {
            let proc = proc?;
            let (pid_str, pid) = extract_pid_from_path(&proc);
            let Ok(pid) = pid else {
                continue;
            };

            let proc_path_str = proc.into_os_string().into_string().unwrap();

            //get process other info
            let other_info = get_pid_info(proc_path_str.clone() + "/status");
            let name = other_info.get("Name").cloned();

            let (cap, files) = get_files_info(target_filetype, proc_path_str);
            let mut fileset = fset(cap.min(1));
            for file in files {
                // PERF: this shared map is a blocker to parallelism
                // Just extract it in a separate loop
                if !target_filename.is_empty() && target_filename == file {
                    data.file_to_pid(&target_filename, pid);
                } else {
                    data.file_to_pid(&file, pid)
                }
                fileset.insert(file);
            }

            data.pid_to_files.insert(
                pid,
                Procinfo {
                    name,
                    files: fileset,
                },
            );
        }
        Ok(data)
    }
    pub fn find(&self, path: String) -> Result<Vec<Fdinfo>> {
        let t = self
            .files_to_pid
            .get(&path)
            .ok_or_else(|| anyhow!("{path} not found in lsof"))?;
        let result = t
            .iter()
            .filter_map(|s| self.pid_to_files.get(s).map(|p| (*s, p.clone())))
            .map(Into::into)
            .collect();
        Ok(result)
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

#[tracing::instrument(level = "info")]
fn get_files_info(
    target_filetype: LsofFiletype,
    proc_path_str: String,
) -> (usize, impl Iterator<Item = String> + 'static) {
    let meminfo = target_filetype
        .includes_mem()
        .then(|| get_mem_info(proc_path_str.clone() + "/maps"))
        .into_iter()
        .flatten();
    let file = glob((proc_path_str + "/fd/*").as_str())
        .unwrap()
        .filter_map(|r| r.ok())
        .filter_map(|p| fs::read_link(p).ok())
        .map(|file| file.into_os_string().into_string().unwrap());
    let cap = meminfo.size_hint().0 + file.size_hint().0;
    let file = chain!(meminfo, file);
    (cap, file)
}

#[tracing::instrument(level = "info")]
fn get_pid_info(path: String) -> FMap<String, String> {
    // TODO: better parser for this
    // can make this lazy because we only need "Name"
    // Also, parse to a struct
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

#[tracing::instrument(level = "info")]
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

use fxhash::{FxHashMap, FxHashSet};
pub type FSet<T> = FxHashSet<T>;
pub type FMap<K, V> = FxHashMap<K, V>;
pub fn fmap<K, V>(cap: usize) -> FMap<K, V> {
    FMap::with_capacity_and_hasher(cap, std::hash::BuildHasherDefault::default())
}
pub fn fset<V>(cap: usize) -> FSet<V> {
    FSet::with_capacity_and_hasher(cap, std::hash::BuildHasherDefault::default())
}
