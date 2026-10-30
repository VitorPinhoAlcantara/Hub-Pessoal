mod view;

#[cfg(test)]
mod tests;

use std::time::Duration;

use hub_core::performance::{Sampler, Snapshot, percent_of, push_history};
use hub_ui_kit::Section;
use hub_ui_kit::iced::{Element, Subscription, Task, time};

#[derive(Debug, Clone)]
pub enum Message {
    Tick,
}

pub struct Performance {
    sampler: Sampler,
    snapshot: Option<Snapshot>,
    cpu_history: Vec<f32>,
    gpu_history: Vec<f32>,
    ram_history: Vec<f32>,
}

impl Performance {
    pub fn new() -> Self {
        Self {
            sampler: Sampler::new(),
            snapshot: None,
            cpu_history: Vec::new(),
            gpu_history: Vec::new(),
            ram_history: Vec::new(),
        }
    }

    pub fn ready(&self) -> bool {
        self.snapshot.is_some()
    }

    pub fn snapshot(&self) -> Option<&Snapshot> {
        self.snapshot.as_ref()
    }

    pub fn cpu_history(&self) -> &[f32] {
        &self.cpu_history
    }

    pub fn gpu_history(&self) -> &[f32] {
        &self.gpu_history
    }

    pub fn ram_history(&self) -> &[f32] {
        &self.ram_history
    }

    fn sample(&mut self) {
        let snapshot = self.sampler.sample();
        push_history(&mut self.cpu_history, snapshot.cpu);
        push_history(
            &mut self.gpu_history,
            snapshot.gpu.as_ref().map_or(0.0, |gpu| gpu.usage),
        );
        push_history(
            &mut self.ram_history,
            percent_of(snapshot.ram_used, snapshot.ram_total),
        );
        self.snapshot = Some(snapshot);
    }
}

impl Default for Performance {
    fn default() -> Self {
        Self::new()
    }
}

impl Section for Performance {
    type Message = Message;

    fn title(&self) -> &str {
        "Desempenho"
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Tick => self.sample(),
        }
        Task::none()
    }

    fn view(&self) -> Element<'_, Message> {
        view::view(self)
    }

    fn subscription(&self) -> Subscription<Message> {
        time::every(Duration::from_secs(1)).map(|_| Message::Tick)
    }
}
