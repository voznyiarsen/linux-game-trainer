use std::fs;
use anyhow::{Result, Context};

/// Find processes matching the game configuration
pub fn find_process(config: &crate::config::ProcessConfig) -> Result<Vec<ProcessInfo>> {
    let mut processes = Vec::new();
    
    for entry in fs::read_dir("/proc")? {
        let entry = entry?;
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        
        if let Ok(pid) = name_str.parse::<i32>() {
            if is_matching_process(pid, config)? {
                if let Ok(info) = get_process_info(pid) {
                    processes.push(info);
                }
            }
        }
    }
    
    Ok(processes)
}

fn is_matching_process(pid: i32, config: &crate::config::ProcessConfig) -> Result<bool> {
    // Check command line
    let cmdline_path = format!("/proc/{}/cmdline", pid);
    if let Ok(cmdline) = fs::read_to_string(&cmdline_path) {
        for name in &config.names {
            let haystack = if config.case_insensitive { cmdline.to_lowercase() } else { cmdline.clone() };
            let needle = if config.case_insensitive { name.to_lowercase() } else { name.clone() };
            if haystack.contains(&needle) {
                return Ok(true);
            }
        }
    }
    
    // Check memory maps
    let maps_path = format!("/proc/{}/maps", pid);
    if let Ok(maps) = fs::read_to_string(&maps_path) {
        for name in &config.names {
            let haystack = if config.case_insensitive { maps.to_lowercase() } else { maps.clone() };
            let needle = if config.case_insensitive { name.to_lowercase() } else { name.clone() };
            if haystack.contains(&needle) {
                return Ok(true);
            }
        }
    }
    
    // Check exe symlink
    let exe_path = format!("/proc/{}/exe", pid);
    if let Ok(target) = fs::read_link(&exe_path) {
        let target_str = target.to_string_lossy();
        for name in &config.names {
            let haystack = if config.case_insensitive { target_str.to_lowercase() } else { target_str.to_string() };
            let needle = if config.case_insensitive { name.to_lowercase() } else { name.clone() };
            if haystack.contains(&needle) {
                return Ok(true);
            }
        }
    }
    
    Ok(false)
}

pub fn get_process_info(pid: i32) -> Result<ProcessInfo> {
    let mut info = ProcessInfo { pid, name: String::new(), cmdline: String::new() };
    
    if let Ok(comm) = fs::read_to_string(format!("/proc/{}/comm", pid)) {
        info.name = comm.trim().to_string();
    }
    
    if let Ok(cmdline) = fs::read_to_string(format!("/proc/{}/cmdline", pid)) {
        info.cmdline = cmdline.replace('\0', " ").trim().to_string();
    }
    
    Ok(info)
}

#[derive(Debug, Clone)]
pub struct ProcessInfo {
    pub pid: i32,
    pub name: String,
    pub cmdline: String,
}