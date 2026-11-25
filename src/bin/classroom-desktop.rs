#[path = "classroom-desktop/mod.rs"]
mod desktop;

fn main() -> iced::Result {
    desktop::run()
}
