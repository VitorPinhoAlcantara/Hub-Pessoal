use iced::widget::{button, checkbox, container, scrollable, svg, text_editor, text_input};
use iced::{Background, Border, Color, Shadow, Theme};

use crate::tokens::{self, *};

pub fn theme() -> Theme {
    Theme::custom(
        "hub",
        iced::theme::Palette {
            background: bg_base(),
            text: text_primary(),
            primary: accent(),
            success: success(),
            warning: warning(),
            danger: tokens::danger(),
        },
    )
}

fn border(color: Color, width: f32, radius: f32) -> Border {
    Border {
        color,
        width,
        radius: radius.into(),
    }
}

pub fn base(_theme: &Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(bg_base())),
        text_color: Some(text_primary()),
        ..container::Style::default()
    }
}

pub fn panel(_theme: &Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(bg_panel())),
        text_color: Some(text_primary()),
        border: border(border_subtle(), 1.0, RADIUS_PANEL),
        ..container::Style::default()
    }
}

pub fn elevated(_theme: &Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(bg_elevated())),
        text_color: Some(text_primary()),
        border: border(border_subtle(), 1.0, RADIUS_CONTROL),
        ..container::Style::default()
    }
}

pub fn banner(_theme: &Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(alpha(accent(), 0.12))),
        text_color: Some(text_primary()),
        border: border(alpha(accent(), 0.4), 1.0, RADIUS_CONTROL),
        ..container::Style::default()
    }
}

pub fn indicator(_theme: &Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(accent())),
        border: border(Color::TRANSPARENT, 0.0, 2.0),
        ..container::Style::default()
    }
}

fn flat(background: Option<Color>, text: Color) -> button::Style {
    button::Style {
        background: background.map(Background::Color),
        text_color: text,
        border: border(Color::TRANSPARENT, 0.0, RADIUS_CONTROL),
        shadow: Shadow::default(),
        snap: true,
    }
}

pub fn ghost(_theme: &Theme, status: button::Status) -> button::Style {
    match status {
        button::Status::Hovered => flat(Some(bg_hover()), text_primary()),
        button::Status::Pressed => flat(Some(bg_elevated()), text_primary()),
        button::Status::Disabled => flat(None, alpha(text_secondary(), 0.5)),
        button::Status::Active => flat(None, text_secondary()),
    }
}

pub fn sidebar(active: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |theme, status| {
        if active {
            flat(Some(bg_elevated()), accent())
        } else {
            ghost(theme, status)
        }
    }
}

pub fn toggle(active: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |theme, status| {
        if active {
            flat(Some(bg_elevated()), text_primary())
        } else {
            ghost(theme, status)
        }
    }
}

pub fn window_close(_theme: &Theme, status: button::Status) -> button::Style {
    match status {
        button::Status::Hovered | button::Status::Pressed => {
            flat(Some(tokens::danger()), contrast_text(tokens::danger()))
        }
        _ => flat(None, text_secondary()),
    }
}

pub fn primary(_theme: &Theme, status: button::Status) -> button::Style {
    let background = match status {
        button::Status::Hovered => Color {
            a: 0.85,
            ..accent()
        },
        button::Status::Disabled => alpha(accent(), 0.4),
        _ => accent(),
    };
    flat(Some(background), on_accent())
}

pub fn danger(_theme: &Theme, status: button::Status) -> button::Style {
    match status {
        button::Status::Hovered | button::Status::Pressed => {
            flat(Some(alpha(tokens::danger(), 0.15)), tokens::danger())
        }
        _ => flat(None, text_secondary()),
    }
}

pub fn list_item(_theme: &Theme, status: button::Status) -> button::Style {
    match status {
        button::Status::Hovered | button::Status::Pressed => flat(Some(bg_hover()), text_primary()),
        _ => flat(None, text_primary()),
    }
}

pub fn input(_theme: &Theme, status: text_input::Status) -> text_input::Style {
    let border_color = match status {
        text_input::Status::Focused { .. } => accent(),
        text_input::Status::Hovered => bg_hover(),
        _ => border_subtle(),
    };
    text_input::Style {
        background: Background::Color(bg_elevated()),
        border: border(border_color, 1.0, RADIUS_CONTROL),
        icon: text_secondary(),
        placeholder: text_secondary(),
        value: text_primary(),
        selection: alpha(accent(), 0.35),
    }
}

pub fn check(_theme: &Theme, status: checkbox::Status) -> checkbox::Style {
    let (checked, hovered) = match status {
        checkbox::Status::Active { is_checked } => (is_checked, false),
        checkbox::Status::Hovered { is_checked } => (is_checked, true),
        checkbox::Status::Disabled { is_checked } => (is_checked, false),
    };
    checkbox::Style {
        background: Background::Color(if checked { accent() } else { bg_elevated() }),
        icon_color: on_accent(),
        border: border(
            if checked || hovered {
                accent()
            } else {
                text_secondary()
            },
            1.0,
            RADIUS_CONTROL - 2.0,
        ),
        text_color: None,
    }
}

pub fn scroll(theme: &Theme, status: scrollable::Status) -> scrollable::Style {
    let mut style = scrollable::default(theme, status);
    for rail in [&mut style.vertical_rail, &mut style.horizontal_rail] {
        rail.background = None;
        rail.border = border(Color::TRANSPARENT, 0.0, 0.0);
        rail.scroller.background = Background::Color(bg_hover());
        rail.scroller.border = border(Color::TRANSPARENT, 0.0, 4.0);
    }
    style
}

pub fn icon_color(color: Color) -> impl Fn(&Theme, svg::Status) -> svg::Style {
    move |_, _| svg::Style { color: Some(color) }
}

pub fn outlined(color: Color, faded: bool) -> impl Fn(&Theme) -> container::Style {
    move |_| container::Style {
        background: Some(Background::Color(bg_elevated())),
        text_color: Some(if faded {
            text_secondary()
        } else {
            text_primary()
        }),
        border: border(
            if faded { alpha(color, 0.35) } else { color },
            1.0,
            RADIUS_CONTROL,
        ),
        ..container::Style::default()
    }
}

pub fn drop_zone(highlight: bool, faded: bool) -> impl Fn(&Theme) -> container::Style {
    move |_| container::Style {
        background: highlight.then_some(Background::Color(alpha(accent(), 0.08))),
        text_color: Some(if faded {
            text_secondary()
        } else {
            text_primary()
        }),
        border: border(
            if highlight {
                accent()
            } else {
                Color::TRANSPARENT
            },
            1.0,
            RADIUS_PANEL,
        ),
        ..container::Style::default()
    }
}

pub fn day_column(highlight: bool, faded: bool) -> impl Fn(&Theme) -> container::Style {
    move |_| container::Style {
        background: Some(Background::Color(if faded {
            alpha(bg_base(), 0.6)
        } else {
            bg_base()
        })),
        text_color: Some(text_primary()),
        border: border(
            if highlight { accent() } else { border_subtle() },
            1.0,
            RADIUS_PANEL,
        ),
        ..container::Style::default()
    }
}

pub fn calendar_cell(
    today: bool,
    in_month: bool,
) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_, status| {
        let background = match status {
            button::Status::Hovered | button::Status::Pressed => bg_hover(),
            _ if in_month => bg_base(),
            _ => alpha(bg_base(), 0.4),
        };
        button::Style {
            background: Some(Background::Color(background)),
            text_color: if in_month {
                text_primary()
            } else {
                text_secondary()
            },
            border: border(
                if today { accent() } else { border_subtle() },
                1.0,
                RADIUS_CONTROL,
            ),
            shadow: Shadow::default(),
            snap: true,
        }
    }
}

pub fn dot(color: Color) -> impl Fn(&Theme) -> container::Style {
    move |_| container::Style {
        background: Some(Background::Color(color)),
        border: border(Color::TRANSPARENT, 0.0, 4.0),
        ..container::Style::default()
    }
}

pub fn pill(color: Color) -> impl Fn(&Theme) -> container::Style {
    move |_| container::Style {
        background: Some(Background::Color(color)),
        text_color: Some(contrast_text(color)),
        border: border(Color::TRANSPARENT, 0.0, 10.0),
        ..container::Style::default()
    }
}

pub fn backdrop(_theme: &Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(Color {
            a: 0.6,
            ..Color::BLACK
        })),
        ..container::Style::default()
    }
}

pub fn dialog(_theme: &Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(solid(bg_panel()))),
        text_color: Some(text_primary()),
        border: border(solid(border_subtle()), 1.0, RADIUS_PANEL),
        ..container::Style::default()
    }
}

pub fn editor(_theme: &Theme, status: text_editor::Status) -> text_editor::Style {
    let border_color = match status {
        text_editor::Status::Focused { .. } => accent(),
        text_editor::Status::Hovered => bg_hover(),
        _ => border_subtle(),
    };
    text_editor::Style {
        background: Background::Color(bg_elevated()),
        border: border(border_color, 1.0, RADIUS_CONTROL),
        placeholder: text_secondary(),
        value: text_primary(),
        selection: alpha(accent(), 0.35),
    }
}

pub fn danger_solid(_theme: &Theme, status: button::Status) -> button::Style {
    let background = match status {
        button::Status::Hovered => Color {
            a: 0.85,
            ..tokens::danger()
        },
        button::Status::Disabled => alpha(tokens::danger(), 0.4),
        _ => tokens::danger(),
    };
    flat(Some(background), contrast_text(tokens::danger()))
}

pub fn console(_theme: &Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(console_bg())),
        text_color: Some(text_primary()),
        border: border(border_subtle(), 1.0, RADIUS_CONTROL),
        ..container::Style::default()
    }
}

pub fn badge(color: Color) -> impl Fn(&Theme) -> container::Style {
    move |_| container::Style {
        background: Some(Background::Color(alpha(color, 0.14))),
        text_color: Some(color),
        border: border(alpha(color, 0.35), 1.0, 10.0),
        ..container::Style::default()
    }
}

pub fn selectable(selected: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_, status| {
        let background = match (selected, status) {
            (true, _) => bg_elevated(),
            (false, button::Status::Hovered | button::Status::Pressed) => alpha(bg_elevated(), 0.6),
            _ => bg_base(),
        };
        button::Style {
            background: Some(Background::Color(background)),
            text_color: text_primary(),
            border: border(
                if selected {
                    alpha(accent(), 0.7)
                } else {
                    border_subtle()
                },
                1.0,
                RADIUS_CONTROL,
            ),
            shadow: Shadow::default(),
            snap: true,
        }
    }
}

pub fn grip(_theme: &Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(border_subtle())),
        border: border(Color::TRANSPARENT, 0.0, 2.0),
        ..container::Style::default()
    }
}
