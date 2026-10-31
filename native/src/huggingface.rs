use anyhow::{bail,Context,Result};
use serde_json::{json,Value};
use std::{fs,path::{Path,PathBuf},io::Write,time::Duration};
use url::Url;

const ENDPOINT:&str="https://huggingface.co";
const DEFAULT_REPO:&str="uri410/novelreader-storage";

fn private_dir()->PathBuf{crate::config::data_dir().join("private")}
pub fn load_key()->Option<String>{
 if let Ok(v)=std::env::var("HF_TOKEN"){if !v.trim().is_empty(){return Some(v.trim().into());}}
 let value=fs::read(private_dir().join("huggingface.json")).ok().and_then(|b|serde_json::from_slice::<Value>(&b).ok())?;
 value["api_key"].as_str().map(str::trim).filter(|s|!s.is_empty()).map(str::to_owned)
}
pub fn save_key(token:&str)->Result<()>{
 if token.trim().is_empty(){let p=private_dir().join("huggingface.json");if p.exists(){fs::remove_file(p)?;}return Ok(());}
 anyhow::ensure!(token.len()<=4096&&!token.chars().any(char::is_control),"Token không hợp lệ.");
 atomic_json(&private_dir().join("huggingface.json"),&json!({"api_key":token.trim()}))
}
fn validate_repo_id(value:&str)->Result<()>{
 let parts:Vec<_>=value.split('/').collect();
 anyhow::ensure!(parts.len()==2&&value.len()<=256&&parts.iter().all(|p|!p.is_empty()&&p!="."&&p!=".."&&p.bytes().all(|b|b.is_ascii_alphanumeric()||b"._-".contains(&b))),"Bucket HF phải có dạng tài_khoản/tên_bucket.");
 Ok(())
}
pub fn repo_id()->String{
 let value=fs::read(private_dir().join("huggingface_repo.json")).ok().and_then(|b|serde_json::from_slice::<Value>(&b).ok())
  .and_then(|v|v["repo_id"].as_str().map(str::to_owned))
  .or_else(||std::env::var("HF_BUCKET_ID").ok()).unwrap_or_else(||DEFAULT_REPO.into());
 if validate_repo_id(&value).is_ok(){value}else{DEFAULT_REPO.into()}
}
pub fn save_repo_id(value:&str)->Result<String>{
 validate_repo_id(value)?;atomic_json(&private_dir().join("huggingface_repo.json"),&json!({"repo_id":value}))?;Ok(value.into())
}
fn atomic_json(path:&Path,value:&Value)->Result<()>{
 let parent=path.parent().context("Private settings path invalid")?;fs::create_dir_all(parent)?;
 #[cfg(unix)]{use std::os::unix::fs::PermissionsExt;fs::set_permissions(parent,fs::Permissions::from_mode(0o700))?;}
 let tmp=parent.join(format!(".{}.{}.tmp",path.file_name().and_then(|x|x.to_str()).unwrap_or("settings"),uuid::Uuid::new_v4()));
 let result=(||{let mut f=fs::File::create(&tmp)?;serde_json::to_writer_pretty(&mut f,value)?;f.sync_all()?;
  #[cfg(unix)]{use std::os::unix::fs::PermissionsExt;fs::set_permissions(&tmp,fs::Permissions::from_mode(0o600))?;}
  fs::rename(&tmp,path)?;Ok::<_,anyhow::Error>(())})();
 let _=fs::remove_file(tmp);result
}
fn client()->Result<reqwest::Client>{Ok(reqwest::Client::builder().user_agent("NovelReader-Native/1.0").timeout(Duration::from_secs(180)).build()?)}
fn auth(request:reqwest::RequestBuilder,token:&str)->reqwest::RequestBuilder{request.bearer_auth(token)}
fn api_url(repo:&str,segments:&[&str])->Result<Url>{
 validate_repo_id(repo)?;let mut url=Url::parse(ENDPOINT)?;
 {let mut p=url.path_segments_mut().map_err(|_|anyhow::anyhow!("URL endpoint invalid"))?;p.push("api");p.push("buckets");
  let mut parts=repo.split('/');p.push(parts.next().unwrap());p.push(parts.next().unwrap());for segment in segments{p.push(segment);}}
 Ok(url)
}
pub fn repository_path(value:&str,allow_root:bool)->Result<()>{
 anyhow::ensure!(value.len()<=1024&&!value.contains('\\')&&!value.contains(':')&&!value.chars().any(char::is_control)&&value==value.trim()&&(allow_root||!value.is_empty()),"Đường dẫn kho không hợp lệ.");
 if !value.is_empty(){anyhow::ensure!(!value.starts_with('/')&&value.split('/').all(|p|!p.is_empty()&&p!="."&&p!=".."),"Đường dẫn kho không hợp lệ.");}
 Ok(())
}
pub async fn check_connection()->Result<Value>{
 let token=load_key().context("Chưa cấu hình HF_TOKEN.")?;let repo=repo_id();let c=client()?;
 let bucket=auth(c.get(format!("{ENDPOINT}/api/buckets/{repo}")),&token).send().await?;
 anyhow::ensure!(bucket.status().is_success(),"Không đọc được Bucket: HTTP {}",bucket.status());
 let info:Value=bucket.json().await?;
 let user=auth(c.get(format!("{ENDPOINT}/api/whoami-v2")),&token).send().await?;
 anyhow::ensure!(user.status().is_success(),"Token không xác thực được: HTTP {}",user.status());
 let user:Value=user.json().await?;
 Ok(json!({"ok":true,"repo_id":repo,"repo_type":"bucket","private":info["private"],
  "username":user["name"],"message":format!("Đã kết nối {repo}.")}))
}
pub async fn list_directory(path:&str)->Result<Value>{
 repository_path(path,true)?;let token=load_key().context("Chưa cấu hình HF_TOKEN.")?;let repo=repo_id();let c=client()?;
 let mut url=api_url(&repo,&["tree"])?;
 if !path.is_empty(){url.path_segments_mut().map_err(|_|anyhow::anyhow!("URL invalid"))?.push(path);}
 url.query_pairs_mut().append_pair("recursive","false");
 let mut entries=Vec::<Value>::new();let mut next=Some(url);let next_re=regex::Regex::new(r#"<([^>]+)>;\s*rel="next""#)?;
 while let Some(current)=next.take(){
  let response=auth(c.get(current),&token).send().await?;
  anyhow::ensure!(response.status().is_success(),"Bucket tree trả HTTP {}",response.status());
  let link=response.headers().get(reqwest::header::LINK).and_then(|x|x.to_str().ok()).unwrap_or("").to_string();
  let mut page:Vec<Value>=response.json().await?;entries.append(&mut page);
  if entries.len()>=501{break;}
  next=next_re.captures(&link).and_then(|x|x.get(1)).and_then(|x|Url::parse(x.as_str()).ok());
 }
 let mut rows=Vec::new();
 for item in entries.into_iter().take(501){
  let remote=item["path"].as_str().unwrap_or("");let name=remote.rsplit('/').next().unwrap_or("");
  if name==".gitkeep"||remote.is_empty(){continue;}
  let directory=item["type"]=="directory";
  rows.push(json!({"name":name,"path":remote,"directory":directory,
   "size":if directory{Value::Null}else{item["size"].clone()},
   "updated_at":item["uploaded_at"].clone()}));
 }
 rows.sort_by(|a,b|(!a["directory"].as_bool().unwrap_or(false)).cmp(&(!b["directory"].as_bool().unwrap_or(false)))
  .then_with(||a["name"].as_str().unwrap_or("").to_lowercase().cmp(&b["name"].as_str().unwrap_or("").to_lowercase())));
 let file_bytes:u64=rows.iter().filter_map(|x|x["size"].as_u64()).sum();
 Ok(json!({"path":path,"entries":rows.iter().take(500).collect::<Vec<_>>(),"truncated":rows.len()>500,
  "file_bytes":file_bytes,"repo_id":repo}))
}
pub async fn download_file(path:&str,destination:&Path)->Result<u64>{
 repository_path(path,false)?;let token=load_key().context("Chưa cấu hình HF_TOKEN.")?;let repo=repo_id();let c=client()?;
 let url=api_url(&repo,&["paths-info"])?;
 let response=auth(c.post(url).json(&json!({"paths":[path]})),&token).send().await?;
 anyhow::ensure!(response.status().is_success(),"Bucket paths-info trả HTTP {}",response.status());
 let info:Vec<Value>=response.json().await?;
 anyhow::ensure!(!info.is_empty(),"Không tìm thấy tệp trong Bucket.");
 let expected=info[0]["size"].as_u64().unwrap_or(0);
 let mut url=Url::parse(&format!("{ENDPOINT}/buckets/"))?;
 {let mut p=url.path_segments_mut().map_err(|_|anyhow::anyhow!("URL invalid"))?;let mut parts=repo.split('/');p.push(parts.next().unwrap());p.push(parts.next().unwrap());p.push("resolve");p.push(path);}
 let mut response=auth(c.get(url),&token).send().await?;
 anyhow::ensure!(response.status().is_success(),"Không tải được tệp: HTTP {}",response.status());
 if let Some(parent)=destination.parent(){tokio::fs::create_dir_all(parent).await?;}
 let tmp=destination.with_extension("download.part");let mut out=tokio::fs::File::create(&tmp).await?;let mut total=0u64;
 while let Some(chunk)=response.chunk().await?{total+=chunk.len() as u64;anyhow::ensure!(total<=2*1024*1024*1024,"Tệp Bucket vượt 2 GB.");out.write_all(&chunk).await?;}
 out.flush().await?;drop(out);
 if expected>0&&expected!=total{let _=tokio::fs::remove_file(&tmp).await;bail!("Kích thước tải xuống không khớp.");}
 tokio::fs::rename(&tmp,destination).await?;Ok(total)
}
pub fn upload_unavailable()->anyhow::Error{
 anyhow::anyhow!("Native Bucket upload chưa được port: Hugging Face Bucket yêu cầu Xet CAS upload + batch commit; không thể thay bằng PUT file thường mà vẫn bảo toàn giao thức.")
}
