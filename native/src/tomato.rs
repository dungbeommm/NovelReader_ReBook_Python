//! Rust port of the Python Tomato bridge. Downloader remains an independent service.
use std::{collections::{HashSet, VecDeque}, sync::atomic::Ordering, time::{Duration, SystemTime, UNIX_EPOCH}};
use anyhow::{bail, Context, Result};
use axum::{extract::State, http::StatusCode, Json};
use reqwest::{Method, Url};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use crate::{db, text, App, ApiResult, error, storage_error};

#[derive(Debug, Clone, Serialize)]
pub struct TomatoStatus {
    pub stage: String,
    pub active: bool,
    pub message: String,
    pub book_id: Option<String>,
    pub book_title: Option<String>,
    pub saved: u64,
    pub total: u64,
    pub error: Option<String>,
}
impl Default for TomatoStatus {
    fn default() -> Self {
        Self {
            stage: "idle".into(), active: false, message: "Chưa tải truyện.".into(),
            book_id: None, book_title: None, saved: 0, total: 0, error: None,
        }
    }
}

#[derive(Deserialize)]
pub struct TomatoStart {
    book_id: String,
    #[serde(default)]
    replace_existing: bool,
    // Keep the existing frontend payload compatible. Translation is not started yet.
    #[serde(default)]
    config: Option<Value>,
}

fn update(app: &App, action: impl FnOnce(&mut TomatoStatus)) {
    if let Ok(mut state) = app.tomato.lock() { action(&mut state); }
}
fn clean_id(value: &str) -> bool {
    (8..=25).contains(&value.len()) && value.bytes().all(|c| c.is_ascii_digit())
}

fn local_url(app: &App, path: &str) -> Result<Url> {
    let base = Url::parse(&format!("{}/", app.tomato_url))?;
    Ok(base.join(path.trim_start_matches('/'))?)
}

async fn send(app: &App, method: Method, url: Url, body: Option<Value>) -> Result<reqwest::Response> {
    let mut builder = app.client.request(method, url);
    if !app.tomato_password.is_empty() {
        builder = builder.header("X-Tomato-Password", &app.tomato_password);
    }
    if let Some(json) = body { builder = builder.json(&json); }
    let res = builder.send().await.context("Không kết nối được Tomato Downloader.")?;
    let status = res.status();
    if !status.is_success() {
        bail!("Tomato API HTTP {status}: {}", res.text().await.unwrap_or_default().chars().take(300).collect::<String>());
    }
    Ok(res)
}
async fn request(app: &App, method: Method, path: &str, body: Option<Value>) -> Result<Value> {
    let response = send(app, method, local_url(app, path)?, body).await?;
    let content = response.bytes().await?;
    if content.len() > 2 * 1024 * 1024 { bail!("Tomato JSON vượt 2 MB."); }
    Ok(serde_json::from_slice(&content)?)
}
fn progress_number(value: &Value, field: &str) -> u64 {
    value.get("progress").and_then(|p|p.get(field)).and_then(Value::as_u64).unwrap_or(0)
}
fn job_path(job_id: &str, suffix: &str) -> Result<String> {
    anyhow::ensure!(!job_id.is_empty() && job_id.bytes().all(|c|c.is_ascii_alphanumeric() || c == b'_' || c == b'-'), "ID job Tomato không hợp lệ.");
    Ok(format!("/api/jobs/{job_id}/{suffix}"))
}
async fn job(app: &App, id: &str) -> Result<Value> {
    let mut url = local_url(app, "/api/jobs")?;
    url.query_pairs_mut().append_pair("id", id).append_pair("all", "true");
    let value = send(app, Method::GET, url, None).await?.json::<Value>().await?;
    value.get("items").and_then(Value::as_array).and_then(|rows| {
        rows.iter().find(|row| row.get("id").is_some_and(|v|v.to_string().trim_matches('"') == id))
    }).cloned().context("Tomato không còn lưu trạng thái tác vụ.")
}

async fn files(app: &App, path: &str) -> Result<Vec<Value>> {
    let mut first = true;
    for _ in 0..24 {
        let mut url = local_url(app, "/api/library")?;
        url.query_pairs_mut().append_pair("path", path)
            .append_pair("start", if first {"true"} else {"false"});
        first = false;
        let value = send(app, Method::GET, url, None).await?.json::<Value>().await?;
        if value.get("error").and_then(Value::as_str).is_some() {
            bail!("Tomato không đọc được thư viện file: {}", value["error"]);
        }
        if !value.get("running").and_then(Value::as_bool).unwrap_or(false) {
            return Ok(value.get("items").and_then(Value::as_array).cloned().unwrap_or_default());
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    bail!("Tomato chưa hoàn tất quét thư viện.")
}

fn normalize_title(name: &str) -> String {
    name.chars().filter(|c| !c.is_whitespace()).collect::<String>().to_lowercase()
}

async fn find_text(app: &App, book_id: &str, title: &str, started_ms: u64) -> Result<(String, Vec<u8>)> {
    let mut pending = VecDeque::from([(String::new(), 0u8)]);
    let mut visited = HashSet::new();
    let mut candidates: Vec<Value> = Vec::new();
    while let Some((path, depth)) = pending.pop_front() {
        if visited.len() >= 80 { break; }
        if !visited.insert(path.clone()) { continue; }
        for item in files(app, &path).await? {
            let relative = item.get("rel_path").and_then(Value::as_str).unwrap_or("");
            if relative.is_empty() || relative.replace('\\', "/").split('/').any(|x| x == ".." || x == ".") { continue; }
            match item.get("kind").and_then(Value::as_str) {
                Some("dir") if depth < 3 => pending.push_back((relative.into(), depth+1)),
                Some("file") => {
                    let ext = item.get("ext").and_then(Value::as_str).unwrap_or("");
                    let size = item.get("size").and_then(Value::as_u64).unwrap_or(0);
                    if (ext == "txt" || ext == "epub") && size > 0 && size <= text::MAX_FILE as u64 {
                        candidates.push(item);
                    }
                },
                _ => {}
            }
        }
    }
    let title_key = normalize_title(title);
    let mut matches: Vec<Value> = candidates.iter().filter(|item| {
        let path = item["rel_path"].as_str().unwrap_or("");
        path.contains(book_id) || (!title_key.is_empty() && normalize_title(path).contains(&title_key))
    }).cloned().collect();
    if matches.is_empty() {
        matches = candidates.into_iter().filter(|item| {
            item["modified_ms"].as_u64().unwrap_or(0) >= started_ms.saturating_sub(5000)
        }).collect();
        if matches.len() != 1 {
            bail!("Không xác định được TXT/EPUB vừa tải. Hãy bật xuất TXT trong Tomato.");
        }
    }
    matches.sort_by(|a,b| {
        let rank = |v: &Value| {
            (v["modified_ms"].as_u64().unwrap_or(0) >= started_ms.saturating_sub(5000),
             v["ext"].as_str() == Some("txt"),v["modified_ms"].as_u64().unwrap_or(0))
        };
        rank(b).cmp(&rank(a))
    });
    let selected = &matches[0];
    let name = selected["name"].as_str().context("File Tomato thiếu tên")?.to_owned();
    if !name.to_ascii_lowercase().ends_with(".txt") {
        bail!("Tomato chỉ xuất EPUB; bản Rust này chưa hỗ trợ EPUB. Hãy chọn TXT.");
    }
    let relative = selected["rel_path"].as_str().context("File Tomato thiếu đường dẫn")?;
    let mut url = local_url(app, "/")?;
    {
        let mut segments = url.path_segments_mut().map_err(|_|anyhow::anyhow!("URL invalid"))?;
        segments.push("download");
        for s in relative.replace('\\', "/").split('/') {
            if s.is_empty() || s=="." || s==".." { bail!("Đường dẫn Tomato không an toàn."); }
            segments.push(s);
        }
    }
    let bytes = send(app, Method::GET, url, None).await?.bytes().await?;
    if bytes.len() > text::MAX_FILE { bail!("Truyện vượt giới hạn nhập 50 MB."); }
    Ok((name, bytes.to_vec()))
}

pub async fn status(State(app): State<App>) -> Json<TomatoStatus> {
    Json(app.tomato.lock().map(|x|x.clone()).unwrap_or_default())
}
pub async fn start(State(app): State<App>, Json(payload): Json<TomatoStart>) -> ApiResult {
    if !clean_id(&payload.book_id) {
        return Err(error(StatusCode::BAD_REQUEST, "ID truyện phải gồm 8–25 chữ số."));
    }
    {
        let state = app.tomato.lock().map_err(|_|error(StatusCode::INTERNAL_SERVER_ERROR,"Tomato lock"))?;
        if state.active { return Err(error(StatusCode::CONFLICT,"Đang tải truyện khác.")); }
    }
    {
        let conn = app.database.lock().map_err(|_|error(StatusCode::INTERNAL_SERVER_ERROR,"SQLite lock"))?;
        if db::book(&conn).map_err(storage_error)?.is_some() && !payload.replace_existing {
            return Err(error(StatusCode::CONFLICT, "Đã có truyện; phải xác nhận thay thế."));
        }
    }
    // Fail before marking the task active when the downloader is not available.
    request(&app, Method::GET, "/api/status", None).await
        .map_err(|e|error(StatusCode::SERVICE_UNAVAILABLE, e.to_string()))?;
    app.cancel.store(false, Ordering::SeqCst);
    update(&app, |s| {
        *s = TomatoStatus {stage:"starting".into(),active:true,
            message:"Đang gửi ID đến Tomato Downloader…".into(),
            book_id:Some(payload.book_id.clone()), ..Default::default()};
    });
    let book_id = payload.book_id;
    let _config = payload.config; // preserved in contract for the forthcoming translation worker
    let runner = app.clone();
    tokio::spawn(async move {
        let result = run(&runner, &book_id).await;
        if let Err(err) = result {
            let canceled = runner.cancel.load(Ordering::SeqCst);
            update(&runner, |s| { s.active=false; s.stage=if canceled {"canceled"} else {"error"}.into();
                s.error=Some(err.to_string()); s.message=err.to_string(); });
        }
    });
    Ok(Json(serde_json::to_value(app.tomato.lock()
        .map_err(|_|error(StatusCode::INTERNAL_SERVER_ERROR,"Tomato lock"))?.clone())
        .map_err(|e|error(StatusCode::INTERNAL_SERVER_ERROR,e.to_string()))?))
}
pub async fn cancel(State(app): State<App>) -> ApiResult {
    {
        let state = app.tomato.lock().map_err(|_|error(StatusCode::INTERNAL_SERVER_ERROR,"Tomato lock"))?;
        if !state.active || !["starting", "downloading"].contains(&state.stage.as_str()) {
            return Err(error(StatusCode::CONFLICT, "Chỉ được hủy khi đang tải chương."));
        }
    }
    app.cancel.store(true, Ordering::SeqCst);
    update(&app, |s|s.message="Đang hủy tải truyện…".into());
    Ok(Json(serde_json::to_value(app.tomato.lock()
        .map_err(|_|error(StatusCode::INTERNAL_SERVER_ERROR,"Tomato lock"))?.clone())
        .map_err(|e|error(StatusCode::INTERNAL_SERVER_ERROR,e.to_string()))?))
}
async fn run(app: &App, book_id: &str) -> Result<()> {
    let started_ms = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis() as u64;
    let created = request(app, Method::POST, "/api/jobs", Some(json!({"book_id":book_id}))).await?;
    let job_id = created.get("id").context("Tomato không trả job id")?;
    let job_id = job_id.as_str().map(str::to_owned).unwrap_or_else(||job_id.to_string());
    let mut final_progress = Value::Null;
    let mut last_title = String::new();
    loop {
        if app.cancel.load(Ordering::SeqCst) {
            if let Ok(path) = job_path(&job_id, "cancel") {
                let _ = request(app, Method::POST, &path, None).await;
            }
            bail!("Đã hủy tải truyện.");
        }
        let info = job(app, &job_id).await?;
        let saved = progress_number(&info, "saved_chapters");
        let total = progress_number(&info, "chapter_total");
        if let Some(title) = info.get("title").and_then(Value::as_str) {
            last_title = title.to_owned();
        }
        update(app, |s| {s.stage="downloading".into(); s.saved=saved; s.total=total;
            if !last_title.is_empty() {s.book_title=Some(last_title.clone());}
            s.message=info.get("message").and_then(Value::as_str)
                .unwrap_or("Đang tải nội dung chữ Hán…").to_owned();});
        if let Some(choice) = info["book_name_options"].as_array().and_then(|x|x.first()) {
            if let Some(value) = choice.get("value") {
                request(app, Method::POST, &job_path(&job_id,"book_name")?, Some(json!({"value":value}))).await?;
            }
        }
        if let Some(choices) = info["format_options"].as_array() {
            let preferred = choices.iter().find(|x|x["value"]=="txt").or_else(||choices.first());
            if let Some(value) = preferred.and_then(|x|x.get("value")) {
                request(app, Method::POST, &job_path(&job_id,"format")?,Some(json!({"value":value}))).await?;
            }
        }
        match info["state"].as_str().unwrap_or("") {
            "done" => {
                if total > 0 && saved < total { bail!("Tomato mới tải {saved}/{total} chương."); }
                final_progress = info["progress"].clone();
                break;
            },
            "failed" | "canceled" => bail!("Tomato thất bại: {}",info["message"]),
            _ => tokio::time::sleep(Duration::from_millis(1500)).await,
        }
    }
    update(app, |s|{s.stage="importing".into();s.message="Đang nhập TXT và tách chương…".into();});
    let (filename, data) = find_text(app, book_id, &last_title, started_ms).await?;
    if app.cancel.load(Ordering::SeqCst) { bail!("Đã hủy tải truyện."); }
    let (decoded,encoding)=text::decode_txt(&data)?;
    let chapters=text::split_chapters(&decoded)?;
    let downloaded=final_progress["saved_chapters"].as_u64().unwrap_or(0);
    if downloaded > 0 && (chapters.len() as u64)*10 < downloaded*9 {
        bail!("Tệp chỉ tách được {}/{} chương; kiểm tra Tomato.",chapters.len(),downloaded);
    }
    {
        let mut conn=app.database.lock().map_err(|_|anyhow::anyhow!("SQLite lock"))?;
        db::replace(&mut conn,&filename,&encoding,&chapters)?;
    }
    update(app, |s|{s.stage="done".into();s.active=false;
        s.saved=chapters.len() as u64;s.total=chapters.len() as u64;
        s.message=format!("Đã nạp {} chương vào danh sách.",chapters.len());s.error=None;});
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::clean_id;
    #[test]
    fn rejects_non_numeric_and_path_traversal() {
        assert!(clean_id("7318247498772674083"));
        assert!(!clean_id("../internal"));
        assert!(!clean_id("https://example.com"));
    }
}
