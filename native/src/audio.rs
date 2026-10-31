use anyhow::{bail, Context, Result};
use serde_json::{json, Value};
use std::{path::{Path,PathBuf}, process::Command, time::Duration};
use tempfile::TempDir;

const FORMATS:&[&str]=&["mp3","m4b","m4a","opus","wav"];
const BITRATES:&[&str]=&["64k","96k","128k","192k"];

pub fn convert_wav(input:&Path, output:&Path, format:&str, bitrate:&str, sample_rate:u32, speed:f32)->Result<()> {
    anyhow::ensure!(FORMATS.contains(&format),"Định dạng audio không hợp lệ.");
    anyhow::ensure!(BITRATES.contains(&bitrate),"Bitrate không hợp lệ.");
    anyhow::ensure!((0.4..=2.5).contains(&speed),"Tốc độ phải từ 0.4 đến 2.5.");
    anyhow::ensure!([0,22050,24000,44100,48000].contains(&sample_rate),"Tần số lấy mẫu không hợp lệ.");
    let mut cmd=Command::new("ffmpeg");
    cmd.args(["-v","error","-y","-threads","0","-i"]).arg(input);
    if speed!=1.0 {
        let mut remaining=speed;let mut filters=Vec::new();
        while remaining<0.5 {filters.push("atempo=0.5".to_string());remaining/=0.5;}
        while remaining>2.0 {filters.push("atempo=2".to_string());remaining/=2.0;}
        filters.push(format!("atempo={remaining:.6}"));
        cmd.args(["-af",&filters.join(",")]);
    }
    if sample_rate>0 {cmd.args(["-ar",&sample_rate.to_string()]);}
    match format {
        "mp3"=>{cmd.args(["-c:a","libmp3lame","-b:a",bitrate]);}
        "m4b"|"m4a"=>{cmd.args(["-c:a","aac","-b:a",bitrate]);}
        "opus"=>{cmd.args(["-c:a","libopus","-b:a",bitrate]);}
        "wav"=>{cmd.args(["-c:a","pcm_s16le"]);}
        _=>unreachable!()
    }
    cmd.arg(output);
    let result=cmd.output().context("Không tìm thấy ffmpeg hoặc không thể khởi chạy.")?;
    anyhow::ensure!(result.status.success(),"FFmpeg lỗi: {}",String::from_utf8_lossy(&result.stderr));
    anyhow::ensure!(output.is_file() && output.metadata()?.len()>0,"FFmpeg không tạo được file audio.");
    Ok(())
}

pub fn merge_audio_files(inputs:&[PathBuf], output:&Path, format:&str, bitrate:&str)->Result<()> {
    anyhow::ensure!(!inputs.is_empty(),"Danh sách file cần gộp rỗng.");
    anyhow::ensure!(FORMATS.contains(&format),"Định dạng audio không hợp lệ.");
    anyhow::ensure!(BITRATES.contains(&bitrate),"Bitrate không hợp lệ.");
    let temp=TempDir::new().context("Không tạo được thư mục tạm cho FFmpeg")?;
    let list=temp.path().join("concat.txt");
    let mut body=String::new();
    for p in inputs {
        anyhow::ensure!(p.is_file(),"Thiếu file audio: {}",p.display());
        let absolute=p.canonicalize()?;
        let escaped=absolute.to_string_lossy().replace('\\',"\\\\").replace("'","'\\\\''");
        body.push_str(&format!("file '{escaped}'\n"));
    }
    std::fs::write(&list,body)?;
    let temp_out=temp.path().join(format!("merged.{format}"));
    let mut cmd=Command::new("ffmpeg");
    cmd.args(["-v","error","-y","-f","concat","-safe","0","-i"]).arg(&list);
    match format {
        "mp3"=>{cmd.args(["-c:a","libmp3lame","-b:a",bitrate]);}
        "m4b"|"m4a"=>{cmd.args(["-c:a","aac","-b:a",bitrate]);}
        "opus"=>{cmd.args(["-c:a","libopus","-b:a",bitrate]);}
        "wav"=>{cmd.args(["-c:a","pcm_s16le"]);}
        _=>unreachable!()
    }
    cmd.arg(&temp_out);
    let result=cmd.output().context("Không tìm thấy ffmpeg hoặc không thể khởi chạy.")?;
    anyhow::ensure!(result.status.success(),"FFmpeg lỗi: {}",String::from_utf8_lossy(&result.stderr));
    anyhow::ensure!(temp_out.is_file() && temp_out.metadata()?.len()>0,"FFmpeg không tạo được file gộp.");
    if let Some(parent)=output.parent(){std::fs::create_dir_all(parent)?;}
    std::fs::copy(temp_out,output)?;
    Ok(())
}

pub fn media_type(path:&Path)->&'static str {
    match path.extension().and_then(|x|x.to_str()).unwrap_or("").to_ascii_lowercase().as_str() {
        "mp3"=>"audio/mpeg","m4b"|"m4a"=>"audio/mp4","wav"=>"audio/wav","opus"|"ogg"=>"audio/ogg",_=>"application/octet-stream"
    }
}

pub fn list_audio_files(root:&Path)->Result<Vec<Value>> {
    let mut out=Vec::new();
    if !root.exists(){return Ok(out);}
    for entry in walkdir::WalkDir::new(root).follow_links(false).max_depth(5) {
        let entry=entry?;
        if !entry.file_type().is_file(){continue;}
        let path=entry.path();
        if !FORMATS.contains(&path.extension().and_then(|x|x.to_str()).unwrap_or("").to_ascii_lowercase().as_str()){continue;}
        let relative=path.strip_prefix(root)?;
        out.push(json!({"filename":path.file_name().and_then(|x|x.to_str()).unwrap_or("audio"),
            "path":relative.to_string_lossy(),"size_bytes":entry.metadata()?.len()}));
    }
    out.sort_by(|a,b|a["path"].as_str().cmp(&b["path"].as_str()));
    Ok(out)
}

pub fn safe_audio_path(root:&Path, filename:&str)->Result<PathBuf> {
    anyhow::ensure!(!filename.is_empty() && !filename.contains('/') && !filename.contains('\\') && filename!="." && filename!="..","Tên file không hợp lệ.");
    let base=root.canonicalize()?;
    let direct=root.join(filename);
    let grouped=root.join("audio_gop").join(filename);
    for path in [direct,grouped] {
        if path.is_file() {
            let resolved=path.canonicalize()?;
            if resolved.starts_with(&base){return Ok(resolved);}
        }
    }
    bail!("Không tìm thấy file audio đã gộp.")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn format_validation_is_strict(){assert!(FORMATS.contains(&"mp3"));assert!(!FORMATS.contains(&"flac"));}
    #[test] fn audio_path_rejects_traversal(){assert!(safe_audio_path(Path::new("."),"../secret.mp3").is_err());}
}
