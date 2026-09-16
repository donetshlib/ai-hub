import { LANGS, setLang, t, applyLang } from "./i18n.js";

const { invoke } = window.__TAURI__.core;
const { emit, listen } = window.__TAURI__.event;

// The settings panel is rendered by this same file, but in a separate window (?panel=settings|add).
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

const SIDEBAR_WIDTH = 56;
const PANE_GAP = 6;
const PANE_HEADER = 30;
const MAX_PANES = 10;

const paneChrome = document.getElementById("pane-chrome");

let tabs = [];
let panes = [];
let fractions = [];
let activeId = null;
let settings = {};
let openView = null;

const favicon = (url) => `https://www.google.com/s2/favicons?domain=${new URL(url).hostname}&sz=64`;

/* ---------- Sidebar ---------- */

// The app icon is the last resort when neither a local file nor the favicon loads.
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
      invoke("open_menu", { id: tab.id, x: e.screenX, y: e.screenY });
    });
    tabList.appendChild(btn);
  }
}

function tabUrl(tab, resetToBase = false) {
  return !resetToBase && settings.restore_session && tab.last_url ? tab.last_url : tab.url;
}

// A click on the sidebar replaces the first pane; a tab already on screen just takes focus.
async function selectTab(tab, resetToBase = false) {
  if (panes.some((pane) => pane.id === tab.id) && !resetToBase) return;
  const entry = { id: tab.id, url: tabUrl(tab, resetToBase) };
  if (panes.length === 0) {
    panes = [entry];
    fractions = [1];
  } else {
    panes[0] = entry;
  }
  activeId = tab.id;
  render();
  await applyLayout();
}

// "Open beside": a new pane on the right, widths split evenly.
async function openBeside(tab) {
  if (panes.some((pane) => pane.id === tab.id)) return;
  if (panes.length >= MAX_PANES) {
    toast(t("paneLimit"));
    return;
  }
  panes.push({ id: tab.id, url: tabUrl(tab) });
  fractions = panes.map(() => 1 / panes.length);
  render();
  await applyLayout();
}

async function closePane(index) {
  const [removed] = panes.splice(index, 1);
  fractions = panes.map(() => 1 / panes.length);
  await invoke("close_tab", { id: removed.id });
  activeId = panes[0]?.id ?? null;
  render();
  await applyLayout();
}

// Reordering by arrows rather than drag and drop: the pane headers are a thin strip of the
// main webview, and a drag across them would pass over the tab webviews, which swallow it.
async function movePane(index, delta) {
  const target = index + delta;
  if (target < 0 || target >= panes.length) return;
  [panes[index], panes[target]] = [panes[target], panes[index]];
  await applyLayout();
}

async function toggleNeverUnload(tab) {
  tab.never_unload = !tab.never_unload;
  await invoke("save_tabs", { tabs });
  render();
  await emit("tabs-changed");
}

/* ---------- Split view ---------- */

// Rectangles are computed here, not in Rust: this side knows the sidebar, the headers
// and the dividers. Rust only applies them.
function paneRects() {
  const header = panes.length > 1 ? PANE_HEADER : 0;
  const contentW = window.innerWidth - SIDEBAR_WIDTH;
  const usable = contentW - PANE_GAP * (panes.length - 1);
  let x = SIDEBAR_WIDTH;
  return panes.map((pane, i) => {
    const width = usable * fractions[i];
    const rect = { ...pane, x, y: header, width, height: window.innerHeight - header };
    x += width + PANE_GAP;
    return rect;
  });
}

async function applyLayout() {
  const rects = paneRects();
  renderPaneChrome(rects);
  try {
    await invoke("sync_panes", { panes: rects });
  } catch (err) {
    console.error("sync_panes failed", err);
  }
}

function renderPaneChrome(rects) {
  paneChrome.innerHTML = "";
  if (panes.length < 2) return;

  rects.forEach((rect, i) => {
    const head = document.createElement("div");
    head.className = "pane-head";
    head.style.left = `${rect.x}px`;
    head.style.width = `${rect.width}px`;
    head.style.height = `${PANE_HEADER}px`;

    const name = tabs.find((tab) => tab.id === rect.id)?.name ?? rect.id;
    head.innerHTML = `<span class="pane-name">${name}</span>`;

    for (const [delta, label] of [[-1, "‹"], [1, "›"]]) {
      const move = document.createElement("button");
      move.type = "button";
      move.textContent = label;
      move.title = t("movePane");
      move.addEventListener("click", () => movePane(i, delta));
      head.appendChild(move);
    }

    const close = document.createElement("button");
    close.type = "button";
    close.className = "pane-close";
    close.textContent = "✕";
    close.title = t("closePane");
    close.addEventListener("click", () => closePane(i));
    head.appendChild(close);

    paneChrome.appendChild(head);

    if (i < rects.length - 1) {
      const divider = document.createElement("div");
      divider.className = "divider";
      divider.style.left = `${rect.x + rect.width}px`;
      divider.style.width = `${PANE_GAP}px`;
      divider.addEventListener("mousedown", (e) => startDividerDrag(e, i));
      paneChrome.appendChild(divider);
    }
  });
}

// Dragging a divider moves width between the two neighbouring panes only.
function startDividerDrag(event, index) {
  event.preventDefault();
  const usable = window.innerWidth - SIDEBAR_WIDTH - PANE_GAP * (panes.length - 1);
  const startX = event.clientX;
  const left = fractions[index];
  const right = fractions[index + 1];
  const minimum = 0.08;

  const onMove = (e) => {
    const delta = (e.clientX - startX) / usable;
    const nextLeft = left + delta;
    const nextRight = right - delta;
    if (nextLeft < minimum || nextRight < minimum) return;
    fractions[index] = nextLeft;
    fractions[index + 1] = nextRight;
    applyLayout();
  };
  const onUp = () => {
    document.removeEventListener("mousemove", onMove);
    document.removeEventListener("mouseup", onUp);
  };
  document.addEventListener("mousemove", onMove);
  document.addEventListener("mouseup", onUp);
}

// The pane headers are the only always-visible strip of the main webview in split mode,
// so a message can be shown there and nowhere else.
function toast(text) {
  const el = document.createElement("div");
  el.className = "toast";
  el.textContent = text;
  document.body.appendChild(el);
  setTimeout(() => el.remove(), 2600);
}

/* ---------- Side panel ---------- */

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
  await invoke("close_tab", { id: tab.id });
  renderTabRows();
  await emit("tabs-changed");
}

/* ---------- Settings ---------- */

function markSegment(seg, value) {
  for (const btn of seg.children) btn.classList.toggle("on", btn.dataset.value === String(value));
}

// Show a custom value in minutes when it is a whole number of minutes.
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
  // Theme of the sites inside the tabs (prefers-color-scheme within WebView2).
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

// Two-step button: the first click only arms it, so accounts are never dropped by a stray click.
let clearArmed = false;
document.getElementById("clear-data").addEventListener("click", async (e) => {
  const btn = e.currentTarget;
  if (!clearArmed) {
    clearArmed = true;
    btn.textContent = t("signOutConfirm");
    btn.classList.add("armed");
    setTimeout(() => {
      clearArmed = false;
      btn.textContent = t("signOutAll");
      btn.classList.remove("armed");
    }, 4000);
    return;
  }
  await invoke("clear_browsing_data");
  // The tab webviews are gone now, so the main window reopens the active one from scratch.
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
// The WebView2 context menu is useless in our own UI: there is nothing to reload here.
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

window.addEventListener("resize", applyLayout);

/* ---------- Sidebar context menu (its own window) ---------- */

function renderMenu() {
  const id = new URLSearchParams(location.search).get("tab");
  const tab = tabs.find((item) => item.id === id);
  if (!tab) return;

  const menu = document.getElementById("menu");
  document.getElementById("menu-pin").textContent = t(tab.never_unload ? "unpin" : "pin");
  applyLang(menu);
  menu.hidden = false;

  menu.addEventListener("click", async (e) => {
    const action = e.target.closest("button")?.dataset.action;
    if (!action) return;
    await emit("menu-action", { action, id });
    invoke("close_menu");
  });
  // Losing focus is the usual way a context menu disappears.
  window.addEventListener("blur", () => invoke("close_menu"));
}

/* ---------- Startup ---------- */

async function init() {
  [tabs, settings] = await Promise.all([invoke("read_tabs"), invoke("read_settings")]);
  for (const [code, label] of Object.entries(LANGS)) {
    langSelect.add(new Option(label, code, false, code === settings.lang));
  }
  applyLanguage();
  applyTheme();

  if (panelKind === "menu") {
    document.body.classList.add("menu-mode");
    renderMenu();
    return;
  }

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

// The panel lives in its own window and edits the same files, so keep the state in sync.
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

  listen("menu-action", async (e) => {
    const { action, id } = e.payload;
    const tab = tabs.find((item) => item.id === id);
    if (!tab) return;
    if (action === "beside") await openBeside(tab);
    if (action === "pin") await toggleNeverUnload(tab);
    if (action === "remove") await removeTab(tab);
    if (action === "reload") {
      await invoke("close_tab", { id: tab.id });
      await applyLayout();
    }
  });

  listen("reset-tab", async () => {
    const tab = tabs.find((t) => t.id === activeId);
    if (!tab) return;
    await invoke("close_tab", { id: tab.id });
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
