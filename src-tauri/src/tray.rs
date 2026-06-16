use crate::{config, windows, workflow};
use tauri::image::Image;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{Emitter, Manager, Wry};
#[cfg(target_os = "windows")]
use {
    std::sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc,
    },
    std::time::Duration,
    tauri::tray::{MouseButton, MouseButtonState, TrayIconEvent},
};

const MENU_SHOW_MAIN: &str = "show_main_window";
const MENU_TOGGLE_VOICE_INPUT: &str = "toggle_voice_input";
const MENU_TOGGLE_LLM_CORRECTION: &str = "toggle_llm_correction";
const MENU_QUIT: &str = "quit_app";
const TRAY_ID: &str = "main";

pub(crate) fn setup(app: &tauri::AppHandle<Wry>) -> tauri::Result<()> {
    let menu = build_menu(
        app,
        current_llm_correction_enabled(),
        current_voice_input_menu_state(app),
    )?;

    let mut builder = TrayIconBuilder::with_id(TRAY_ID)
        .menu(&menu)
        .tooltip("BoltScribe")
        .show_menu_on_left_click(show_menu_on_left_click())
        .on_menu_event(|app, event| match event.id().as_ref() {
            MENU_SHOW_MAIN => {
                if let Err(err) = windows::show_main_window(app) {
                    eprintln!("failed to show main window from tray: {err:?}");
                }
            }
            MENU_TOGGLE_VOICE_INPUT => {
                if let Err(err) = workflow::toggle_recording_from_app(app.clone()) {
                    eprintln!("failed to toggle voice input from tray: {err:?}");
                }
            }
            MENU_TOGGLE_LLM_CORRECTION => {
                if let Err(err) = toggle_llm_correction(app) {
                    eprintln!("failed to toggle LLM correction from tray: {err}");
                }
            }
            MENU_QUIT => app.exit(0),
            _ => {}
        });

    #[cfg(target_os = "windows")]
    {
        let click_controller = Arc::new(WindowsTrayClickController::default());
        builder = builder.on_tray_icon_event(move |tray, event| {
            handle_windows_tray_icon_event(
                click_controller.clone(),
                tray.app_handle().clone(),
                event,
            );
        });
    }

    builder = builder.icon(tray_template_icon()).icon_as_template(true);

    builder.build(app)?;
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
    app: &tauri::AppHandle<Wry>,
    status: &workflow::WorkflowStatus,
) -> tauri::Result<()> {
    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        tray.set_menu(Some(build_menu(
            app,
            current_llm_correction_enabled(),
            voice_input_menu_state(&status.mode),
        )?))?;
    }
    Ok(())
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
    let quit = MenuItem::with_id(app, MENU_QUIT, "退出", true, None::<&str>)?;
    let separator = PredefinedMenuItem::separator(app)?;
    Menu::with_items(
        app,
        &[
            &show_main,
            &toggle_voice_input,
            &toggle_llm,
            &separator,
            &quit,
        ],
    )
}

fn toggle_llm_correction(app: &tauri::AppHandle<Wry>) -> Result<(), String> {
    let mut next = config::ConfigStore::load().map_err(|err| err.to_string())?;
    next.correction.enabled = !next.correction.enabled;
    let saved = config::ConfigStore::save(&next).map_err(|err| err.to_string())?;
    sync_llm_correction_label(app, saved.correction.enabled).map_err(|err| err.to_string())?;
    let _ = app.emit("config://updated", &saved);
    Ok(())
}

fn current_llm_correction_enabled() -> bool {
    config::ConfigStore::load()
        .map(|config| config.correction.enabled)
        .unwrap_or_default()
}

fn current_voice_input_menu_state(app: &tauri::AppHandle<Wry>) -> VoiceInputMenuState {
    app.try_state::<workflow::AppState>()
        .map(|state| voice_input_menu_state(&state.status().mode))
        .unwrap_or_else(|| voice_input_menu_state(&workflow::WorkflowMode::Idle))
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

fn voice_input_menu_state(mode: &workflow::WorkflowMode) -> VoiceInputMenuState {
    match mode {
        workflow::WorkflowMode::Recording => VoiceInputMenuState {
            label: "停止语音输入",
            enabled: true,
        },
        workflow::WorkflowMode::Processing => VoiceInputMenuState {
            label: "开始语音输入",
            enabled: false,
        },
        workflow::WorkflowMode::Idle | workflow::WorkflowMode::Error => VoiceInputMenuState {
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
    event: TrayIconEvent,
) {
    match event {
        TrayIconEvent::Click {
            button: MouseButton::Left,
            button_state: MouseButtonState::Up,
            ..
        } => queue_windows_left_click_toggle(click_controller, app),
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
        if let Err(err) = workflow::toggle_recording_from_app(app) {
            eprintln!("failed to toggle voice input from tray left click: {err:?}");
        }
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

fn tray_template_icon() -> Image<'static> {
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
                    if point_in_polygon(px, py, &BOLT) {
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

        assert_eq!(
            voice_input_menu_state(&workflow::WorkflowMode::Idle),
            expected
        );
        assert_eq!(
            voice_input_menu_state(&workflow::WorkflowMode::Error),
            expected
        );
    }

    #[test]
    fn voice_input_menu_item_stops_while_recording() {
        assert_eq!(
            voice_input_menu_state(&workflow::WorkflowMode::Recording),
            VoiceInputMenuState {
                label: "停止语音输入",
                enabled: true,
            }
        );
    }

    #[test]
    fn voice_input_menu_item_is_disabled_while_processing() {
        assert_eq!(
            voice_input_menu_state(&workflow::WorkflowMode::Processing),
            VoiceInputMenuState {
                label: "开始语音输入",
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
