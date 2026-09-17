from typing import Dict, Optional
from playback import Snapshot, extrapolate

def format_time(ms: int) -> str:
    total_sec = max(0, ms // 1000)
    hours = total_sec // 3600
    mins = (total_sec % 3600) // 60
    secs = total_sec % 60
    if hours > 0:
        return f"{hours}:{mins:02d}:{secs:02d}"
    return f"{mins:02d}:{secs:02d}"

def render_progress_bar(progress_ms: int, duration_ms: Optional[int]) -> str:
    if duration_ms is None or duration_ms <= 0:
        return f"[--------] {format_time(progress_ms)}"
    ratio = min(max(progress_ms / duration_ms, 0.0), 1.0)
    filled = int(ratio * 8)
    bar = "=" * filled + "-" * (8 - filled)
    return f"[{bar}] {format_time(progress_ms)} / {format_time(duration_ms)}"

def calc_progress_metrics(current_progress: int, duration_ms: Optional[int]) -> Dict[str, str]:
    if duration_ms is None or duration_ms <= 0:
        return {
            "progress_cols": "0",
            "progress_cols_mob": "0",
            "progress_cur": format_time(current_progress),
            "progress_dur": "00:00"
        }
    ratio = min(max(current_progress / duration_ms, 0.0), 1.0)
    # 62 cols for desktop progress track, 32 cols for mobile
    progress_cols = int(round(ratio * 62))
    progress_cols_mob = int(round(ratio * 32))
    return {
        "progress_cols": str(progress_cols),
        "progress_cols_mob": str(progress_cols_mob),
        "progress_cur": format_time(current_progress),
        "progress_dur": format_time(duration_ms)
    }

def full_patch(snapshot: Snapshot, now_mono_ms: int, player_error: str = "", player_info: str = "") -> Dict[str, str]:
    current_progress = extrapolate(snapshot, now_mono_ms)
    
    if player_info:
        p_info = player_info
    elif snapshot.device_restricted:
        p_info = "urządzenie nie przyjmuje poleceń"
    elif snapshot.kind == "unsupported":
        if snapshot.title == "Reklama":
            p_info = "reklama"
        else:
            p_info = "typ treści nieobsługiwany przez API Spotify"
    elif snapshot.poll_state == "IDLE" and snapshot.title == "Brak aktywnego odtwarzacza":
        p_info = "brak aktywnego urządzenia — dotknij telefonu"
    else:
        p_info = ""

    p_error = player_error if player_error else ""
    prog_metrics = calc_progress_metrics(current_progress, snapshot.duration_ms)
    vol_cols = int(round((max(0, min(100, snapshot.volume)) / 100.0) * 26))

    patch = {
        "is_authenticated": "true",
        "track_name": str(snapshot.title)[:35],
        "artist_name": str(snapshot.subtitle)[:35],
        "album_name": str(snapshot.tertiary)[:35],
        "device_name": f"📱 {str(snapshot.device_name)[:35]}",
        "is_playing": "true" if snapshot.is_playing else "false",
        "play_icon": "❚❚ Pause" if snapshot.is_playing else "Play",
        "progress_bar": render_progress_bar(current_progress, snapshot.duration_ms),
        "volume": f"{snapshot.volume}%",
        "vol_cols": str(vol_cols),
        "can_next": "true" if snapshot.can_next else "false",
        "can_prev": "true" if snapshot.can_prev else "false",
        "can_seek": "true" if snapshot.can_seek else "false",
        "can_volume": "true" if snapshot.can_volume else "false",
        "device_restricted": "true" if snapshot.device_restricted else "false",
        "player_info": p_info,
        "player_error": p_error
    }
    patch.update(prog_metrics)
    return patch

def tick_patch(snapshot: Snapshot, now_mono_ms: int) -> Dict[str, str]:
    current_progress = extrapolate(snapshot, now_mono_ms)
    prog_metrics = calc_progress_metrics(current_progress, snapshot.duration_ms)
    patch = {
        "progress_bar": render_progress_bar(current_progress, snapshot.duration_ms)
    }
    patch.update(prog_metrics)
    return patch
