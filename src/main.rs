#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() -> anyhow::Result<()> {
    let mut arguments = std::env::args_os().skip(1);
    let first = arguments.next();
    if first.as_deref() == Some(std::ffi::OsStr::new("--apply-update")) {
        return rust_auto_updator::apply_update(&scribe_seek::updater::config()?)
            .map_err(anyhow::Error::msg);
    }
    let mut source = None;
    let mut data = None;
    let mut cleanup = None;
    let mut argument = first;
    while let Some(value) = argument {
        match value.to_str() {
            Some("--data-dir") => {
                data = Some(std::path::PathBuf::from(
                    arguments
                        .next()
                        .ok_or_else(|| anyhow::anyhow!("Missing data directory"))?,
                ))
            }
            Some("--source") => {
                source = Some(std::path::PathBuf::from(
                    arguments
                        .next()
                        .ok_or_else(|| anyhow::anyhow!("Missing source file"))?,
                ))
            }
            Some("--cleanup-update") => {
                cleanup =
                    Some(std::path::PathBuf::from(arguments.next().ok_or_else(
                        || anyhow::anyhow!("Missing cleanup directory"),
                    )?))
            }
            _ if source.is_none() => source = Some(std::path::PathBuf::from(value)),
            _ => anyhow::bail!("Unexpected command-line argument"),
        }
        argument = arguments.next();
    }
    let storage = match data.map_or_else(
        scribe_seek::storage::Storage::discover,
        scribe_seek::storage::Storage::at,
    ) {
        Ok(storage) => storage,
        Err(error) => {
            rfd::MessageDialog::new()
                .set_title("Scribe Seek")
                .set_description(format!(
                    "{}:\n{error:#}",
                    scribe_seek::i18n::Locale::English.tr("Could not open the data directory")
                ))
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
        rfd::MessageDialog::new().set_title("Scribe Seek").set_description(locale.tr("This build requires a CPU with AVX2, FMA and F16C (Intel Haswell / AMD Ryzen or newer).")).set_level(rfd::MessageLevel::Error).show();
        return Ok(());
    }
    #[cfg(windows)]
    let result = scribe_seek::desktop::run(storage, source, cleanup);
    #[cfg(not(windows))]
    let _ = cleanup;
    #[cfg(not(windows))]
    let result = run_eframe(storage, source);
    if let Err(error) = result {
        rfd::MessageDialog::new()
            .set_title("Scribe Seek")
            .set_description(format!(
                "{}:\n{error:#}",
                locale.tr("Could not start Scribe Seek")
            ))
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
