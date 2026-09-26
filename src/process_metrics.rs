//! Untimed resource diagnostics for the product process and all descendants.
use serde_json::{Value, json};

#[cfg(not(windows))]
pub fn idle_tree(_root: u32) -> Value {
    json!({"status":"unavailable", "reason":"Windows process counters required"})
}

#[cfg(windows)]
pub fn idle_tree(root: u32) -> Value {
    use std::{
        collections::BTreeSet,
        mem::size_of,
        time::{Duration, Instant},
    };
    use windows_sys::Win32::{
        Foundation::{CloseHandle, FILETIME, INVALID_HANDLE_VALUE},
        System::{
            Diagnostics::ToolHelp::{
                CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW,
                TH32CS_SNAPPROCESS,
            },
            ProcessStatus::{K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS},
            Threading::{
                GetProcessHandleCount, GetProcessTimes, OpenProcess,
                PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_VM_READ,
            },
        },
    };
    fn snapshot(root: u32) -> Option<Value> {
        // SAFETY: documented process snapshot API; owned handles close below.
        let handle = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
        if handle == INVALID_HANDLE_VALUE {
            return None;
        }
        let mut entry = PROCESSENTRY32W {
            dwSize: size_of::<PROCESSENTRY32W>() as u32,
            ..Default::default()
        };
        let mut all = Vec::new();
        let mut found = unsafe { Process32FirstW(handle, &mut entry) };
        while found != 0 {
            all.push((
                entry.th32ProcessID,
                entry.th32ParentProcessID,
                entry.cntThreads,
            ));
            found = unsafe { Process32NextW(handle, &mut entry) };
        }
        unsafe {
            CloseHandle(handle);
        }
        let mut ids = BTreeSet::from([root]);
        loop {
            let previous = ids.len();
            for &(pid, parent, _) in &all {
                if ids.contains(&parent) {
                    ids.insert(pid);
                }
            }
            if ids.len() == previous {
                break;
            }
        }
        let mut processes = Vec::new();
        let mut complete = true;
        for &(pid, parent, threads) in &all {
            if !ids.contains(&pid) {
                continue;
            }
            let process =
                unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_VM_READ, 0, pid) };
            if process.is_null() {
                complete = false;
                processes.push(json!({"pid":pid,"status":"unavailable"}));
                continue;
            }
            let mut created = FILETIME::default();
            let mut exited = FILETIME::default();
            let mut kernel = FILETIME::default();
            let mut user = FILETIME::default();
            let mut handles = 0;
            let mut memory = PROCESS_MEMORY_COUNTERS {
                cb: size_of::<PROCESS_MEMORY_COUNTERS>() as u32,
                ..Default::default()
            };
            let ok = unsafe {
                GetProcessTimes(process, &mut created, &mut exited, &mut kernel, &mut user) != 0
                    && GetProcessHandleCount(process, &mut handles) != 0
                    && K32GetProcessMemoryInfo(process, &mut memory, memory.cb) != 0
            };
            unsafe {
                CloseHandle(process);
            }
            if !ok {
                complete = false;
                processes.push(json!({"pid":pid,"status":"unavailable"}));
                continue;
            }
            let ticks =
                |v: FILETIME| (u64::from(v.dwHighDateTime) << 32) | u64::from(v.dwLowDateTime);
            processes.push(
                json!({"pid":pid,"parent_pid":parent,"created_100ns":ticks(created),
                "cpu_100ns":ticks(kernel)+ticks(user),"working_set_bytes":memory.WorkingSetSize,
                "handles":handles,"threads":threads,"status":"available"}),
            );
        }
        if !all.iter().any(|(pid, _, _)| *pid == root) {
            complete = false;
        }
        Some(json!({"complete":complete,"processes":processes}))
    }
    let before = snapshot(root);
    let started = Instant::now();
    std::thread::sleep(Duration::from_secs(1));
    let after = snapshot(root);
    let elapsed = started.elapsed().as_secs_f64();
    let mut cpu = 0u64;
    let mut stable = before.as_ref().is_some_and(|v| v["complete"] == true)
        && after.as_ref().is_some_and(|v| v["complete"] == true);
    if let (Some(before), Some(after)) = (&before, &after) {
        let previous = before["processes"].as_array().unwrap();
        let current = after["processes"].as_array().unwrap();
        stable &= previous.len() == current.len();
        for process in current {
            if let Some(old) = previous.iter().find(|old| {
                old["pid"] == process["pid"] && old["created_100ns"] == process["created_100ns"]
            }) {
                cpu += process["cpu_100ns"]
                    .as_u64()
                    .unwrap_or(0)
                    .saturating_sub(old["cpu_100ns"].as_u64().unwrap_or(0));
            } else {
                stable = false;
            }
        }
    }
    json!({"root_pid":root,"before":before,"after":after,"elapsed_seconds":elapsed,
        "status":if stable {"available"}else{"incomplete"},
        "cpu_percent_one_core":if stable {Some(cpu as f64/10_000_000.0/elapsed*100.0)}else{None},
        "wakeups":{"status":"unavailable","reason":"requires a separate scheduler/ETW trace; CPU counters are not wakeup counts"}})
}
