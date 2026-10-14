//! Process measurements for the long-run scenario: CPU time and memory of this process and of the child.

use std::time::Duration;

#[derive(Clone, Copy, Debug, Default)]
pub struct ProcessSample {
    /// Kernel plus user CPU time.
    pub cpu: Duration,
    /// Working set in bytes.
    pub working_set: usize,
    /// Committed private memory in bytes.
    pub private_bytes: usize,
}

#[cfg(windows)]
mod imp {
    use std::{mem::size_of, time::Duration};

    use windows_sys::Win32::{
        Foundation::{CloseHandle, FILETIME, HANDLE, STILL_ACTIVE},
        System::{
            ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS},
            Threading::{
                GetCurrentProcess, GetExitCodeProcess, GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
            },
        },
    };

    use super::ProcessSample;

    fn filetime_to_duration(time: FILETIME) -> Duration {
        let ticks = (u64::from(time.dwHighDateTime) << 32) | u64::from(time.dwLowDateTime);
        Duration::from_nanos(ticks * 100)
    }

    fn sample_handle(handle: HANDLE) -> Option<ProcessSample> {
        // SAFETY: the structures are zero-initialised plain data and the handle is valid for the call.
        unsafe {
            let (mut creation, mut exit, mut kernel, mut user) = (
                std::mem::zeroed::<FILETIME>(),
                std::mem::zeroed::<FILETIME>(),
                std::mem::zeroed::<FILETIME>(),
                std::mem::zeroed::<FILETIME>(),
            );
            if GetProcessTimes(handle, &mut creation, &mut exit, &mut kernel, &mut user) == 0 {
                return None;
            }
            let mut counters: PROCESS_MEMORY_COUNTERS = std::mem::zeroed();
            counters.cb = size_of::<PROCESS_MEMORY_COUNTERS>() as u32;
            if GetProcessMemoryInfo(handle, &mut counters, counters.cb) == 0 {
                return None;
            }
            Some(ProcessSample {
                cpu: filetime_to_duration(kernel) + filetime_to_duration(user),
                working_set: counters.WorkingSetSize,
                private_bytes: counters.PagefileUsage,
            })
        }
    }

    pub fn current() -> Option<ProcessSample> {
        // SAFETY: the pseudo handle needs no closing.
        sample_handle(unsafe { GetCurrentProcess() })
    }

    pub fn of_pid(pid: u32) -> Option<ProcessSample> {
        // SAFETY: the handle is closed before returning.
        unsafe {
            let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if handle.is_null() {
                return None;
            }
            let sample = sample_handle(handle);
            CloseHandle(handle);
            sample
        }
    }

    pub fn is_running(pid: u32) -> bool {
        // SAFETY: the handle is closed before returning.
        unsafe {
            let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if handle.is_null() {
                return false;
            }
            let mut code = 0u32;
            let known = GetExitCodeProcess(handle, &mut code) != 0;
            CloseHandle(handle);
            known && code == STILL_ACTIVE as u32
        }
    }
}

#[cfg(not(windows))]
mod imp {
    use super::ProcessSample;

    pub fn current() -> Option<ProcessSample> {
        None
    }

    pub fn of_pid(_pid: u32) -> Option<ProcessSample> {
        None
    }

    pub fn is_running(_pid: u32) -> bool {
        false
    }
}

/// CPU time and memory of this process.
pub fn current_process() -> Option<ProcessSample> {
    imp::current()
}

/// CPU time and memory of another process (the probed program).
pub fn process(pid: u32) -> Option<ProcessSample> {
    imp::of_pid(pid)
}

/// Whether the process still exists and has not exited.
pub fn is_running(pid: u32) -> bool {
    imp::is_running(pid)
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    #[test]
    fn this_process_has_cpu_time_and_memory() {
        let mut spin = 0u64;
        for i in 0..50_000_000u64 {
            spin = spin.wrapping_add(i);
        }
        std::hint::black_box(spin);
        let sample = current_process().expect("own process");
        assert!(sample.working_set > 1024 * 1024);
        assert!(sample.private_bytes > 0);
        assert!(sample.cpu > Duration::ZERO);
    }

    #[test]
    fn a_missing_pid_has_no_sample() {
        assert!(process(u32::MAX - 3).is_none());
        assert!(!is_running(u32::MAX - 3));
    }

    #[test]
    fn this_process_is_running() {
        assert!(is_running(std::process::id()));
    }
}
