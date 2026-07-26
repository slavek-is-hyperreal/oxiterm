import time

def now_mono_ms() -> int:
    return int(time.monotonic() * 1000)

def now_wall_ms() -> int:
    return int(time.time() * 1000)

class Clock:
    def now_mono_ms(self) -> int:
        return now_mono_ms()

    def now_wall_ms(self) -> int:
        return now_wall_ms()

class FakeClock(Clock):
    def __init__(self, mono_ms: int = 100000, wall_ms: int = 1700000000000):
        self._mono_ms = mono_ms
        self._wall_ms = wall_ms

    def now_mono_ms(self) -> int:
        return self._mono_ms

    def now_wall_ms(self) -> int:
        return self._wall_ms

    def advance_mono(self, ms: int):
        self._mono_ms += ms
        self._wall_ms += ms
