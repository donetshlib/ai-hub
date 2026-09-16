use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tauri::{Manager, PhysicalPosition, PhysicalSize, Webview, WebviewUrl};

const SIDEBAR_WIDTH: u32 = 56;
/// Неактивные webview паркуются здесь: hide() на Windows не убирает дочерний HWND
/// ни из отрисовки, ни из зоны перехвата кликов.
const OFFSCREEN: PhysicalPosition<i32> = PhysicalPosition::new(-10000, -10000);
const PANEL_LABEL: &str = "__panel";
/// Ширина панели настроек в логических пикселях.
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
    /// Последний URL внутри сервиса, для восстановления сессии.
    #[serde(default)]
    last_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Settings {
    /// None = "никогда не выгружать". В секундах, произвольное значение задаётся в настройках.
    #[serde(default = "default_unload")]
    unload_timeout_secs: Option<u64>,
    #[serde(default = "default_theme")]
    theme: String,
    /// Открывать при старте ту же вкладку и тот же URL, что были при выходе.
    #[serde(default)]
    restore_session: bool,
    #[serde(default)]
    last_active: Option<String>,
    #[serde(default = "default_lang")]
    lang: String,
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
        }
    }
}

struct AppState {
    webviews: Mutex<HashMap<String, Webview>>,
    active_tab: Mutex<Option<String>>,
    /// Момент, когда вкладка стала неактивной (скрыта). Используется для отсчёта таймаута.
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
    // Фронтенд держит свою копию списка и не знает про last_url, который пишет
    // save_session. Без переноса с диска любое сохранение из UI его бы стирало.
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

/// Область контента: окно минус полоса сайдбара слева.
fn content_bounds(window: &tauri::Window) -> (PhysicalPosition<i32>, PhysicalSize<u32>) {
    let size = window.inner_size().unwrap_or(PhysicalSize::new(1000, 700));
    let scale = window.scale_factor().unwrap_or(1.0);
    let sidebar_px = (SIDEBAR_WIDTH as f64 * scale) as i32;
    (
        PhysicalPosition::new(sidebar_px, 0),
        PhysicalSize::new(size.width.saturating_sub(sidebar_px as u32), size.height),
    )
}

/// Панель настроек это отдельное безрамочное окно-владелец поверх главного, а не часть
/// HTML: дочерний webview вкладки на Windows свой HWND и всегда выше любого HTML главного
/// окна, поэтому иначе панель пришлось бы двигать/сжимать саму вкладку.
#[tauri::command(async)]
fn open_panel(app: tauri::AppHandle, webview: tauri::Webview, kind: String) -> Result<(), String> {
    // Повторный клик по той же кнопке закрывает панель, по другой — переключает вид.
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

#[tauri::command(async)]
fn close_panel(app: tauri::AppHandle) {
    if let Some(panel) = app.get_webview_window(PANEL_LABEL) {
        panel.close().ok();
    }
}

/// Окно панели не связано с главным физически — двигаем и растягиваем его сами.
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

/// Тема самих сайтов внутри вкладок: WebView2 отдаёт её странице как prefers-color-scheme.
#[tauri::command(async)]
fn set_webview_theme(webview: tauri::Webview, theme: String) {
    let theme = match theme.as_str() {
        "light" => tauri::Theme::Light,
        _ => tauri::Theme::Dark,
    };
    webview.window().set_theme(Some(theme)).ok();
}

/// Пишет URL активной вкладки в tabs.json, а её id в settings.json.
/// Вызывается при уходе с вкладки и при закрытии окна.
fn save_session(app: &tauri::AppHandle) {
    let state = app.state::<AppState>();
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

/// URL, на котором сейчас реально находится активная вкладка (после навигации внутри
/// сервиса), чтобы открыть то же самое в обычном браузере.
#[tauri::command(async)]
fn active_url(app: tauri::AppHandle) -> Option<String> {
    let state = app.state::<AppState>();
    let active = state.active_tab.lock().unwrap().clone();
    let wv = active.and_then(|id| state.webviews.lock().unwrap().get(&id).cloned());
    wv.and_then(|wv| wv.url().ok()).map(|u| u.to_string())
}

// async: иначе команда исполняется на главном потоке и add_child/нативные вызовы
// подвешивают event loop (WebView2 нужен свободный message pump).
#[tauri::command(async)]
fn switch_tab(app: tauri::AppHandle, webview: tauri::Webview, id: String, url: String) -> Result<(), String> {
    // Аргумент именно Webview, а не WebviewWindow: после первого add_child окно держит
    // несколько webview, и Tauri отказывается резолвить WebviewWindow
    // ("current webview is not a WebviewWindow") — ломались все команды разом.
    let window = webview.window();
    save_session(&app);
    let state = app.state::<AppState>();

    let existing_wv = state.webviews.lock().unwrap().get(&id).cloned();
    if let Some(wv) = existing_wv {
        let (position, size) = content_bounds(&window);
        wv.set_position(position).ok();
        wv.set_size(size).ok();
        wv.set_focus().ok();
    } else {
        // Не держим лок на webviews во время add_child: создание нового дочернего
        // webview может синхронно триггернуть событие ресайза окна на этом же потоке,
        // а resize_active_webview тоже пытается взять этот лок — без освобождения
        // здесь это самоблокировка (поток вешает сам себя, включая всё окно).
        let (position, size) = content_bounds(&window);
        let parsed_url = tauri::Url::parse(&url).map_err(|e| e.to_string())?;
        let wv = window
            .add_child(
                tauri::webview::WebviewBuilder::new(&id, WebviewUrl::External(parsed_url)),
                position,
                size,
            )
            .inspect_err(|e| eprintln!("[switch_tab] add_child FAILED: {e}"))
            .map_err(|e| e.to_string())?;
        state.webviews.lock().unwrap().insert(id.clone(), wv);
    }

    // Прячем предыдущую вкладку только теперь, когда новая уже на экране: при обратном
    // порядке на кадр-два видно пустое окно под ней.
    if let Some(prev) = state.active_tab.lock().unwrap().clone() {
        if prev != id {
            let prev_wv = state.webviews.lock().unwrap().get(&prev).cloned();
            if let Some(wv) = prev_wv {
                wv.set_position(OFFSCREEN).ok();
            }
            state.hidden_since.lock().unwrap().insert(prev, Instant::now());
        }
    }

    state.hidden_since.lock().unwrap().remove(&id);
    *state.active_tab.lock().unwrap() = Some(id);
    Ok(())
}

#[tauri::command(async)]
fn remove_tab(app: tauri::AppHandle, id: String) {
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

fn resize_active_webview(app: &tauri::AppHandle, window: &tauri::Window) {
    let state = app.state::<AppState>();
    let active = state.active_tab.lock().unwrap().clone();
    let wv = active.and_then(|id| state.webviews.lock().unwrap().get(&id).cloned());
    if let Some(wv) = wv {
        let (position, size) = content_bounds(window);
        wv.set_position(position).ok();
        wv.set_size(size).ok();
    }
}

/// Периодически выгружает (уничтожает) webview вкладок, неактивных дольше таймаута.
/// ВАЖНО: не отслеживает "идёт ли генерация ответа" внутри вкладки — Tauri не видит
/// сетевую активность конкретного сайта изнутри webview без встраивания JS-хуков
/// под каждый сервис отдельно. В v1 это не реализовано — исключение только через
/// ручной оверрайд never_unload.
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

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Дочерние webview на Linux двигаются только через X11 (см. wry: set_bounds под
    // cfg(feature = "x11")). На Wayland позиционирование окон запрещено протоколом,
    // поэтому вкладка и панель разъезжаются — форсируем XWayland.
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
            switch_tab,
            remove_tab,
            active_url,
            open_panel,
            close_panel,
            set_webview_theme
        ])
        .setup(|app| {
            let handle = app.handle().clone();
            *handle.state::<AppState>().settings.lock().unwrap() = read_settings(handle.clone());
            spawn_unload_watcher(handle);
            Ok(())
        })
        .on_window_event(|window, event| {
            if window.label() == PANEL_LABEL {
                return;
            }
            let app = window.app_handle().clone();
            match event {
                tauri::WindowEvent::CloseRequested { .. } => save_session(&app),
                tauri::WindowEvent::Resized(_) => {
                    resize_active_webview(&app, window);
                    sync_panel(&app, window);
                }
                tauri::WindowEvent::Moved(_) => sync_panel(&app, window),
                _ => {}
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
