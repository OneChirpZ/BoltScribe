use crate::{
    config, windows,
    workflow_status::{WorkflowMode, WorkflowStatus},
};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use tauri::image::Image;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Emitter, Manager, Wry};
#[cfg(target_os = "windows")]
use {
    std::sync::atomic::AtomicU64,
    std::time::Duration,
    tauri::tray::{MouseButton, MouseButtonState, TrayIconEvent},
};

const MENU_SHOW_MAIN: &str = "show_main_window";
const MENU_TOGGLE_VOICE_INPUT: &str = "toggle_voice_input";
const MENU_TOGGLE_LLM_CORRECTION: &str = "toggle_llm_correction";
const MENU_SHOW_LEARNING: &str = "show_learning";
const MENU_QUIT: &str = "quit_app";
const TRAY_ID: &str = "main";
static LEARNING_OPEN_REQUESTED: AtomicBool = AtomicBool::new(false);

type ToggleVoiceInput = Arc<dyn Fn(AppHandle<Wry>) + Send + Sync>;

#[derive(Default)]
pub(crate) struct TrayState {
    voice_input_mode: Mutex<WorkflowMode>,
    learning: Mutex<LearningMenuState>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct LearningMenuState {
    observing: usize,
    queued: usize,
    analyzing: usize,
}

impl LearningMenuState {
    fn active(self) -> bool {
        self.observing + self.queued + self.analyzing > 0
    }

    fn label(self) -> String {
        let mut stages = Vec::new();
        if self.observing > 0 {
            stages.push(format!("观察中 {}", self.observing));
        }
        if self.analyzing > 0 {
            stages.push(format!("分析中 {}", self.analyzing));
        }
        if self.queued > 0 {
            stages.push(format!("等待分析 {}", self.queued));
        }
        if stages.is_empty() {
            "自学习".to_string()
        } else {
            format!("自学习 · {}", stages.join(" · "))
        }
    }
}

pub(crate) fn setup(
    app: &AppHandle<Wry>,
    show_menu_bar_icon: bool,
    toggle_voice_input: impl Fn(AppHandle<Wry>) + Send + Sync + 'static,
) -> tauri::Result<()> {
    let toggle_voice_input: ToggleVoiceInput = Arc::new(toggle_voice_input);
    let menu = build_menu(
        app,
        current_llm_correction_enabled(),
        current_voice_input_menu_state(app),
    )?;

    let menu_toggle = toggle_voice_input.clone();
    let mut builder = TrayIconBuilder::with_id(TRAY_ID)
        .menu(&menu)
        .tooltip("BoltScribe")
        .show_menu_on_left_click(show_menu_on_left_click())
        .on_menu_event(move |app, event| match event.id().as_ref() {
            MENU_SHOW_MAIN => {
                if let Err(err) = windows::show_main_window(app) {
                    eprintln!("failed to show main window from tray: {err:?}");
                }
            }
            MENU_TOGGLE_VOICE_INPUT => {
                menu_toggle(app.clone());
            }
            MENU_TOGGLE_LLM_CORRECTION => {
                if let Err(err) = toggle_llm_correction(app) {
                    eprintln!("failed to toggle LLM correction from tray: {err}");
                }
            }
            MENU_SHOW_LEARNING => {
                LEARNING_OPEN_REQUESTED.store(true, Ordering::Release);
                if let Err(err) = windows::show_main_window(app) {
                    eprintln!("failed to show learning window from tray: {err:?}");
                }
                let _ = app.emit("learning://open", ());
            }
            MENU_QUIT => app.exit(0),
            _ => {}
        });

    #[cfg(target_os = "windows")]
    {
        let click_controller = Arc::new(WindowsTrayClickController::default());
        let click_toggle = toggle_voice_input.clone();
        builder = builder.on_tray_icon_event(move |tray, event| {
            handle_windows_tray_icon_event(
                click_controller.clone(),
                tray.app_handle().clone(),
                click_toggle.clone(),
                event,
            );
        });
    }

    builder = builder
        .icon(tray_template_icon(false))
        .icon_as_template(true);

    builder.build(app)?;
    sync_menu_bar_visibility(app, show_menu_bar_icon)
}

pub(crate) fn take_learning_open_request() -> bool {
    LEARNING_OPEN_REQUESTED.swap(false, Ordering::AcqRel)
}

pub(crate) fn sync_menu_bar_visibility(app: &AppHandle<Wry>, visible: bool) -> tauri::Result<()> {
    #[cfg(target_os = "macos")]
    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        tray.set_visible(visible)?;
    }
    #[cfg(not(target_os = "macos"))]
    let _ = (app, visible);
    Ok(())
}

#[cfg(target_os = "windows")]
fn show_menu_on_left_click() -> bool {
    false
}

#[cfg(not(target_os = "windows"))]
fn show_menu_on_left_click() -> bool {
    true
}

pub(crate) fn sync_llm_correction_label(
    app: &tauri::AppHandle<Wry>,
    enabled: bool,
) -> tauri::Result<()> {
    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        tray.set_menu(Some(build_menu(
            app,
            enabled,
            current_voice_input_menu_state(app),
        )?))?;
    }
    Ok(())
}

pub(crate) fn sync_voice_input_label(
    app: &AppHandle<Wry>,
    status: &WorkflowStatus,
) -> tauri::Result<()> {
    if let Some(state) = app.try_state::<TrayState>() {
        if let Ok(mut mode) = state.voice_input_mode.lock() {
            *mode = status.mode.clone();
        }
    }
    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        tray.set_menu(Some(build_menu(
            app,
            current_llm_correction_enabled(),
            voice_input_menu_state(&status.mode),
        )?))?;
    }
    Ok(())
}

pub(crate) fn sync_learning_status(
    app: &AppHandle<Wry>,
    observing: usize,
    queued: usize,
    analyzing: usize,
) -> tauri::Result<()> {
    let learning = LearningMenuState {
        observing,
        queued,
        analyzing,
    };
    if let Some(state) = app.try_state::<TrayState>() {
        if let Ok(mut current) = state.learning.lock() {
            *current = learning;
        }
    }
    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        tray.set_icon(Some(tray_template_icon(learning.active())))?;
        tray.set_tooltip(Some(format!("BoltScribe · {}", learning.label())))?;
        tray.set_menu(Some(build_menu(
            app,
            current_llm_correction_enabled(),
            current_voice_input_menu_state(app),
        )?))?;
    }
    Ok(())
}

fn current_learning_menu_state(app: &AppHandle<Wry>) -> LearningMenuState {
    app.try_state::<TrayState>()
        .and_then(|state| state.learning.lock().ok().map(|learning| *learning))
        .unwrap_or_default()
}

fn build_menu(
    app: &tauri::AppHandle<Wry>,
    llm_correction_enabled: bool,
    voice_input: VoiceInputMenuState,
) -> tauri::Result<Menu<Wry>> {
    let show_main = MenuItem::with_id(app, MENU_SHOW_MAIN, "设置", true, None::<&str>)?;
    let toggle_voice_input = MenuItem::with_id(
        app,
        MENU_TOGGLE_VOICE_INPUT,
        voice_input.label,
        voice_input.enabled,
        None::<&str>,
    )?;
    let toggle_llm = MenuItem::with_id(
        app,
        MENU_TOGGLE_LLM_CORRECTION,
        llm_correction_label(llm_correction_enabled),
        true,
        None::<&str>,
    )?;
    let learning = MenuItem::with_id(
        app,
        MENU_SHOW_LEARNING,
        current_learning_menu_state(app).label(),
        true,
        None::<&str>,
    )?;
    let quit = MenuItem::with_id(app, MENU_QUIT, "退出", true, None::<&str>)?;
    let separator = PredefinedMenuItem::separator(app)?;
    Menu::with_items(
        app,
        &[
            &show_main,
            &toggle_voice_input,
            &toggle_llm,
            &learning,
            &separator,
            &quit,
        ],
    )
}

fn toggle_llm_correction(app: &tauri::AppHandle<Wry>) -> Result<(), String> {
    let saved = config::ConfigStore::update(|config| {
        config.correction.enabled = !config.correction.enabled;
    })
    .map_err(|err| err.to_string())?;
    sync_llm_correction_label(app, saved.correction.enabled).map_err(|err| err.to_string())?;
    let _ = app.emit("config://updated", &saved);
    Ok(())
}

fn current_llm_correction_enabled() -> bool {
    config::ConfigStore::load()
        .map(|config| config.correction.enabled)
        .unwrap_or_default()
}

fn current_voice_input_menu_state(app: &AppHandle<Wry>) -> VoiceInputMenuState {
    app.try_state::<TrayState>()
        .and_then(|state| state.voice_input_mode.lock().ok().map(|mode| mode.clone()))
        .map(|mode| voice_input_menu_state(&mode))
        .unwrap_or_else(|| voice_input_menu_state(&WorkflowMode::Idle))
}

fn llm_correction_label(enabled: bool) -> &'static str {
    if enabled {
        "关闭 LLM 纠错"
    } else {
        "启用 LLM 纠错"
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct VoiceInputMenuState {
    label: &'static str,
    enabled: bool,
}

fn voice_input_menu_state(mode: &WorkflowMode) -> VoiceInputMenuState {
    match mode {
        WorkflowMode::Starting => VoiceInputMenuState {
            label: "正在启动语音输入",
            enabled: false,
        },
        WorkflowMode::Recording => VoiceInputMenuState {
            label: "停止语音输入",
            enabled: true,
        },
        WorkflowMode::Processing => VoiceInputMenuState {
            label: "开始语音输入",
            enabled: false,
        },
        WorkflowMode::Idle | WorkflowMode::Error => VoiceInputMenuState {
            label: "开始语音输入",
            enabled: true,
        },
    }
}

#[cfg(target_os = "windows")]
#[derive(Default)]
struct WindowsTrayClickController {
    pending_left_click_id: AtomicU64,
    ignore_next_left_button_up: AtomicBool,
}

#[cfg(target_os = "windows")]
impl WindowsTrayClickController {
    fn queue_left_click(&self) -> Option<u64> {
        if self
            .ignore_next_left_button_up
            .swap(false, Ordering::AcqRel)
        {
            return None;
        }

        Some(self.pending_left_click_id.fetch_add(1, Ordering::AcqRel) + 1)
    }

    fn cancel_for_double_click(&self) {
        self.pending_left_click_id.fetch_add(1, Ordering::AcqRel);
        self.ignore_next_left_button_up
            .store(true, Ordering::Release);
    }

    fn is_left_click_current(&self, click_id: u64) -> bool {
        self.pending_left_click_id.load(Ordering::Acquire) == click_id
    }
}

#[cfg(target_os = "windows")]
fn handle_windows_tray_icon_event(
    click_controller: Arc<WindowsTrayClickController>,
    app: tauri::AppHandle<Wry>,
    toggle_voice_input: ToggleVoiceInput,
    event: TrayIconEvent,
) {
    match event {
        TrayIconEvent::Click {
            button: MouseButton::Left,
            button_state: MouseButtonState::Up,
            ..
        } => queue_windows_left_click_toggle(click_controller, app, toggle_voice_input),
        TrayIconEvent::DoubleClick {
            button: MouseButton::Left,
            ..
        } => {
            click_controller.cancel_for_double_click();
            if let Err(err) = windows::show_main_window(&app) {
                eprintln!("failed to show main window from tray double click: {err:?}");
            }
        }
        _ => {}
    }
}

#[cfg(target_os = "windows")]
fn queue_windows_left_click_toggle(
    click_controller: Arc<WindowsTrayClickController>,
    app: tauri::AppHandle<Wry>,
    toggle_voice_input: ToggleVoiceInput,
) {
    let Some(click_id) = queue_windows_left_click_if_enabled(
        &click_controller,
        current_tray_left_click_recording_enabled(),
    ) else {
        return;
    };
    std::thread::spawn(move || {
        std::thread::sleep(windows_single_click_delay());
        if !should_run_windows_left_click_toggle(
            &click_controller,
            click_id,
            current_tray_left_click_recording_enabled(),
        ) {
            return;
        }
        toggle_voice_input(app);
    });
}

#[cfg(target_os = "windows")]
fn queue_windows_left_click_if_enabled(
    click_controller: &WindowsTrayClickController,
    enabled: bool,
) -> Option<u64> {
    if !enabled {
        return None;
    }
    click_controller.queue_left_click()
}

#[cfg(target_os = "windows")]
fn should_run_windows_left_click_toggle(
    click_controller: &WindowsTrayClickController,
    click_id: u64,
    enabled: bool,
) -> bool {
    enabled && click_controller.is_left_click_current(click_id)
}

#[cfg(target_os = "windows")]
fn current_tray_left_click_recording_enabled() -> bool {
    config::ConfigStore::load()
        .map(|config| config.system.tray_left_click_recording_enabled)
        .unwrap_or(true)
}

#[cfg(target_os = "windows")]
fn windows_single_click_delay() -> Duration {
    let double_click_ms =
        unsafe { ::windows::Win32::UI::Input::KeyboardAndMouse::GetDoubleClickTime() };
    Duration::from_millis(u64::from(double_click_ms) + 50)
}

fn tray_template_icon(learning_active: bool) -> Image<'static> {
    const SIZE: usize = 64;
    const SUPERSAMPLE: usize = 4;
    const BOLT: [(f64, f64); 6] = [
        (38.0, 4.0),
        (17.0, 34.0),
        (31.5, 34.0),
        (23.0, 60.0),
        (47.0, 26.0),
        (33.0, 26.0),
    ];

    let mut rgba = vec![0; SIZE * SIZE * 4];
    for y in 0..SIZE {
        for x in 0..SIZE {
            let mut hits = 0usize;
            for sy in 0..SUPERSAMPLE {
                for sx in 0..SUPERSAMPLE {
                    let px = x as f64 + (sx as f64 + 0.5) / SUPERSAMPLE as f64;
                    let py = y as f64 + (sy as f64 + 0.5) / SUPERSAMPLE as f64;
                    let marker = learning_active
                        && (px - 53.0).powi(2) + (py - 52.0).powi(2) <= 6.0_f64.powi(2);
                    if point_in_polygon(px, py, &BOLT) || marker {
                        hits += 1;
                    }
                }
            }

            if hits == 0 {
                continue;
            }
            let alpha = ((hits * 255) / (SUPERSAMPLE * SUPERSAMPLE)) as u8;
            let index = (y * SIZE + x) * 4;
            rgba[index..index + 4].copy_from_slice(&[255, 255, 255, alpha]);
        }
    }

    Image::new_owned(rgba, SIZE as u32, SIZE as u32)
}

fn point_in_polygon(x: f64, y: f64, points: &[(f64, f64)]) -> bool {
    let mut inside = false;
    let mut previous = points.len() - 1;
    for current in 0..points.len() {
        let (xi, yi) = points[current];
        let (xj, yj) = points[previous];
        if (yi > y) != (yj > y) && x < (xj - xi) * (y - yi) / (yj - yi) + xi {
            inside = !inside;
        }
        previous = current;
    }
    inside
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn voice_input_menu_item_starts_for_idle_and_error() {
        let expected = VoiceInputMenuState {
            label: "开始语音输入",
            enabled: true,
        };

        assert_eq!(voice_input_menu_state(&WorkflowMode::Idle), expected);
        assert_eq!(voice_input_menu_state(&WorkflowMode::Error), expected);
    }

    #[test]
    fn voice_input_menu_item_stops_while_recording() {
        assert_eq!(
            voice_input_menu_state(&WorkflowMode::Recording),
            VoiceInputMenuState {
                label: "停止语音输入",
                enabled: true,
            }
        );
    }

    #[test]
    fn voice_input_menu_item_is_disabled_while_processing() {
        assert_eq!(
            voice_input_menu_state(&WorkflowMode::Processing),
            VoiceInputMenuState {
                label: "开始语音输入",
                enabled: false,
            }
        );
    }

    #[test]
    fn voice_input_menu_item_is_disabled_while_starting() {
        assert_eq!(
            voice_input_menu_state(&WorkflowMode::Starting),
            VoiceInputMenuState {
                label: "正在启动语音输入",
                enabled: false,
            }
        );
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn windows_left_click_queues_toggle() {
        let controller = WindowsTrayClickController::default();

        let click_id = queue_windows_left_click_if_enabled(&controller, true);

        assert_eq!(click_id, Some(1));
        assert!(controller.is_left_click_current(1));
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn windows_left_click_disabled_does_not_queue_toggle() {
        let controller = WindowsTrayClickController::default();

        let click_id = queue_windows_left_click_if_enabled(&controller, false);

        assert_eq!(click_id, None);
        assert!(!controller.is_left_click_current(1));
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn windows_left_click_disabled_before_delay_skips_toggle() {
        let controller = WindowsTrayClickController::default();
        let click_id = queue_windows_left_click_if_enabled(&controller, true).unwrap();

        assert!(!should_run_windows_left_click_toggle(
            &controller,
            click_id,
            false
        ));
        assert!(should_run_windows_left_click_toggle(
            &controller,
            click_id,
            true
        ));
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn windows_double_click_cancels_pending_toggle() {
        let controller = WindowsTrayClickController::default();
        let click_id = controller.queue_left_click().unwrap();

        controller.cancel_for_double_click();

        assert!(!controller.is_left_click_current(click_id));
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn windows_double_click_ignores_following_left_button_up() {
        let controller = WindowsTrayClickController::default();

        controller.cancel_for_double_click();

        assert_eq!(controller.queue_left_click(), None);
        assert_eq!(controller.queue_left_click(), Some(2));
    }
}
