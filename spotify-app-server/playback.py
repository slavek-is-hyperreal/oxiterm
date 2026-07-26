from dataclasses import dataclass, field
from typing import Optional, Dict, Tuple, Any, List

@dataclass(frozen=True)
class PollGaps:
    playing_s: float = 25.0
    paused_s: float = 60.0
    idle_s: float = 90.0
    end_grace_ms: int = 800
    min_gap_s: float = 1.0
    ladder_s: Tuple[float, ...] = (0.4, 1.0, 2.0, 3.0, 5.0, 8.0, 12.0, 20.0)

@dataclass
class Snapshot:
    item_uri: Optional[str]
    kind: str  # "track", "episode", "unsupported", "none"
    title: str
    subtitle: str
    tertiary: str
    duration_ms: Optional[int]
    progress_ms: int
    is_playing: bool
    spotify_timestamp: Optional[int]
    device_name: str
    volume: int
    can_next: bool
    can_prev: bool
    can_seek: bool
    can_volume: bool
    device_restricted: bool
    poll_state: str  # "PLAYING", "PAUSED", "IDLE", "UNSUPPORTED", "BLOCKED"
    base_mono_ms: int

def _get_dict(val: Any) -> Optional[Any]:
    if val is not None and (isinstance(val, dict) or hasattr(val, "get")):
        return val
    return None

def parse_snapshot(body: Optional[Any], now_mono_ms: int, status_code: int = 200) -> Snapshot:
    if status_code == 204 or not body:
        return Snapshot(
            item_uri=None,
            kind="none",
            title="Brak aktywnego odtwarzacza",
            subtitle="Włącz muzykę na telefonie/PC",
            tertiary="Spotify Connect",
            duration_ms=None,
            progress_ms=0,
            is_playing=False,
            spotify_timestamp=None,
            device_name="Brak aktywnego urządzenia",
            volume=0,
            can_next=True,
            can_prev=True,
            can_seek=True,
            can_volume=True,
            device_restricted=False,
            poll_state="IDLE",
            base_mono_ms=now_mono_ms
        )

    # Device resolution
    raw_device = body.get("device") if hasattr(body, "get") else None
    device_obj = _get_dict(raw_device) or {}
    device_name = (device_obj.get("name") if hasattr(device_obj, "get") else None) or "Brak urządzenia"
    is_restricted = (device_obj.get("is_restricted") is True) if hasattr(device_obj, "get") else False
    supports_vol = (device_obj.get("supports_volume") is not False) if hasattr(device_obj, "get") else True
    raw_vol = device_obj.get("volume_percent") if hasattr(device_obj, "get") else None
    volume = raw_vol if isinstance(raw_vol, int) else 50

    # Actions resolution (DisallowsObject polarity: true = forbidden)
    raw_actions = body.get("actions") if hasattr(body, "get") else None
    actions = _get_dict(raw_actions) or {}
    raw_disallows = actions.get("disallows") if hasattr(actions, "get") else None
    disallows_obj = _get_dict(raw_disallows)
    src = disallows_obj if disallows_obj is not None else actions

    if src and hasattr(src, "get"):
        can_next = not bool(src.get("skipping_next", False))
        can_prev = not bool(src.get("skipping_prev", False))
        can_seek = not bool(src.get("seeking", False))
    else:
        can_next = True
        can_prev = True
        can_seek = True

    if is_restricted:
        can_next = False
        can_prev = False
        can_seek = False
        can_volume = False
        device_restricted = True
        poll_state = "BLOCKED"
    else:
        device_restricted = False
        can_volume = supports_vol
        poll_state = "IDLE"

    raw_item = body.get("item") if hasattr(body, "get") else None
    item = _get_dict(raw_item)
    item_type = item.get("type") if item and hasattr(item, "get") else None
    curr_type = body.get("currently_playing_type") if hasattr(body, "get") else None
    is_playing = bool(body.get("is_playing", False)) if hasattr(body, "get") else False

    if item and item_type == "episode":
        kind = "episode"
        item_uri = item.get("uri")
        title = item.get("name") or "Brak tytułu"
        show_obj = _get_dict(item.get("show")) or {}
        subtitle = (show_obj.get("name") if hasattr(show_obj, "get") else None) or "Podcast"
        tertiary = item.get("release_date") or ""
        raw_dur = item.get("duration_ms")
        duration_ms = raw_dur if isinstance(raw_dur, int) else None
        if not is_restricted:
            poll_state = "PLAYING" if is_playing else "PAUSED"
    elif item and (item_type == "track" or item_type is None):
        kind = "track"
        item_uri = item.get("uri")
        title = item.get("name") or "Brak tytułu"
        raw_artists = item.get("artists") if hasattr(item, "get") else []
        artists_list = []
        if isinstance(raw_artists, list):
            for a in raw_artists:
                if isinstance(a, dict) or hasattr(a, "get"):
                    aname = a.get("name") if hasattr(a, "get") else None
                    if aname is not None:
                        artists_list.append(str(aname))
        subtitle = ", ".join(artists_list) or "Nieznany wykonawca"
        album_obj = _get_dict(item.get("album")) or {}
        tertiary = (album_obj.get("name") if hasattr(album_obj, "get") else None) or "Album"
        raw_dur = item.get("duration_ms")
        duration_ms = raw_dur if isinstance(raw_dur, int) else None
        if not is_restricted:
            poll_state = "PLAYING" if is_playing else "PAUSED"
    elif item:
        kind = "unsupported"
        item_uri = item.get("uri")
        title = "Treść nieobsługiwana"
        subtitle = "-"
        tertiary = "-"
        duration_ms = None
        if not is_restricted:
            poll_state = "UNSUPPORTED"
    else:  # item is null
        item_uri = None
        duration_ms = None
        if curr_type == "ad":
            kind = "unsupported"
            title = "Reklama"
            subtitle = "-"
            tertiary = "-"
            if not is_restricted:
                poll_state = "UNSUPPORTED"
        elif curr_type and curr_type not in ("track", "episode"):
            kind = "unsupported"
            title = "Nieobsługiwany typ"
            subtitle = "-"
            tertiary = "-"
            if not is_restricted:
                poll_state = "UNSUPPORTED"
        else:
            kind = "none"
            title = "Brak odtwarzania"
            subtitle = "-"
            tertiary = "-"
            if not is_restricted:
                poll_state = "IDLE"

    raw_prog = body.get("progress_ms", 0) if hasattr(body, "get") else 0
    progress_ms = raw_prog if isinstance(raw_prog, int) else 0
    raw_ts = body.get("timestamp") if hasattr(body, "get") else None
    spotify_timestamp = raw_ts if isinstance(raw_ts, int) else None

    return Snapshot(
        item_uri=item_uri,
        kind=kind,
        title=title,
        subtitle=subtitle,
        tertiary=tertiary,
        duration_ms=duration_ms,
        progress_ms=progress_ms,
        is_playing=is_playing,
        spotify_timestamp=spotify_timestamp,
        device_name=device_name,
        volume=volume,
        can_next=can_next,
        can_prev=can_prev,
        can_seek=can_seek,
        can_volume=can_volume,
        device_restricted=device_restricted,
        poll_state=poll_state,
        base_mono_ms=now_mono_ms
    )

def extrapolate(snapshot: Snapshot, now_mono_ms: int) -> int:
    if not snapshot.is_playing or snapshot.duration_ms is None:
        if snapshot.duration_ms is not None:
            return min(snapshot.progress_ms, snapshot.duration_ms)
        return snapshot.progress_ms
    elapsed = max(0, now_mono_ms - snapshot.base_mono_ms)
    return min(snapshot.progress_ms + elapsed, snapshot.duration_ms)

def next_second_boundary_mono_ms(snapshot: Snapshot, now_mono_ms: int) -> Optional[int]:
    if not snapshot.is_playing or snapshot.duration_ms is None:
        return None
    p = extrapolate(snapshot, now_mono_ms)
    if p >= snapshot.duration_ms:
        return None
    next_sec_p = ((p // 1000) + 1) * 1000
    if next_sec_p > snapshot.duration_ms:
        return None
    ms_until = next_sec_p - p
    return now_mono_ms + ms_until

def next_deadline(snapshot: Snapshot, now_mono_ms: int, ladder_cursor: Optional[int], gaps: PollGaps) -> Tuple[int, Optional[int]]:
    if ladder_cursor is not None and ladder_cursor < len(gaps.ladder_s):
        gap_s = gaps.ladder_s[ladder_cursor]
        effective_gap_s = max(gap_s, gaps.min_gap_s)
        next_cursor = ladder_cursor + 1
        if next_cursor >= len(gaps.ladder_s):
            next_cursor = None
        return (now_mono_ms + int(effective_gap_s * 1000), next_cursor)

    next_cursor = None
    if snapshot.is_playing and snapshot.duration_ms is not None:
        remaining_ms = max(0, snapshot.duration_ms - extrapolate(snapshot, now_mono_ms))
        end_of_track_deadline = now_mono_ms + remaining_ms + gaps.end_grace_ms
        base_deadline = now_mono_ms + int(gaps.playing_s * 1000)
        deadline_ms = min(base_deadline, end_of_track_deadline)
        min_deadline = now_mono_ms + int(gaps.min_gap_s * 1000)
        return (max(deadline_ms, min_deadline), None)
    elif snapshot.poll_state == "PAUSED":
        return (now_mono_ms + int(gaps.paused_s * 1000), None)
    else:
        return (now_mono_ms + int(gaps.idle_s * 1000), None)
