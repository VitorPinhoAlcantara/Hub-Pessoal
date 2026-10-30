use std::f32::consts::{FRAC_PI_2, TAU};

use hub_core::performance::HISTORY_LEN;
use iced::alignment::Vertical;
use iced::widget::canvas::{self, Canvas, Frame, Geometry, LineCap, Path, Stroke, Text, path};
use iced::widget::text::Alignment as TextAlignment;
use iced::{Color, Element, Length, Pixels, Point, Radians, Rectangle, Renderer, Theme, mouse};

use crate::tokens::*;

struct AreaChart {
    values: Vec<f32>,
    color: Color,
}

fn padded(history: &[f32]) -> Vec<f32> {
    let mut values = vec![0.0; HISTORY_LEN.saturating_sub(history.len())];
    values.extend(
        history
            .iter()
            .skip(history.len().saturating_sub(HISTORY_LEN)),
    );
    values
}

impl<M> canvas::Program<M> for AreaChart {
    type State = ();

    fn draw(
        &self,
        _state: &Self::State,
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());
        let (width, height) = (bounds.width, bounds.height);
        for fraction in [0.25, 0.5, 0.75] {
            let y = height * fraction;
            frame.stroke(
                &Path::line(Point::new(0.0, y), Point::new(width, y)),
                Stroke::default()
                    .with_width(1.0)
                    .with_color(border_subtle()),
            );
        }
        let step = width / (self.values.len() - 1) as f32;
        let points: Vec<Point> = self
            .values
            .iter()
            .enumerate()
            .map(|(index, value)| {
                let level = value.clamp(0.0, 100.0) / 100.0;
                Point::new(index as f32 * step, height - level * (height - 2.0) - 1.0)
            })
            .collect();
        let line = Path::new(|builder: &mut path::Builder| {
            builder.move_to(points[0]);
            for point in &points[1..] {
                builder.line_to(*point);
            }
        });
        let area = Path::new(|builder: &mut path::Builder| {
            builder.move_to(Point::new(0.0, height));
            for point in &points {
                builder.line_to(*point);
            }
            builder.line_to(Point::new(width, height));
            builder.close();
        });
        let gradient = canvas::gradient::Linear::new(Point::ORIGIN, Point::new(0.0, height))
            .add_stop(0.0, alpha(self.color, 0.35))
            .add_stop(1.0, alpha(self.color, 0.0));
        frame.fill(&area, canvas::Gradient::Linear(gradient));
        frame.stroke(
            &line,
            Stroke::default().with_width(1.6).with_color(self.color),
        );
        vec![frame.into_geometry()]
    }
}

pub fn area_chart<'a, M: 'a>(
    history: &[f32],
    color: Color,
    height: impl Into<Length>,
) -> Element<'a, M> {
    Canvas::new(AreaChart {
        values: padded(history),
        color,
    })
    .width(Length::Fill)
    .height(height)
    .into()
}

struct RingGauge {
    value: f32,
    color: Color,
    label: String,
}

impl<M> canvas::Program<M> for RingGauge {
    type State = ();

    fn draw(
        &self,
        _state: &Self::State,
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());
        let size = bounds.width.min(bounds.height);
        let center = Point::new(bounds.width / 2.0, bounds.height / 2.0);
        let thickness = size * 0.082;
        let radius = size / 2.0 - thickness;
        let fraction = self.value.clamp(0.0, 100.0) / 100.0;
        frame.stroke(
            &Path::circle(center, radius),
            Stroke::default()
                .with_width(thickness)
                .with_color(bg_hover()),
        );
        if fraction > 0.0 {
            let arc = Path::new(|builder: &mut path::Builder| {
                builder.arc(path::Arc {
                    center,
                    radius,
                    start_angle: Radians(-FRAC_PI_2),
                    end_angle: Radians(-FRAC_PI_2 + TAU * fraction.min(0.9999)),
                });
            });
            frame.stroke(
                &arc,
                Stroke::default()
                    .with_width(thickness)
                    .with_color(self.color)
                    .with_line_cap(LineCap::Round),
            );
        }
        frame.fill_text(Text {
            content: format!("{:.0}%", self.value.clamp(0.0, 100.0)),
            position: center,
            color: text_primary(),
            size: Pixels(size * 0.2),
            align_x: TextAlignment::Center,
            align_y: Vertical::Center,
            ..Text::default()
        });
        if !self.label.is_empty() {
            frame.fill_text(Text {
                content: self.label.clone(),
                position: Point::new(center.x, center.y + size * 0.18),
                color: text_secondary(),
                size: Pixels(FONT_SIZE_SMALL),
                align_x: TextAlignment::Center,
                align_y: Vertical::Center,
                ..Text::default()
            });
        }
        vec![frame.into_geometry()]
    }
}

pub fn ring_gauge<'a, M: 'a>(
    value: f32,
    color: Color,
    size: f32,
    label: impl Into<String>,
) -> Element<'a, M> {
    Canvas::new(RingGauge {
        value,
        color,
        label: label.into(),
    })
    .width(size)
    .height(size)
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_is_padded_with_zeros_to_full_length() {
        assert_eq!(padded(&[]), vec![0.0; HISTORY_LEN]);
        let values = padded(&[5.0, 7.0]);
        assert_eq!(values.len(), HISTORY_LEN);
        assert_eq!(&values[HISTORY_LEN - 2..], &[5.0, 7.0]);
        assert_eq!(padded(&vec![1.0; HISTORY_LEN + 3]).len(), HISTORY_LEN);
    }
}
