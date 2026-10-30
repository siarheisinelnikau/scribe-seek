#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() -> anyhow::Result<()> {
    if std::env::args_os().nth(1).as_deref() == Some(std::ffi::OsStr::new("--apply-update")) {
        if let Some(path) = std::env::args_os().nth(2).map(std::path::PathBuf::from) {
            let locale = scribe_seek::updater::helper_locale(&path);
            if let Err(error) = scribe_seek::updater::run_helper(&path) {
                rfd::MessageDialog::new()
                    .set_title("Scribe Seek")
                    .set_description(format!(
                        "{}\n\n{}",
                        locale.tr("Update failed. The previous version was kept or restored."),
                        locale.message(&format!("{error:#}"))
                    ))
                    .set_level(rfd::MessageLevel::Error)
                    .show();
            }
        }
        return Ok(());
    }
    let storage = match scribe_seek::storage::Storage::discover() {
        Ok(storage) => storage,
        Err(error) => {
            rfd::MessageDialog::new()
                .set_title("Scribe Seek")
                .set_description(format!("Could not open the data directory:\n{error:#}"))
                .set_level(rfd::MessageLevel::Error)
                .show();
            return Ok(());
        }
    };
    let locale = scribe_seek::settings::Settings::load(&storage)
        .unwrap_or_default()
        .ui_language;
    #[cfg(target_arch = "x86_64")]
    if !std::is_x86_feature_detected!("avx2")
        || !std::is_x86_feature_detected!("fma")
        || !std::is_x86_feature_detected!("f16c")
    {
        rfd::MessageDialog::new().set_title("Scribe Seek")
            .set_description(locale.tr("This build requires a CPU with AVX2, FMA and F16C (Intel Haswell / AMD Ryzen or newer)."))
            .set_level(rfd::MessageLevel::Error).show();
        return Ok(());
    }
    let source = std::env::args_os().nth(1).map(std::path::PathBuf::from);
    #[cfg(windows)]
    let result = scribe_seek::desktop::run(storage, source);
    #[cfg(not(windows))]
    let result = run_eframe(storage, source);
    if let Err(error) = result {
        rfd::MessageDialog::new()
            .set_title("Scribe Seek")
            .set_description(format!("Could not start Scribe Seek:\n{error:#}"))
            .set_level(rfd::MessageLevel::Error)
            .show();
    }
    Ok(())
}

#[cfg(not(windows))]
fn run_eframe(
    storage: scribe_seek::storage::Storage,
    source: Option<std::path::PathBuf>,
) -> anyhow::Result<()> {
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_inner_size([1180.0, 800.0])
            .with_min_inner_size([960.0, 700.0])
            .with_drag_and_drop(true),
        ..Default::default()
    };
    eframe::run_native(
        "Scribe Seek",
        options,
        Box::new(move |cc| {
            Ok(Box::new(scribe_seek::app::App::new(
                &cc.egui_ctx,
                storage,
                source,
            )))
        }),
    )
    .map_err(|error| anyhow::anyhow!("{error}"))
}
