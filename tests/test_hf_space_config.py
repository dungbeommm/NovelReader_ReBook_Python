from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]


def test_hf_cpu_requirements_do_not_install_cuda_runtime():
    requirements = (ROOT / "requirements-hf-cpu.txt").read_text(encoding="utf-8").lower()
    assert "onnxruntime>=" in requirements
    assert "onnxruntime-gpu" not in requirements
    assert "nvidia-cudnn" not in requirements
    assert "nvidia-cublas" not in requirements


def test_hf_dockerfile_embeds_tomato_and_uses_spaces_user():
    dockerfile = (ROOT / "Dockerfile").read_text(encoding="utf-8")
    assert "tomato-novel-downloader-webui:latest" in dockerfile
    assert "COPY --from=tomato-runtime /app/tomato-novel-downloader" in dockerfile
    assert "useradd --create-home --uid 1000 user" in dockerfile
    assert "PIPER_DEVICE=cpu" in dockerfile
    assert "TOMATO_WEB_ADDR=127.0.0.1:18423" in dockerfile


def test_hf_space_metadata_declares_docker_port():
    readme = (ROOT / "README.md").read_text(encoding="utf-8")
    assert readme.startswith("---\n")
    assert "sdk: docker" in readme.split("---", 2)[1]
    assert "app_port: 8000" in readme.split("---", 2)[1]
