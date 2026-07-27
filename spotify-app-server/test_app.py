import os
import time
import asyncio
import sqlite3
import pytest
from unittest.mock import patch, MagicMock
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
    test_db = str(tmp_path / "test_spotify.db")
    monkeypatch.setattr(app_module, "DB_PATH", test_db)
    init_db()
    pending_oauth_states.clear()
    active_oxiterm_sessions.clear()
    app_module.last_sent_app_token.clear()
    from poller import poller_manager
    from spotify_api import spotify_api_client
    poller_manager.accounts.clear()
    poller_manager.sessions.clear()
    poller_manager.last_sent_progress.clear()
    poller_manager.last_sent_app_token.clear()
    spotify_api_client._history.clear()
    spotify_api_client._breaker_until_mono_ms = 0
    yield test_db
    pending_oauth_states.clear()
    active_oxiterm_sessions.clear()
    app_module.last_sent_app_token.clear()
    poller_manager.accounts.clear()
    poller_manager.sessions.clear()
    poller_manager.last_sent_progress.clear()
    poller_manager.last_sent_app_token.clear()
    spotify_api_client._history.clear()
    spotify_api_client._breaker_until_mono_ms = 0

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
# Tests T-1..T-4 (Phase 1 — Session vitality)
# ---------------------------------------------------------------------------

def test_t1_push_200_updates_last_seen(isolated_db):
    import asyncio
    with sqlite3.connect(isolated_db) as conn:
        conn.execute("""
            INSERT OR REPLACE INTO users
                (spotify_user_id, display_name, access_token, refresh_token, expires_at, session_token, last_seen)
            VALUES ('u1', 'User 1', 'acc_1', 'ref_1', 9999999999, 'stoken_1', 9999999999)
        """)
        conn.commit()

    old_ts = time.time() - 100
    active_oxiterm_sessions[42] = ("stoken_1", old_ts)

    with patch("requests.post") as mock_post, patch("requests.get") as mock_get:
        mock_post.return_value.status_code = 200
        mock_get.return_value.status_code = 204

        asyncio.run(app_module.poll_once())

        assert 42 in active_oxiterm_sessions
        new_token, new_ts = active_oxiterm_sessions[42]
        assert new_token == "stoken_1"
        assert new_ts > old_ts


def test_t2_push_404_removes_session(isolated_db):
    import asyncio
    with sqlite3.connect(isolated_db) as conn:
        conn.execute("""
            INSERT OR REPLACE INTO users
                (spotify_user_id, display_name, access_token, refresh_token, expires_at, session_token, last_seen)
            VALUES ('u1', 'User 1', 'acc_1', 'ref_1', 9999999999, 'stoken_1', 9999999999)
        """)
        conn.commit()

    active_oxiterm_sessions[42] = ("stoken_1", time.time())

    with patch("requests.post") as mock_post, patch("requests.get") as mock_get:
        mock_post.return_value.status_code = 404
        mock_get.return_value.status_code = 204

        asyncio.run(app_module.poll_once())

        assert 42 not in active_oxiterm_sessions


def test_t3_session_survives_400s_of_pushes_without_events(isolated_db):
    import asyncio
    with sqlite3.connect(isolated_db) as conn:
        conn.execute("""
            INSERT OR REPLACE INTO users
                (spotify_user_id, display_name, access_token, refresh_token, expires_at, session_token, last_seen)
            VALUES ('u1', 'User 1', 'acc_1', 'ref_1', 9999999999, 'stoken_1', 9999999999)
        """)
        conn.commit()

    # Session registered 400 seconds ago
    start_ts = time.time() - 400
    active_oxiterm_sessions[42] = ("stoken_1", start_ts)

    with patch("requests.post") as mock_post, patch("requests.get") as mock_get:
        mock_post.return_value.status_code = 200
        mock_get.return_value.status_code = 204

        asyncio.run(app_module.poll_once())

        assert 42 in active_oxiterm_sessions
        _, ts = active_oxiterm_sessions[42]
        assert ts > start_ts


def test_t4_no_time_based_deletion_in_code():
    import inspect
    source = inspect.getsource(app_module.poll_spotify_and_push_patches)
    if hasattr(app_module, "poll_once"):
        source += inspect.getsource(app_module.poll_once)
    assert "now - last_seen > 300" not in source
    assert "> 300" not in source



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


# ---------------------------------------------------------------------------
# Tests T-13..T-20 (Phase 3 — App identity)
# ---------------------------------------------------------------------------

def test_t13_events_with_valid_app_token_returns_authenticated(isolated_db):
    with sqlite3.connect(isolated_db) as conn:
        conn.execute("""
            INSERT OR REPLACE INTO users
                (spotify_user_id, display_name, access_token, refresh_token, expires_at, session_token, last_seen)
            VALUES ('spot_t13', 'User T13', 'acc_t13', 'ref_t13', 9999999999, 'app_token_valid_123', 9999999999)
        """)
        conn.commit()

    with patch("requests.get") as mock_get:
        mock_get.return_value.status_code = 204
        r = client.post(
            "/events",
            json={"action": "set:tab=player", "state": {}, "session_id": 99, "app_token": "app_token_valid_123"},
            headers={"Authorization": "Bearer test_secret_token_123"}
        )
        assert r.status_code == 200
        data = r.json()
        assert data.get("is_authenticated") == "true"
        assert "User T13" in data.get("auth_status", "")


def test_t14_events_with_unknown_app_token_returns_unauthenticated():
    r = client.post(
        "/events",
        json={"action": "set:tab=player", "state": {}, "session_id": 99, "app_token": "unknown_token_999"},
        headers={"Authorization": "Bearer test_secret_token_123"}
    )
    assert r.status_code == 200
    data = r.json()
    assert data.get("is_authenticated") == "false"


def test_t15_events_without_app_token_returns_unauthenticated():
    r = client.post(
        "/events",
        json={"action": "set:tab=player", "state": {}, "session_id": 99},
        headers={"Authorization": "Bearer test_secret_token_123"}
    )
    assert r.status_code == 200
    data = r.json()
    assert data.get("is_authenticated") == "false"


def test_t16_relogin_same_spotify_user_id_keeps_existing_session_token(isolated_db):
    state1 = "state_relogin_1"
    pending_oauth_states[state1] = (10, time.time())

    with patch("requests.post") as mock_post, patch("requests.get") as mock_get:
        _mock_spotify(mock_post, mock_get, user_id="same_user_1", display_name="Same User")
        r1 = client.get(f"/callback?code=c1&state={state1}")
        assert r1.status_code == 200

    with sqlite3.connect(isolated_db) as conn:
        cursor = conn.cursor()
        cursor.execute("SELECT session_token FROM users WHERE spotify_user_id = 'same_user_1'")
        initial_token = cursor.fetchone()[0]

    # Relogin the same user
    state2 = "state_relogin_2"
    pending_oauth_states[state2] = (11, time.time())
    with patch("requests.post") as mock_post, patch("requests.get") as mock_get:
        _mock_spotify(mock_post, mock_get, user_id="same_user_1", display_name="Same User Updated")
        r2 = client.get(f"/callback?code=c2&state={state2}")
        assert r2.status_code == 200

    with sqlite3.connect(isolated_db) as conn:
        cursor = conn.cursor()
        cursor.execute("SELECT session_token FROM users WHERE spotify_user_id = 'same_user_1'")
        token_after_relogin = cursor.fetchone()[0]

    assert token_after_relogin == initial_token, "session_token must NOT be overwritten on relogin"


def test_t17_new_spotify_user_id_generates_new_nonempty_session_token(isolated_db):
    state = "state_new_user"
    pending_oauth_states[state] = (12, time.time())

    with patch("requests.post") as mock_post, patch("requests.get") as mock_get:
        _mock_spotify(mock_post, mock_get, user_id="brand_new_user_777", display_name="New User")
        r = client.get(f"/callback?code=c_new&state={state}")
        assert r.status_code == 200

    with sqlite3.connect(isolated_db) as conn:
        cursor = conn.cursor()
        cursor.execute("SELECT session_token FROM users WHERE spotify_user_id = 'brand_new_user_777'")
        row = cursor.fetchone()
        assert row is not None
        stoken = row[0]
        assert len(stoken) > 0


def test_t18_logout_action_removes_user_from_db_and_emits_empty_set_app_token(isolated_db):
    tok = "stoken_logout_test"
    with sqlite3.connect(isolated_db) as conn:
        conn.execute("""
            INSERT OR REPLACE INTO users
                (spotify_user_id, display_name, access_token, refresh_token, expires_at, session_token, last_seen)
            VALUES ('spot_logout', 'User Logout', 'acc', 'ref', 9999999999, 'stoken_logout_test', 9999999999)
        """)
        conn.commit()

    r = client.post(
        "/events",
        json={"action": "logout", "state": {}, "session_id": 55, "app_token": tok},
        headers={"Authorization": "Bearer test_secret_token_123"}
    )
    assert r.status_code == 200
    data = r.json()
    assert data.get("set_app_token") == ""
    assert data.get("is_authenticated") == "false"

    with sqlite3.connect(isolated_db) as conn:
        cursor = conn.cursor()
        cursor.execute("SELECT * FROM users WHERE session_token = ?", (tok,))
        assert cursor.fetchone() is None


def test_t19_events_after_logout_returns_unauthenticated(isolated_db):
    tok = "stoken_logout_t19"
    with sqlite3.connect(isolated_db) as conn:
        conn.execute("""
            INSERT OR REPLACE INTO users
                (spotify_user_id, display_name, access_token, refresh_token, expires_at, session_token, last_seen)
            VALUES ('spot_t19', 'User T19', 'acc', 'ref', 9999999999, 'stoken_logout_t19', 9999999999)
        """)
        conn.commit()

    # Logout
    client.post(
        "/events",
        json={"action": "logout", "state": {}, "session_id": 55, "app_token": tok},
        headers={"Authorization": "Bearer test_secret_token_123"}
    )

    # Subsequent /events with previous token
    r = client.post(
        "/events",
        json={"action": "set:tab=player", "state": {}, "session_id": 55, "app_token": tok},
        headers={"Authorization": "Bearer test_secret_token_123"}
    )
    assert r.status_code == 200
    assert r.json().get("is_authenticated") == "false"


def test_t20_username_payload_field_does_not_affect_identity_resolution(isolated_db):
    tok = "stoken_t20"
    with sqlite3.connect(isolated_db) as conn:
        conn.execute("""
            INSERT OR REPLACE INTO users
                (spotify_user_id, display_name, access_token, refresh_token, expires_at, session_token, last_seen)
            VALUES ('spot_t20', 'User T20', 'acc', 'ref', 9999999999, 'stoken_t20', 9999999999)
        """)
        conn.commit()

    with patch("requests.get") as mock_get:
        mock_get.return_value.status_code = 204
        # Send payload with username different from user in db
        r = client.post(
            "/events",
            json={
                "action": "set:tab=player",
                "state": {},
                "session_id": 55,
                "username": "completely_unrelated_username",
                "app_token": tok
            },
            headers={"Authorization": "Bearer test_secret_token_123"}
        )
        assert r.status_code == 200
        data = r.json()
        assert data.get("is_authenticated") == "true"
        assert "User T20" in data.get("auth_status", "")


def test_p41_t6_poller_two_cycles_set_app_token_only_in_first(isolated_db):
    tok = "stoken_poller_t6"
    with sqlite3.connect(isolated_db) as conn:
        conn.execute("""
            INSERT OR REPLACE INTO users
                (spotify_user_id, display_name, access_token, refresh_token, expires_at, session_token, last_seen)
            VALUES ('spot_t6', 'User T6', 'acc_t6', 'ref_t6', 9999999999, 'stoken_poller_t6', 9999999999)
        """)
        conn.commit()

    active_oxiterm_sessions[88] = (tok, time.time())
    app_module.last_sent_app_token.pop(88, None)

    with patch("requests.post") as mock_post, patch("requests.get") as mock_get:
        mock_get.return_value.status_code = 204
        mock_post.return_value.status_code = 200

        # First cycle
        asyncio.run(app_module.poll_once())
        assert mock_post.call_count == 1
        first_patch = mock_post.call_args[1].get("json") or mock_post.call_args[0][1]
        assert first_patch.get("set_app_token") == tok

        # Second cycle
        mock_post.reset_mock()
        asyncio.run(app_module.poll_once())
        assert mock_post.call_count == 1
        second_patch = mock_post.call_args[1].get("json") or mock_post.call_args[0][1]
        assert "set_app_token" not in second_patch


def test_p41_t7_session_transition_unauthenticated_to_authenticated_emits_set_app_token(isolated_db):
    tok = "stoken_transition_t7"

    with sqlite3.connect(isolated_db) as conn:
        conn.execute("""
            INSERT OR REPLACE INTO users
                (spotify_user_id, display_name, access_token, refresh_token, expires_at, session_token, last_seen)
            VALUES ('spot_t7', 'User T7', 'acc_t7', 'ref_t7', 9999999999, 'stoken_transition_t7', 9999999999)
        """)
        conn.commit()

    active_oxiterm_sessions[77] = (tok, time.time())
    app_module.last_sent_app_token.pop(77, None)

    with patch("requests.post") as mock_post, patch("requests.get") as mock_get:
        mock_get.return_value.status_code = 204
        mock_post.return_value.status_code = 200

        asyncio.run(app_module.poll_once())
        assert mock_post.call_count == 1
        patch_data = mock_post.call_args[1].get("json")
        assert patch_data.get("set_app_token") == tok


# ---------------------------------------------------------------------------
# PLAN 4.2 Part A Tests (T-1 .. T-8)
# ---------------------------------------------------------------------------

def _insert_play_user(db_path, tok="stoken_play_test", user_id="user_play"):
    with sqlite3.connect(db_path) as conn:
        conn.execute("""
            INSERT OR REPLACE INTO users
                (spotify_user_id, display_name, access_token, refresh_token, expires_at, session_token, last_seen)
            VALUES (?, 'User Play', 'acc_play', 'ref_play', 9999999999, ?, 9999999999)
        """, (user_id, tok))
        conn.commit()
    return tok


def test_t1_play_uri_resolves_key_from_state_and_sends_uris(isolated_db):
    tok = _insert_play_user(isolated_db)
    with patch("requests.put") as mock_put, patch("requests.get") as mock_get:
        mock_get.return_value.status_code = 204
        mock_put.return_value.status_code = 204
        r = client.post(
            "/events",
            json={
                "action": "play_uri:res_1_uri",
                "state": {"res_1_uri": "spotify:track:abc12345"},
                "session_id": 101,
                "app_token": tok
            },
            headers={"Authorization": "Bearer test_secret_token_123"}
        )
        assert r.status_code == 200
        assert mock_put.call_count == 1
        call_kwargs = mock_put.call_args[1]
        assert call_kwargs.get("json") == {"uris": ["spotify:track:abc12345"]}


def test_t2_play_uri_missing_key_in_state_sends_no_spotify_request(isolated_db):
    tok = _insert_play_user(isolated_db)
    with patch("requests.put") as mock_put, patch("requests.get") as mock_get:
        mock_get.return_value.status_code = 204
        r = client.post(
            "/events",
            json={
                "action": "play_uri:res_1_uri",
                "state": {},
                "session_id": 102,
                "app_token": tok
            },
            headers={"Authorization": "Bearer test_secret_token_123"}
        )
        assert r.status_code == 200
        assert mock_put.call_count == 0
        data = r.json()
        assert data.get("player_error") != ""
        assert "Brak URI" in data.get("player_error", "")


def test_t3_play_uri_empty_key_value_sends_no_spotify_request(isolated_db):
    tok = _insert_play_user(isolated_db)
    with patch("requests.put") as mock_put, patch("requests.get") as mock_get:
        mock_get.return_value.status_code = 204
        r = client.post(
            "/events",
            json={
                "action": "play_uri:res_1_uri",
                "state": {"res_1_uri": "   "},
                "session_id": 103,
                "app_token": tok
            },
            headers={"Authorization": "Bearer test_secret_token_123"}
        )
        assert r.status_code == 200
        assert mock_put.call_count == 0
        data = r.json()
        assert data.get("player_error") != ""


def test_t4_play_uri_playlist_sends_context_uri(isolated_db):
    tok = _insert_play_user(isolated_db)
    with patch("requests.put") as mock_put, patch("requests.get") as mock_get:
        mock_get.return_value.status_code = 204
        mock_put.return_value.status_code = 204
        r = client.post(
            "/events",
            json={
                "action": "play_uri:pl_1_uri",
                "state": {"pl_1_uri": "spotify:playlist:xyz789"},
                "session_id": 104,
                "app_token": tok
            },
            headers={"Authorization": "Bearer test_secret_token_123"}
        )
        assert r.status_code == 200
        assert mock_put.call_count == 1
        assert mock_put.call_args[1].get("json") == {"context_uri": "spotify:playlist:xyz789"}


def test_t5_play_uri_episode_sends_uris_not_context_uri(isolated_db):
    tok = _insert_play_user(isolated_db)
    with patch("requests.put") as mock_put, patch("requests.get") as mock_get:
        mock_get.return_value.status_code = 204
        mock_put.return_value.status_code = 204
        r = client.post(
            "/events",
            json={
                "action": "play_uri:ep_1_uri",
                "state": {"ep_1_uri": "spotify:episode:ep999"},
                "session_id": 105,
                "app_token": tok
            },
            headers={"Authorization": "Bearer test_secret_token_123"}
        )
        assert r.status_code == 200
        assert mock_put.call_count == 1
        assert mock_put.call_args[1].get("json") == {"uris": ["spotify:episode:ep999"]}


def test_t6_play_uri_show_sends_context_uri(isolated_db):
    tok = _insert_play_user(isolated_db)
    with patch("requests.put") as mock_put, patch("requests.get") as mock_get:
        mock_get.return_value.status_code = 204
        mock_put.return_value.status_code = 204
        r = client.post(
            "/events",
            json={
                "action": "play_uri:sh_1_uri",
                "state": {"sh_1_uri": "spotify:show:show456"},
                "session_id": 106,
                "app_token": tok
            },
            headers={"Authorization": "Bearer test_secret_token_123"}
        )
        assert r.status_code == 200
        assert mock_put.call_count == 1
        assert mock_put.call_args[1].get("json") == {"context_uri": "spotify:show:show456"}


def test_t7_play_uri_unknown_type_sends_no_request(isolated_db):
    tok = _insert_play_user(isolated_db)
    with patch("requests.put") as mock_put, patch("requests.get") as mock_get:
        mock_get.return_value.status_code = 204
        r = client.post(
            "/events",
            json={
                "action": "play_uri:res_1_uri",
                "state": {"res_1_uri": "spotify:unknown_type:123"},
                "session_id": 107,
                "app_token": tok
            },
            headers={"Authorization": "Bearer test_secret_token_123"}
        )
        assert r.status_code == 200
        assert mock_put.call_count == 0
        data = r.json()
        assert data.get("player_error") != ""


def test_t8_play_uri_non_spotify_prefix_never_creates_request(isolated_db):
    tok = _insert_play_user(isolated_db)
    with patch("requests.put") as mock_put, patch("requests.get") as mock_get:
        mock_get.return_value.status_code = 204
        r = client.post(
            "/events",
            json={
                "action": "play_uri:res_1_uri",
                "state": {"res_1_uri": "https://attacker.com/malicious"},
                "session_id": 108,
                "app_token": tok
            },
            headers={"Authorization": "Bearer test_secret_token_123"}
        )
        assert r.status_code == 200
        assert mock_put.call_count == 0
        data = r.json()
        assert data.get("player_error") != ""


# ---------------------------------------------------------------------------
# PLAN 4.2 Part B Tests (T-9 .. T-11)
# ---------------------------------------------------------------------------

def test_t9_spotify_403_populates_player_error_and_logs_status(isolated_db):
    tok = _insert_play_user(isolated_db)
    with patch("requests.put") as mock_put, patch("requests.get") as mock_get:
        mock_get.return_value.status_code = 204
        mock_put.return_value.status_code = 403
        mock_put.return_value.text = "Forbidden / Not Premium"
        r = client.post(
            "/events",
            json={
                "action": "play_uri:res_1_uri",
                "state": {"res_1_uri": "spotify:track:abc12345"},
                "session_id": 109,
                "app_token": tok
            },
            headers={"Authorization": "Bearer test_secret_token_123"}
        )
        assert r.status_code == 200
        data = r.json()
        assert data.get("player_error") != ""
        assert "403" in data.get("player_error", "")


def test_t10_spotify_204_clears_player_error(isolated_db):
    tok = _insert_play_user(isolated_db)
    with patch("requests.put") as mock_put, patch("requests.get") as mock_get:
        mock_get.return_value.status_code = 204
        mock_put.return_value.status_code = 204
        r = client.post(
            "/events",
            json={
                "action": "play_uri:res_1_uri",
                "state": {"res_1_uri": "spotify:track:abc12345"},
                "session_id": 110,
                "app_token": tok
            },
            headers={"Authorization": "Bearer test_secret_token_123"}
        )
        assert r.status_code == 200
        data = r.json()
        assert data.get("player_error") == ""


def test_t11_network_exception_populates_player_error_poller_continues(isolated_db):
    tok = _insert_play_user(isolated_db)
    import requests
    with patch("requests.put") as mock_put, patch("requests.get") as mock_get:
        mock_get.return_value.status_code = 204
        mock_put.side_effect = requests.RequestException("Connection refused")
        r = client.post(
            "/events",
            json={
                "action": "play_uri:res_1_uri",
                "state": {"res_1_uri": "spotify:track:abc12345"},
                "session_id": 111,
                "app_token": tok
            },
            headers={"Authorization": "Bearer test_secret_token_123"}
        )
        assert r.status_code == 200
        data = r.json()
        assert data.get("player_error") != ""


# ---------------------------------------------------------------------------
# PLAN 4.2 Part C Tests (T-12 .. T-20 & T-23)
# ---------------------------------------------------------------------------

def test_t12_all_me_player_calls_include_additional_types_episode():
    """Static analysis test verifying every GET /v1/me/player call in app.py passes additional_types=episode."""
    app_py_path = os.path.join(os.path.dirname(__file__), "app.py")
    with open(app_py_path, "r", encoding="utf-8") as f:
        content = f.read()

    import re
    # Match /v1/me/player when NOT followed by slash or endpoint sub-path (/play, /pause, /next, etc.)
    matches = [m.start() for m in re.finditer(r"/v1/me/player(?=[\"?\s,]|\Z)", content)]
    assert len(matches) >= 1, "Must find /v1/me/player calls in app.py"
    for idx in matches:
        snippet = content[idx:idx + 120]
        assert "additional_types=episode" in snippet, f"Call at position {idx} missing additional_types=episode: {snippet}"


def test_t13_episode_item_parses_show_name_and_publisher(isolated_db):
    tok = _insert_play_user(isolated_db, tok="stoken_ep13")
    active_oxiterm_sessions[301] = (tok, time.time())

    mock_player_resp = {
        "is_playing": True,
        "progress_ms": 120000,
        "currently_playing_type": "episode",
        "item": {
            "type": "episode",
            "name": "Episode 42: Python Internals",
            "duration_ms": 1800000,
            "release_date": "2026-07-25",
            "show": {
                "name": "Tech Talk Podcast",
                "publisher": "Tech Media Network"
            }
        },
        "actions": {"disallows": {}}
    }

    with patch("requests.get") as mock_get, patch("requests.post") as mock_post:
        mock_get.return_value.status_code = 200
        mock_get.return_value.json.return_value = mock_player_resp
        mock_post.return_value.status_code = 200

        asyncio.run(app_module.poll_once())

        assert mock_post.call_count == 1
        patch_data = mock_post.call_args[1].get("json")
        assert patch_data.get("track_name") == "Episode 42: Python Internals"[:35]
        assert patch_data.get("artist_name") == "Tech Talk Podcast"[:35]
        assert patch_data.get("album_name") == "2026-07-25"


def test_t14_track_item_parses_track_and_artists_without_regression(isolated_db):
    tok = _insert_play_user(isolated_db, tok="stoken_tr14")
    active_oxiterm_sessions[302] = (tok, time.time())

    mock_player_resp = {
        "is_playing": True,
        "progress_ms": 60000,
        "currently_playing_type": "track",
        "item": {
            "type": "track",
            "name": "Get Lucky",
            "duration_ms": 240000,
            "artists": [{"name": "Daft Punk"}, {"name": "Pharrell Williams"}],
            "album": {"name": "Random Access Memories"}
        },
        "actions": {"disallows": {}}
    }

    with patch("requests.get") as mock_get, patch("requests.post") as mock_post:
        mock_get.return_value.status_code = 200
        mock_get.return_value.json.return_value = mock_player_resp
        mock_post.return_value.status_code = 200

        asyncio.run(app_module.poll_once())

        assert mock_post.call_count == 1
        patch_data = mock_post.call_args[1].get("json")
        assert patch_data.get("track_name") == "Get Lucky"
        assert "Daft Punk" in patch_data.get("artist_name", "")
        assert patch_data.get("album_name") == "Random Access Memories"[:35]


def test_t15_null_item_with_currently_playing_episode_handles_gracefully(isolated_db):
    tok = _insert_play_user(isolated_db, tok="stoken_null15")
    active_oxiterm_sessions[303] = (tok, time.time())

    mock_player_resp = {
        "is_playing": False,
        "progress_ms": 0,
        "currently_playing_type": "episode",
        "item": None,
        "actions": {"disallows": {}}
    }

    with patch("requests.get") as mock_get, patch("requests.post") as mock_post:
        mock_get.return_value.status_code = 200
        mock_get.return_value.json.return_value = mock_player_resp
        mock_post.return_value.status_code = 200

        asyncio.run(app_module.poll_once())
        assert mock_post.call_count == 1
        patch_data = mock_post.call_args[1].get("json")
        assert patch_data.get("track_name") == "Brak odtwarzania"


def test_t23_null_item_does_not_reset_auth_state(isolated_db):
    """CRITICAL K-1: item == null MUST NOT set is_authenticated=false or revoke session auth."""
    tok = _insert_play_user(isolated_db, tok="stoken_t23")
    active_oxiterm_sessions[304] = (tok, time.time())

    mock_player_resp = {
        "is_playing": False,
        "progress_ms": 0,
        "currently_playing_type": "unknown",
        "item": None,
        "actions": {"disallows": {}}
    }

    with patch("requests.get") as mock_get, patch("requests.post") as mock_post:
        mock_get.return_value.status_code = 200
        mock_get.return_value.json.return_value = mock_player_resp
        mock_post.return_value.status_code = 200

        asyncio.run(app_module.poll_once())
        assert mock_post.call_count == 1
        patch_data = mock_post.call_args[1].get("json")
        assert "is_authenticated" not in patch_data or patch_data.get("is_authenticated") == "true"


def test_t16_disallows_skipping_next_sets_can_next_false(isolated_db):
    tok = _insert_play_user(isolated_db, tok="stoken_dis16")
    active_oxiterm_sessions[305] = (tok, time.time())

    mock_player_resp = {
        "is_playing": True,
        "progress_ms": 10000,
        "item": {"type": "track", "name": "Song", "duration_ms": 100000, "artists": [], "album": {"name": "A"}},
        "actions": {"disallows": {"skipping_next": True}}
    }

    with patch("requests.get") as mock_get, patch("requests.post") as mock_post:
        mock_get.return_value.status_code = 200
        mock_get.return_value.json.return_value = mock_player_resp
        mock_post.return_value.status_code = 200

        asyncio.run(app_module.poll_once())
        assert mock_post.call_count == 1
        patch_data = mock_post.call_args[1].get("json")
        assert patch_data.get("can_next") == "false"
        assert patch_data.get("can_prev") == "true"


def test_t17_disallows_empty_sets_all_controls_true(isolated_db):
    tok = _insert_play_user(isolated_db, tok="stoken_dis17")
    active_oxiterm_sessions[306] = (tok, time.time())

    mock_player_resp = {
        "is_playing": True,
        "progress_ms": 10000,
        "item": {"type": "track", "name": "Song", "duration_ms": 100000, "artists": [], "album": {"name": "A"}},
        "actions": {"disallows": {}}
    }

    with patch("requests.get") as mock_get, patch("requests.post") as mock_post:
        mock_get.return_value.status_code = 200
        mock_get.return_value.json.return_value = mock_player_resp
        mock_post.return_value.status_code = 200

        asyncio.run(app_module.poll_once())
        assert mock_post.call_count == 1
        patch_data = mock_post.call_args[1].get("json")
        assert patch_data.get("can_next") == "true"
        assert patch_data.get("can_prev") == "true"
        assert patch_data.get("can_seek") == "true"


def test_t18_seek_fwd_clamped_to_duration_ms(isolated_db):
    tok = _insert_play_user(isolated_db)
    mock_pb = {
        "progress_ms": 175000,
        "item": {"duration_ms": 180000}
    }
    with patch("requests.get") as mock_get, patch("requests.put") as mock_put:
        mock_get.return_value.status_code = 200
        mock_get.return_value.json.return_value = mock_pb
        mock_put.return_value.status_code = 204

        r = client.post(
            "/events",
            json={
                "action": "seek_fwd",
                "state": {},
                "session_id": 307,
                "app_token": tok
            },
            headers={"Authorization": "Bearer test_secret_token_123"}
        )
        assert r.status_code == 200
        assert mock_put.call_count == 1
        url = mock_put.call_args[0][0]
        assert "position_ms=180000" in url


def test_t19_seek_back_clamped_to_zero(isolated_db):
    tok = _insert_play_user(isolated_db)
    mock_pb = {
        "progress_ms": 5000,
        "item": {"duration_ms": 180000}
    }
    with patch("requests.get") as mock_get, patch("requests.put") as mock_put:
        mock_get.return_value.status_code = 200
        mock_get.return_value.json.return_value = mock_pb
        mock_put.return_value.status_code = 204

        r = client.post(
            "/events",
            json={
                "action": "seek_back",
                "state": {},
                "session_id": 308,
                "app_token": tok
            },
            headers={"Authorization": "Bearer test_secret_token_123"}
        )
        assert r.status_code == 200
        assert mock_put.call_count == 1
        url = mock_put.call_args[0][0]
        assert "position_ms=0" in url


def test_t20_seek_fwd_issues_put_to_seek_endpoint(isolated_db):
    tok = _insert_play_user(isolated_db)
    mock_pb = {
        "progress_ms": 30000,
        "item": {"duration_ms": 180000}
    }
    with patch("requests.get") as mock_get, patch("requests.put") as mock_put:
        mock_get.return_value.status_code = 200
        mock_get.return_value.json.return_value = mock_pb
        mock_put.return_value.status_code = 204

        r = client.post(
            "/events",
            json={
                "action": "seek_fwd",
                "state": {},
                "session_id": 309,
                "app_token": tok
            },
            headers={"Authorization": "Bearer test_secret_token_123"}
        )
        assert r.status_code == 200
        assert mock_put.call_count == 1
        url = mock_put.call_args[0][0]
        assert "/v1/me/player/seek" in url
        assert "position_ms=45000" in url


# ---------------------------------------------------------------------------
# PLAN 4.3 Tests (T-1 .. T-19)
# ---------------------------------------------------------------------------

def test_p43_t1_disallows_skipping_next_true_sets_can_next_false(isolated_db):
    tok = _insert_play_user(isolated_db, tok="stoken_p43_t1")
    active_oxiterm_sessions[401] = (tok, time.time())
    mock_player_resp = {
        "is_playing": True,
        "progress_ms": 10000,
        "item": {"type": "track", "name": "Song", "duration_ms": 100000, "artists": [], "album": {"name": "A"}},
        "actions": {"disallows": {"skipping_next": True}}
    }
    with patch("requests.get") as mock_get, patch("requests.post") as mock_post:
        mock_get.return_value.status_code = 200
        mock_get.return_value.json.return_value = mock_player_resp
        mock_post.return_value.status_code = 200
        asyncio.run(app_module.poll_once())
        assert mock_post.call_count == 1
        patch_data = mock_post.call_args[1].get("json")
        assert patch_data.get("can_next") == "false"


def test_p43_t2_flat_actions_skipping_next_true_sets_can_next_false(isolated_db):
    tok = _insert_play_user(isolated_db, tok="stoken_p43_t2")
    active_oxiterm_sessions[402] = (tok, time.time())
    mock_player_resp = {
        "is_playing": True,
        "progress_ms": 10000,
        "item": {"type": "track", "name": "Song", "duration_ms": 100000, "artists": [], "album": {"name": "A"}},
        "actions": {"skipping_next": True}
    }
    with patch("requests.get") as mock_get, patch("requests.post") as mock_post:
        mock_get.return_value.status_code = 200
        mock_get.return_value.json.return_value = mock_player_resp
        mock_post.return_value.status_code = 200
        asyncio.run(app_module.poll_once())
        assert mock_post.call_count == 1
        patch_data = mock_post.call_args[1].get("json")
        assert patch_data.get("can_next") == "false"


def test_p43_t3_flat_actions_skipping_next_false_sets_can_next_true(isolated_db):
    tok = _insert_play_user(isolated_db, tok="stoken_p43_t3")
    active_oxiterm_sessions[403] = (tok, time.time())
    mock_player_resp = {
        "is_playing": True,
        "progress_ms": 10000,
        "item": {"type": "track", "name": "Song", "duration_ms": 100000, "artists": [], "album": {"name": "A"}},
        "actions": {"skipping_next": False}
    }
    with patch("requests.get") as mock_get, patch("requests.post") as mock_post:
        mock_get.return_value.status_code = 200
        mock_get.return_value.json.return_value = mock_player_resp
        mock_post.return_value.status_code = 200
        asyncio.run(app_module.poll_once())
        assert mock_post.call_count == 1
        patch_data = mock_post.call_args[1].get("json")
        assert patch_data.get("can_next") == "true"


def test_p43_t4_missing_actions_sets_all_controls_true(isolated_db):
    tok = _insert_play_user(isolated_db, tok="stoken_p43_t4")
    active_oxiterm_sessions[404] = (tok, time.time())
    mock_player_resp = {
        "is_playing": True,
        "progress_ms": 10000,
        "item": {"type": "track", "name": "Song", "duration_ms": 100000, "artists": [], "album": {"name": "A"}}
    }
    with patch("requests.get") as mock_get, patch("requests.post") as mock_post:
        mock_get.return_value.status_code = 200
        mock_get.return_value.json.return_value = mock_player_resp
        mock_post.return_value.status_code = 200
        asyncio.run(app_module.poll_once())
        assert mock_post.call_count == 1
        patch_data = mock_post.call_args[1].get("json")
        assert patch_data.get("can_next") == "true"
        assert patch_data.get("can_prev") == "true"
        assert patch_data.get("can_seek") == "true"


def test_p43_t5_device_is_restricted_sets_all_can_false_and_player_info(isolated_db):
    tok = _insert_play_user(isolated_db, tok="stoken_p43_t5")
    active_oxiterm_sessions[405] = (tok, time.time())
    mock_player_resp = {
        "is_playing": True,
        "progress_ms": 10000,
        "device": {"name": "Web Player", "is_restricted": True},
        "item": {"type": "track", "name": "Song", "duration_ms": 100000, "artists": [], "album": {"name": "A"}}
    }
    with patch("requests.get") as mock_get, patch("requests.post") as mock_post:
        mock_get.return_value.status_code = 200
        mock_get.return_value.json.return_value = mock_player_resp
        mock_post.return_value.status_code = 200
        asyncio.run(app_module.poll_once())
        assert mock_post.call_count == 1
        patch_data = mock_post.call_args[1].get("json")
        assert patch_data.get("can_next") == "false"
        assert patch_data.get("can_prev") == "false"
        assert patch_data.get("can_seek") == "false"
        assert patch_data.get("can_volume") == "false"
        assert patch_data.get("player_info") != ""


def test_p43_t6_device_restricted_precedes_permissive_actions(isolated_db):
    tok = _insert_play_user(isolated_db, tok="stoken_p43_t6")
    active_oxiterm_sessions[406] = (tok, time.time())
    mock_player_resp = {
        "is_playing": True,
        "progress_ms": 10000,
        "device": {"name": "Restricted Speaker", "is_restricted": True},
        "item": {"type": "track", "name": "Song", "duration_ms": 100000, "artists": [], "album": {"name": "A"}},
        "actions": {"disallows": {}}
    }
    with patch("requests.get") as mock_get, patch("requests.post") as mock_post:
        mock_get.return_value.status_code = 200
        mock_get.return_value.json.return_value = mock_player_resp
        mock_post.return_value.status_code = 200
        asyncio.run(app_module.poll_once())
        assert mock_post.call_count == 1
        patch_data = mock_post.call_args[1].get("json")
        assert patch_data.get("can_next") == "false"
        assert patch_data.get("can_prev") == "false"


def test_p43_t7_device_supports_volume_false_sets_can_volume_false(isolated_db):
    tok = _insert_play_user(isolated_db, tok="stoken_p43_t7")
    active_oxiterm_sessions[407] = (tok, time.time())
    mock_player_resp = {
        "is_playing": True,
        "device": {"name": "TV", "is_restricted": False, "supports_volume": False},
        "item": {"type": "track", "name": "Song", "duration_ms": 100000, "artists": [], "album": {"name": "A"}}
    }
    with patch("requests.get") as mock_get, patch("requests.post") as mock_post:
        mock_get.return_value.status_code = 200
        mock_get.return_value.json.return_value = mock_player_resp
        mock_post.return_value.status_code = 200
        asyncio.run(app_module.poll_once())
        assert mock_post.call_count == 1
        patch_data = mock_post.call_args[1].get("json")
        assert patch_data.get("can_volume") == "false"


def test_p43_t8_missing_supports_volume_defaults_can_volume_true(isolated_db):
    tok = _insert_play_user(isolated_db, tok="stoken_p43_t8")
    active_oxiterm_sessions[408] = (tok, time.time())
    mock_player_resp = {
        "is_playing": True,
        "device": {"name": "Phone", "is_restricted": False},
        "item": {"type": "track", "name": "Song", "duration_ms": 100000, "artists": [], "album": {"name": "A"}}
    }
    with patch("requests.get") as mock_get, patch("requests.post") as mock_post:
        mock_get.return_value.status_code = 200
        mock_get.return_value.json.return_value = mock_player_resp
        mock_post.return_value.status_code = 200
        asyncio.run(app_module.poll_once())
        assert mock_post.call_count == 1
        patch_data = mock_post.call_args[1].get("json")
        assert patch_data.get("can_volume") == "true"


def test_p43_t9_status_204_sets_player_info_and_keeps_is_authenticated(isolated_db):
    tok = _insert_play_user(isolated_db, tok="stoken_p43_t9")
    active_oxiterm_sessions[409] = (tok, time.time())
    with patch("requests.get") as mock_get, patch("requests.post") as mock_post:
        mock_get.return_value.status_code = 204
        mock_post.return_value.status_code = 200
        asyncio.run(app_module.poll_once())
        assert mock_post.call_count == 1
        patch_data = mock_post.call_args[1].get("json")
        assert patch_data.get("player_info") != ""
        assert patch_data.get("is_authenticated") == "true"


def test_p43_t10_ad_currently_playing_type_sets_player_info_reklama(isolated_db):
    tok = _insert_play_user(isolated_db, tok="stoken_p43_t10")
    active_oxiterm_sessions[410] = (tok, time.time())
    mock_player_resp = {
        "is_playing": True,
        "currently_playing_type": "ad",
        "item": None
    }
    with patch("requests.get") as mock_get, patch("requests.post") as mock_post:
        mock_get.return_value.status_code = 200
        mock_get.return_value.json.return_value = mock_player_resp
        mock_post.return_value.status_code = 200
        asyncio.run(app_module.poll_once())
        assert mock_post.call_count == 1
        patch_data = mock_post.call_args[1].get("json")
        assert patch_data.get("player_info") == "reklama"
        assert patch_data.get("is_authenticated") == "true"


def test_p43_t11_unknown_currently_playing_type_sets_player_info_unsupported(isolated_db):
    tok = _insert_play_user(isolated_db, tok="stoken_p43_t11")
    active_oxiterm_sessions[411] = (tok, time.time())
    mock_player_resp = {
        "is_playing": False,
        "currently_playing_type": "unknown",
        "item": None
    }
    with patch("requests.get") as mock_get, patch("requests.post") as mock_post:
        mock_get.return_value.status_code = 200
        mock_get.return_value.json.return_value = mock_player_resp
        mock_post.return_value.status_code = 200
        asyncio.run(app_module.poll_once())
        assert mock_post.call_count == 1
        patch_data = mock_post.call_args[1].get("json")
        assert "unobslugiwan" in patch_data.get("player_info", "").lower() or patch_data.get("player_info") != ""


def test_p43_t12_null_item_not_playing_sets_empty_player_info(isolated_db):
    tok = _insert_play_user(isolated_db, tok="stoken_p43_t12")
    active_oxiterm_sessions[412] = (tok, time.time())
    mock_player_resp = {
        "is_playing": False,
        "currently_playing_type": "track",
        "item": None
    }
    with patch("requests.get") as mock_get, patch("requests.post") as mock_post:
        mock_get.return_value.status_code = 200
        mock_get.return_value.json.return_value = mock_player_resp
        mock_post.return_value.status_code = 200
        asyncio.run(app_module.poll_once())
        assert mock_post.call_count == 1
        patch_data = mock_post.call_args[1].get("json")
        assert patch_data.get("player_info") == ""


def test_p43_t13_episode_album_name_uses_release_date(isolated_db):
    tok = _insert_play_user(isolated_db, tok="stoken_p43_t13")
    active_oxiterm_sessions[413] = (tok, time.time())
    mock_player_resp = {
        "is_playing": True,
        "currently_playing_type": "episode",
        "item": {
            "type": "episode",
            "name": "Ep 1",
            "release_date": "2026-07-25",
            "show": {"name": "Podcast Show", "publisher": "Forbidden Publisher"},
            "description": "Forbidden Description"
        }
    }
    with patch("requests.get") as mock_get, patch("requests.post") as mock_post:
        mock_get.return_value.status_code = 200
        mock_get.return_value.json.return_value = mock_player_resp
        mock_post.return_value.status_code = 200
        asyncio.run(app_module.poll_once())
        assert mock_post.call_count == 1
        patch_data = mock_post.call_args[1].get("json")
        assert patch_data.get("album_name") == "2026-07-25"
        assert "Forbidden" not in patch_data.get("album_name", "")


def test_p43_t14_play_uri_audiobook_sends_context_uri(isolated_db):
    tok = _insert_play_user(isolated_db)
    with patch("requests.put") as mock_put, patch("requests.get") as mock_get:
        mock_get.return_value.status_code = 204
        mock_put.return_value.status_code = 204
        r = client.post(
            "/events",
            json={
                "action": "play_uri:ab_uri",
                "state": {"ab_uri": "spotify:audiobook:ab123"},
                "session_id": 414,
                "app_token": tok
            },
            headers={"Authorization": "Bearer test_secret_token_123"}
        )
        assert r.status_code == 200
        assert mock_put.call_count == 1
        assert mock_put.call_args[1].get("json") == {"context_uri": "spotify:audiobook:ab123"}


def test_p43_t15_play_uri_chapter_sends_uris(isolated_db):
    tok = _insert_play_user(isolated_db)
    with patch("requests.put") as mock_put, patch("requests.get") as mock_get:
        mock_get.return_value.status_code = 204
        mock_put.return_value.status_code = 204
        r = client.post(
            "/events",
            json={
                "action": "play_uri:ch_uri",
                "state": {"ch_uri": "spotify:chapter:ch999"},
                "session_id": 415,
                "app_token": tok
            },
            headers={"Authorization": "Bearer test_secret_token_123"}
        )
        assert r.status_code == 200
        assert mock_put.call_count == 1
        assert mock_put.call_args[1].get("json") == {"uris": ["spotify:chapter:ch999"]}


def test_p43_t16_play_uri_unknown_type_sends_no_request_and_sets_player_error(isolated_db):
    tok = _insert_play_user(isolated_db)
    with patch("requests.put") as mock_put, patch("requests.get") as mock_get:
        mock_get.return_value.status_code = 204
        r = client.post(
            "/events",
            json={
                "action": "play_uri:bad_uri",
                "state": {"bad_uri": "spotify:nieznany:999"},
                "session_id": 416,
                "app_token": tok
            },
            headers={"Authorization": "Bearer test_secret_token_123"}
        )
        assert r.status_code == 200
        assert mock_put.call_count == 0
        data = r.json()
        assert data.get("player_error") != ""


def test_g3_429_during_poll_once_with_two_accounts(isolated_db):
    tok1 = _insert_play_user(isolated_db, tok="stoken_g3_1", user_id="u_g3_1")
    tok2 = _insert_play_user(isolated_db, tok="stoken_g3_2", user_id="u_g3_2")

    active_oxiterm_sessions[4191] = (tok1, time.time())
    active_oxiterm_sessions[4192] = (tok2, time.time())

    mock_resp_429 = MagicMock()
    mock_resp_429.status_code = 429
    mock_resp_429.headers = {"Retry-After": "4500"}

    with patch("requests.get") as mock_get, patch("requests.post") as mock_post:
        mock_get.return_value = mock_resp_429
        mock_post.return_value.status_code = 200
        mock_post.return_value.json.return_value = {"page": "spotify/panel.thtml"}

        asyncio.run(app_module.poll_once())

        # First account hit 429, setting global breaker. Second account was NOT fetched in same cycle.
        assert mock_get.call_count == 1
        assert len(active_oxiterm_sessions[4191]) == 2
        assert len(active_oxiterm_sessions[4192]) == 2


def test_31_vol_up_with_model_present(isolated_db):
    tok = _insert_play_user(isolated_db, tok="stoken_t31", user_id="u_t31")
    acc = app_module.poller_manager.accounts.get("u_t31")
    if not acc:
        app_module.poller_manager.register_session(310, "u_t31", "acc_t31", tok)
        acc = app_module.poller_manager.accounts["u_t31"]
    
    acc.model = app_module.parse_snapshot({
        "is_playing": True, "progress_ms": 10000,
        "device": {"volume_percent": 50},
        "item": {"type": "track", "uri": "t1", "name": "T1", "duration_ms": 180000}
    }, app_module.now_mono_ms())

    with patch("requests.get") as mock_get, patch("requests.put") as mock_put:
        mock_put.return_value.status_code = 200
        r = client.post(
            "/events",
            json={"action": "vol_up", "session_id": 310, "app_token": tok},
            headers={"Authorization": "Bearer test_secret_token_123"}
        )
        assert r.status_code == 200
        assert mock_get.call_count == 0
        assert mock_put.call_count == 1
        assert "volume?volume_percent=60" in mock_put.call_args[0][0]
        data = r.json()
        assert data.get("volume") == "60%"


def test_32_seek_fwd_with_model_present(isolated_db):
    tok = _insert_play_user(isolated_db, tok="stoken_t32", user_id="u_t32")
    app_module.poller_manager.register_session(320, "u_t32", "acc_t32", tok)
    acc = app_module.poller_manager.accounts["u_t32"]
    
    acc.model = app_module.parse_snapshot({
        "is_playing": True, "progress_ms": 10000,
        "item": {"type": "track", "uri": "t1", "name": "T1", "duration_ms": 180000}
    }, app_module.now_mono_ms())

    with patch("requests.get") as mock_get, patch("requests.put") as mock_put:
        mock_put.return_value.status_code = 200
        r = client.post(
            "/events",
            json={"action": "seek_fwd", "session_id": 320, "app_token": tok},
            headers={"Authorization": "Bearer test_secret_token_123"}
        )
        assert r.status_code == 200
        assert mock_get.call_count == 0
        assert mock_put.call_count == 1
        assert "seek?position_ms=" in mock_put.call_args[0][0]


def test_33_player_toggle_with_playing_model(isolated_db):
    tok = _insert_play_user(isolated_db, tok="stoken_t33", user_id="u_t33")
    app_module.poller_manager.register_session(330, "u_t33", "acc_t33", tok)
    acc = app_module.poller_manager.accounts["u_t33"]
    
    acc.model = app_module.parse_snapshot({
        "is_playing": True, "progress_ms": 10000,
        "item": {"type": "track", "uri": "t1", "name": "T1", "duration_ms": 180000}
    }, app_module.now_mono_ms())

    with patch("requests.get") as mock_get, patch("requests.put") as mock_put:
        mock_put.return_value.status_code = 200
        r = client.post(
            "/events",
            json={"action": "player_toggle", "session_id": 330, "app_token": tok},
            headers={"Authorization": "Bearer test_secret_token_123"}
        )
        assert r.status_code == 200
        assert mock_get.call_count == 0
        assert mock_put.call_count == 1
        assert "pause" in mock_put.call_args[0][0]
        data = r.json()
        assert data.get("is_playing") == "false"


def test_34_command_resets_ladder_no_inline_fetch(isolated_db):
    tok = _insert_play_user(isolated_db, tok="stoken_t34", user_id="u_t34")
    app_module.poller_manager.register_session(340, "u_t34", "acc_t34", tok)
    acc = app_module.poller_manager.accounts["u_t34"]
    acc.model = app_module.parse_snapshot({
        "is_playing": True, "progress_ms": 10000,
        "item": {"type": "track", "uri": "t1", "name": "T1", "duration_ms": 180000}
    }, app_module.now_mono_ms())
    acc.ladder_cursor = 5

    with patch("requests.get") as mock_get, patch("requests.post") as mock_post:
        mock_post.return_value.status_code = 200
        r = client.post(
            "/events",
            json={"action": "player_next", "session_id": 340, "app_token": tok},
            headers={"Authorization": "Bearer test_secret_token_123"}
        )
        assert r.status_code == 200
        assert mock_get.call_count == 0
        assert acc.ladder_cursor == 0


def test_35_set_tab_player_no_spotify_call(isolated_db):
    tok = _insert_play_user(isolated_db, tok="stoken_t35", user_id="u_t35")
    app_module.poller_manager.register_session(350, "u_t35", "acc_t35", tok)
    acc = app_module.poller_manager.accounts["u_t35"]
    acc.model = app_module.parse_snapshot({
        "is_playing": True, "progress_ms": 10000,
        "item": {"type": "track", "uri": "t1", "name": "Model Song", "duration_ms": 180000}
    }, app_module.now_mono_ms())

    with patch("requests.get") as mock_get:
        r = client.post(
            "/events",
            json={"action": "set:tab=player", "session_id": 350, "app_token": tok},
            headers={"Authorization": "Bearer test_secret_token_123"}
        )
        assert r.status_code == 200
        assert mock_get.call_count == 0
        data = r.json()
        assert data.get("track_name") == "Model Song"
        assert data.get("tab") == "player"


def test_37_budget_log_emitted_per_cycle(isolated_db, caplog):
    tok = _insert_play_user(isolated_db, tok="stoken_t37", user_id="u_t37")
    active_oxiterm_sessions[370] = (tok, time.time())

    with patch("requests.get") as mock_get, patch("requests.post") as mock_post, caplog.at_level("INFO"):
        mock_get.return_value.status_code = 200
        mock_get.return_value.json.return_value = {
            "is_playing": True, "progress_ms": 5000,
            "item": {"type": "track", "uri": "t1", "name": "T1", "duration_ms": 180000}
        }
        mock_post.return_value.status_code = 200
        mock_post.return_value.json.return_value = {"page": "spotify/panel.thtml"}

        asyncio.run(app_module.poll_once())
        assert any("Spotify API budget:" in record.message for record in caplog.records)


def test_f1_status_200_empty_body_returns_inactive_player(isolated_db):
    tok = _insert_play_user(isolated_db, tok="stoken_f1")
    with patch("requests.get") as mock_get:
        mock_resp = MagicMock()
        mock_resp.status_code = 200
        mock_resp.json.return_value = {}
        mock_get.return_value = mock_resp

        res = app_module.fetch_playback_for_user("acc_tok_f1")
        assert res.get("is_authenticated") == "true"
        assert res.get("track_name") == "Brak aktywnego odtwarzacza"
        assert res.get("player_error") == ""


def test_f2_logger_exception_called_on_playback_error(isolated_db):
    with patch("requests.get") as mock_get, patch.object(app_module.logger, "exception") as mock_log_exc:
        mock_resp = MagicMock()
        mock_resp.status_code = 200
        mock_resp.json.side_effect = ValueError("Corrupted JSON body")
        mock_get.return_value = mock_resp

        res = app_module.fetch_playback_for_user("acc_tok_f2")
        assert res.get("player_error") == "Błąd połączenia"
        assert mock_log_exc.called


def test_f3_null_fields_in_item_and_device_handled_safely(isolated_db):
    # Test track with null name, null artist name, null album name, null device name
    pb_track_nulls = {
        "is_playing": True,
        "currently_playing_type": "track",
        "device": {"name": None, "supports_volume": True},
        "item": {
            "type": "track",
            "name": None,
            "artists": [{"name": None}],
            "album": {"name": None},
            "duration_ms": 120000
        }
    }
    with patch("requests.get") as mock_get:
        mock_resp = MagicMock()
        mock_resp.status_code = 200
        mock_resp.json.return_value = pb_track_nulls
        mock_get.return_value = mock_resp

        res = app_module.fetch_playback_for_user("acc_tok_f3")
        assert res.get("track_name") == "Brak tytułu"
        assert res.get("artist_name") == "Nieznany wykonawca"
        assert res.get("album_name") == "Album"
        assert res.get("device_name") == "📱 Brak urządzenia"
        assert res.get("player_error") == ""

    # Test episode with null name, null show name, null release_date
    pb_ep_nulls = {
        "is_playing": True,
        "currently_playing_type": "episode",
        "item": {
            "type": "episode",
            "name": None,
            "show": {"name": None, "publisher": None},
            "release_date": None,
            "duration_ms": 180000
        }
    }
    with patch("requests.get") as mock_get:
        mock_resp = MagicMock()
        mock_resp.status_code = 200
        mock_resp.json.return_value = pb_ep_nulls
        mock_get.return_value = mock_resp

        res = app_module.fetch_playback_for_user("acc_tok_f3_ep")
        assert res.get("track_name") == "Brak tytułu"
        assert res.get("artist_name") == "Podcast"
        assert res.get("album_name") == ""
        assert res.get("player_error") == ""


def test_63_app_py_contains_no_api_spotify_com_outside_comment():
    app_py_path = os.path.join(os.path.dirname(__file__), "app.py")
    with open(app_py_path, "r", encoding="utf-8") as f:
        lines = f.readlines()
    for idx, line in enumerate(lines, 1):
        if "api.spotify.com" in line:
            assert line.strip().startswith("#"), f"api.spotify.com found at line {idx} in app.py: {line}"


def test_64_command_sets_player_info_retained_after_poll_cycle(isolated_db):
    tok = _insert_play_user(isolated_db, tok="stoken_t64", user_id="u_t64")
    app_module.poller_manager.register_session(640, "u_t64", "acc_t64", tok)
    acc = app_module.poller_manager.accounts["u_t64"]
    acc.model = app_module.parse_snapshot({
        "is_playing": True, "progress_ms": 10000,
        "item": {"type": "track", "uri": "t1", "name": "T1", "duration_ms": 180000}
    }, app_module.now_mono_ms())

    active_oxiterm_sessions[640] = (tok, time.time())

    with patch("requests.get") as mock_get, patch("requests.post") as mock_post, patch("requests.put") as mock_put:
        mock_put.return_value.status_code = 403
        mock_get.return_value.status_code = 200
        mock_get.return_value.json.return_value = {
            "is_playing": True, "progress_ms": 12000,
            "item": {"type": "track", "uri": "t1", "name": "T1", "duration_ms": 180000}
        }
        mock_post.return_value.status_code = 200

        # Command sets player_error on 403
        r = client.post(
            "/events",
            json={"action": "player_toggle", "session_id": 640, "app_token": tok},
            headers={"Authorization": "Bearer test_secret_token_123"}
        )
        assert r.status_code == 200
        cmd_error = r.json().get("player_error")
        assert cmd_error == "Błąd Spotify (403)"

        # One poll_once runs immediately afterwards -> pushed patch MUST carry the exact same player_error
        mock_post.reset_mock()
        asyncio.run(app_module.poll_once())
        assert mock_post.call_count >= 1
        pushed_json = mock_post.call_args[1].get("json")
        assert pushed_json.get("player_error") == "Błąd Spotify (403)"

        # Advance clock past PENDING_MSG_TTL_S (5.0s) -> next poll_once carries empty player_error
        start_mono = app_module.now_mono_ms()
        app_module.poller_manager._clock_fn = lambda: start_mono + 6000
        mock_post.reset_mock()
        asyncio.run(app_module.poll_once())
        assert mock_post.call_count >= 1
        pushed_json_after = mock_post.call_args[1].get("json")
        assert pushed_json_after.get("player_error") == ""









