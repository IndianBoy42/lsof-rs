use std::fs::read_to_string;

use crate::{fmap, FMap, StrLeakExt};
use anyhow::{Context, Result};

// https://github.com/eminence/procfs/blob/master/procfs-core/src/process/stat.rs
pub struct Stat {
    /// The process ID.
    pub pid: i32,
    /// The filename of the executable, without the parentheses.
    ///
    /// This is visible whether or not the executable is swapped out.
    ///
    /// Note that if the actual comm field contains invalid UTF-8 characters, they will be replaced
    /// here by the U+FFFD replacement character.
    pub comm: String,
    /// Process State.
    ///
    /// See [state()](#method.state) to get the process state as an enum.
    pub state: char,
    /// The PID of the parent of this process.
    pub ppid: i32,
    /// The process group ID of the process.
    pub pgrp: i32,
    /// The session ID of the process.
    pub session: i32,
    /// The controlling terminal of the process.
    ///
    /// The minor device number is contained in the combination of bits 31 to 20 and  7  to  0;
    /// the major device number is in bits 15 to 8.
    ///
    /// See [tty_nr()](#method.tty_nr) to get this value decoded into a (major, minor) tuple
    pub tty_nr: i32,
    /// The ID of the foreground process group of the controlling terminal of the process.
    pub tpgid: i32,
    /// The kernel flags  word of the process.
    ///
    /// For bit meanings, see the PF_* defines in  the  Linux  kernel  source  file
    /// [`include/linux/sched.h`](https://github.com/torvalds/linux/blob/master/include/linux/sched.h).
    ///
    /// See [flags()](#method.flags) to get a [`StatFlags`](struct.StatFlags.html) bitfield object.
    pub flags: u32,
    /// The number of minor faults the process has made which have not required loading a memory
    /// page from disk.
    pub minflt: u64,
    /// The number of minor faults that the process's waited-for children have made.
    pub cminflt: u64,
    /// The number of major faults the process has made which have required loading a memory page
    /// from disk.
    pub majflt: u64,
    /// The number of major faults that the process's waited-for children have made.
    pub cmajflt: u64,
    /// Amount of time that this process has been scheduled in user mode, measured in clock ticks
    /// (divide by `ticks_per_second()`).
    ///
    /// This includes guest time, guest_time (time spent running a virtual CPU, see below), so that
    /// applications that are not aware of the guest time field  do not lose that time from their
    /// calculations.
    pub utime: u64,
    /// Amount of time that this process has been scheduled in kernel mode, measured in clock ticks
    /// (divide by `ticks_per_second()`).
    pub stime: u64,
    /// Amount  of  time  that  this  process's  waited-for  children  have  been  scheduled  in
    /// user  mode,  measured  in clock ticks (divide by `ticks_per_second()`).
    ///
    /// This includes guest time, cguest_time (time spent running a virtual CPU, see below).
    pub cutime: i64,
    /// Amount of time that this process's waited-for  children  have  been  scheduled  in  kernel
    /// mode,  measured  in  clock  ticks  (divide  by `ticks_per_second()`).
    pub cstime: i64,
    /// For processes running a real-time scheduling policy (policy below; see sched_setscheduler(2)),
    /// this is the negated scheduling priority, minus one;
    ///
    /// That is, a number in the range -2 to -100,
    /// corresponding to real-time priority 1 to 99.  For processes running under a non-real-time
    /// scheduling policy, this is the raw nice value (setpriority(2)) as represented in the kernel.
    /// The kernel stores nice values as numbers in the range 0 (high) to 39  (low),  corresponding
    /// to the user-visible nice range of -20 to 19.
    /// (This explanation is for Linux 2.6)
    ///
    /// Before Linux 2.6, this was a scaled value based on the scheduler weighting given to this process.
    pub priority: i64,
    /// The nice value (see `setpriority(2)`), a value in the range 19 (low priority) to -20 (high priority).
    pub nice: i64,
    /// Number  of  threads in this process (since Linux 2.6).  Before kernel 2.6, this field was
    /// hard coded to 0 as a placeholder for an earlier removed field.
    pub num_threads: i64,
    /// The time in jiffies before the next SIGALRM is sent to the process due to an interval
    /// timer.
    ///
    /// Since kernel 2.6.17, this  field is no longer maintained, and is hard coded as 0.
    pub itrealvalue: i64,
    /// The time the process started after system boot.
    ///
    /// In kernels before Linux 2.6, this value was expressed in  jiffies.  Since  Linux 2.6, the
    /// value is expressed in clock ticks (divide by `sysconf(_SC_CLK_TCK)`).
    ///
    pub starttime: u64,
    /// Virtual memory size in bytes.
    pub vsize: u64,
    /// Resident Set Size: number of pages the process has in real memory.
    ///
    /// This is just the pages which count toward text,  data,  or stack space.
    /// This does not include pages which have not been demand-loaded in, or which are swapped out.
    pub rss: u64,
    /// Current soft limit in bytes on the rss of the process; see the description of RLIMIT_RSS in
    /// getrlimit(2).
    pub rsslim: u64,
    /// The address above which program text can run.
    pub startcode: u64,
    /// The address below which program text can run.
    pub endcode: u64,
    /// The address of the start (i.e., bottom) of the stack.
    pub startstack: u64,
    /// The current value of ESP (stack pointer), as found in the kernel stack page for the
    /// process.
    pub kstkesp: u64,
    /// The current EIP (instruction pointer).
    pub kstkeip: u64,
    /// The  bitmap of pending signals, displayed as a decimal number.  Obsolete, because it does
    /// not provide information on real-time signals; use `/proc/<pid>/status` instead.
    pub signal: u64,
    /// The bitmap of blocked signals, displayed as a decimal number.  Obsolete, because it does
    /// not provide information on  real-time signals; use `/proc/<pid>/status` instead.
    pub blocked: u64,
    /// The  bitmap of ignored signals, displayed as a decimal number.  Obsolete, because it does
    /// not provide information on real-time signals; use `/proc/<pid>/status` instead.
    pub sigignore: u64,
    /// The bitmap of caught signals, displayed as a decimal number.  Obsolete, because it does not
    /// provide information  on  real-time signals; use `/proc/<pid>/status` instead.
    pub sigcatch: u64,
    /// This  is  the  "channel"  in which the process is waiting.  It is the address of a location
    /// in the kernel where the process is sleeping.  The corresponding symbolic name can be found in
    /// `/proc/<pid>/wchan`.
    pub wchan: u64,
    /// Number of pages swapped **(not maintained)**.
    pub nswap: u64,
    /// Cumulative nswap for child processes **(not maintained)**.
    pub cnswap: u64,
    /// Signal to be sent to parent when we die.
    ///
    /// (since Linux 2.1.22)
    pub exit_signal: Option<i32>,
    /// CPU number last executed on.
    ///
    /// (since Linux 2.2.8)
    pub processor: Option<i32>,
    /// Real-time scheduling priority
    ///
    ///  Real-time scheduling priority, a number in the range 1 to 99 for processes scheduled under a real-time policy, or 0, for non-real-time processes
    ///
    /// (since Linux 2.5.19)
    pub rt_priority: Option<u32>,
    /// Scheduling policy (see sched_setscheduler(2)).
    ///
    /// Decode using the `SCHED_*` constants in `linux/sched.h`.
    ///
    /// (since Linux 2.5.19)
    pub policy: Option<u32>,
    /// Aggregated block I/O delays, measured in clock ticks (centiseconds).
    ///
    /// (since Linux 2.6.18)
    pub delayacct_blkio_ticks: Option<u64>,
    /// Guest time of the process (time spent running a virtual CPU for a guest operating system),
    /// measured in clock ticks (divide by `ticks_per_second()`)
    ///
    /// (since Linux 2.6.24)
    pub guest_time: Option<u64>,
    /// Guest time of the process's children, measured in clock ticks (divide by
    /// `ticks_per_second()`).
    ///
    /// (since Linux 2.6.24)
    pub cguest_time: Option<i64>,
    /// Address above which program initialized and uninitialized (BSS) data are placed.
    ///
    /// (since Linux 3.3)
    pub start_data: Option<u64>,
    /// Address below which program initialized and uninitialized (BSS) data are placed.
    ///
    /// (since Linux 3.3)
    pub end_data: Option<u64>,
    /// Address above which program heap can be expanded with brk(2).
    ///
    /// (since Linux 3.3)
    pub start_brk: Option<u64>,
    /// Address above which program command-line arguments (argv) are placed.
    ///
    /// (since Linux 3.5)
    pub arg_start: Option<u64>,
    /// Address below program command-line arguments (argv) are placed.
    ///
    /// (since Linux 3.5)
    pub arg_end: Option<u64>,
    /// Address above which program environment is placed.
    ///
    /// (since Linux 3.5)
    pub env_start: Option<u64>,
    /// Address below which program environment is placed.
    ///
    /// (since Linux 3.5)
    pub env_end: Option<u64>,
    /// The thread's exit status in the form reported by waitpid(2).
    ///
    /// (since Linux 3.5)
    pub exit_code: Option<i32>,
}

#[tracing::instrument(level = "trace")]
#[must_use]
pub fn get_pid_info_status(path: String) -> FMap<String, String> {
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

// https://github.com/heim-rs/heim/issues/154

/// Get the name for a specific pid (given as the proc path `/proc/{pid}`)
#[tracing::instrument(level = "trace")]
#[must_use]
pub fn get_pid_name(path: String) -> Option<&'static str> {
    let path = path + "/stat";
    let stat = read_to_string(path).ok()?;
    let stat = stat.trim();
    let (_, name) = stat.split_once('(')?;
    let (name, _) = name.split_once(')')?;

    Some(name.leak_str())
}
#[tracing::instrument(level = "trace")]
#[must_use]
pub fn get_pid_cmdline(path: String) -> Option<&'static str> {
    let path = path + "/cmdline";
    let stat = read_to_string(path).ok()?;
    let stat = stat.trim();
    let (_, name) = stat.split_once(' ')?;

    Some(name.leak_str())
}
#[tracing::instrument(level = "trace")]
#[must_use]
pub fn get_pid_name_status(proc_path_str: String) -> Option<&'static str> {
    let other_info = get_pid_info_status(proc_path_str.clone());
    other_info.get("Name").cloned().map(StrLeakExt::leak_str)
}
