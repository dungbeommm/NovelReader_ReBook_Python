# Native migration status

Branch: migration/rust-cpp-phase1
Source of truth: existing Python modules in the repository.

| Area | Original | Native | Status |
| --- | --- | --- | --- |
| HTTP frontend | server/app.py, web/routes | Rust Axum | partial |
| SQLite | storage/store.py | Rust rusqlite | partial (schema, import, read) |
| Chinese TXT segmentation | translation/text.py | Rust text.rs, C++ ICU | partial |
| Tomato client | core/tomato_service.py | Rust reqwest | ported with TXT-only limitation |
| WebSocket | web/routes/progress.py | Rust Axum ws | snapshots only |
| HTTP book API | web/routes/books.py | Rust endpoints | partial |
| HachimiMT | translation/core/translator.py | CTranslate2 C++ | pending |
| SentencePiece | translation/core/token_chunker.py | SentencePiece C++ | pending |
| OpenCC | translation/core/text_preprocess.py | OpenCC C++ | pending |
| Pronoun harmonizer | translation/core/pronoun_harmonizer_v9.py | ICU/PCRE2 C++ | pending |
| Piper | audio/audio_service.py | Piper/ORT C++ | pending |
| VieNeu | audio/vieneu_service.py | ONNX/LibTorch C++ | pending |
| Intel XPU | PyTorch XPU, OpenVINO | OpenVINO/LibTorch native | pending |
| Audio files | audio/merge_service.py | FFmpeg native | pending |
| EPUB/ZIP | storage/files.py | Rust zip/quick-xml | pending |
| HF Bucket | storage/huggingface_service.py | Rust API/S3 | pending |

## Acceptance gates

1. No silent fallback to a different model or hardware backend.
2. Exact JSON contracts for all old frontend endpoints.
3. Chunks, chapter counts, and line layout must match Python golden test inputs.
4. Keep SQL schema and output folders backward compatible.
5. Tests on Windows and Linux, CPU + NVIDIA CUDA + Intel XPU when hardware available.
6. Python application continues to work until the migration is fully verified.

This branch does not claim the full app is already ported.
