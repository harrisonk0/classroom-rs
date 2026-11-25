use super::*;
use iced::widget::column;

pub(super) fn stream_editor_binding(course: i64, event: text_editor::KeyPress) -> Option<text_editor::Binding<Message>> {
    if matches!(event.status, text_editor::Status::Focused { .. }) && event.modifiers.command() {
        let format = match event.key.to_latin(event.physical_key) {
            Some('b') => Some(StreamFormat::Bold),
            Some('i') => Some(StreamFormat::Italic),
            Some('u') => Some(StreamFormat::Underline),
            _ => None,
        };
        if let Some(format) = format {
            return Some(text_editor::Binding::Custom(Message::Stream(StreamMessage::FormatStreamDraft(course, format))));
        }
    }
    text_editor::Binding::from_key_press(event)
}

pub(super) fn paragraph_start(text: &str, offset: usize) -> usize {
    text.as_bytes()[..offset.min(text.len())].iter().rposition(|&byte| byte == b'\n').map_or(0, |index| index + 1)
}

pub(super) fn paragraph_starts(text: &str) -> impl Iterator<Item = usize> + '_ {
    std::iter::once(0).chain(text.match_indices('\n').map(|(index, _)| index + 1))
}

pub(super) fn formatted_description(body: &str, styles: &[DescriptionStyle], bullets: &[usize]) -> Element<'static, Message> {
    if bullets.is_empty() {
        return formatted_inline(body, styles);
    }
    let mut paragraphs = column![].spacing(4);
    for start in paragraph_starts(body) {
        let line = body[start..].split('\n').next().unwrap_or_default().trim_end_matches('\r');
        let end = start + line.len();
        let mut inline = Vec::new();
        if let Some(previous) = styles.iter().filter(|style| style.start <= start).max_by_key(|style| style.start) {
            inline.push(DescriptionStyle { start: 0, ..*previous });
        }
        inline.extend(styles.iter().filter(|style| (start + 1..end).contains(&style.start))
            .map(|style| DescriptionStyle { start: style.start - start, ..*style }));
        let content = formatted_inline(line, &inline);
        if bullets.contains(&start) {
            paragraphs = paragraphs.push(row![text("•").size(15).style(move |theme| themed_text(theme, FG)).width(15), content].spacing(3));
        } else if line.is_empty() {
            paragraphs = paragraphs.push(space().height(15));
        } else {
            paragraphs = paragraphs.push(content);
        }
    }
    paragraphs.width(Fill).into()
}

pub(super) fn formatted_inline(body: &str, styles: &[DescriptionStyle]) -> Element<'static, Message> {
    if styles.iter().all(|style| style.is_plain()) {
        return text(body.to_owned()).size(15).style(move |theme| themed_text(theme, FG)).wrapping(text::Wrapping::WordOrGlyph).into();
    }
    let mut runs = BTreeMap::from([(0, DescriptionStyle::default())]);
    runs.extend(styles.iter().copied().filter(|style| style.start < body.len() && body.is_char_boundary(style.start))
        .map(|style| (style.start, style)));
    let mut runs = runs.into_iter().peekable();
    let mut spans: Vec<text::Span<'static, (), Font>> = Vec::new();
    while let Some((start, style)) = runs.next() {
        let end = runs.peek().map_or(body.len(), |(next, _)| *next);
        spans.push(span(body[start..end].to_owned()).font(Font {
            weight: if style.bold { Weight::Bold } else { Weight::Normal },
            style: if style.italic { iced::font::Style::Italic } else { iced::font::Style::Normal },
            ..FORMATTED_FONT
        }).underline(style.underline));
    }
    rich_text(spans).size(15).width(Fill).wrapping(text::Wrapping::WordOrGlyph).into()
}
