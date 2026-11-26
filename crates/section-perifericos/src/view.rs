use hub_core::leds::{Device, DeviceKind, Snapshot};
use hub_ui_kit::Icon;
use hub_ui_kit::color_picker;
use hub_ui_kit::iced::widget::{
    Column, Row, button, column, container, row, scrollable, slider, space, text,
};
use hub_ui_kit::iced::{Alignment, Element, Length, Padding};
use hub_ui_kit::style;
use hub_ui_kit::tokens::*;
use hub_ui_kit::widgets::*;

use crate::{Focus, Message, Peripherals, Status, SubTab};

pub const NONE_LABEL: &str = "Nenhum";
const CARD_HEIGHT: f32 = 64.0;
const LIST_WIDTH: f32 = 290.0;
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
    } else {
        actions = actions.push(primary_button(
            if section.installing() {
                "Instalando"
            } else {
                "Instalar OpenRGB"
            },
            (!section.installing()).then_some(Message::InstallOpenRgb),
        ));
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

fn swatch_row<'a>(section: &Peripherals, focus: &Focus) -> Element<'a, Message> {
    let focus = focus.clone();
    color_picker::swatches(
        &color_picker::SWATCHES,
        match &focus {
            Focus::All => section.all_color(),
            Focus::Device(name) => section.device_color(name),
        },
        move |color| Message::Swatch(focus.clone(), color),
    )
}

fn kind_icon(kind: DeviceKind) -> Icon {
    match kind {
        DeviceKind::Keyboard | DeviceKind::Keypad => Icon::Keyboard,
        DeviceKind::Mouse | DeviceKind::Mousemat => Icon::Mouse,
        DeviceKind::Motherboard | DeviceKind::Case | DeviceKind::Laptop => Icon::Board,
        DeviceKind::Memory | DeviceKind::Storage => Icon::Memory,
        DeviceKind::Gpu => Icon::Gpu,
        DeviceKind::Cooler => Icon::Fan,
        DeviceKind::LedStrip | DeviceKind::Light => Icon::Light,
        DeviceKind::Headset
        | DeviceKind::HeadsetStand
        | DeviceKind::Microphone
        | DeviceKind::Speaker => Icon::Headphones,
        DeviceKind::Gamepad => Icon::Game,
        DeviceKind::Monitor => Icon::Monitor,
        _ => Icon::Peripherals,
    }
}

fn card<'a>(
    section: &Peripherals,
    focus: Focus,
    glyph: Icon,
    name: &'a str,
    detail: String,
) -> Element<'a, Message> {
    let selected = section.focus() == Some(&focus);
    let tint = if selected { accent() } else { text_secondary() };
    let color = match &focus {
        Focus::All => section.all_color(),
        Focus::Device(name) => section.device_color(name),
    };
    button(
        row![
            container(icon(glyph, ICON_SIZE + 2.0, tint))
                .padding(units(2))
                .style(style::badge(tint)),
            column![
                container(body(name).wrapping(text::Wrapping::None)).clip(true),
                small(detail)
            ]
            .spacing(2)
            .width(Length::Fill),
            container(space())
                .width(12)
                .height(12)
                .style(style::dot(color)),
        ]
        .spacing(units(3))
        .align_y(Alignment::Center),
    )
    .padding([units(2), units(3)])
    .width(Length::Fill)
    .height(CARD_HEIGHT)
    .style(style::selectable(selected))
    .on_press(Message::Focus(focus))
    .into()
}

fn group_detail(devices: &[&Device]) -> String {
    let leds: usize = devices.iter().map(|d| d.led_count).sum();
    match devices {
        [one] => format!("{}, {} LEDs", one.kind.label(), leds),
        [first, ..] => format!(
            "{}, {} unidades, {} LEDs",
            first.kind.label(),
            devices.len(),
            leds
        ),
        [] => String::new(),
    }
}

pub fn groups(snapshot: &Snapshot) -> Vec<Vec<&Device>> {
    let mut groups: Vec<Vec<&Device>> = Vec::new();
    for device in &snapshot.devices {
        match groups.iter_mut().find(|g| g[0].name == device.name) {
            Some(group) => group.push(device),
            None => groups.push(vec![device]),
        }
    }
    groups
}

fn device_list<'a>(section: &'a Peripherals, snapshot: &'a Snapshot) -> Element<'a, Message> {
    let mut cards: Vec<Element<'a, Message>> = vec![card(
        section,
        Focus::All,
        Icon::Peripherals,
        "Todos os dispositivos",
        format!("{} dispositivos", snapshot.devices.len()),
    )];
    cards.extend(groups(snapshot).into_iter().map(|group| {
        let first = group[0];
        card(
            section,
            Focus::Device(first.name.clone()),
            kind_icon(first.kind),
            first.name.as_str(),
            group_detail(&group),
        )
    }));
    cards
        .into_iter()
        .fold(Column::new().spacing(units(2)), |list, card| {
            list.push(card)
        })
        .width(LIST_WIDTH)
        .into()
}

fn controls<'a>(
    section: &'a Peripherals,
    snapshot: &'a Snapshot,
    focus: Focus,
) -> Element<'a, Message> {
    let mut content = Column::new().spacing(units(3));
    let heading_text = match &focus {
        Focus::All => "Todos os dispositivos".to_string(),
        Focus::Device(name) => name.clone(),
    };
    content = content.push(
        row![
            heading(heading_text),
            space().width(Length::Fill),
            ghost_button("Desligar", Some(Message::TurnOff)),
        ]
        .align_y(Alignment::Center),
    );
    content = content
        .push(swatch_row(section, &focus))
        .push(color_picker::view(section.picker(), Message::Picker));
    if let Focus::Device(name) = &focus
        && let Some(device) = snapshot.devices.iter().find(|d| d.name == *name)
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
                    Message::Mode(name.clone(), position),
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
            let inverted = section.speed_inverted(&device.name);
            let speed = mode.clamp_speed(section.device_speed(name).unwrap_or(mode.speed));
            let level = mode.level_of_speed(speed, inverted);
            let scale = mode.clone();
            let speed_name = name.clone();
            content = content.push(
                row![
                    muted("Velocidade").width(90),
                    small("Lento"),
                    slider(0..=mode.speed_steps(), level, move |value| {
                        Message::Speed(speed_name.clone(), scale.speed_from_level(value, inverted))
                    })
                    .on_release(Message::SpeedReleased(name.clone()))
                    .width(220),
                    small("Rápido"),
                    choice(
                        "Inverter",
                        inverted,
                        Message::InvertSpeed(device.name.clone())
                    ),
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
    let mut header =
        row![heading("Perfis"), space().width(Length::Fill)].align_y(Alignment::Center);
    if section.openrgb().is_some() {
        header = header.push(ghost_button("Abrir OpenRGB", Some(Message::ShowOpenRgb)));
    }
    let mut content = Column::new().spacing(units(3)).push(header);
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
                let focus = section.focus().cloned().unwrap_or(Focus::All);
                content = content.push(
                    row![
                        device_list(section, snapshot),
                        container(controls(section, snapshot, focus)).width(Length::Fill),
                    ]
                    .spacing(GAP_PANEL),
                );
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
