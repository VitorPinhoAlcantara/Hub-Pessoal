use hub_core::chrono::Timelike;
use hub_core::performance::percent_of;
use hub_core::shortcuts::{Shortcut, ShortcutKind};
use hub_ui_kit::iced::widget::image::Handle;
use hub_ui_kit::iced::widget::scrollable::{Direction, Scrollbar};
use hub_ui_kit::iced::widget::{
    Column, Row, button, column, container, image, mouse_area, row, scrollable, space, text,
};
use hub_ui_kit::iced::{Alignment, Color, Element, Length, Padding, mouse};
use hub_ui_kit::tokens::*;
use hub_ui_kit::widgets::*;
use hub_ui_kit::{Icon, font, ring_gauge, style};

use crate::{AgendaItem, Destination, Home, ItemKind, Message, NOTE_LINES, format_date};

const CLOCK_SIZE: f32 = 64.0;
const SECONDS_SIZE: f32 = 24.0;
const CARD_HEIGHT: f32 = 288.0;
const TOP_HEIGHT: f32 = 148.0;
const PERFORMANCE_WIDTH: f32 = 360.0;
const RING_SIZE: f32 = 84.0;
const TILE_WIDTH: f32 = 104.0;
const TILE_HEIGHT: f32 = 98.0;
const TILE_ICON: f32 = 40.0;
const NAME_CHARS: usize = 12;

fn kind_color(kind: ItemKind) -> Color {
    match kind {
        ItemKind::Appointment => kind_appointment(),
        ItemKind::Routine => kind_routine(),
        ItemKind::Reminder => kind_reminder(),
    }
}

fn kind_icon(kind: ShortcutKind) -> (Icon, Color) {
    match kind {
        ShortcutKind::Program => (Icon::App, accent()),
        ShortcutKind::Folder => (Icon::Folder, warning()),
        ShortcutKind::Url => (Icon::Link, success()),
        ShortcutKind::Command => (Icon::Command, kind_routine()),
    }
}

fn short_name(name: &str) -> String {
    let chars: Vec<char> = name.chars().collect();
    if chars.len() <= NAME_CHARS {
        return name.to_string();
    }
    let head: String = chars[..NAME_CHARS - 1].iter().collect();
    format!("{head}…")
}

fn picture<'a>(handle: Option<&Handle>, kind: ShortcutKind, size: f32) -> Element<'a, Message> {
    match handle {
        Some(handle) => image(handle.clone()).width(size).height(size).into(),
        None => {
            let (glyph, color) = kind_icon(kind);
            container(icon(glyph, size * 0.55, color))
                .center_x(size)
                .center_y(size)
                .style(style::badge(color))
                .into()
        }
    }
}

fn card<'a>(
    heading_text: &'a str,
    action: Option<Element<'a, Message>>,
    content: impl Into<Element<'a, Message>>,
    height: Length,
) -> Element<'a, Message> {
    let mut header = Row::new()
        .push(heading(heading_text))
        .push(space().width(Length::Fill))
        .align_y(Alignment::Center);
    if let Some(action) = action {
        header = header.push(action);
    }
    container(column![header, content.into()].spacing(units(3)))
        .padding(units(4))
        .width(Length::Fill)
        .height(height)
        .clip(true)
        .style(style::panel)
        .into()
}

fn clickable<'a>(
    content: impl Into<Element<'a, Message>>,
    message: Message,
) -> Element<'a, Message> {
    mouse_area(content)
        .on_press(message)
        .interaction(mouse::Interaction::Pointer)
        .into()
}

pub fn view(section: &Home) -> Element<'_, Message> {
    let mut content = Column::new().spacing(GAP_PANEL).push(
        row![clock_card(section), performance_card(section)]
            .spacing(GAP_PANEL)
            .width(Length::Fill),
    );
    if let Some(error) = section.error() {
        content = content.push(
            container(
                row![
                    icon(Icon::Warning, ICON_SIZE - 4.0, danger()),
                    body(error).color(danger()).width(Length::Fill),
                    icon_button(Icon::Close, text_secondary(), Some(Message::ErrorDismissed)),
                ]
                .spacing(units(2))
                .align_y(Alignment::Center),
            )
            .padding([units(1), units(3)])
            .width(Length::Fill)
            .style(style::elevated),
        );
    }
    content = content
        .push(
            row![agenda_card(section), note_card(section)]
                .spacing(GAP_PANEL)
                .width(Length::Fill),
        )
        .push(most_used_card(section));
    scrollable(container(content).padding(Padding::default().right(units(2))))
        .height(Length::Fill)
        .width(Length::Fill)
        .style(style::scroll)
        .into()
}

fn clock_card(section: &Home) -> Element<'_, Message> {
    let now = section.now();
    let clock = row![
        text(format!("{:02}:{:02}", now.hour(), now.minute()))
            .size(CLOCK_SIZE)
            .font(font::ui_medium())
            .color(text_primary()),
        container(
            text(format!("{:02}", now.second()))
                .size(SECONDS_SIZE)
                .color(text_secondary())
        )
        .padding(Padding::default().bottom(units(2))),
    ]
    .spacing(units(2))
    .align_y(Alignment::End);
    let content = column![
        clock,
        text(format_date(now.date()))
            .size(FONT_SIZE_TITLE)
            .color(text_secondary()),
    ]
    .spacing(units(1))
    .align_x(Alignment::Center);
    container(content)
        .padding(units(4))
        .width(Length::Fill)
        .center_x(Length::Fill)
        .center_y(TOP_HEIGHT)
        .height(TOP_HEIGHT)
        .style(style::panel)
        .into()
}

fn performance_card(section: &Home) -> Element<'_, Message> {
    let snapshot = section.snapshot();
    let cpu = snapshot.map_or(0.0, |s| s.cpu);
    let ram = snapshot.map_or(0.0, |s| percent_of(s.ram_used, s.ram_total));
    let gpu = snapshot.and_then(|s| s.gpu.as_ref());
    let gpu_ring = match (gpu, snapshot) {
        (Some(gpu), _) => ring_gauge(
            gpu.usage,
            usage_color(gpu.usage, metric_gpu()),
            RING_SIZE,
            "GPU",
        ),
        (None, Some(_)) => ring_gauge(0.0, bg_hover(), RING_SIZE, "sem GPU"),
        (None, None) => ring_gauge(0.0, metric_gpu(), RING_SIZE, "GPU"),
    };
    let rings = row![
        ring_gauge(cpu, usage_color(cpu, metric_cpu()), RING_SIZE, "CPU"),
        gpu_ring,
        ring_gauge(ram, usage_color(ram, metric_memory()), RING_SIZE, "RAM"),
    ]
    .spacing(units(3));
    let content = column![small("Desempenho"), rings]
        .spacing(units(2))
        .align_x(Alignment::Center);
    clickable(
        container(content)
            .padding(units(3))
            .width(PERFORMANCE_WIDTH)
            .height(TOP_HEIGHT)
            .center_x(PERFORMANCE_WIDTH)
            .style(style::panel),
        Message::Navigate(Destination::Performance),
    )
}

fn agenda_row(item: &AgendaItem) -> Element<'_, Message> {
    let label_color = if item.overdue {
        warning()
    } else {
        text_secondary()
    };
    row![
        container(space())
            .width(8)
            .height(8)
            .style(style::dot(kind_color(item.kind))),
        text(item.label.as_str())
            .size(FONT_SIZE)
            .color(label_color)
            .width(64),
        text(item.title.as_str())
            .size(FONT_SIZE)
            .color(text_primary())
            .width(Length::Fill)
            .wrapping(text::Wrapping::None),
    ]
    .spacing(units(2))
    .align_y(Alignment::Center)
    .into()
}

fn agenda_card(section: &Home) -> Element<'_, Message> {
    let open = Message::Navigate(Destination::Agenda);
    let mut list = Column::new().spacing(units(2));
    if section.overdue() > 0 {
        let label = if section.overdue() == 1 {
            "1 tarefa atrasada".to_string()
        } else {
            format!("{} tarefas atrasadas", section.overdue())
        };
        list = list.push(text(label).size(FONT_SIZE).color(warning()));
    }
    let body_view: Element<'_, Message> = if section.items().is_empty() {
        container(muted("Nada marcado para hoje"))
            .center_x(Length::Fill)
            .padding([units(8), 0.0])
            .into()
    } else {
        section
            .items()
            .iter()
            .fold(list, |col, item| col.push(agenda_row(item)))
            .into()
    };
    card(
        "Agenda de hoje",
        Some(ghost_button("Abrir agenda", Some(open.clone())).into()),
        clickable(
            container(body_view)
                .width(Length::Fill)
                .height(Length::Fill),
            open,
        ),
        Length::Fixed(CARD_HEIGHT),
    )
}

fn note_card(section: &Home) -> Element<'_, Message> {
    let body_view: Element<'_, Message> = match section.favorite() {
        Some(note) => {
            let preview: String = note
                .body
                .lines()
                .take(NOTE_LINES)
                .collect::<Vec<_>>()
                .join("\n");
            clickable(
                container(
                    column![
                        body(note.display_title()).font(font::ui_medium()),
                        text(preview).size(FONT_SIZE).color(text_secondary()),
                    ]
                    .spacing(units(2)),
                )
                .width(Length::Fill)
                .height(Length::Fill),
                Message::Navigate(Destination::Notes(Some(note.id))),
            )
        }
        None => container(
            column![
                icon(Icon::Notes, 32.0, text_secondary()),
                muted("Favorite uma nota na aba Anotações para ver aqui"),
                ghost_button(
                    "Abrir anotações",
                    Some(Message::Navigate(Destination::Notes(None)))
                ),
            ]
            .spacing(units(3))
            .align_x(Alignment::Center),
        )
        .center_x(Length::Fill)
        .padding([units(6), 0.0])
        .into(),
    };
    card("Nota favorita", None, body_view, Length::Fixed(CARD_HEIGHT))
}

fn tile<'a>(section: &'a Home, index: usize, shortcut: &'a Shortcut) -> Element<'a, Message> {
    let content = column![
        picture(section.shortcut_icon(shortcut), shortcut.kind, TILE_ICON),
        text(short_name(&shortcut.name))
            .size(FONT_SIZE)
            .color(text_primary())
            .align_x(text::Alignment::Center)
            .width(Length::Fill)
            .wrapping(text::Wrapping::None),
    ]
    .spacing(units(2))
    .align_x(Alignment::Center);
    button(
        container(content)
            .center_x(Length::Fill)
            .center_y(Length::Fill)
            .clip(true),
    )
    .padding(units(2))
    .width(TILE_WIDTH)
    .height(TILE_HEIGHT)
    .style(style::selectable(false))
    .on_press(Message::Open(index))
    .into()
}

fn most_used_card(section: &Home) -> Element<'_, Message> {
    let body_view: Element<'_, Message> = if section.most_used().is_empty() {
        container(muted("Nenhum atalho ainda"))
            .center_x(Length::Fill)
            .padding([units(4), 0.0])
            .into()
    } else {
        let tiles = section
            .most_used()
            .iter()
            .enumerate()
            .fold(Row::new().spacing(units(2)), |row, (index, shortcut)| {
                row.push(tile(section, index, shortcut))
            });
        scrollable(container(tiles).padding(Padding::default().bottom(units(3))))
            .direction(Direction::Horizontal(
                Scrollbar::new().width(4).scroller_width(4),
            ))
            .width(Length::Fill)
            .style(style::scroll)
            .into()
    };
    card(
        "Mais usados",
        Some(ghost_button("Ver todos", Some(Message::Navigate(Destination::Shortcuts))).into()),
        body_view,
        Length::Shrink,
    )
}
