pub mod font;
pub mod icon;
pub mod style;
pub mod tokens;
pub mod widgets;

pub use iced;
pub use icon::Icon;

use iced::{Element, Subscription, Task};

pub trait Section {
    type Message;
    fn title(&self) -> &str;
    fn update(&mut self, message: Self::Message) -> Task<Self::Message>;
    fn view(&self) -> Element<'_, Self::Message>;
    fn subscription(&self) -> Subscription<Self::Message>;
    fn on_hide(&mut self) {}
}
