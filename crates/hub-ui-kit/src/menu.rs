use iced::widget::{Column, button, container, mouse_area, opaque, pin, row, space, text};
use iced::{Alignment, Element, Length, Point, Size};

use crate::icon::Icon;
use crate::style;
use crate::tokens::*;
use crate::widgets::icon;

pub const MENU_WIDTH: f32 = 230.0;
const ITEM_HEIGHT: f32 = 32.0;
const MARGIN: f32 = 8.0;

#[derive(Debug, Clone)]
pub struct MenuItem<M> {
    pub label: String,
    pub icon: Option<Icon>,
    pub shortcut: Option<String>,
    pub message: Option<M>,
    pub danger: bool,
}

impl<M> MenuItem<M> {
    pub fn new(label: impl Into<String>, message: M) -> Self {
        Self {
            label: label.into(),
            icon: None,
            shortcut: None,
            message: Some(message),
            danger: false,
        }
    }

    pub fn icon(mut self, icon: Icon) -> Self {
        self.icon = Some(icon);
        self
    }

    pub fn shortcut(mut self, shortcut: impl Into<String>) -> Self {
        self.shortcut = Some(shortcut.into());
        self
    }

    pub fn danger(mut self) -> Self {
        self.danger = true;
        self
    }

    pub fn disabled(mut self) -> Self {
        self.message = None;
        self
    }

    pub fn map<N>(self, f: impl Fn(M) -> N) -> MenuItem<N> {
        MenuItem {
            label: self.label,
            icon: self.icon,
            shortcut: self.shortcut,
            message: self.message.map(f),
            danger: self.danger,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ContextMenu<M> {
    pub at: Point,
    pub items: Vec<MenuItem<M>>,
    pub on_close: M,
}

impl<M> ContextMenu<M> {
    pub fn new(at: Point, items: Vec<MenuItem<M>>, on_close: M) -> Self {
        Self {
            at,
            items,
            on_close,
        }
    }

    pub fn at_cursor(items: Vec<MenuItem<M>>, on_close: M) -> Self {
        Self::new(crate::cursor::position(), items, on_close)
    }

    pub fn map<N>(self, f: impl Fn(M) -> N) -> ContextMenu<N> {
        ContextMenu {
            at: self.at,
            items: self.items.into_iter().map(|item| item.map(&f)).collect(),
            on_close: f(self.on_close),
        }
    }
}

pub fn placement(at: Point, items: usize, viewport: Size) -> Point {
    let height = items as f32 * ITEM_HEIGHT + MARGIN;
    let x = at.x.min(viewport.width - MENU_WIDTH - MARGIN).max(MARGIN);
    let y = if at.y + height > viewport.height - MARGIN {
        (at.y - height).max(MARGIN)
    } else {
        at.y
    };
    Point::new(x, y)
}

pub fn view<'a, M: Clone + 'a>(menu: &ContextMenu<M>) -> Element<'a, M> {
    let entries = menu
        .items
        .iter()
        .fold(Column::new().spacing(0), |col, item| {
            let enabled = item.message.is_some();
            let color = if !enabled {
                alpha(TEXT_SECONDARY, 0.5)
            } else if item.danger {
                DANGER
            } else {
                TEXT_PRIMARY
            };
            let mut line = row![].spacing(units(2)).align_y(Alignment::Center);
            line = match item.icon {
                Some(glyph) => line.push(icon(glyph, ICON_SIZE - 4.0, color)),
                None => line.push(space().width(ICON_SIZE - 4.0)),
            };
            line = line.push(
                text(item.label.clone())
                    .size(FONT_SIZE)
                    .color(color)
                    .width(Length::Fill),
            );
            if let Some(shortcut) = &item.shortcut {
                line = line.push(
                    text(shortcut.clone())
                        .size(FONT_SIZE_SMALL)
                        .color(TEXT_SECONDARY),
                );
            }
            col.push(
                button(line)
                    .padding([units(1) + 2.0, units(3)])
                    .width(Length::Fill)
                    .style(style::list_item)
                    .on_press_maybe(item.message.clone()),
            )
        });
    let card = container(entries)
        .padding(units(1))
        .width(MENU_WIDTH)
        .style(style::dialog);
    let position = placement(menu.at, menu.items.len(), crate::cursor::viewport());
    let layer = mouse_area(
        pin(opaque(card))
            .position(position)
            .width(Length::Fill)
            .height(Length::Fill),
    )
    .on_press(menu.on_close.clone())
    .on_right_press(menu.on_close.clone());
    opaque(layer)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn placement_keeps_menu_inside_window() {
        let viewport = Size::new(800.0, 600.0);
        assert_eq!(
            placement(Point::new(100.0, 100.0), 3, viewport),
            Point::new(100.0, 100.0)
        );
        let right = placement(Point::new(790.0, 100.0), 3, viewport);
        assert!(right.x + MENU_WIDTH <= 800.0);
        let bottom = placement(Point::new(100.0, 590.0), 5, viewport);
        assert!(bottom.y < 590.0);
    }

    #[test]
    fn map_converts_messages() {
        let menu = ContextMenu::new(
            Point::ORIGIN,
            vec![
                MenuItem::new("Abrir", 1).icon(Icon::Play),
                MenuItem::new("x", 2).disabled(),
            ],
            0,
        );
        let mapped = menu.map(|n| n * 10);
        assert_eq!(mapped.items[0].message, Some(10));
        assert_eq!(mapped.items[1].message, None);
        assert_eq!(mapped.on_close, 0);
    }
}
