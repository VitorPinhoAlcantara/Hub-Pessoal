use iced::widget::{
    Button, Container, Text, button, container, row, scrollable, stack, svg, text, text_input,
    tooltip,
};
use iced::{Alignment, Color, Element, Length, Padding};

use crate::icon::Icon;
use crate::style;
use crate::tokens::*;

pub fn icon<'a, M: 'a>(icon: Icon, size: f32, color: Color) -> Element<'a, M> {
    svg(icon.handle())
        .width(size)
        .height(size)
        .style(style::icon_color(color))
        .into()
}

pub fn panel<'a, M: 'a>(content: impl Into<Element<'a, M>>) -> Container<'a, M> {
    container(content)
        .padding(units(4))
        .width(Length::Fill)
        .height(Length::Fill)
        .style(style::panel)
}

pub fn title<'a>(value: impl text::IntoFragment<'a>) -> Text<'a> {
    text(value).size(FONT_SIZE_TITLE).color(TEXT_PRIMARY)
}

pub fn heading<'a>(value: impl text::IntoFragment<'a>) -> Text<'a> {
    text(value).size(FONT_SIZE_HEADING).color(TEXT_PRIMARY)
}

pub fn body<'a>(value: impl text::IntoFragment<'a>) -> Text<'a> {
    text(value).size(FONT_SIZE).color(TEXT_PRIMARY)
}

pub fn muted<'a>(value: impl text::IntoFragment<'a>) -> Text<'a> {
    text(value).size(FONT_SIZE).color(TEXT_SECONDARY)
}

pub fn small<'a>(value: impl text::IntoFragment<'a>) -> Text<'a> {
    text(value).size(FONT_SIZE_SMALL).color(TEXT_SECONDARY)
}

pub fn icon_button<'a, M: Clone + 'a>(
    glyph: Icon,
    color: Color,
    on_press: Option<M>,
) -> Button<'a, M> {
    button(
        container(icon(glyph, ICON_SIZE - 2.0, color))
            .center_x(Length::Fill)
            .center_y(Length::Fill),
    )
    .width(units(7))
    .height(units(7))
    .padding(0)
    .style(style::ghost)
    .on_press_maybe(on_press)
}

pub fn with_tooltip<'a, M: 'a>(
    content: impl Into<Element<'a, M>>,
    label: &'a str,
    position: tooltip::Position,
) -> Element<'a, M> {
    tooltip(
        content,
        container(small(label).color(TEXT_PRIMARY))
            .padding([units(1), units(2)])
            .style(style::elevated),
        position,
    )
    .gap(units(1))
    .into()
}

pub fn sidebar_button<'a, M: Clone + 'a>(
    glyph: Icon,
    label: &'a str,
    active: bool,
    on_press: M,
) -> Element<'a, M> {
    let color = if active { ACCENT } else { TEXT_SECONDARY };
    let button = button(
        container(icon(glyph, ICON_SIZE + 2.0, color))
            .center_x(Length::Fill)
            .center_y(Length::Fill),
    )
    .width(units(10))
    .height(units(10))
    .padding(0)
    .style(style::sidebar(active))
    .on_press(on_press);
    let marker: Element<'a, M> = if active {
        container(
            container(iced::widget::space())
                .width(3)
                .height(units(5))
                .style(style::indicator),
        )
        .height(units(10))
        .center_y(units(10))
        .into()
    } else {
        iced::widget::space().width(3).into()
    };
    let content = row![
        marker,
        iced::widget::space().width(units(1)),
        with_tooltip(button, label, tooltip::Position::Right)
    ]
    .align_y(Alignment::Center);
    stack![content].into()
}

pub fn text_field<'a, M: Clone + 'a>(
    placeholder: &str,
    value: &str,
    on_input: impl Fn(String) -> M + 'a,
) -> text_input::TextInput<'a, M> {
    text_input(placeholder, value)
        .on_input(on_input)
        .padding([units(2), units(3)])
        .size(FONT_SIZE)
        .style(style::input)
}

pub fn primary_button<'a, M: Clone + 'a>(label: &'a str, on_press: Option<M>) -> Button<'a, M> {
    button(text(label).size(FONT_SIZE))
        .padding([units(2), units(4)])
        .style(style::primary)
        .on_press_maybe(on_press)
}

pub fn ghost_button<'a, M: Clone + 'a>(label: &'a str, on_press: Option<M>) -> Button<'a, M> {
    button(text(label).size(FONT_SIZE))
        .padding([units(2), units(3)])
        .style(style::ghost)
        .on_press_maybe(on_press)
}

pub fn tab_button<'a, M: Clone + 'a>(
    glyph: Icon,
    label: &'a str,
    active: bool,
    on_press: M,
) -> Button<'a, M> {
    let color = if active { TEXT_PRIMARY } else { TEXT_SECONDARY };
    button(
        row![
            icon(glyph, ICON_SIZE - 4.0, color),
            text(label).size(FONT_SIZE)
        ]
        .spacing(units(2))
        .align_y(Alignment::Center),
    )
    .padding([units(1) + 2.0, units(3)])
    .style(style::toggle(active))
    .on_press(on_press)
}

pub fn scroll<'a, M: 'a>(content: impl Into<Element<'a, M>>) -> Element<'a, M> {
    scrollable(container(content).padding(Padding::default().right(units(3))))
        .height(Length::Fill)
        .style(style::scroll)
        .into()
}
