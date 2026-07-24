import os
import time
import pytest
from fastapi.testclient import TestClient

# Set environment before importing app
os.environ["OXITERM_APP_TOKEN"] = "test_secret_token_123"
os.environ["SPOTIFY_CLIENT_ID"] = "a2cff4fceae146db8ded92dae9ed9ddd"
os.environ["SPOTIFY_CLIENT_SECRET"] = "test_secret"

from app import app, pending_oauth_states, active_oxiterm_sessions, DB_PATH, init_db

client = TestClient(app)

@pytest.fixture(autouse=True)
def reset_state():
    pending_oauth_states.clear()
    active_oxiterm_sessions.clear()

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

def test_10_no_fallback_to_other_users_in_db():
    # Insert dummy user into SQLite DB
    import sqlite3
    with sqlite3.connect(DB_PATH) as conn:
        cursor = conn.cursor()
        cursor.execute("""
            INSERT OR REPLACE INTO users (spotify_user_id, display_name, access_token, refresh_token, expires_at, session_token, last_seen)
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
    assert data.get("is_authenticated") == "false"
    assert "Test User" not in str(data)

def test_11_callback_unknown_state_returns_400():
    r = client.get("/callback?code=test_code&state=unknown_state_xyz")
    assert r.status_code == 400
    assert "nieprawidłowy lub przeterminowany" in r.text

def test_12_callback_binds_token_only_to_state_session():
    state = "valid_state_123"
    pending_oauth_states[state] = (42, time.time())
    
    # Mock requests.post and requests.get for Spotify API exchange
    from unittest.mock import patch, MagicMock
    with patch("requests.post") as mock_post, patch("requests.get") as mock_get:
        mock_post.return_value.status_code = 200
        mock_post.return_value.json.return_value = {
            "access_token": "acc_tok", "refresh_token": "ref_tok", "expires_in": 3600
        }
        mock_get.return_value.status_code = 200
        mock_get.return_value.json.return_value = {
            "id": "spot_user_1", "display_name": "User 1"
        }
        
        r = client.get(f"/callback?code=valid_code&state={state}")
        assert r.status_code == 200
        assert "Zalogowano pomyślnie" in r.text
        assert 42 in active_oxiterm_sessions
        assert 43 not in active_oxiterm_sessions

def test_13_callback_expired_state_returns_400():
    state = "expired_state"
    pending_oauth_states[state] = (42, time.time() - 601)  # > 10 min old
    
    r = client.get(f"/callback?code=valid_code&state={state}")
    assert r.status_code == 400
    assert "przeterminowany" in r.text

def test_14_callback_state_single_use():
    state = "single_use_state"
    pending_oauth_states[state] = (42, time.time())
    
    from unittest.mock import patch
    with patch("requests.post") as mock_post, patch("requests.get") as mock_get:
        mock_post.return_value.status_code = 200
        mock_post.return_value.json.return_value = {
            "access_token": "acc_tok", "refresh_token": "ref_tok", "expires_in": 3600
        }
        mock_get.return_value.status_code = 200
        mock_get.return_value.json.return_value = {"id": "u1", "display_name": "U1"}
        
        r1 = client.get(f"/callback?code=code1&state={state}")
        assert r1.status_code == 200
        
        # Second call with same state must return 400
        r2 = client.get(f"/callback?code=code2&state={state}")
        assert r2.status_code == 400

def test_15_callback_reflected_xss_escaped():
    r = client.get("/callback?error=<script>alert('xss')</script>")
    assert r.status_code == 400
    assert "<script>" not in r.text
    assert "&lt;script&gt;" in r.text

def test_16_events_patch_does_not_leak_session_token():
    # Setup active authenticated session
    active_oxiterm_sessions[10] = ("stoken_secret_10", time.time())
    
    import sqlite3
    with sqlite3.connect(DB_PATH) as conn:
        cursor = conn.cursor()
        cursor.execute("""
            INSERT OR REPLACE INTO users (spotify_user_id, display_name, access_token, refresh_token, expires_at, session_token, last_seen)
            VALUES ('spot_10', 'User 10', 'acc_10', 'ref_10', 9999999999, 'stoken_secret_10', 9999999999)
        """)
        conn.commit()
        
    from unittest.mock import patch
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
    from unittest.mock import patch
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
