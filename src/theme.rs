//! Persisted application appearance and shared colors for custom-painted controls.
use egui::{Color32, FontId, Stroke, Vec2};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Theme {
    #[default]
    Light,
    Dark,
}

#[derive(Clone, Copy)]
pub(crate) struct Palette {
    pub bg: Color32,
    pub panel: Color32,
    pub border: Color32,
    pub muted: Color32,
    pub accent: Color32,
    pub text: Color32,
    pub soft: Color32,
    pub active: Color32,
    pub shadow: Color32,
    pub error: Color32,
    pub error_bg: Color32,
    pub highlight: Color32,
    pub highlight_text: Color32,
    pub on_accent: Color32,
}

impl Theme {
    pub fn label(self) -> &'static str {
        match self {
            Self::Light => "Light",
            Self::Dark => "Dark",
        }
    }

    pub(crate) fn palette(self) -> Palette {
        match self {
            Self::Light => Palette {
                bg: Color32::from_rgb(247, 248, 252),
                panel: Color32::WHITE,
                border: Color32::from_rgb(224, 228, 239),
                muted: Color32::from_rgb(91, 102, 125),
                accent: Color32::from_rgb(91, 78, 232),
                text: Color32::from_rgb(28, 32, 51),
                soft: Color32::from_rgb(238, 236, 255),
                active: Color32::from_rgb(235, 230, 255),
                shadow: Color32::from_rgb(235, 236, 245),
                error: Color32::from_rgb(175, 55, 55),
                error_bg: Color32::from_rgb(255, 235, 231),
                highlight: Color32::from_rgb(255, 234, 166),
                highlight_text: Color32::from_rgb(28, 32, 51),
                on_accent: Color32::WHITE,
            },
            Self::Dark => Palette {
                bg: Color32::from_rgb(21, 24, 34),
                panel: Color32::from_rgb(29, 33, 46),
                border: Color32::from_rgb(64, 71, 91),
                muted: Color32::from_rgb(174, 184, 207),
                accent: Color32::from_rgb(185, 175, 255),
                text: Color32::from_rgb(239, 242, 251),
                soft: Color32::from_rgb(56, 49, 88),
                active: Color32::from_rgb(73, 63, 112),
                shadow: Color32::from_rgb(12, 14, 22),
                error: Color32::from_rgb(255, 164, 156),
                error_bg: Color32::from_rgb(75, 37, 42),
                highlight: Color32::from_rgb(111, 84, 30),
                highlight_text: Color32::from_rgb(255, 245, 210),
                on_accent: Color32::from_rgb(26, 21, 47),
            },
        }
    }

    pub(crate) fn from_ui(ui: &egui::Ui) -> Self {
        if ui.visuals().dark_mode {
            Self::Dark
        } else {
            Self::Light
        }
    }

    /// Apply only when the saved choice changes, never from an idle timer.
    pub(crate) fn apply(self, ctx: &egui::Context) {
        let colors = self.palette();
        ctx.send_viewport_cmd(egui::ViewportCommand::SetTheme(match self {
            Self::Light => egui::SystemTheme::Light,
            Self::Dark => egui::SystemTheme::Dark,
        }));
        ctx.set_theme(match self {
            Self::Light => egui::ThemePreference::Light,
            Self::Dark => egui::ThemePreference::Dark,
        });
        let mut style = (*ctx.style()).clone();
        style.visuals = match self {
            Self::Light => egui::Visuals::light(),
            Self::Dark => egui::Visuals::dark(),
        };
        style.visuals.panel_fill = colors.bg;
        style.visuals.window_fill = colors.panel;
        style.visuals.extreme_bg_color = colors.bg;
        style.visuals.faint_bg_color = colors.panel;
        style.visuals.override_text_color = Some(colors.text);
        style.visuals.selection.bg_fill = colors.soft;
        style.visuals.selection.stroke = Stroke::new(1.0, colors.accent);
        style.visuals.hyperlink_color = colors.accent;
        style.visuals.error_fg_color = colors.error;
        style.visuals.window_stroke = Stroke::new(1.0, colors.border);
        style.visuals.window_corner_radius = egui::CornerRadius::same(14);
        style.visuals.widgets.inactive.bg_fill = colors.panel;
        style.visuals.widgets.inactive.weak_bg_fill = colors.panel;
        style.visuals.widgets.inactive.bg_stroke = Stroke::new(1.0, colors.border);
        style.visuals.widgets.inactive.corner_radius = egui::CornerRadius::same(8);
        style.visuals.widgets.inactive.fg_stroke = Stroke::new(1.0, colors.muted);
        style.visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0, colors.border);
        style.visuals.widgets.inactive.expansion = 0.0;
        style.visuals.widgets.hovered.bg_fill = colors.soft;
        style.visuals.widgets.hovered.weak_bg_fill = colors.soft;
        style.visuals.widgets.hovered.bg_stroke = Stroke::new(1.0, colors.accent);
        style.visuals.widgets.hovered.corner_radius = egui::CornerRadius::same(8);
        style.visuals.widgets.hovered.fg_stroke = Stroke::new(1.0, colors.accent);
        style.visuals.widgets.hovered.expansion = 0.0;
        style.visuals.widgets.active.bg_fill = colors.active;
        style.visuals.widgets.active.weak_bg_fill = colors.active;
        style.visuals.widgets.active.corner_radius = egui::CornerRadius::same(8);
        style.visuals.widgets.active.fg_stroke = Stroke::new(1.0, colors.accent);
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
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn contrast(a: Color32, b: Color32) -> f32 {
        fn luminance(color: Color32) -> f32 {
            let channel = |value: u8| {
                let value = f32::from(value) / 255.0;
                if value <= 0.04045 {
                    value / 12.92
                } else {
                    ((value + 0.055) / 1.055).powf(2.4)
                }
            };
            0.2126 * channel(color.r()) + 0.7152 * channel(color.g()) + 0.0722 * channel(color.b())
        }
        let a = luminance(a);
        let b = luminance(b);
        (a.max(b) + 0.05) / (a.min(b) + 0.05)
    }

    #[test]
    fn both_palettes_keep_labels_actions_and_search_highlights_readable() {
        for theme in [Theme::Light, Theme::Dark] {
            let colors = theme.palette();
            for surface in [colors.bg, colors.panel, colors.soft, colors.active] {
                for foreground in [colors.text, colors.muted, colors.accent] {
                    assert!(
                        contrast(foreground, surface) >= 4.5,
                        "{theme:?}: {foreground:?} on {surface:?}"
                    );
                }
            }
            assert!(contrast(colors.on_accent, colors.accent) >= 4.5);
            assert!(contrast(colors.highlight_text, colors.highlight) >= 4.5);
            assert!(contrast(colors.error, colors.error_bg) >= 4.5);
        }
    }

    #[test]
    fn switching_themes_updates_framework_widgets_and_custom_palette_together() {
        let context = egui::Context::default();
        for theme in [Theme::Dark, Theme::Light, Theme::Dark] {
            theme.apply(&context);
            assert_eq!(context.style().visuals.dark_mode, theme == Theme::Dark);
            assert_eq!(context.style().visuals.panel_fill, theme.palette().bg);
            assert_eq!(
                context.style().visuals.override_text_color,
                Some(theme.palette().text)
            );
        }
    }
}
