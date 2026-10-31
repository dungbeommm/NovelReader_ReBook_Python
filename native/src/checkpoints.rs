use anyhow::{bail, Context, Result};
use rusqlite::{backup::Backup, Connection, OpenFlags};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, fs::File, io::{Read, Write}, path::{Path,PathBuf}, time::Duration};
use walkdir::WalkDir;
use crate::db;
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
    let tables:Vec<String>={
        let mut statement=db.prepare("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name")?;
        let rows=statement.query_map([],|r|r.get(0))?;
        let collected=rows.collect::<rusqlite::Result<_>>()?;
        collected
    };
    anyhow::ensure!(tables==vec!["book".to_string(),"chapter".to_string()],"Schema checkpoint không hợp lệ.");
    let books:i64=db.query_row("SELECT COUNT(*) FROM book",[],|r|r.get(0))?;
    anyhow::ensure!(books==1,"Checkpoint phải chứa đúng một truyện.");
    Ok(json!({"ok":true,"version":1,"expanded_bytes":expanded,"entries":names.len(),"include_audio":manifest["include_audio"]}))
}


fn copy_allowed_tree(source:&Path,destination:&Path,allowed:&[&str])->Result<()>{
    std::fs::create_dir_all(destination)?;
    if !source.is_dir(){return Ok(());}
    for entry in WalkDir::new(source).follow_links(false).into_iter().filter_map(|e|e.ok()){
        if !entry.file_type().is_file(){continue;}
        let rel=entry.path().strip_prefix(source)?;
        if rel.components().any(|c|c.as_os_str().to_string_lossy().starts_with('.')){continue;}
        let ext=entry.path().extension().and_then(|x|x.to_str()).unwrap_or("").to_ascii_lowercase();
        if !allowed.contains(&ext.as_str()){continue;}
        let target=destination.join(rel);
        if let Some(parent)=target.parent(){std::fs::create_dir_all(parent)?;}
        std::fs::copy(entry.path(),target)?;
    }
    Ok(())
}
fn available_folder(root:&Path,base:&str)->Result<String>{
    std::fs::create_dir_all(root)?;
    let clean=base.chars().map(|c|if r#"\/:*?"<>|"#.contains(c)||c.is_control(){'_' }else{c}).collect::<String>()
        .trim().trim_matches('.').chars().take(80).collect::<String>();
    let base=if clean.is_empty(){"truyen".to_string()}else{clean};
    let mut folder=base.clone();let mut suffix=2usize;
    while root.join(&folder).exists(){let tail=format!("_{suffix}");folder=format!("{}{}",base.chars().take(80usize.saturating_sub(tail.len())).collect::<String>(),tail);suffix+=1;}
    Ok(folder)
}
pub fn restore_checkpoint(conn:&mut Connection,archive_path:&Path,text_root:&Path,audio_root:&Path,draft_path:&Path)->Result<Value>{
    validate_checkpoint(archive_path)?;
    let stage=tempfile::tempdir()?;
    {
        let mut archive=ZipArchive::new(File::open(archive_path)?)?;
        for i in 0..archive.len(){
            let mut entry=archive.by_index(i)?;let name=entry.name().to_owned();
            if name=="manifest.json"{continue;}
            anyhow::ensure!(safe_rel(&name)&&allowed(&name),"Checkpoint có đường dẫn không hợp lệ.");
            let dest=stage.path().join(&name);
            if let Some(parent)=dest.parent(){std::fs::create_dir_all(parent)?;}
            let mut out=File::create(&dest)?;std::io::copy(&mut entry,&mut out)?;
        }
    }
    let source_db=Connection::open_with_flags(stage.path().join("data/novel.sqlite3"),OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let book=db::book(&source_db)?.context("Checkpoint thiếu truyện.")?;
    let raw_config=book["config"].as_str().and_then(|s|serde_json::from_str::<Value>(s).ok()).unwrap_or(json!({}));
    let clean_config=crate::config::AppConfig::from_value(raw_config)
        .and_then(|v|serde_json::to_value(v).map_err(Into::into))?;
    let rows=db::all_chapters(&source_db)?;
    for row in &rows{
        anyhow::ensure!(["done","pending","translating","error","stopped"].contains(&row["status"].as_str().unwrap_or("")),"Trạng thái chương checkpoint không hợp lệ.");
        anyhow::ensure!(["none","queued","generating","done","error"].contains(&row["audio_status"].as_str().unwrap_or("")),"Trạng thái audio checkpoint không hợp lệ.");
        for field in ["source","plan","outputs","translated"]{anyhow::ensure!(row[field].is_array(),"Dữ liệu chương {field} không hợp lệ.");}
    }
    let draft_value=if stage.path().join("workspace_draft.json").is_file(){
        let value:Value=serde_json::from_slice(&std::fs::read(stage.path().join("workspace_draft.json"))?)?;
        Some(serde_json::to_value(crate::config::AppConfig::from_value(value)?)?)
    }else{None};
    let title=book["title"].as_str().unwrap_or("truyen");
    let folder=available_folder(text_root,title)?;
    std::fs::create_dir_all(audio_root)?;
    let new_text=text_root.join(&folder);let new_audio=audio_root.join(&folder);
    anyhow::ensure!(!new_text.exists()&&!new_audio.exists(),"Tên thư mục khôi phục đã tồn tại; thử lại.");
    let mut created_text=false;let mut created_audio=false;
    let result=(||->Result<()>{
        created_text=true;copy_allowed_tree(&stage.path().join("text"),&new_text,TEXT_EXT)?;
        created_audio=true;copy_allowed_tree(&stage.path().join("audio"),&new_audio,AUDIO_EXT)?;
        let tx=conn.transaction()?;
        tx.execute("DELETE FROM chapter",[])?;tx.execute("DELETE FROM book",[])?;
        tx.execute("INSERT INTO book(id,title,filename,encoding,config,output_folder) VALUES(1,?,?,?,?,?)",
            rusqlite::params![title,book["filename"].as_str().unwrap_or(""),book["encoding"].as_str().unwrap_or(""),clean_config.to_string(),folder])?;
        for row in &rows{
            let id=row["id"].as_i64().context("Chapter id invalid")?;
            let position=row["position"].as_i64().context("Chapter position invalid")?;
            let mut status=row["status"].as_str().unwrap_or("pending").to_string();
            if status=="translating"{status="pending".into();}
            let mut audio_status=row["audio_status"].as_str().unwrap_or("none").to_string();
            if audio_status=="queued"||audio_status=="generating"||crate::audio::chapter_audio_path(&new_audio,id,position).is_none(){audio_status="none".into();}
            tx.execute("INSERT INTO chapter(id,position,title,source,status,plan,outputs,translated_title,translated,error,audio_status) VALUES(?,?,?,?,?,?,?,?,?,?,?)",
                rusqlite::params![id,position,row["title"].as_str().unwrap_or(""),row["source"].to_string(),status,row["plan"].to_string(),
                    row["outputs"].to_string(),row["translated_title"].as_str().unwrap_or(""),row["translated"].to_string(),
                    row["error"].as_str().unwrap_or(""),audio_status])?;
        }
        tx.commit()?;
        if let Some(draft)=draft_value.as_ref(){save_draft(draft_path,draft)?;}else{let _=std::fs::remove_file(draft_path);}
        Ok(())
    })();
    if let Err(err)=result{
        if created_text{let _=std::fs::remove_dir_all(&new_text);}
        if created_audio{let _=std::fs::remove_dir_all(&new_audio);}
        return Err(err);
    }
    Ok(json!({"ok":true,"chapters":rows.len(),"book_title":title,
        "message":"Đã khôi phục checkpoint. Chương đang dịch được chuyển về pending; audio thiếu sẽ cần tạo lại."}))
}
