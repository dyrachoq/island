const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

const $ = (s) => document.querySelector(s);
const island = $("#island");

// ---------------- пружинная анимация ----------------
// Считаем физику пружины и превращаем её в CSS-функцию linear(),
// так морфинг острова «пружинит», как на iPhone.
function spring(stiffness, damping, mass = 1) {
  const dt = 1 / 600;
  let x = 0, v = 0, t = 0;
  const samples = [0];
  while (t < 2.5) {
    const a = (-stiffness * (x - 1) - damping * v) / mass;
    v += a * dt;
    x += v * dt;
    t += dt;
    samples.push(x);
    if (t > 0.15 && Math.abs(x - 1) < 0.0008 && Math.abs(v) < 0.01) break;
  }
  const n = 48;
  const pts = [];
  for (let i = 0; i <= n; i++) {
    const idx = Math.round((i / n) * (samples.length - 1));
    pts.push(+samples[idx].toFixed(4));
  }
  pts[pts.length - 1] = 1;
  return { ease: `linear(${pts.join(", ")})`, dur: Math.round(t * 1000) };
}

const supportsLinear = CSS.supports("transition-timing-function", "linear(0, 1)");
const SPRING_OPEN = supportsLinear ? spring(230, 20) : { ease: "cubic-bezier(.34,1.4,.64,1)", dur: 550 };
const SPRING_CLOSE = supportsLinear ? spring(300, 32) : { ease: "cubic-bezier(.32,.72,0,1)", dur: 420 };

function useSpring(s) {
  island.style.setProperty("--ease", s.ease);
  island.style.setProperty("--dur", s.dur + "ms");
}

// ---------------- состояние ----------------
const state = {
  settings: { expandOnHover: true, chargingActivity: true, timerSound: true },
  expanded: false,
  hidden: false,
  hovering: false,
  tab: "music",
  battery: null,
  chargingFlashUntil: 0,
  alert: false,
};

const SIZE_ORDER = { "s-compact": 0, "s-activity": 1, "s-osd": 1, "s-alert": 2, "s-expanded": 3, "s-tall": 4 };
const TALL_TABS = ["clipboard", "files"];
let currentSize = "s-compact";

function activity() {
  if (Date.now() < state.chargingFlashUntil) return "charging";
  if (Date.now() < clip.flashUntil) return "clip";
  if (timer.running) return "timer";
  if (sw.running) return "stopwatch";
  if (media.info.active && media.info.playing) return "music";
  return null;
}

function render() {
  let size = "s-compact";
  let layer = null;

  if (state.alert) { size = "s-alert"; layer = "alert"; }
  else if (state.expanded) {
    size = TALL_TABS.includes(state.tab) ? "s-tall" : "s-expanded";
    layer = "expanded";
  }
  else if (Date.now() < osd.until) { size = "s-osd"; layer = "osd"; }
  else if (activity()) { size = "s-activity"; layer = "activity"; }

  if (size !== currentSize) {
    useSpring(SIZE_ORDER[size] > SIZE_ORDER[currentSize] ? SPRING_OPEN : SPRING_CLOSE);
    island.classList.replace(currentSize, size);
    currentSize = size;
  }
  island.classList.toggle("hidden", state.hidden);

  document.querySelectorAll(".layer").forEach((el) => {
    el.classList.toggle("on", el.dataset.layer === layer);
  });
  if (state.tab === "clipboard" && !state.settings.clipboardHistory) state.tab = "music";
  document.querySelectorAll(".tab").forEach((el) => {
    el.classList.toggle("active", el.dataset.tab === state.tab);
    if (el.dataset.tab === "clipboard") el.hidden = !state.settings.clipboardHistory;
  });
  $("#tab-clear").hidden = !(
    (state.tab === "clipboard" && clip.items.some((i) => !i.pinned)) ||
    (state.tab === "files" && shelf.items.length > 0)
  );
  island.classList.toggle("dropping", shelf.dropping);
  document.querySelectorAll(".panel").forEach((el) => {
    el.classList.toggle("on", el.dataset.panel === state.tab);
  });

  renderActivity();
  renderBatteryMini();
  reportHitRect();
}

// ---------------- «живая активность» в компактном виде ----------------
const ICON = (id, cls = "") => `<svg class="ico ${cls}"><use href="#${id}"/></svg>`;

function renderActivity() {
  const left = $("#act-left");
  const right = $("#act-right");
  const kind = activity();
  left.className = "slot left";
  right.className = "slot right";

  if (kind === "charging" && state.battery) {
    left.classList.add("green");
    right.classList.add("green");
    left.innerHTML = ICON("i-bolt");
    right.innerHTML = `${state.battery.percent}% <span class="batt" style="--level:${state.battery.percent}%"><span class="batt-fill"></span></span>`;
  } else if (kind === "timer") {
    left.classList.add("orange");
    right.classList.add("orange");
    left.innerHTML = ICON(timer.paused ? "i-pause" : "i-timer");
    right.textContent = fmtTimer(timer.remaining());
  } else if (kind === "stopwatch") {
    left.classList.add("green");
    right.classList.add("green");
    left.innerHTML = ICON("i-watch");
    right.textContent = fmtStopwatch(sw.elapsed(), false);
  } else if (kind === "clip") {
    left.innerHTML = ICON("i-clip");
    right.innerHTML = `<span class="green">Скопировано</span>`;
  } else if (kind === "music") {
    const key = "music|" + (media.artUrl || "");
    if (left.dataset.key !== key) {
      left.dataset.key = key;
      left.innerHTML = media.artUrl
        ? `<img class="mini-art" src="${media.artUrl}" alt="">`
        : `<span class="mini-art ph"><svg><use href="#i-note"/></svg></span>`;
      right.innerHTML = `<span class="eq small playing"><i></i><i></i><i></i><i></i></span>`;
    }
    return;
  }
  left.dataset.key = "";
}

function renderBatteryMini() {
  const el = $("#battery-mini");
  const b = state.battery;
  if (!b || !b.hasBattery) { el.hidden = true; return; }
  el.hidden = false;
  el.classList.toggle("charging", b.charging);
  el.querySelector(".batt").style.setProperty("--level", b.percent + "%");
  el.querySelector(".batt-text").textContent = b.percent + "%";
}

// ---------------- клики сквозь прозрачную часть окна ----------------
// Сообщаем Rust, где сейчас остров, — вне этой области мышь уходит к окнам ниже.
let lastRect = "";
function reportHitRect() {
  requestAnimationFrame(() => {
    const r = island.getBoundingClientRect();
    const pad = 4;
    const rect = state.hidden
      ? { x: 0, y: 0, w: 0, h: 0 }
      : { x: r.left - pad, y: r.top, w: r.width + pad * 2, h: r.height + pad };
    const key = JSON.stringify(rect);
    if (key !== lastRect) {
      lastRect = key;
      invoke("set_hit_rect", rect).catch(() => {});
    }
  });
}
new ResizeObserver(reportHitRect).observe(island);

// ---------------- раскрытие / сворачивание ----------------
let openTimer = 0, closeTimer = 0;

function expand() {
  clearTimeout(closeTimer);
  if (state.hidden || state.alert) return;
  if (!state.expanded) {
    // открываем ту вкладку, которая сейчас «живёт» в свёрнутом острове
    const a = activity();
    if (a === "timer" || a === "stopwatch" || a === "music") state.tab = a;
    state.expanded = true;
    render();
  }
}
function collapse() {
  clearTimeout(openTimer);
  if (state.expanded) {
    state.expanded = false;
    render();
  }
}

listen("hover", (e) => {
  state.hovering = e.payload;
  if (state.hovering) {
    clearTimeout(closeTimer);
    if (state.settings.expandOnHover) openTimer = setTimeout(expand, 140);
  } else {
    clearTimeout(openTimer);
    closeTimer = setTimeout(collapse, state.settings.expandOnHover ? 350 : 900);
  }
});

island.addEventListener("click", (e) => {
  if (state.alert) return;
  if (!state.expanded) expand();
  else if (!state.settings.expandOnHover && e.target === island) collapse();
});
island.addEventListener("contextmenu", (e) => {
  e.preventDefault();
  invoke("open_settings");
});
$("#open-settings").addEventListener("click", (e) => {
  e.stopPropagation();
  invoke("open_settings");
});
document.querySelectorAll(".tab").forEach((el) =>
  el.addEventListener("click", (e) => {
    e.stopPropagation();
    state.tab = el.dataset.tab;
    render();
  })
);

// ---------------- форматирование ----------------
const pad2 = (n) => String(n).padStart(2, "0");
function fmtTimer(ms) {
  const total = Math.ceil(Math.max(0, ms) / 1000);
  const h = Math.floor(total / 3600), m = Math.floor((total % 3600) / 60), s = total % 60;
  return h ? `${h}:${pad2(m)}:${pad2(s)}` : `${pad2(m)}:${pad2(s)}`;
}
function fmtStopwatch(ms, withCs = true) {
  const cs = Math.floor(ms / 10) % 100;
  const s = Math.floor(ms / 1000) % 60;
  const m = Math.floor(ms / 60000);
  return withCs ? `${pad2(m)}:${pad2(s)},${pad2(cs)}` : `${pad2(m)}:${pad2(s)}`;
}

// ---------------- таймер ----------------
const timer = {
  setMinutes: 5,
  running: false,
  paused: false,
  endAt: 0,
  left: 0,      // остаток на паузе
  total: 0,
  remaining() {
    if (!this.running) return this.setMinutes * 60000;
    return this.paused ? this.left : this.endAt - Date.now();
  },
};

function renderTimer() {
  $("#t-setup-time").textContent = fmtTimer(timer.setMinutes * 60000);
  document.querySelectorAll(".chip").forEach((c) =>
    c.classList.toggle("active", +c.dataset.min === timer.setMinutes)
  );
  $("#timer-setup").hidden = timer.running;
  $("#timer-run").hidden = !timer.running;
  $("#t-cancel").hidden = !timer.running;

  const main = $("#t-main");
  main.textContent = !timer.running ? "Старт" : timer.paused ? "Дальше" : "Пауза";

  if (timer.running) {
    const rem = timer.remaining();
    $("#t-run-time").textContent = fmtTimer(rem);
    const frac = Math.max(0, rem / timer.total);
    $("#t-ring").style.strokeDashoffset = (119.38 * (1 - frac)).toFixed(2);
  }
}

document.querySelectorAll(".chip").forEach((c) =>
  c.addEventListener("click", (e) => {
    e.stopPropagation();
    timer.setMinutes = +c.dataset.min;
    renderTimer();
  })
);
$("#t-minus").addEventListener("click", (e) => {
  e.stopPropagation();
  timer.setMinutes = Math.max(1, timer.setMinutes - 1);
  renderTimer();
});
$("#t-plus").addEventListener("click", (e) => {
  e.stopPropagation();
  timer.setMinutes = Math.min(180, timer.setMinutes + 1);
  renderTimer();
});
$("#t-main").addEventListener("click", (e) => {
  e.stopPropagation();
  unlockAudio();
  if (!timer.running) {
    timer.running = true;
    timer.paused = false;
    timer.total = timer.setMinutes * 60000;
    timer.endAt = Date.now() + timer.total;
  } else if (timer.paused) {
    timer.paused = false;
    timer.endAt = Date.now() + timer.left;
  } else {
    timer.paused = true;
    timer.left = timer.endAt - Date.now();
  }
  renderTimer();
  render();
});
$("#t-cancel").addEventListener("click", (e) => {
  e.stopPropagation();
  timer.running = false;
  timer.paused = false;
  renderTimer();
  render();
});

function timerFinished() {
  timer.running = false;
  timer.paused = false;
  state.alert = true;
  state.expanded = false;
  renderTimer();
  render();
  if (state.settings.timerSound) chime();
  clearTimeout(timerFinished.auto);
  timerFinished.auto = setTimeout(dismissAlert, 60000);
}
function dismissAlert() {
  stopChime();
  state.alert = false;
  render();
}
$("#alert-stop").addEventListener("click", (e) => {
  e.stopPropagation();
  dismissAlert();
});

// ---------------- звук ----------------
let audio = null, chimeLoop = 0;
function unlockAudio() {
  if (!audio) audio = new AudioContext();
  if (audio.state === "suspended") audio.resume();
}
function beep(at, freq) {
  const o = audio.createOscillator();
  const g = audio.createGain();
  o.type = "sine";
  o.frequency.value = freq;
  g.gain.setValueAtTime(0, at);
  g.gain.linearRampToValueAtTime(0.25, at + 0.02);
  g.gain.exponentialRampToValueAtTime(0.0001, at + 0.6);
  o.connect(g).connect(audio.destination);
  o.start(at);
  o.stop(at + 0.65);
}
function chime() {
  unlockAudio();
  const play = () => {
    const t = audio.currentTime;
    beep(t, 880); beep(t + 0.18, 1175); beep(t + 0.36, 1568);
  };
  play();
  let count = 0;
  chimeLoop = setInterval(() => { if (++count > 8) stopChime(); else play(); }, 1500);
}
function stopChime() { clearInterval(chimeLoop); }

// ---------------- секундомер ----------------
const sw = {
  running: false,
  startAt: 0,
  acc: 0,
  laps: [],
  elapsed() { return this.acc + (this.running ? Date.now() - this.startAt : 0); },
};

function renderStopwatch() {
  $("#sw-time").textContent = fmtStopwatch(sw.elapsed());
  const main = $("#sw-main");
  main.textContent = sw.running ? "Стоп" : sw.acc ? "Дальше" : "Старт";
  main.className = "pill " + (sw.running ? "red" : "green");
  $("#sw-secondary").textContent = sw.running ? "Круг" : "Сброс";
  const n = sw.laps.length;
  $("#sw-lap").textContent = n ? `Круг ${n}: ${fmtStopwatch(sw.laps[n - 1])}` : "";
}

$("#sw-main").addEventListener("click", (e) => {
  e.stopPropagation();
  if (sw.running) {
    sw.acc += Date.now() - sw.startAt;
    sw.running = false;
  } else {
    sw.startAt = Date.now();
    sw.running = true;
  }
  renderStopwatch();
  render();
});
$("#sw-secondary").addEventListener("click", (e) => {
  e.stopPropagation();
  if (sw.running) {
    const total = sw.elapsed();
    const prev = sw.laps.reduce((a, b) => a + b, 0);
    sw.laps.push(total - prev);
  } else {
    sw.acc = 0;
    sw.laps = [];
  }
  renderStopwatch();
});


// ---------------- музыка ----------------
const media = {
  info: { active: false },
  artKey: "",
  artUrl: null,
  basePos: 0,     // позиция на момент baseAt
  baseAt: 0,
  lastKey: "",
};

const trackKey = (m) => [m.source, m.title, m.artist, m.album].join("|");

function fmtClock(ms) {
  const t = Math.max(0, Math.floor(ms / 1000));
  const h = Math.floor(t / 3600), m = Math.floor((t % 3600) / 60), s = t % 60;
  return h ? `${h}:${pad2(m)}:${pad2(s)}` : `${m}:${pad2(s)}`;
}

function mediaPosition() {
  const m = media.info;
  if (!m.durationMs) return 0;
  const pos = media.basePos + (m.playing ? Date.now() - media.baseAt : 0);
  return Math.min(Math.max(0, pos), m.durationMs);
}

function applyMedia(info) {
  const key = trackKey(info);
  const trackChanged = key !== media.lastKey;
  media.info = info;
  media.lastKey = key;

  // позиция: если плеер сообщил время обновления — досчитываем от него
  const now = Date.now();
  const fresh = info.updatedAt > 0 && info.updatedAt <= now + 1000 && now - info.updatedAt < 3600_000;
  media.basePos = info.positionMs + (fresh && info.playing ? now - info.updatedAt : 0);
  media.baseAt = now;

  if (trackChanged) {
    if (media.artKey !== key) setArt(null, "");
    ["#m-track", "#m-art"].forEach((sel) => {
      const el = $(sel);
      el.classList.remove("swap");
      void el.offsetWidth;
      el.classList.add("swap");
    });
  }
  renderMedia();
  render();
}

function setArt(url, key) {
  media.artUrl = url;
  media.artKey = key;
  const img = $("#m-img");
  if (url) {
    img.src = url;
    img.hidden = false;
    img.onload = () => island.style.setProperty("--accent", accentFrom(img));
  } else {
    img.hidden = true;
    img.removeAttribute("src");
    island.style.setProperty("--accent", "#fff");
  }
  renderActivity();
}

// средний цвет обложки, подкрученный по яркости и насыщенности
function accentFrom(img) {
  try {
    const c = document.createElement("canvas");
    c.width = c.height = 12;
    const ctx = c.getContext("2d", { willReadFrequently: true });
    ctx.drawImage(img, 0, 0, 12, 12);
    const d = ctx.getImageData(0, 0, 12, 12).data;
    let r = 0, g = 0, b = 0, w = 0;
    for (let i = 0; i < d.length; i += 4) {
      const R = d[i], G = d[i + 1], B = d[i + 2];
      const sat = Math.max(R, G, B) - Math.min(R, G, B);
      const k = 1 + sat; // насыщенные пиксели важнее серых
      r += R * k; g += G * k; b += B * k; w += k;
    }
    let [h, s, l] = rgbToHsl(r / w, g / w, b / w);
    s = s < 0.08 ? s : Math.max(s, 0.55);
    l = Math.min(Math.max(l, 0.6), 0.75);
    return `hsl(${Math.round(h * 360)} ${Math.round(s * 100)}% ${Math.round(l * 100)}%)`;
  } catch {
    return "#fff";
  }
}
function rgbToHsl(r, g, b) {
  r /= 255; g /= 255; b /= 255;
  const max = Math.max(r, g, b), min = Math.min(r, g, b);
  let h = 0, s = 0;
  const l = (max + min) / 2;
  if (max !== min) {
    const d = max - min;
    s = l > 0.5 ? d / (2 - max - min) : d / (max + min);
    h = max === r ? (g - b) / d + (g < b ? 6 : 0) : max === g ? (b - r) / d + 2 : (r - g) / d + 4;
    h /= 6;
  }
  return [h, s, l];
}

function renderMedia() {
  const m = media.info;
  const active = !!m.active;
  $("#m-title").textContent = active ? m.title || "Без названия" : "Ничего не играет";
  $("#m-artist").textContent = active ? m.artist || m.album || "" : "Включите музыку в любом плеере";
  $("#m-app").textContent = active ? m.app || "" : "";
  $("#m-eq").classList.toggle("playing", active && m.playing);
  $("#m-eq").hidden = !active;
  $("#m-play-ico").setAttribute("href", m.playing ? "#i-pause" : "#i-play");
  $("#m-prev").disabled = !active || !m.canPrev;
  $("#m-next").disabled = !active || !m.canNext;
  $("#m-play").disabled = !active || !m.canPlayPause;
  $("#m-bottom").classList.toggle("no-time", !active || !m.durationMs);
  renderProgress();
}

function renderProgress() {
  const m = media.info;
  if (!m.active || !m.durationMs) return;
  const pos = mediaPosition();
  $("#m-bar").style.width = ((pos / m.durationMs) * 100).toFixed(2) + "%";
  $("#m-pos").textContent = fmtClock(pos);
  $("#m-left").textContent = "−" + fmtClock(m.durationMs - pos);
}

function control(action, positionMs) {
  invoke("media_control", { action, positionMs: positionMs ?? null }).catch(() => {});
}

$("#m-play").addEventListener("click", (e) => {
  e.stopPropagation();
  // сразу показываем результат, не дожидаясь плеера
  media.basePos = mediaPosition();
  media.baseAt = Date.now();
  media.info = { ...media.info, playing: !media.info.playing };
  renderMedia();
  render();
  control("toggle");
});
$("#m-prev").addEventListener("click", (e) => { e.stopPropagation(); control("prev"); });
$("#m-next").addEventListener("click", (e) => { e.stopPropagation(); control("next"); });
$("#m-progress").addEventListener("click", (e) => {
  e.stopPropagation();
  const m = media.info;
  if (!m.active || !m.durationMs) return;
  const r = e.currentTarget.getBoundingClientRect();
  const pos = Math.round(((e.clientX - r.left) / r.width) * m.durationMs);
  media.basePos = pos;
  media.baseAt = Date.now();
  renderProgress();
  control("seek", pos);
});

listen("media", (e) => applyMedia(e.payload));
listen("media-art", (e) => {
  const { key, url } = e.payload;
  if (key === media.lastKey || !media.lastKey) setArt(url, key);
});


// ---------------- буфер обмена ----------------
const clip = {
  items: [],
  flashUntil: 0,
  seen: new Set(),
};

function timeAgo(ms) {
  const s = Math.floor((Date.now() - ms) / 1000);
  if (s < 45) return "только что";
  const m = Math.floor(s / 60);
  if (m < 60) return `${Math.max(1, m)} мин назад`;
  const h = Math.floor(m / 60);
  if (h < 24) return `${h} ч назад`;
  return new Date(ms).toLocaleDateString("ru-RU", { day: "numeric", month: "short" });
}

function clipMeta(it) {
  const when = timeAgo(it.at);
  if (it.kind === "image") return `Картинка ${it.width}×${it.height}, ${when}`;
  return it.chars > 80 ? `${it.chars} симв., ${when}` : when;
}

function clipRow(it, isNew) {
  const row = document.createElement("div");
  row.className = "clip-row" + (it.pinned ? " pinned" : "") + (isNew ? " new" : "");
  row.setAttribute("role", "listitem");
  row.tabIndex = 0;
  row.title = "Нажмите, чтобы скопировать";
  row.dataset.id = it.id;

  const thumb = document.createElement("div");
  thumb.className = "clip-thumb";
  if (it.kind === "image" && it.thumb) {
    const img = document.createElement("img");
    img.src = it.thumb;
    img.alt = "";
    thumb.append(img);
  } else {
    thumb.textContent = /^https?:\/\//i.test(it.text.trim()) ? "🔗" : "Aa";
  }

  const body = document.createElement("div");
  body.className = "clip-body";
  const text = document.createElement("div");
  text.className = "clip-text";
  text.textContent = it.kind === "image" ? "Изображение" : it.text.replace(/\s+/g, " ").trim();
  const meta = document.createElement("div");
  meta.className = "clip-meta";
  meta.textContent = clipMeta(it);
  body.append(text, meta);

  const actions = document.createElement("div");
  actions.className = "clip-actions";
  actions.innerHTML = `
    <button class="clip-act pin" title="${it.pinned ? "Открепить" : "Закрепить"}"><svg><use href="#i-pin"/></svg></button>
    <button class="clip-act del" title="Удалить"><svg><use href="#i-trash"/></svg></button>`;

  row.append(thumb, body, actions);

  const copy = async () => {
    try {
      await invoke("clipboard_copy", { id: it.id });
      row.classList.add("copied");
      meta.textContent = "Скопировано";
      setTimeout(() => {
        row.classList.remove("copied");
        meta.textContent = clipMeta(it);
      }, 1400);
    } catch (err) {
      meta.textContent = "Не удалось скопировать: " + err;
    }
  };
  row.addEventListener("click", (e) => { e.stopPropagation(); copy(); });
  row.addEventListener("keydown", (e) => { if (e.key === "Enter") copy(); });
  actions.querySelector(".pin").addEventListener("click", (e) => {
    e.stopPropagation();
    invoke("clipboard_pin", { id: it.id });
  });
  actions.querySelector(".del").addEventListener("click", (e) => {
    e.stopPropagation();
    invoke("clipboard_delete", { id: it.id });
  });
  return row;
}

function renderClipboard() {
  const list = $("#clip-list");
  const scroll = list.scrollTop;
  list.replaceChildren(
    ...clip.items.map((it) => {
      const isNew = clip.seen.size > 0 && !clip.seen.has(it.id);
      return clipRow(it, isNew);
    })
  );
  clip.seen = new Set(clip.items.map((i) => i.id));
  list.scrollTop = scroll;
  $("#clip-empty").classList.toggle("on", clip.items.length === 0);
}

function applyClipboard(update) {
  clip.items = update.items;
  renderClipboard();
  if (update.added && state.settings.clipboardFlash && !state.expanded) {
    clip.flashUntil = Date.now() + 1500;
    setTimeout(render, 1600);
  }
  render();
}

$("#tab-clear").addEventListener("click", (e) => {
  e.stopPropagation();
  if (state.tab === "clipboard") invoke("clipboard_clear");
  if (state.tab === "files") invoke("shelf_clear");
});

listen("clipboard", (e) => applyClipboard(e.payload));

// подписи «N мин назад» обновляем раз в 30 секунд
setInterval(() => {
  if (state.expanded && state.tab === "clipboard") {
    document.querySelectorAll(".clip-row").forEach((row) => {
      const it = clip.items.find((i) => String(i.id) === row.dataset.id);
      if (it && !row.classList.contains("copied")) row.querySelector(".clip-meta").textContent = clipMeta(it);
    });
  }
}, 30000);


// ---------------- полка файлов ----------------
const shelf = {
  items: [],
  icons: {},          // id → data-URL значка от Windows
  iconRequested: new Set(),
  seen: new Set(),
  dropping: false,
  draggingOut: false, // сейчас вытаскиваем файл с полки — свои же события drop игнорируем
};

const IMAGE_EXT = ["png", "jpg", "jpeg", "gif", "webp", "bmp", "avif"];
const DOC_COLORS = {
  pdf: "#e5484d", doc: "#2b6cdf", docx: "#2b6cdf", xls: "#1f9d55", xlsx: "#1f9d55", csv: "#1f9d55",
  ppt: "#e5672b", pptx: "#e5672b", zip: "#a1772e", rar: "#a1772e", "7z": "#a1772e",
  mp3: "#d6409f", wav: "#d6409f", flac: "#d6409f", mp4: "#7c5cff", mkv: "#7c5cff", mov: "#7c5cff",
  txt: "#8e8e93", md: "#8e8e93", exe: "#5b6b7f", go: "#00a7d0", py: "#3572a5", rs: "#c8613a",
  kt: "#a97bff", java: "#b07219", js: "#d4b400", ts: "#3178c6", html: "#e34c26", json: "#6d6d72",
};

function fmtSize(bytes) {
  if (!bytes) return "";
  const units = ["Б", "КБ", "МБ", "ГБ"];
  let i = 0, v = bytes;
  while (v >= 1024 && i < units.length - 1) { v /= 1024; i++; }
  return `${v >= 10 || i === 0 ? Math.round(v) : v.toFixed(1)} ${units[i]}`;
}

function fileSrc(path) {
  const conv = window.__TAURI__.core.convertFileSrc;
  return conv ? conv(path) : path;
}

function tileIcon(it) {
  const box = document.createElement("div");
  box.className = "tile-icon";
  if (state.settings.fileIcons !== "simple" && it.exists) {
    // значки как в Проводнике: просим у Windows, пока ждём — простой значок
    const url = shelf.icons[it.id];
    if (url) {
      const img = document.createElement("img");
      img.className = "sys-icon";
      img.alt = "";
      img.draggable = false;
      img.src = url;
      box.append(img);
      return box;
    }
    if (!shelf.iconRequested.has(it.id)) {
      shelf.iconRequested.add(it.id);
      invoke("shelf_icon", { id: it.id }).catch(() => {});
    }
  }
  if (it.isDir) {
    box.innerHTML = `<svg class="folder"><use href="#i-folder"/></svg>`;
  } else if (IMAGE_EXT.includes(it.ext) && it.exists) {
    const img = document.createElement("img");
    img.alt = "";
    img.draggable = false;
    img.loading = "lazy";
    img.src = fileSrc(it.path);
    // не открылась картинка — показываем обычный значок
    img.onerror = () => { box.replaceChildren(docIcon(it)); };
    box.append(img);
  } else {
    box.append(docIcon(it));
  }
  return box;
}

function docIcon(it) {
  const d = document.createElement("div");
  d.className = "doc";
  d.style.setProperty("--doc", DOC_COLORS[it.ext] || "#8e8e93");
  d.textContent = (it.ext || "файл").slice(0, 4);
  return d;
}

function shelfTile(it, isNew) {
  const tile = document.createElement("div");
  tile.className = "tile" + (it.exists ? "" : " missing") + (isNew ? " new" : "");
  tile.setAttribute("role", "listitem");
  tile.title = it.exists ? `${it.path}\nПеретащите в любое окно, двойной клик — открыть` : `Файл не найден: ${it.path}`;
  tile.dataset.id = it.id;

  const name = document.createElement("div");
  name.className = "tile-name";
  name.textContent = it.name;
  const sub = document.createElement("div");
  sub.className = "tile-sub";
  sub.textContent = it.exists ? (it.isDir ? "Папка" : fmtSize(it.size)) : "Не найден";

  const acts = document.createElement("div");
  acts.className = "tile-acts";
  acts.innerHTML = `
    <button class="tile-act reveal" title="Показать в папке"><svg><use href="#i-reveal"/></svg></button>
    <button class="tile-act del" title="Убрать с полки"><svg><use href="#i-close"/></svg></button>`;
  if (!it.exists) acts.querySelector(".reveal").remove();

  tile.append(tileIcon(it), name, sub, acts);

  acts.querySelector(".del").addEventListener("click", (e) => {
    e.stopPropagation();
    invoke("shelf_remove", { id: it.id });
  });
  acts.querySelector(".reveal")?.addEventListener("click", (e) => {
    e.stopPropagation();
    invoke("shelf_reveal", { id: it.id });
  });
  tile.addEventListener("click", (e) => e.stopPropagation());
  tile.addEventListener("dblclick", (e) => {
    e.stopPropagation();
    if (it.exists) invoke("shelf_open", { id: it.id });
  });

  // вытаскивание наружу: зажали кнопку и сдвинули мышь на пару пикселей
  tile.addEventListener("mousedown", (e) => {
    if (e.button !== 0 || !it.exists || e.target.closest(".tile-act")) return;
    e.preventDefault();
    const sx = e.clientX, sy = e.clientY;
    const move = (m) => {
      if (Math.hypot(m.clientX - sx, m.clientY - sy) < 5) return;
      cleanup();
      tile.classList.add("dragging");
      shelf.draggingOut = true;
      clearTimeout(shelf.dragSafety);
      shelf.dragSafety = setTimeout(endDragOut, 30000); // на случай, если конец потеряется
      invoke("shelf_drag", { ids: [it.id] }).catch(endDragOut);
    };
    const cleanup = () => {
      window.removeEventListener("mousemove", move);
      window.removeEventListener("mouseup", cleanup);
    };
    window.addEventListener("mousemove", move);
    window.addEventListener("mouseup", cleanup);
  });
  return tile;
}

function renderShelf() {
  const grid = $("#shelf");
  const scroll = grid.scrollTop;
  grid.replaceChildren(
    ...shelf.items.map((it) => shelfTile(it, shelf.seen.size > 0 && !shelf.seen.has(it.id)))
  );
  shelf.seen = new Set(shelf.items.map((i) => i.id));
  grid.scrollTop = scroll;
  $("#shelf-empty").classList.toggle("on", shelf.items.length === 0);
}

function applyShelf(items) {
  shelf.items = items;
  renderShelf();
  render();
}
listen("shelf", (e) => applyShelf(e.payload));
listen("shelf-icon", (e) => {
  const { id, url } = e.payload;
  shelf.icons[id] = url;
  const tile = document.querySelector(`.tile[data-id="${id}"]`);
  if (tile && state.settings.fileIcons !== "simple") {
    const it = shelf.items.find((i) => i.id === id);
    if (it) tile.querySelector(".tile-icon").replaceWith(tileIcon(it));
  }
});

function endDragOut() {
  shelf.draggingOut = false;
  shelf.dropping = false;
  document.querySelectorAll(".tile.dragging").forEach((t) => t.classList.remove("dragging"));
  render();
}
listen("shelf-drag-end", endDragOut);

// файлы тащат на остров из Проводника
listen("tauri://drag-enter", () => {
  if (shelf.draggingOut) return;
  shelf.dropping = true;
  state.tab = "files";
  clearTimeout(closeTimer);
  expand();
  render();
});
listen("tauri://drag-over", () => {
  if (!shelf.draggingOut && !shelf.dropping) {
    shelf.dropping = true;
    render();
  }
});
listen("tauri://drag-leave", () => {
  shelf.dropping = false;
  render();
});
listen("tauri://drag-drop", (e) => {
  shelf.dropping = false;
  if (shelf.draggingOut) return render();
  const paths = (e.payload && e.payload.paths) || [];
  if (paths.length) invoke("shelf_add", { paths });
  render();
});

// ---------------- громкость и яркость ----------------
const osd = { until: 0, volume: null, brightness: null, kind: "volume" };

function showOsd(kind) {
  if (!state.settings.osd || state.hidden) return;
  osd.kind = kind;
  osd.until = Date.now() + 1600;
  renderOsd();
  render();
  clearTimeout(showOsd.t);
  showOsd.t = setTimeout(render, 1650);
}

function renderOsd() {
  const layer = document.querySelector('.layer[data-layer="osd"]');
  let pct, icon, muted = false;
  if (osd.kind === "brightness") {
    pct = osd.brightness ?? 0;
    icon = "#i-sun";
  } else {
    const v = osd.volume || { level: 0, muted: false };
    pct = Math.round(v.level * 100);
    muted = v.muted || pct === 0;
    icon = muted ? "#i-speaker-off" : pct < 50 ? "#i-speaker-low" : "#i-speaker";
  }
  layer.classList.toggle("osd-muted", muted);
  $("#osd-ico-use").setAttribute("href", icon);
  $("#osd-fill").style.width = pct + "%";
  $("#osd-val").textContent = pct;
}

listen("volume", (e) => {
  const first = osd.volume === null;
  osd.volume = e.payload;
  if (!first) showOsd("volume");
});
listen("brightness", (e) => {
  const first = osd.brightness === null;
  osd.brightness = e.payload;
  if (!first) showOsd("brightness");
});

// колесо мыши над свёрнутым островом — громкость
island.addEventListener("wheel", (e) => {
  if (state.expanded || state.alert || !state.settings.scrollVolume || !osd.volume) return;
  e.preventDefault();
  const step = e.deltaY < 0 ? 0.02 : -0.02;
  const level = Math.min(1, Math.max(0, Math.round((osd.volume.level + step) * 50) / 50));
  osd.volume = { level, muted: level === 0 ? osd.volume.muted : false };
  showOsd("volume");
  invoke("set_volume", { level });
}, { passive: false });

// ---------------- тики ----------------
function frame() {
  if (timer.running && !timer.paused && timer.remaining() <= 0) timerFinished();
  if (timer.running) renderTimer();
  if (sw.running) renderStopwatch();
  if (timer.running || sw.running) renderActivity();
  if (state.expanded && state.tab === "music" && media.info.playing) renderProgress();
  requestAnimationFrame(frame);
}
requestAnimationFrame(frame);

// ---------------- события из Rust ----------------
function applyBattery(b) {
  const prev = state.battery;
  state.battery = b;
  if (prev && b.hasBattery && !prev.charging && b.charging && state.settings.chargingActivity) {
    state.chargingFlashUntil = Date.now() + 3500;
    island.classList.remove("bump");
    void island.offsetWidth;
    island.classList.add("bump");
    setTimeout(render, 3600);
  }
  render();
}
listen("battery", (e) => applyBattery(e.payload));

listen("fullscreen", (e) => {
  state.hidden = e.payload;
  if (state.hidden) state.expanded = false;
  useSpring(SPRING_CLOSE);
  render();
});

listen("settings-changed", (e) => {
  const iconsChanged = state.settings.fileIcons !== e.payload.fileIcons;
  state.settings = e.payload;
  if (iconsChanged) renderShelf();
  render();
});

// ---------------- старт ----------------
(async () => {
  try { state.settings = await invoke("get_settings"); } catch {}
  try { state.battery = await invoke("get_battery"); } catch {}
  try {
    applyClipboard({ items: await invoke("get_clipboard"), added: false });
  } catch {}
  try { applyShelf(await invoke("shelf_get")); } catch {}
  try {
    const [volume, brightness] = await invoke("get_levels");
    if (volume) osd.volume = volume;
    if (brightness !== null && brightness !== undefined) osd.brightness = brightness;
  } catch {}
  try {
    const [info, art] = await invoke("get_media");
    applyMedia(info);
    if (art) setArt(art.url, art.key);
  } catch {}
  renderTimer();
  renderStopwatch();
  render();
})();
