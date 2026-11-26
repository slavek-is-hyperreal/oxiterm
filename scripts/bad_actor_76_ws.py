#!/usr/bin/env python3
"""BAD ACTOR 7.6: probe the web transport over a raw WebSocket (127.0.0.1 only).
usage: bad_actor_76_ws.py PORT   -> prints, per `page=` value, the page announce (0x30) and the media messages (0x20/0x21)."""
import asyncio, sys, struct, websockets

async def probe(port, query, w=100, h=40, wait=2.5):
    url = f"ws://127.0.0.1:{port}/ws{query}"
    out = {"0x30": None, "0x20": [], "0x21": 0, "err": None}
    try:
        async with websockets.connect(url, max_size=64 * 1024 * 1024, open_timeout=5, close_timeout=1) as ws:
            await ws.send(bytes([0x10]) + struct.pack("<HH", w, h))
            end = asyncio.get_event_loop().time() + wait
            while True:
                left = end - asyncio.get_event_loop().time()
                if left <= 0:
                    break
                try:
                    m = await asyncio.wait_for(ws.recv(), left)
                except asyncio.TimeoutError:
                    break
                if isinstance(m, bytes) and m:
                    if m[0] == 0x30:
                        out["0x30"] = m[1:].decode(errors="replace")
                    elif m[0] == 0x20:
                        out["0x20"].append((struct.unpack("<I", m[1:5])[0], len(m) - 5))
                    elif m[0] == 0x21 and len(m) > 1:
                        out["0x21"] += 1
    except Exception as e:
        out["err"] = f"{type(e).__name__}: {e}"[:120]
    return out

async def main(port):
    for q in sys.argv[2:] or ["", "?page=demos/media.thtml", "?page=demos%2Fmedia.thtml", "?page=demos%2fmedia.thtml", "?page=%64emos/media.thtml",
              "?page=../README.md", "?page=..%2fREADME.md", "?page=%2e%2e/README.md", "?page=/etc/passwd", "?page=" + "a/" * 5000 + "x.thtml"]:
        r = await probe(port, q)
        print(f"{q[:48]:50} announce={r['0x30']!r:28} blobs={r['0x20']} coords={r['0x21']} err={r['err']}", flush=True)

asyncio.run(main(int(sys.argv[1])))
