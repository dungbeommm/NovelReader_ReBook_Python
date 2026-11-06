"""Smoke-test the integrated Hugging Face CPU container.

Run inside the container after both processes have started:
    python hf-space/smoke_test.py
"""
import json
import os
from urllib.request import Request, urlopen


BASE_URL = os.getenv("SMOKE_BASE_URL", "http://127.0.0.1:8000").rstrip("/")
TOKEN = os.environ["NOVEL_WEB_ACCESS_TOKEN"]
BUCKET_ID = os.environ["HF_BUCKET_ID"]
TOMATO_URL = os.getenv("TOMATO_WEB_URL", "http://127.0.0.1:18423").rstrip("/")
PASSWORD = os.getenv("TOMATO_WEB_PASSWORD", "").strip()


def get(url, *, protected=False, tomato=False):
    headers = {}
    if protected:
        headers["X-Novel-Access"] = TOKEN
    if tomato and PASSWORD:
        headers["X-Tomato-Password"] = PASSWORD
    request = Request(url, headers=headers)
    with urlopen(request, timeout=10) as response:
        return response.read().decode("utf-8")


def get_json(url, *, protected=False, tomato=False):
    return json.loads(get(url, protected=protected, tomato=tomato))


def main():
    html = get(BASE_URL + "/", protected=True)
    assert "html" in html.lower() and len(html) > 100, "WebUI root did not return HTML"

    hardware = get_json(BASE_URL + "/api/hardware", protected=True)
    assert hardware.get("is_gpu") is False, f"Expected CPU-only hardware, got: {hardware}"

    storage = get_json(BASE_URL + "/api/storage/status", protected=True)
    assert storage.get("repo_id") == BUCKET_ID, f"HF_BUCKET_ID mismatch: {storage}"

    tomato_ui = get_json(BASE_URL + "/api/tomato/status", protected=True)
    assert tomato_ui.get("active") is False, f"Unexpected Tomato job state: {tomato_ui}"

    tomato_api = get_json(TOMATO_URL + "/api/status", tomato=True)
    assert isinstance(tomato_api, dict), "Tomato internal API did not return a JSON object"

    import onnxruntime as ort
    providers = ort.get_available_providers()
    assert "CUDAExecutionProvider" not in providers, f"CUDA provider present in CPU image: {providers}"
    assert "CPUExecutionProvider" in providers, f"CPU provider missing: {providers}"

    import ctranslate2
    assert ctranslate2.get_cuda_device_count() == 0, "CTranslate2 unexpectedly detects a CUDA device"

    print(json.dumps({
        "webui": "ok",
        "hardware": hardware.get("summary", hardware.get("device")),
        "storage_bucket": storage.get("repo_id"),
        "tomato_web_api": "ok",
        "onnxruntime_providers": providers,
        "ctranslate2_cuda_devices": 0,
    }, ensure_ascii=False, indent=2))


if __name__ == "__main__":
    main()
