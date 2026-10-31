use anyhow::{bail, Result};
use serde::{Deserialize,Serialize};
use serde_json::Value;
use std::{fs,path::{Path,PathBuf}};
#[derive(Debug,Clone,Serialize,Deserialize)]
#[serde(default)]
pub struct AppConfig {
 pub reset:bool,pub cloud_checkpoint:bool,pub cloud_checkpoint_every:usize,pub cloud_checkpoint_minutes:usize,pub checkpoint_include_audio:bool,pub auto_group_audio:bool,
 pub engine:String,pub backend:String,pub glossary:String,pub batch_size:usize,pub honorific_mode:String,
 pub pronoun_harmonizer:bool,pub normalize_t2s:bool,pub normalize_mode:String,pub beam_size:usize,pub chunk_mode:String,
 pub generate_audio:bool,pub audio_pipeline:bool,pub audio_workers:usize,pub audio_queue_size:usize,
 pub tts_engine:String,pub voice_id:String,pub audio_speed:f32,pub noise_scale:f32,pub noise_w_scale:f32,
 pub audio_format:String,pub audio_bitrate:String,pub audio_sample_rate:u32,pub vieneu_temperature:f32,pub vieneu_top_p:f32,pub vieneu_batch_size:usize,
 pub group_size:usize,pub start_pos:Option<usize>,pub end_pos:Option<usize>,pub chapter_ids:Option<Vec<i64>>,pub force_retranslate:bool,
}
impl Default for AppConfig {
 fn default()->Self{Self{reset:false,cloud_checkpoint:false,cloud_checkpoint_every:10,cloud_checkpoint_minutes:10,checkpoint_include_audio:false,auto_group_audio:false,engine:"HachimiMT-60-QT".into(),backend:"ct2".into(),glossary:String::new(),batch_size:8,honorific_mode:"safe".into(),pronoun_harmonizer:true,normalize_t2s:true,normalize_mode:"auto".into(),beam_size:2,chunk_mode:"sentence".into(),generate_audio:true,audio_pipeline:true,audio_workers:1,audio_queue_size:4,tts_engine:"piper".into(),voice_id:"ngochuyennew".into(),audio_speed:1.0,noise_scale:0.667,noise_w_scale:0.8,audio_format:"mp3".into(),audio_bitrate:"96k".into(),audio_sample_rate:0,vieneu_temperature:0.8,vieneu_top_p:0.95,vieneu_batch_size:8,group_size:50,start_pos:None,end_pos:None,chapter_ids:None,force_retranslate:false}}
}
impl AppConfig{
 pub fn validate(&mut self)->Result<()>{
  anyhow::ensure!(["ct2","transformers","pytorch"].contains(&self.backend.as_str()),"Backend không hợp lệ.");
  anyhow::ensure!(["sentence","paragraph"].contains(&self.chunk_mode.as_str()),"Chunk mode không hợp lệ.");
  anyhow::ensure!(["safe","xianxia_strict","off"].contains(&self.honorific_mode.as_str()),"Honorific mode không hợp lệ.");
  anyhow::ensure!((1..=1024).contains(&self.batch_size)&&(1..=4).contains(&self.beam_size),"Batch/beam size ngoài giới hạn.");
  anyhow::ensure!((2..=500).contains(&self.group_size)&&(1..=8).contains(&self.audio_workers)&&(1..=32).contains(&self.audio_queue_size),"Cấu hình hàng đợi không hợp lệ.");
  anyhow::ensure!((0.4..=2.5).contains(&self.audio_speed)&&(0.1..=2.0).contains(&self.noise_scale)&&(0.1..=2.0).contains(&self.noise_w_scale),"Thông số TTS không hợp lệ.");
  anyhow::ensure!(["mp3","m4b","m4a","wav","opus"].contains(&self.audio_format.as_str())&&["64k","96k","128k","192k"].contains(&self.audio_bitrate.as_str()),"Định dạng audio/bitrate không hợp lệ.");
  anyhow::ensure!([0,22050,24000,44100,48000].contains(&self.audio_sample_rate),"Tần số lấy mẫu không hợp lệ.");
  anyhow::ensure!(["piper","vieneu"].contains(&self.tts_engine.as_str()),"Engine TTS không hợp lệ.");
  if self.normalize_mode=="none"{self.normalize_t2s=false;}else if self.normalize_mode=="t2s"{self.normalize_t2s=true;}
  if self.audio_format=="opus"{self.audio_sample_rate=48000;}
  anyhow::ensure!(!(self.audio_format=="mp3"&&self.audio_bitrate=="192k"&&[22050,24000].contains(&self.audio_sample_rate)),"MP3 192 kbps cần 44.1 hoặc 48 kHz.");
  if let (Some(a),Some(b))=(self.start_pos,self.end_pos){anyhow::ensure!(b>=a,"Chương bắt đầu không được lớn hơn chương kết thúc.");}
  anyhow::ensure!(self.chapter_ids.is_none()||(self.start_pos.is_none()&&self.end_pos.is_none()),"Chỉ dùng dải chương hoặc danh sách chương.");
  Ok(())
 }
 pub fn from_value(v:Value)->Result<Self>{let mut c:Self=serde_json::from_value(v)?;c.validate()?;Ok(c)}
}
pub fn save_secret(path:&Path,secret:&str)->Result<()>{
 anyhow::ensure!(!secret.trim().is_empty()&&secret.len()<=4096&&!secret.chars().any(char::is_control),"Token không hợp lệ.");
 if let Some(parent)=path.parent(){fs::create_dir_all(parent)?;}
 let tmp=path.with_extension("tmp");fs::write(&tmp,secret.trim())?;
 #[cfg(unix)] {use std::os::unix::fs::PermissionsExt;fs::set_permissions(&tmp,fs::Permissions::from_mode(0o600))?;}
 fs::rename(tmp,path)?;Ok(())
}
pub fn load_secret(path:&Path)->Option<String>{fs::read_to_string(path).ok().map(|s|s.trim().to_owned()).filter(|s|!s.is_empty())}
pub fn root_dir()->PathBuf{std::env::var_os("NOVEL_ROOT").map(PathBuf::from).unwrap_or_else(||std::env::current_dir().unwrap_or_else(|_|PathBuf::from(".")))}
pub fn voice_dir()->PathBuf{std::env::var_os("TTS_VOICE_DIR").map(PathBuf::from).unwrap_or_else(||root_dir().join("models/voices"))}
pub fn data_dir()->PathBuf{std::env::var_os("NOVEL_DATA").map(PathBuf::from).unwrap_or_else(||PathBuf::from("data"))}
pub fn output_text_dir()->PathBuf{std::env::var_os("NOVEL_OUTPUT_TEXT").map(PathBuf::from).unwrap_or_else(||PathBuf::from("output_text"))}
pub fn output_audio_dir()->PathBuf{std::env::var_os("NOVEL_OUTPUT_AUDIO").map(PathBuf::from).unwrap_or_else(||PathBuf::from("output_audio"))}
