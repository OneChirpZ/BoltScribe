use crate::{config, windows};
use tauri::image::Image;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{Emitter, Wry};

const MENU_SHOW_MAIN: &str = "show_main_window";
const MENU_TOGGLE_LLM_CORRECTION: &str = "toggle_llm_correction";
const MENU_QUIT: &str = "quit_app";
const TRAY_ID: &str = "main";

pub(crate) fn setup(app: &tauri::AppHandle<Wry>) -> tauri::Result<()> {
    let menu = build_menu(app, current_llm_correction_enabled())?;

    let mut builder = TrayIconBuilder::with_id(TRAY_ID)
        .menu(&menu)
        .tooltip("BoltScribe")
        .show_menu_on_left_click(true)
        .on_menu_event(|app, event| match event.id().as_ref() {
            MENU_SHOW_MAIN => {
                if let Err(err) = windows::show_main_window(app) {
                    eprintln!("failed to show main window from tray: {err:?}");
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

    builder = builder.icon(tray_template_icon()).icon_as_template(true);

    builder.build(app)?;
    Ok(())
}

pub(crate) fn sync_llm_correction_label(
    app: &tauri::AppHandle<Wry>,
    enabled: bool,
) -> tauri::Result<()> {
    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        tray.set_menu(Some(build_menu(app, enabled)?))?;
    }
    Ok(())
}

fn build_menu(
    app: &tauri::AppHandle<Wry>,
    llm_correction_enabled: bool,
) -> tauri::Result<Menu<Wry>> {
    let show_main = MenuItem::with_id(app, MENU_SHOW_MAIN, "设置", true, None::<&str>)?;
    let toggle_llm = MenuItem::with_id(
        app,
        MENU_TOGGLE_LLM_CORRECTION,
        llm_correction_label(llm_correction_enabled),
        true,
        None::<&str>,
    )?;
    let quit = MenuItem::with_id(app, MENU_QUIT, "退出", true, None::<&str>)?;
    let separator = PredefinedMenuItem::separator(app)?;
    Menu::with_items(app, &[&show_main, &toggle_llm, &separator, &quit])
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

fn llm_correction_label(enabled: bool) -> &'static str {
    if enabled {
        "关闭 LLM 纠错"
    } else {
        "启用 LLM 纠错"
    }
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
