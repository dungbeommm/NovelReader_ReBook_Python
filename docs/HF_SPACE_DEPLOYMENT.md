# Triển khai NovelReader lên Hugging Face Spaces (CPU + Tomato)

Nhánh `hf-space` đóng gói FastAPI/WebUI và Tomato Downloader trong **một Docker Space**. Chỉ FastAPI được mở ra ngoài; Tomato lắng nghe ở loopback `127.0.0.1:18423`.

## 1. Thành phần của profile CPU

- FastAPI + WebUI: cổng `8000`, một Uvicorn worker.
- Dịch: CTranslate2 CPU, tải model INT8 khi cần; vẫn có thể chọn engine Hachimi online qua HF Space API.
- TTS: Piper + ONNX Runtime CPU.
- Tomato: binary Rust lấy từ image phát hành công khai `zhongbai233/tomato-novel-downloader-webui`, chạy cùng container.
- VieNeu/PyTorch CUDA không được cài trong profile CPU này. Chọn Piper khi tạo audio.
- `Dockerfile.local` giữ cấu hình Docker cũ cho `compose.yaml`; Dockerfile ở thư mục gốc dành cho Hugging Face.

## 2. Tạo Space

1. Tạo một Space mới, chọn SDK **Docker**.
2. Chọn phần cứng CPU phù hợp (CPU Basic là cấu hình khởi đầu tiết kiệm).
3. Đưa nội dung nhánh `hf-space` vào repository của Space. Metadata ở đầu README đã khai báo `sdk: docker` và `app_port: 8000`.
4. Đặt Space ở chế độ Private nếu chỉ mình bạn sử dụng. Dù Space công khai, ứng dụng vẫn yêu cầu access token riêng.

## 3. Variables và Secrets bắt buộc

Trong **Space Settings → Variables and secrets**, cấu hình:

| Tên | Loại | Giá trị |
|---|---|---|
| `NOVEL_WEB_ACCESS_TOKEN` | Secret | Mật khẩu ngẫu nhiên tối thiểu 24 ký tự (khuyến nghị 32+). |
| `HF_BUCKET_ID` | Variable | ID Bucket do bạn sở hữu, dạng `username/novelreader-storage`. |
| `HF_TOKEN` | Secret | Token Hugging Face có quyền ghi vào Bucket `HF_BUCKET_ID`. |

`HF_BUCKET_ID` là bắt buộc trong Space. Ứng dụng cố ý không dùng giá trị mặc định `uri410/novelreader-storage` ở môi trường triển khai để tránh ghi dữ liệu vào Bucket của người khác.

Có thể đặt thêm:

- `TOMATO_WEB_PASSWORD` (Secret): mật khẩu cho API Tomato nội bộ. Không bắt buộc vì Tomato chỉ bind loopback.
- `TOMATO_IMAGE` là Docker build argument, mặc định dùng tag `latest` của image Tomato công khai; có thể pin tag/digest đã kiểm thử nếu muốn build tái lập.

Không đưa token hoặc mật khẩu vào Git, README hay Dockerfile.

## 4. Lưu trữ bền vững

Ổ đĩa mặc định của Space là tạm thời; dữ liệu có thể mất khi Space restart/stop. Để giữ SQLite, model, voice, output và cấu hình Tomato:

1. Tạo một **private Hugging Face Storage Bucket** dùng cho dữ liệu runtime.
2. Gắn Bucket đó vào Space tại mount path `/data`, quyền read-write.
3. Có thể dùng một Bucket riêng khác làm đích xuất/checkpoint và đặt ID của Bucket đó vào `HF_BUCKET_ID`. Nếu dùng chung một Bucket cho cả mount `/data` và export API, thư mục runtime cũng sẽ xuất hiện khi duyệt Bucket trong WebUI.
4. Đảm bảo `HF_TOKEN` có quyền ghi vào Bucket xuất/checkpoint.

Cache model và voice được đặt dưới `/data` để không phải tải lại sau mỗi lần khởi động. Token HF nhập qua giao diện, nếu không cấp bằng Secret `HF_TOKEN`, chỉ được lưu tạm trong `/tmp` và sẽ mất khi container khởi động lại.

## 5. Tomato chạy cùng container

Entrypoint khởi động Tomato trước, chờ `GET /api/status` sẵn sàng rồi mới khởi động FastAPI. Cấu hình mặc định:

- Tomato: `127.0.0.1:18423`
- NovelReader gọi: `http://127.0.0.1:18423`
- Cổng công khai của Space: `8000`

Không cần triển khai thêm Space hoặc dịch vụ bên ngoài cho Tomato. Không mở cổng `18423` ra Internet. Nếu Tomato không khởi động được, entrypoint dừng thay vì để WebUI chạy với tích hợp tải truyện hỏng.

## 6. Kiểm tra sau build

GitHub Actions của nhánh `hf-space` chạy:

1. Cài bộ dependencies CPU.
2. Compile-check các module Python và chạy toàn bộ pytest.
3. Kiểm tra cú pháp Bash entrypoint.
4. Build Docker image, bao gồm bước lấy binary Tomato.
5. Chạy container thử, kiểm tra WebUI, API phần cứng CPU, cấu hình Bucket, API Tomato nội bộ và xác nhận ONNX Runtime/CTranslate2 không dùng CUDA.

Sau khi Space chạy, mở WebUI và kiểm tra:

- Phần cứng hiển thị CPU, không có GPU.
- Mục tải truyện bằng ID kết nối được Tomato.
- Dịch thử một chương ngắn với model HachimiMT.
- Tạo audio thử bằng Piper.
- Xuất checkpoint hoặc TXT lên Bucket.

## 7. Lưu ý vận hành

- CPU Basic phù hợp kiểm thử và tác vụ vừa phải; dịch hàng nghìn chương hoặc tạo audio dài có thể chậm. Không coi CPU miễn phí là máy chủ luôn chạy 24/7.
- Nếu không gắn Bucket read-write tại `/data`, dữ liệu runtime là ephemeral.
- VieNeu/PyTorch không nằm trong image CPU để tránh kéo theo bộ phụ thuộc nặng; profile này dùng Piper.
- Build phụ thuộc image Tomato công khai. Nếu upstream thay đổi hoặc xóa tag, build sẽ lỗi rõ ràng; có thể pin image đã xác minh qua `TOMATO_IMAGE`.
- Không tự động tạo Space hoặc deploy từ commit này. Nhánh chỉ được chuẩn bị và kiểm tra qua CI; bước tạo Space/đưa code lên Space vẫn là thao tác riêng.
