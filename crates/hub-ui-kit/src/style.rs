use iced::widget::{button, checkbox, container, scrollable, svg, text_editor, text_input};
use iced::{Background, Border, Color, Shadow, Theme};

use crate::tokens::*;

pub fn theme() -> Theme {
    Theme::custom(
        "hub",
        iced::theme::Palette {
            background: BG_BASE,
            text: TEXT_PRIMARY,
            primary: ACCENT,
            success: SUCCESS,
            warning: WARNING,
            danger: DANGER,
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
        background: Some(Background::Color(BG_BASE)),
        text_color: Some(TEXT_PRIMARY),
        ..container::Style::default()
    }
}

pub fn panel(_theme: &Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(BG_PANEL)),
        text_color: Some(TEXT_PRIMARY),
        border: border(BORDER_SUBTLE, 1.0, RADIUS_PANEL),
        ..container::Style::default()
    }
}

pub fn elevated(_theme: &Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(BG_ELEVATED)),
        text_color: Some(TEXT_PRIMARY),
        border: border(BORDER_SUBTLE, 1.0, RADIUS_CONTROL),
        ..container::Style::default()
    }
}

pub fn banner(_theme: &Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(alpha(ACCENT, 0.12))),
        text_color: Some(TEXT_PRIMARY),
        border: border(alpha(ACCENT, 0.4), 1.0, RADIUS_CONTROL),
        ..container::Style::default()
    }
}

pub fn indicator(_theme: &Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(ACCENT)),
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
        button::Status::Hovered => flat(Some(BG_HOVER), TEXT_PRIMARY),
        button::Status::Pressed => flat(Some(BG_ELEVATED), TEXT_PRIMARY),
        button::Status::Disabled => flat(None, alpha(TEXT_SECONDARY, 0.5)),
        button::Status::Active => flat(None, TEXT_SECONDARY),
    }
}

pub fn sidebar(active: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |theme, status| {
        if active {
            flat(Some(BG_ELEVATED), ACCENT)
        } else {
            ghost(theme, status)
        }
    }
}

pub fn toggle(active: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |theme, status| {
        if active {
            flat(Some(BG_ELEVATED), TEXT_PRIMARY)
        } else {
            ghost(theme, status)
        }
    }
}

pub fn window_close(_theme: &Theme, status: button::Status) -> button::Style {
    match status {
        button::Status::Hovered | button::Status::Pressed => flat(Some(DANGER), TEXT_PRIMARY),
        _ => flat(None, TEXT_SECONDARY),
    }
}

pub fn primary(_theme: &Theme, status: button::Status) -> button::Style {
    let background = match status {
        button::Status::Hovered => Color { a: 0.85, ..ACCENT },
        button::Status::Disabled => alpha(ACCENT, 0.4),
        _ => ACCENT,
    };
    flat(Some(background), BG_BASE)
}

pub fn danger(_theme: &Theme, status: button::Status) -> button::Style {
    match status {
        button::Status::Hovered | button::Status::Pressed => {
            flat(Some(alpha(DANGER, 0.15)), DANGER)
        }
        _ => flat(None, TEXT_SECONDARY),
    }
}

pub fn list_item(_theme: &Theme, status: button::Status) -> button::Style {
    match status {
        button::Status::Hovered | button::Status::Pressed => flat(Some(BG_HOVER), TEXT_PRIMARY),
        _ => flat(None, TEXT_PRIMARY),
    }
}

pub fn input(_theme: &Theme, status: text_input::Status) -> text_input::Style {
    let border_color = match status {
        text_input::Status::Focused { .. } => ACCENT,
        text_input::Status::Hovered => BG_HOVER,
        _ => BORDER_SUBTLE,
    };
    text_input::Style {
        background: Background::Color(BG_ELEVATED),
        border: border(border_color, 1.0, RADIUS_CONTROL),
        icon: TEXT_SECONDARY,
        placeholder: TEXT_SECONDARY,
        value: TEXT_PRIMARY,
        selection: alpha(ACCENT, 0.35),
    }
}

pub fn check(_theme: &Theme, status: checkbox::Status) -> checkbox::Style {
    let (checked, hovered) = match status {
        checkbox::Status::Active { is_checked } => (is_checked, false),
        checkbox::Status::Hovered { is_checked } => (is_checked, true),
        checkbox::Status::Disabled { is_checked } => (is_checked, false),
    };
    checkbox::Style {
        background: Background::Color(if checked { ACCENT } else { BG_ELEVATED }),
        icon_color: BG_BASE,
        border: border(
            if checked || hovered {
                ACCENT
            } else {
                TEXT_SECONDARY
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
        rail.scroller.background = Background::Color(BG_HOVER);
        rail.scroller.border = border(Color::TRANSPARENT, 0.0, 4.0);
    }
    style
}

pub fn icon_color(color: Color) -> impl Fn(&Theme, svg::Status) -> svg::Style {
    move |_, _| svg::Style { color: Some(color) }
}

pub fn outlined(color: Color, faded: bool) -> impl Fn(&Theme) -> container::Style {
    move |_| container::Style {
        background: Some(Background::Color(BG_ELEVATED)),
        text_color: Some(if faded { TEXT_SECONDARY } else { TEXT_PRIMARY }),
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
        background: highlight.then_some(Background::Color(alpha(ACCENT, 0.08))),
        text_color: Some(if faded { TEXT_SECONDARY } else { TEXT_PRIMARY }),
        border: border(
            if highlight {
                ACCENT
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
            alpha(BG_BASE, 0.6)
        } else {
            BG_BASE
        })),
        text_color: Some(TEXT_PRIMARY),
        border: border(
            if highlight { ACCENT } else { BORDER_SUBTLE },
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
            button::Status::Hovered | button::Status::Pressed => BG_HOVER,
            _ if in_month => BG_BASE,
            _ => alpha(BG_BASE, 0.4),
        };
        button::Style {
            background: Some(Background::Color(background)),
            text_color: if in_month {
                TEXT_PRIMARY
            } else {
                TEXT_SECONDARY
            },
            border: border(
                if today { ACCENT } else { BORDER_SUBTLE },
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
        text_color: Some(BG_BASE),
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
        background: Some(Background::Color(BG_PANEL)),
        text_color: Some(TEXT_PRIMARY),
        border: border(BORDER_SUBTLE, 1.0, RADIUS_PANEL),
        ..container::Style::default()
    }
}

pub fn editor(_theme: &Theme, status: text_editor::Status) -> text_editor::Style {
    let border_color = match status {
        text_editor::Status::Focused { .. } => ACCENT,
        text_editor::Status::Hovered => BG_HOVER,
        _ => BORDER_SUBTLE,
    };
    text_editor::Style {
        background: Background::Color(BG_ELEVATED),
        border: border(border_color, 1.0, RADIUS_CONTROL),
        placeholder: TEXT_SECONDARY,
        value: TEXT_PRIMARY,
        selection: alpha(ACCENT, 0.35),
    }
}

pub fn danger_solid(_theme: &Theme, status: button::Status) -> button::Style {
    let background = match status {
        button::Status::Hovered => Color { a: 0.85, ..DANGER },
        button::Status::Disabled => alpha(DANGER, 0.4),
        _ => DANGER,
    };
    flat(Some(background), BG_BASE)
}

pub fn console(_theme: &Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(Color::from_rgb8(0x0C, 0x0D, 0x0F))),
        text_color: Some(TEXT_PRIMARY),
        border: border(BORDER_SUBTLE, 1.0, RADIUS_CONTROL),
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
            (true, _) => BG_ELEVATED,
            (false, button::Status::Hovered | button::Status::Pressed) => alpha(BG_ELEVATED, 0.6),
            _ => BG_BASE,
        };
        button::Style {
            background: Some(Background::Color(background)),
            text_color: TEXT_PRIMARY,
            border: border(
                if selected {
                    alpha(ACCENT, 0.7)
                } else {
                    BORDER_SUBTLE
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
        background: Some(Background::Color(BORDER_SUBTLE)),
        border: border(Color::TRANSPARENT, 0.0, 2.0),
        ..container::Style::default()
    }
}
