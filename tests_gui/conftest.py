"""Shared fixtures and configuration for rb-gui e2e tests."""

import os
import signal
import subprocess
import sys
import time
from pathlib import Path

import pytest
import requests


def _is_server_ready(url: str, timeout: int = 600) -> bool:
    """Poll *url* until it responds with HTTP 200."""
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        try:
            r = requests.get(url, timeout=2)
            if r.status_code == 200:
                return True
        except requests.ConnectionError:
            pass
        time.sleep(0.5)
    return False


@pytest.fixture(scope="session")
def web_server():
    """Start the Dioxus dev-server once per test session."""
    base_url = "http://127.0.0.1:9990"

    # In CI we always start our own server; locally we reuse an existing one.
    if not os.environ.get("CI"):
        if _is_server_ready(base_url, timeout=2):
            yield base_url
            return

    # tests_gui/ → repo root → crates/rb-gui/
    crate_dir = Path(__file__).resolve().parents[1] / "crates" / "rb-gui"
    cmd = ["dx", "serve", "--platform", "web", "--port", "9990"]
    # dx serve requires the working dir to be the crate root
    proc = subprocess.Popen(
        cmd,
        cwd=str(crate_dir),
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        # On Windows we need CREATE_NEW_PROCESS_GROUP so we can send Ctrl+C
        creationflags=subprocess.CREATE_NEW_PROCESS_GROUP if sys.platform == "win32" else 0,
    )

    if not _is_server_ready(base_url):
        proc.terminate()
        pytest.fail("Dioxus dev-server did not become ready in time.")

    yield base_url

    # Tear-down
    if sys.platform == "win32":
        proc.send_signal(signal.CTRL_BREAK_EVENT)
    else:
        proc.terminate()
    try:
        proc.wait(timeout=10)
    except subprocess.TimeoutExpired:
        proc.kill()
