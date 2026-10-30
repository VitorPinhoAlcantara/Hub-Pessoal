use hub_core::performance::{format_gb, percent_of};
use hub_ui_kit::area_chart;
use hub_ui_kit::iced::widget::{column, container, row, scrollable};
use hub_ui_kit::iced::{Color, Element, Length, Padding};
use hub_ui_kit::style;
use hub_ui_kit::tokens::*;
use hub_ui_kit::widgets::*;

use crate::{Message, Performance};

const CARD_HEIGHT: f32 = 230.0;
const BIG_SIZE: f32 = 34.0;

fn card<'a>(
    label: &'a str,
    measuring: bool,
    percent: f32,
    color: Color,
    history: &[f32],
    footer: String,
) -> Element<'a, Message> {
    let mut header = row![small(label)].spacing(units(2));
    if measuring {
        header = header.push(small("medindo"));
    }
    let big = title(format!("{percent:.0}%"))
        .size(BIG_SIZE)
        .color(usage_color(percent, color));
    container(
        column![
            header,
            big,
            area_chart(history, color, Length::Fill),
            small(footer)
        ]
        .spacing(units(2))
        .width(Length::Fill)
        .height(Length::Fill),
    )
    .padding(units(3))
    .width(Length::FillPortion(1))
    .height(CARD_HEIGHT)
    .style(style::elevated)
    .into()
}

pub fn view(section: &Performance) -> Element<'_, Message> {
    let measuring = !section.ready();
    let snapshot = section.snapshot();
    let cpu = snapshot.map_or(0.0, |s| s.cpu);
    let cores = snapshot.map_or(0, |s| s.cores);
    let (ram_used, ram_total) = snapshot.map_or((0, 0), |s| (s.ram_used, s.ram_total));
    let gpu = snapshot.and_then(|s| s.gpu.as_ref());
    let gpu_footer = match (gpu, measuring) {
        (Some(gpu), _) => format!(
            "{}, VRAM {} de {}",
            gpu.name,
            format_gb(gpu.vram_used),
            format_gb(gpu.vram_total)
        ),
        (None, true) => String::new(),
        (None, false) => "GPU indisponível neste sistema".to_string(),
    };
    let cards = row![
        card(
            "CPU",
            measuring,
            cpu,
            metric_cpu(),
            section.cpu_history(),
            format!("{cores} núcleos lógicos"),
        ),
        card(
            "GPU",
            measuring,
            gpu.map_or(0.0, |g| g.usage),
            metric_gpu(),
            section.gpu_history(),
            gpu_footer,
        ),
        card(
            "Memória",
            measuring,
            percent_of(ram_used, ram_total),
            metric_memory(),
            section.ram_history(),
            format!("{} de {}", format_gb(ram_used), format_gb(ram_total)),
        ),
    ]
    .spacing(GAP_PANEL)
    .width(Length::Fill);
    let content = column![title("Desempenho"), cards].spacing(units(3));
    panel(
        scrollable(container(content).padding(Padding::default().right(units(2))))
            .width(Length::Fill)
            .style(style::scroll),
    )
    .into()
}
