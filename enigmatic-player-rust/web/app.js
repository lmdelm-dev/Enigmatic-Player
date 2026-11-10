(function () {
'use strict';

var K = window.THEME_KEYS, T = window.THEMES, S = window.LIBRARY;
var RV = [['off',0],['room',25],['hall',45],['cave',65],['space',85]];
var SP = [0.5,0.75,1,1.25,1.5,2];
var NB = 20;
var CFG_KEY = 'enigmatic-player.config';

var si = 0, ti = 0, pos = 0, playing = true, open = true, playSrc = 0, filt = 'all', tab = 0;
var theme = 29, dir = '~/Music/enigmatic-player';
var vol = 0.7, muted = false, speed = 1, rvName = 'off', wet = 0, seeking = false;

var $ = function (i) { return document.getElementById(i); };
function fmt(s) { s = Math.floor(s); var m = Math.floor(s / 60); return (m < 10 ? '0' : '') + m + ':' + (s % 60 < 10 ? '0' : '') + (s % 60); }
function esc(x) { return x.replace(/&/g,'&amp;').replace(/</g,'&lt;').replace(/"/g,'&quot;'); }
function dur() { return S[playSrc].t[ti][2]; }

/* ---------- config (localStorage) ---------- */
function saveCfg() {
  try {
    localStorage.setItem(CFG_KEY, JSON.stringify({ theme: T[theme][0], dir: dir, vol: vol, muted: muted, speed: speed, rvName: rvName, wet: wet }));
  } catch (e) {}
}
function loadCfg() {
  try {
    var c = JSON.parse(localStorage.getItem(CFG_KEY) || 'null');
    if (!c) return;
    T.forEach(function (t, i) { if (t[0] === c.theme) theme = i; });
    if (typeof c.dir === 'string') { dir = c.dir; S[0].sub = dir; }
    if (typeof c.vol === 'number') vol = c.vol;
    if (typeof c.muted === 'boolean') muted = c.muted;
    if (typeof c.speed === 'number') speed = c.speed;
    if (typeof c.rvName === 'string') rvName = c.rvName;
    if (typeof c.wet === 'number') wet = c.wet;
  } catch (e) {}
}

/* ---------- visualizer ---------- */
var vis = $('vis'), bars = [], prev = [];
var cols = ['var(--green)','var(--green)','var(--green)','var(--cyan)','var(--cyan)','var(--cyan)','var(--purple)','var(--purple)','var(--purple)','var(--red)','var(--red)','var(--red)','var(--red)','var(--red)'];
cols.forEach(function (c) {
  var b = document.createElement('div');
  b.className = 'vb'; b.style.setProperty('--c', c); b.style.height = '20px';
  vis.appendChild(b); bars.push(b); prev.push(20);
});

/* ---------- pointer helpers ---------- */
function frac(e, el) { var r = el.getBoundingClientRect(); return Math.min(1, Math.max(0, (e.clientX - r.left) / r.width)); }
function drag(el, fn) {
  el.addEventListener('pointerdown', function (e) {
    e.preventDefault(); el.setPointerCapture(e.pointerId); fn(frac(e, el), 'down');
    var mv = function (ev) { fn(frac(ev, el), 'move'); };
    var up = function (ev) {
      el.removeEventListener('pointermove', mv);
      el.removeEventListener('pointerup', up);
      el.removeEventListener('pointercancel', up);
      fn(frac(ev, el), 'up');
    };
    el.addEventListener('pointermove', mv); el.addEventListener('pointerup', up); el.addEventListener('pointercancel', up);
  });
}
function mkBlocks(el) { for (var i = 0; i < NB; i++) el.appendChild(document.createElement('i')); }
mkBlocks($('vs')); mkBlocks($('rs'));

/* ---------- volume ---------- */
function paintVol() {
  var on = muted ? 0 : Math.round(vol * NB), bl = $('vs').children;
  for (var i = 0; i < NB; i++) {
    bl[i].style.background = i < on ? (i < 12 ? 'var(--green)' : (i < 17 ? 'var(--yellow)' : 'var(--red)')) : 'var(--sel)';
  }
  $('vv').textContent = muted ? 'mute' : Math.round(vol * 100) + '%';
  $('mute').style.color = muted ? 'var(--red)' : 'var(--green)';
  $('mute').textContent = muted ? 'mute' : 'vol';
  $('vs').setAttribute('aria-valuenow', Math.round(vol * 100));
  updStat(); saveCfg();
}
function setVol(v) {
  vol = Math.min(1, Math.max(0, Math.round(v * NB) / NB));
  muted = vol === 0;
  paintVol();
}

/* ---------- reverb ---------- */
function paintRv() {
  var on = Math.round(wet / 100 * NB), bl = $('rs').children;
  for (var i = 0; i < NB; i++) bl[i].style.background = i < on ? 'var(--purple)' : 'var(--sel)';
  $('rv').textContent = wet + '%';
  var h = '';
  RV.forEach(function (r) { h += '<button class="chip ' + (rvName === r[0] ? 'on' : '') + '" data-r="' + r[0] + '">' + r[0] + '</button>'; });
  $('rvc').innerHTML = h;
  $('rvc').querySelectorAll('button').forEach(function (b) {
    b.onclick = function () {
      var r = RV.filter(function (x) { return x[0] === b.dataset.r; })[0];
      rvName = r[0]; wet = r[1]; paintRv(); setNp();
    };
  });
  updStat(); saveCfg();
}
function setWet(w) {
  wet = Math.round(w * NB) * 5;
  if (wet === 0) rvName = 'off';
  else if (rvName === 'off') rvName = 'room';
  paintRv(); setNp();
}

/* ---------- speed ---------- */
function paintSpeed() {
  var h = '';
  SP.forEach(function (s) { h += '<button class="chip ' + (Math.abs(speed - s) < 0.001 ? 'on' : '') + '" data-s="' + s + '">' + s + 'x</button>'; });
  $('spc').innerHTML = h;
  $('spc').querySelectorAll('button').forEach(function (b) { b.onclick = function () { setSpeed(+b.dataset.s); }; });
  $('sv').textContent = speed.toFixed(2) + 'x';
  updStat(); saveCfg();
}
function setSpeed(s) { speed = Math.min(2, Math.max(0.5, Math.round(s * 100) / 100)); paintSpeed(); setNp(); }

function updStat() {
  $('st1').textContent = muted ? 'vol muted' : 'vol ' + Math.round(vol * 100) + '%';
  $('st2').textContent = 'speed ' + speed.toFixed(2) + 'x - reverb ' + (wet ? rvName + ' ' + wet + '%' : 'off');
}
function setNp() {
  var x = 'now playing - ' + S[playSrc].name;
  if (Math.abs(speed - 1) > 0.001) x += ' - ' + speed.toFixed(2) + 'x';
  if (wet) x += ' - reverb ' + rvName;
  $('np').textContent = x;
}

/* ---------- seek ---------- */
function paintPos() {
  var p = pos / dur() * 100;
  $('cur').textContent = fmt(pos); $('fill').style.width = p + '%'; $('head').style.left = p + '%';
  $('sk').setAttribute('aria-valuenow', Math.round(p));
}
function seekTo(t) { var d = dur(); pos = Math.min(d - 0.01, Math.max(0, t)); paintPos(); }
function showTip(f) {
  var t = $('tip'); t.style.display = 'block';
  t.style.left = Math.min(94, Math.max(6, f * 100)) + '%'; t.textContent = fmt(f * dur());
}
drag($('sk'), function (f, ph) {
  if (ph === 'down') seeking = true;
  seekTo(f * dur()); showTip(f);
  if (ph === 'up') { seeking = false; $('tip').style.display = 'none'; }
});
$('sk').addEventListener('pointermove', function (e) { if (!seeking) showTip(frac(e, $('sk'))); });
$('sk').addEventListener('pointerleave', function () { if (!seeking) $('tip').style.display = 'none'; });
$('sk').addEventListener('keydown', function (e) {
  if (e.key === 'ArrowLeft') { e.preventDefault(); seekTo(pos - 5); }
  else if (e.key === 'ArrowRight') { e.preventDefault(); seekTo(pos + 5); }
});
drag($('vs'), function (f) { setVol(f); });
drag($('rs'), function (f) { setWet(f); });
$('mute').onclick = function () { muted = !muted; if (!muted && vol === 0) vol = 0.5; paintVol(); };
$('sdn').onclick = function () { setSpeed(speed - 0.05); };
$('sup').onclick = function () { setSpeed(speed + 0.05); };
$('bk').onclick = function () { seekTo(pos - 10); };
$('fw').onclick = function () { seekTo(pos + 10); };

/* ---------- theme ---------- */
function applyTheme(i) {
  var t = T[i], p = $('px');
  K.forEach(function (k, j) { p.style.setProperty('--' + k, t[j + 1]); });
  theme = i; $('thn').textContent = t[0]; saveCfg();
}

/* ---------- library and search ---------- */
function renderSrc() {
  var h = '';
  S.forEach(function (s, i) {
    h += '<button class="' + (i === si ? 'sel' : '') + '" data-s="' + i + '"><span style="color:' + s.c + '">[' + s.tag + ']</span> ' + s.name +
         '<div class="dim" style="font-size:12px;padding-left:28px">' + esc(s.sub) + ' - ' + s.st + '</div></button>';
  });
  $('srcs').innerHTML = h;
  $('srcs').querySelectorAll('button').forEach(function (b) { b.onclick = function () { si = +b.dataset.s; renderSrc(); renderTracks(); }; });
}
function renderTracks() {
  var s = S[si], h = '';
  s.t.forEach(function (t, i) {
    var on = (si === playSrc && i === ti);
    h += '<button class="' + (on ? 'sel' : '') + '" data-t="' + i + '"><span class="row"><span style="color:' + (on ? 'var(--red)' : 'var(--text)') + '">' +
         (on ? '&gt; ' : '  ') + ('0' + (i + 1)).slice(-2) + '  ' + esc(t[0]) + ' <span class="dim">- ' + esc(t[1]) + '</span></span><span class="dim">' + fmt(t[2]) + '</span></span></button>';
  });
  $('tracks').innerHTML = h;
  $('tracks').querySelectorAll('button').forEach(function (b) { b.onclick = function () { playTrack(si, +b.dataset.t); }; });
}
function playTrack(s, t) { playSrc = s; ti = t; pos = 0; playing = true; load(); }
function renderChips() {
  var h = '<button class="chip ' + (filt === 'all' ? 'on' : '') + '" data-f="all">all</button>';
  S.forEach(function (s) { h += '<button class="chip ' + (filt === s.k ? 'on' : '') + '" data-f="' + s.k + '"><span style="color:' + s.c + '">[' + s.tag + ']</span> ' + s.k + '</button>'; });
  $('chips').innerHTML = h;
  $('chips').querySelectorAll('button').forEach(function (b) { b.onclick = function () { filt = b.dataset.f; renderChips(); renderRes(); }; });
}
function renderRes() {
  var q = $('q').value.trim().toLowerCase(), box = $('res');
  if (!q) { box.innerHTML = '<div class="dim" style="font-size:12px">type a title or artist to search every connected source. press / to focus.</div>'; return; }
  var hits = [];
  S.forEach(function (s, a) {
    if (filt !== 'all' && filt !== s.k) return;
    s.t.forEach(function (t, b) { if ((t[0] + ' ' + t[1]).toLowerCase().indexOf(q) > -1) hits.push([a, b]); });
  });
  if (!hits.length) { box.innerHTML = '<div style="color:var(--yellow);font-size:12px">no matches for "' + esc(q) + '". try an artist name or another source.</div>'; return; }
  var h = '<div class="dim" style="font-size:12px;margin-bottom:2px">' + hits.length + ' result' + (hits.length > 1 ? 's' : '') + '</div>';
  hits.forEach(function (x, i) {
    var s = S[x[0]], t = s.t[x[1]];
    h += '<button data-i="' + i + '"><span class="row"><span><span style="color:' + s.c + '">[' + s.tag + ']</span> ' + esc(t[0]) + ' <span class="dim">- ' + esc(t[1]) + '</span></span><span class="dim">' + fmt(t[2]) + '</span></span></button>';
  });
  box.innerHTML = h;
  box.querySelectorAll('button').forEach(function (b) { b.onclick = function () { var x = hits[+b.dataset.i]; playTrack(x[0], x[1]); }; });
}

/* ---------- playback ---------- */
function load() {
  var t = S[playSrc].t[ti];
  $('title').textContent = t[0]; $('artist').textContent = t[1]; $('dur').textContent = fmt(t[2]);
  $('sky').style.background = t[3][0]; $('sun').style.background = t[3][1]; $('hor').style.background = t[3][1]; $('sea').style.background = t[3][2];
  $('pp').textContent = playing ? '||' : '>';
  setNp(); renderTracks(); paintPos();
}
function nextTrack(d) { ti = (ti + d + S[playSrc].t.length) % S[playSrc].t.length; pos = 0; load(); }
function tick() {
  if (playing && !seeking) { pos += 0.15 * speed; if (pos >= dur()) { nextTrack(1); return; } }
  paintPos();
  var ev = muted ? 0 : vol, decay = playing ? 0.45 + 0.5 * (wet / 100) : 0.3;
  bars.forEach(function (b, i) {
    var tg = playing ? (8 + Math.random() * 52 * (1 - Math.abs(i - 6) / 16)) * (0.25 + 0.75 * ev) * (0.8 + 0.2 * speed) : 4;
    var h = Math.max(tg, prev[i] * decay); prev[i] = h;
    b.style.height = Math.max(4, Math.round(h)) + 'px';
  });
}
$('pp').onclick = function () { playing = !playing; $('pp').textContent = playing ? '||' : '>'; };
$('next').onclick = function () { nextTrack(1); };
$('prev').onclick = function () { if (pos > 3) seekTo(0); else nextTrack(-1); };

/* ---------- settings window ---------- */
var TABS = ['themes', 'local music', 'about'];
function renderNav() {
  var h = '';
  TABS.forEach(function (n, i) { h += '<button class="' + (i === tab ? 'sel' : '') + '" data-n="' + i + '" style="padding:8px 12px">' + (i === tab ? '&gt; ' : '  ') + n + '</button>'; });
  $('snav').innerHTML = h;
  $('snav').querySelectorAll('button').forEach(function (b) { b.onclick = function () { tab = +b.dataset.n; showTab(); }; });
}
function showTab() {
  renderNav();
  var b = $('sbody'), h = '';
  if (tab === 0) {
    h = '<div class="dim" style="font-size:12px;margin-bottom:8px">pick a theme. the player changes right away.</div><div style="display:grid;grid-template-columns:repeat(auto-fill,minmax(150px,1fr));gap:6px">';
    T.forEach(function (t, i) {
      h += '<button class="th ' + (i === theme ? 'on' : '') + '" data-i="' + i + '"><span class="sw" style="background:' + t[1] + '"><i style="background:' + t[7] + '"></i><i style="background:' + t[8] + '"></i><i style="background:' + t[9] + '"></i><i style="background:' + t[10] + '"></i></span><span style="' + (i === theme ? 'color:var(--purple)' : '') + ';overflow:hidden;text-overflow:ellipsis;white-space:nowrap">' + (i === theme ? '> ' : '') + t[0] + '</span></button>';
    });
    h += '</div>';
    b.innerHTML = h;
    b.querySelectorAll('.th').forEach(function (x) { x.onclick = function () { applyTheme(+x.dataset.i); showTab(); }; });
  } else if (tab === 1) {
    h = '<div style="color:var(--yellow);margin-bottom:4px">local music folder</div><div class="dim" style="font-size:12px;margin-bottom:10px">Songs you download are saved here, and the local files source reads from it.</div>' +
        '<div style="display:flex;gap:8px;align-items:center"><input id="dir" type="text" value="' + esc(dir) + '" aria-label="Local music folder path"><button class="chip" id="dsave" style="color:var(--green)">save</button></div>' +
        '<div id="dmsg" style="font-size:12px;min-height:18px;margin-top:6px"></div>' +
        '<div class="dim" style="font-size:12px;margin:8px 0 4px">quick picks</div><div class="chips" id="dpick"></div>' +
        '<div class="dim" style="font-size:12px;margin-top:12px">current: <span style="color:var(--cyan)" id="dcur">' + esc(dir) + '</span></div>';
    b.innerHTML = h;
    var ph = '';
    ['~/Music/enigmatic-player', '~/Music', '~/Downloads/music', '/mnt/media/music'].forEach(function (p) { ph += '<button class="chip" data-p="' + esc(p) + '">' + esc(p) + '</button>'; });
    $('dpick').innerHTML = ph;
    $('dpick').querySelectorAll('button').forEach(function (x) { x.onclick = function () { $('dir').value = x.dataset.p; $('dmsg').textContent = ''; }; });
    $('dir').oninput = function () { $('dmsg').textContent = ''; };
    $('dsave').onclick = function () {
      var v = $('dir').value.trim(), m = $('dmsg');
      if (!v) { m.style.color = 'var(--red)'; m.textContent = 'Enter a folder path first.'; return; }
      if (!/^(~|\/|[A-Za-z]:[\\\/])/.test(v)) { m.style.color = 'var(--red)'; m.textContent = 'Start the path with ~, /, or a drive letter like C:\\.'; return; }
      dir = v; S[0].sub = v; $('dcur').textContent = v;
      m.style.color = 'var(--green)'; m.textContent = 'Saved. New downloads go to this folder.';
      renderSrc(); saveCfg();
    };
  } else {
    h = '<div style="color:var(--yellow);margin-bottom:6px">about enigmatic player</div>' +
        '<div style="margin-bottom:10px">A pixel-style terminal music player that brings local files, YouTube, SoundCloud, and Bandcamp into one place.</div>' +
        '<div style="border:2px solid var(--line);padding:8px 10px;margin-bottom:10px">' +
        '<div class="row"><span class="dim">version</span><span>v1.4</span></div>' +
        '<div class="row"><span class="dim">made by</span><span style="color:var(--cyan)">Jamal Tiliouan</span></div>' +
        '<div class="row"><span class="dim">project</span><span style="color:var(--green)">open source</span></div></div>' +
        '<div class="dim" style="font-size:12px;margin-bottom:4px">shortcuts</div>';
    [['/','search'],['tab','hide or show library'],['space','play or pause'],['left / right','seek 5 seconds'],['up / down','volume'],['m','mute'],['[ and ]','slower and faster'],['r','cycle reverb'],['esc','close settings']].forEach(function (s) {
      h += '<div class="row" style="font-size:12px"><span>' + s[0] + '</span><span class="dim">' + s[1] + '</span></div>';
    });
    h += '<div style="margin-top:12px;color:var(--purple)">Contributions and ideas are welcome.</div>';
    b.innerHTML = h;
  }
}
function openSet() { $('set').style.display = 'flex'; tab = 0; showTab(); }
function closeSet() { $('set').style.display = 'none'; }
$('gear').onclick = openSet; $('sx').onclick = closeSet;
$('set').onclick = function (e) { if (e.target === $('set')) closeSet(); };

function toggle() { open = !open; $('lib').style.display = open ? 'grid' : 'none'; $('togi').textContent = open ? '[-]' : '[+]'; }
$('tog').onclick = toggle;
$('q').oninput = renderRes;
$('clr').onclick = function () { $('q').value = ''; renderRes(); $('q').focus(); };

/* ---------- keyboard ---------- */
document.addEventListener('keydown', function (e) {
  if ($('set').style.display === 'flex') { if (e.key === 'Escape') closeSet(); return; }
  var tag = e.target && e.target.tagName;
  if (tag === 'INPUT') { if (e.key === 'Escape') { $('q').value = ''; renderRes(); $('q').blur(); } return; }
  var k = e.key;
  if (k === 'Tab') { e.preventDefault(); toggle(); }
  else if (k === ' ') { e.preventDefault(); $('pp').click(); }
  else if (k === '/') { e.preventDefault(); $('q').focus(); }
  else if (k === 'ArrowLeft') { e.preventDefault(); seekTo(pos - 5); }
  else if (k === 'ArrowRight') { e.preventDefault(); seekTo(pos + 5); }
  else if (k === 'ArrowUp') { e.preventDefault(); setVol(vol + 0.05); }
  else if (k === 'ArrowDown') { e.preventDefault(); setVol(vol - 0.05); }
  else if (k === 'm') { $('mute').click(); }
  else if (k === '[') { setSpeed(speed - 0.05); }
  else if (k === ']') { setSpeed(speed + 0.05); }
  else if (k === 'r') {
    var n = 0; RV.forEach(function (r, i) { if (r[0] === rvName) n = i; });
    var r = RV[(n + 1) % RV.length]; rvName = r[0]; wet = r[1]; paintRv(); setNp();
  }
});

/* ---------- boot ---------- */
loadCfg();
applyTheme(theme); paintVol(); paintRv(); paintSpeed();
renderSrc(); renderChips(); renderRes(); load();
setInterval(tick, 150);

})();
