import { LANGS, setLang, t, applyLang } from "./i18n.js";

const { invoke } = window.__TAURI__.core;
const { emit, listen } = window.__TAURI__.event;

// Панель настроек рендерится этим же файлом, но в отдельном окне (?panel=settings|add).
const panelKind = new URLSearchParams(location.search).get("panel");

const PRESETS = [
  { id: "claude", name: "Claude", url: "https://claude.ai" },
  { id: "chatgpt", name: "ChatGPT", url: "https://chatgpt.com" },
  { id: "gemini", name: "Gemini", url: "https://gemini.google.com" },
  { id: "grok", name: "Grok", url: "https://grok.com" },
  { id: "perplexity", name: "Perplexity", url: "https://perplexity.ai" },
  { id: "deepseek", name: "DeepSeek", url: "https://chat.deepseek.com" },
];

const tabList = document.getElementById("tab-list");
const panel = document.getElementById("panel");
const panelTitle = document.getElementById("panel-title");
const viewSettings = document.getElementById("view-settings");
const viewAdd = document.getElementById("view-add");
const tabRows = document.getElementById("tab-rows");
const urlInput = document.getElementById("current-url");
const restoreSwitch = document.getElementById("restore-session");
const presetsEl = document.getElementById("presets");
const langSelect = document.getElementById("lang");
const customValue = document.getElementById("custom-value");
const customUnit = document.getElementById("custom-unit");
const themeSeg = document.getElementById("theme-seg");
const timeoutSeg = document.getElementById("timeout-seg");

let tabs = [];
let activeId = null;
let settings = {};
let openView = null;

const favicon = (url) => `https://www.google.com/s2/favicons?domain=${new URL(url).hostname}&sz=64`;

/* ---------- Сайдбар ---------- */

// Иконка приложения — последний рубеж, когда ни локальный файл, ни favicon не открылись.
const FALLBACK_ICON = "/assets/icon.png";

function iconContent(tab) {
  const src = tab.icon.startsWith("local:") ? `/icons/${tab.icon.slice(6)}` : favicon(tab.url);
  return `<img src="${src}" alt="" onerror="this.onerror=null; this.src='${FALLBACK_ICON}'" />`;
}

function render() {
  tabList.innerHTML = "";
  for (const tab of tabs) {
    const btn = document.createElement("button");
    btn.className = "tab-icon" + (tab.id === activeId ? " active" : "");
    btn.title = tab.name;
    btn.innerHTML = iconContent(tab);
    if (tab.never_unload) btn.classList.add("pinned");
    btn.addEventListener("click", () => selectTab(tab));
    btn.addEventListener("contextmenu", (e) => {
      e.preventDefault();
      toggleNeverUnload(tab);
    });
    tabList.appendChild(btn);
  }
}

async function selectTab(tab, resetToBase = false) {
  activeId = tab.id;
  render();
  try {
    const url = !resetToBase && settings.restore_session && tab.last_url ? tab.last_url : tab.url;
    await invoke("switch_tab", { id: tab.id, url });
  } catch (err) {
    console.error("switch_tab failed", tab.id, err);
  }
}

async function toggleNeverUnload(tab) {
  tab.never_unload = !tab.never_unload;
  await invoke("save_tabs", { tabs });
  render();
  await emit("tabs-changed");
}

/* ---------- Боковая панель ---------- */

async function renderPanel() {
  openView = panelKind;
  panelTitle.textContent = t(panelKind === "add" ? "newTab" : "settings");
  viewSettings.hidden = panelKind !== "settings";
  viewAdd.hidden = panelKind !== "add";
  panel.hidden = false;

  if (panelKind === "add") {
    renderPresets();
  } else {
    renderTabRows();
    urlInput.value = (await invoke("active_url")) ?? "";
    restoreSwitch.checked = settings.restore_session;
    markSegment(themeSeg, settings.theme);
    renderTimeout();
  }
}

const closePanel = () => invoke("close_panel");

function renderTabRows() {
  tabRows.innerHTML = "";
  for (const tab of tabs) {
    const row = document.createElement("div");
    row.className = "tab-row" + (tab.id === activeId ? " current" : "");
    row.innerHTML = `<img src="${favicon(tab.url)}" alt="" /><span>${tab.name}</span>`;
    if (tab.never_unload) {
      row.insertAdjacentHTML("beforeend", `<span class="pin" title="${t("pinned")}">📌</span>`);
    }
    const del = document.createElement("button");
    del.type = "button";
    del.className = "del";
    del.textContent = "✕";
    del.title = t("deleteTab");
    del.addEventListener("click", () => removeTab(tab));
    row.appendChild(del);
    tabRows.appendChild(row);
  }
}

function renderPresets() {
  presetsEl.innerHTML = "";
  for (const preset of PRESETS) {
    if (tabs.some((tab) => tab.id === preset.id)) continue;
    const btn = document.createElement("button");
    btn.type = "button";
    btn.className = "preset";
    btn.innerHTML = `<img src="${favicon(preset.url)}" alt="" /><span>${preset.name}</span>`;
    btn.addEventListener("click", () => addTab({ ...preset, icon: "favicon" }));
    presetsEl.appendChild(btn);
  }
}

async function addTab(tab) {
  if (tabs.some((t) => t.id === tab.id)) return;
  tabs.push(tab);
  await invoke("save_tabs", { tabs });
  await emit("select-tab", tab.id);
  closePanel();
}

async function removeTab(tab) {
  tabs = tabs.filter((t) => t.id !== tab.id);
  await invoke("save_tabs", { tabs });
  await invoke("remove_tab", { id: tab.id });
  renderTabRows();
  await emit("tabs-changed");
}

/* ---------- Настройки ---------- */

function markSegment(seg, value) {
  for (const btn of seg.children) btn.classList.toggle("on", btn.dataset.value === String(value));
}

// Своё значение показываем в минутах, если оно кратно минуте.
function renderTimeout() {
  const secs = settings.unload_timeout_secs;
  markSegment(timeoutSeg, secs ?? "never");
  if (secs == null) {
    customValue.value = "";
    return;
  }
  const inMinutes = secs % 60 === 0;
  customValue.value = inMinutes ? secs / 60 : secs;
  customUnit.value = inMinutes ? "60" : "1";
}

function applyTheme() {
  document.documentElement.dataset.theme = settings.theme;
  // Тема самих сайтов во вкладках (prefers-color-scheme внутри WebView2).
  if (!panelKind) invoke("set_webview_theme", { theme: settings.theme });
}

function applyLanguage() {
  setLang(settings.lang);
  document.documentElement.lang = settings.lang;
  applyLang();
  if (openView) panelTitle.textContent = t(openView === "add" ? "newTab" : "settings");
  renderTimeout();
}

const save = () => invoke("save_settings", { settings });

themeSeg.addEventListener("click", async (e) => {
  const btn = e.target.closest("button");
  if (!btn) return;
  settings.theme = btn.dataset.value;
  markSegment(themeSeg, settings.theme);
  applyTheme();
  await save();
  await emit("settings-changed", settings);
});

timeoutSeg.addEventListener("click", async (e) => {
  const btn = e.target.closest("button");
  if (!btn) return;
  settings.unload_timeout_secs = btn.dataset.value === "never" ? null : Number(btn.dataset.value);
  renderTimeout();
  await save();
});

for (const el of [customValue, customUnit]) {
  el.addEventListener("change", async () => {
    const secs = Number(customValue.value) * Number(customUnit.value);
    if (!secs) return;
    settings.unload_timeout_secs = secs;
    markSegment(timeoutSeg, secs);
    await save();
  });
}

restoreSwitch.addEventListener("change", async (e) => {
  settings.restore_session = e.target.checked;
  await save();
});

langSelect.addEventListener("change", async (e) => {
  settings.lang = e.target.value;
  applyLanguage();
  renderTabRows();
  await save();
  await emit("settings-changed", settings);
});

document.getElementById("copy-url").addEventListener("click", async (e) => {
  if (!urlInput.value) return;
  await navigator.clipboard.writeText(urlInput.value);
  const btn = e.currentTarget;
  btn.textContent = "✓";
  setTimeout(() => (btn.textContent = "🔗"), 1000);
});

document.getElementById("reset-url").addEventListener("click", async () => {
  await emit("reset-tab");
  closePanel();
});

document.getElementById("cancel-add").addEventListener("click", closePanel);
document.getElementById("close-btn").addEventListener("click", closePanel);
document.getElementById("add-tab-btn").addEventListener("click", () =>
  invoke("open_panel", { kind: "add" })
);
document.getElementById("settings-btn").addEventListener("click", () =>
  invoke("open_panel", { kind: "settings" })
);
// Контекстное меню WebView2 в собственном UI не нужно: перезагружать тут нечего.
document.addEventListener("contextmenu", (e) => e.preventDefault());

document.addEventListener("keydown", (e) => {
  if (e.key === "Escape" && panelKind) closePanel();
});

viewAdd.addEventListener("submit", (e) => {
  e.preventDefault();
  const name = document.getElementById("tab-name").value.trim();
  const url = document.getElementById("tab-url").value.trim();
  if (!name || !url) return;
  const id = name.toLowerCase().replace(/[^a-z0-9]+/g, "-") + "-" + Date.now();
  addTab({ id, name, url, icon: "favicon" });
  viewAdd.reset();
});

/* ---------- Старт ---------- */

async function init() {
  [tabs, settings] = await Promise.all([invoke("read_tabs"), invoke("read_settings")]);
  for (const [code, label] of Object.entries(LANGS)) {
    langSelect.add(new Option(label, code, false, code === settings.lang));
  }
  applyLanguage();
  applyTheme();

  if (panelKind) {
    document.body.classList.add("panel-mode");
    await renderPanel();
    return;
  }

  render();
  const last = settings.restore_session && tabs.find((t) => t.id === settings.last_active);
  const first = last || tabs[0];
  if (first) await selectTab(first);
}

// Панель живёт в своём окне и правит те же файлы — синхронизируем состояние.
if (!panelKind) {
  listen("tabs-changed", async () => {
    tabs = await invoke("read_tabs");
    render();
    if (!tabs.some((tab) => tab.id === activeId)) {
      activeId = null;
      if (tabs.length > 0) await selectTab(tabs[0]);
    }
  });

  listen("select-tab", async (e) => {
    tabs = await invoke("read_tabs");
    render();
    const tab = tabs.find((t) => t.id === e.payload);
    if (tab) await selectTab(tab);
  });

  listen("reset-tab", async () => {
    const tab = tabs.find((t) => t.id === activeId);
    if (!tab) return;
    await invoke("remove_tab", { id: tab.id });
    await selectTab(tab, true);
  });

  listen("settings-changed", (e) => {
    settings = e.payload;
    applyLanguage();
    applyTheme();
    render();
  });
}

init();
