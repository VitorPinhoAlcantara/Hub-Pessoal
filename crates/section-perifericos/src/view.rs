use hub_core::leds::{Device, Snapshot};
use hub_ui_kit::Icon;
use hub_ui_kit::color_picker;
use hub_ui_kit::iced::widget::{Column, Row, column, container, row, scrollable, slider, space};
use hub_ui_kit::iced::{Alignment, Element, Length, Padding};
use hub_ui_kit::style;
use hub_ui_kit::tokens::*;
use hub_ui_kit::widgets::*;

use crate::{Focus, Message, Peripherals, Status, SubTab};

pub const NONE_LABEL: &str = "Nenhum";
const CARD_HEIGHT: f32 = 150.0;
const COLUMNS: usize = 3;
const MODES_PER_ROW: usize = 5;

fn sub_tabs(section: &Peripherals) -> Element<'_, Message> {
    SubTab::ALL
        .iter()
        .fold(Row::new().spacing(units(2)), |tabs, tab| {
            tabs.push(tab_button(
                Icon::Peripherals,
                tab.label(),
                section.sub_tab() == *tab,
                Message::SubTab(*tab),
            ))
        })
        .into()
}

fn failure<'a>(section: &'a Peripherals, reason: &'a str) -> Element<'a, Message> {
    let mut actions = Row::new().spacing(units(2));
    if section.openrgb().is_some() {
        actions = actions.push(primary_button("Abrir OpenRGB", Some(Message::OpenOpenRgb)));
    }
    actions = actions.push(ghost_button("Tentar de novo", Some(Message::Retry)));
    container(
        column![
            heading("Não foi possível falar com o OpenRGB"),
            muted(reason),
            body("O OpenRGB não está aberto ou não está instalado. Para controlar os LEDs:"),
            body("1. Instale o OpenRGB em https://openrgb.org ou com o comando winget install OpenRGB.OpenRGB"),
            body("2. Abra o OpenRGB com o parâmetro --server, ou ative o \"SDK Server\" na aba SDK Server das configurações dele e clique em Start Server"),
            body("3. Volte aqui e clique em Tentar de novo"),
            actions,
        ]
        .spacing(units(3)),
    )
    .padding(units(4))
    .width(Length::Fill)
    .style(style::elevated)
    .into()
}

fn swatch_row<'a>(section: &Peripherals, focus: Focus) -> Element<'a, Message> {
    color_picker::swatches(
        &color_picker::SWATCHES,
        match focus {
            Focus::All => section.all_color(),
            Focus::Device(index) => section.device_color(index),
        },
        move |color| Message::Swatch(focus, color),
    )
}

fn card<'a>(
    section: &Peripherals,
    focus: Focus,
    name: &'a str,
    detail: String,
) -> Element<'a, Message> {
    let selected = section.focus() == focus;
    container(
        column![
            heading(name),
            small(detail),
            swatch_row(section, focus),
            choice("Controlar", selected, Message::Focus(focus)),
        ]
        .spacing(units(2))
        .width(Length::Fill),
    )
    .padding(units(3))
    .width(Length::FillPortion(1))
    .height(CARD_HEIGHT)
    .style(if selected {
        style::panel
    } else {
        style::elevated
    })
    .into()
}

fn device_detail(device: &Device) -> String {
    format!("{}, {} LEDs", device.kind.label(), device.led_count)
}

fn grid<'a>(section: &'a Peripherals, snapshot: &'a Snapshot) -> Element<'a, Message> {
    let mut cards: Vec<Element<'a, Message>> = vec![card(
        section,
        Focus::All,
        "Todos os dispositivos",
        format!("{} dispositivos", snapshot.devices.len()),
    )];
    cards.extend(snapshot.devices.iter().map(|device| {
        card(
            section,
            Focus::Device(device.index),
            device.name.as_str(),
            device_detail(device),
        )
    }));
    let mut rows = Column::new().spacing(GAP_PANEL);
    let mut items = cards.into_iter().peekable();
    while items.peek().is_some() {
        let mut line = Row::new().spacing(GAP_PANEL).width(Length::Fill);
        for _ in 0..COLUMNS {
            line = line.push(
                items
                    .next()
                    .unwrap_or_else(|| space().width(Length::FillPortion(1)).into()),
            );
        }
        rows = rows.push(line);
    }
    rows.into()
}

fn controls<'a>(section: &'a Peripherals, snapshot: &'a Snapshot) -> Element<'a, Message> {
    let focus = section.focus();
    let mut content = Column::new().spacing(units(3));
    let heading_text = match focus {
        Focus::All => "Todos os dispositivos".to_string(),
        Focus::Device(index) => snapshot
            .devices
            .iter()
            .find(|d| d.index == index)
            .map_or_else(String::new, |d| d.name.clone()),
    };
    content = content.push(heading(heading_text));
    content = content.push(color_picker::view(section.picker(), Message::Picker));
    if let Focus::Device(index) = focus
        && let Some(device) = snapshot.devices.iter().find(|d| d.index == index)
    {
        content = content.push(small("Efeitos"));
        let modes: Vec<Element<'a, Message>> = device
            .modes
            .iter()
            .enumerate()
            .map(|(position, mode)| {
                choice(
                    mode.name.as_str(),
                    position == device.active_mode,
                    Message::Mode(index, position),
                )
            })
            .collect();
        let mut lines = Column::new().spacing(units(2));
        let mut items = modes.into_iter().peekable();
        while items.peek().is_some() {
            let line: Vec<Element<'a, Message>> = items.by_ref().take(MODES_PER_ROW).collect();
            lines = lines.push(Row::with_children(line).spacing(units(2)));
        }
        content = content.push(lines);
        if let Some(mode) = device.active().filter(|m| m.has_speed()) {
            let speed = section
                .device_speed(index)
                .unwrap_or(mode.speed)
                .clamp(mode.speed_min, mode.speed_max);
            content = content.push(
                row![
                    muted("Velocidade").width(90),
                    slider(mode.speed_min..=mode.speed_max, speed, move |value| {
                        Message::Speed(index, value)
                    })
                    .on_release(Message::SpeedReleased(index))
                    .width(260),
                    body(speed.to_string()),
                ]
                .spacing(units(3))
                .align_y(Alignment::Center),
            );
        }
    }
    container(content)
        .padding(units(3))
        .width(Length::Fill)
        .style(style::elevated)
        .into()
}

fn profile_choices<'a>(
    profiles: &'a [String],
    current: &str,
    on_pick: impl Fn(String) -> Message,
) -> Element<'a, Message> {
    let mut line = Row::new().spacing(units(2));
    line = line.push(choice(
        NONE_LABEL,
        current.is_empty(),
        on_pick(NONE_LABEL.to_string()),
    ));
    for profile in profiles {
        line = line.push(choice(
            profile.as_str(),
            profile == current,
            on_pick(profile.clone()),
        ));
    }
    line.wrap().into()
}

fn profiles<'a>(section: &'a Peripherals, snapshot: &'a Snapshot) -> Element<'a, Message> {
    let config = section.leds_config();
    let mut content = Column::new().spacing(units(3)).push(heading("Perfis"));
    if snapshot.profiles.is_empty() {
        content = content.push(muted("Nenhum perfil salvo no OpenRGB"));
    } else {
        let list = snapshot
            .profiles
            .iter()
            .fold(Row::new().spacing(units(2)), |list, name| {
                list.push(ghost_button(
                    name.as_str(),
                    Some(Message::Profile(name.clone())),
                ))
            });
        content = content.push(list.wrap());
    }
    content = content
        .push(labeled(
            "Ao ligar o modo jogo, aplicar o perfil",
            profile_choices(
                &snapshot.profiles,
                &config.game_profile,
                Message::GameProfile,
            ),
        ))
        .push(labeled(
            "Ao desligar, aplicar o perfil",
            profile_choices(
                &snapshot.profiles,
                &config.normal_profile,
                Message::NormalProfile,
            ),
        ));
    container(content)
        .padding(units(3))
        .width(Length::Fill)
        .style(style::elevated)
        .into()
}

pub fn view(section: &Peripherals) -> Element<'_, Message> {
    let mut content = Column::new()
        .spacing(units(3))
        .push(title("Periféricos"))
        .push(sub_tabs(section));
    if let Some(notice) = section.notice() {
        content = content.push(muted(notice));
    }
    match section.status() {
        Status::Loading => content = content.push(muted("Conectando ao OpenRGB")),
        Status::Failed(reason) => content = content.push(failure(section, reason)),
        Status::Ready(snapshot) => {
            if snapshot.devices.is_empty() {
                content = content.push(muted("O OpenRGB não encontrou nenhum dispositivo"));
            } else {
                content = content
                    .push(grid(section, snapshot))
                    .push(controls(section, snapshot));
            }
            content = content.push(profiles(section, snapshot));
        }
    }
    panel(
        scrollable(container(content).padding(Padding::default().right(units(2))))
            .width(Length::Fill)
            .style(style::scroll),
    )
    .into()
}
