use iced::mouse;
use iced::widget::canvas::{self, Canvas, Frame, Geometry, Path, Stroke};
use iced::widget::{Space, button, column, container, row, slider, text};
use iced::{
    Alignment, Background, Border, Color, Element, Event, Length, Point, Rectangle, Renderer, Size,
    Theme,
};

use crate::tokens::*;
use crate::widgets::{choice, small, text_field};

pub const SWATCHES: [Color; 8] = [
    hex(0x7C9CF5),
    hex(0xB48CF2),
    hex(0xE5676B),
    hex(0xE5B567),
    hex(0x6FCF97),
    hex(0x4FC1D9),
    hex(0xF28CC1),
    hex(0x9AA0A6),
];

const AREA_WIDTH: f32 = 220.0;
const AREA_HEIGHT: f32 = 150.0;
const BAR_WIDTH: f32 = 16.0;

fn same_color(a: Color, b: Color) -> bool {
    let tolerance = 1.0 / 255.0 + 1e-6;
    (a.r - b.r).abs() <= tolerance
        && (a.g - b.g).abs() <= tolerance
        && (a.b - b.b).abs() <= tolerance
}

pub fn swatches<'a, M: Clone + 'a>(
    colors: &[Color],
    selected: Color,
    on_pick: impl Fn(Color) -> M + 'a,
) -> Element<'a, M> {
    let mut items = row![].spacing(units(2)).align_y(Alignment::Center);
    for &color in colors {
        let active = same_color(color, selected);
        let swatch = button(Space::new())
            .width(26.0)
            .height(26.0)
            .padding(0)
            .style(move |_theme: &Theme, _status| button::Style {
                background: Some(Background::Color(color)),
                text_color: Color::WHITE,
                border: Border {
                    color: if active {
                        text_primary()
                    } else {
                        border_subtle()
                    },
                    width: if active { 2.0 } else { 1.0 },
                    radius: RADIUS_CONTROL.into(),
                },
                ..button::Style::default()
            })
            .on_press(on_pick(color));
        items = items.push(swatch);
    }
    items.into()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PickerMode {
    #[default]
    Hsv,
    Rgb,
}

#[derive(Debug, Clone)]
pub enum PickerMessage {
    SaturationValue(f32, f32),
    Hue(f32),
    Red(u8),
    Green(u8),
    Blue(u8),
    HexChanged(String),
    HexSubmitted,
    Mode(PickerMode),
}

#[derive(Debug, Clone)]
pub struct ColorPicker {
    pub hue: f32,
    pub saturation: f32,
    pub value: f32,
    pub mode: PickerMode,
    pub hex: String,
}

fn channel(value: f32) -> u8 {
    (value.clamp(0.0, 1.0) * 255.0).round() as u8
}

fn to_rgb8(color: Color) -> (u8, u8, u8) {
    (channel(color.r), channel(color.g), channel(color.b))
}

pub fn hsv_to_rgb(hue: f32, saturation: f32, value: f32) -> Color {
    let h = hue.rem_euclid(360.0) / 60.0;
    let s = saturation.clamp(0.0, 1.0);
    let v = value.clamp(0.0, 1.0);
    let sector = h.floor();
    let f = h - sector;
    let p = v * (1.0 - s);
    let q = v * (1.0 - s * f);
    let t = v * (1.0 - s * (1.0 - f));
    let (r, g, b) = match sector as u32 {
        0 => (v, t, p),
        1 => (q, v, p),
        2 => (p, v, t),
        3 => (p, q, v),
        4 => (t, p, v),
        _ => (v, p, q),
    };
    Color::from_rgb(r, g, b)
}

pub fn rgb_to_hsv(color: Color) -> (f32, f32, f32) {
    let max = color.r.max(color.g).max(color.b);
    let min = color.r.min(color.g).min(color.b);
    let delta = max - min;
    let hue = if delta <= f32::EPSILON {
        0.0
    } else if max == color.r {
        60.0 * ((color.g - color.b) / delta).rem_euclid(6.0)
    } else if max == color.g {
        60.0 * ((color.b - color.r) / delta + 2.0)
    } else {
        60.0 * ((color.r - color.g) / delta + 4.0)
    };
    let saturation = if max <= f32::EPSILON {
        0.0
    } else {
        delta / max
    };
    (hue.rem_euclid(360.0), saturation, max)
}

pub fn to_hex(color: Color) -> String {
    let (r, g, b) = to_rgb8(color);
    format!("#{r:02X}{g:02X}{b:02X}")
}

pub fn parse_hex(text: &str) -> Option<Color> {
    let digits = text.trim().trim_start_matches('#');
    if digits.len() != 6 || !digits.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let r = u8::from_str_radix(&digits[0..2], 16).ok()?;
    let g = u8::from_str_radix(&digits[2..4], 16).ok()?;
    let b = u8::from_str_radix(&digits[4..6], 16).ok()?;
    Some(Color::from_rgb8(r, g, b))
}

impl ColorPicker {
    pub fn new(color: Color) -> Self {
        let (hue, saturation, value) = rgb_to_hsv(color);
        Self {
            hue,
            saturation,
            value,
            mode: PickerMode::Hsv,
            hex: to_hex(color),
        }
    }

    pub fn color(&self) -> Color {
        hsv_to_rgb(self.hue, self.saturation, self.value)
    }

    pub fn set_color(&mut self, color: Color) {
        let (hue, saturation, value) = rgb_to_hsv(color);
        if saturation > 0.0 && value > 0.0 {
            self.hue = hue;
        }
        self.saturation = saturation;
        self.value = value;
        self.hex = to_hex(color);
    }

    pub fn update(&mut self, message: PickerMessage) -> Option<Color> {
        let before = to_rgb8(self.color());
        match message {
            PickerMessage::SaturationValue(saturation, value) => {
                self.saturation = saturation.clamp(0.0, 1.0);
                self.value = value.clamp(0.0, 1.0);
                self.hex = to_hex(self.color());
            }
            PickerMessage::Hue(hue) => {
                self.hue = hue.clamp(0.0, 360.0);
                self.hex = to_hex(self.color());
            }
            PickerMessage::Red(r) => {
                let (_, g, b) = before;
                self.set_color(Color::from_rgb8(r, g, b));
            }
            PickerMessage::Green(g) => {
                let (r, _, b) = before;
                self.set_color(Color::from_rgb8(r, g, b));
            }
            PickerMessage::Blue(b) => {
                let (r, g, _) = before;
                self.set_color(Color::from_rgb8(r, g, b));
            }
            PickerMessage::HexChanged(text) => match parse_hex(&text) {
                Some(color) => self.set_color(color),
                None => self.hex = text,
            },
            PickerMessage::HexSubmitted => match parse_hex(&self.hex) {
                Some(color) => self.set_color(color),
                None => self.hex = to_hex(self.color()),
            },
            PickerMessage::Mode(mode) => {
                self.mode = mode;
                return None;
            }
        }
        let color = self.color();
        (to_rgb8(color) != before).then_some(color)
    }
}

#[derive(Default)]
struct DragState {
    dragging: bool,
}

fn fraction(bounds: Rectangle, point: Point) -> (f32, f32) {
    (
        ((point.x - bounds.x) / bounds.width).clamp(0.0, 1.0),
        ((point.y - bounds.y) / bounds.height).clamp(0.0, 1.0),
    )
}

fn drag_action<M>(
    state: &mut DragState,
    event: &Event,
    bounds: Rectangle,
    cursor: mouse::Cursor,
    make: impl Fn(f32, f32) -> M,
) -> Option<canvas::Action<M>> {
    match event {
        Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
            let position = cursor.position_over(bounds)?;
            state.dragging = true;
            let (x, y) = fraction(bounds, position);
            Some(canvas::Action::publish(make(x, y)).and_capture())
        }
        Event::Mouse(mouse::Event::CursorMoved { position }) if state.dragging => {
            let (x, y) = fraction(bounds, *position);
            Some(canvas::Action::publish(make(x, y)).and_capture())
        }
        Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) if state.dragging => {
            state.dragging = false;
            Some(canvas::Action::capture())
        }
        _ => None,
    }
}

struct SaturationValueArea {
    hue: f32,
    saturation: f32,
    value: f32,
}

impl canvas::Program<PickerMessage> for SaturationValueArea {
    type State = DragState;

    fn update(
        &self,
        state: &mut Self::State,
        event: &Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<canvas::Action<PickerMessage>> {
        drag_action(state, event, bounds, cursor, |x, y| {
            PickerMessage::SaturationValue(x, 1.0 - y)
        })
    }

    fn draw(
        &self,
        _state: &Self::State,
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());
        let size = bounds.size();
        let pure = hsv_to_rgb(self.hue, 1.0, 1.0);

        let horizontal = canvas::gradient::Linear::new(Point::ORIGIN, Point::new(size.width, 0.0))
            .add_stop(0.0, Color::WHITE)
            .add_stop(1.0, pure);
        frame.fill_rectangle(Point::ORIGIN, size, canvas::Gradient::Linear(horizontal));

        let vertical = canvas::gradient::Linear::new(Point::ORIGIN, Point::new(0.0, size.height))
            .add_stop(0.0, Color::TRANSPARENT)
            .add_stop(1.0, Color::BLACK);
        frame.fill_rectangle(Point::ORIGIN, size, canvas::Gradient::Linear(vertical));

        let marker = Point::new(
            self.saturation * size.width,
            (1.0 - self.value) * size.height,
        );
        frame.stroke(
            &Path::circle(marker, 6.0),
            Stroke::default()
                .with_width(1.0)
                .with_color(Color::from_rgba(0.0, 0.0, 0.0, 0.6)),
        );
        frame.stroke(
            &Path::circle(marker, 5.0),
            Stroke::default().with_width(2.0).with_color(Color::WHITE),
        );
        vec![frame.into_geometry()]
    }

    fn mouse_interaction(
        &self,
        state: &Self::State,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> mouse::Interaction {
        if state.dragging || cursor.is_over(bounds) {
            mouse::Interaction::Crosshair
        } else {
            mouse::Interaction::default()
        }
    }
}

struct HueBar {
    hue: f32,
}

impl canvas::Program<PickerMessage> for HueBar {
    type State = DragState;

    fn update(
        &self,
        state: &mut Self::State,
        event: &Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<canvas::Action<PickerMessage>> {
        drag_action(state, event, bounds, cursor, |_, y| {
            PickerMessage::Hue(y * 360.0)
        })
    }

    fn draw(
        &self,
        _state: &Self::State,
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());
        let size = bounds.size();
        let mut gradient =
            canvas::gradient::Linear::new(Point::ORIGIN, Point::new(0.0, size.height));
        for step in 0..=6 {
            let offset = step as f32 / 6.0;
            gradient = gradient.add_stop(offset, hsv_to_rgb(offset * 360.0, 1.0, 1.0));
        }
        frame.fill_rectangle(Point::ORIGIN, size, canvas::Gradient::Linear(gradient));

        let y = (self.hue / 360.0).clamp(0.0, 1.0) * size.height;
        frame.fill_rectangle(
            Point::new(0.0, (y - 2.0).max(0.0)),
            Size::new(size.width, 4.0),
            Color::from_rgba(0.0, 0.0, 0.0, 0.6),
        );
        frame.fill_rectangle(
            Point::new(0.0, (y - 1.0).max(0.0)),
            Size::new(size.width, 2.0),
            Color::WHITE,
        );
        vec![frame.into_geometry()]
    }

    fn mouse_interaction(
        &self,
        state: &Self::State,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> mouse::Interaction {
        if state.dragging || cursor.is_over(bounds) {
            mouse::Interaction::Crosshair
        } else {
            mouse::Interaction::default()
        }
    }
}

fn channel_row<'a, M: Clone + 'a>(
    label: &'a str,
    value: u8,
    on_change: impl Fn(u8) -> M + 'a,
) -> Element<'a, M> {
    row![
        small(label).width(Length::Fixed(14.0)),
        slider(0..=255u8, value, on_change).width(Length::Fill),
        text(value.to_string())
            .size(FONT_SIZE_SMALL)
            .color(text_secondary())
            .width(Length::Fixed(28.0))
            .align_x(iced::alignment::Horizontal::Right),
    ]
    .spacing(units(2))
    .align_y(Alignment::Center)
    .into()
}

pub fn view<'a, M: Clone + 'a>(
    picker: &'a ColorPicker,
    on_message: impl Fn(PickerMessage) -> M + Clone + 'a,
) -> Element<'a, M> {
    let modes = row![
        choice(
            "HSV",
            picker.mode == PickerMode::Hsv,
            on_message(PickerMessage::Mode(PickerMode::Hsv))
        ),
        choice(
            "RGB",
            picker.mode == PickerMode::Rgb,
            on_message(PickerMessage::Mode(PickerMode::Rgb))
        ),
    ]
    .spacing(units(2));

    let body: Element<'a, M> = match picker.mode {
        PickerMode::Hsv => {
            let area = Canvas::new(SaturationValueArea {
                hue: picker.hue,
                saturation: picker.saturation,
                value: picker.value,
            })
            .width(Length::Fixed(AREA_WIDTH))
            .height(Length::Fixed(AREA_HEIGHT));
            let bar = Canvas::new(HueBar { hue: picker.hue })
                .width(Length::Fixed(BAR_WIDTH))
                .height(Length::Fixed(AREA_HEIGHT));
            row![
                Element::from(area).map(on_message.clone()),
                Element::from(bar).map(on_message.clone()),
            ]
            .spacing(units(3))
            .into()
        }
        PickerMode::Rgb => {
            let (r, g, b) = to_rgb8(picker.color());
            let on_red = on_message.clone();
            let on_green = on_message.clone();
            let on_blue = on_message.clone();
            column![
                channel_row("R", r, move |v| on_red(PickerMessage::Red(v))),
                channel_row("G", g, move |v| on_green(PickerMessage::Green(v))),
                channel_row("B", b, move |v| on_blue(PickerMessage::Blue(v))),
            ]
            .spacing(units(3))
            .width(Length::Fixed(AREA_WIDTH + BAR_WIDTH + units(3)))
            .into()
        }
    };

    let current = picker.color();
    let preview = container(Space::new())
        .width(28.0)
        .height(28.0)
        .style(move |_theme: &Theme| container::Style {
            background: Some(Background::Color(current)),
            border: Border {
                color: border_subtle(),
                width: 1.0,
                radius: RADIUS_CONTROL.into(),
            },
            ..container::Style::default()
        });
    let on_input = on_message.clone();
    let hex_field = text_field("#RRGGBB", &picker.hex, move |v| {
        on_input(PickerMessage::HexChanged(v))
    })
    .on_submit(on_message(PickerMessage::HexSubmitted))
    .width(Length::Fixed(110.0));

    column![
        modes,
        body,
        row![preview, hex_field]
            .spacing(units(3))
            .align_y(Alignment::Center),
    ]
    .spacing(units(3))
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: Color, b: Color) -> bool {
        to_rgb8(a) == to_rgb8(b)
    }

    #[test]
    fn primary_colors() {
        assert!(close(
            hsv_to_rgb(0.0, 1.0, 1.0),
            Color::from_rgb8(255, 0, 0)
        ));
        assert!(close(
            hsv_to_rgb(120.0, 1.0, 1.0),
            Color::from_rgb8(0, 255, 0)
        ));
        assert!(close(
            hsv_to_rgb(240.0, 1.0, 1.0),
            Color::from_rgb8(0, 0, 255)
        ));
        assert_eq!(rgb_to_hsv(Color::from_rgb8(255, 0, 0)), (0.0, 1.0, 1.0));
        assert_eq!(rgb_to_hsv(Color::from_rgb8(0, 255, 0)).0, 120.0);
        assert_eq!(rgb_to_hsv(Color::from_rgb8(0, 0, 255)).0, 240.0);
    }

    #[test]
    fn gray_has_no_saturation() {
        let (_, s, v) = rgb_to_hsv(Color::from_rgb8(128, 128, 128));
        assert_eq!(s, 0.0);
        assert!((v - 128.0 / 255.0).abs() < 1e-6);
    }

    #[test]
    fn round_trip() {
        for &(r, g, b) in &[
            (12, 200, 99),
            (255, 255, 255),
            (0, 0, 0),
            (124, 156, 245),
            (229, 103, 107),
            (1, 2, 3),
        ] {
            let color = Color::from_rgb8(r, g, b);
            let (h, s, v) = rgb_to_hsv(color);
            assert!(close(hsv_to_rgb(h, s, v), color));
        }
    }

    #[test]
    fn hex_parsing() {
        assert!(close(
            parse_hex("#7C9CF5").unwrap(),
            Color::from_rgb8(0x7C, 0x9C, 0xF5)
        ));
        assert!(close(
            parse_hex("7c9cf5").unwrap(),
            Color::from_rgb8(0x7C, 0x9C, 0xF5)
        ));
        assert!(parse_hex("#12345").is_none());
        assert!(parse_hex("GGGGGG").is_none());
        assert!(parse_hex("").is_none());
        assert_eq!(to_hex(Color::from_rgb8(0x7C, 0x9C, 0xF5)), "#7C9CF5");
    }

    #[test]
    fn red_update_keeps_mode() {
        let mut picker = ColorPicker::new(Color::from_rgb8(10, 20, 30));
        picker.mode = PickerMode::Rgb;
        let color = picker.update(PickerMessage::Red(200)).unwrap();
        assert_eq!(to_rgb8(color), (200, 20, 30));
        assert_eq!(picker.mode, PickerMode::Rgb);
        assert_eq!(picker.hex, "#C8141E");
    }

    #[test]
    fn hex_update() {
        let mut picker = ColorPicker::new(Color::from_rgb8(10, 20, 30));
        assert!(
            picker
                .update(PickerMessage::HexChanged("#12".into()))
                .is_none()
        );
        assert_eq!(picker.hex, "#12");
        assert_eq!(to_rgb8(picker.color()), (10, 20, 30));
        let color = picker
            .update(PickerMessage::HexChanged("ff0000".into()))
            .unwrap();
        assert_eq!(to_rgb8(color), (255, 0, 0));
        picker.hex = "zz".into();
        assert!(picker.update(PickerMessage::HexSubmitted).is_none());
        assert_eq!(picker.hex, "#FF0000");
    }

    #[test]
    fn hue_preserved_on_zero_saturation() {
        let mut picker = ColorPicker::new(Color::from_rgb8(0, 0, 255));
        picker.set_color(Color::from_rgb8(0, 0, 0));
        assert_eq!(picker.hue, 240.0);
    }
}
