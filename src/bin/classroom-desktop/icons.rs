use super::{appearance_color, button, desktop_data, quiet_style, svg, text, tooltip, Color, CourseworkKind, Element, Message, Pixels, FG, MUTED};

#[derive(Clone, Copy)]
pub(super) enum Icon {
    Copy, External, Download, Link, Document, Form,
    Menu, Home, Calendar, Check, Classroom, Refresh, Arrow, Book, Feed, People, Info, Folder, Video, Settings, Grip,
}

impl Icon {
    fn path(self) -> &'static str {
        match self {
            Self::Link => "m10 13 4-4m-5 2-2 2a3.5 3.5 0 0 0 5 5l3-3m0-2 2-2a3.5 3.5 0 0 0-5-5l-3 3",
            Self::Document => "M5 3h9l5 5v13H5zM14 3v6h5M8 13h8m-8 4h6",
            Self::Form => "M5 3h14v18H5zM8 8h1m3 0h4m-8 4h1m3 0h4m-8 4h1m3 0h4",
            Self::Copy => "M8 8h12v13H8zM5 16H3V3h12v2",
            Self::External => "M13 3h8v8m0-8L10 14m0-9H5a2 2 0 0 0-2 2v12a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2v-5",
            Self::Download => "M12 3v12m-5-5 5 5 5-5M4 16v4h16v-4",
            Self::Grip => "M9 5h.01m6 0h.01M9 12h.01m6 0h.01M9 19h.01m6 0h.01",
            Self::Menu => "M3 6h18M3 12h18M3 18h18",
            Self::Home => "M3 11l9-8 9 8M5 10v10h5v-6h4v6h5V10",
            Self::Calendar => "M7 3v4m10-4v4M4 10h16M5 5h14a1 1 0 0 1 1 1v13a1 1 0 0 1-1 1H5a1 1 0 0 1-1-1V6a1 1 0 0 1 1-1Z",
            Self::Check => "M5 4h14a1 1 0 0 1 1 1v14a1 1 0 0 1-1 1H5a1 1 0 0 1-1-1V5a1 1 0 0 1 1-1Zm3 8 3 3 5-6",
            Self::Classroom => "M4 4h16a1 1 0 0 1 1 1v12H3V5a1 1 0 0 1 1-1Zm3 16h10m-8-7 3-2 3 2",
            Self::Refresh => "M20 11a8 8 0 1 0-2.4 6M20 4v7h-7",
            Self::Arrow => "m9 6 6 6-6 6",
            Self::Book => "M5 3h14v18H6a2 2 0 0 1 0-4h13M5 3v16m4-11h6m-6 4h5",
            Self::Feed => "M4 4h16v16H4zm4 5h8m-8 4h8m-8 4h4",
            Self::People => "M16 21v-2a4 4 0 0 0-4-4H8a4 4 0 0 0-4 4v2m16 0v-2a4 4 0 0 0-2-3.46M13.5 7.5a3.5 3.5 0 1 1-7 0 3.5 3.5 0 0 1 7 0Zm4 3.4a3.5 3.5 0 0 0 0-6.8",
            Self::Info => "M12 22a10 10 0 1 0 0-20 10 10 0 0 0 0 20Zm0-11v5m0-8h.01",
            Self::Folder => "M3 6h7l2 2h9v11H3V6Z",
            Self::Video => "M3 6h12v12H3zM15 10l6-3v10l-6-3",
            Self::Settings => "M10 3h4l1 3 3 1 3 2-1 3 1 3-3 2-3 1-1 3h-4l-1-3-3-1-3-2 1-3-1-3 3-2 3-1zM15 12a3 3 0 1 0-6 0 3 3 0 0 0 6 0Z",
        }
    }
}

pub(super) fn icon(glyph: Icon, color: Color, size: impl Into<Pixels>) -> Element<'static, Message> {
    let size = size.into();
    let path = glyph.path();
    let markup = format!(r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path d="{path}" fill="none" stroke="black" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round"/></svg>"#);
    svg(svg::Handle::from_memory(markup.into_bytes()))
        .width(size).height(size)
        .style(move |theme, _| svg::Style { color: Some(appearance_color(theme, color)) })
        .into()
}

pub(super) fn icon_action(glyph: Icon, label: &'static str, action: Option<Message>) -> Element<'static, Message> {
    let tint = if action.is_some() { FG } else { MUTED };
    let control = button(icon(glyph, tint, 18)).padding(9).style(quiet_style).on_press_maybe(action);
    tooltip(control, text(label).size(13), tooltip::Position::Bottom).padding(6).into()
}

pub(super) fn classroom_mark() -> Element<'static, Message> {
    let artwork = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 32 28">
      <rect x="1" y="2" width="30" height="24" rx="3.5" fill="#197A50"/>
      <rect x="3" y="4" width="26" height="20" rx="2" fill="#229664"/>
      <circle cx="16" cy="11" r="3.2" fill="#fff"/>
      <circle cx="8.8" cy="12.2" r="2.4" fill="#C9E9D7"/>
      <circle cx="23.2" cy="12.2" r="2.4" fill="#C9E9D7"/>
      <path d="M4.8 20a4 4 0 0 1 7.1-2.5l-.6 2.5zm16 0-.6-2.5a4 4 0 0 1 7.1 2.5z" fill="#C9E9D7"/>
      <path d="M10.5 20v-1a5.5 5.5 0 1 1 11 0v1z" fill="#fff"/>
    </svg>"##;
    svg(svg::Handle::from_memory(artwork.as_slice()))
        .width(Pixels(31.0)).height(Pixels(27.0)).into()
}

pub(super) fn coursework_icon(kind: CourseworkKind) -> Icon {
    match kind {
        CourseworkKind::Announcement => Icon::Feed,
        CourseworkKind::Assignment => Icon::Check,
        CourseworkKind::Question => Icon::Info,
        CourseworkKind::Material => Icon::Book,
    }
}

pub(super) fn attachment_icon(kind: desktop_data::AttachmentKind) -> Icon {
    use desktop_data::AttachmentKind::*;
    match kind {
        Link => Icon::Link,
        DriveFile | Other => Icon::Document,
        Form => Icon::Form,
        Video => Icon::Video,
    }
}
