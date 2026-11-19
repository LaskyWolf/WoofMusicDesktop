//! The Equalizer page (Swift `EqualizerView`): on/off, a slider per band
//! from −12 to +12 dB, and the presets. Settings → Equalizer opens it.

use iced::font::Weight;
use iced::widget::{Space, button, column, container, row, svg, text, vertical_slider};
use iced::{Alignment, Background, Border, Element, Length};
use trop::audio::equalizer::{CUSTOM, FREQUENCIES, MAX_GAIN, MIN_GAIN, PRESETS};
use trop::audio::now_playing::PlayerDefaults;

use crate::settings::{caption, group, setting, toggle};
use crate::{Message, theme, ui};

#[derive(Clone, Debug)]
pub enum Msg {
    Enabled(bool),
    /// A band's slider moved: the page shows it, the sound changes on release.
    Band(usize, f64),
    BandReleased,
    Preset(&'static str),
}

/// Applies `msg` to the saved settings. True when the sound should change
/// now (and the settings be saved).
pub fn update(defaults: &mut PlayerDefaults, msg: Msg) -> bool {
    match msg {
        Msg::Enabled(on) => defaults.equalizer_enabled = on,
        Msg::Band(index, gain) => {
            defaults.equalizer_gains.resize(FREQUENCIES.len(), 0.0);
            defaults.equalizer_gains[index] = gain;
            defaults.equalizer_preset = CUSTOM.into();
            return false;
        }
        Msg::BandReleased => {}
        Msg::Preset(id) => {
            let Some(preset) = PRESETS.iter().find(|preset| preset.id == id) else {
                return false;
            };
            defaults.equalizer_gains = preset.gains.to_vec();
            defaults.equalizer_preset = id.into();
        }
    }
    true
}

pub fn view(defaults: &PlayerDefaults) -> Element<'_, Message> {
    let on = defaults.equalizer_enabled;
    let switch = setting(
        if on { "Equalizer On" } else { "Equalizer Off" },
        caption("Changes apply when you let go of a band."),
        Some(toggle(on, |on| Message::Equalizer(Msg::Enabled(on)))),
    );
    let bands = row(FREQUENCIES.iter().enumerate().map(|(index, frequency)| {
        let gain = defaults.equalizer_gains.get(index).copied().unwrap_or(0.0);
        band(index, *frequency, gain, on)
    }))
    .spacing(4)
    .padding([18, 12]);
    let scale = row![
        caption(format!("{} dB", MIN_GAIN as i64)),
        Space::new().width(Length::Fill),
        caption("0 dB"),
        Space::new().width(Length::Fill),
        caption(format!("+{} dB", MAX_GAIN as i64)),
    ]
    .padding([0, 18]);
    let mut presets: Vec<Element<'_, Message>> = Vec::new();
    if defaults.equalizer_preset == CUSTOM {
        presets.push(preset_row("Custom", true, None));
    }
    presets.extend(PRESETS.iter().map(|preset| {
        preset_row(
            preset.name,
            defaults.equalizer_preset == preset.id,
            Some(Message::Equalizer(Msg::Preset(preset.id))),
        )
    }));
    let page = column![
        text("Equalizer").size(32).font(theme::weight(Weight::Bold)),
        Space::new().height(20),
        group(vec![switch, bands.into()]),
        Space::new().height(6),
        scale,
        Space::new().height(28),
        ui::section_title("Presets"),
        Space::new().height(14),
        group(presets),
    ]
    .max_width(720);
    ui::scroll_page(vec![page.into()])
}

/// One band: its gain on top, the slider, its frequency below.
fn band<'a>(index: usize, frequency: f64, gain: f64, on: bool) -> Element<'a, Message> {
    let label = if frequency >= 1000.0 {
        format!("{}k", frequency / 1000.0)
    } else {
        format!("{frequency}")
    };
    let gain_text = match gain {
        g if g > 0.0 => format!("+{g:.0}"),
        g => format!("{g:.0}"),
    };
    column![
        text(gain_text)
            .size(12)
            .color(if on { theme::TEXT } else { theme::SECONDARY }),
        vertical_slider(MIN_GAIN..=MAX_GAIN, gain, move |gain| {
            Message::Equalizer(Msg::Band(index, gain))
        })
        .on_release(Message::Equalizer(Msg::BandReleased))
        .step(0.5)
        .height(180)
        .style(crate::seek_style),
        text(label).size(12).color(theme::SECONDARY),
    ]
    .spacing(8)
    .align_x(Alignment::Center)
    .width(Length::Fill)
    .into()
}

fn preset_row<'a>(name: &'a str, selected: bool, message: Option<Message>) -> Element<'a, Message> {
    let check: Element<'a, Message> = if selected {
        ui::icon(crate::song_menu::icons::CHECK, 18.0)
            .style(|_, _| svg::Style {
                color: Some(theme::accent()),
            })
            .into()
    } else {
        Space::new().width(18).into()
    };
    let content = row![text(name).size(15).width(Length::Fill), check]
        .align_y(Alignment::Center)
        .padding([13, 18]);
    button(container(content))
        .padding(0)
        .width(Length::Fill)
        .on_press_maybe(message)
        .style(|_, status| button::Style {
            background: (status == button::Status::Hovered)
                .then_some(Background::Color(theme::HOVER)),
            border: Border::default().rounded(16),
            text_color: theme::TEXT,
            ..button::Style::default()
        })
        .into()
}
