# NovelReader native (migration phase 1)

This is the start of a Python-free Rust + C++ migration on a separate branch.
It is intentionally NOT a complete replacement for the Python app yet.
The existing main.py and WebUI are preserved.

## Included and operational in code

- Axum local HTTP server on 127.0.0.1:8000 (default)
- Existing WebUI assets served without rewriting HTML/JS/CSS
- Existing SQLite WAL schema read/created, compatible book/chapter views
- /api/state, /api/chapters, /api/chapters/{id}
- TXT upload, Unicode chapter detection and transactional book import
- /api/book/delete and /api/book/title (title update does NOT rename output folders yet)
- Tomato Downloader status/start/cancel and TXT auto-import through its HTTP API
- /ws/progress snapshot messages; native translation worker is not yet connected
- Clear 501 errors on unported translation and synthesis start endpoints

## Build (from repository root)

Install the stable Rust toolchain. Then run:

    cargo test --manifest-path native/Cargo.toml
    cargo run --release --manifest-path native/Cargo.toml

Open http://127.0.0.1:8000/ on the SAME computer.
To change the frontend path: NOVEL_WEB_DIR=/absolute/path/to/web/static.
To change the data path: NOVEL_DATA=/absolute/path/to/data.
To change bind: NOVEL_BIND=127.0.0.1:8000.

To enable Tomato, start the separate Tomato Downloader service, then set:
TOMATO_WEB_URL=http://127.0.0.1:18423
TOMATO_WEB_PASSWORD=your_password

The application reads the same SQLite schema. DO NOT run Rust and Python
servers against one book while a Python translation job is active.
Back up data/novel.sqlite3 before first native migration testing.

## Not implemented yet

- HachimiMT inference native CTranslate2, CPU/CUDA engine selection
- GPU Intel XPU / OpenVINO translation backend
- Piper ONNX and VieNeu TTS runtime
- EPUB import/export, audio merge, checkpoints, HF Bucket integration
- Advanced chapter controls, job pause/resume/stop, hardware metrics
- Full visual/behavioral compatibility of every existing WebUI screen

See PORT_STATUS.md for detailed migration steps and parity rules.

The previous Python service remains the stable functional application while
these native features are implemented and verified.
