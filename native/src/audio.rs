use anyhow::{bail, Context, Result};
use serde_json::{json, Value};
use std::{io::Write,path::{Path,PathBuf},process::{Command,Stdio}};
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


pub fn synthesize_piper(executable:&Path,text:&str,model:&Path,config:&Path,output:&Path,
    length_scale:f32,noise_scale:f32,noise_w:f32)->Result<()>{
    anyhow::ensure!(!text.trim().is_empty(),"Không có nội dung để tạo audio.");
    anyhow::ensure!(model.is_file(),"Không tìm thấy Piper ONNX voice: {}",model.display());
    anyhow::ensure!(config.is_file(),"Không tìm thấy Piper voice config: {}",config.display());
    anyhow::ensure!((0.4..=2.5).contains(&length_scale)&&(0.1..=2.0).contains(&noise_scale)&&(0.1..=2.0).contains(&noise_w),"Thông số Piper không hợp lệ.");
    if let Some(parent)=output.parent(){std::fs::create_dir_all(parent)?;}
    let mut child=Command::new(executable)
        .args(["--model"]).arg(model)
        .args(["--config"]).arg(config)
        .args(["--output_file"]).arg(output)
        .args(["--length_scale",&format!("{length_scale:.4}")])
        .args(["--noise_scale",&format!("{noise_scale:.4}")])
        .args(["--noise_w",&format!("{noise_w:.4}")])
        .stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::piped())
        .spawn().context("Không khởi chạy được Piper native CLI.")?;
    {
        let mut stdin=child.stdin.take().context("Piper không mở stdin")?;
        stdin.write_all(text.as_bytes())?;
        if !text.ends_with('\n'){stdin.write_all(b"\n")?;}
    }
    let result=child.wait_with_output()?;
    anyhow::ensure!(result.status.success(),"Piper lỗi: {}",String::from_utf8_lossy(&result.stderr));
    anyhow::ensure!(output.is_file()&&output.metadata()?.len()>44,"Piper không tạo được WAV hợp lệ.");
    Ok(())
}

pub fn chapter_audio_path(root:&Path,id:i64,position:i64)->Option<PathBuf>{
    for stem in [format!("chapter_{id}"),format!("chuong_{position:04}") ]{
        for ext in ["mp3","m4b","m4a","opus","wav","ogg"]{
            let p=root.join(format!("{stem}.{ext}"));if p.is_file(){return Some(p);}
        }
    }
    None
}

pub fn merge_range(root:&Path,chapters:&[Value],start_pos:i64,end_pos:i64,format:&str,bitrate:&str)->Result<Value>{
    anyhow::ensure!(start_pos>0&&end_pos>0,"Vị trí chương phải lớn hơn 0.");
    let (start,end)=if start_pos<=end_pos{(start_pos,end_pos)}else{(end_pos,start_pos)};
    let mut selected:Vec<_>=chapters.iter().filter(|c|{
        let p=c["position"].as_i64().unwrap_or(0);p>=start&&p<=end
    }).collect();
    selected.sort_by_key(|c|c["position"].as_i64().unwrap_or(0));
    anyhow::ensure!(!selected.is_empty(),"Không tìm thấy chương trong khoảng đã chọn.");
    let mut inputs=Vec::new();let mut missing=Vec::new();
    for c in &selected{
        let id=c["id"].as_i64().unwrap_or(0);let pos=c["position"].as_i64().unwrap_or(0);
        if let Some(p)=chapter_audio_path(root,id,pos){inputs.push(p);}else{missing.push(pos);}
    }
    anyhow::ensure!(!inputs.is_empty(),"Không có audio trong khoảng đã chọn.");
    let dir=root.join("audio_gop");std::fs::create_dir_all(&dir)?;
    let filename=format!("chuong_{start:04}_{end:04}.{format}");
    let output=dir.join(&filename);
    merge_audio_files(&inputs,&output,format,bitrate)?;
    Ok(json!({"ok":true,"filename":filename,"start_pos":start,"end_pos":end,
        "total_chapters":selected.len(),"merged_audio_count":inputs.len(),"missing_count":missing.len(),
        "missing_positions":missing.iter().take(10).collect::<Vec<_>>(),
        "size_mb":output.metadata()?.len() as f64/1048576.0,
        "url":format!("/api/audio/merged/{filename}"),"download_url":format!("/api/audio/merged/{filename}")}))
}

pub fn merge_selected(root:&Path,chapters:&[Value],ids:&[i64],format:&str,bitrate:&str)->Result<Value>{
    anyhow::ensure!(!ids.is_empty(),"Chưa chọn chương nào để gộp audio.");
    let selected_ids:std::collections::HashSet<i64>=ids.iter().copied().collect();
    let mut selected:Vec<_>=chapters.iter().filter(|c|selected_ids.contains(&c["id"].as_i64().unwrap_or(0))).collect();
    selected.sort_by_key(|c|c["position"].as_i64().unwrap_or(0));
    anyhow::ensure!(!selected.is_empty(),"Không tìm thấy các chương đã chọn.");
    let mut inputs=Vec::new();let mut missing=Vec::new();
    for c in &selected{
        let id=c["id"].as_i64().unwrap_or(0);let pos=c["position"].as_i64().unwrap_or(0);
        if let Some(p)=chapter_audio_path(root,id,pos){inputs.push(p);}else{missing.push(pos);}
    }
    anyhow::ensure!(!inputs.is_empty(),"Không có audio ở các chương đã chọn.");
    let start=selected.first().unwrap()["position"].as_i64().unwrap_or(0);
    let end=selected.last().unwrap()["position"].as_i64().unwrap_or(0);
    let dir=root.join("audio_gop");std::fs::create_dir_all(&dir)?;
    let filename=format!("chuong_{start:04}_{end:04}.{format}");
    let output=dir.join(&filename);merge_audio_files(&inputs,&output,format,bitrate)?;
    Ok(json!({"ok":true,"filename":filename,"start_pos":start,"end_pos":end,"total_chapters":selected.len(),
        "merged_audio_count":inputs.len(),"missing_count":missing.len(),"missing_positions":missing.iter().take(10).collect::<Vec<_>>(),
        "size_mb":output.metadata()?.len() as f64/1048576.0,"url":format!("/api/audio/merged/{filename}")}))
}

pub fn auto_group(root:&Path,chapters:&[Value],group_size:usize,format:&str,bitrate:&str)->Result<Vec<Value>>{
    anyhow::ensure!((2..=500).contains(&group_size),"Kích thước nhóm phải từ 2 đến 500 chương.");
    let mut sorted:Vec<_>=chapters.iter().collect();sorted.sort_by_key(|c|c["position"].as_i64().unwrap_or(0));
    let mut groups=Vec::new();
    for chunk in sorted.chunks(group_size){
        let with_audio=chunk.iter().filter(|c|chapter_audio_path(root,c["id"].as_i64().unwrap_or(0),c["position"].as_i64().unwrap_or(0)).is_some()).count();
        if with_audio==0{continue;}
        let start=chunk.first().unwrap()["position"].as_i64().unwrap_or(0);
        let end=chunk.last().unwrap()["position"].as_i64().unwrap_or(0);
        groups.push(merge_range(root,chapters,start,end,format,bitrate)?);
    }
    Ok(groups)
}

pub fn list_groups(root:&Path)->Result<Vec<Value>>{
    let dir=root.join("audio_gop");let mut rows=Vec::new();if !dir.exists(){return Ok(rows);}
    for entry in std::fs::read_dir(dir)?{
        let path=entry?.path();if !path.is_file(){continue;}
        let name=path.file_name().and_then(|x|x.to_str()).unwrap_or("");
        let Some(rest)=name.strip_prefix("chuong_") else{continue;};
        let Some((a,tail))=rest.split_once('_') else{continue;};
        let Some((b,ext))=tail.rsplit_once('.') else{continue;};
        let (Ok(start),Ok(end))=(a.parse::<i64>(),b.parse::<i64>()) else{continue;};
        rows.push(json!({"filename":name,"start_pos":start,"end_pos":end,"format":ext,
            "size_bytes":path.metadata()?.len(),"url":format!("/api/audio/merged/{name}")}));
    }
    rows.sort_by(|a,b|a["start_pos"].as_i64().cmp(&b["start_pos"].as_i64()));
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn format_validation_is_strict(){assert!(FORMATS.contains(&"mp3"));assert!(!FORMATS.contains(&"flac"));}
    #[test] fn audio_path_rejects_traversal(){assert!(safe_audio_path(Path::new("."),"../secret.mp3").is_err());}
}
