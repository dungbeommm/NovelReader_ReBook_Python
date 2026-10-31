use anyhow::{bail, Context, Result};
use serde_json::{json,Value};
use std::{io::{BufRead,BufReader,BufWriter,Write},path::Path,process::{Child,ChildStdin,ChildStdout,Command,Stdio}};

pub struct Ct2Engine { child:Child, input:BufWriter<ChildStdin>, output:BufReader<ChildStdout> }
impl Ct2Engine {
    pub fn spawn(binary:&Path,model_dir:&Path,source_spm:&Path,target_spm:&Path,device:&str,compute_type:&str,device_index:i32,intra_threads:usize)->Result<Self>{
        anyhow::ensure!(["cpu","cuda"].contains(&device),"CTranslate2 chỉ hỗ trợ CPU/CUDA; Intel XPU phải dùng backend OpenVINO riêng.");
        anyhow::ensure!(binary.is_file(),"Không tìm thấy C++ worker: {}",binary.display());
        anyhow::ensure!(model_dir.join("model.bin").is_file(),"Model CT2 chưa có model.bin.");
        anyhow::ensure!(source_spm.is_file()&&target_spm.is_file(),"Thiếu SentencePiece source/target.");
        let mut child=Command::new(binary).arg(model_dir).arg(source_spm).arg(target_spm).arg(device).arg(compute_type)
            .arg(device_index.to_string()).arg(intra_threads.to_string())
            .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::inherit()).spawn()
            .context("Không khởi chạy được CTranslate2 C++ worker")?;
        let input=BufWriter::new(child.stdin.take().context("CT2 worker thiếu stdin")?);
        let mut output=BufReader::new(child.stdout.take().context("CT2 worker thiếu stdout")?);
        let mut line=String::new();output.read_line(&mut line)?;
        let ready:Value=serde_json::from_str(&line).context("CT2 worker trả handshake không hợp lệ")?;
        if ready["ready"]!=true {let _=child.kill();let _=child.wait();bail!("Không nạp được CTranslate2 model: {}",ready["error"].as_str().unwrap_or("unknown"));}
        Ok(Self{child,input,output})
    }
    fn request(&mut self, request:Value)->Result<Value>{
        writeln!(self.input,"{}",request)?;self.input.flush()?;
        let mut line=String::new();self.output.read_line(&mut line)?;
        anyhow::ensure!(!line.is_empty(),"CTranslate2 worker đã đóng stdout.");
        Ok(serde_json::from_str(&line).context("CT2 worker trả JSON không hợp lệ")?)
    }
    pub fn token_count(&mut self,text:&str)->Result<usize>{
        let response=self.request(json!({"op":"count_tokens","text":text}))?;
        if response["ok"]!=true {bail!("Không đếm được token: {}",response["error"].as_str().unwrap_or("unknown"));}
        response["tokens"].as_u64().map(|v|v as usize).context("CT2 token_count response invalid")
    }
    pub fn translate(&mut self,text:&str,beam_size:usize,max_input:usize,max_output:usize,repetition_penalty:f32,newline_token:Option<&str>,opener_spacing:bool)->Result<String>{
        let request=json!({"op":"translate","text":text,"beam_size":beam_size.clamp(1,4),
            "max_input_tokens":max_input.max(1),"max_output_tokens":max_output.max(1),"repetition_penalty":repetition_penalty,
            "newline_token":newline_token.unwrap_or(""),"opener_spacing":opener_spacing});
        let response=self.request(request)?;
        if response["ok"]!=true {bail!("CTranslate2 inference lỗi: {}",response["error"].as_str().unwrap_or("unknown"));}
        Ok(response["text"].as_str().unwrap_or_default().to_owned())
    }
    pub fn is_running(&mut self)->bool{self.child.try_wait().ok().flatten().is_none()}
}
impl Drop for Ct2Engine {
    fn drop(&mut self){let _=writeln!(self.input,"{}",json!({"op":"shutdown"}));let _=self.input.flush();
        if self.child.try_wait().ok().flatten().is_none(){let _=self.child.kill();}let _=self.child.wait();}
}
