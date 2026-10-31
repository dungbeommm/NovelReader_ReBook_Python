use anyhow::{Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};
use std::path::Path;

pub fn open(path: &Path) -> Result<Connection> {
    if let Some(parent) = path.parent() { std::fs::create_dir_all(parent)?; }
    let conn = Connection::open(path)?;
    conn.execute_batch(r#"
        PRAGMA journal_mode=WAL;
        PRAGMA busy_timeout=30000;
        CREATE TABLE IF NOT EXISTS book(
            id INTEGER PRIMARY KEY, title TEXT, filename TEXT, encoding TEXT,
            config TEXT DEFAULT '{}', output_folder TEXT DEFAULT ''
        );
        CREATE TABLE IF NOT EXISTS chapter(
            id INTEGER PRIMARY KEY, position INTEGER, title TEXT, source TEXT,
            status TEXT DEFAULT 'pending', plan TEXT DEFAULT '[]',
            outputs TEXT DEFAULT '[]', translated_title TEXT DEFAULT '',
            translated TEXT DEFAULT '[]', error TEXT DEFAULT '',
            audio_status TEXT DEFAULT 'none'
        );
    "#)?;
    let mut statement = conn.prepare("PRAGMA table_info(book)")?;
    let fields = statement.query_map([], |row| row.get::<_, String>(1))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    drop(statement);
    if !fields.iter().any(|name| name == "output_folder") {
        conn.execute("ALTER TABLE book ADD COLUMN output_folder TEXT DEFAULT ''", [])?;
    }
    let mut statement = conn.prepare("PRAGMA table_info(chapter)")?;
    let fields = statement.query_map([], |row| row.get::<_, String>(1))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    drop(statement);
    if !fields.iter().any(|name| name == "audio_status") {
        conn.execute("ALTER TABLE chapter ADD COLUMN audio_status TEXT DEFAULT 'none'", [])?;
    }
    // Do not modify in-progress Python jobs: this migration keeps Python/main independent.
    Ok(conn)
}

pub fn book(conn: &Connection) -> Result<Option<Value>> {
    conn.query_row("SELECT id,title,filename,encoding,config,output_folder FROM book LIMIT 1",
        [], |row| {
            Ok(json!({
                "id": row.get::<_, i64>(0)?, "title": row.get::<_, String>(1)?,
                "filename": row.get::<_, String>(2)?, "encoding": row.get::<_, String>(3)?,
                "config": row.get::<_, String>(4)?, "output_folder": row.get::<_, String>(5)?
            }))
        }).optional().map_err(Into::into)
}

pub fn chapters(conn: &Connection) -> Result<Vec<Value>> {
    let mut query = conn.prepare("SELECT id,position,title,translated_title,status,error,audio_status FROM chapter ORDER BY position")?;
    let rows = query.query_map([], |row| {
        Ok(json!({"id": row.get::<_, i64>(0)?, "position": row.get::<_, i64>(1)?,
          "title": row.get::<_, String>(2)?, "translated_title": row.get::<_, String>(3)?,
          "status": row.get::<_, String>(4)?, "error": row.get::<_, String>(5)?,
          "audio_status": row.get::<_, String>(6)?}))
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

pub fn chapter(conn: &Connection, id: i64) -> Result<Option<Value>> {
    let raw: Option<(i64, i64, String, String, String, String, String, String, String, String, String)> =
      conn.query_row("SELECT id,position,title,source,status,plan,outputs,translated_title,translated,error,audio_status FROM chapter WHERE id=?",
        [id], |row| {
            Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?,
                row.get(5)?,row.get(6)?,row.get(7)?,row.get(8)?,row.get(9)?,row.get(10)?))
        }).optional()?;
    raw.map(|(id, position, title, source, status, plan, outputs, translated_title, translated, error, audio_status)| {
        Ok(json!({"id":id,"position":position,"title":title,
            "source":serde_json::from_str::<Value>(&source).context("source JSON")?,
            "status":status,"plan":serde_json::from_str::<Value>(&plan).context("plan JSON")?,
            "outputs":serde_json::from_str::<Value>(&outputs).context("outputs JSON")?,
            "translated_title":translated_title,
            "translated":serde_json::from_str::<Value>(&translated).context("translated JSON")?,
            "error":error,"audio_status":audio_status}))
    }).transpose()
}

fn safe_folder(title: &str) -> String {
    let mut name = title.chars().map(|c| if r#"\/:*?"<>|"#.contains(c) || c.is_control() { '_' } else { c }).collect::<String>();
    name = name.trim().trim_matches('.').chars().take(80).collect::<String>();
    if name.is_empty() { name = "truyen".into(); }
    name
}

pub fn replace(conn: &mut Connection, filename: &str, encoding: &str, items: &[(String, String)]) -> Result<()> {
    let title = filename.rsplit('/').next().unwrap_or(filename).rsplit_once('.')
        .map(|(name, _)| name).unwrap_or(filename).to_owned();
    let base = safe_folder(&title);
    let text_root = crate::config::output_text_dir();
    let audio_root = crate::config::output_audio_dir();
    std::fs::create_dir_all(&text_root)?;
    std::fs::create_dir_all(&audio_root)?;
    let mut folder = base.clone();
    let mut suffix = 2usize;
    while text_root.join(&folder).exists() || audio_root.join(&folder).exists() {
        let tail = format!("_{suffix}");
        folder = format!("{}{}", base.chars().take(80usize.saturating_sub(tail.len())).collect::<String>(), tail);
        suffix += 1;
    }
    let tx = conn.transaction()?;
    tx.execute("DELETE FROM chapter", [])?;
    tx.execute("DELETE FROM book", [])?;
    tx.execute("INSERT INTO book(id,title,filename,encoding,output_folder) VALUES(1,?,?,?,?)",
        params![title,filename,encoding,folder])?;
    for (index, (chapter_title, body)) in items.iter().enumerate() {
        let lines = crate::text::paragraphs(body);
        tx.execute("INSERT INTO chapter(position,title,source) VALUES(?,?,?)",
            params![index as i64 + 1, chapter_title, serde_json::to_string(&lines)?])?;
    }
    tx.commit()?;
    Ok(())
}

pub fn clear(conn: &mut Connection) -> Result<()> {
    let tx = conn.transaction()?;
    tx.execute("DELETE FROM chapter", [])?;
    tx.execute("DELETE FROM book", [])?;
    tx.commit()?;
    Ok(())
}

pub fn progress(conn: &Connection) -> Result<Value> {
    let (total, done): (i64,i64) = conn.query_row(
        "SELECT COUNT(*),COALESCE(SUM(CASE WHEN status='done' THEN 1 ELSE 0 END),0) FROM chapter", [],
        |row| Ok((row.get(0)?,row.get(1)?)))?;
    Ok(json!({"status":"ready","message":"","done":done,"total":total,
              "active":false,"scope":""}))
}

pub fn title(conn: &Connection, title: &str) -> Result<Value> {
    anyhow::ensure!(!title.trim().is_empty() && title.chars().count()<=200,
        "Tên truyện phải có từ 1 đến 200 ký tự.");
    anyhow::ensure!(book(conn)?.is_some(), "Hãy nạp truyện trước.");
    conn.execute("UPDATE book SET title=? WHERE id=(SELECT id FROM book LIMIT 1)", [title.trim()])?;
    Ok(book(conn)?.expect("book exists"))
}


pub fn all_chapters(conn:&Connection)->Result<Vec<Value>>{
    let mut query=conn.prepare("SELECT id,position,title,source,status,plan,outputs,translated_title,translated,error,audio_status FROM chapter ORDER BY position")?;
    let rows=query.query_map([],|row|{
        let source:String=row.get(3)?;let plan:String=row.get(5)?;let outputs:String=row.get(6)?;let translated:String=row.get(8)?;
        Ok(json!({"id":row.get::<_,i64>(0)?,"position":row.get::<_,i64>(1)?,"title":row.get::<_,String>(2)?,
            "source":serde_json::from_str::<Value>(&source).unwrap_or(json!([])),
            "status":row.get::<_,String>(4)?,"plan":serde_json::from_str::<Value>(&plan).unwrap_or(json!([])),
            "outputs":serde_json::from_str::<Value>(&outputs).unwrap_or(json!([])),
            "translated_title":row.get::<_,String>(7)?,"translated":serde_json::from_str::<Value>(&translated).unwrap_or(json!([])),
            "error":row.get::<_,String>(9)?,"audio_status":row.get::<_,String>(10)?}))
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

pub fn update_translation(conn:&Connection,id:i64,status:&str,plan:&Value,outputs:&Value,title:&str,translated:&Value,error:&str)->Result<()>{
    anyhow::ensure!(["pending","translating","done","error","stopped"].contains(&status),"Trạng thái chương không hợp lệ.");
    conn.execute("UPDATE chapter SET status=?,plan=?,outputs=?,translated_title=?,translated=?,error=? WHERE id=?",
        params![status,plan.to_string(),outputs.to_string(),title,translated.to_string(),error,id])?;
    anyhow::ensure!(conn.changes()==1,"Không tìm thấy chương để cập nhật.");
    Ok(())
}

pub fn update_audio_status(conn:&Connection,id:i64,status:&str)->Result<()>{
    anyhow::ensure!(["none","queued","generating","done","error"].contains(&status),"Trạng thái audio không hợp lệ.");
    conn.execute("UPDATE chapter SET audio_status=? WHERE id=?",params![status,id])?;
    anyhow::ensure!(conn.changes()==1,"Không tìm thấy chương.");
    Ok(())
}

pub fn save_config(conn:&Connection,config:&Value)->Result<()>{
    anyhow::ensure!(book(conn)?.is_some(),"Hãy nạp truyện trước.");
    conn.execute("UPDATE book SET config=?",[config.to_string()])?;
    Ok(())
}

pub fn reset_chapters(conn:&mut Connection,ids:&[i64])->Result<Vec<(i64,i64)>>{
    if ids.is_empty(){return Ok(vec![]);}
    let marks=vec!["?";ids.len()].join(",");
    let sql=format!("SELECT id,position FROM chapter WHERE id IN ({marks})");
    let rows={
        let mut q=conn.prepare(&sql)?;
        let mapped=q.query_map(rusqlite::params_from_iter(ids.iter()),|r|Ok((r.get::<_,i64>(0)?,r.get::<_,i64>(1)?)))?;
        mapped.collect::<rusqlite::Result<Vec<_>>>()?
    };
    let tx=conn.transaction()?;
    let sql=format!("UPDATE chapter SET status='pending',plan='[]',outputs='[]',translated_title='',translated='[]',error='',audio_status='none' WHERE id IN ({marks})");
    tx.execute(&sql,rusqlite::params_from_iter(ids.iter()))?;
    tx.commit()?;
    Ok(rows)
}

pub fn delete_chapters(conn:&mut Connection,ids:&[i64])->Result<Vec<(i64,i64,i64)>>{
    if ids.is_empty(){return Ok(vec![]);}
    let marks=vec!["?";ids.len()].join(",");
    let sql=format!("SELECT id,position FROM chapter WHERE id IN ({marks})");
    let removed={
        let mut q=conn.prepare(&sql)?;
        q.query_map(rusqlite::params_from_iter(ids.iter()),|r|Ok((r.get::<_,i64>(0)?,r.get::<_,i64>(1)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?
    };
    if removed.is_empty(){return Ok(vec![]);}
    let tx=conn.transaction()?;
    let sql=format!("DELETE FROM chapter WHERE id IN ({marks})");
    tx.execute(&sql,rusqlite::params_from_iter(ids.iter()))?;
    let remaining={
        let mut q=tx.prepare("SELECT id,position FROM chapter ORDER BY position,id")?;
        let mapped=q.query_map([],|r|Ok((r.get::<_,i64>(0)?,r.get::<_,i64>(1)?)))?;
        mapped.collect::<rusqlite::Result<Vec<_>>>()?
    };
    let mut shifts=Vec::new();
    for (new_pos,(id,old_pos)) in remaining.into_iter().enumerate(){
        let new_pos=new_pos as i64+1;
        if old_pos!=new_pos{tx.execute("UPDATE chapter SET position=? WHERE id=?",params![new_pos,id])?;shifts.push((id,old_pos,new_pos));}
    }
    tx.commit()?;
    Ok(removed.into_iter().map(|(id,pos)|(id,pos,0)).chain(shifts.into_iter().map(|(id,old,new)|(id,old,new))).collect())
}

pub fn purge_empty_chapters(conn:&mut Connection)->Result<Vec<i64>>{
    let rows=all_chapters(conn)?;
    let ids:Vec<i64>=rows.iter().filter(|c|{
        c["source"].as_array().map(|a|a.is_empty()||a.iter().all(|x|x.as_str().unwrap_or("").trim().is_empty())).unwrap_or(false)
    }).filter_map(|c|c["id"].as_i64()).collect();
    if !ids.is_empty(){let _=delete_chapters(conn,&ids)?;}
    Ok(ids)
}
