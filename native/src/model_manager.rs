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
        validate_repo(repo)?;
        anyhow::ensure!(subdir=="ct2-int8_float32"||subdir=="ct2-int8"||subdir=="ct2","Thư mục CT2 không hợp lệ.");
        let files=self.list_repo_files(repo,"main").await?;
        let required=vec![format!("{subdir}/model.bin"),format!("{subdir}/config.json")];
        for r in &required{anyhow::ensure!(files.iter().any(|f|&f.path==r),"Model thiếu tệp bắt buộc: {r}");}
        let dest=self.root.join(repo.replace('/',"--"));
        fs::create_dir_all(&dest).await?;
        let wanted:Vec<_>=files.into_iter().filter(|f|{
            f.path.starts_with(&format!("{subdir}/"))||f.path=="source.spm"||f.path=="target.spm"||f.path=="tokenizer.json"
        }).collect();
        for item in wanted{
            anyhow::ensure!(safe_path(&item.path),"Model chứa đường dẫn không an toàn.");
            let target=dest.join(&item.path);
            if let Some(parent)=target.parent(){fs::create_dir_all(parent).await?;}
            if target.is_file() && item.size>0 && fs::metadata(&target).await?.len()==item.size{continue;}
            let _permit=self.limit.acquire().await?;
            let mut url = url::Url::parse(&format!("https://huggingface.co/{repo}/resolve/main/"))?;
            { let mut segments=url.path_segments_mut().map_err(|_|anyhow::anyhow!("URL model không hợp lệ"))?; for part in item.path.split('/') { segments.push(part); } }
            let mut response=self.auth(self.client.get(url)).send().await?;
            anyhow::ensure!(response.status().is_success(),"Tải {} lỗi HTTP {}",item.path,response.status());
            let tmp=target.with_extension("download.part");
            let mut out=fs::File::create(&tmp).await?;let mut count=0u64;
            while let Some(chunk)=response.chunk().await?{count+=chunk.len() as u64;out.write_all(&chunk).await?;}
            out.flush().await?;drop(out);
            if item.size>0{anyhow::ensure!(count==item.size,"Kích thước tải model không khớp: {}",item.path);}
            fs::rename(&tmp,&target).await?;
        }
        let model=dest.join(subdir);
        anyhow::ensure!(model.join("model.bin").is_file(),"Tải model chưa hoàn tất.");
        anyhow::ensure!((dest.join("source.spm").is_file()&&dest.join("target.spm").is_file())||dest.join("tokenizer.json").is_file(),"Model thiếu tokenizer.");
        Ok(dest)
    }
}
fn validate_repo(repo:&str)->Result<()>{
    let parts:Vec<_>=repo.split('/').collect();
    anyhow::ensure!(parts.len()==2&&parts.iter().all(|p|!p.is_empty()&&p.bytes().all(|b|b.is_ascii_alphanumeric()||b"._-".contains(&b))),"Hugging Face repo ID không hợp lệ.");Ok(())
}
fn safe_path(path:&str)->bool{!path.starts_with('/')&&!path.contains('\\')&&path.split('/').all(|p|!p.is_empty()&&p!="."&&p!="..")}
