use hub_core::platform::{MediaCommand, Os};
use hub_ui_kit::iced::widget::{
    Column, button, column, container, row, scrollable, slider, space, text,
};
use hub_ui_kit::iced::{Alignment, Element, Length, Padding};
use hub_ui_kit::tokens::*;
use hub_ui_kit::widgets::*;
use hub_ui_kit::{Icon, style};

use crate::mpv::format_time;
use crate::{Message, Player};

pub const COLLAPSED_WIDTH: f32 = 340.0;
pub const EXPANDED_WIDTH: f32 = 400.0;

fn control<'a>(glyph: Icon, message: Option<Message>) -> Element<'a, Message> {
    icon_button(glyph, TEXT_PRIMARY, message).into()
}

fn play_button<'a>(paused: bool, message: Option<Message>) -> Element<'a, Message> {
    button(
        container(icon(
            if paused { Icon::Play } else { Icon::Pause },
            ICON_SIZE - 2.0,
            BG_BASE,
        ))
        .center_x(units(8))
        .center_y(units(8)),
    )
    .padding(0)
    .style(style::primary)
    .on_press_maybe(message)
    .into()
}

fn single_line<'a>(
    value: String,
    size: f32,
    color: hub_ui_kit::iced::Color,
) -> Element<'a, Message> {
    container(
        text(value)
            .size(size)
            .color(color)
            .wrapping(text::Wrapping::None),
    )
    .clip(true)
    .width(Length::Fill)
    .into()
}

fn now_playing(player: &Player) -> (String, String, bool, bool) {
    if player.is_active() {
        let status = player.status();
        let title = status
            .title
            .clone()
            .unwrap_or_else(|| "Carregando".to_string());
        let subtitle = match (status.playlist_pos, status.playlist.len()) {
            (Some(pos), total) if total > 1 => format!("Faixa {} de {total}", pos + 1),
            _ => "YouTube".to_string(),
        };
        (title, subtitle, status.paused, true)
    } else if let Some(media) = player.system() {
        (
            media.title.clone(),
            media
                .artist
                .clone()
                .unwrap_or_else(|| "Mídia do sistema".to_string()),
            !media.playing,
            false,
        )
    } else {
        (
            "Nada tocando".to_string(),
            "Cole um link do YouTube".to_string(),
            true,
            false,
        )
    }
}

fn controls(player: &Player, compact: bool) -> Element<'_, Message> {
    let (_, _, paused, mpv) = now_playing(player);
    let has_system = player.system().is_some();
    let (previous, toggle, next) = if mpv {
        (
            Some(Message::Previous),
            Some(Message::TogglePause),
            Some(Message::Next),
        )
    } else if has_system {
        (
            Some(Message::SystemCommand(MediaCommand::Previous)),
            Some(Message::SystemCommand(MediaCommand::PlayPause)),
            Some(Message::SystemCommand(MediaCommand::Next)),
        )
    } else {
        (None, None, None)
    };
    let mut row = row![
        control(Icon::Previous, previous),
        play_button(paused, toggle),
        control(Icon::Next, next),
    ]
    .spacing(units(1))
    .align_y(Alignment::Center);
    if compact {
        row = row.push(control(Icon::ChevronUp, Some(Message::ToggleExpanded)));
    }
    row.into()
}

fn install_hint(os: Os) -> &'static str {
    match os {
        Os::Windows => {
            "Instale com: winget install mpv e winget install yt-dlp (ou scoop install mpv yt-dlp)"
        }
        Os::Linux => {
            "Instale com o gerenciador da distribuição, por exemplo: sudo apt install mpv yt-dlp"
        }
    }
}

fn collapsed(player: &Player) -> Element<'_, Message> {
    let (title, subtitle, _, _) = now_playing(player);
    row![
        icon(Icon::Player, ICON_SIZE, ACCENT),
        column![
            single_line(title, FONT_SIZE, TEXT_PRIMARY),
            single_line(subtitle, FONT_SIZE_SMALL, TEXT_SECONDARY),
        ]
        .spacing(2)
        .width(Length::Fill),
        controls(player, true),
    ]
    .spacing(units(3))
    .align_y(Alignment::Center)
    .width(COLLAPSED_WIDTH)
    .into()
}

fn expanded(player: &Player) -> Element<'_, Message> {
    let (title, subtitle, _, mpv) = now_playing(player);
    let mut col = Column::new().spacing(units(3)).width(EXPANDED_WIDTH);

    let mut header = row![
        icon(Icon::Player, ICON_SIZE, ACCENT),
        column![
            single_line(title, FONT_SIZE_HEADING, TEXT_PRIMARY),
            single_line(subtitle, FONT_SIZE_SMALL, TEXT_SECONDARY),
        ]
        .spacing(2)
        .width(Length::Fill),
    ]
    .spacing(units(3))
    .align_y(Alignment::Center);
    if mpv {
        header = header.push(control(Icon::Stop, Some(Message::Stop)));
    }
    header = header.push(control(Icon::ChevronDown, Some(Message::ToggleExpanded)));
    col = col.push(header);

    if mpv {
        let status = player.status();
        let duration = status.duration.unwrap_or_default().max(1.0);
        let position = player.position().min(duration);
        col = col.push(
            column![
                slider(0.0..=duration, position, Message::SeekPreview)
                    .on_release(Message::SeekCommit)
                    .step(1.0),
                row![
                    small(format_time(position)),
                    space().width(Length::Fill),
                    small(format_time(status.duration.unwrap_or_default())),
                ],
            ]
            .spacing(units(1)),
        );
    }

    col = col.push(
        row![
            controls(player, false),
            space().width(Length::Fill),
            icon(Icon::Volume, ICON_SIZE - 4.0, TEXT_SECONDARY),
            slider(0.0..=100.0, player.status().volume, Message::VolumeChanged)
                .step(1.0)
                .width(110),
        ]
        .spacing(units(2))
        .align_y(Alignment::Center),
    );

    match player.tools() {
        Some(tools) if !tools.ready() => {
            let missing = match (tools.mpv.is_some(), tools.ytdlp.is_some()) {
                (false, false) => "mpv e yt-dlp não encontrados",
                (false, true) => "mpv não encontrado",
                _ => "yt-dlp não encontrado",
            };
            col = col.push(
                container(
                    column![
                        row![
                            icon(Icon::Warning, ICON_SIZE - 4.0, WARNING),
                            body(missing).color(WARNING)
                        ]
                        .spacing(units(2)),
                        small(install_hint(player.os())),
                    ]
                    .spacing(units(1)),
                )
                .padding(units(3))
                .width(Length::Fill)
                .style(style::outlined(WARNING, false)),
            );
        }
        _ => {
            col = col.push(
                row![
                    text_field(
                        "Link de vídeo ou playlist do YouTube",
                        player.url(),
                        Message::UrlChanged
                    )
                    .on_submit(Message::PlayInput),
                    button(icon(Icon::Play, ICON_SIZE - 4.0, BG_BASE))
                        .padding(units(2))
                        .style(style::primary)
                        .on_press_maybe(
                            (!player.url().trim().is_empty()).then_some(Message::PlayInput)
                        ),
                ]
                .spacing(units(2))
                .align_y(Alignment::Center),
            );
        }
    }

    for (message, color) in [(player.error(), DANGER), (player.notice(), TEXT_SECONDARY)] {
        if let Some(message) = message {
            col = col.push(
                row![
                    small(message).color(color).width(Length::Fill),
                    icon_button(Icon::Close, TEXT_SECONDARY, Some(Message::ErrorDismissed)),
                ]
                .align_y(Alignment::Center),
            );
        }
    }

    let status = player.status();
    if mpv && status.playlist.len() > 1 {
        let items = status.playlist.iter().enumerate().fold(
            Column::new().spacing(2),
            |col, (index, entry)| {
                let current = status.playlist_pos == Some(index);
                col.push(
                    button(
                        row![
                            small(format!("{}", index + 1)).width(24),
                            single_line(
                                entry.label().to_string(),
                                FONT_SIZE_SMALL + 1.0,
                                if current { ACCENT } else { TEXT_PRIMARY },
                            ),
                        ]
                        .align_y(Alignment::Center),
                    )
                    .padding([units(1), units(2)])
                    .width(Length::Fill)
                    .style(style::toggle(current))
                    .on_press(Message::PlayIndex(index)),
                )
            },
        );
        col = col.push(labeled(
            "Fila",
            scrollable(container(items).padding(Padding::default().right(units(2))))
                .height(160)
                .style(style::scroll),
        ));
    }

    if !player.favorites().is_empty() {
        let favorites =
            player
                .favorites()
                .iter()
                .fold(Column::new().spacing(2), |col, favorite| {
                    col.push(
                        row![
                            button(
                                row![
                                    icon(
                                        if favorite.is_playlist() {
                                            Icon::List
                                        } else {
                                            Icon::Star
                                        },
                                        ICON_SIZE - 6.0,
                                        WARNING
                                    ),
                                    single_line(
                                        favorite.label().to_string(),
                                        FONT_SIZE_SMALL + 1.0,
                                        TEXT_PRIMARY
                                    ),
                                ]
                                .spacing(units(2))
                                .align_y(Alignment::Center),
                            )
                            .padding([units(1), units(2)])
                            .width(Length::Fill)
                            .style(style::list_item)
                            .on_press(Message::Play(favorite.url.clone())),
                            icon_button(
                                Icon::Close,
                                TEXT_SECONDARY,
                                Some(Message::FavoriteRemoved(favorite.url.clone()))
                            ),
                        ]
                        .align_y(Alignment::Center),
                    )
                });
        col = col.push(labeled(
            "Favoritas",
            scrollable(container(favorites).padding(Padding::default().right(units(2))))
                .height(Length::Shrink)
                .style(style::scroll),
        ));
    }

    if player.tools().is_some_and(|t| t.ytdlp.is_some()) {
        col = col.push(
            row![
                small(
                    player
                        .tools()
                        .and_then(|t| t.ytdlp.clone())
                        .map(|v| format!("yt-dlp {v}"))
                        .unwrap_or_default()
                ),
                space().width(Length::Fill),
                icon_label_button(
                    Icon::Refresh,
                    if player.updating_ytdlp() {
                        "Atualizando"
                    } else {
                        "Atualizar yt-dlp"
                    },
                    (!player.updating_ytdlp()).then_some(Message::UpdateYtdlp)
                ),
            ]
            .align_y(Alignment::Center),
        );
    }

    col.into()
}

pub fn view(player: &Player) -> Element<'_, Message> {
    let content = if player.expanded() {
        expanded(player)
    } else {
        collapsed(player)
    };
    container(content)
        .padding(units(3))
        .style(style::dialog)
        .into()
}
