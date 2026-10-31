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
use crate::config::AppConfig;
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
    job: jobs::JobManager,
    db_path: PathBuf,
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
    Ok(json!({"book": db::book(&conn)?, "progress": app.job.summary()}))
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

#[derive(Deserialize)]
struct ModelDownloadInput { model_id:String, #[serde(default="default_backend")] backend:String }
fn default_backend()->String{"ct2".into()}
async fn models() -> ApiResult {
    Ok(Json(json!({"backend":"ct2","models":translation::MODELS.iter().map(|m|json!({
        "id":m.key,"label":m.key,"model_id":m.model_repo,"backend":"ct2",
        "max_input_tokens":m.max_input,"max_output_tokens":m.max_output,
        "default_beam":m.default_beam,"device_support":["cpu","cuda"]
    })).collect::<Vec<_>>()})))
}
async fn hardware(State(app):State<App>)->ApiResult{
    let mut system=sysinfo::System::new_all();system.refresh_cpu();system.refresh_memory();
    let cpu=system.cpus().first().map(|c|c.brand().trim().to_string()).filter(|s|!s.is_empty()).unwrap_or_else(||std::env::consts::ARCH.into());
    let cores=system.cpus().len().max(1);
    let paths=vec![("Dữ liệu".to_string(),config::data_dir()),("Văn bản".to_string(),config::output_text_dir()),("Audio".to_string(),config::output_audio_dir())];
    let resources=monitoring::snapshot(&paths);
    let gpus=resources["gpus"].as_array().cloned().unwrap_or_default();
    let gpu=gpus.first();
    let has_cuda=gpu.is_some();
    Ok(Json(json!({"cpu_name":cpu,"cpu_count":cores,"intra_threads":cores.saturating_sub(1).max(1).min(16),
        "is_gpu":has_cuda,"has_discrete_gpu":has_cuda,"device":gpu.and_then(|g|g["name"].as_str()).unwrap_or("cpu"),
        "gpu_name":gpu.and_then(|g|g["name"].as_str()),"vram_gb":gpu.and_then(|g|g["total_bytes"].as_f64()).map(|v|v/1073741824.0).unwrap_or(0.0),
        "batch_size":8,"summary":format!("CPU {cores} luồng"),"resources":resources})))
}
async fn download_model(State(app):State<App>,Json(payload):Json<ModelDownloadInput>)->ApiResult{
    if payload.backend!="ct2"{return Err(error(StatusCode::NOT_IMPLEMENTED,"Native hiện chỉ hỗ trợ model CTranslate2; PyTorch/Transformers chưa được chuyển."));}
    let spec=translation::model(&payload.model_id).map_err(|e|error(StatusCode::NOT_FOUND,e.to_string()))?;
    let root=std::env::var_os("NOVEL_MODELS").map(PathBuf::from).unwrap_or_else(||config::data_dir().join("models"));
    let token=config::load_secret(&config::data_dir().join("hf_token"));
    let manager=model_manager::ModelManager::new(root,token).map_err(storage_error)?;
    let path=manager.download_ct2_from(spec.ct2_repo,spec.model_repo,spec.subdir).await
        .map_err(|e|error(StatusCode::BAD_GATEWAY,e.to_string()))?;
    Ok(Json(json!({"status":"success","model_id":spec.key,"backend":"ct2","path":path.join(spec.subdir)})))
}
async fn start_translation(State(app):State<App>,Json(payload):Json<Value>)->ApiResult{
    let mut config=AppConfig::from_value(payload.clone()).map_err(|e|error(StatusCode::BAD_REQUEST,e.to_string()))?;
    if config.backend!="ct2"{return Err(error(StatusCode::NOT_IMPLEMENTED,"Bản native chỉ chạy CTranslate2. Backend PyTorch/Transformers cần được chuyển model sang CT2 hoặc OpenVINO."));}
    let spec=translation::model(&config.engine).map_err(|e|error(StatusCode::NOT_FOUND,e.to_string()))?;
    let reset=payload["reset"].as_bool().unwrap_or(false);
    let worker=std::env::var_os("NOVEL_CT2_WORKER").map(PathBuf::from).unwrap_or_else(||{
        let mut p=PathBuf::from("native/cpp/build/novelreader-ct2-worker");
        if cfg!(windows){p.set_extension("exe");}p
    });
    if !worker.is_file(){return Err(error(StatusCode::SERVICE_UNAVAILABLE,format!("Chưa tìm thấy C++ worker {}. Cần build CTranslate2 + SentencePiece native.",worker.display())));}
    let mut conn=app.database.lock().map_err(|_|error(StatusCode::INTERNAL_SERVER_ERROR,"SQLite lock"))?;
    let book=db::book(&conn).map_err(storage_error)?.ok_or_else(||error(StatusCode::BAD_REQUEST,"Hãy nạp truyện trước."))?;
    let all=db::all_chapters(&conn).map_err(storage_error)?;
    let selected:Vec<i64>=all.iter().filter(|c|{
        let id=c["id"].as_i64().unwrap_or(0);let pos=c["position"].as_i64().unwrap_or(0);
        let in_ids=config.chapter_ids.as_ref().map(|v|v.contains(&id)).unwrap_or(true);
        let in_range=config.start_pos.map(|v|pos>=v as i64).unwrap_or(true)&&config.end_pos.map(|v|pos<=v as i64).unwrap_or(true);
        let pending=config.force_retranslate||reset||c["status"]!="done";
        in_ids&&in_range&&pending
    }).filter_map(|c|c["id"].as_i64()).collect();
    if selected.is_empty(){return Err(error(StatusCode::BAD_REQUEST,"Không có chương phù hợp để dịch."));}
    if reset{store::reset_chapters(&mut conn,&selected).map_err(storage_error)?;}
    db::save_config(&conn,&payload).map_err(storage_error)?;
    let scope=format!("{} chương · {}",selected.len(),book["title"].as_str().unwrap_or("truyện"));
    app.job.begin(selected.len(),scope).map_err(|e|error(StatusCode::CONFLICT,e.to_string()))?;
    let job=app.job.clone();let db_path=app.db_path.clone();let ids=selected;
    let model_root=std::env::var_os("NOVEL_MODELS").map(PathBuf::from).unwrap_or_else(||config::data_dir().join("models"));
    let token=config::load_secret(&config::data_dir().join("hf_token"));
    let model_repo=spec.ct2_repo.to_string();let tokenizer_repo=spec.model_repo.to_string();let subdir=spec.subdir.to_string();
    let device=std::env::var("NOVEL_CT2_DEVICE").unwrap_or_else(|_|"cpu".into());
    let compute=std::env::var("NOVEL_CT2_COMPUTE_TYPE").unwrap_or_else(|_|"int8_float32".into());
    let threads=std::env::var("NOVEL_CT2_THREADS").ok().and_then(|v|v.parse().ok()).unwrap_or(4usize).clamp(1,64);
    let spec=spec;
    tokio::spawn(async move{
        job.update("downloading","Đang kiểm tra/tải model CTranslate2…",0,ids.len());
        let result:Result<(),anyhow::Error>=async{
            let manager=model_manager::ModelManager::new(model_root,token)?;
            let root=manager.download_ct2_from(&model_repo,&tokenizer_repo,&subdir).await?;
            let model_dir=root.join(&subdir);let source_spm=root.join("source.spm");let target_spm=root.join("target.spm");
            let config_for_job=config.clone();let job_for_worker=job.clone();
            tokio::task::spawn_blocking(move||jobs::run_translation(&db_path,&worker,&model_dir,&source_spm,&target_spm,&device,&compute,threads,spec,config_for_job,ids,job_for_worker)).await??;
            Ok(())
        }.await;
        if let Err(err)=result{job.finish("error",&err.to_string());}
    });
    Ok(Json(app.job.summary()))
}
async fn control_translation(State(app):State<App>,Path(action):Path<String>)->ApiResult{
    let result=match action.as_str(){"pause"=>app.job.pause(),"resume"=>app.job.resume(),"stop"=>app.job.stop(),_=>return Err(error(StatusCode::NOT_FOUND,"Lệnh không hợp lệ."))};
    result.map_err(|e|error(StatusCode::CONFLICT,e.to_string()))?;
    Ok(Json(app.job.summary()))
}
async fn save_hf_token(State(_app):State<App>,Json(payload):Json<Value>)->ApiResult{
    let token=payload["token"].as_str().unwrap_or("");
    config::save_secret(&config::data_dir().join("hf_token"),token).map_err(|e|error(StatusCode::BAD_REQUEST,e.to_string()))?;
    Ok(Json(json!({"ok":true,"configured":true})))
}
async fn storage_status(State(_app):State<App>)->ApiResult{
    let configured=config::load_secret(&config::data_dir().join("hf_token")).is_some();
    Ok(Json(json!({"configured":configured,"provider":"huggingface","native":true})))
}
async fn not_ported() -> ApiError {
    error(StatusCode::NOT_IMPLEMENTED,
        "Chức năng này chưa được chuyển sang Rust/C++. Hãy dùng Python main cho đến khi engine native hoàn thiện.")
}

async fn monitor(State(app): State<App>) -> ApiResult {
    let conn = app.database.lock().map_err(|_|error(StatusCode::INTERNAL_SERVER_ERROR, "SQLite lock"))?;
    let job = app.job.summary();
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
    let db_path = data_root.join("novel.sqlite3");
    let db = db::open(&db_path)?;
    let app = App {
        database: Arc::new(Mutex::new(db)),
        tomato: Arc::new(Mutex::new(tomato::TomatoStatus::default())),
        cancel: Arc::new(AtomicBool::new(false)),
        client: reqwest::Client::builder().timeout(Duration::from_secs(35)).build()?,
        tomato_url: tomato_url.trim_end_matches('/').to_string(),
        tomato_password: std::env::var("TOMATO_WEB_PASSWORD").unwrap_or_default(),
        job: jobs::JobManager::new(),
        db_path,
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
        .route("/api/translate/start", post(start_translation))
        .route("/api/translate/{action}", post(control_translation))
        .route("/api/models", get(models))
        .route("/api/models/download", post(download_model))
        .route("/api/hardware", get(hardware))
        .route("/api/storage/status", get(storage_status))
        .route("/api/storage/token", post(save_hf_token))
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
