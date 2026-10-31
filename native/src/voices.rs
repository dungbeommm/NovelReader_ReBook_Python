use anyhow::{Context,Result,bail};
use serde_json::{json,Value};
use std::{path::{Path,PathBuf},fs};
use tokio::{fs as async_fs,io::AsyncWriteExt};

fn catalog_path()->PathBuf{crate::config::root_dir().join("tts/voices_catalog.json")}
pub fn list()->Result<Vec<Value>>{
 let catalog:Vec<Value>=serde_json::from_slice(&fs::read(catalog_path()).context("Thiếu tts/voices_catalog.json")?)?;
 let root=crate::config::voice_dir();let mut rows=Vec::new();
 for item in catalog{
  let id=item["id"].as_str().unwrap_or("");if id.is_empty(){continue;}
  let model=root.join(format!("{id}.onnx"));let cfg=root.join(format!("{id}.onnx.json"));
  let valid=model.is_file()&&model.metadata().map(|m|m.len()>=10*1024*1024).unwrap_or(false)&&valid_config(&cfg);
  rows.push(json!({"id":id,"name":item["name"],"gender":item["gender"],"is_default":id=="ngochuyennew",
   "size_mb":if valid{model.metadata().map(|m|m.len() as f64/1048576.0).unwrap_or(0.0)}else{0.0},
   "onnx_file":format!("{id}.onnx"),"json_file":format!("{id}.onnx.json"),"downloaded":valid}));
 }
 rows.sort_by(|a,b|(!a["is_default"].as_bool().unwrap_or(false)).cmp(&(!b["is_default"].as_bool().unwrap_or(false)))
  .then_with(||a["gender"].as_str().cmp(&b["gender"].as_str())).then_with(||a["name"].as_str().cmp(&b["name"].as_str())));
 Ok(rows)
}
fn valid_config(path:&Path)->bool{
 let Ok(data)=fs::read(path)else{return false;};
 serde_json::from_slice::<Value>(&data).map(|v|v["audio"].is_object()&&v["phoneme_id_map"].is_object()).unwrap_or(false)
}
async fn download_one(client:&reqwest::Client,urls:&[String],path:&Path,min_size:u64,is_config:bool)->Result<()>{
 if path.is_file()&&async_fs::metadata(path).await?.len()>=min_size&&(!is_config||valid_config(path)){return Ok(());}
 if let Some(parent)=path.parent(){async_fs::create_dir_all(parent).await?;}
 let tmp=path.with_extension("download.part");let mut last_error=String::new();
 for url in urls{
  let attempt=async{
   let mut response=client.get(url).send().await?;
   anyhow::ensure!(response.status().is_success(),"HTTP {}",response.status());
   let mut out=async_fs::File::create(&tmp).await?;let mut total=0u64;
   while let Some(chunk)=response.chunk().await?{
    total+=chunk.len() as u64;anyhow::ensure!(total<=1024*1024*1024,"Voice model vượt 1 GB.");
    out.write_all(&chunk).await?;
   }
   out.flush().await?;drop(out);
   anyhow::ensure!(total>=min_size,"File tải quá nhỏ.");
   if is_config{anyhow::ensure!(valid_config(&tmp),"Piper JSON config không hợp lệ.");}
   async_fs::rename(&tmp,path).await?;Ok::<_,anyhow::Error>(())
  }.await;
  match attempt{Ok(())=>return Ok(()),Err(e)=>{last_error=e.to_string();let _=async_fs::remove_file(&tmp).await;}}
 }
 bail!("Không tải được {}: {}",path.file_name().and_then(|x|x.to_str()).unwrap_or("voice"),last_error)
}
pub async fn ensure_voice(client:&reqwest::Client,id:&str)->Result<(PathBuf,PathBuf)>{
 anyhow::ensure!(!id.is_empty()&&id.len()<=100&&id.bytes().all(|b|b.is_ascii_alphanumeric()||b==b'_'||b==b'-'),"ID giọng không hợp lệ.");
 let catalog:Vec<Value>=serde_json::from_slice(&async_fs::read(catalog_path()).await?)?;
 let item=catalog.iter().find(|v|v["id"].as_str()==Some(id)).context("Giọng không có trong catalog.")?;
 let model=crate::config::voice_dir().join(format!("{id}.onnx"));
 let config=crate::config::voice_dir().join(format!("{id}.onnx.json"));
 let base=format!("https://raw.githubusercontent.com/dungbeommm/NovelReader_voices/main/voices/low/{}/{}",item["folder"].as_str().unwrap_or(""),id);
 let onnx_urls=vec![item["onnxUrl"].as_str().unwrap_or("").to_string(),format!("https://cdn.ainovelreader.com/tts_model/{id}.onnx"),format!("{base}.onnx")].into_iter().filter(|x|!x.is_empty()).collect::<Vec<_>>();
 let json_urls=vec![item["jsonUrl"].as_str().unwrap_or("").to_string(),format!("https://cdn.ainovelreader.com/tts_model/{id}.onnx.json"),format!("{base}.onnx.json")].into_iter().filter(|x|!x.is_empty()).collect::<Vec<_>>();
 download_one(client,&json_urls,&config,50,true).await?;
 download_one(client,&onnx_urls,&model,10*1024*1024,false).await?;
 Ok((model,config))
}
