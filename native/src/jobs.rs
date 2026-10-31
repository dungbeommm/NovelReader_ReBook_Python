use anyhow::{Context,Result};
use serde_json::{json,Value};
use std::{path::{Path,PathBuf},sync::{Arc,Mutex,atomic::{AtomicBool,Ordering}},thread,time::{Duration,Instant}};
use rusqlite::Connection;
use crate::{config::AppConfig,ct2::Ct2Engine,db,store,translation::{self,ModelSpec}};

#[derive(Clone)]
pub struct JobManager {
 state:Arc<Mutex<Value>>,
 paused:Arc<AtomicBool>,
 stopped:Arc<AtomicBool>,
}
impl JobManager{
 pub fn new()->Self{Self{state:Arc::new(Mutex::new(json!({"status":"ready","message":"","done":0,"total":0,"active":false,"scope":""}))),paused:Arc::new(AtomicBool::new(false)),stopped:Arc::new(AtomicBool::new(false))}}
 pub fn summary(&self)->Value{self.state.lock().map(|s|s.clone()).unwrap_or(json!({"status":"error","message":"Job state lock lỗi","active":false}))}
 pub fn active(&self)->bool{self.summary()["active"].as_bool().unwrap_or(false)}
 pub fn begin(&self,total:usize,scope:String)->Result<()>{
  let mut state=self.state.lock().map_err(|_|anyhow::anyhow!("Job lock lỗi"))?;
  anyhow::ensure!(state["active"]!=true,"Đang có job dịch chạy.");
  self.paused.store(false,Ordering::SeqCst);self.stopped.store(false,Ordering::SeqCst);
  *state=json!({"status":"starting","message":"Đang chuẩn bị model native…","done":0,"total":total,"active":true,"scope":scope,"started_at":chrono::Utc::now().to_rfc3339()});
  Ok(())
 }
 pub fn update(&self,status:&str,message:&str,done:usize,total:usize){
  if let Ok(mut s)=self.state.lock(){*s=json!({"status":status,"message":message,"done":done,"total":total,"active":true,"scope":s["scope"].clone(),"started_at":s["started_at"].clone()});}
 }
 pub fn finish(&self,status:&str,message:&str){
  if let Ok(mut s)=self.state.lock(){let done=s["done"].clone();let total=s["total"].clone();let scope=s["scope"].clone();let started=s["started_at"].clone();
   *s=json!({"status":status,"message":message,"done":done,"total":total,"active":false,"scope":scope,"started_at":started,"finished_at":chrono::Utc::now().to_rfc3339()});}
  self.paused.store(false,Ordering::SeqCst);
 }
 pub fn pause(&self)->Result<()>{
  anyhow::ensure!(self.active(),"Không có job đang chạy.");
  self.paused.store(true,Ordering::SeqCst);
  if let Ok(mut s)=self.state.lock(){s["status"]="paused".into();s["message"]="Đã tạm dừng.".into();}
  Ok(())
 }
 pub fn resume(&self)->Result<()>{
  anyhow::ensure!(self.active(),"Không có job đang chạy.");
  self.paused.store(false,Ordering::SeqCst);
  if let Ok(mut s)=self.state.lock(){s["status"]="running".into();s["message"]="Tiếp tục dịch.".into();}
  Ok(())
 }
 pub fn stop(&self)->Result<()>{
  anyhow::ensure!(self.active(),"Không có job đang chạy.");
  self.stopped.store(true,Ordering::SeqCst);self.paused.store(false,Ordering::SeqCst);
  if let Ok(mut s)=self.state.lock(){s["status"]="stopping".into();s["message"]="Đang dừng sau chunk hiện tại…".into();}
  Ok(())
 }
 fn wait(&self)->bool{
  while self.paused.load(Ordering::SeqCst)&&!self.stopped.load(Ordering::SeqCst){thread::sleep(Duration::from_millis(100));}
  !self.stopped.load(Ordering::SeqCst)
 }
}
impl Default for JobManager{fn default()->Self{Self::new()}}

fn glossary_terms(raw:&str)->Vec<String>{
 raw.lines().filter_map(|line|{
  let line=line.trim();if line.is_empty()||line.starts_with('#'){return None;}
  let term=line.split(['\t','=','|']).next()?.trim();
  if term.is_empty(){None}else{Some(term.to_string())}
 }).collect()
}
fn translate_text(engine:&mut Ct2Engine,text:&str,spec:&ModelSpec,config:&AppConfig,terms:&[String],job:&JobManager,done:usize,total:usize)->Result<(String,Vec<String>,Vec<String>)>{
 let chunks=translation::split_for_ct2(text,spec.max_input,terms,|piece|engine.token_count(piece))?;
 let mut outputs=Vec::with_capacity(chunks.len());
 for chunk in &chunks{
  if !job.wait(){break;}
  let translated=engine.translate(chunk,config.beam_size.max(1),spec.max_input,spec.max_output,spec.repetition_penalty,spec.newline_token,spec.opener_spacing)?;
  outputs.push(translated);
  job.update("running",&format!("Đang dịch chunk {}/{} · chương {}",outputs.len(),chunks.len(),done+1),done,total);
 }
 let assembled=translation::postprocess(&translation::assemble(&chunks,&outputs)?);
 anyhow::ensure!(outputs.len()==chunks.len(),"Job bị dừng giữa chương; không lưu bản dịch chưa hoàn tất.");
 Ok((assembled,chunks,outputs))
}

pub fn run_translation(
 db_path:&Path,worker:&Path,model_dir:&Path,source_spm:&Path,target_spm:&Path,
 device:&str,compute_type:&str,threads:usize,spec:&'static ModelSpec,config:AppConfig,
 chapter_ids:Vec<i64>,job:JobManager
)->Result<()>{
 let mut conn=db::open(db_path)?;
 let mut engine=Ct2Engine::spawn(worker,model_dir,source_spm,target_spm,device,compute_type,0,threads)?;
 let terms=glossary_terms(&config.glossary);
 let total=chapter_ids.len();let mut done=0usize;
 for id in chapter_ids.iter().copied(){
  if !job.wait(){break;}
  let chapter=db::chapter(&conn,id)?.with_context(||format!("Không tìm thấy chương id={id}"))?;
  let title=chapter["title"].as_str().unwrap_or("").to_string();
  let source=chapter["source"].as_array().cloned().unwrap_or_default().into_iter().filter_map(|x|x.as_str().map(str::to_owned)).collect::<Vec<_>>();
  db::update_translation(&conn,id,"translating",&json!([]),&json!([]),"",&json!([]),"")?;
  let started=Instant::now();
  let translated=(||->Result<(String,Vec<String>,Vec<String>,Vec<String>)>{
   let (translated_title,title_chunks,title_outputs)=translate_text(&mut engine,&title,spec,&config,&terms,&job,done,total)?;
   let mut translated_paragraphs=Vec::with_capacity(source.len());let mut all_plan=Vec::new();let mut all_outputs=Vec::new();
   for (index,paragraph) in source.iter().enumerate(){
    if !job.wait(){break;}
    let (value,chunks,outputs)=translate_text(&mut engine,paragraph,spec,&config,&terms,&job,done,total)?;
    translated_paragraphs.push(value);
    all_plan.push(json!({"paragraph":index,"chunks":chunks}));
    all_outputs.extend(outputs);
   }
   anyhow::ensure!(translated_paragraphs.len()==source.len(),"Job bị dừng giữa chương; không lưu bản dịch chưa hoàn tất.");
   all_plan.insert(0,json!({"paragraph":-1,"chunks":title_chunks}));
   let mut flat=title_outputs;flat.extend(all_outputs);
   Ok((translated_title,translated_paragraphs,all_plan,flat))
  })();
  match translated{
   Ok((translated_title,translated_paragraphs,plan,outputs))=>{
    db::update_translation(&conn,id,"done",&json!(plan),&json!(outputs),&translated_title,&json!(translated_paragraphs),"")?;
    let _=store::export_chapter_text(&conn,id);
    let _=store::export_full_book_text(&conn);
    done+=1;
    job.update("running",&format!("Đã dịch xong chương {}/{} · {:.1}s",done,total,started.elapsed().as_secs_f64()),done,total);
   }
   Err(err)=>{
    let message=err.to_string();
    let _=db::update_translation(&conn,id,"error",&json!([]),&json!([]),"",&json!([]),&message);
    if job.summary()["status"]=="stopping"||job.stopped.load(Ordering::SeqCst){break;}
    job.update("running",&format!("Chương {id} lỗi: {message}"),done,total);
   }
  }
 }
 if job.stopped.load(Ordering::SeqCst){
  job.finish("stopped",&format!("Đã dừng. Hoàn tất {done}/{total} chương."));
 }else{
  let errors=db::all_chapters(&conn)?.iter().filter(|c|chapter_ids_contains(&chapter_ids,&c["id"] )&&c["status"]=="error").count();
  if errors>0{job.finish("done_with_errors",&format!("Đã xử lý {done}/{total} chương; {errors} chương lỗi."));}
  else{job.finish("done",&format!("Đã dịch xong {done}/{total} chương. TTS sẽ chạy sau khi native Piper được tích hợp."));}
 }
 Ok(())
}
fn chapter_ids_contains(ids:&[i64],id:&Value)->bool{id.as_i64().is_some_and(|v|ids.contains(&v))}
