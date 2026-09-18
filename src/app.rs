use crate::{
    audio::Audio,
    i18n::{LOCALES, Locale},
    languages::{language_name, speech_languages, valid_selection},
    model::MODELS,
    playback::Playback,
    settings::Settings,
    storage::{Storage, atomic_write},
    transcript::{Transcript, clock},
    updater::{self, Downloaded, UpdateState, Updater},
    worker::{Event, Job, Task},
};
use eframe::egui::{self, Color32, FontId, RichText, Stroke, Vec2};
use std::{
    path::{Path, PathBuf},
    time::Duration,
};

const BG: Color32 = Color32::from_rgb(247, 248, 252);
const PANEL: Color32 = Color32::WHITE;
const BORDER: Color32 = Color32::from_rgb(224, 228, 239);
const MUTED: Color32 = Color32::from_rgb(113, 122, 143);
const ACCENT: Color32 = Color32::from_rgb(91, 78, 232);
const TEXT: Color32 = Color32::from_rgb(28, 32, 51);
const SOFT: Color32 = Color32::from_rgb(238, 236, 255);

pub struct App {
    storage: Storage,
    library: Vec<Transcript>,
    source: Option<PathBuf>,
    fingerprint: (u64, u64),
    audio: Option<Audio>,
    document: Option<Transcript>,
    playback: Playback,
    job: Option<Job>,
    phase: String,
    progress: f32,
    notice: Option<(String, bool)>,
    model: usize,
    language: String,
    language_filter: String,
    locale: Locale,
    show_settings: bool,
    check_updates_on_startup: bool,
    updater: Updater,
    restart_update: Option<Downloaded>,
    query: String,
    matches: Vec<usize>,
    match_cursor: usize,
    scroll_to: Option<usize>,
    only_matches: bool,
    speed: f32,
    focus_search: bool,
    show_licenses: bool,
}

impl App {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        storage: Storage,
        source: Option<PathBuf>,
    ) -> Self {
        Self::with_context(&cc.egui_ctx, storage, source)
    }

    fn with_context(ctx: &egui::Context, storage: Storage, source: Option<PathBuf>) -> Self {
        ctx.set_visuals(egui::Visuals::light());
        let mut style = (*ctx.style()).clone();
        style.visuals.panel_fill = BG;
        style.visuals.window_fill = PANEL;
        style.visuals.extreme_bg_color = BG;
        style.visuals.faint_bg_color = PANEL;
        style.visuals.override_text_color = Some(TEXT);
        style.visuals.selection.bg_fill = SOFT;
        style.visuals.selection.stroke = Stroke::new(1.0_f32, ACCENT);
        style.visuals.hyperlink_color = ACCENT;
        style.visuals.window_stroke = Stroke::new(1.0_f32, BORDER);
        style.visuals.window_corner_radius = egui::CornerRadius::same(14);
        style.visuals.widgets.inactive.bg_fill = PANEL;
        style.visuals.widgets.inactive.weak_bg_fill = PANEL;
        style.visuals.widgets.inactive.bg_stroke = Stroke::new(1.0_f32, BORDER);
        style.visuals.widgets.inactive.corner_radius = egui::CornerRadius::same(8);
        style.visuals.widgets.inactive.fg_stroke = Stroke::new(1.0_f32, MUTED);
        style.visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, BORDER);
        style.visuals.widgets.inactive.expansion = 0.0;
        style.visuals.widgets.hovered.bg_fill = SOFT;
        style.visuals.widgets.hovered.weak_bg_fill = SOFT;
        style.visuals.widgets.hovered.bg_stroke = Stroke::new(1.0_f32, ACCENT);
        style.visuals.widgets.hovered.corner_radius = egui::CornerRadius::same(8);
        style.visuals.widgets.hovered.fg_stroke = Stroke::new(1.0_f32, ACCENT);
        style.visuals.widgets.hovered.expansion = 0.0;
        style.visuals.widgets.active.bg_fill = Color32::from_rgb(221, 216, 255);
        style.visuals.widgets.active.weak_bg_fill = Color32::from_rgb(221, 216, 255);
        style.visuals.widgets.active.corner_radius = egui::CornerRadius::same(8);
        style.visuals.widgets.active.fg_stroke = Stroke::new(1.0_f32, ACCENT);
        style.visuals.widgets.active.expansion = 0.0;
        style.visuals.widgets.open.expansion = 0.0;
        style.spacing.scroll = egui::style::ScrollStyle {
            floating: false,
            foreground_color: true,
            bar_width: 4.0,
            bar_inner_margin: 8.0,
            ..egui::style::ScrollStyle::solid()
        };
        style.spacing.item_spacing = Vec2::new(10.0, 10.0);
        style.spacing.button_padding = Vec2::new(14.0, 9.0);
        style
            .text_styles
            .insert(egui::TextStyle::Body, FontId::proportional(16.0));
        style
            .text_styles
            .insert(egui::TextStyle::Button, FontId::proportional(15.0));
        style
            .text_styles
            .insert(egui::TextStyle::Small, FontId::proportional(13.0));
        ctx.set_style(style);
        let (settings, settings_notice) = match Settings::load(&storage) {
            Ok(settings) => (settings, None),
            Err(error) => (
                Settings::default(),
                Some((format!("Could not load settings: {error:#}"), true)),
            ),
        };
        let (library, notice) = match storage.library() {
            Ok((library, 0)) => (library, None),
            Ok((library, n)) => (
                library,
                Some((format!("Could not read saved transcripts: {n}"), true)),
            ),
            Err(e) => (
                Vec::new(),
                Some((format!("Could not read the library: {e:#}"), true)),
            ),
        };
        let mut app = Self {
            storage,
            library,
            source: None,
            fingerprint: (0, 0),
            audio: None,
            document: None,
            playback: Playback::default(),
            job: None,
            phase: String::new(),
            progress: 0.0,
            notice: notice.or(settings_notice),
            model: MODELS
                .iter()
                .position(|m| m.id == settings.model)
                .unwrap_or(1),
            language: if valid_selection(&settings.speech_language) {
                settings.speech_language
            } else {
                "auto".into()
            },
            language_filter: String::new(),
            locale: settings.ui_language,
            show_settings: false,
            check_updates_on_startup: settings.check_updates_on_startup,
            updater: Updater::default(),
            restart_update: None,
            query: String::new(),
            matches: Vec::new(),
            match_cursor: 0,
            scroll_to: None,
            only_matches: false,
            speed: 1.0,
            focus_search: false,
            show_licenses: false,
        };
        if let Some(source) = source {
            app.open_media(source);
        }
        if app.check_updates_on_startup {
            app.updater.check();
        }
        app
    }

    fn current_settings(&self) -> Settings {
        Settings {
            ui_language: self.locale,
            speech_language: self.language.clone(),
            model: MODELS[self.model].id.into(),
            check_updates_on_startup: self.check_updates_on_startup,
        }
    }

    fn speech_language_picker(&mut self, ui: &mut egui::Ui) {
        let l = self.locale;
        ui.label(
            RichText::new(l.tr("Speech language"))
                .size(12.0)
                .color(MUTED),
        );
        egui::ComboBox::from_id_salt("speech_language")
            .width(ui.available_width())
            .truncate()
            .height(360.0)
            .selected_text(language_name(l, &self.language).to_owned())
            .show_ui(ui, |ui| {
                ui.set_min_width(250.0);
                ui.add(
                    egui::TextEdit::singleline(&mut self.language_filter)
                        .hint_text(l.tr("Search languages…")),
                );
                if ui
                    .selectable_label(self.language == "auto", l.tr("Detect automatically"))
                    .clicked()
                {
                    self.language = "auto".into();
                    self.language_filter.clear();
                    ui.close();
                }
                ui.separator();
                let filter = self.language_filter.trim().to_lowercase();
                let mut languages = speech_languages();
                languages.sort_by_key(|language| language_name(l, language.code).to_lowercase());
                let mut found = false;
                for language in languages {
                    let name = language_name(l, language.code);
                    if !filter.is_empty()
                        && !name.to_lowercase().contains(&filter)
                        && !language_name(Locale::English, language.code)
                            .to_lowercase()
                            .contains(&filter)
                        && !language.code.contains(&filter)
                    {
                        continue;
                    }
                    found = true;
                    let response = ui
                        .add_enabled(
                            language.model_supported,
                            egui::Button::new(format!("{name}  ({})", language.code))
                                .selected(self.language == language.code),
                        )
                        .on_disabled_hover_text(l.tr(
                            "Requires a Whisper v3 model; unavailable with Tiny, Base and Small.",
                        ));
                    if response.clicked() {
                        self.language = language.code.into();
                        self.language_filter.clear();
                        ui.close();
                    }
                }
                if !found {
                    ui.label(l.tr("No languages found"));
                }
            });
    }

    fn settings_window(&mut self, ctx: &egui::Context) {
        let l = self.locale;
        let mut open = self.show_settings;
        egui::Window::new(l.tr("Settings"))
            .id(egui::Id::new("settings_window"))
            .open(&mut open)
            .default_width(510.0)
            .vscroll(true)
            .max_height(660.0)
            .resizable(false)
            .show(ctx, |ui| {
                ui.label(RichText::new(l.tr("Interface language")).strong());
                egui::ComboBox::from_id_salt("ui_language")
                    .width(280.0)
                    .height(320.0)
                    .selected_text(self.locale.native_name())
                    .show_ui(ui, |ui| {
                        for locale in LOCALES {
                            ui.selectable_value(&mut self.locale, locale, locale.native_name());
                        }
                    });
                ui.add_space(8.0);
                ui.label(
                    RichText::new(l.tr("Changes apply immediately and are saved automatically."))
                        .color(MUTED)
                        .size(13.0),
                );
                ui.label(
                    RichText::new(
                        l.tr("Interface language does not change the language of your recording."),
                    )
                    .color(MUTED)
                    .size(13.0),
                );
                ui.add_space(12.0);
                ui.separator();
                ui.label(RichText::new(l.tr("Storage")).strong());
                ui.label(
                    RichText::new(l.tr("Models are stored in"))
                        .size(13.0)
                        .color(MUTED),
                );
                let models = self.storage.models_dir();
                let data = self.storage.root.clone();
                ui.label(RichText::new(display_path(&models)).monospace().size(12.0));
                if ui.button(l.tr("Open models folder")).clicked() {
                    self.open_folder(&models);
                }
                ui.add_space(6.0);
                ui.label(
                    RichText::new(l.tr("Settings and transcripts are stored in"))
                        .size(13.0)
                        .color(MUTED),
                );
                ui.label(RichText::new(display_path(&data)).monospace().size(12.0));
                ui.label(
                    RichText::new("settings.json")
                        .monospace()
                        .size(12.0)
                        .color(MUTED),
                );
                if ui.button(l.tr("Open data folder")).clicked() {
                    self.open_folder(&data);
                }
                ui.add_space(12.0);
                ui.separator();
                self.update_settings_ui(ui);
            });
        self.show_settings = open;
    }

    fn open_folder(&mut self, path: &Path) {
        if let Err(error) = Storage::open_folder(path) {
            self.notice = Some((format!("{error:#}"), true));
        }
    }

    fn update_settings_ui(&mut self, ui: &mut egui::Ui) {
        let l = self.locale;
        ui.label(RichText::new(l.tr("Updates")).strong());
        ui.label(l.format(
            "Current version: {version}",
            &[("version", env!("CARGO_PKG_VERSION").into())],
        ));
        ui.checkbox(
            &mut self.check_updates_on_startup,
            l.tr("Check for updates at startup"),
        );
        match self.updater.state.clone() {
            UpdateState::Idle => {}
            UpdateState::Checking => {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(l.tr("Checking GitHub Releases…"));
                });
            }
            UpdateState::UpToDate => {
                status_line(ui, l.tr("You are up to date"), ACCENT);
            }
            UpdateState::NoReleases => {
                ui.label(l.tr("No releases have been published yet"));
            }
            UpdateState::Available(release) => {
                ui.label(l.format(
                    "Version {version} is available",
                    &[("version", release.version.clone())],
                ));
                if ui.button(l.tr("Download update")).clicked() {
                    self.updater.download(release, self.storage.clone());
                }
            }
            UpdateState::Downloading(progress) => {
                ui.label(l.tr("Downloading update…"));
                ui.add(
                    egui::ProgressBar::new(progress)
                        .fill(ACCENT)
                        .show_percentage(),
                );
                if ui.button(l.tr("Cancel")).clicked() {
                    self.updater.cancel();
                }
            }
            UpdateState::Ready(downloaded) => {
                ui.label(l.format(
                    "Version {version} is ready to install",
                    &[("version", downloaded.release.version.clone())],
                ));
                if ui
                    .add_enabled(
                        self.job.is_none(),
                        egui::Button::new(l.tr("Restart and update")),
                    )
                    .clicked()
                {
                    self.restart_update = Some(downloaded);
                }
                if self.job.is_some() {
                    ui.label(l.tr("Finish the current operation before restarting"));
                }
            }
            UpdateState::Failed(error) => {
                ui.label(RichText::new(l.message(&error)).color(Color32::from_rgb(175, 55, 55)));
            }
        }
        if ui
            .add_enabled(
                !self.updater.busy(),
                egui::Button::new(l.tr("Check for updates")),
            )
            .clicked()
        {
            self.updater.check();
        }
        ui.hyperlink_to(l.tr("View releases on GitHub"), updater::RELEASES_URL);
    }

    fn start(&mut self, task: Task, phase: &str) {
        self.notice = None;
        self.progress = 0.0;
        self.phase = phase.into();
        self.job = Some(Job::spawn(task, self.storage.clone()));
    }

    fn poll(&mut self) {
        let events: Vec<_> = self
            .job
            .as_ref()
            .map_or_else(Vec::new, |j| j.events.try_iter().collect());
        for event in events {
            match event {
                Event::Progress(phase, progress) => {
                    self.phase = phase;
                    self.progress = progress;
                }
                Event::AudioReady(path, audio, fingerprint) => {
                    self.source = Some(path.clone());
                    self.fingerprint = fingerprint;
                    self.playback.load(audio.clone());
                    self.speed = 1.0;
                    self.audio = Some(audio);
                    let cached = self
                        .library
                        .iter()
                        .find(|t| {
                            t.source == path && (t.source_size, t.source_modified_ms) == fingerprint
                        })
                        .cloned();
                    if self
                        .document
                        .as_ref()
                        .is_some_and(|t| (t.source_size, t.source_modified_ms) != fingerprint)
                    {
                        self.notice =
                            Some(("Source file changed. Transcribe it again.".into(), true));
                    }
                    self.document = cached;
                    self.update_matches();
                }
                Event::ModelReady => {
                    self.notice =
                        Some(("Model ready. You can now transcribe offline.".into(), false))
                }
                Event::TranscriptReady(document) => {
                    self.library.retain(|t| t.source != document.source);
                    self.library.insert(0, document.clone());
                    self.notice = Some((
                        if document.segments.is_empty() {
                            "No speech found. Try another model or language."
                        } else {
                            "Transcript ready and saved in the library."
                        }
                        .into(),
                        false,
                    ));
                    self.document = Some(document);
                    self.update_matches();
                }
                Event::Error(error) => self.notice = Some((error, true)),
                Event::Finished => self.job = None,
            }
        }
    }

    fn open_media(&mut self, path: PathBuf) {
        if self.job.is_some() {
            return;
        }
        let path = std::fs::canonicalize(&path).unwrap_or(path);
        self.document = self.library.iter().find(|t| t.source == path).cloned();
        self.source = Some(path.clone());
        self.audio = None;
        self.playback.clear();
        self.query.clear();
        self.update_matches();
        if path.is_file() {
            self.start(Task::Decode(path), "Opening recording");
        } else {
            self.notice = Some((
                "Source recording unavailable. You can still read and export its saved transcript."
                    .into(),
                true,
            ));
        }
    }

    fn choose_file(&mut self) {
        let l = self.locale;
        if let Some(path) = rfd::FileDialog::new()
            .set_title(l.tr("Open audio or video"))
            .add_filter(
                l.tr("Audio and video"),
                &[
                    "wav", "mp3", "flac", "m4a", "mp4", "mkv", "ogg", "opus", "aac", "aiff", "mov",
                ],
            )
            .add_filter(l.tr("All files"), &["*"])
            .pick_file()
        {
            self.open_media(path);
        }
    }

    fn update_matches(&mut self) {
        self.matches = self
            .document
            .as_ref()
            .map_or_else(Vec::new, |t| t.search(&self.query));
        self.match_cursor = 0;
        self.scroll_to = self.matches.first().copied();
    }

    fn next_match(&mut self, backwards: bool) {
        if self.matches.is_empty() {
            return;
        }
        self.match_cursor = if backwards {
            (self.match_cursor + self.matches.len() - 1) % self.matches.len()
        } else {
            (self.match_cursor + 1) % self.matches.len()
        };
        self.scroll_to = Some(self.matches[self.match_cursor]);
    }

    fn seek(&mut self, seconds: f64, play: bool) {
        if let Err(e) = self.playback.seek(seconds, play) {
            self.notice = Some((format!("{e:#}"), true));
        }
    }

    fn export(&mut self, extension: &str) {
        let l = self.locale;
        let Some(document) = &self.document else {
            return;
        };
        let stem = document
            .source
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy();
        if let Some(path) = rfd::FileDialog::new()
            .set_title(l.tr("Export transcript"))
            .set_file_name(format!("{stem}.{extension}"))
            .add_filter(extension.to_uppercase(), &[extension])
            .save_file()
        {
            let content = match extension {
                "srt" => document.srt(),
                "vtt" => document.vtt(),
                "json" => match serde_json::to_string_pretty(document) {
                    Ok(text) => text,
                    Err(e) => {
                        self.notice = Some((e.to_string(), true));
                        return;
                    }
                },
                _ => document.text(),
            };
            self.notice = Some(match atomic_write(&path, content.as_bytes()) {
                Ok(()) => (format!("Saved: {}", path.display()), false),
                Err(e) => (format!("{e:#}"), true),
            });
        }
    }

    fn sidebar(&mut self, ctx: &egui::Context) -> (egui::Rect, egui::Rect) {
        let l = self.locale;
        let panel = egui::SidePanel::left("library")
            .exact_width(280.0)
            .resizable(false)
            .frame(egui::Frame::new().fill(PANEL).inner_margin(20.0))
            .show(ctx, |ui| {
                ui.spacing_mut().item_spacing = Vec2::new(8.0, 6.0);
                ui.spacing_mut().button_padding = Vec2::new(10.0, 6.0);
                ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);

                // Keep actions visible; only the recording list scrolls.
                egui::TopBottomPanel::bottom("recognition_controls")
                    .show_separator_line(false)
                    .frame(egui::Frame::NONE)
                    .show_inside(ui, |ui| self.recognition_controls(ui));
                egui::TopBottomPanel::top("sidebar_header")
                    .show_separator_line(false)
                    .frame(egui::Frame::NONE)
                    .show_inside(ui, |ui| {
                        ui.horizontal(|ui| {
                            let (rect, _) =
                                ui.allocate_exact_size(Vec2::splat(36.0), egui::Sense::hover());
                            ui.painter().rect_filled(rect, 10.0, ACCENT);
                            for (i, h) in [9.0, 19.0, 13.0, 23.0, 11.0].iter().enumerate() {
                                let x = rect.left() + 9.0 + i as f32 * 4.5;
                                ui.painter().line_segment(
                                    [
                                        egui::pos2(x, rect.center().y - h / 2.0),
                                        egui::pos2(x, rect.center().y + h / 2.0),
                                    ],
                                    Stroke::new(2.0_f32, PANEL),
                                );
                            }
                            ui.label(RichText::new("scribe seek").size(23.0).strong());
                        });
                        ui.add_space(16.0);
                        if ui
                            .add_enabled(
                                self.job.is_none(),
                                egui::Button::new(
                                    RichText::new(l.tr("+  New recording"))
                                        .color(PANEL)
                                        .strong(),
                                )
                                .fill(ACCENT)
                                .min_size(Vec2::new(ui.available_width(), 40.0)),
                            )
                            .clicked()
                        {
                            self.choose_file();
                        }
                        ui.add_space(4.0);
                        if text_action(ui, l.tr("Settings")).clicked() {
                            self.show_settings = true;
                        }
                        ui.add_space(18.0);
                    });
                egui::CentralPanel::default()
                    .frame(egui::Frame::NONE)
                    .show_inside(ui, |ui| {
                        ui.horizontal(|ui| {
                            eyebrow(ui, l.tr("LIBRARY"));
                            ui.label(
                                RichText::new(self.library.len().to_string())
                                    .color(MUTED)
                                    .small(),
                            );
                        });
                        ui.add_space(6.0);
                        let mut selected = None;
                        egui::ScrollArea::vertical()
                            .id_salt("library_scroll")
                            .auto_shrink([false, false])
                            .show(ui, |ui| {
                                if self.library.is_empty() {
                                    ui.add(
                                        egui::Label::new(
                                            RichText::new(
                                                l.tr("No recordings yet.\nAdd your first one."),
                                            )
                                            .color(MUTED)
                                            .size(13.0),
                                        )
                                        .wrap(),
                                    );
                                }
                                ui.add_enabled_ui(self.job.is_none(), |ui| {
                                    for document in &self.library {
                                        let active = self.source.as_ref() == Some(&document.source);
                                        let summary = format!(
                                            "{} · {}",
                                            clock(document.duration),
                                            l.segments(document.segments.len())
                                        );
                                        if recording_item(ui, &document.title(), &summary, active)
                                            .on_hover_text(document.source.display().to_string())
                                            .clicked()
                                        {
                                            selected = Some(document.source.clone());
                                        }
                                    }
                                });
                            });
                        if let Some(path) = selected {
                            self.open_media(path);
                        }
                    });
                ui.min_rect()
            });
        (panel.response.rect, panel.inner)
    }

    fn recognition_controls(&mut self, ui: &mut egui::Ui) {
        let l = self.locale;
        ui.separator();
        ui.add_space(8.0);
        eyebrow(ui, l.tr("TRANSCRIPTION"));
        ui.add_space(4.0);
        ui.add_enabled_ui(self.job.is_none(), |ui| {
            ui.label(RichText::new(l.tr("Model")).size(12.0).color(MUTED));
            egui::ComboBox::from_id_salt("model")
                .width(ui.available_width())
                .truncate()
                .selected_text(l.tr(MODELS[self.model].label))
                .show_ui(ui, |ui| {
                    for (i, model) in MODELS.iter().enumerate() {
                        ui.selectable_value(&mut self.model, i, l.tr(model.label));
                    }
                });
            let model = MODELS[self.model];
            if model.installed(&self.storage) {
                status_line(ui, l.tr("Model installed"), ACCENT);
                if text_action(ui, l.tr("Download again")).clicked() {
                    self.start(Task::Download(model), "Connecting to Hugging Face");
                }
            } else if ui.button(l.tr("Download model")).clicked() {
                self.start(Task::Download(model), "Connecting to Hugging Face");
            }
            ui.add_space(6.0);
            self.speech_language_picker(ui);
            ui.add_space(8.0);
            let ready = self.audio.is_some() && model.installed(&self.storage);
            if ui
                .add_enabled(
                    ready,
                    egui::Button::new(
                        RichText::new(if self.document.is_some() {
                            l.tr("Transcribe again")
                        } else {
                            l.tr("Transcribe recording")
                        })
                        .color(if ready { PANEL } else { MUTED })
                        .strong(),
                    )
                    .fill(if ready { ACCENT } else { SOFT })
                    .min_size(Vec2::new(ui.available_width(), 38.0)),
                )
                .clicked()
            {
                self.start(
                    Task::Transcribe {
                        source: self.source.clone().unwrap(),
                        audio: self.audio.clone().unwrap(),
                        fingerprint: self.fingerprint,
                        model,
                        language: self.language.clone(),
                    },
                    "Verifying and loading model",
                );
            }
        });
        ui.add_space(12.0);
        status_line(ui, l.tr("Local. Your recording stays here."), MUTED);
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(format!("v{}", env!("CARGO_PKG_VERSION")))
                    .color(MUTED)
                    .size(12.0),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if text_action(ui, l.tr("Licenses")).clicked() {
                    self.show_licenses = true;
                }
            });
        });
        if matches!(
            self.updater.state,
            UpdateState::Available(_) | UpdateState::Ready(_)
        ) && ui.button(l.tr("Update available")).clicked()
        {
            self.show_settings = true;
        }
    }

    fn transport(&mut self, ctx: &egui::Context) {
        let l = self.locale;
        if self.source.is_none() {
            return;
        }
        egui::TopBottomPanel::bottom("transport")
            .frame(
                egui::Frame::new()
                    .fill(PANEL)
                    .inner_margin(egui::Margin::symmetric(28, 16)),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.add_enabled_ui(self.playback.ready(), |ui| {
                        if ui.button(l.tr("−10 s")).clicked() {
                            self.seek(self.playback.position() - 10.0, false);
                        }
                        if ui
                            .scope(|ui| {
                                icon_text_button(
                                    ui,
                                    if self.playback.playing() {
                                        l.tr("Pause")
                                    } else {
                                        l.tr("Play")
                                    },
                                    if self.playback.playing() {
                                        Icon::Pause
                                    } else {
                                        Icon::Play
                                    },
                                )
                            })
                            .inner
                            .clicked()
                            && let Err(e) = self.playback.toggle()
                        {
                            self.notice = Some((format!("{e:#}"), true));
                        }
                        if ui.button(l.tr("+10 s")).clicked() {
                            self.seek(self.playback.position() + 10.0, false);
                        }
                    });
                    let duration = self.audio.as_ref().map_or_else(
                        || self.document.as_ref().map_or(0.0, |t| t.duration),
                        Audio::duration,
                    );
                    ui.label(
                        RichText::new(format!(
                            "{} / {}",
                            clock(self.playback.position()),
                            clock(duration)
                        ))
                        .monospace()
                        .size(14.0),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        egui::ComboBox::from_id_salt("speed")
                            .width(68.0)
                            .selected_text(format!("{}×", self.speed))
                            .show_ui(ui, |ui| {
                                for speed in [0.75, 1.0, 1.25, 1.5, 2.0] {
                                    if ui
                                        .selectable_value(
                                            &mut self.speed,
                                            speed,
                                            format!("{speed}×"),
                                        )
                                        .changed()
                                    {
                                        self.playback.set_speed(speed);
                                    }
                                }
                            });
                    });
                });
                ui.add_space(5.0);
                let (rect, response) = ui.allocate_exact_size(
                    Vec2::new(ui.available_width(), 43.0),
                    egui::Sense::click_and_drag(),
                );
                let painter = ui.painter_at(rect);
                if let Some(audio) = &self.audio {
                    let position = self.playback.position() / audio.duration();
                    let bars = (rect.width() / 4.0).max(1.0) as usize;
                    for bar in 0..bars {
                        let index = (bar * audio.peaks.len() / bars).min(audio.peaks.len() - 1);
                        let peak = audio.peaks[index].sqrt();
                        let x = rect.left() + bar as f32 * rect.width() / bars as f32;
                        let half = (peak * 19.0).max(1.5);
                        painter.line_segment(
                            [
                                egui::pos2(x, rect.center().y - half),
                                egui::pos2(x, rect.center().y + half),
                            ],
                            Stroke::new(
                                2.0_f32,
                                if (bar as f64 / bars as f64) <= position {
                                    ACCENT
                                } else {
                                    BORDER
                                },
                            ),
                        );
                    }
                    if (response.clicked() || response.dragged())
                        && let Some(pointer) = response.interact_pointer_pos()
                    {
                        let seconds = ((pointer.x - rect.left()) / rect.width()).clamp(0.0, 1.0)
                            as f64
                            * audio.duration();
                        self.seek(seconds, false);
                    }
                    response.on_hover_text(l.tr("Click to jump to this moment"));
                } else {
                    painter.line_segment(
                        [rect.left_center(), rect.right_center()],
                        Stroke::new(2.0_f32, BORDER),
                    );
                }
            });
    }

    fn empty(&mut self, ui: &mut egui::Ui) {
        let l = self.locale;
        ui.horizontal(|ui| {
            eyebrow(ui, l.tr("YOUR SPACE FOR RECORDINGS"));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                egui::Frame::new()
                    .fill(SOFT)
                    .corner_radius(20.0)
                    .inner_margin(egui::Margin::symmetric(12, 5))
                    .show(ui, |ui| {
                        ui.label(
                            RichText::new(l.tr("100% local"))
                                .color(ACCENT)
                                .size(12.0)
                                .strong(),
                        );
                    });
            });
        });
        ui.add_space(36.0);
        let headline = if ui.available_width() < 720.0 {
            38.0
        } else {
            46.0
        };
        ui.spacing_mut().item_spacing.y = 7.0;
        ui.label(
            RichText::new(l.tr("Forty minutes of audio."))
                .size(headline)
                .strong(),
        );
        ui.label(
            RichText::new(l.tr("One phrase that matters."))
                .size(headline)
                .color(ACCENT)
                .strong(),
        );
        ui.add_space(13.0);
        ui.label(RichText::new(l.tr("Turn lectures, interviews and voice notes into text.\nFind what matters — and listen from that exact moment.")).size(17.0).color(MUTED));
        ui.add_space(28.0);
        let (rect, response) =
            ui.allocate_exact_size(Vec2::new(ui.available_width(), 172.0), egui::Sense::click());
        let painter = ui.painter_at(rect);
        painter.rect_filled(
            rect.translate(Vec2::new(0.0, 4.0)),
            16.0,
            Color32::from_rgb(235, 236, 245),
        );
        painter.rect_filled(rect, 16.0, PANEL);
        painter.rect_stroke(
            rect,
            16.0,
            Stroke::new(1.5_f32, if response.hovered() { ACCENT } else { BORDER }),
            egui::StrokeKind::Inside,
        );
        let icon = rect.left_top() + Vec2::new(54.0, 61.0);
        painter.rect_filled(
            egui::Rect::from_center_size(icon, Vec2::splat(54.0)),
            14.0,
            SOFT,
        );
        painter.line_segment(
            [icon + Vec2::new(0.0, 9.0), icon - Vec2::new(0.0, 10.0)],
            Stroke::new(2.0_f32, ACCENT),
        );
        painter.line_segment(
            [icon - Vec2::new(7.0, 3.0), icon - Vec2::new(0.0, 10.0)],
            Stroke::new(2.0_f32, ACCENT),
        );
        painter.line_segment(
            [icon + Vec2::new(7.0, -3.0), icon - Vec2::new(0.0, 10.0)],
            Stroke::new(2.0_f32, ACCENT),
        );
        painter.line_segment(
            [icon + Vec2::new(-10.0, 13.0), icon + Vec2::new(10.0, 13.0)],
            Stroke::new(2.0_f32, ACCENT),
        );
        let text_x = rect.left() + 100.0;
        painter.text(
            egui::pos2(text_x, rect.top() + 48.0),
            egui::Align2::LEFT_CENTER,
            l.tr("Drop your recording here"),
            FontId::proportional(21.0),
            TEXT,
        );
        painter.text(
            egui::pos2(text_x, rect.top() + 80.0),
            egui::Align2::LEFT_CENTER,
            l.tr("Audio or video. We’ll find the words."),
            FontId::proportional(14.0),
            MUTED,
        );
        let button = egui::Rect::from_min_size(
            egui::pos2(rect.right() - 166.0, rect.top() + 42.0),
            Vec2::new(140.0, 42.0),
        );
        painter.rect_filled(button, 9.0, ACCENT);
        painter.text(
            button.center(),
            egui::Align2::CENTER_CENTER,
            l.tr("Choose file"),
            FontId::proportional(14.0),
            PANEL,
        );
        painter.line_segment(
            [
                egui::pos2(rect.left() + 26.0, rect.bottom() - 47.0),
                egui::pos2(rect.right() - 26.0, rect.bottom() - 47.0),
            ],
            Stroke::new(1.0_f32, BORDER),
        );
        painter.text(
            egui::pos2(rect.left() + 28.0, rect.bottom() - 23.0),
            egui::Align2::LEFT_CENTER,
            "MP3   WAV   M4A   OGG   MP4   MKV",
            FontId::proportional(11.0),
            MUTED,
        );
        painter.text(
            egui::pos2(rect.right() - 28.0, rect.bottom() - 23.0),
            egui::Align2::RIGHT_CENTER,
            l.tr("No cloud uploads"),
            FontId::proportional(12.0),
            ACCENT,
        );
        if response.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        if response.clicked() && self.job.is_none() {
            self.choose_file();
        }
        ui.add_space(24.0);
        ui.columns(3, |columns| {
            for (column, (number, title, body)) in columns.iter_mut().zip([
                (
                    "01",
                    l.tr("Open a recording"),
                    l.tr("Audio or the soundtrack\nof a video"),
                ),
                (
                    "02",
                    l.tr("Get the transcript"),
                    l.tr("Whisper transcribes speech\non your computer"),
                ),
                (
                    "03",
                    l.tr("Click a phrase"),
                    l.tr("Playback starts\nfrom its timestamp"),
                ),
            ]) {
                egui::Frame::new()
                    .fill(PANEL)
                    .stroke(Stroke::new(1.0_f32, BORDER))
                    .corner_radius(12.0)
                    .inner_margin(16.0)
                    .show(column, |ui| {
                        ui.set_min_width(ui.available_width());
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(number).color(ACCENT).size(12.0).strong());
                            ui.label(RichText::new(title).strong().size(14.0));
                        });
                        ui.add_space(7.0);
                        ui.label(RichText::new(body).color(MUTED).size(13.0));
                    });
            }
        });
        ui.add_space(20.0);
        ui.label(
            RichText::new(l.tr("No account. No subscription. At your own pace."))
                .size(12.0)
                .color(MUTED),
        );
    }

    fn content(&mut self, ui: &mut egui::Ui) {
        let l = self.locale;
        let title = self
            .source
            .as_ref()
            .and_then(|p| p.file_name())
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        eyebrow(ui, l.tr("YOUR RECORDING"));
        ui.add_space(5.0);
        ui.horizontal(|ui| {
            ui.allocate_ui_with_layout(
                Vec2::new((ui.available_width() - 152.0).max(100.0), 38.0),
                egui::Layout::left_to_right(egui::Align::Center),
                |ui| {
                    ui.add(egui::Label::new(RichText::new(title).size(25.0).strong()).truncate());
                },
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add_enabled_ui(self.document.is_some(), |ui| {
                    let menu = ui.menu_button(format!("{}    ", l.tr("Export")), |ui| {
                        for (extension, name) in [
                            ("txt", l.tr("Text · TXT")),
                            ("srt", l.tr("Subtitles · SRT")),
                            ("vtt", l.tr("Subtitles · VTT")),
                            ("json", l.tr("Timestamps · JSON")),
                        ] {
                            if ui.button(name).clicked() {
                                ui.close();
                                self.export(extension);
                            }
                        }
                    });
                    paint_icon(
                        ui.painter(),
                        egui::Rect::from_center_size(
                            menu.response.rect.right_center() - Vec2::new(14.0, 0.0),
                            Vec2::splat(14.0),
                        ),
                        Icon::Down,
                        TEXT,
                    );
                });
            });
        });
        ui.horizontal(|ui| {
            if let Some(document) = &self.document {
                ui.label(
                    RichText::new(format!(
                        "{}  /  {}  /  {}  /  Whisper {}",
                        clock(document.duration),
                        l.segments(document.segments.len()),
                        language_name(l, &document.language),
                        document.model
                    ))
                    .color(MUTED)
                    .small(),
                );
            } else if let Some(audio) = &self.audio {
                ui.label(
                    RichText::new(format!(
                        "{}  /  {}",
                        clock(audio.duration()),
                        l.tr("Ready to transcribe")
                    ))
                    .color(MUTED)
                    .small(),
                );
            }
        });
        ui.add_space(20.0);
        if self.document.is_none() {
            ui.add_space(50.0);
            ui.label(RichText::new(l.tr("Recording opened")).size(25.0));
            ui.add_space(8.0);
            ui.label(RichText::new(l.tr("Choose a model and speech language, then click\n“Transcribe recording”. Your text stays on this computer.")).color(MUTED).size(17.0));
            ui.add_space(16.0);
            ui.label(
                RichText::new(l.tr(
                    "Choose Small for better accuracy.\nTiny is useful for a quick first try.",
                ))
                .color(MUTED)
                .size(14.0),
            );
            return;
        }
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            let search_width = (ui.available_width() - (3.0 * 28.0 + 56.0 + 4.0 * 6.0)).max(80.0);
            let response = ui.add_sized(
                [search_width, 36.0],
                egui::TextEdit::singleline(&mut self.query)
                    .hint_text(l.tr("Find a phrase…  Ctrl+F"))
                    .margin(egui::Margin::symmetric(12, 9)),
            );
            if self.focus_search {
                response.request_focus();
                self.focus_search = false;
            }
            if response.changed() {
                self.update_matches();
            }
            if response.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                self.next_match(ui.input(|i| i.modifiers.shift));
            }
            ui.add_sized(
                [56.0, 24.0],
                egui::Label::new(
                    RichText::new(if self.matches.is_empty() {
                        "0 / 0".into()
                    } else {
                        format!("{} / {}", self.match_cursor + 1, self.matches.len())
                    })
                    .color(MUTED)
                    .small(),
                )
                .truncate(),
            );
            if icon_button(ui, Icon::Up, l.tr("Previous match")).clicked() {
                self.next_match(true);
            }
            if icon_button(ui, Icon::Down, l.tr("Next match")).clicked() {
                self.next_match(false);
            }
            if icon_button(ui, Icon::Close, l.tr("Clear search")).clicked() {
                self.query.clear();
                self.update_matches();
            }
        });
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(l.tr("Click a segment to listen"))
                    .color(MUTED)
                    .small(),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.checkbox(
                    &mut self.only_matches,
                    RichText::new(l.tr("Matches only")).size(13.0),
                );
            });
        });
        ui.add_space(8.0);
        ui.separator();
        let document = self.document.as_ref().unwrap();
        let position = self.playback.position();
        let mut seek_to = None;
        let mut scrolled = false;
        egui::ScrollArea::vertical()
            .id_salt("transcript_scroll")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                if !self.query.trim().is_empty() && self.matches.is_empty() {
                    ui.add_space(20.0);
                    ui.label(
                        RichText::new(l.tr("No matches. Try part of the phrase.")).color(MUTED),
                    );
                }
                for (i, segment) in document.segments.iter().enumerate() {
                    let matched = self.matches.binary_search(&i).is_ok();
                    if self.only_matches && !self.query.trim().is_empty() && !matched {
                        continue;
                    }
                    let current_match = self.matches.get(self.match_cursor) == Some(&i);
                    let playing = self.playback.ready()
                        && position >= segment.start
                        && position < segment.end;
                    let fill = if current_match {
                        SOFT
                    } else if playing {
                        Color32::from_rgb(240, 243, 253)
                    } else {
                        Color32::TRANSPARENT
                    };
                    let response = egui::Frame::new()
                        .fill(fill)
                        .corner_radius(7.0)
                        .inner_margin(egui::Margin::symmetric(12, 13))
                        .show(ui, |ui| {
                            ui.horizontal_top(|ui| {
                                ui.add_sized(
                                    [54.0, 22.0],
                                    egui::Label::new(
                                        RichText::new(clock(segment.start))
                                            .monospace()
                                            .color(if matched || playing { ACCENT } else { MUTED })
                                            .size(13.0),
                                    ),
                                );
                                ui.add(
                                    egui::Label::new(highlight(&segment.text, &self.query)).wrap(),
                                );
                            });
                            ui.set_min_width(ui.available_width());
                        })
                        .response;
                    let clicked = ui.interact(
                        response.rect,
                        ui.id().with(("segment", i)),
                        egui::Sense::click(),
                    );
                    if clicked.hovered() {
                        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                    }
                    if clicked.clicked() {
                        seek_to = Some(segment.start);
                    }
                    if self.scroll_to == Some(i) {
                        response.scroll_to_me(Some(egui::Align::Center));
                        scrolled = true;
                    }
                    ui.add_space(3.0);
                }
                if document.segments.is_empty() {
                    ui.label(
                        RichText::new(l.tr("No speech was found in this recording")).color(MUTED),
                    );
                }
            });
        if scrolled {
            self.scroll_to = None;
        }
        if let Some(seconds) = seek_to {
            self.seek(seconds, true);
        }
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let l = self.locale;
        let settings_before = self.current_settings();
        self.poll();
        self.updater.poll();
        if self.job.is_some() || self.playback.playing() || self.updater.busy() {
            ctx.request_repaint_after(Duration::from_millis(80));
        }
        let dropped: Vec<_> = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .filter_map(|f| f.path.clone())
                .collect()
        });
        if let Some(path) = dropped.first() {
            if self.job.is_none() {
                self.open_media(path.clone());
            } else {
                self.notice = Some((
                    "Wait for the current operation to finish or cancel it".into(),
                    true,
                ));
            }
        }
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::CTRL, egui::Key::O))
            && self.job.is_none()
        {
            self.choose_file();
        }
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::CTRL, egui::Key::F)) {
            self.focus_search = true;
        }
        if !ctx.wants_keyboard_input()
            && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Space))
            && self.playback.ready()
            && let Err(e) = self.playback.toggle()
        {
            self.notice = Some((format!("{e:#}"), true));
        }
        self.sidebar(ctx);
        self.settings_window(ctx);
        self.transport(ctx);
        egui::Window::new(l.tr("Open-source licenses"))
            .id(egui::Id::new("licenses_window"))
            .open(&mut self.show_licenses)
            .default_size([740.0, 460.0])
            .show(ctx, |ui| {
                static LINES: std::sync::OnceLock<Vec<&'static str>> = std::sync::OnceLock::new();
                let lines = LINES.get_or_init(|| {
                    include_str!("../LICENSE")
                        .lines()
                        .chain(include_str!("../THIRD_PARTY_LICENSES.txt").lines())
                        .chain(include_str!("../locales/UNICODE_LICENSE.txt").lines())
                        .collect()
                });
                egui::ScrollArea::both()
                    .auto_shrink([false, false])
                    .show_rows(ui, 16.0, lines.len(), |ui, rows| {
                        for row in rows {
                            ui.add(
                                egui::Label::new(RichText::new(lines[row]).monospace().size(12.0))
                                    .extend(),
                            );
                        }
                    });
            });
        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(BG)
                    .inner_margin(egui::Margin::symmetric(34, 26)),
            )
            .show(ctx, |ui| {
                if let Some((notice, error)) = self.notice.clone() {
                    egui::Frame::new()
                        .fill(if error {
                            Color32::from_rgb(255, 235, 231)
                        } else {
                            SOFT
                        })
                        .corner_radius(6.0)
                        .inner_margin(12.0)
                        .show(ui, |ui| {
                            ui.horizontal_wrapped(|ui| {
                                ui.label(RichText::new(l.message(&notice)).size(14.0));
                                if icon_button(ui, Icon::Close, l.tr("Dismiss")).clicked() {
                                    self.notice = None;
                                }
                            });
                        });
                    ui.add_space(12.0);
                }
                if let Some(job) = &self.job {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(RichText::new(l.message(&self.phase)).size(14.0));
                        if text_action(ui, l.tr("Cancel")).clicked() {
                            job.cancel();
                            self.phase = "Cancelling…".into();
                        }
                    });
                    ui.add(
                        egui::ProgressBar::new(self.progress)
                            .fill(ACCENT)
                            .show_percentage()
                            .desired_height(4.0),
                    );
                    ui.add_space(18.0);
                }
                if self.source.is_none() {
                    egui::ScrollArea::vertical()
                        .auto_shrink([false, false])
                        .show(ui, |ui| self.empty(ui));
                } else {
                    self.content(ui);
                }
            });
        if ctx.input(|i| !i.raw.hovered_files.is_empty()) {
            let painter = ctx.layer_painter(egui::LayerId::new(
                egui::Order::Foreground,
                egui::Id::new("drop_overlay"),
            ));
            let rect = ctx.content_rect().shrink(12.0);
            painter.rect_filled(
                rect,
                14.0,
                Color32::from_rgba_unmultiplied(245, 243, 255, 248),
            );
            painter.rect_stroke(
                rect,
                14.0,
                Stroke::new(2.0_f32, ACCENT),
                egui::StrokeKind::Inside,
            );
            painter.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                if self.job.is_some() {
                    l.tr("Finish the current operation first")
                } else {
                    l.tr("Release the file to open it")
                },
                FontId::proportional(25.0),
                ACCENT,
            );
        }
        let settings_after = self.current_settings();
        if settings_before != settings_after {
            if let Err(error) = settings_after.save(&self.storage) {
                self.notice = Some((format!("Could not save settings: {error:#}"), true));
            }
            ctx.request_repaint();
        }
        if let Some(downloaded) = self.restart_update.take() {
            match updater::prepare_restart(
                &downloaded,
                &self.storage,
                self.source.clone(),
                self.locale,
            ) {
                Ok(()) => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
                Err(error) => self.updater.state = UpdateState::Failed(format!("{error:#}")),
            }
        }
    }
}

fn eyebrow(ui: &mut egui::Ui, text: &str) {
    ui.label(RichText::new(text).color(MUTED).size(11.0).strong());
}

fn text_action(ui: &mut egui::Ui, text: &str) -> egui::Response {
    // Small buttons remove vertical padding, which clips their rounded outlines
    // against the sidebar footer. Text actions keep a full hit area and no outline.
    ui.add(
        egui::Button::new(RichText::new(text).size(13.0).color(ACCENT))
            .frame(false)
            .min_size(Vec2::new(0.0, 28.0)),
    )
}

fn display_path(path: &Path) -> String {
    let absolute = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_owned());
    let text = absolute.to_string_lossy();
    if let Some(unc) = text.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{unc}")
    } else {
        text.strip_prefix(r"\\?\").unwrap_or(&text).to_owned()
    }
}

fn status_line(ui: &mut egui::Ui, text: &str, color: Color32) {
    ui.horizontal_top(|ui| {
        let (rect, _) = ui.allocate_exact_size(Vec2::new(7.0, 16.0), egui::Sense::hover());
        ui.painter().circle_filled(rect.center(), 2.5, color);
        ui.add(egui::Label::new(RichText::new(text).color(color).size(13.0)).wrap());
    });
}

fn recording_item(ui: &mut egui::Ui, title: &str, summary: &str, selected: bool) -> egui::Response {
    let (rect, response) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), 60.0), egui::Sense::click());
    let fill = if selected {
        SOFT
    } else if response.hovered() {
        BG
    } else {
        PANEL
    };
    ui.painter().rect(
        rect,
        8.0,
        fill,
        Stroke::new(1.0_f32, BORDER),
        egui::StrokeKind::Inside,
    );
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(rect.shrink2(Vec2::new(12.0, 9.0)))
            .layout(egui::Layout::top_down(egui::Align::Min)),
    );
    child.set_clip_rect(ui.clip_rect().intersect(rect));
    child.spacing_mut().item_spacing.y = 3.0;
    child.add(
        egui::Label::new(RichText::new(title).size(14.0).color(if selected {
            ACCENT
        } else {
            TEXT
        }))
        .truncate(),
    );
    child.add(egui::Label::new(RichText::new(summary).size(12.0).color(MUTED)).truncate());
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), title)
    });
    response
}

#[derive(Clone, Copy)]
enum Icon {
    Up,
    Down,
    Close,
    Play,
    Pause,
}

fn paint_icon(painter: &egui::Painter, rect: egui::Rect, icon: Icon, color: Color32) {
    let c = rect.center();
    let stroke = Stroke::new(1.7_f32, color);
    match icon {
        Icon::Up | Icon::Down => {
            let direction = if matches!(icon, Icon::Up) { -1.0 } else { 1.0 };
            painter.add(egui::Shape::line(
                vec![
                    c + Vec2::new(-4.0, -direction * 2.0),
                    c + Vec2::new(0.0, direction * 2.0),
                    c + Vec2::new(4.0, -direction * 2.0),
                ],
                stroke,
            ));
        }
        Icon::Close => {
            painter.line_segment([c - Vec2::splat(3.5), c + Vec2::splat(3.5)], stroke);
            painter.line_segment([c + Vec2::new(-3.5, 3.5), c + Vec2::new(3.5, -3.5)], stroke);
        }
        Icon::Play => {
            painter.add(egui::Shape::convex_polygon(
                vec![
                    c + Vec2::new(-4.0, -6.0),
                    c + Vec2::new(6.0, 0.0),
                    c + Vec2::new(-4.0, 6.0),
                ],
                color,
                Stroke::NONE,
            ));
        }
        Icon::Pause => {
            for x in [-3.5, 3.5] {
                painter.rect_filled(
                    egui::Rect::from_center_size(c + Vec2::new(x, 0.0), Vec2::new(2.5, 12.0)),
                    0.5,
                    color,
                );
            }
        }
    }
}

fn icon_button(ui: &mut egui::Ui, icon: Icon, hint: &str) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::new(28.0, 24.0), egui::Sense::click());
    let visual = ui.style().interact(&response);
    ui.painter().rect(
        rect,
        visual.corner_radius,
        visual.weak_bg_fill,
        visual.bg_stroke,
        egui::StrokeKind::Inside,
    );
    paint_icon(ui.painter(), rect, icon, TEXT);
    response
        .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), hint));
    response.on_hover_text(hint)
}

fn icon_text_button(ui: &mut egui::Ui, text: &str, icon: Icon) -> egui::Response {
    let response = ui.add(
        egui::Button::new(RichText::new(format!("    {text}")).color(PANEL).strong())
            .fill(ACCENT)
            .min_size(Vec2::new(112.0, 38.0)),
    );
    paint_icon(
        ui.painter(),
        egui::Rect::from_center_size(
            response.rect.left_center() + Vec2::new(18.0, 0.0),
            Vec2::splat(16.0),
        ),
        icon,
        PANEL,
    );
    response
}

fn highlight(text: &str, query: &str) -> egui::text::LayoutJob {
    let normal = egui::TextFormat {
        font_id: FontId::proportional(17.0),
        color: TEXT,
        ..Default::default()
    };
    let mut job = egui::text::LayoutJob::default();
    let query = query.trim().to_lowercase().replace('\u{451}', "\u{435}");
    if query.is_empty() {
        job.append(text, 0.0, normal);
        return job;
    }
    // Keep original UTF-8 boundaries, including letters whose lowercase expands.
    let mut normalized = String::new();
    let mut boundaries = Vec::new();
    for (index, ch) in text.char_indices() {
        for lower in ch.to_lowercase() {
            let lower = if lower == '\u{451}' { '\u{435}' } else { lower };
            for _ in 0..lower.len_utf8() {
                boundaries.push((index, index + ch.len_utf8()));
            }
            normalized.push(lower);
        }
    }
    let mut cursor = 0;
    for (start, matched) in normalized.match_indices(&query) {
        let original_start = boundaries[start].0;
        let original_end = boundaries[start + matched.len() - 1].1;
        if original_start < cursor {
            continue;
        }
        job.append(&text[cursor..original_start], 0.0, normal.clone());
        job.append(
            &text[original_start..original_end],
            0.0,
            egui::TextFormat {
                background: Color32::from_rgb(255, 234, 166),
                color: TEXT,
                ..normal.clone()
            },
        );
        cursor = original_end;
    }
    job.append(&text[cursor..], 0.0, normal);
    job
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn localized_layout_stays_within_panels_at_supported_window_sizes() {
        let directory = tempfile::tempdir().unwrap();
        let storage = Storage::at(directory.path().to_owned()).unwrap();
        Settings {
            check_updates_on_startup: false,
            ..Default::default()
        }
        .save(&storage)
        .unwrap();
        let ctx = egui::Context::default();
        let mut app = App::with_context(&ctx, storage.clone(), None);
        app.model = 0;
        for i in 0..12 {
            let mut document = crate::transcript::tests::fixture();
            document.source = format!(
                "{i}-A very long lecture recording title with spaces and Unicode — résumé.mp4"
            )
            .into();
            app.library.push(document);
        }
        app.source = Some(app.library[0].source.clone());
        app.document = Some(app.library[0].clone());
        for installed in [false, true] {
            if installed {
                for model in MODELS {
                    std::fs::File::create(storage.model_path(model.id))
                        .unwrap()
                        .set_len(model.size)
                        .unwrap();
                }
            }
            for locale in LOCALES {
                app.locale = locale;
                for (width, height, model_index) in
                    [(960.0, 668.0), (1180.0, 768.0), (1440.0, 1000.0)]
                        .into_iter()
                        .flat_map(|(width, height)| {
                            (0..MODELS.len()).map(move |index| (width, height, index))
                        })
                {
                    app.model = model_index;
                    let mut sidebar_bounds = None;
                    let mut content_bounds = None;
                    for _ in 0..2 {
                        let _ = ctx.run(
                            egui::RawInput {
                                screen_rect: Some(egui::Rect::from_min_size(
                                    egui::Pos2::ZERO,
                                    Vec2::new(width, height),
                                )),
                                ..Default::default()
                            },
                            |ctx| {
                                sidebar_bounds = Some(app.sidebar(ctx));
                                app.transport(ctx);
                                let panel = egui::CentralPanel::default()
                                    .frame(
                                        egui::Frame::new()
                                            .inner_margin(egui::Margin::symmetric(34, 26)),
                                    )
                                    .show(ctx, |ui| {
                                        app.content(ui);
                                        ui.min_rect()
                                    });
                                content_bounds = Some((panel.response.rect, panel.inner));
                            },
                        );
                    }
                    assert!(
                        (sidebar_bounds.unwrap().0.width() - 280.0).abs() < 0.5,
                        "sidebar grew beyond its fixed width"
                    );
                    for (panel, content) in [sidebar_bounds.unwrap(), content_bounds.unwrap()] {
                        assert!(
                            content.min.x >= panel.min.x - 0.5
                                && content.max.x <= panel.max.x + 0.5,
                            "horizontal overflow: {locale:?}, {width}x{height}, installed={installed}, panel={panel:?}, content={content:?}"
                        );
                        assert!(
                            content.min.y >= panel.min.y - 0.5
                                && content.max.y <= panel.max.y + 0.5,
                            "vertical overflow: {locale:?}, {width}x{height}, installed={installed}, panel={panel:?}, content={content:?}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn highlight_preserves_original_unicode_text() {
        for (text, query) in [
            ("Résumé of the LECTURE", "RÉSUMÉ"),
            ("İstanbul", "i"),
            ("中文音频", "文"),
            ("😀 Hello", "HELLO"),
        ] {
            let job = highlight(text, query);
            assert_eq!(job.text, text);
            assert!(
                job.sections
                    .iter()
                    .any(|s| s.format.background != Color32::TRANSPARENT)
            );
        }
    }
}
