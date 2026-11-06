"""Stable entry point: python -m uvicorn main:app --host 127.0.0.1 --port 8000."""
from core import runtime  # Backward-compatible runtime handle for tests and integrations.
from server.app import create_app

app = create_app()
