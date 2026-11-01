use serde_json::{json, Value};
use std::{path::{Path, PathBuf}, process::Command, time::{SystemTime, UNIX_EPOCH}};
use sysinfo::{Disks, System};

fn nvidia_devices() -> (Vec<Value>, Option<String>) {
    let output = Command::new("nvidia-smi")
        .args(["--query-gpu=index,name,utilization.gpu,memory.used,memory.total", "--format=csv,noheader,nounits"])
        .output();
    let Ok(output) = output else { return (vec![], Some("Không có NVIDIA hoặc nvidia-smi không khả dụng; không giả định GPU bằng 0%.".into())); };
    if !output.status.success() { return (vec![], Some("nvidia-smi không trả về số liệu.".into())); }
    let text = String::from_utf8_lossy(&output.stdout);
    let mut devices = Vec::new();
    for line in text.lines() {
        let cols: Vec<_> = line.splitn(5, ',').map(str::trim).collect();
        if cols.len() != 5 { continue; }
        let (Ok(index), Ok(util), Ok(used), Ok(total)) = (
            cols[0].parse::<u32>(), cols[2].parse::<f64>(), cols[3].parse::<f64>(), cols[4].parse::<f64>()
        ) else { continue; };
        devices.push(json!({"index":index.to_string(),"name":cols[1],"percent":util,
            "used_bytes":used*1024.0*1024.0,"total_bytes":total*1024.0*1024.0}));
    }
    let note = if devices.is_empty() { Some("Không nhận được số liệu NVIDIA.".into()) } else { None };
    (devices,note)
}

fn disk_info(paths: &[(String, PathBuf)]) -> Vec<Value> {
    let disks = Disks::new_with_refreshed_list();
    let mut result=Vec::new();
    let mut seen=std::collections::HashSet::new();
    for (label,path) in paths {
        let canonical = nearest_existing(path);
        if let Some(disk)=disks.iter().find(|d| canonical.starts_with(d.mount_point())) {
            let key=disk.mount_point().to_string_lossy().to_string();
            if !seen.insert(key.clone()) { continue; }
            let total=disk.total_space(); let free=disk.available_space(); let used=total.saturating_sub(free);
            result.push(json!({"label":label,"mount":key,"percent":if total>0 {Some(100.0*used as f64/total as f64)} else {None},
                "used_bytes":used,"total_bytes":total,"free_bytes":free}));
        }
    }
    result
}
fn nearest_existing(path:&Path)->PathBuf {
    let mut p=path.to_path_buf();
    while !p.exists() { if !p.pop() { return PathBuf::from("."); } }
    p.canonicalize().unwrap_or(p)
}

pub fn snapshot(paths:&[(String,PathBuf)]) -> Value {
    let mut system=System::new_all();
    system.refresh_cpu();
    system.refresh_memory();
    let cpu=system.global_cpu_info().cpu_usage() as f64;
    let total=system.total_memory();
    let used=system.used_memory();
    let ram=if total>0 {Some(100.0*used as f64/total as f64)} else {None};
    let (gpus,gpu_note)=nvidia_devices();
    let gpu_values:Vec<f64>=gpus.iter().filter_map(|g|g["percent"].as_f64()).collect();
    let gpu=if gpu_values.is_empty() {None} else {Some(gpu_values.iter().sum::<f64>()/gpu_values.len() as f64)};
    let all_vram=gpus.iter().all(|g|g["used_bytes"].is_number() && g["total_bytes"].as_f64().unwrap_or(0.0)>0.0);
    let used_vram:f64=gpus.iter().filter_map(|g|g["used_bytes"].as_f64()).sum();
    let total_vram:f64=gpus.iter().filter_map(|g|g["total_bytes"].as_f64()).sum();
    let vram=if all_vram && total_vram>0.0 {Some(100.0*used_vram/total_vram)} else {None};
    let disks=disk_info(paths);
    let disk=disks.first().cloned().unwrap_or(json!({"percent":null}));
    let now=SystemTime::now().duration_since(UNIX_EPOCH).map(|x|x.as_secs_f64()).unwrap_or(0.0);
    json!({"sampled_at":now,"warming_up":false,
        "cpu":{"percent":cpu,"logical_cores":system.cpus().len(),"scope":"host_visible"},
        "ram":{"percent":ram,"used_bytes":used,"total_bytes":total,"scope":"host_visible"},
        "gpu":{"percent":gpu},"vram":{"percent":vram,"used_bytes":if all_vram {Some(used_vram)} else {None},"total_bytes":if all_vram {Some(total_vram)} else {None}},
        "disk":disk,"gpus":gpus,"disks":disks,"gpu_note":gpu_note,
        "process_memory_bytes":std::process::id()})
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn returns_unknown_gpu_without_nvidia() {
        let (devices, note)=nvidia_devices();
        if devices.is_empty() { assert!(note.is_some()); }
    }
}
