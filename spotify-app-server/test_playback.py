import pytest
from clock import FakeClock
from playback import parse_snapshot, extrapolate, next_deadline, next_second_boundary_mono_ms, PollGaps
from render import full_patch, tick_patch, format_time, render_progress_bar

gaps = PollGaps(
    playing_s=25.0,
    paused_s=60.0,
    idle_s=90.0,
    end_grace_ms=800,
    min_gap_s=1.0,
    ladder_s=(0.4, 1.0, 2.0, 3.0, 5.0, 8.0, 12.0, 20.0)
)

def test_1_playing_extrapolation():
    clock = FakeClock(mono_ms=100000)
    body = {
        "is_playing": True,
        "progress_ms": 10000,
        "item": {"type": "track", "uri": "spotify:track:t1", "name": "T1", "duration_ms": 180000}
    }
    snap = parse_snapshot(body, clock.now_mono_ms())
    clock.advance_mono(5000)
    assert extrapolate(snap, clock.now_mono_ms()) == 15000

def test_2_paused_extrapolation():
    clock = FakeClock(mono_ms=100000)
    body = {
        "is_playing": False,
        "progress_ms": 10000,
        "item": {"type": "track", "uri": "spotify:track:t1", "name": "T1", "duration_ms": 180000}
    }
    snap = parse_snapshot(body, clock.now_mono_ms())
    clock.advance_mono(5000)
    assert extrapolate(snap, clock.now_mono_ms()) == 10000

def test_3_extrapolation_clamped_to_duration():
    clock = FakeClock(mono_ms=100000)
    body = {
        "is_playing": True,
        "progress_ms": 175000,
        "item": {"type": "track", "uri": "spotify:track:t1", "name": "T1", "duration_ms": 180000}
    }
    snap = parse_snapshot(body, clock.now_mono_ms())
    clock.advance_mono(10000)
    assert extrapolate(snap, clock.now_mono_ms()) == 180000

def test_4_identical_timestamp_and_uri():
    clock = FakeClock(mono_ms=100000)
    body1 = {
        "timestamp": 1600000000000,
        "is_playing": True,
        "progress_ms": 10000,
        "item": {"type": "track", "uri": "spotify:track:t1", "name": "T1", "duration_ms": 180000}
    }
    body2 = {
        "timestamp": 1600000000000,
        "is_playing": True,
        "progress_ms": 15000,
        "item": {"type": "track", "uri": "spotify:track:t1", "name": "T1", "duration_ms": 180000}
    }
    snap1 = parse_snapshot(body1, clock.now_mono_ms())
    snap2 = parse_snapshot(body2, clock.now_mono_ms() + 5000)
    assert (snap1.spotify_timestamp, snap1.item_uri) == (snap2.spotify_timestamp, snap2.item_uri)

def test_5_different_timestamp_resets_base():
    clock = FakeClock(mono_ms=100000)
    body1 = {
        "timestamp": 1600000000000,
        "is_playing": True,
        "progress_ms": 10000,
        "item": {"type": "track", "uri": "spotify:track:t1", "name": "T1", "duration_ms": 180000}
    }
    snap1 = parse_snapshot(body1, clock.now_mono_ms())
    
    clock.advance_mono(5000)
    body2 = {
        "timestamp": 1600000005000,
        "is_playing": True,
        "progress_ms": 15000,
        "item": {"type": "track", "uri": "spotify:track:t1", "name": "T1", "duration_ms": 180000}
    }
    snap2 = parse_snapshot(body2, clock.now_mono_ms())
    assert snap2.progress_ms == 15000
    assert snap2.base_mono_ms == 105000

def test_6_end_of_track_deadline():
    clock = FakeClock(mono_ms=100000)
    body = {
        "is_playing": True,
        "progress_ms": 178800,
        "item": {"type": "track", "uri": "spotify:track:t1", "name": "T1", "duration_ms": 180000}
    }
    snap = parse_snapshot(body, clock.now_mono_ms())
    deadline, cursor = next_deadline(snap, clock.now_mono_ms(), None, gaps)
    # Remaining: 1200 ms + 800 ms end_grace = 2000 ms.
    # min_gap_s is 1.0s (1000ms), max(2000, 1000) = 2000
    assert deadline == clock.now_mono_ms() + 2000
    assert cursor is None

def test_7_playing_base_deadline():
    clock = FakeClock(mono_ms=100000)
    body = {
        "is_playing": True,
        "progress_ms": 10000,
        "item": {"type": "track", "uri": "spotify:track:t1", "name": "T1", "duration_ms": 180000}
    }
    snap = parse_snapshot(body, clock.now_mono_ms())
    deadline, cursor = next_deadline(snap, clock.now_mono_ms(), None, gaps)
    assert deadline == clock.now_mono_ms() + 25000
    assert cursor is None

def test_8_paused_deadline():
    clock = FakeClock(mono_ms=100000)
    body = {
        "is_playing": False,
        "progress_ms": 10000,
        "item": {"type": "track", "uri": "spotify:track:t1", "name": "T1", "duration_ms": 180000}
    }
    snap = parse_snapshot(body, clock.now_mono_ms())
    deadline, cursor = next_deadline(snap, clock.now_mono_ms(), None, gaps)
    assert deadline == clock.now_mono_ms() + 60000
    assert cursor is None

def test_9_status_204_idle_deadline():
    clock = FakeClock(mono_ms=100000)
    snap = parse_snapshot(None, clock.now_mono_ms(), status_code=204)
    assert snap.poll_state == "IDLE"
    deadline, cursor = next_deadline(snap, clock.now_mono_ms(), None, gaps)
    assert deadline == clock.now_mono_ms() + 90000
    assert cursor is None

def test_10_ad_unsupported_idle_deadline():
    clock = FakeClock(mono_ms=100000)
    body = {
        "is_playing": True,
        "currently_playing_type": "ad",
        "item": None
    }
    snap = parse_snapshot(body, clock.now_mono_ms())
    assert snap.poll_state == "UNSUPPORTED"
    deadline, cursor = next_deadline(snap, clock.now_mono_ms(), None, gaps)
    assert deadline == clock.now_mono_ms() + 90000
    assert cursor is None

def test_11_null_duration_handling():
    clock = FakeClock(mono_ms=100000)
    body = {
        "is_playing": True,
        "progress_ms": 5000,
        "item": {"type": "track", "uri": "spotify:track:t1", "name": "T1", "duration_ms": None}
    }
    snap = parse_snapshot(body, clock.now_mono_ms())
    assert extrapolate(snap, clock.now_mono_ms()) == 5000
    deadline, cursor = next_deadline(snap, clock.now_mono_ms(), None, gaps)
    assert deadline == clock.now_mono_ms() + 90000

def test_12_deadline_elapsed_exhausted_cursor_uses_base():
    clock = FakeClock(mono_ms=100000)
    body = {
        "is_playing": True,
        "progress_ms": 10000,
        "item": {"type": "track", "uri": "spotify:track:t1", "name": "T1", "duration_ms": 180000}
    }
    snap = parse_snapshot(body, clock.now_mono_ms())
    deadline, cursor = next_deadline(snap, clock.now_mono_ms(), None, gaps)
    assert deadline > clock.now_mono_ms()
    assert deadline == clock.now_mono_ms() + 25000

def test_13_hours_formatting():
    assert format_time(5194000) == "1:26:34"

def test_14_minutes_formatting():
    assert format_time(180000) == "03:00"

def test_15_nested_disallows_skipping_next():
    clock = FakeClock(mono_ms=100000)
    body = {
        "actions": {"disallows": {"skipping_next": True}},
        "item": {"type": "track", "uri": "t1", "name": "T1", "duration_ms": 180000}
    }
    snap = parse_snapshot(body, clock.now_mono_ms())
    assert snap.can_next is False
    patch = full_patch(snap, clock.now_mono_ms())
    assert patch["can_next"] == "false"

def test_16_flat_disallows_skipping_next():
    clock = FakeClock(mono_ms=100000)
    body = {
        "actions": {"skipping_next": True},
        "item": {"type": "track", "uri": "t1", "name": "T1", "duration_ms": 180000}
    }
    snap = parse_snapshot(body, clock.now_mono_ms())
    assert snap.can_next is False
    patch = full_patch(snap, clock.now_mono_ms())
    assert patch["can_next"] == "false"

def test_17_device_restricted_overrides_actions():
    clock = FakeClock(mono_ms=100000)
    body = {
        "device": {"name": "Smart TV", "is_restricted": True},
        "actions": {"skipping_next": False},
        "item": {"type": "track", "uri": "t1", "name": "T1", "duration_ms": 180000}
    }
    snap = parse_snapshot(body, clock.now_mono_ms())
    assert snap.can_next is False
    assert snap.can_prev is False
    assert snap.can_seek is False
    assert snap.can_volume is False
    assert snap.device_restricted is True
    patch = full_patch(snap, clock.now_mono_ms())
    assert patch["device_restricted"] == "true"
    assert patch["can_next"] == "false"

def test_48_next_second_boundary():
    clock = FakeClock(mono_ms=100000)
    body = {
        "is_playing": True,
        "progress_ms": 10400,
        "item": {"type": "track", "uri": "t1", "name": "T1", "duration_ms": 180000}
    }
    snap = parse_snapshot(body, clock.now_mono_ms())
    next_boundary = next_second_boundary_mono_ms(snap, clock.now_mono_ms())
    # 10400 ms -> next second is 11000 ms -> 600 ms ahead.
    assert next_boundary == clock.now_mono_ms() + 600

def test_49_phase_locked_tick_simulation():
    clock = FakeClock(mono_ms=100000)
    body = {
        "is_playing": True,
        "progress_ms": 0,
        "item": {"type": "track", "uri": "t1", "name": "T1", "duration_ms": 180000}
    }
    snap = parse_snapshot(body, clock.now_mono_ms())
    
    pushed_strings = []
    for _ in range(180):
        next_boundary = next_second_boundary_mono_ms(snap, clock.now_mono_ms())
        assert next_boundary is not None
        clock._mono_ms = next_boundary
        t_patch = tick_patch(snap, clock.now_mono_ms())
        pushed_strings.append(t_patch["progress_bar"])
    
    expected_strings = []
    for sec in range(1, 181):
        ratio = min(max((sec * 1000) / 180000, 0.0), 1.0)
        filled = int(ratio * 8)
        bar = "=" * filled + "-" * (8 - filled)
        mins = sec // 60
        secs = sec % 60
        expected_strings.append(f"[{bar}] {mins:02d}:{secs:02d} / 03:00")

    assert pushed_strings == expected_strings

def test_50_fetch_replacing_model_mid_second():
    clock = FakeClock(mono_ms=100000)
    body1 = {
        "is_playing": True,
        "progress_ms": 10200,
        "item": {"type": "track", "uri": "t1", "name": "T1", "duration_ms": 180000}
    }
    snap1 = parse_snapshot(body1, clock.now_mono_ms())
    b1 = next_second_boundary_mono_ms(snap1, clock.now_mono_ms())
    assert b1 == 100800

    clock.advance_mono(300)
    body2 = {
        "is_playing": True,
        "progress_ms": 10500,
        "item": {"type": "track", "uri": "t1", "name": "T1", "duration_ms": 180000}
    }
    snap2 = parse_snapshot(body2, clock.now_mono_ms())
    b2 = next_second_boundary_mono_ms(snap2, clock.now_mono_ms())
    assert b2 == 100800

def test_51_paused_model_second_boundary():
    clock = FakeClock(mono_ms=100000)
    body = {
        "is_playing": False,
        "progress_ms": 10400,
        "item": {"type": "track", "uri": "t1", "name": "T1", "duration_ms": 180000}
    }
    snap = parse_snapshot(body, clock.now_mono_ms())
    assert next_second_boundary_mono_ms(snap, clock.now_mono_ms()) is None
