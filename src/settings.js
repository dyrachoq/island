const { invoke } = window.__TAURI__.core;
const $ = (s) => document.querySelector(s);

let settings = null;
let saveTimer = 0;

function render() {
  document.querySelectorAll("input[data-key]").forEach((el) => {
    const v = settings[el.dataset.key];
    if (el.type === "checkbox") el.checked = !!v;
    else el.value = v;
  });
  document.querySelectorAll(".segmented").forEach((seg) => {
    seg.querySelectorAll("button").forEach((b) => {
      const on = String(settings[seg.dataset.key]) === b.dataset.value;
      b.classList.toggle("active", on);
      b.setAttribute("aria-checked", on);
    });
  });
  // прятать системный индикатор имеет смысл, только если показываем свой
  const hideRow = $("#row-hide-system");
  hideRow.classList.toggle("disabled", !settings.osd);
  hideRow.querySelector("input").disabled = !settings.osd;
  $("#offset-value").textContent = `${settings.topOffset} px`;
  $("#preview-pill").parentElement.style.setProperty("--offset", `${4 + settings.topOffset / 4}px`);
}

function setStatus(text, error = false) {
  const el = $("#status");
  el.textContent = text;
  el.classList.toggle("error", error);
}

function save(delay = 0) {
  clearTimeout(saveTimer);
  saveTimer = setTimeout(async () => {
    try {
      await invoke("save_settings", { value: settings });
      setStatus("Сохранено");
    } catch (e) {
      setStatus(String(e), true);
      settings = await invoke("get_settings");
      render();
    }
  }, delay);
}

document.querySelectorAll("input[data-key]").forEach((el) => {
  el.addEventListener("input", () => {
    const key = el.dataset.key;
    settings[key] = el.type === "checkbox" ? el.checked : Number(el.value);
    render();
    save(el.type === "range" ? 250 : 0);
  });
});

document.querySelectorAll(".segmented").forEach((seg) =>
  seg.querySelectorAll("button").forEach((b) =>
    b.addEventListener("click", () => {
      const v = b.dataset.value;
      settings[seg.dataset.key] = v === "true" ? true : v === "false" ? false : v;
      render();
      save();
    })
  )
);

$("#quit").addEventListener("click", () => invoke("quit"));

(async () => {
  settings = await invoke("get_settings");
  render();
})();
