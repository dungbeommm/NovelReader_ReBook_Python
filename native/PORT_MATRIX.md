# Python to Rust/C++ port matrix

The Python application remains the behavioral reference until each row passes
parity tests. A module is not considered ported merely because a Rust file or
route exists: it must preserve observable behavior and pass tests.

| Python area | Native destination | Status |
| --- | --- | --- |
| server/app.py, web/application.py, web/routes/* | Rust Axum | Partial: state, upload, chapter read, delete/title, Tomato, export txt/epub |
| server/auth/access.py | Rust auth middleware | Pending: login page/cookie/session parity |
| core/config.py, paths.py | native/config.rs | Partial: paths/config/token storage |
| core/job_manager.py, state_manager.py | native/jobs.rs + OS file lock | Pending |
| core/tomato_service.py | native/tomato.rs | Partial: HTTP polling/import; real downloader tests pending |
| storage/store.py | native/db.rs + store.rs | Partial: schema/read/import; CRUD, output folders, safe purge pending |
| storage/files.py | native/files.rs | Partial: TXT/EPUB parse/export; fixture parity pending |
| storage/checkpoints.py | native/checkpoints.rs | Partial: secure create/validate; restore/draft/API pending |
| storage/cloud_settings.py | native/config.rs | Partial |
| storage/huggingface_service.py | native/huggingface.rs | Pending: Bucket upload/download/list parity |
| storage/sync_manager.py | native/cloud_sync.rs | Pending |
| monitoring/resources.py | native/monitoring.rs | Partial: CPU/RAM/disk/NVIDIA; Intel GPU/VRAM/process telemetry pending |
| monitoring/tasks.py | native/telemetry.rs | Pending |
| translation/core/translator.py | native/ct2.rs + cpp/ct2_worker.cpp | Partial: CT2 SentencePiece worker; model matrix/runtime tests pending |
| translation/core/token_chunker.py, chunker.py, line_restore.py | native/translation_text.rs + C++ ICU | Partial: basic text chunker only |
| translation/core/glossary.py | native/glossary.rs | Pending |
| translation/core/honorific_normalize.py | native/honorific.rs | Pending |
| translation/core/pronoun_harmonizer_v9.py | native/pronoun.rs | Pending; exact rules/fixtures required |
| translation/core/text_preprocess.py, postprocess_policy.py | native/translation_text.rs | Pending |
| translation/core/hardware.py, gpu_setup.py | native/hardware.rs | Pending: CPU/CUDA/XPU detection and explicit selection |
| audio/audio_service.py | native/audio.rs + Piper native | Partial: FFmpeg conversion/merge; synthesis and voice management pending |
| audio/vieneu_service.py | native/vieneu.cpp (ONNX/OpenVINO) | Pending: model graph/export and codec compatibility |
| audio/merge_service.py | native/audio.rs | Partial: low-level FFmpeg merge; group/range selection pending |
| workflows/jobs.py, audio_queue.py | native/jobs.rs | Pending |
| scripts/*.py | native setup/CLI utilities | Pending |
| Python tests | Rust unit/integration tests + C++ tests | Partial |

## Backend constraints

- CTranslate2 officially exposes CPU and CUDA devices. It does not expose Intel XPU.
- Intel XPU needs a separately converted OpenVINO model or compatible LibTorch XPU model.
- ONNX Runtime/OpenVINO can run Piper only if the voice graph and execution provider are compatible.
- VieNeu is not interchangeable with Piper; it needs its own native model/codec pipeline.
- Hugging Face Bucket is not equivalent to a simple hf_xet file transfer. Preserve the existing Bucket API and auth semantics when porting.

## Release gate

Do not delete Python source or claim 100% until the native build passes on Windows and Linux,
all routes are parity-tested, each translation/TTS model has real inference fixtures, and
CPU/CUDA/XPU are reported only when the selected backend really runs on that device.
