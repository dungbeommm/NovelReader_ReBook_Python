# NovelReader ReBook Python 📚

[![Python Version](https://img.shields.io/badge/python-3.10%20%7C%203.11%20%7C%203.12-blue.svg)](https://www.python.org/)
[![Framework](https://img.shields.io/badge/framework-FastAPI-009688.svg)](https://fastapi.tiangolo.com/)
[![Inference](https://img.shields.io/badge/inference-CTranslate2%20%7C%20PyTorch-orange.svg)](https://github.com/OpenNMT/CTranslate2)
[![TTS Engine](https://img.shields.io/badge/TTS-Piper%20%7C%20VieNeu-purple.svg)](https://github.com/rhasspy/piper)
[![Database](https://img.shields.io/badge/database-SQLite%20(WAL)-lightgrey.svg)](https://www.sqlite.org/)
[![License](https://img.shields.io/badge/license-MIT-green.svg)](LICENSE)

**NovelReader ReBook Python** là hệ thống xử lý, dịch thuật và xuất bản tiểu thuyết thông minh được tối ưu hóa chuyên sâu cho cặp ngôn ngữ **Trung - Việt**. Nền tảng kết hợp mô hình dịch thần kinh **HachimiMT**, bộ lọc ngữ pháp đại từ/thuật ngữ văn phong tiếng Việt, công nghệ sinh giọng đọc tự nhiên (TTS đa vùng miền) và tích hợp tự động hóa tải sách trực tiếp từ các nền tảng đọc truyện trực tuyến.

---

## 📑 Mục lục

- [Tổng quan kiến trúc](#-tổng-quan-kiến-trúc)
- [Tính năng nổi bật](#-tính-năng-nổi-bật)
- [Cấu trúc thư mục](#-cấu-trúc-thư-mục)
- [Yêu cầu hệ thống](#-yêu-cầu-hệ-thống)
- [Cài đặt & Khởi chạy nhanh](#-cài-đặt--khởi-chạy-nhanh)
  - [Cách 1: Chạy trực tiếp (Local Python)](#cách-1-chạy-trực-tiếp-local-python)
  - [Cách 2: Chạy qua Docker Compose](#cách-2-chạy-qua-docker-compose)
- [Tải Model & Giọng đọc](#-tải-model--giọng-đọc)
- [Tích hợp Tải truyện Tomato (Fanqie)](#-tích-hợp-tải-truyện-tomato-fanqie)
- [Cấu hình biến môi trường (.env)](#-cấu-hình-biến-môi-trường-env)
- [Kiểm thử tự động](#-kiểm-thử-tự-động)
- [Giấy phép](#-giấy-phép)

---

## 🏛 Tổng quan kiến trúc

Hệ thống được thiết kế theo hướng module hóa chặt chẽ, tách bạch giữa giao diện điều khiển (WebUI), lõi điều phối tác vụ (Core Runtime & Job Queue), hệ thống dịch máy (Translation Pipeline), động cơ âm thanh (Audio/TTS Engine) và lưu trữ bền vững (Storage):

```
+-----------------------------------------------------------------------+
|                         WebUI (FastAPI + Vanilla JS)                  |
|          Trình đọc truyện • Giám sát tiến độ • Cấu hình giọng/model   |
+-----------------------------------+-----------------------------------+
                                    | REST API / Server-Sent Events
+-----------------------------------v-----------------------------------+
|                        Core Runtime & Job Engine                      |
|       JobLease Lock • Quản lý tài nguyên CPU/VRAM • Hàng đợi song song|
+-------------------+-------------------------------+-------------------+
                    |                               |
+-------------------v---------------+   +-----------v-------------------+
|      Translation Pipeline         |   |        Audio & TTS Engine     |
| • Mô hình: HachimiMT-40/60-QT     |   | • Động cơ: Piper / VieNeu     |
| • Backend: CTranslate2 / PyTorch  |   | • Hàng đợi tổng hợp giọng     |
| • Hậu xử lý đại từ (Harmonizer v9)|   | • Xử lý & ghép nối MP3/WAV    |
| • Thuật ngữ (Glossary/Honorifics) |   | • Quản lý danh mục voice ONNX |
+-------------------+---------------+   +-----------+-------------------+
                    |                               |
+-------------------v-------------------------------v-------------------+
|                           Storage Layer                               |
|   • SQLite (WAL Mode, Book & Chapter Schema)                          |
|   • Checkpoint Snapshot & Quản lý sao lưu đám mây Hugging Face        |
|   • File xuất bản: output_text/ (TXT) & output_audio/ (MP3)           |
+-----------------------------------------------------------------------+
```

---

## 🌟 Tính năng nổi bật

### 1. Dịch thuật chuyên sâu cho văn học mạng
- **Mô hình HachimiMT**: Hỗ trợ dòng model dịch chất lượng cao `HachimiMT-40-QT` và `HachimiMT-60-QT`.
- **Tối ưu hóa phần cứng vượt trội**: Chạy qua `CTranslate2` tăng tốc tối đa trên CPU và GPU (CUDA int8/fp16/float32), hỗ trợ Intel Arc (XPU) và Apple Silicon/ARM.
- **Bộ hậu xử lý văn phong tiếng Việt**:
  - `Pronoun Harmonizer v9`: Chuẩn hóa xưng hô theo ngữ cảnh (hắn, nàng, ta, ngươi, huynh, muội,...).
  - `Glossary & Honorifics`: Nhận diện và quy đổi danh xưng, thuật ngữ tu tiên, kiếm hiệp, hiện đại linh hoạt.
  - Tự động gộp câu, chia đoạn hợp lý, bảo toàn cấu trúc tác phẩm.

### 2. Sinh giọng đọc thông minh (Text-to-Speech)
- **Hỗ trợ đa động cơ**:
  - **Piper TTS**: Siêu nhẹ, độ trễ thấp, phát âm tiếng Việt tự nhiên, phù hợp đọc sách số lượng lớn.
  - **VieNeu TTS**: Chất lượng âm thanh cao cấp, biểu cảm phong phú.
- **Hàng mục Voice phong phú**: Nhiều giọng đọc nam/nữ Bắc - Trung - Nam được đóng gói chuẩn ONNX.
- **Ghép nối âm thanh tự động**: Phân tách câu không vượt quá giới hạn âm vị, xử lý song song và xuất tệp MP3 theo từng chương hoàn chỉnh.

### 3. Tải truyện tự động từ 番茄小说 (Fanqie Novel)
- Tích hợp trực tiếp dịch vụ tải Tomato Downloader: Chỉ cần nhập Book ID hoặc URL truyện, hệ thống sẽ tự động kéo toàn bộ danh mục chương, làm sạch và nạp trực tiếp vào pipeline dịch thuật.
- Hỗ trợ các định dạng tệp đầu vào thông dụng: `.txt`, `.epub`, `.zip`.
- Khả năng tự động dò tìm bảng mã ký tự (`utf-8`, `gbk`, `gb2312`, `big5`,...).

### 4. Vận hành tin cậy & Lưu trữ bền vững
- **Cơ sở dữ liệu SQLite WAL**: Đảm bảo an toàn dữ liệu, chống phân mảnh khi ghi log và cập nhật tiến trình liên tục.
- **Checkpoint & Cloud Sync**: Lưu trữ snapshot thư viện truyện và hỗ trợ đồng bộ lên Hugging Face Dataset Bucket.
- **Job Lease Guard**: Kiểm soát độc quyền tác vụ, ngăn chặn xung đột tiến trình nền khi thực hiện dịch khối lượng lớn.

---

## 📁 Cấu trúc thư mục

```text
NovelReader/
├── audio/                   # Dịch vụ xử lý âm thanh, ghép nối MP3, VieNeu audio
├── core/                    # Quản lý vòng đời ứng dụng, runtime, job queue, tài nguyên
├── data/                    # Cơ sở dữ liệu SQLite (novel.sqlite3), lock, logs (tạo khi chạy)
├── docs/                    # Tài liệu chuyên sâu (Tích hợp Tomato, hướng dẫn)
├── exports/                 # Thư mục xuất bản file tổng hợp
├── models/                  # Nơi chứa model dịch (HachimiMT) và giọng đọc (Piper/VieNeu)
│   ├── translation/         # HachimiMT-40-QT, HachimiMT-60-QT
│   └── voices/              # Các tệp giọng .onnx và .onnx.json
├── monitoring/              # Giám sát tài nguyên máy chủ, logging
├── output_audio/            # Thư mục chứa audio đầu ra theo từng truyện/chương
├── output_text/             # Thư mục chứa bản dịch văn bản theo từng truyện/chương
├── scripts/                 # Kịch bản tải model, kiểm tra GPU, cài đặt phụ thuộc
├── server/                  # FastAPI app, API routers, cơ chế xác thực và tunnel
├── storage/                 # Quản trị SQLite, file parser, Hugging Face sync, checkpoint
├── tests/                   # Bộ kiểm thử tự động (Unit & Integration tests)
├── translation/             # Động cơ HachimiMT, bộ chuẩn hóa đại từ, từ điển thuật ngữ
├── tts/                     # Động cơ tổng hợp giọng đọc Piper, VieNeu, catalog voice
├── web/                     # Giao diện WebUI (HTML, CSS, JavaScript, Routes)
├── app.py                   # Điểm khởi chạy Space / Gradio
├── compose.yaml             # Cấu hình Docker Compose chính
├── compose.tomato.yaml      # Cấu hình Docker Compose tích hợp Tomato Downloader
├── Dockerfile               # Dockerfile cho ứng dụng FastAPI chính
├── main.py                  # Điểm khởi chạy chính của FastAPI WebUI
├── paths.py                 # Quản lý đường dẫn tập trung của dự án
└── requirements.txt         # Danh sách phụ thuộc thư viện Python
```

---

## 💻 Yêu cầu hệ thống

- **Hệ điều hành**: Windows 10/11, Linux (Ubuntu 20.04+ khuyến nghị), macOS.
- **Python**: Phiên bản `3.10`, `3.11` hoặc `3.12`.
- **Bộ nhớ RAM**: Tối thiểu 4 GB (Khuyến nghị từ 8 GB trở lên nếu vừa dịch vừa tạo Audio).
- **Phần cứng suy luận (Tùy chọn)**:
  - GPU NVIDIA hỗ trợ CUDA 11.8+ / 12+.
  - GPU Intel Arc (yêu cầu bộ mở rộng Intel PyTorch/OpenVINO).
  - Chạy thuần CPU đa nhân (đã được tối ưu tốt qua CTranslate2).

---

## 🚀 Cài đặt & Khởi chạy nhanh

### Cách 1: Chạy trực tiếp (Local Python)

1. **Clone repository về máy**:
   ```bash
   git clone https://github.com/dungbeommm/NovelReader_Translate.git NovelReader
   cd NovelReader
   ```

2. **Khởi tạo và kích hoạt môi trường ảo (Khuyến nghị)**:
   ```bash
   # Windows
   python -m venv .venv
   .venv\Scripts\activate

   # Linux / macOS
   python3 -m venv .venv
   source .venv/bin/activate
   ```

3. **Cài đặt các gói thư viện phụ thuộc**:
   ```bash
   pip install --upgrade pip
   pip install -r requirements.txt
   ```

4. **Tải model dịch và giọng đọc**:
   ```bash
   # Tải model dịch mặc định HachimiMT-60-QT
   python scripts/download_model.py --model HachimiMT-60-QT

   # Tải danh mục giọng đọc Piper
   python scripts/download_voices.py --catalog-only
   ```

5. **Khởi chạy máy chủ WebUI**:
   ```bash
   python -m uvicorn main:app --host 127.0.0.1 --port 8000 --workers 1
   ```

6. Mở trình duyệt và truy cập: **`http://127.0.0.1:8000`**

---

### Cách 2: Chạy qua Docker Compose

Sử dụng Docker để đóng gói toàn bộ môi trường và khởi chạy chỉ bằng một lệnh:

1. **Khởi chạy bản tiêu chuẩn**:
   ```bash
   docker compose up -d --build
   ```

2. **Khởi chạy kèm dịch vụ tải truyện Tomato Downloader**:
   ```bash
   # Tạo file .env và đặt mật khẩu cho Tomato Downloader
   echo "TOMATO_WEB_PASSWORD=YourSecurePassword123" > .env

   # Khởi chạy cả 2 dịch vụ
   docker compose -f compose.yaml -f compose.tomato.yaml up -d --build
   ```

---

## 📦 Tải Model & Giọng đọc

Dự án cung cấp sẵn các CLI script trong thư mục `scripts/` giúp người dùng dễ dàng chuẩn bị dữ liệu:

- **Tải model dịch HachimiMT**:
  ```bash
  # Tải bản 60-QT (Mặc định, cân bằng tốc độ và chất lượng)
  python scripts/download_model.py --model HachimiMT-60-QT

  # Tải bản 40-QT (Nhẹ hơn, siêu tốc)
  python scripts/download_model.py --model HachimiMT-40-QT

  # Kiểm tra thử dịch ngay sau khi tải
  python scripts/download_model.py --model HachimiMT-60-QT --smoke
  ```

- **Tải giọng đọc TTS**:
  ```bash
  # Tải toàn bộ cấu hình voice
  python scripts/download_voices.py --catalog-only

  # Tải toàn bộ file mô hình giọng đọc ONNX
  python scripts/download_voices.py --all
  ```

- **Cài đặt động cơ VieNeu TTS**:
  ```bash
  python scripts/setup_tts.py --engine vieneu --install
  ```

---

## 🍅 Tích hợp Tải truyện Tomato (Fanqie)

Dự án tích hợp sâu với **Tomato-Novel-Downloader** cho phép lấy nội dung tiểu thuyết Trung Quốc trực tiếp:

1. Trong giao diện WebUI, truy cập phần **Truyện nguồn** -> chọn tab **Tải truyện bằng ID**.
2. Dán link truyện hoặc mã ID số của bộ truyện từ Fanqie.
3. Bấm **Bắt đầu tải**: Hệ thống sẽ tự động đồng bộ danh sách chương về cơ sở dữ liệu và sẵn sàng cho việc dịch thuật.

> Chi tiết cấu hình tham số máy chủ xem tại: **[docs/TOMATO_INTEGRATION.md](docs/TOMATO_INTEGRATION.md)**.

---

## ⚙️ Cấu hình biến môi trường (.env)

Tạo tệp `.env` tại thư mục gốc của dự án để tùy biến hành vi của ứng dụng:

```ini
# Thiết bị chạy Piper TTS: auto | cpu | cuda | openvino
PIPER_DEVICE=auto
PIPER_REQUIRE_CUDA=0

# Batch size cho tác vụ dịch HachimiMT
HACHIMIMT_WEB_BATCH_SIZE=8

# Thiết bị PyTorch cho dịch thuật (mặc định: auto)
# HACHIMIMT_TORCH_DEVICE=cuda

# Token Hugging Face (Dùng khi kết nối và đồng bộ lên Bucket/Dataset riêng)
HF_TOKEN=

# Mật khẩu bảo vệ WebUI (nếu muốn khóa truy cập ngoài)
# NOVEL_WEB_ACCESS_TOKEN=your-private-password

# Cấu hình kết nối Tomato Novel Downloader
# TOMATO_WEB_URL=http://127.0.0.1:18423
# TOMATO_WEB_PASSWORD=your_tomato_password
```

---

## 🧪 Kiểm thử tự động

Bộ kiểm thử được viết bằng `pytest` bao quát toàn bộ logic cốt lõi:

```bash
# Chạy kiểm thử kiến trúc và API
python -m pytest tests/test_architecture_refactor.py tests/test_api.py tests/test_runtime_config.py -v

# Chạy toàn bộ test suite
python -m pytest -q
```

---

## 📄 Giấy phép

Dự án được phân phối dưới giấy phép **MIT License**. Xem chi tiết tại [LICENSE](LICENSE).

---

<p align="center">
  Được phát triển với niềm đam mê dành cho cộng đồng đọc và dịch tiểu thuyết 📖✨
</p>
