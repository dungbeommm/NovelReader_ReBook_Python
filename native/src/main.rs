mod db;
mod text;
mod tomato;
mod files;
mod checkpoints;
mod monitoring;
mod audio;
mod model_manager;
mod config;
mod ct2;
mod translation;
mod store;
mod jobs;

use std::{net::SocketAddr, path::PathBuf, sync::{atomic::AtomicBool, Arc, Mutex}, time::Duration};

use axum::{
    extract::{DefaultBodyLimit, Multipart, Path, State, WebSocketUpgrade, ws::{Message, WebSocket}},
    http::{header, HeaderMap, Method, Request, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use rusqlite::Connection;
use serde::Deserialize;
use serde_json::{json, Value};
use tower_http::services::{ServeDir, ServeFile};

#[derive(Clone)]
struct App {
    database: Arc<Mutex<Connection>>,
    tomato: Arc<Mutex<tomato::TomatoStatus>>,
    cancel: Arc<AtomicBool>,
    client: reqwest::Client,
    tomato_url: String,
    tomato_password: String,
}

type ApiError = (StatusCode, Json<Value>);
type ApiResult = Result<Json<Value>, ApiError>;

fn error(code: StatusCode, message: impl Into<String>) -> ApiError {
    (code, Json(json!({"detail":message.into()})))
}

fn storage_error(err: anyhow::Error) -> ApiError {
    error(StatusCode::INTERNAL_SERVER_ERROR, format!("Lỗi SQLite: {err}"))
}

async fn guard(request: Request<axum::body::Body>, next: Next) -> Response {
    let headers = request.headers();
    let host = headers.get(header::HOST).and_then(|x| x.to_str().ok()).unwrap_or("");
    if let Some(origin) = headers.get(header::ORIGIN).and_then(|x| x.to_str().ok()) {
        let matches = url::Url::parse(origin).ok()
            .and_then(|u| u.host_str().map(|h| match u.port() {
                Some(port) => format!("{h}:{port}"),
                None => h.to_string()
            })).is_some_and(|origin_host| origin_host == host);
        if !matches { return StatusCode::FORBIDDEN.into_response(); }
    }
    if request.method() != Method::GET && request.method() != Method::HEAD &&
       request.method() != Method::OPTIONS {
        let request_tag = headers.get("x-app-request").and_then(|x| x.to_str().ok());
        if request_tag != Some("novel-translator") {
            return error(StatusCode::FORBIDDEN, "Thiếu header bảo vệ.").into_response();
        }
    }
    next.run(request).await
}

fn snapshot(app: &App) -> Result<Value, anyhow::Error> {
    let conn = app.database.lock().map_err(|_| anyhow::anyhow!("SQLite lock"))?;
    Ok(json!({"book": db::book(&conn)?, "progress": db::progress(&conn)?}))
}

async fn state(State(app): State<App>) -> ApiResult {
    snapshot(&app).map(Json).map_err(storage_error)
}

async fn chapters(State(app): State<App>) -> ApiResult {
    let conn = app.database.lock().map_err(|_| error(StatusCode::INTERNAL_SERVER_ERROR, "SQLite lock"))?;
    db::chapters(&conn).map(|rows| Json(json!(rows))).map_err(storage_error)
}

async fn chapter(State(app): State<App>, Path(id): Path<i64>) -> ApiResult {
    let conn = app.database.lock().map_err(|_| error(StatusCode::INTERNAL_SERVER_ERROR, "SQLite lock"))?;
    db::chapter(&conn, id).map_err(storage_error)?.map(Json)
        .ok_or_else(|| error(StatusCode::NOT_FOUND, "Không tìm thấy chương."))
}

async fn upload(State(app): State<App>, mut multipart: Multipart) -> ApiResult {
    if app.tomato.lock().map_err(|_| error(StatusCode::INTERNAL_SERVER_ERROR, "Tomato lock"))?.active {
        return Err(error(StatusCode::CONFLICT, "Đang tải truyện bằng ID."));
    }
    let mut file = None;
    while let Some(field) = multipart.next_field().await
        .map_err(|e| error(StatusCode::BAD_REQUEST, format!("Upload lỗi: {e}")))? {
        if field.name() != Some("file") { continue; }
        let name = field.file_name().unwrap_or("novel.txt").replace('\\', "/");
        let name = name.rsplit('/').next().unwrap_or("novel.txt").to_string();
        let data = field.bytes().await
            .map_err(|e| error(StatusCode::BAD_REQUEST, format!("Upload lỗi: {e}")))?;
        file = Some((name, data));
        break;
    }
    let (filename, data) = file.ok_or_else(|| error(StatusCode::BAD_REQUEST, "Thiếu trường file."))?;
    if data.len() > text::MAX_FILE {
        return Err(error(StatusCode::PAYLOAD_TOO_LARGE, "File tối đa 50 MB."));
    }
    let (items, encoding) = files::parse_file(&filename, &data)
        .map_err(|e| error(StatusCode::BAD_REQUEST, e.to_string()))?;
    let mut conn = app.database.lock().map_err(|_| error(StatusCode::INTERNAL_SERVER_ERROR, "SQLite lock"))?;
    db::replace(&mut conn, &filename, &encoding, &items).map_err(storage_error)?;
    Ok(Json(json!({"book":db::book(&conn).map_err(storage_error)?,
        "chapters":db::chapters(&conn).map_err(storage_error)?})))
}

async fn clear_book(State(app): State<App>) -> ApiResult {
    if app.tomato.lock().map_err(|_| error(StatusCode::INTERNAL_SERVER_ERROR, "Tomato lock"))?.active {
        return Err(error(StatusCode::CONFLICT, "Đang tải truyện bằng ID."));
    }
    let mut conn = app.database.lock().map_err(|_| error(StatusCode::INTERNAL_SERVER_ERROR, "SQLite lock"))?;
    store::clear(&mut conn).map_err(storage_error)?;
    *app.tomato.lock().map_err(|_| error(StatusCode::INTERNAL_SERVER_ERROR, "Tomato lock"))? =
        tomato::TomatoStatus::default();
    Ok(Json(json!({"ok":true,"book":null,"chapters":[]})))
}

#[derive(Deserialize)]
struct Rename { title: String }

#[derive(Deserialize)]
struct ChapterIds { #[serde(default)] ids: Vec<i64> }

#[derive(Deserialize)]
struct CheckpointOptions { #[serde(default)] include_audio: bool }

async fn rename(State(app): State<App>, Json(payload): Json<Rename>) -> ApiResult {
    if app.tomato.lock().map_err(|_| error(StatusCode::INTERNAL_SERVER_ERROR, "Tomato lock"))?.active {
        return Err(error(StatusCode::CONFLICT, "Đang tải truyện bằng ID."));
    }
    let conn = app.database.lock().map_err(|_| error(StatusCode::INTERNAL_SERVER_ERROR, "SQLite lock"))?;
    let book = store::set_title(&conn, &payload.title).map_err(|e|error(StatusCode::BAD_REQUEST,e.to_string()))?;
    Ok(Json(json!({"ok":true,"book":book})))
}

async fn not_ported() -> ApiError {
    error(StatusCode::NOT_IMPLEMENTED,
        "Chức năng này chưa được chuyển sang Rust/C++. Hãy dùng Python main cho đến khi engine native hoàn thiện.")
}

async fn monitor(State(app): State<App>) -> ApiResult {
    let conn = app.database.lock().map_err(|_|error(StatusCode::INTERNAL_SERVER_ERROR, "SQLite lock"))?;
    let job = db::progress(&conn).map_err(storage_error)?;
    let data = config::data_dir();
    let paths = vec![
        ("Dữ liệu".to_string(), data),
        ("Văn bản".to_string(), config::output_text_dir()),
        ("Audio".to_string(), config::output_audio_dir()),
    ];
    Ok(Json(json!({"job":job,"resources":monitoring::snapshot(&paths),"tasks":null})))
}

async fn export_book(State(app): State<App>, Path(format): Path<String>) -> Result<Response, ApiError> {
    if !["txt", "epub"].contains(&format.as_str()) {
        return Err(error(StatusCode::NOT_FOUND, "Định dạng không hợp lệ."));
    }
    let conn = app.database.lock().map_err(|_| error(StatusCode::INTERNAL_SERVER_ERROR, "SQLite lock"))?;
    let book = db::book(&conn).map_err(storage_error)?
        .ok_or_else(|| error(StatusCode::BAD_REQUEST, "Chưa có truyện."))?;
    let chapters = db::all_chapters(&conn).map_err(storage_error)?;
    let bytes = if format == "txt" {
        files::export_txt(&book, &chapters)
    } else {
        files::export_epub(&book, &chapters)
            .map_err(|e| error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
    };
    let content_type = if format == "txt" { "text/plain; charset=utf-8" } else { "application/epub+zip" };
    Ok(Response::builder().status(StatusCode::OK)
        .header(header::CONTENT_TYPE, content_type)
        .header(header::CONTENT_DISPOSITION, format!("attachment; filename=translated.{format}"))
        .body(axum::body::Body::from(bytes))
        .map_err(|e| error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?)
}

async fn reset_chapters(State(app): State<App>, Json(payload): Json<ChapterIds>) -> ApiResult {
    if payload.ids.is_empty() { return Err(error(StatusCode::BAD_REQUEST,"Danh sách chương rỗng.")); }
    let mut conn=app.database.lock().map_err(|_|error(StatusCode::INTERNAL_SERVER_ERROR,"SQLite lock"))?;
    let count=store::reset_chapters(&mut conn,&payload.ids).map_err(storage_error)?;
    Ok(Json(json!({"ok":true,"reset_count":count})))
}
async fn delete_chapters(State(app): State<App>, Json(payload): Json<ChapterIds>) -> ApiResult {
    if payload.ids.is_empty() { return Err(error(StatusCode::BAD_REQUEST,"Danh sách chương rỗng.")); }
    let mut conn=app.database.lock().map_err(|_|error(StatusCode::INTERNAL_SERVER_ERROR,"SQLite lock"))?;
    let count=store::delete_chapters(&mut conn,&payload.ids).map_err(storage_error)?;
    let rows=db::chapters(&conn).map_err(storage_error)?;
    Ok(Json(json!({"ok":true,"deleted_count":count,"chapters":rows})))
}
async fn purge_empty_chapters(State(app): State<App>) -> ApiResult {
    let mut conn=app.database.lock().map_err(|_|error(StatusCode::INTERNAL_SERVER_ERROR,"SQLite lock"))?;
    let ids=db::purge_empty_chapters(&mut conn).map_err(storage_error)?;
    let rows=db::chapters(&conn).map_err(storage_error)?;
    Ok(Json(json!({"ok":true,"purged_count":ids.len(),"chapters":rows})))
}
async fn export_options(State(app): State<App>) -> ApiResult {
    let conn=app.database.lock().map_err(|_|error(StatusCode::INTERNAL_SERVER_ERROR,"SQLite lock"))?;
    let book=db::book(&conn).map_err(storage_error)?;
    let rows=db::all_chapters(&conn).map_err(storage_error)?;
    let (audio_count,audio_bytes)=if book.is_some(){
        let (_,audio_dir)=store::book_dirs(&conn).map_err(storage_error)?;
        let files=audio::list_audio_files(&audio_dir).map_err(storage_error)?;
        let count=files.len();let bytes=files.iter().filter_map(|x|x["size_bytes"].as_u64()).sum::<u64>();(count,bytes)
    }else{(0,0)};
    Ok(Json(json!({"book_title":book.as_ref().and_then(|b|b["title"].as_str()),"chapters":rows.len(),
        "translated":rows.iter().filter(|c|c["status"]=="done").count(),"audio_files":audio_count,
        "audio_bytes":audio_bytes,"busy":false})))
}
async fn checkpoint_download(State(app): State<App>, Json(options): Json<CheckpointOptions>) -> Result<Response,ApiError> {
    let conn=app.database.lock().map_err(|_|error(StatusCode::INTERNAL_SERVER_ERROR,"SQLite lock"))?;
    let (text_dir,audio_dir)=store::book_dirs(&conn).map_err(|e|error(StatusCode::BAD_REQUEST,e.to_string()))?;
    let work=tempfile::tempdir().map_err(|e|error(StatusCode::INTERNAL_SERVER_ERROR,e.to_string()))?;
    let path=work.path().join("NovelReader_checkpoint.zip");
    let draft=config::data_dir().join("workspace_draft.json");
    checkpoints::create_checkpoint(&conn,&text_dir,&audio_dir,Some(&draft),&path,options.include_audio)
        .map_err(|e|error(StatusCode::BAD_REQUEST,e.to_string()))?;
    let bytes=std::fs::read(path).map_err(|e|error(StatusCode::INTERNAL_SERVER_ERROR,e.to_string()))?;
    Ok(Response::builder().status(StatusCode::OK).header(header::CONTENT_TYPE,"application/zip")
        .header(header::CONTENT_DISPOSITION,"attachment; filename=NovelReader_checkpoint.zip")
        .body(axum::body::Body::from(bytes)).map_err(|e|error(StatusCode::INTERNAL_SERVER_ERROR,e.to_string()))?)
}
async fn save_draft(State(_app): State<App>, Json(config_value): Json<Value>) -> ApiResult {
    let path=config::data_dir().join("workspace_draft.json");
    checkpoints::save_draft(&path,&config_value).map_err(|e|error(StatusCode::BAD_REQUEST,e.to_string()))?;
    Ok(Json(json!({"ok":true,"config":config_value})))
}
async fn load_draft(State(_app): State<App>) -> ApiResult {
    let path=config::data_dir().join("workspace_draft.json");
    let value=checkpoints::load_draft(&path).map_err(storage_error)?
        .ok_or_else(||error(StatusCode::NOT_FOUND,"Chưa có bảng tạm đã lưu."))?;
    Ok(Json(json!({"config":value})))
}
async fn checkpoint_status(State(_app): State<App>) -> ApiResult {
    let path=config::data_dir().join("workspace_draft.json");
    let draft=config::data_dir().join("cloud_checkpoint_last.json");
    let last=std::fs::read(&draft).ok().and_then(|b|serde_json::from_slice::<Value>(&b).ok());
    Ok(Json(json!({"last_cloud":last,"has_draft":path.is_file(),"max_archive_bytes":checkpoints::MAX_ARCHIVE})))
}

async fn websocket(State(app): State<App>, upgrade: WebSocketUpgrade) -> impl IntoResponse {
    upgrade.on_upgrade(move |socket| async move { ws_loop(socket, app).await; })
}

async fn ws_loop(mut socket: WebSocket, app: App) {
    loop {
        let progress = match snapshot(&app) {
            Ok(state) => state.get("progress").cloned().unwrap_or(Value::Null),
            Err(_) => break,
        };
        let value = json!({"type":"snapshot","progress":progress}).to_string();
        if socket.send(Message::Text(value.into())).await.is_err() { break; }
        tokio::time::sleep(Duration::from_secs(3)).await;
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let repo = std::env::current_dir()?;
    let data_root = std::env::var("NOVEL_DATA").map(PathBuf::from)
        .unwrap_or_else(|_| repo.join("data"));
    let static_dir = std::env::var("NOVEL_WEB_DIR").map(PathBuf::from)
        .unwrap_or_else(|_| repo.join("web/static"));
    if !static_dir.join("index.html").is_file() {
        anyhow::bail!("Không tìm thấy WebUI: {}. Chạy binary từ thư mục repo hoặc đặt NOVEL_WEB_DIR.",
            static_dir.display());
    }
    let tomato_url = std::env::var("TOMATO_WEB_URL")
        .unwrap_or_else(|_| "http://127.0.0.1:18423".to_string());
    let parsed = url::Url::parse(&tomato_url)?;
    anyhow::ensure!(
        (parsed.scheme() == "http" || parsed.scheme() == "https") &&
        parsed.host_str().is_some() && parsed.username().is_empty() &&
        parsed.password().is_none() && parsed.query().is_none() &&
        parsed.fragment().is_none() && (parsed.path() == "/" || parsed.path().is_empty()),
        "TOMATO_WEB_URL phải là địa chỉ gốc HTTP(S) hợp lệ."
    );
    let db = db::open(&data_root.join("novel.sqlite3"))?;
    let app = App {
        database: Arc::new(Mutex::new(db)),
        tomato: Arc::new(Mutex::new(tomato::TomatoStatus::default())),
        cancel: Arc::new(AtomicBool::new(false)),
        client: reqwest::Client::builder().timeout(Duration::from_secs(35)).build()?,
        tomato_url: tomato_url.trim_end_matches('/').to_string(),
        tomato_password: std::env::var("TOMATO_WEB_PASSWORD").unwrap_or_default(),
    };
    let router = Router::new()
        .route("/", get(|| async { axum::response::Redirect::temporary("/static/index.html") }))
        .route("/classic", get(|| async { axum::response::Redirect::temporary("/static/index.html") }))
        .route("/api/state", get(state))
        .route("/api/chapters", get(chapters))
        .route("/api/chapters/{id}", get(chapter))
        .route("/api/upload", post(upload))
        .route("/api/book/delete", post(clear_book))
        .route("/api/book/title", post(rename))
        .route("/api/chapters/reset", post(reset_chapters))
        .route("/api/chapters/delete", post(delete_chapters))
        .route("/api/chapters/purge-empty", post(purge_empty_chapters))
        .route("/api/checkpoints/status", get(checkpoint_status))
        .route("/api/checkpoints/draft", post(save_draft).get(load_draft))
        .route("/api/checkpoints/download", post(checkpoint_download))
        .route("/api/export/options", get(export_options))
        .route("/api/monitor", get(monitor))
        .route("/api/export/{format}", get(export_book))
        .route("/ws/progress", get(websocket))
        .route("/api/tomato/status", get(tomato::status))
        .route("/api/tomato/start", post(tomato::start))
        .route("/api/tomato/cancel", post(tomato::cancel))
        .route("/api/translate/start", post(not_ported))
        .route("/api/translate/{action}", post(not_ported))
        .route("/api/chapters/{id}/audio", post(not_ported))
        .route("/api/models/download", post(not_ported))
        .nest_service("/static", ServeDir::new(static_dir))
        .layer(DefaultBodyLimit::max(text::MAX_FILE + 1024 * 1024))
        .layer(middleware::from_fn(guard))
        .with_state(app);
    let bind: SocketAddr = std::env::var("NOVEL_BIND")
        .unwrap_or_else(|_| "127.0.0.1:8000".into()).parse()?;
    let listener = tokio::net::TcpListener::bind(bind).await?;
    println!("NovelReader native phase 1: http://{bind}/");
    println!("Lưu ý: dịch/TTS/EPUB chưa được port; server trả về lỗi rõ ràng.");
    axum::serve(listener, router).await?;
    Ok(())
}
