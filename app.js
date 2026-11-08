(() => {
  "use strict";
  const audio = new Audio();
  audio.preload = "metadata";
  const $ = (id) => document.getElementById(id);
  const fileInput = $("file-input"), playlist = $("playlist"), dropZone = $("drop-zone");
  const title = $("track-title"), artist = $("track-artist"), state = $("track-state");
  const playButton = $("play"), seek = $("seek"), volume = $("volume");
  const currentTime = $("current-time"), totalTime = $("total-time");
  let tracks = [], current = -1, objectUrls = [], audioContext = null, analyser = null, source = null, animation = 0;
  const fmt = (seconds) => {
    if (!Number.isFinite(seconds)) return "00:00";
    const s = Math.floor(seconds);
    return String(Math.floor(s / 60)).padStart(2, "0") + ":" + String(s % 60).padStart(2, "0");
  };
  const safeName = (name) => name.replace(/\.[^.]+$/, "").replace(/[_-]+/g, " ").trim() || "Unknown track";
  function render() {
    const query = $("filter").value.trim().toLowerCase();
    const visible = tracks.map((track, index) => ({track,index})).filter(({track}) => (track.name + " " + track.artist).toLowerCase().includes(query));
    $("track-count").textContent = tracks.length + (tracks.length === 1 ? " TRACK" : " TRACKS");
    if (!visible.length) {
      playlist.innerHTML = '<div class="empty-library">' + (tracks.length ? "No tracks match your search." : "Your next favourite song is one file away.") + "</div>";
      return;
    }
    playlist.innerHTML = visible.map(({track,index}) => `<div class="track-row ${index === current ? "active" : ""}" data-index="${index}" role="button" tabindex="0" aria-label="Play ${escapeHtml(track.name)}">
      <span class="track-index">${index === current && !audio.paused ? "♫" : String(index + 1).padStart(2,"0")}</span>
      <span class="track-meta"><b>${escapeHtml(track.name)}</b><small>${escapeHtml(track.artist)}</small></span>
      <span class="track-duration">${fmt(track.duration)}</span>
      <button class="remove-track" data-remove="${index}" aria-label="Remove ${escapeHtml(track.name)}">×</button>
    </div>`).join("");
    playlist.querySelectorAll(".track-row").forEach(row => {
      const select = () => loadTrack(Number(row.dataset.index), true);
      row.addEventListener("click", e => { if (!e.target.closest("[data-remove]")) select(); });
      row.addEventListener("keydown", e => { if (e.key === "Enter" || e.key === " ") { e.preventDefault(); select(); } });
    });
    playlist.querySelectorAll("[data-remove]").forEach(button => button.addEventListener("click", e => {
      e.stopPropagation(); removeTrack(Number(button.dataset.remove));
    }));
  }
  function escapeHtml(value) {
    return String(value).replace(/[&<>"']/g, char => ({"&":"&amp;","<":"&lt;",">":"&gt;",'"':"&quot;","'":"&#39;"}[char]));
  }
  function addFiles(files) {
    const accepted = Array.from(files).filter(file => file.type.startsWith("audio/") || /\.(mp3|wav|flac|ogg|m4a|aac|opus)$/i.test(file.name));
    accepted.forEach(file => {
      const url = URL.createObjectURL(file);
      objectUrls.push(url);
      const track = {name:safeName(file.name),artist:"Local file",url,duration:0};
      tracks.push(track);
      const item = new Audio(); item.preload = "metadata"; item.src = url;
      item.addEventListener("loadedmetadata", () => { track.duration = item.duration; render(); }, {once:true});
    });
    render();
    if (current < 0 && tracks.length) loadTrack(0, false);
    fileInput.value = "";
  }
  function loadTrack(index, autoplay) {
    if (!tracks.length) return;
    current = (index + tracks.length) % tracks.length;
    audio.src = tracks[current].url;
    title.textContent = tracks[current].name;
    artist.textContent = tracks[current].artist;
    state.textContent = "LOCAL FILE · READY";
    totalTime.textContent = fmt(tracks[current].duration);
    currentTime.textContent = "00:00"; seek.value = 0;
    $("record-disc").classList.remove("playing");
    render();
    if (autoplay) playAudio();
    else { audio.pause(); playButton.textContent = "▶"; playButton.setAttribute("aria-label","Play"); }
  }
  async function setupAudioGraph() {
    if (audioContext) { if (audioContext.state === "suspended") await audioContext.resume(); return; }
    const Context = window.AudioContext || window.webkitAudioContext;
    if (!Context) return;
    audioContext = new Context();
    analyser = audioContext.createAnalyser(); analyser.fftSize = 128; analyser.smoothingTimeConstant = .82;
    source = audioContext.createMediaElementSource(audio);
    source.connect(analyser); analyser.connect(audioContext.destination);
  }
  async function playAudio() {
    if (current < 0 && tracks.length) loadTrack(0, false);
    if (current < 0) { fileInput.click(); return; }
    try {
      await setupAudioGraph();
      await audio.play();
      playButton.textContent = "Ⅱ"; playButton.setAttribute("aria-label","Pause");
      state.textContent = "NOW PLAYING · LOCAL";
      $("record-disc").classList.add("playing");
      document.querySelector(".player-content").classList.add("playing-state");
      drawVisualizer();
      render();
    } catch (error) {
      state.textContent = "CANNOT PLAY THIS FORMAT";
      console.warn("Audio playback failed:", error);
    }
  }
  function pauseAudio() {
    audio.pause(); playButton.textContent = "▶"; playButton.setAttribute("aria-label","Play");
    state.textContent = "PAUSED"; $("record-disc").classList.remove("playing");
    document.querySelector(".player-content").classList.remove("playing-state");
    cancelAnimationFrame(animation); render();
  }
  function removeTrack(index) {
    const removed = tracks[index];
    if (!removed) return;
    if (index === current) {
      audio.pause(); audio.removeAttribute("src"); audio.load();
      tracks.splice(index,1); current = -1;
      if (tracks.length) loadTrack(Math.min(index,tracks.length-1),false);
      else { title.textContent="A quiet place to begin."; artist.textContent="Choose a song from your device"; state.textContent="WAITING FOR YOUR MUSIC"; }
    } else {
      tracks.splice(index,1);
      if (index < current) current--;
    }
    URL.revokeObjectURL(removed.url);
    render();
  }
  function drawVisualizer() {
    const canvas = $("visualizer"), ctx = canvas.getContext("2d");
    const width = canvas.clientWidth * (window.devicePixelRatio || 1);
    const height = canvas.clientHeight * (window.devicePixelRatio || 1);
    if (canvas.width !== width || canvas.height !== height) { canvas.width = width; canvas.height = height; }
    const data = new Uint8Array(analyser ? analyser.frequencyBinCount : 32);
    const paint = () => {
      animation = requestAnimationFrame(paint);
      if (analyser) analyser.getByteFrequencyData(data);
      ctx.clearRect(0,0,width,height);
      const count =  forty(data.length);
      const gap = 4 * (window.devicePixelRatio || 1);
      const barWidth = Math.max(2,(width-gap*(count-1))/count);
      for (let i=0;i<count;i++) {
        const value = analyser ? data[i] / 255 : .08 + Math.random()*.12;
        const barHeight = Math.max(3,value*height*.9);
        const x = i*(barWidth+gap), y = height-barHeight;
        const gradient = ctx.createLinearGradient(0,y,0,height);
        gradient.addColorStop(0,"#7de4e5"); gradient.addColorStop(1,"#b59cff");
        ctx.fillStyle = gradient; ctx.fillRect(x,y,barWidth,barHeight);
      }
    };
    paint();
  }
  function forty(n) { return Math.min(40, Math.max(12,n)); }
  fileInput.addEventListener("change", () => addFiles(fileInput.files));
  $("play").addEventListener("click", () => audio.paused ? playAudio() : pauseAudio());
  $("previous").addEventListener("click", () => { if (current >= 0) loadTrack(current - 1,true); });
  $("next").addEventListener("click", () => { if (current >= 0) loadTrack(current + 1,true); });
  $("volume").addEventListener("input", () => { audio.volume = Number(volume.value); });
  $("seek").addEventListener("input", () => { if (Number.isFinite(audio.duration)) audio.currentTime = audio.duration * Number(seek.value)/100; });
  $("filter").addEventListener("input", render);
  audio.volume = Number(volume.value);
  audio.addEventListener("timeupdate", () => {
    currentTime.textContent = fmt(audio.currentTime);
    if (Number.isFinite(audio.duration) && audio.duration > 0) seek.value = audio.currentTime/audio.duration*100;
  });
  audio.addEventListener("loadedmetadata", () => { totalTime.textContent = fmt(audio.duration); if (tracks[current]) tracks[current].duration=audio.duration; render(); });
  audio.addEventListener("ended", () => { if (tracks.length > 1) loadTrack(current+1,true); else pauseAudio(); });
  audio.addEventListener("error", () => { if (audio.src) state.textContent = "FORMAT NOT SUPPORTED BY THIS BROWSER"; });
  ["dragenter","dragover"].forEach(event => dropZone.addEventListener(event, e => { e.preventDefault(); dropZone.classList.add("drag-over"); }));
  ["dragleave","drop"].forEach(event => dropZone.addEventListener(event, e => { e.preventDefault(); dropZone.classList.remove("drag-over"); }));
  dropZone.addEventListener("drop", e => addFiles(e.dataTransfer.files));
  $("year").textContent = new Date().getFullYear();
  $("record-disc").classList.remove("playing");
  render();
  window.addEventListener("beforeunload", () => objectUrls.forEach(URL.revokeObjectURL));
})();