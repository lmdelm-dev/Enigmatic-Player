import asyncio
from pathlib import Path

import pytest

from enigmatic_player.app import EnigmaticApp
from enigmatic_player.config import Config
from enigmatic_player.core.queue import Queue
from enigmatic_player.core.track import Source, Track


def _app(tmp_path, **kw) -> EnigmaticApp:
    config = Config(cfg_path=tmp_path / "config.json", data_path=tmp_path / "state.json")
    config.add_library_dir(str(tmp_path))
    return EnigmaticApp(config=config, **kw)


def run(coro):
    return asyncio.run(coro)


def test_queue_basics():
    q = Queue()
    q.set_items(
        [Track("a", uri="1"), Track("b", uri="2"), Track("c", uri="3")], start=0
    )
    assert q.current.title == "a"
    assert q.next().title == "b"
    assert q.prev().title == "a"
    assert q.next().title == "b"
    q.jump(2)
    assert q.current.title == "c"
    assert q.next() is None  # no repeat, queue ends


def test_queue_repeat_wraps():
    q = Queue()
    q.set_items([Track("a", uri="1"), Track("b", uri="2")], start=1)
    q.repeat = True
    assert q.current.title == "b"
    assert q.next().title == "a"
    assert q.next().title == "b"


def test_queue_shuffle_restart():
    q = Queue()
    q.set_items([Track("a", uri="1"), Track("b", uri="2"), Track("c", uri="3")], start=0)
    q.shuffle = True
    nxt = q.next()
    assert nxt.title in {"b", "c"}
    assert q.current.title != "a"


def test_track_key_and_duration():
    t = Track("song", artist="peach", uri="x", provider=Source.LOCAL)
    assert t.key == "local:x"
    from enigmatic_player.core.track import fmt_time

    assert fmt_time(0) == "0:00"
    assert fmt_time(65) == "1:05"
    assert fmt_time(3661) == "1:01:01"


def test_config_roundtrip(tmp_path):
    cfg = Config(cfg_path=tmp_path / "cfg.json", data_path=tmp_path / "state.json")
    cfg.add_library_dir("~/Music")
    cfg.set("default_provider", "youtube")
    cfg.save()

    loaded = Config(cfg_path=tmp_path / "cfg.json", data_path=tmp_path / "state.json")
    assert loaded.library_dirs == [str(Path.home() / "Music")]
    assert loaded.all["default_provider"] == "youtube"


def test_local_scan(tmp_path):
    import wave

    wav = tmp_path / "tone.wav"
    with wave.open(str(wav), "wb") as w:
        w.setnchannels(1)
        w.setsampwidth(2)
        w.setframerate(8000)
        frame = b"\x00\x00" * 100
        w.writeframes(frame)
    cfg = Config(cfg_path=tmp_path / "config.json", data_path=tmp_path / "state.json")
    cfg.add_library_dir(str(tmp_path))
    from enigmatic_player.providers.local import LocalProvider

    prov = LocalProvider(cfg)
    tracks = prov.scan(force=True)
    assert any(t.title == "tone" for t in tracks)
    assert all(prov.resolve_stream(t) is not None for t in tracks)


def test_app_smoke(tmp_path):
    # The TUI should boot headless without raising, even if mpv is missing.
    async def _boot():
        app = _app(tmp_path)
        async with app.run_test() as pilot:
            await pilot.pause()
            await pilot.pause()
            # keep a hard-coded timeout so the suite never hangs
            assert app._source is Source.LOCAL
        return True

    assert run(_boot())


def test_app_play_pause_with_mpv(tmp_path):
    async def _boot():
        app = _app(tmp_path)
        async with app.run_test() as pilot:
            await pilot.pause()
            if not app.player:
                pytest.skip("mpv not available")
            app.action_play_pause()
            await pilot.pause()
        return True

    assert run(_boot())


# ---------------------------------------------------------------- youtube headers
class _FakeResolver:
    """Stands in for yt_dlp.YoutubeDL; returns canned extraction info."""

    info = {
        "format_id": "251",
        "url": "https://media.example/audio.webm",
        "http_headers": {"User-Agent": "generic/1.0"},
        "formats": [
            {"format_id": "140", "url": "https://media.example/m4a"},
            {
                "format_id": "251",
                "url": "https://media.example/audio.webm",
                # the client-matched UA that googlevideo now enforces
                "http_headers": {"User-Agent": "com.google.android.apps.youtube.vr/1.0"},
            },
        ],
    }

    def __init__(self, opts):
        self.opts = opts

    def extract_info(self, url, download=False):
        assert download is False
        return self.info


def test_youtube_resolve_returns_format_headers(monkeypatch):
    import yt_dlp

    from enigmatic_player.core.track import Source, Track
    from enigmatic_player.providers.youtube import YoutubeProvider

    monkeypatch.setattr(yt_dlp, "YoutubeDL", _FakeResolver)
    prov = YoutubeProvider()
    track = Track("song", provider=Source.YOUTUBE, uri="abc123")

    url, headers = prov.resolve_stream_with_headers(track)
    assert url == "https://media.example/audio.webm"
    # must carry the *selected format's* headers, not the generic ones
    assert headers["User-Agent"] == "com.google.android.apps.youtube.vr/1.0"
    # legacy API still works
    assert prov.resolve_stream(track) == url


def test_youtube_failure_is_not_cached(monkeypatch):
    import yt_dlp

    from enigmatic_player.core.track import Source, Track
    from enigmatic_player.providers.youtube import YoutubeProvider

    class _Boom:
        def __init__(self, opts):
            pass

        def extract_info(self, url, download=False):
            raise RuntimeError("HTTP Error 403")

    monkeypatch.setattr(yt_dlp, "YoutubeDL", _Boom)
    prov = YoutubeProvider()
    track = Track("song", provider=Source.YOUTUBE, uri="abc123")

    assert prov.resolve_stream(track) is None
    # failure must NOT be cached: a retry (after e.g. updating yt-dlp) should work.
    # Fresh provider proves the failed result wasn't persisted anywhere.
    monkeypatch.setattr(yt_dlp, "YoutubeDL", _FakeResolver)
    prov2 = YoutubeProvider()
    url, headers = prov2.resolve_stream_with_headers(track)
    assert url and headers


def test_mpv_load_sets_http_headers_before_loadfile():
    from enigmatic_player.core.player import MpvPlayer

    sent = []

    def _fake_send(command, callback=None):
        sent.append(command)
        return len(sent)

    player = MpvPlayer.__new__(MpvPlayer)  # skip __init__ / mpv binary lookup
    player.state = {}
    player._running = False  # keep load()'s refresh timer a no-op
    player._send = _fake_send  # type: ignore[method-assign]

    player.load("https://media.example/x.webm", http_headers={"User-Agent": "ua/1"})
    assert ["set_property", "http-header-fields", ["User-Agent: ua/1"]] in sent
    # header property must be set BEFORE loadfile so it applies to this load
    order = [("http-header-fields" if c[0] == "set_property" else c[0]) for c in sent]
    assert order.index("http-header-fields") < order.index("loadfile")

    # no headers -> clears the property instead of leaving stale state
    sent.clear()
    player.load("/some/file.mp3")
    assert ["set_property", "http-header-fields", []] in sent


if __name__ == "__main__":
    import sys

    sys.exit(pytest.main([__file__, "-v"]))
