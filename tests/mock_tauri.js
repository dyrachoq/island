// Подмена Tauri для тестов в обычном Chromium: записывает вызовы invoke
// и позволяет «присылать» события, как будто их шлёт Rust.
(() => {
  const calls = [];
  const listeners = {};
  const responses = {
    get_settings: {
      autostart: false, hideInFullscreen: true, expandOnHover: true, topOffset: 8,
      chargingActivity: true, timerSound: false, clipboardHistory: true, clipboardFlash: true,
      osd: true, scrollVolume: true, hideSystemOsd: true, fileIcons: "system",
    },
    save_settings: null,
    get_battery: { hasBattery: true, percent: 64, charging: false },
    get_media: [{ active: false }, null],
    get_clipboard: [],
    shelf_get: [],
    get_levels: [{ level: 0.4, muted: false }, 70],
  };
  window.__calls = calls;
  window.__responses = responses;
  window.__TAURI__ = {
    core: {
      invoke(cmd, args) {
        calls.push([cmd, args ?? null]);
        const r = responses[cmd];
        if (r instanceof Error) return Promise.reject(r.message);
        // настоящий IPC всегда отдаёт копию данных
        const value = typeof r === "function" ? r(args) : r;
        return Promise.resolve(value === undefined ? null : structuredClone(value));
      },
      convertFileSrc: (p) => "asset://localhost/" + encodeURIComponent(p),
    },
    event: {
      listen(ev, cb) {
        (listeners[ev] ||= []).push(cb);
        return Promise.resolve(() => {});
      },
    },
  };
  window.__emit = (ev, payload) => (listeners[ev] || []).forEach((cb) => cb({ event: ev, payload }));
  window.__callsOf = (cmd) => calls.filter((c) => c[0] === cmd);
})();
