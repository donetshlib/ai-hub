use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tauri::{Manager, PhysicalPosition, PhysicalSize, Webview, WebviewUrl};

const SIDEBAR_WIDTH: u32 = 56;
/// Inactive webviews are parked here: on Windows hide() removes the child HWND neither
/// from rendering nor from the click-hit area.
const OFFSCREEN: PhysicalPosition<i32> = PhysicalPosition::new(-10000, -10000);
const PANEL_LABEL: &str = "__panel";
const MENU_LABEL: &str = "__menu";
/// Settings panel width in logical pixels.
const PANEL_WIDTH: f64 = 380.0;
const UNLOAD_CHECK_INTERVAL: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Tab {
    id: String,
    name: String,
    url: String,
    icon: String,
    #[serde(default)]
    never_unload: bool,
    /// Last URL inside the service, used to restore the session.
    #[serde(default)]
    last_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Settings {
    /// None = "never unload". Seconds; an arbitrary value can be set in the settings.
    #[serde(default = "default_unload")]
    unload_timeout_secs: Option<u64>,
    #[serde(default = "default_theme")]
    theme: String,
    /// Open the same tab and URL that were active on exit.
    #[serde(default)]
    restore_session: bool,
    #[serde(default)]
    last_active: Option<String>,
    #[serde(default = "default_lang")]
    lang: String,
    /// Window geometry from the previous run: logical x, y, width, height.
    #[serde(default)]
    window: Option<[f64; 4]>,
    #[serde(default)]
    maximized: bool,
}

fn default_lang() -> String {
    "ru".into()
}

fn default_unload() -> Option<u64> {
    Some(900)
}

fn default_theme() -> String {
    "dark".into()
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            unload_timeout_secs: default_unload(),
            theme: default_theme(),
            restore_session: false,
            last_active: None,
            lang: default_lang(),
            window: None,
            maximized: false,
        }
    }
}

struct AppState {
    webviews: Mutex<HashMap<String, Webview>>,
    active_tab: Mutex<Option<String>>,
    /// When the tab became inactive (hidden). Drives the unload timeout.
    hidden_since: Mutex<HashMap<String, Instant>>,
    settings: Mutex<Settings>,
}

fn default_tabs() -> Vec<Tab> {
    vec![
        Tab { id: "claude".into(), name: "Claude".into(), url: "https://claude.ai".into(), icon: "favicon".into(), never_unload: false, last_url: None },
        Tab { id: "chatgpt".into(), name: "ChatGPT".into(), url: "https://chatgpt.com".into(), icon: "favicon".into(), never_unload: false, last_url: None },
        Tab { id: "gemini".into(), name: "Gemini".into(), url: "https://gemini.google.com".into(), icon: "favicon".into(), never_unload: false, last_url: None },
        Tab { id: "grok".into(), name: "Grok".into(), url: "https://grok.com".into(), icon: "favicon".into(), never_unload: false, last_url: None },
        Tab { id: "deepseek".into(), name: "DeepSeek".into(), url: "https://chat.deepseek.com".into(), icon: "favicon".into(), never_unload: false, last_url: None },
    ]
}

fn config_dir(app: &tauri::AppHandle) -> std::path::PathBuf {
    let dir = app.path().app_config_dir().expect("no app config dir");
    std::fs::create_dir_all(&dir).ok();
    dir
}

fn tabs_path(app: &tauri::AppHandle) -> std::path::PathBuf {
    config_dir(app).join("tabs.json")
}

fn settings_path(app: &tauri::AppHandle) -> std::path::PathBuf {
    config_dir(app).join("settings.json")
}

#[tauri::command]
fn read_tabs(app: tauri::AppHandle) -> Vec<Tab> {
    let path = tabs_path(&app);
    if let Ok(data) = std::fs::read_to_string(&path) {
        if let Ok(tabs) = serde_json::from_str(&data) {
            return tabs;
        }
    }
    let tabs = default_tabs();
    save_tabs(app, tabs.clone());
    tabs
}

#[tauri::command]
fn save_tabs(app: tauri::AppHandle, mut tabs: Vec<Tab>) {
    let path = tabs_path(&app);
    // The frontend keeps its own copy of the list and knows nothing about last_url, which
    // save_session writes. Without carrying it over from disk, any UI save would wipe it.
    if let Ok(data) = std::fs::read_to_string(&path) {
        if let Ok(old) = serde_json::from_str::<Vec<Tab>>(&data) {
            for tab in tabs.iter_mut().filter(|t| t.last_url.is_none()) {
                tab.last_url = old.iter().find(|o| o.id == tab.id).and_then(|o| o.last_url.clone());
            }
        }
    }
    let data = serde_json::to_string_pretty(&tabs).unwrap();
    std::fs::write(path, data).ok();
}

#[tauri::command]
fn read_settings(app: tauri::AppHandle) -> Settings {
    let path = settings_path(&app);
    if let Ok(data) = std::fs::read_to_string(&path) {
        if let Ok(settings) = serde_json::from_str(&data) {
            return settings;
        }
    }
    let settings = Settings::default();
    save_settings(app, settings.clone());
    settings
}

#[tauri::command]
fn save_settings(app: tauri::AppHandle, settings: Settings) {
    let path = settings_path(&app);
    let data = serde_json::to_string_pretty(&settings).unwrap();
    std::fs::write(path, data).ok();
    *app.state::<AppState>().settings.lock().unwrap() = settings;
}

/// The settings panel is a separate undecorated owner window on top of the main one, not
/// part of the HTML: on Windows a tab's child webview is its own HWND and always sits above
/// any HTML of the main window, so otherwise the panel would have to move or shrink the tab.
#[tauri::command(async)]
fn open_panel(app: tauri::AppHandle, webview: tauri::Webview, kind: String) -> Result<(), String> {
    // Clicking the same button again closes the panel; a different one switches the view.
    if let Some(panel) = app.get_webview_window(PANEL_LABEL) {
        let same = panel.url().is_ok_and(|u| u.query() == Some(&format!("panel={kind}")));
        panel.close().ok();
        if same {
            return Ok(());
        }
    }
    let parent = webview.window();

    #[allow(unused_mut)]
    let mut builder = tauri::WebviewWindowBuilder::new(
        &app,
        PANEL_LABEL,
        WebviewUrl::App(format!("index.html?panel={kind}").into()),
    )
    .decorations(false)
    .skip_taskbar(true)
    .resizable(false)
    .inner_size(PANEL_WIDTH, 100.0);

    #[cfg(windows)]
    if let Ok(hwnd) = parent.hwnd() {
        builder = builder.owner_raw(hwnd);
    }

    builder.build().map_err(|e| e.to_string())?;
    sync_panel(&app, &parent);
    Ok(())
}

/// Sidebar context menu. Also a separate owner window, and for the same reason as the
/// settings panel: HTML of the main window would end up under the tab webviews.
#[tauri::command(async)]
fn open_menu(app: tauri::AppHandle, webview: tauri::Webview, id: String, x: f64, y: f64) -> Result<(), String> {
    if let Some(menu) = app.get_webview_window(MENU_LABEL) {
        menu.close().ok();
    }
    let parent = webview.window();
    let scale = parent.scale_factor().unwrap_or(1.0);

    #[allow(unused_mut)]
    let mut builder = tauri::WebviewWindowBuilder::new(
        &app,
        MENU_LABEL,
        WebviewUrl::App(format!("index.html?panel=menu&tab={id}").into()),
    )
    .decorations(false)
    .skip_taskbar(true)
    .resizable(false)
    .always_on_top(true)
    .inner_size(230.0, 150.0)
    .position(x / scale, y / scale);

    #[cfg(windows)]
    if let Ok(hwnd) = parent.hwnd() {
        builder = builder.owner_raw(hwnd);
    }

    builder.build().map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command(async)]
fn close_menu(app: tauri::AppHandle) {
    if let Some(menu) = app.get_webview_window(MENU_LABEL) {
        menu.close().ok();
    }
}

#[tauri::command(async)]
fn close_panel(app: tauri::AppHandle) {
    if let Some(panel) = app.get_webview_window(PANEL_LABEL) {
        panel.close().ok();
    }
}

/// The panel window is not physically tied to the main one, so we move and resize it ourselves.
fn sync_panel(app: &tauri::AppHandle, parent: &tauri::Window) {
    let Some(panel) = app.get_webview_window(PANEL_LABEL) else { return };
    let Some(scale) = parent.scale_factor().ok() else { return };
    let Ok(position) = parent.inner_position() else { return };
    let Ok(size) = parent.inner_size() else { return };
    let position = position.to_logical::<f64>(scale);
    let size = size.to_logical::<f64>(scale);
    panel
        .set_position(tauri::LogicalPosition::new(position.x + SIDEBAR_WIDTH as f64, position.y))
        .ok();
    panel.set_size(tauri::LogicalSize::new(PANEL_WIDTH, size.height)).ok();
}

/// Theme of the sites inside the tabs: WebView2 passes it to the page as prefers-color-scheme.
#[tauri::command(async)]
fn set_webview_theme(webview: tauri::Webview, theme: String) {
    let theme = match theme.as_str() {
        "light" => tauri::Theme::Light,
        _ => tauri::Theme::Dark,
    };
    webview.window().set_theme(Some(theme)).ok();
}

/// Writes the active tab's URL into tabs.json and its id into settings.json.
/// Called when leaving a tab and when the window closes.
fn save_session(app: &tauri::AppHandle, window: &tauri::Window) {
    let state = app.state::<AppState>();
    save_window_state(app, window);
    let Some(id) = state.active_tab.lock().unwrap().clone() else { return };
    let wv = state.webviews.lock().unwrap().get(&id).cloned();
    let Some(url) = wv.and_then(|wv| wv.url().ok()).map(|u| u.to_string()) else { return };

    let mut tabs = read_tabs(app.clone());
    if let Some(tab) = tabs.iter_mut().find(|t| t.id == id) {
        tab.last_url = Some(url);
    }
    save_tabs(app.clone(), tabs);

    let settings = state.settings.lock().unwrap().clone();
    save_settings(app.clone(), Settings { last_active: Some(id), ..settings });
}

/// Remembers where and how big the window was, so the next run opens the same way.
fn save_window_state(app: &tauri::AppHandle, window: &tauri::Window) {
    let maximized = window.is_maximized().unwrap_or(false);
    let settings = {
        let state = app.state::<AppState>();
        let mut settings = state.settings.lock().unwrap().clone();
        settings.maximized = maximized;
        // A maximized window would save the maximized size, so the restored size is kept as is.
        if !maximized {
            if let (Ok(scale), Ok(position), Ok(size)) =
                (window.scale_factor(), window.outer_position(), window.inner_size())
            {
                let position = position.to_logical::<f64>(scale);
                let size = size.to_logical::<f64>(scale);
                settings.window = Some([position.x, position.y, size.width, size.height]);
            }
        }
        settings
    };
    save_settings(app.clone(), settings);
}

/// The URL the active tab is actually on right now (after navigating inside the service),
/// so the same page can be opened in a regular browser.
#[tauri::command(async)]
fn active_url(app: tauri::AppHandle) -> Option<String> {
    let state = app.state::<AppState>();
    let active = state.active_tab.lock().unwrap().clone();
    let wv = active.and_then(|id| state.webviews.lock().unwrap().get(&id).cloned());
    wv.and_then(|wv| wv.url().ok()).map(|u| u.to_string())
}

/// One pane of the split view: which tab to show and where, in logical pixels.
#[derive(Debug, Deserialize)]
struct Pane {
    id: String,
    url: String,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

// async: otherwise the command runs on the main thread and add_child / native calls stall
// the event loop (WebView2 needs a free message pump).
//
// The whole layout is computed in the frontend: it is the side that knows the sidebar width,
// the pane headers and the dividers. Rust only applies the rectangles, creates missing
// webviews and parks the ones that are not on screen.
#[tauri::command(async)]
fn sync_panes(app: tauri::AppHandle, webview: tauri::Webview, panes: Vec<Pane>) -> Result<(), String> {
    // The argument is Webview, not WebviewWindow: after the first add_child the window holds
    // several webviews and Tauri refuses to resolve a WebviewWindow
    // ("current webview is not a WebviewWindow"), which broke every command at once.
    let window = webview.window();
    save_session(&app, &window);
    let state = app.state::<AppState>();
    let scale = window.scale_factor().unwrap_or(1.0);

    let mut shown: Vec<String> = Vec::with_capacity(panes.len());
    for pane in &panes {
        let position = PhysicalPosition::new((pane.x * scale) as i32, (pane.y * scale) as i32);
        let size = PhysicalSize::new((pane.width * scale) as u32, (pane.height * scale) as u32);

        let existing = state.webviews.lock().unwrap().get(&pane.id).cloned();
        if let Some(wv) = existing {
            wv.set_position(position).ok();
            wv.set_size(size).ok();
        } else {
            // Do not hold the webviews lock during add_child: creating a child webview can
            // synchronously trigger a window resize event on the same thread, and a resize
            // handler taking the same lock would deadlock the thread against itself.
            let url = tauri::Url::parse(&pane.url).map_err(|e| e.to_string())?;
            let wv = window
                .add_child(
                    tauri::webview::WebviewBuilder::new(&pane.id, WebviewUrl::External(url)),
                    position,
                    size,
                )
                .map_err(|e| e.to_string())?;
            state.webviews.lock().unwrap().insert(pane.id.clone(), wv);
        }
        shown.push(pane.id.clone());
    }

    // Everything not on screen is parked off-screen and starts its unload countdown. Parking
    // happens after the visible panes are placed, otherwise an empty window flashes underneath.
    let parked: Vec<(String, Webview)> = state
        .webviews
        .lock()
        .unwrap()
        .iter()
        .filter(|(id, _)| !shown.contains(id))
        .map(|(id, wv)| (id.clone(), wv.clone()))
        .collect();
    {
        let mut hidden_since = state.hidden_since.lock().unwrap();
        for (id, wv) in parked {
            wv.set_position(OFFSCREEN).ok();
            hidden_since.entry(id).or_insert_with(Instant::now);
        }
        for id in &shown {
            hidden_since.remove(id);
        }
    }

    *state.active_tab.lock().unwrap() = shown.first().cloned();
    if let Some(first) = shown.first() {
        let wv = state.webviews.lock().unwrap().get(first).cloned();
        if let Some(wv) = wv {
            wv.set_focus().ok();
        }
    }
    Ok(())
}

/// Destroys a tab's webview right away: used by the pane close button and by tab removal,
/// where waiting for the unload timeout makes no sense.
#[tauri::command(async)]
fn close_tab(app: tauri::AppHandle, id: String) {
    let state = app.state::<AppState>();
    if let Some(wv) = state.webviews.lock().unwrap().remove(&id) {
        wv.close().ok();
    }
    state.hidden_since.lock().unwrap().remove(&id);
    let mut active = state.active_tab.lock().unwrap();
    if active.as_deref() == Some(id.as_str()) {
        *active = None;
    }
}

/// Periodically unloads (destroys) webviews of tabs inactive longer than the timeout.
/// NOTE: it does not track whether a response is still streaming inside a tab — Tauri cannot
/// see a site's network activity from outside the webview without injecting per-service JS
/// hooks. Not implemented in v1; the only exception is the manual never_unload override.
fn spawn_unload_watcher(app: tauri::AppHandle) {
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(UNLOAD_CHECK_INTERVAL).await;

            let state = app.state::<AppState>();
            let timeout_secs = state.settings.lock().unwrap().unload_timeout_secs;
            let Some(timeout_secs) = timeout_secs else { continue };
            let timeout = Duration::from_secs(timeout_secs);

            let tabs = read_tabs(app.clone());
            let never_unload: std::collections::HashSet<String> = tabs
                .iter()
                .filter(|t| t.never_unload)
                .map(|t| t.id.clone())
                .collect();

            let to_unload: Vec<String> = {
                let hidden_since = state.hidden_since.lock().unwrap();
                hidden_since
                    .iter()
                    .filter(|(id, since)| !never_unload.contains(*id) && since.elapsed() >= timeout)
                    .map(|(id, _)| id.clone())
                    .collect()
            };

            if to_unload.is_empty() {
                continue;
            }

            let mut webviews = state.webviews.lock().unwrap();
            let mut hidden_since = state.hidden_since.lock().unwrap();
            for id in to_unload {
                if let Some(wv) = webviews.remove(&id) {
                    wv.close().ok();
                }
                hidden_since.remove(&id);
            }
        }
    });
}

fn restore_window_state(app: &tauri::AppHandle) {
    let settings = app.state::<AppState>().settings.lock().unwrap().clone();
    let Some(window) = app.get_webview_window("main") else { return };
    if let Some([x, y, width, height]) = settings.window {
        window.set_size(tauri::LogicalSize::new(width, height)).ok();
        window.set_position(tauri::LogicalPosition::new(x, y)).ok();
    }
    if settings.maximized {
        window.maximize().ok();
    }
}

/// Wipes cookies and storage of every tab: a quick way to sign out of all accounts.
/// The data is shared by the whole app profile, so one call covers every service.
#[tauri::command(async)]
fn clear_browsing_data(app: tauri::AppHandle, webview: tauri::Webview) -> Result<(), String> {
    let state = app.state::<AppState>();
    for (_, wv) in state.webviews.lock().unwrap().drain() {
        wv.close().ok();
    }
    state.hidden_since.lock().unwrap().clear();
    *state.active_tab.lock().unwrap() = None;
    webview.clear_all_browsing_data().map_err(|e| e.to_string())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Child webviews on Linux are only moved through X11 (see wry: set_bounds under
    // cfg(feature = "x11")). Wayland forbids window positioning by the protocol, so the tab
    // and the panel drift apart — force XWayland.
    #[cfg(target_os = "linux")]
    std::env::set_var("GDK_BACKEND", "x11");

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(AppState {
            webviews: Mutex::new(HashMap::new()),
            active_tab: Mutex::new(None),
            hidden_since: Mutex::new(HashMap::new()),
            settings: Mutex::new(Settings::default()),
        })
        .invoke_handler(tauri::generate_handler![
            read_tabs,
            save_tabs,
            read_settings,
            save_settings,
            sync_panes,
            close_tab,
            active_url,
            open_panel,
            close_panel,
            open_menu,
            close_menu,
            set_webview_theme,
            clear_browsing_data
        ])
        .setup(|app| {
            let handle = app.handle().clone();
            *handle.state::<AppState>().settings.lock().unwrap() = read_settings(handle.clone());
            restore_window_state(&handle);
            spawn_unload_watcher(handle);
            Ok(())
        })
        .on_window_event(|window, event| {
            if window.label() != "main" {
                return;
            }
            let app = window.app_handle().clone();
            match event {
                tauri::WindowEvent::CloseRequested { .. } => save_session(&app, window),
                // Pane geometry is recomputed by the frontend on its own resize event.
                tauri::WindowEvent::Resized(_) => sync_panel(&app, window),
                tauri::WindowEvent::Moved(_) => sync_panel(&app, window),
                _ => {}
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
