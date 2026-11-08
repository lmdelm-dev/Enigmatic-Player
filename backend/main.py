import os
from urllib.parse import urlparse
from fastapi import FastAPI, HTTPException, Query
from fastapi.middleware.cors import CORSMiddleware
from fastapi.responses import JSONResponse
import yt_dlp

app = FastAPI(title="Enigmatic Player Streaming API", version="1.0.0")
origins = [x.strip() for x in os.getenv("ALLOWED_ORIGINS", "*").split(",") if x.strip()]
app.add_middleware(CORSMiddleware, allow_origins=origins, allow_credentials=False, allow_methods=["GET"], allow_headers=["*"])

SEARCH_PREFIX = {"youtube": "ytsearch8:", "soundcloud": "scsearch8:", "bandcamp": "bcsearch8:"}
ALLOWED_HOSTS = ("youtube.com", "youtu.be", "soundcloud.com", "bandcamp.com")

def options():
    return {"quiet": True, "no_warnings": True, "skip_download": True, "noplaylist": True, "extract_flat": "in_playlist"}

def safe_url(value: str) -> str:
    parsed = urlparse(value)
    host = (parsed.hostname or "").lower()
    if parsed.scheme != "https" or not any(host == h or host.endswith("." + h) for h in ALLOWED_HOSTS):
        raise HTTPException(status_code=400, detail="Only YouTube, SoundCloud and Bandcamp URLs are accepted.")
    return value

def public_entry(item, provider):
    return {
        "id": item.get("id"),
        "title": item.get("title") or "Untitled",
        "artist": item.get("artist") or item.get("uploader") or item.get("channel") or "Unknown artist",
        "uploader": item.get("uploader"),
        "duration": item.get("duration"),
        "webpage_url": item.get("webpage_url") or (("https://www.youtube.com/watch?v=" + str(item.get("id"))) if provider == "youtube" and item.get("id") else item.get("url")),
        "url": item.get("url"),
        "thumbnail": item.get("thumbnail"),
    }

@app.get("/health")
def health():
    return {"ok": True, "service": "enigmatic-player-api"}

@app.get("/api/search")
def search(provider: str = Query(...), q: str = Query(..., min_length=1, max_length=180)):
    if provider not in SEARCH_PREFIX:
        raise HTTPException(status_code=400, detail="Unsupported provider.")
    if provider == "bandcamp" and not q.startswith("https://"):
        raise HTTPException(status_code=400, detail="Bandcamp search is not available. Paste a Bandcamp track or album URL.")
    try:
        with yt_dlp.YoutubeDL(options()) as ydl:
            target = q if provider == "bandcamp" else SEARCH_PREFIX[provider] + q
            info = ydl.extract_info(target, download=False)
        if provider == "bandcamp":
            return {"provider": provider, "results": [public_entry(info, provider)]}
        entries = (info or {}).get("entries") or []
        return {"provider": provider, "results": [public_entry(x, provider) for x in entries if x]}
    except HTTPException:
        raise
    except Exception as exc:
        raise HTTPException(status_code=502, detail=f"Search provider error: {str(exc)[:240]}")

@app.get("/api/resolve")
def resolve(url: str = Query(..., min_length=8, max_length=2000)):
    safe_url(url)
    try:
        opts = {"quiet": True, "no_warnings": True, "skip_download": True, "format": "bestaudio/best", "noplaylist": True}
        with yt_dlp.YoutubeDL(opts) as ydl:
            info = ydl.extract_info(url, download=False)
        stream_url = info.get("url")
        if not stream_url:
            formats = info.get("formats") or []
            audio_formats = [f for f in formats if f.get("url") and (f.get("acodec") or "none") != "none"]
            if audio_formats:
                stream_url = audio_formats[-1]["url"]
        if not stream_url:
            raise HTTPException(status_code=502, detail="No playable audio format was found.")
        return {"stream_url": stream_url, "title": info.get("title"), "duration": info.get("duration")}
    except HTTPException:
        raise
    except Exception as exc:
        raise HTTPException(status_code=502, detail=f"Stream resolver error: {str(exc)[:240]}")
