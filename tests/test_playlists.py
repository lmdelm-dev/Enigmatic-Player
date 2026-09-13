import asyncio

from textual.widgets import ListView

from enigmatic_player.app import EnigmaticApp, PlaylistPickerScreen
from enigmatic_player.config import Config
from enigmatic_player.core.track import Source, Track
from enigmatic_player.ui.tracklist import TrackList


def test_playlist_picker_and_playback(tmp_path, monkeypatch):
    cfg = Config(cfg_path=tmp_path / "config.json", data_path=tmp_path / "state.json")
    cfg.add_library_dir(str(tmp_path))
    cfg.create_playlist("First")
    cfg.create_playlist("Second")
    tracks = [Track(name, uri=name, provider=Source.YOUTUBE) for name in ("a", "b", "c")]
    played = []

    async def record_play(track):
        played.append(track)

    async def exercise():
        app = EnigmaticApp(config=cfg)
        monkeypatch.setattr(app, "play_track", record_play)
        async with app.run_test(size=(120, 50)) as pilot:
            await pilot.pause()
            app._add_track_to_playlist(tracks[0])
            await pilot.pause()
            assert isinstance(app.screen, PlaylistPickerScreen)
            await pilot.press("down", "enter")
            await pilot.pause()
            assert cfg.playlists[0]["tracks"] == []
            assert cfg.playlists[1]["tracks"][0]["uri"] == "a"

            app._add_track_to_playlist(tracks[1])
            await pilot.pause()
            await pilot.press("escape")
            await pilot.pause()
            assert len(cfg.playlists[1]["tracks"]) == 1

            for track in tracks[1:]:
                cfg.add_track_to_playlist(1, app._track_to_dict(track))
            app.action_toggle_queue()
            app._enter_playlist_mode(1)
            await pilot.pause()
            view = app.query_one("#list-view", TrackList)
            assert not app._queue_mode
            assert not view.has_class("invisible")
            assert not app.query_one("#queue-view").has_class("visible")
            assert view.items == tracks
            assert view.selected.provider is Source.YOUTUBE
            await pilot.press("enter")
            await pilot.pause()
            assert app.queue.items == tracks
            assert app.queue.position == 0
            assert played[-1] == tracks[0]
            app.action_next()
            await pilot.pause()
            assert played[-1] == tracks[1]

            app.action_toggle_queue()
            await pilot.pause()
            app.query_one("#queue-view-items", ListView).index = 2
            await pilot.press("enter")
            await pilot.pause()
            assert app.queue.position == 2
            assert played[-1] == tracks[2]
            assert app.queue.next() is None

            app._enter_playlist_mode(1)
            await pilot.pause()
            app.action_add_to_queue()
            await pilot.pause()
            assert app.queue.items[-1] == tracks[0]
            app._remove_track_from_current_playlist(0)
            await pilot.pause()
            assert view.items == tracks[1:]
        restored = Config(cfg_path=tmp_path / "config.json", data_path=tmp_path / "state.json")
        assert len(restored.playlists[1]["tracks"]) == 2
        assert len(restored.load_state()["queue"]) == 4

    asyncio.run(exercise())
