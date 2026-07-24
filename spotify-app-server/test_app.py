import os
import time
import sqlite3
import pytest
from unittest.mock import patch
from fastapi.testclient import TestClient

# Set environment BEFORE importing app so module-level constants are populated.
os.environ["OXITERM_APP_TOKEN"] = "test_secret_token_123"
os.environ["SPOTIFY_CLIENT_ID"] = "a2cff4fceae146db8ded92dae9ed9ddd"
os.environ["SPOTIFY_CLIENT_SECRET"] = "test_secret"

import app as app_module
from app import app, pending_oauth_states, active_oxiterm_sessions, init_db

client = TestClient(app)

# ---------------------------------------------------------------------------
# Fixtures
# ---------------------------------------------------------------------------

@pytest.fixture(autouse=True)
def isolated_db(tmp_path, monkeypatch):
    """
    Each test gets a fresh, isolated SQLite database rooted in pytest's tmp_path.
    Prevents test_10 / test_16 inserts from contaminating the production .cache/spotify_app.db.
    """
    test_db = str(tmp_path / "test_spotify.db")
    monkeypatch.setattr(app_module, "DB_PATH", test_db)
    init_db()
    pending_oauth_states.clear()
    active_oxiterm_sessions.clear()
    yield test_db
    pending_oauth_states.clear()
    active_oxiterm_sessions.clear()

# ---------------------------------------------------------------------------
# Helper: mock Spotify token + profile exchange
# ---------------------------------------------------------------------------

def _mock_spotify(mock_post, mock_get, *, user_id="spot_user_1", display_name="User 1"):
    mock_post.return_value.status_code = 200
    mock_post.return_value.json.return_value = {
        "access_token": "acc_tok", "refresh_token": "ref_tok", "expires_in": 3600
    }
    mock_get.return_value.status_code = 200
    mock_get.return_value.json.return_value = {"id": user_id, "display_name": display_name}

# ---------------------------------------------------------------------------
# Tests 08–18
# ---------------------------------------------------------------------------

def test_08_events_unauthorized_without_bearer():
    r = client.post("/events", json={"action": "tab:player", "state": {}, "session_id": 1})
    assert r.status_code == 401


def test_09_events_unknown_session_id_returns_unauthenticated():
    r = client.post(
        "/events",
        json={"action": "tab:player", "state": {}, "session_id": 999},
        headers={"Authorization": "Bearer test_secret_token_123"}
    )
    assert r.status_code == 200
    data = r.json()
    assert data.get("is_authenticated") == "false"
    assert data.get("auth_status") == "Brak autoryzacji"


def test_10_no_fallback_to_other_users_in_db(isolated_db):
    # Insert a user into the isolated test DB.
    with sqlite3.connect(isolated_db) as conn:
        conn.execute("""
            INSERT OR REPLACE INTO users
                (spotify_user_id, display_name, access_token, refresh_token, expires_at, session_token, last_seen)
            VALUES ('test_user_id', 'Test User', 'acc_123', 'ref_123', 9999999999, 'stoken_secret_99', 9999999999)
        """)
        conn.commit()

    r = client.post(
        "/events",
        json={"action": "tab:player", "state": {}, "session_id": 888},
        headers={"Authorization": "Bearer test_secret_token_123"}
    )
    assert r.status_code == 200
    data = r.json()
    # Must NOT fall back to the DB user for an unknown session.
    assert data.get("is_authenticated") == "false"
    assert "Test User" not in str(data)


def test_11_callback_unknown_state_returns_400_and_no_session_created():
    # Seed a known session entry so we can verify it is not mutated.
    active_oxiterm_sessions[7] = ("existing_token", time.time())

    r = client.get("/callback?code=test_code&state=unknown_state_xyz")

    assert r.status_code == 400
    assert "nieprawidłowy lub przeterminowany" in r.text
    # active_oxiterm_sessions must remain unchanged.
    assert active_oxiterm_sessions.get(7) is not None
    assert active_oxiterm_sessions[7][0] == "existing_token"


def test_12_callback_binds_token_only_to_state_session():
    state = "valid_state_123"
    pending_oauth_states[state] = (42, time.time())

    # Seed an unrelated session that must NOT be touched.
    active_oxiterm_sessions[43] = ("inny_token", time.time())
    token_before = active_oxiterm_sessions[43][0]

    with patch("requests.post") as mock_post, patch("requests.get") as mock_get:
        _mock_spotify(mock_post, mock_get)

        r = client.get(f"/callback?code=valid_code&state={state}")
        assert r.status_code == 200
        assert "Zalogowano pomyślnie" in r.text

        # Session 42 must now be registered.
        assert 42 in active_oxiterm_sessions

        # Session 43 must be completely unchanged.
        assert active_oxiterm_sessions[43][0] == token_before


def test_13_callback_expired_state_returns_400():
    state = "expired_state"
    pending_oauth_states[state] = (42, time.time() - 601)  # > 10 min old

    r = client.get(f"/callback?code=valid_code&state={state}")
    assert r.status_code == 400
    assert "przeterminowany" in r.text


def test_14_callback_state_single_use():
    state = "single_use_state"
    pending_oauth_states[state] = (42, time.time())

    with patch("requests.post") as mock_post, patch("requests.get") as mock_get:
        _mock_spotify(mock_post, mock_get)

        r1 = client.get(f"/callback?code=code1&state={state}")
        assert r1.status_code == 200

        # Second call with the same state token must be rejected.
        r2 = client.get(f"/callback?code=code2&state={state}")
        assert r2.status_code == 400


def test_15_callback_reflected_xss_escaped():
    r = client.get("/callback?error=<script>alert('xss')</script>")
    assert r.status_code == 400
    assert "<script>" not in r.text
    assert "&lt;script&gt;" in r.text


def test_16_events_patch_does_not_leak_session_token(isolated_db):
    active_oxiterm_sessions[10] = ("stoken_secret_10", time.time())

    with sqlite3.connect(isolated_db) as conn:
        conn.execute("""
            INSERT OR REPLACE INTO users
                (spotify_user_id, display_name, access_token, refresh_token, expires_at, session_token, last_seen)
            VALUES ('spot_10', 'User 10', 'acc_10', 'ref_10', 9999999999, 'stoken_secret_10', 9999999999)
        """)
        conn.commit()

    with patch("requests.get") as mock_get:
        mock_get.return_value.status_code = 204  # No active playback
        r = client.post(
            "/events",
            json={"action": "tab:player", "state": {}, "session_id": 10},
            headers={"Authorization": "Bearer test_secret_token_123"}
        )
        assert r.status_code == 200
        data = r.json()
        assert "user_session_token" not in data
        assert "access_token" not in data
        assert "refresh_token" not in data


def test_17_background_patch_does_not_leak_tokens():
    from app import fetch_playback_for_user
    with patch("requests.get") as mock_get:
        mock_get.return_value.status_code = 204
        patch_data = fetch_playback_for_user("fake_acc_token")
        assert "user_session_token" not in patch_data
        assert "access_token" not in patch_data
        assert "refresh_token" not in patch_data


def test_18_trigger_login_generates_auth_url_with_state():
    r = client.post(
        "/events",
        json={"action": "trigger_login", "state": {}, "session_id": 77},
        headers={"Authorization": "Bearer test_secret_token_123"}
    )
    assert r.status_code == 200
    data = r.json()
    auth_url = data.get("auth_url", "")
    assert "state=" in auth_url
    assert "client_id=" in auth_url


def test_19_spotify_panel_htmx_events_contract():
    import re
    from pathlib import Path

    possible_dirs = [
        Path("examples/spotify"),
        Path("../examples/spotify"),
        Path("/app/examples/spotify"),
    ]
    spotify_dir = None
    for d in possible_dirs:
        if d.exists() and d.is_dir():
            spotify_dir = d
            break

    assert spotify_dir is not None, "examples/spotify directory not found!"

    panel_files = list(spotify_dir.glob("*.thtml"))
    assert len(panel_files) >= 1, "No .thtml files found in examples/spotify!"

    actions = set()
    for pf in panel_files:
        content = pf.read_text(encoding="utf-8")
        matches = re.findall(r'event-htmx="([^"]+)"', content)
        for act in matches:
            # Ignore engine built-ins & navigation
            if act.startswith(("set:", "inc:", "dec:", "toggle:", "append:", "clear:", "open:")):
                continue
            if act.endswith(".thtml"):
                continue
            actions.add(act)

    assert len(actions) > 0, "No custom HTMX actions found in Spotify panel files!"

    # Test each custom action against /events endpoint in test app
    for action in actions:
        r = client.post(
            "/events",
            json={"action": action, "state": {}, "session_id": 1},
            headers={"Authorization": "Bearer test_secret_token_123"}
        )
        assert r.status_code == 200, f"Custom action '{action}' failed with status {r.status_code}"

