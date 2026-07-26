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

def full_patch(snapshot: Snapshot, now_mono_ms: int) -> Dict[str, str]:
    current_progress = extrapolate(snapshot, now_mono_ms)
    
    if snapshot.device_restricted:
        player_info = "urządzenie nie przyjmuje poleceń"
    elif snapshot.kind == "unsupported":
        if snapshot.title == "Reklama":
            player_info = "reklama"
        else:
            player_info = "typ treści nieobsługiwany przez API Spotify"
    elif snapshot.poll_state == "IDLE" and snapshot.title == "Brak aktywnego odtwarzacza":
        player_info = "brak aktywnego urządzenia — dotknij telefonu"
    else:
        player_info = ""

    return {
        "is_authenticated": "true",
        "track_name": str(snapshot.title)[:35],
        "artist_name": str(snapshot.subtitle)[:35],
        "album_name": str(snapshot.tertiary)[:35],
        "device_name": f"📱 {str(snapshot.device_name)[:35]}",
        "is_playing": "true" if snapshot.is_playing else "false",
        "play_icon": "❚❚ Pause" if snapshot.is_playing else "Play",
        "progress_bar": render_progress_bar(current_progress, snapshot.duration_ms),
        "volume": f"{snapshot.volume}%",
        "can_next": "true" if snapshot.can_next else "false",
        "can_prev": "true" if snapshot.can_prev else "false",
        "can_seek": "true" if snapshot.can_seek else "false",
        "can_volume": "true" if snapshot.can_volume else "false",
        "device_restricted": "true" if snapshot.device_restricted else "false",
        "player_info": player_info,
        "player_error": ""
    }

def tick_patch(snapshot: Snapshot, now_mono_ms: int) -> Dict[str, str]:
    current_progress = extrapolate(snapshot, now_mono_ms)
    return {
        "progress_bar": render_progress_bar(current_progress, snapshot.duration_ms)
    }
