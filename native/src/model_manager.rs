use anyhow::{Context, Result};
use serde::Deserialize;
use std::{path::{Path,PathBuf}, sync::Arc};
use tokio::{fs, io::AsyncWriteExt, sync::Semaphore};

#[derive(Debug,Clone,Deserialize)]
pub struct ModelFile { pub path:String, #[serde(default)] pub size:u64, #[serde(rename="type",default)] pub kind:String }
#[derive(Clone)]
pub struct ModelManager { client:reqwest::Client, root:PathBuf, token:Option<String>, limit:Arc<Semaphore> }
impl ModelManager {
    pub fn new(root:PathBuf,token:Option<String>)->Result<Self>{
        Ok(Self{client:reqwest::Client::builder().user_agent("NovelReader-Native/1.0").build()?,root,token,limit:Arc::new(Semaphore::new(3))})
    }
    pub fn root(&self)->&Path{&self.root}
    fn auth(&self,req:reqwest::RequestBuilder)->reqwest::RequestBuilder{
        if let Some(token)=&self.token{req.bearer_auth(token)}else{req}
    }
    pub async fn list_repo_files(&self,repo:&str,revision:&str)->Result<Vec<ModelFile>>{
        validate_repo(repo)?;
        let url=format!("https://huggingface.co/api/models/{repo}/tree/{revision}?recursive=true&expand=false");
        let response=self.auth(self.client.get(url)).send().await?;
        anyhow::ensure!(response.status().is_success(),"Hugging Face API trả HTTP {} khi đọc model.",response.status());
        Ok(response.json().await?)
    }
    pub async fn download_ct2(&self,repo:&str,subdir:&str)->Result<PathBuf>{
        self.download_ct2_from(repo,repo,subdir).await
    }
    pub async fn download_ct2_from(&self,model_repo:&str,tokenizer_repo:&str,subdir:&str)->Result<PathBuf>{
        validate_repo(model_repo)?;validate_repo(tokenizer_repo)?;
        anyhow::ensure!(subdir=="ct2-int8_float32"||subdir=="ct2-int8"||subdir=="ct2"||subdir=="ct2-int8-keeppad","Thư mục CT2 không hợp lệ.");
        let model_files=self.list_repo_files(model_repo,"main").await?;
        let mut selected:Vec<(String,u64)>=model_files.into_iter().filter(|f|f.path.starts_with(&format!("{subdir}/"))).map(|f|(f.path,f.size)).collect();
        anyhow::ensure!(selected.iter().any(|(p,_)|p==&format!("{subdir}/model.bin")),"Model CT2 thiếu model.bin.");
        anyhow::ensure!(selected.iter().any(|(p,_)|p==&format!("{subdir}/config.json")),"Model CT2 thiếu config.json.");
        if tokenizer_repo==model_repo {
            let files=self.list_repo_files(model_repo,"main").await?;
            for f in files {
                if f.path=="source.spm"||f.path=="target.spm"||f.path=="tokenizer.json" {
                    if !selected.iter().any(|(p,_)|p==&f.path){selected.push((f.path,f.size));}
                }
            }
        } else {
            let tokenizer_files=self.list_repo_files(tokenizer_repo,"main").await?;
            for f in tokenizer_files {
                if f.path=="source.spm"||f.path=="target.spm"||f.path=="tokenizer.json" {
                    selected.push((f.path,f.size));
                }
            }
        }
        let dest=self.root.join(model_repo.replace('/',"--"));
        fs::create_dir_all(&dest).await?;
        for (relative,size) in selected {
            anyhow::ensure!(safe_path(&relative),"Model chứa đường dẫn không an toàn.");
            let target=dest.join(&relative);
            if let Some(parent)=target.parent(){fs::create_dir_all(parent).await?;}
            if target.is_file() && size>0 && fs::metadata(&target).await?.len()==size{continue;}
            let _permit=self.limit.acquire().await?;
            let repo=if relative.starts_with(&format!("{subdir}/")){model_repo}else{tokenizer_repo};
            let mut url=url::Url::parse(&format!("https://huggingface.co/{repo}/resolve/main/"))?;
            {let mut segments=url.path_segments_mut().map_err(|_|anyhow::anyhow!("URL model không hợp lệ"))?;for part in relative.split('/') {segments.push(part);}}
            let mut response=self.auth(self.client.get(url)).send().await?;
            anyhow::ensure!(response.status().is_success(),"Tải {} lỗi HTTP {}",relative,response.status());
            let tmp=target.with_extension("download.part");
            let mut out=fs::File::create(&tmp).await?;let mut count=0u64;
            while let Some(chunk)=response.chunk().await?{count+=chunk.len() as u64;out.write_all(&chunk).await?;}
            out.flush().await?;drop(out);
            if size>0{anyhow::ensure!(count==size,"Kích thước tải model không khớp: {}",relative);}
            fs::rename(&tmp,&target).await?;
        }
        anyhow::ensure!((dest.join("source.spm").is_file()&&dest.join("target.spm").is_file())||dest.join("tokenizer.json").is_file(),"Model thiếu tokenizer.");
        Ok(dest)
    }}
fn validate_repo(repo:&str)->Result<()>{
    let parts:Vec<_>=repo.split('/').collect();
    anyhow::ensure!(parts.len()==2&&parts.iter().all(|p|!p.is_empty()&&p.bytes().all(|b|b.is_ascii_alphanumeric()||b"._-".contains(&b))),"Hugging Face repo ID không hợp lệ.");Ok(())
}
fn safe_path(path:&str)->bool{!path.starts_with('/')&&!path.contains('\\')&&path.split('/').all(|p|!p.is_empty()&&p!="."&&p!="..")}
