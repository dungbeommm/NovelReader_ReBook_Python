import pytest

from storage import cloud_settings
from storage import huggingface_service
from translation.engines import create_engine


def test_bucket_setting_shared_between_process_calls(monkeypatch, tmp_path):
    monkeypatch.setattr(cloud_settings, "data_dir", lambda: tmp_path)
    monkeypatch.delenv("HF_BUCKET_ID", raising=False)
    cloud_settings.save_repo_id("example/novel-storage")
    assert cloud_settings.load_repo_id() == "example/novel-storage"
    assert huggingface_service.repo_id() == "example/novel-storage"
    assert huggingface_service.base_url().endswith("/example/novel-storage")


def test_invalid_bucket_rejected():
    with pytest.raises(ValueError):
        cloud_settings.save_repo_id("../invalid")


def test_unknown_model_is_not_silently_substituted():
    with pytest.raises(ValueError, match="không được hỗ trợ"):
        create_engine("HachimiMT-125-QT")



def test_hf_space_requires_an_explicit_bucket(monkeypatch, tmp_path):
    monkeypatch.setattr(cloud_settings, "data_dir", lambda: tmp_path)
    monkeypatch.setenv("HF_SPACE_DEPLOYMENT", "1")
    monkeypatch.delenv("HF_BUCKET_ID", raising=False)
    with pytest.raises(ValueError, match="HF_BUCKET_ID"):
        cloud_settings.load_repo_id()


def test_hf_space_bucket_environment_overrides_saved_value(monkeypatch, tmp_path):
    monkeypatch.setattr(cloud_settings, "data_dir", lambda: tmp_path)
    monkeypatch.setenv("HF_SPACE_DEPLOYMENT", "1")
    cloud_settings.save_repo_id("old-owner/old-bucket")
    monkeypatch.setenv("HF_BUCKET_ID", "my-owner/my-novel-bucket")
    assert cloud_settings.load_repo_id() == "my-owner/my-novel-bucket"


def test_hf_space_token_file_is_outside_persistent_data(monkeypatch, tmp_path):
    monkeypatch.setenv("HF_SPACE_DEPLOYMENT", "1")
    monkeypatch.setenv("NOVEL_SECRET_DIR", str(tmp_path / "ephemeral-secrets"))
    assert cloud_settings.key_path() == tmp_path / "ephemeral-secrets" / "huggingface.json"
