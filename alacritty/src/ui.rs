use alacritty_terminal::vte::ansi::NamedColor;
use winit::keyboard::ModifiersState;

use crate::config::font::Font;
use crate::display::SizeInfo;
use crate::display::color::{List, Rgb};
use crate::renderer::rects::RenderRect;

pub fn font(scale_factor: f32) -> Font {
    Font::default().with_size(crossfont::Size::new(11.).scale(scale_factor))
}

pub fn shortcut_pressed(modifiers: ModifiersState) -> bool {
    modifiers.control_key() || (cfg!(target_os = "macos") && modifiers.super_key())
}

pub fn dialog_size(terminal: SizeInfo, cell: (f32, f32), scale_factor: f32) -> SizeInfo {
    let padding = (8. * scale_factor).round();
    SizeInfo::new(terminal.width(), terminal.height(), cell.0, cell.1, padding, padding, false)
}

#[derive(Clone, Copy)]
pub struct Palette {
    pub panel: Rgb,
    pub surface: Rgb,
    pub input: Rgb,
    pub text: Rgb,
    pub muted: Rgb,
    pub border: Rgb,
    pub accent: Rgb,
    pub on_accent: Rgb,
    pub selected: Rgb,
    pub hover: Rgb,
    pub disabled: Rgb,
    pub danger: Rgb,
    pub danger_hover: Rgb,
}

impl Palette {
    pub fn new(colors: &List) -> Self {
        let background = colors[NamedColor::Background];
        let foreground = colors[NamedColor::Foreground];
        let accent = colors[NamedColor::Blue];
        let danger = colors[NamedColor::Red];
        let mix = |base: Rgb, tint: Rgb, amount: f32| {
            let channel =
                |a: u8, b: u8| (f32::from(a) * (1. - amount) + f32::from(b) * amount).round() as u8;
            Rgb::new(channel(base.r, tint.r), channel(base.g, tint.g), channel(base.b, tint.b))
        };
        let linear = |channel: u8| {
            let value = f32::from(channel) / 255.;
            if value <= 0.04045 { value / 12.92 } else { ((value + 0.055) / 1.055).powf(2.4) }
        };
        let luminance =
            linear(accent.r) * 0.2126 + linear(accent.g) * 0.7152 + linear(accent.b) * 0.0722;
        let panel = mix(background, foreground, 0.035);
        Self {
            panel,
            surface: mix(background, foreground, 0.075),
            input: background,
            text: foreground,
            muted: mix(panel, foreground, 0.65),
            border: mix(panel, foreground, 0.22),
            accent,
            on_accent: if luminance > 0.179 { Rgb::new(0, 0, 0) } else { Rgb::new(255, 255, 255) },
            selected: mix(panel, accent, 0.16),
            hover: mix(panel, accent, 0.1),
            disabled: mix(panel, foreground, 0.4),
            danger,
            danger_hover: mix(panel, danger, 0.15),
        }
    }
}

pub fn agent_color(state: crate::agent::AgentState, colors: &List, frame: usize) -> Rgb {
    use crate::agent::AgentState;
    let palette = Palette::new(colors);
    let color = match state {
        AgentState::Input | AgentState::Permission => colors[NamedColor::Yellow],
        AgentState::Working => palette.accent,
        AgentState::Error => palette.danger,
        AgentState::Done => colors[NamedColor::Green],
        AgentState::Ready | AgentState::Unknown => palette.muted,
    };
    if !matches!(state, AgentState::Input | AgentState::Permission) {
        return color;
    }
    let amount = [0.55, 0.7, 0.85, 1., 1., 0.85, 0.7, 0.55][frame % 8];
    let channel = |base: u8, target: u8| {
        (f32::from(base) * (1. - amount) + f32::from(target) * amount).round() as u8
    };
    Rgb::new(
        channel(palette.panel.r, color.r),
        channel(palette.panel.g, color.g),
        channel(palette.panel.b, color.b),
    )
}

pub fn rounded(rect: RenderRect, radius: f32) -> Vec<RenderRect> {
    if rect.width <= 0. || rect.height <= 0. {
        return Vec::new();
    }
    let radius = radius.min(rect.width / 2.).min(rect.height / 2.).max(0.);
    if radius < 1. {
        return vec![rect];
    }
    let bands = radius.ceil() as usize;
    let step = radius / bands as f32;
    let mut result = Vec::with_capacity(bands * 2 + 1);
    if rect.height > radius * 2. {
        result.push(RenderRect { y: rect.y + radius, height: rect.height - radius * 2., ..rect });
    }
    for band in 0..bands {
        let distance = radius - (band as f32 + 0.5) * step;
        let inset = radius - (radius * radius - distance * distance).max(0.).sqrt();
        for y in [rect.y + band as f32 * step, rect.y + rect.height - (band + 1) as f32 * step] {
            result.push(RenderRect {
                x: rect.x + inset,
                y,
                width: (rect.width - inset * 2.).max(0.),
                height: step,
                ..rect
            });
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::color::Colors;
    use alacritty_terminal::grid::Dimensions;

    #[test]
    fn dialog_geometry_ignores_terminal_font_and_spacing() {
        let small = SizeInfo::new(900., 700., 8., 16., 0., 16., false);
        let large = SizeInfo::new(900., 700., 28., 55., 40., 80., true);
        let left = dialog_size(small, (8., 17.), 1.);
        let right = dialog_size(large, (8., 17.), 1.);
        assert_eq!(left.columns(), right.columns());
        assert_eq!(left.screen_lines(), right.screen_lines());
        assert_eq!(left.padding_x(), right.padding_x());
        assert_eq!(left.padding_y(), right.padding_y());
        assert_eq!(left.cell_width(), right.cell_width());
        assert_eq!(left.cell_height(), right.cell_height());
    }

    #[test]
    fn dialog_font_and_layout_follow_display_dpi() {
        let normal = dialog_size(SizeInfo::new(900., 700., 8., 16., 0., 0., false), (8., 17.), 1.);
        let retina =
            dialog_size(SizeInfo::new(1800., 1400., 8., 16., 0., 0., false), (16., 34.), 2.);
        assert_eq!(normal.columns(), retina.columns());
        assert_eq!(normal.screen_lines(), retina.screen_lines());
        assert_eq!(font(2.).size(), font(1.).size().scale(2.));
    }

    #[test]
    fn waiting_agents_pulse_while_finished_agents_remain_still() {
        use crate::agent::AgentState;
        let colors = List::from(&Colors::default());
        for state in [AgentState::Input, AgentState::Permission] {
            assert!(state.animated());
            assert_eq!(state.animated_marker(0), "?");
            assert_ne!(agent_color(state, &colors, 0), agent_color(state, &colors, 3));
        }
        assert_ne!(AgentState::Working.animated_marker(0), AgentState::Working.animated_marker(2));
        for state in [AgentState::Ready, AgentState::Unknown, AgentState::Done, AgentState::Error] {
            assert!(!state.animated());
            assert_eq!(state.animated_marker(0), state.animated_marker(4));
            assert_eq!(agent_color(state, &colors, 0), agent_color(state, &colors, 3));
        }
    }

    #[test]
    fn custom_terminal_colors_reach_all_ui_surfaces() {
        let mut colors = Colors::default();
        colors.primary.background = Rgb::new(30, 25, 20);
        colors.primary.foreground = Rgb::new(240, 225, 200);
        colors.normal.blue = Rgb::new(50, 160, 90);
        colors.normal.red = Rgb::new(200, 80, 60);
        let palette = Palette::new(&List::from(&colors));
        assert_eq!(palette.input, colors.primary.background);
        assert_eq!(palette.text, colors.primary.foreground);
        assert_eq!(palette.accent, colors.normal.blue);
        assert_eq!(palette.danger, colors.normal.red);
        assert_ne!(palette.selected, palette.panel);
        assert_ne!(palette.hover, palette.panel);
        colors.primary.background = Rgb::new(250, 240, 220);
        colors.primary.foreground = Rgb::new(40, 30, 20);
        let light = Palette::new(&List::from(&colors));
        assert_ne!(light.panel, palette.panel);
        assert_ne!(light.selected, palette.selected);
        assert_eq!(light.text, colors.primary.foreground);
        assert_eq!(light.accent, palette.accent);
    }

    #[test]
    fn text_on_light_and_dark_accents_keeps_contrast() {
        let mut colors = Colors::default();
        for accent in [Rgb::new(255, 255, 255), Rgb::new(255, 220, 100), Rgb::new(90, 210, 140)] {
            colors.normal.blue = accent;
            assert_eq!(Palette::new(&List::from(&colors)).on_accent, Rgb::new(0, 0, 0));
        }
        for accent in [Rgb::new(0, 0, 0), Rgb::new(10, 40, 120), Rgb::new(100, 20, 50)] {
            colors.normal.blue = accent;
            assert_eq!(Palette::new(&List::from(&colors)).on_accent, Rgb::new(255, 255, 255));
        }
    }
}
