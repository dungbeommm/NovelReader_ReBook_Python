use anyhow::{bail, Context, Result};
use rusqlite::{backup::Backup, Connection, OpenFlags};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, fs::File, io::{Read, Write}, path::{Path,PathBuf}, time::Duration};
use walkdir::WalkDir;
use zip::{write::FileOptions, CompressionMethod, ZipArchive, ZipWriter};

pub const MAX_ARCHIVE:u64=2*1024*1024*1024;
pub const MAX_EXPANDED:u64=8*1024*1024*1024;
const TEXT_EXT:&[&str]=&["txt","json","epub"];
const AUDIO_EXT:&[&str]=&["mp3","m4a","m4b","wav","opus","ogg","aac","json"];

fn digest(path:&Path)->Result<String>{
    let mut f=File::open(path)?;let mut hasher=Sha256::new();let mut buf=[0u8;1024*1024];
    loop{let n=f.read(&mut buf)?;if n==0{break;}hasher.update(&buf[..n]);}
    Ok(hex::encode(hasher.finalize()))
}
fn safe_rel(name:&str)->bool {
    !name.starts_with('/') && !name.contains('\\') &&
    name.split('/').all(|p|!p.is_empty() && p!="." && p!="..")
}
fn allowed(name:&str)->bool {
    if name=="manifest.json"||name=="data/novel.sqlite3"||name=="workspace_draft.json"{return true;}
    let parts:Vec<_>=name.split('/').collect();
    if parts.len()<2{return false;}
    let ext=Path::new(name).extension().and_then(|x|x.to_str()).unwrap_or("").to_ascii_lowercase();
    (parts[0]=="text"&&TEXT_EXT.contains(&ext.as_str()))||(parts[0]=="audio"&&AUDIO_EXT.contains(&ext.as_str()))
}

pub fn save_draft(path:&Path, config:&Value)->Result<()> {
    let parent=path.parent().context("Draft path has no parent")?;std::fs::create_dir_all(parent)?;
    let tmp=parent.join(format!(".draft-{}.tmp",uuid::Uuid::new_v4()));
    let result=(||{let mut f=File::create(&tmp)?;serde_json::to_writer_pretty(&mut f,config)?;f.sync_all()?;std::fs::rename(&tmp,path)?;Ok::<_,anyhow::Error>(())})();
    let _=std::fs::remove_file(tmp);result?;Ok(())
}
pub fn load_draft(path:&Path)->Result<Option<Value>>{
    if !path.is_file(){return Ok(None);}
    Ok(Some(serde_json::from_slice(&std::fs::read(path)?)?))
}

pub fn create_checkpoint(conn:&Connection,text_root:&Path,audio_root:&Path,draft:Option<&Path>,destination:&Path,include_audio:bool)->Result<Value>{
    let exists:i64=conn.query_row("SELECT COUNT(*) FROM book",[],|r|r.get(0)).context("SQLite schema book missing")?;
    anyhow::ensure!(exists>0,"Hãy nạp truyện trước khi tạo checkpoint.");
    let temp=tempfile::tempdir()?;let db_path=temp.path().join("novel.sqlite3");
    {
        let mut target=Connection::open(&db_path)?;
        let backup=Backup::new(conn,&mut target)?;
        backup.run_to_completion(128,Duration::from_millis(50),None)?;
    }
    let mut entries=BTreeMap::<String,String>::new();
    let mut zip=ZipWriter::new(File::create(destination)?);
    let opts=FileOptions::default().compression_method(CompressionMethod::Deflated);
    fn add(zip:&mut ZipWriter<File>,opts:FileOptions,name:&str,path:&Path,entries:&mut BTreeMap<String,String>)->Result<()>{
        zip.start_file(name,opts)?;let mut f=File::open(path)?;let mut hasher=Sha256::new();let mut buf=[0u8;1024*1024];
        loop{let n=f.read(&mut buf)?;if n==0{break;}zip.write_all(&buf[..n])?;hasher.update(&buf[..n]);}
        entries.insert(name.into(),hex::encode(hasher.finalize()));Ok(())
    }
    add(&mut zip,opts,"data/novel.sqlite3",&db_path,&mut entries)?;
    for (kind,root,extensions) in std::iter::once(("text",text_root,TEXT_EXT)).chain(if include_audio{Some(("audio",audio_root,AUDIO_EXT))}else{None}) {
        if !root.exists(){continue;}
        for entry in WalkDir::new(root).follow_links(false).into_iter().filter_map(Result::ok) {
            if !entry.file_type().is_file(){continue;}
            let rel=entry.path().strip_prefix(root)?.to_string_lossy().replace('\\',"/");
            if !safe_rel(&rel)||rel.split('/').any(|p|p.starts_with('.')){continue;}
            let ext=entry.path().extension().and_then(|x|x.to_str()).unwrap_or("").to_ascii_lowercase();
            if !extensions.contains(&ext.as_str()){continue;}
            add(&mut zip,opts,&format!("{kind}/{rel}"),entry.path(),&mut entries)?;
        }
    }
    if let Some(path)=draft.filter(|p|p.is_file()){add(&mut zip,opts,"workspace_draft.json",path,&mut entries)?;}
    let manifest=json!({"format":"novelreader-checkpoint","version":1,"include_audio":include_audio,"sha256":entries});
    zip.start_file("manifest.json",opts)?;zip.write_all(serde_json::to_string(&manifest)?.as_bytes())?;zip.finish()?;
    let size=destination.metadata()?.len();if size>MAX_ARCHIVE{std::fs::remove_file(destination)?;bail!("Checkpoint vượt 2 GB; bỏ chọn kèm audio.");}
    Ok(json!({"ok":true,"filename":destination.file_name().and_then(|x|x.to_str()).unwrap_or("checkpoint.zip"),"size_bytes":size,"include_audio":include_audio,"files":entries.len()+1}))
}

pub fn validate_checkpoint(path:&Path)->Result<Value>{
    anyhow::ensure!(path.metadata()?.len()<=MAX_ARCHIVE,"Checkpoint vượt giới hạn 2 GB.");
    let mut archive=ZipArchive::new(File::open(path)?)?;
    anyhow::ensure!(archive.len()<=100_000,"ZIP có quá nhiều mục.");
    let mut names=std::collections::HashSet::new();let mut expanded=0u64;
    for i in 0..archive.len(){let f=archive.by_index(i)?;let name=f.name().to_owned();
        anyhow::ensure!(names.insert(name.clone())&&safe_rel(&name)&&allowed(&name)&&!f.is_dir(),"Đường dẫn hoặc nội dung ZIP không hợp lệ.");
        expanded=expanded.saturating_add(f.size());anyhow::ensure!(expanded<=MAX_EXPANDED,"ZIP giải nén quá 8 GB.");
    }
    anyhow::ensure!(names.contains("manifest.json")&&names.contains("data/novel.sqlite3"),"Không phải checkpoint NovelReader v1.");
    let manifest:Value=serde_json::from_slice(&{let mut f=archive.by_name("manifest.json")?;let mut b=Vec::new();f.take(16*1024*1024+1).read_to_end(&mut b)?;b})?;
    anyhow::ensure!(manifest["format"]=="novelreader-checkpoint"&&manifest["version"]==1,"Phiên bản checkpoint không được hỗ trợ.");
    let checks=manifest["sha256"].as_object().context("Manifest thiếu SHA256")?;
    anyhow::ensure!(checks.len()+1==names.len(),"Manifest không khớp nội dung ZIP.");
    let stage=tempfile::tempdir()?;
    for i in 0..archive.len(){
        let mut f=archive.by_index(i)?;let name=f.name().to_owned();if name=="manifest.json"{continue;}
        let dest=stage.path().join(&name);if let Some(p)=dest.parent(){std::fs::create_dir_all(p)?;}
        let mut out=File::create(&dest)?;std::io::copy(&mut f,&mut out)?;
        anyhow::ensure!(checks[&name].as_str()==Some(&digest(&dest)?),"Checkpoint bị hỏng: SHA256 không khớp.");
    }
    let db_path=stage.path().join("data/novel.sqlite3");
    let db=Connection::open_with_flags(db_path,OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let check:String=db.query_row("PRAGMA quick_check",[],|r|r.get(0))?;
    anyhow::ensure!(check=="ok","SQLite checkpoint bị hỏng.");
    let tables:Vec<String>={let mut s=db.prepare("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name")?;s.query_map([],|r|r.get(0))?.collect::<rusqlite::Result<_>>()?};
    anyhow::ensure!(tables==vec!["book".to_string(),"chapter".to_string()],"Schema checkpoint không hợp lệ.");
    let books:i64=db.query_row("SELECT COUNT(*) FROM book",[],|r|r.get(0))?;
    anyhow::ensure!(books==1,"Checkpoint phải chứa đúng một truyện.");
    Ok(json!({"ok":true,"version":1,"expanded_bytes":expanded,"entries":names.len(),"include_audio":manifest["include_audio"]}))
}
