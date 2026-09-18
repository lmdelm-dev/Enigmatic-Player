import asyncio
import socket
import subprocess
import time
import wave

import pytest
from mutagen.id3 import TALB, TIT2, TPE1
from mutagen.wave import WAVE

from enigmatic_player import cli
from enigmatic_player.app import EnigmaticApp
from enigmatic_player.config import Config
from enigmatic_player.core import binaries as core_binaries
from enigmatic_player.core.player import MpvPlayer, _SocketIPC
from enigmatic_player.providers.local import LocalProvider


def make_wav(path, seconds=2):
    with wave.open(str(path), "wb") as wav:
        wav.setnchannels(1)
        wav.setsampwidth(2)
        wav.setframerate(8000)
        wav.writeframes(b"\x00\x00" * int(8000 * seconds))


def test_socket_messages_preserve_boundaries_and_partial_utf8():
    receiving, sending = socket.socketpair()
    ipc = _SocketIPC("")
    ipc._sock = receiving
    try:
        sending.sendall(b'{"a":1}\n{"b":2}\n{"title":"\xc3')
        assert ipc.recv_until_newline() == '{"a":1}'
        assert ipc.recv_until_newline() == '{"b":2}'
        with pytest.raises(socket.timeout):
            ipc.recv_until_newline(timeout=0.01)
        sending.sendall(b'\xa9"}\n')
        assert ipc.recv_until_newline() == '{"title":"é"}'
        sending.close()
        with pytest.raises(OSError):
            ipc.recv_until_newline()
    finally:
        ipc.close()
        sending.close()


@pytest.mark.parametrize("url", [
    "https://youtu.be/abcdefghijk?t=12&list=xyz",
    "https://example.com/music/a%20b.mp3",
    "http://localhost:8000/audio.wav",
])
def test_cli_preserves_urls(monkeypatch, url):
    commands = []
    monkeypatch.setattr(core_binaries, "mpv_path", lambda: "/usr/bin/mpv")
    monkeypatch.setattr(cli.subprocess, "call", lambda args: commands.append(args) or 0)
    assert cli.main(["play", url]) == 0
    assert commands[0][-2:] == ["--", url]


def test_cli_local_paths_and_missing_files(tmp_path, monkeypatch):
    commands = []
    monkeypatch.setattr(core_binaries, "mpv_path", lambda: "/usr/bin/mpv")
    monkeypatch.setattr(cli.subprocess, "call", lambda args: commands.append(args) or 0)
    audio = tmp_path / "-song.wav"
    make_wav(audio)
    (tmp_path / "not-a-file.mp3").mkdir()
    assert cli.main(["play", str(tmp_path)]) == 0
    assert commands[0][-2:] == ["--", str(audio)]
    assert cli.main(["play", str(tmp_path / "missing.mp3")]) == 1
    assert len(commands) == 1


def test_local_metadata_duration_search_and_corrupt_file(tmp_path):
    audio = tmp_path / "filename.wav"
    make_wav(audio)
    tagged = WAVE(audio)
    tagged.add_tags()
    tagged.tags.add(TIT2(encoding=3, text=["Actual title"]))
    tagged.tags.add(TPE1(encoding=3, text=["Artist"]))
    tagged.tags.add(TALB(encoding=3, text=["Album"]))
    tagged.save()
    (tmp_path / "broken.mp3").write_bytes(b"invalid audio")
    cfg = Config(cfg_path=tmp_path / "config.json", data_path=tmp_path / "state.json")
    cfg.add_library_dir(str(tmp_path))
    provider = LocalProvider(cfg)
    tracks = provider.scan()
    assert len(tracks) == 2
    track = provider.search("Artist Album")[0]
    assert track.title == "Actual title"
    assert track.artist == "Artist"
    assert track.album == "Album"
    assert track.duration == pytest.approx(2)
    assert provider.resolve_stream(track) == str(audio)
    assert next(t for t in tracks if t.title == "broken").duration == 0


@pytest.fixture
def null_audio(monkeypatch):
    # Exercise the real engine with a null audio output (no sound device required).
    original_popen = subprocess.Popen

    def silent_mpv(args, **kwargs):
        return original_popen([*args, "--no-config", "--ao=null"], **kwargs)

    monkeypatch.setattr(subprocess, "Popen", silent_mpv)


def test_real_mpv_playback_and_idle_ipc(tmp_path, null_audio):
    audio = tmp_path / "tone.wav"
    make_wav(audio, seconds=5)
    with MpvPlayer() as player:
        for volume in range(10, 20):
            player.set_volume(volume)
            assert player.get_property("volume", timeout=2) == volume
        # Several receive timeouts while idle must not kill the reader.
        time.sleep(1.2)
        assert player.running
        assert player.get_property("volume", timeout=2) == 19
        player.load(str(audio))
        player.play()
        deadline = time.monotonic() + 5
        while time.monotonic() < deadline:
            if player.state.get("time_pos", 0) > 0:
                break
            time.sleep(0.05)
        assert player.state["time_pos"] > 0
        assert player.get_property("duration", timeout=2) == pytest.approx(5)
        player.pause()
        assert player.get_property("pause", timeout=2) is True
        player.seek(2, absolute=True)
        # Command acceptance precedes asynchronous seek completion in mpv.
        deadline = time.monotonic() + 3
        while time.monotonic() < deadline:
            if player.get_property("time-pos", timeout=2) == pytest.approx(2, abs=0.2):
                break
            time.sleep(0.05)
        assert player.get_property("time-pos", timeout=2) == pytest.approx(2, abs=0.2)
        player.set_volume(0)
        assert player.get_property("volume", timeout=2) == 0
    assert not player.running
    assert player._proc.poll() is not None


def test_saved_playlist_through_tui_and_real_player(tmp_path, null_audio):
    audio = tmp_path / "tone.wav"
    make_wav(audio, seconds=5)
    cfg = Config(cfg_path=tmp_path / "config.json", data_path=tmp_path / "state.json")
    cfg.add_library_dir(str(tmp_path))
    track = LocalProvider(cfg).scan()[0]
    cfg.create_playlist("Local tracks")
    cfg.add_track_to_playlist(0, EnigmaticApp._track_to_dict(track))

    async def exercise():
        app = EnigmaticApp(config=cfg)
        async with app.run_test(size=(120, 50)) as pilot:
            await pilot.pause()
            app._enter_playlist_mode(0)
            await pilot.pause()
            await pilot.press("enter")
            deadline = time.monotonic() + 5
            while time.monotonic() < deadline:
                if app._current_track and app.player.state.get("time_pos", 0) > 0:
                    break
                await pilot.pause(0.05)
            assert app._current_track == track
            assert app.player.state["time_pos"] > 0
            assert app.queue.current == track
            app.action_play_pause()
            assert app.player.get_property("pause", timeout=2) is True
        assert cfg.load_state()["queue"][0]["uri"] == str(audio)
        assert app.player._proc.poll() is not None

    asyncio.run(exercise())
