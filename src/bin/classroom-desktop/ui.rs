use super::*;
use iced::widget::column;

pub(super) fn empty_panel(
    glyph: Icon, title: &'static str, body: &'static str,
) -> Element<'static, Message> {
    let panel = column![
        container(icon(glyph, ACTIVE_FG, 25))
            .center_x(52).center_y(52).style(|_| surface(ACTIVE_BG, 26.0)),
        text(title).color(FG).font(medium()).size(17),
        text(body).color(MUTED).size(14),
    ]
    .spacing(10).align_x(Alignment::Center);
    container(panel).width(Fill).center_x(Fill).padding([32, 15])
        .style(|_| outline_surface(10.0)).into()
}

pub(super) fn medium() -> Font {
    Font { weight: Weight::Medium, ..CLASSROOM_FONT }
}

pub(super) fn truncate(label: &str, max: usize) -> String {
    let mut chars = label.chars();
    let mut text: String = chars.by_ref().take(max).collect();
    if chars.next().is_some() {
        text.push('…');
    }
    text
}

pub(super) fn first_initial(name: &str) -> String {
    name.chars()
        .find(|letter| letter.is_alphanumeric())
        .map(|letter| letter.to_uppercase().collect())
        .unwrap_or_else(|| "C".to_owned())
}

pub(super) fn course_color(course: &Course) -> Color {
    if let Some([r, g, b]) = course.theme.as_ref().and_then(|theme| theme.color_rgb) {
        Color::from_rgb8(r, g, b)
    } else {
        CARD_COLORS[course.id.rem_euclid(CARD_COLORS.len() as i64) as usize]
    }
}

pub(super) fn course_banner<'a>(
    course: &'a Course, artwork: Option<&image::Handle>,
    height: f32, hero: bool, teacher: Option<&'a str>,
) -> Element<'a, Message> {
    let accent = course_color(course);
    let radius = if hero { iced::border::radius(11.0) } else { iced::border::radius(11.0).bottom(0.0) };
    let mut labels = column![
        text(if hero { course.title.clone() } else { truncate(&course.title, 24) })
            .size(if hero { 34 } else { 22 }).font(medium()).color(WHITE),
        text(if hero { course.subtitle.clone() } else { truncate(&course.subtitle, 32) })
            .size(if hero { 16 } else { 14 }).color(WHITE.scale_alpha(0.94)),
    ].spacing(4);
    if let Some(teacher) = teacher {
        labels = labels.push(text(truncate(teacher, 32)).size(13).color(WHITE.scale_alpha(0.94)));
    }
    let labels = container(labels).width(Fill).height(height)
        .align_y(if hero { Alignment::End } else { Alignment::Start })
        .padding(if hero { 24 } else { 15 });

    if let Some(handle) = artwork {
        let backdrop = container(
            image(handle.clone()).width(Fill).height(height).content_fit(ContentFit::Cover).border_radius(radius),
        ).width(Fill).height(height).style(move |_| surface(accent, radius)).clip(true);
        let overlay = labels.style(move |_| {
            let gradient = iced::gradient::Linear::new(iced::Radians(std::f32::consts::FRAC_PI_2))
                .add_stop(0.0, Color::BLACK.scale_alpha(0.68))
                .add_stop(0.76, Color::BLACK.scale_alpha(0.36))
                .add_stop(1.0, Color::BLACK.scale_alpha(0.24));
            container::Style {
                background: Some(Background::Gradient(gradient.into())),
                ..surface(Color::TRANSPARENT, radius)
            }
        });
        container(stack![backdrop, overlay].width(Fill).height(height))
            .width(Fill).height(height).clip(true).into()
    } else {
        labels.style(move |_| {
            let luminance = accent.r * 0.2126 + accent.g * 0.7152 + accent.b * 0.0722;
            let background = if luminance > 0.56 {
                Color::from_rgb(accent.r * 0.70, accent.g * 0.70, accent.b * 0.70)
            } else { accent };
            surface(background, radius)
        }).into()
    }
}

pub(super) fn avatar(label: String, background: Color, diameter: f32, font_size: f32) -> Element<'static, Message> {
    container(text(label).color(WHITE).font(medium()).size(font_size))
        .center_x(diameter)
        .center_y(diameter)
        .style(move |_| surface(background, diameter / 2.0))
        .into()
}

pub(super) fn pill<'a>(label: impl Into<String>, background: Color, foreground: Color) -> Element<'a, Message> {
    container(text(label.into()).size(13).color(foreground))
        .padding([5, 10])
        .style(move |_| surface(background, 13.0))
        .into()
}

pub(super) fn border(color: Color, width: f32, radius: impl Into<iced::border::Radius>) -> Border {
    Border { color, width, radius: radius.into() }
}

pub(super) fn surface(background: Color, radius: impl Into<iced::border::Radius>) -> container::Style {
    container::Style {
        background: Some(Background::Color(background)),
        border: border(Color::TRANSPARENT, 0.0, radius),
        text_color: Some(FG),
        ..Default::default()
    }
}

pub(super) fn outline_surface(radius: f32) -> container::Style {
    container::Style {
        border: border(BORDER, 1.0, radius),
        ..surface(WHITE, radius)
    }
}

pub(super) fn divider_style(theme: &Theme) -> rule::Style {
    rule::Style { color: BORDER, ..rule::default(theme) }
}

pub(super) fn quiet_style(_: &Theme, status: button::Status) -> button::Style {
    button::Style {
        background: match status {
            button::Status::Hovered | button::Status::Pressed => Some(SOFT.into()),
            _ => None,
        },
        text_color: if status == button::Status::Disabled { MUTED } else { FG },
        border: border(Color::TRANSPARENT, 0.0, 7.0),
        ..Default::default()
    }
}

pub(super) fn nav_style(active: bool, status: button::Status) -> button::Style {
    let mut style = quiet_style(&Theme::Light, status);
    style.border.radius = 23.0.into();
    if active {
        style.background = Some(ACTIVE_BG.into());
        style.text_color = ACTIVE_FG;
    }
    style
}

pub(super) fn outline_style(_: &Theme, status: button::Status) -> button::Style {
    let mut style = quiet_style(&Theme::Light, status);
    style.text_color = if status == button::Status::Disabled { MUTED } else { ACTIVE_FG };
    style.border = border(BORDER, 1.0, 7.0);
    style
}

pub(super) fn tab_style(active: bool, status: button::Status) -> button::Style {
    let mut style = quiet_style(&Theme::Light, status);
    style.text_color = if active { ACTIVE_FG } else { MUTED };
    style
}

pub(super) fn primary_style(_: &Theme, status: button::Status) -> button::Style {
    let background = match status {
        button::Status::Hovered | button::Status::Pressed => color!(0x1765cc),
        button::Status::Disabled => BORDER,
        _ => BLUE,
    };
    button::Style {
        background: Some(background.into()),
        text_color: if status == button::Status::Disabled { MUTED } else { WHITE },
        border: border(Color::TRANSPARENT, 0.0, 7.0),
        ..Default::default()
    }
}

pub(super) fn card_style(_: &Theme, status: button::Status) -> button::Style {
    let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
    button::Style {
        background: Some(WHITE.into()),
        text_color: FG,
        border: border(if hovered { color!(0xa1b9d9) } else { BORDER }, 1.0, 12.0),
        shadow: if hovered {
            Shadow { color: Color::BLACK.scale_alpha(0.10), offset: Vector::new(0.0, 3.0), blur_radius: 10.0 }
        } else {
            Shadow::default()
        },
        ..Default::default()
    }
}

pub(super) fn search_style(theme: &Theme, status: text_input::Status) -> text_input::Style {
    let mut style = text_input::default(theme, status);
    let active = matches!(status, text_input::Status::Focused { .. });
    style.background = if active { WHITE.into() } else { SOFT.into() };
    style.border = border(if active { BLUE } else { Color::TRANSPARENT }, 1.0, 8.0);
    style.placeholder = MUTED;
    style.value = FG;
    style
}

pub(super) fn coursework_row<'a>(item: &'a Coursework, course_name: Option<&'a str>) -> Element<'a, Message> {
    coursework_row_with_preview(item, course_name, true)
}

pub(super) fn coursework_row_with_preview<'a>(item: &'a Coursework, course_name: Option<&'a str>, preview: bool) -> Element<'a, Message> {
    let glyph = coursework_icon(item.kind);
    let subtitle = match course_name {
        Some(course) => format!("{course} · {}", item.kind.label()),
        None => item.kind.label().into(),
    };
    let mut body = column![
        text(&item.title).font(medium()).size(16).color(FG),
        text(subtitle).size(13).color(MUTED)
    ].spacing(4);
    if preview && !item.description.trim().is_empty() {
        body = body.push(text(truncate(&item.description, 105)).size(14).color(MUTED));
    }
    if preview && let Some(submission) = &item.submission {
        let label = grade_label(submission).unwrap_or_else(|| submission.status.label().into());
        body = body.push(text(label).size(13).color(ACTIVE_FG));
    }
    let due: Element<'_, Message> = match item.due_label.as_ref().filter(|_| preview) {
        Some(label) => pill(label, SOFT, MUTED),
        None => space().width(0).into(),
    };
    button(row![
        container(icon(glyph, ACTIVE_FG, 20)).center_x(42).center_y(42)
            .style(|_| surface(ACTIVE_BG, 21.0)),
        body.width(Fill),
        due,
        icon(Icon::Arrow, MUTED, 18),
    ].spacing(13).align_y(Alignment::Center))
        .padding(16).width(Fill).style(card_style)
        .on_press(Message::OpenItem(item.course_id, item.id.clone()))
        .into()
}
