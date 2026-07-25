import os
import time
import asyncio
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

def _insert_play_user(db_path, tok="stoken_play_test"):
    with sqlite3.connect(db_path) as conn:
        conn.execute("""
            INSERT OR REPLACE INTO users
                (spotify_user_id, display_name, access_token, refresh_token, expires_at, session_token, last_seen)
            VALUES ('user_play', 'User Play', 'acc_play', 'ref_play', 9999999999, ?, 9999999999)
        """, (tok,))
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
        assert patch_data.get("album_name") == "Tech Media Network"[:35]


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






