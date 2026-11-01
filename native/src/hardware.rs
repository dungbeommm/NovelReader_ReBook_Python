use serde_json::{json,Value};
use std::{path::PathBuf,process::Command};
fn command_text(program:&str,args:&[&str])->Option<String>{
 let out=Command::new(program).args(args).output().ok()?;
 if !out.status.success(){return None;}
 let text=String::from_utf8_lossy(&out.stdout).trim().to_string();
 if text.is_empty(){None}else{Some(text)}
}
pub fn report()->Value{
 let mut system=sysinfo::System::new_all();system.refresh_cpu();system.refresh_memory();
 let cpu_name=system.cpus().first().map(|c|c.brand().trim().to_string()).filter(|s|!s.is_empty()).unwrap_or_else(||std::env::consts::ARCH.into());
 let cores=system.cpus().len().max(1);
 let nvidia=command_text("nvidia-smi",&["--query-gpu=index,name,memory.total","--format=csv,noheader,nounits"]);
 let nvidia_devices=nvidia.as_deref().map(|text|text.lines().filter_map(|line|{
  let cols:Vec<_>=line.splitn(3,',').map(str::trim).collect();if cols.len()!=3{return None;}
  Some(json!({"index":cols[0],"name":cols[1],"vram_mb":cols[2].parse::<u64>().ok()}))
 }).collect::<Vec<_>>()).unwrap_or_default();
 let sycl=command_text("sycl-ls",&[]);
 let clinfo=command_text("clinfo",&["-l"]);
 let intel_source=sycl.as_deref().or(clinfo.as_deref()).unwrap_or("");
 let intel_devices:Vec<Value>=intel_source.lines().filter(|line|{
  let lower=line.to_ascii_lowercase();lower.contains("intel")&&(lower.contains("gpu")||lower.contains("arc")||lower.contains("xe"))
 }).map(|line|json!({"name":line.trim()})).collect();
 let openvino=command_text("benchmark_app",&["--help"]).is_some()||command_text("ovc",&["--help"]).is_some();
 let worker=std::env::var_os("NOVEL_CT2_WORKER").map(PathBuf::from).unwrap_or_else(||{
  let mut p=PathBuf::from("native/cpp/build/novelreader-ct2-worker");if cfg!(windows){p.set_extension("exe");}p
 });
 let worker_ready=worker.is_file();
 let cuda_count=if worker_ready{
  Command::new(&worker).arg("--probe").output().ok().filter(|o|o.status.success())
   .and_then(|o|serde_json::from_slice::<Value>(&o.stdout).ok()).and_then(|v|v["cuda_device_count"].as_u64()).unwrap_or(0)
 }else{0};
 let cuda_runtime=cuda_count>0;
 json!({"cpu":{"name":cpu_name,"logical_cores":cores,"ram_total_bytes":system.total_memory()},
  "devices":[{"id":"cpu","name":"CPU","detected":true,"runtime_ready":true,"backend":"CTranslate2 CPU"},
   {"id":"cuda","name":"NVIDIA CUDA","detected":!nvidia_devices.is_empty(),"runtime_ready":cuda_runtime,
    "backend":"CTranslate2 CUDA","cuda_device_count":cuda_count,"reason":if !cuda_runtime{"Cần build CT2 worker có CUDA và driver NVIDIA hoạt động."}else{""}},
   {"id":"xpu","name":"Intel GPU / XPU","detected":!intel_devices.is_empty(),"runtime_ready":false,
    "backend":"OpenVINO / LibTorch XPU","reason":"Phát hiện thiết bị không đồng nghĩa model đã có graph OpenVINO/LibTorch XPU native."}],
  "nvidia":nvidia_devices,"intel":intel_devices,"openvino_runtime_detected":openvino,
  "ct2_worker_ready":worker_ready,"ct2_worker_path":worker.to_string_lossy(),
  "selected_device":std::env::var("NOVEL_CT2_DEVICE").unwrap_or_else(|_|"cpu".into()),
  "warnings":["CTranslate2 native hiện chỉ chạy CPU/CUDA.","Intel XPU chưa có model inference native được xác nhận; không tự fallback sang CPU."]})
}
