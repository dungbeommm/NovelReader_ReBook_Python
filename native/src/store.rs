use anyhow::{Context,Result};
use rusqlite::Connection;
use serde_json::{json,Value};
use std::{fs,path::{Path,PathBuf}};
use crate::{config,db};

fn safe_name(value:&str)->String{
 let mut name=value.chars().map(|c|if r#"\/:*?"<>|"#.contains(c)||c.is_control(){'_' }else{c}).collect::<String>();
 name=name.trim().trim_matches('.').chars().take(80).collect::<String>();
 if name.is_empty(){name="truyen".into();}
 let stem=name.split('.').next().unwrap_or("").to_ascii_uppercase();
 if ["CON","PRN","AUX","NUL","COM1","COM2","COM3","COM4","COM5","COM6","COM7","COM8","COM9","LPT1","LPT2","LPT3","LPT4","LPT5","LPT6","LPT7","LPT8","LPT9"].contains(&stem.as_str()){name=format!("_{name}");}
 name
}
fn checked_dir(root:&Path,folder:&str)->Result<PathBuf>{
 anyhow::ensure!(!folder.is_empty()&&folder==safe_name(folder)&&!folder.contains('/')&&!folder.contains('\\'),"Tên thư mục truyện không hợp lệ.");
 fs::create_dir_all(root)?;
 let root=root.canonicalize()?;
 let target=root.join(folder);
 if !target.exists(){fs::create_dir_all(&target)?;}
 let canonical=target.canonicalize()?;
 anyhow::ensure!(canonical.starts_with(&root),"Thư mục truyện nằm ngoài output root.");
 Ok(canonical)
}
pub fn book_dirs(conn:&Connection)->Result<(PathBuf,PathBuf)>{
 let book=db::book(conn)?.context("Chưa có truyện.")?;
 let title=book["title"].as_str().unwrap_or("truyen");
 let folder=book["output_folder"].as_str().filter(|x|!x.is_empty()).unwrap_or(title);
 Ok((checked_dir(&config::output_text_dir(),folder)?,checked_dir(&config::output_audio_dir(),folder)?))
}
pub fn export_chapter_text(conn:&Connection,id:i64)->Result<PathBuf>{
 let chapter=db::chapter(conn,id)?.context("Không tìm thấy chương.")?;
 anyhow::ensure!(chapter["status"]=="done","Chương chưa dịch xong.");
 let (text_dir,_)=book_dirs(conn)?;
 let pos=chapter["position"].as_i64().unwrap_or(0);
 let title=chapter["translated_title"].as_str().filter(|s|!s.is_empty()).unwrap_or(chapter["title"].as_str().unwrap_or("Chương"));
 let paras=chapter["translated"].as_array().cloned().unwrap_or_default();
 let body=paras.iter().filter_map(Value::as_str).map(str::trim).filter(|s|!s.is_empty()&&!s.starts_with('(')).collect::<Vec<_>>().join("\n\n");
 let content=if body.is_empty(){format!("{title}\n")}else{format!("{title}\n\n{body}\n")};
 let target=text_dir.join(format!("chuong_{pos:04}.txt"));
 let temp=target.with_extension("txt.tmp");fs::write(&temp,content)?;fs::rename(temp,&target)?;Ok(target)
}
pub fn export_full_book_text(conn:&Connection)->Result<Option<PathBuf>>{
 let Some(book)=db::book(conn)? else{return Ok(None)};
 let rows=db::all_chapters(conn)?.into_iter().filter(|c|c["status"]=="done").collect::<Vec<_>>();
 let (text_dir,_)=book_dirs(conn)?;let target=text_dir.join("toan_bo_truyen.txt");
 if rows.is_empty(){let _=fs::remove_file(&target);return Ok(None);}
 let mut content=format!("=== {} ===\n\n",book["title"].as_str().unwrap_or("TRUYỆN").to_uppercase());
 for c in rows{
  let title=c["translated_title"].as_str().filter(|s|!s.is_empty()).unwrap_or(c["title"].as_str().unwrap_or("Chương"));
  content.push_str(&format!("\n\n--- {title} ---\n\n"));
  if let Some(paras)=c["translated"].as_array(){content.push_str(&paras.iter().filter_map(Value::as_str).map(str::trim).filter(|s|!s.is_empty()).collect::<Vec<_>>().join("\n\n"));}
 }
 let tmp=target.with_extension("txt.tmp");fs::write(&tmp,content)?;fs::rename(tmp,&target)?;Ok(Some(target))
}
pub fn invalidate_groups(audio_dir:&Path,positions:Option<&[i64]>)->Result<()>{
 for name in ["audio_gop","groups","merged"]{
  let dir=audio_dir.join(name);if !dir.is_dir(){continue;}
  for entry in fs::read_dir(dir)?{let p=entry?.path();if !p.is_file(){continue;}
   let filename=p.file_name().and_then(|x|x.to_str()).unwrap_or("");
   let Some(rest)=filename.strip_prefix("chuong_") else{continue;};
   let Some((a,b))=rest.split_once('_') else{continue;};
   let start=a.parse::<i64>().ok();let end=b.split('.').next().and_then(|x|x.parse::<i64>().ok());
   let intersects=positions.map(|list|start.zip(end).map(|(s,e)|list.iter().any(|n|*n>=s&&*n<=e)).unwrap_or(false)).unwrap_or(true);
   if intersects{let _=fs::remove_file(p);}
  }
 }
 Ok(())
}
pub fn reset_chapters(conn:&mut Connection,ids:&[i64])->Result<usize>{
 let rows=db::reset_chapters(conn,ids)?;if rows.is_empty(){return Ok(0);}
 let (text_dir,audio_dir)=book_dirs(conn)?;let positions:Vec<i64>=rows.iter().map(|x|x.1).collect();invalidate_groups(&audio_dir,Some(&positions))?;
 for (id,pos) in rows{
  let _=fs::remove_file(text_dir.join(format!("chuong_{pos:04}.txt")));
  for ext in ["mp3","m4b","m4a","opus","wav"]{let _=fs::remove_file(audio_dir.join(format!("chapter_{id}.{ext}")));let _=fs::remove_file(audio_dir.join(format!("chuong_{pos:04}.{ext}")));}
 }
 export_full_book_text(conn)?;Ok(positions.len())
}
pub fn delete_chapters(conn:&mut Connection,ids:&[i64])->Result<usize>{
 let (text_dir,audio_dir)=book_dirs(conn)?;
 let before=db::all_chapters(conn)?;
 let selected:std::collections::HashSet<i64>=ids.iter().copied().collect();
 let removed:Vec<_>=before.iter().filter(|c|selected.contains(&c["id"].as_i64().unwrap_or(-1))).map(|c|(c["id"].as_i64().unwrap_or(0),c["position"].as_i64().unwrap_or(0))).collect();
 if removed.is_empty(){return Ok(0);}
 invalidate_groups(&audio_dir,None)?;
 for (id,pos) in &removed{
  let _=fs::remove_file(text_dir.join(format!("chuong_{pos:04}.txt")));
  for ext in ["mp3","m4b","m4a","opus","wav"]{let _=fs::remove_file(audio_dir.join(format!("chapter_{id}.{ext}")));let _=fs::remove_file(audio_dir.join(format!("chuong_{pos:04}.{ext}")));}
 }
 let _=db::delete_chapters(conn,ids)?;
 let after=db::all_chapters(conn)?;
 for c in after{
  let id=c["id"].as_i64().unwrap_or(0);let new_pos=c["position"].as_i64().unwrap_or(0);
  if let Some(old)=before.iter().find(|x|x["id"].as_i64()==Some(id)){
   let old_pos=old["position"].as_i64().unwrap_or(0);if old_pos==new_pos{continue;}
   let old_txt=text_dir.join(format!("chuong_{old_pos:04}.txt"));let new_txt=text_dir.join(format!("chuong_{new_pos:04}.txt"));
   if old_txt.is_file(){let _=fs::rename(old_txt,new_txt);}
   for ext in ["mp3","m4b","m4a","opus","wav"]{let old_audio=audio_dir.join(format!("chuong_{old_pos:04}.{ext}"));let new_audio=audio_dir.join(format!("chuong_{new_pos:04}.{ext}"));if old_audio.is_file(){let _=fs::rename(old_audio,new_audio);}}
  }
 }
 export_full_book_text(conn)?;Ok(removed.len())
}
pub fn set_title(conn:&Connection,title:&str)->Result<Value>{
 let title=title.trim();anyhow::ensure!(!title.is_empty()&&title.chars().count()<=200,"Tên truyện phải có từ 1 đến 200 ký tự.");
 let book=db::book(conn)?.context("Hãy nạp truyện trước.")?;
 let old=book["output_folder"].as_str().filter(|s|!s.is_empty()).unwrap_or(book["title"].as_str().unwrap_or("truyen")).to_string();
 let base=safe_name(title);let mut folder=if base==old{old.clone()}else{base.clone()};let mut suffix=2usize;
 let text_root=config::output_text_dir();let audio_root=config::output_audio_dir();fs::create_dir_all(&text_root)?;fs::create_dir_all(&audio_root)?;
 while folder!=old&&(text_root.join(&folder).exists()||audio_root.join(&folder).exists()){let tail=format!("_{suffix}");folder=format!("{}{}",base.chars().take(80usize.saturating_sub(tail.len())).collect::<String>(),tail);suffix+=1;}
 let old_text=text_root.join(&old);let old_audio=audio_root.join(&old);let new_text=text_root.join(&folder);let new_audio=audio_root.join(&folder);
 let mut moved=Vec::new();
 if folder!=old{
  if old_text.exists(){fs::rename(&old_text,&new_text)?;moved.push((new_text.clone(),old_text.clone()));}
  if old_audio.exists(){if let Err(e)=fs::rename(&old_audio,&new_audio){for (n,o) in moved.iter().rev(){let _=fs::rename(n,o);}return Err(e.into());}moved.push((new_audio.clone(),old_audio.clone()));}
 }
 let result=conn.execute("UPDATE book SET title=?,output_folder=? WHERE id=(SELECT id FROM book LIMIT 1)",rusqlite::params![title,folder]);
 if let Err(e)=result{for (n,o) in moved.iter().rev(){let _=fs::rename(n,o);}return Err(e.into());}
 db::book(conn)?.context("Không tìm thấy truyện sau khi đổi tên").map(|b|b)
}
pub fn clear(conn:&mut Connection)->Result<()>{
 if let Some(book)=db::book(conn)?{let folder=book["output_folder"].as_str().filter(|s|!s.is_empty()).unwrap_or(book["title"].as_str().unwrap_or("")).to_string();
  for root in [config::output_text_dir(),config::output_audio_dir()]{let path=root.join(&folder);if path.exists(){let root=root.canonicalize()?;let resolved=path.canonicalize()?;anyhow::ensure!(resolved.starts_with(root),"Không xóa thư mục ngoài output root.");fs::remove_dir_all(resolved)?;}}
 }
 db::clear(conn)?;Ok(())
}
