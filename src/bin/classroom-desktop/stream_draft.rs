use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct CommentTarget {
    pub(super) course: i64,
    pub(super) item: i64,
    pub(super) id: i64,
    pub(super) audience: CommentAudience,
}


#[derive(Default)]
pub(super) struct StreamDraft {
    pub(super) body: text_editor::Content,
    pub(super) links: Vec<String>,
    pub(super) link_input: Option<String>,
    pub(super) selected_file: Option<PathBuf>,
    pub(super) files: Vec<DriveFileAttachment>,
    pub(super) styles: Vec<u8>,
    pub(super) typing_style: Option<u8>,
    pub(super) bullets: BTreeSet<usize>,
}

#[derive(Clone, Copy)]
pub(super) enum StreamFormat {
    Bold = 1,
    Italic = 2,
    Underline = 4,
    BulletList = 8,
}

impl StreamDraft {
    pub(super) fn position(&self, position: text_editor::Position) -> usize {
        let lines_before: usize = self.body.lines().take(position.line).map(|line| {
            line.text.len() + if line.ending == text_editor::LineEnding::None {
                1
            } else {
                line.ending.as_str().len()
            }
        }).sum();
        lines_before + self.body.line(position.line).map_or(0, |line| position.column.min(line.text.len()))
    }

    pub(super) fn format(&mut self, format: StreamFormat) {
        let cursor = self.body.cursor();
        let position = self.position(cursor.position);
        if matches!(format, StreamFormat::BulletList) {
            let text = self.body.text();
            let selection = cursor.selection.map_or(position, |selection| self.position(selection));
            let start = paragraph_start(&text, position.min(selection));
            let end = position.max(selection);
            let positions: Vec<_> = paragraph_starts(&text)
                .filter(|&line| line == start || (start < line && line < end)).collect();
            let remove = positions.iter().all(|line| self.bullets.contains(line));
            for line in positions {
                if remove {
                    self.bullets.remove(&line);
                } else {
                    self.bullets.insert(line);
                }
            }
            return;
        }
        let bit = format as u8;
        if let Some(selection) = cursor.selection {
            let selection = self.position(selection);
            let text = self.body.text();
            let start = position.min(selection);
            let end = position.max(selection).min(text.len());
            if start < end && text.is_char_boundary(start) && text.is_char_boundary(end) {
                self.styles.resize(text.len(), 0);
                let range = &mut self.styles[start..end];
                let enabled = range.iter().all(|style| style & bit != 0);
                for style in range {
                    *style = if enabled { *style & !bit } else { *style | bit };
                }
                self.typing_style = None;
                return;
            }
        }
        let previous = self.styles.get(position.saturating_sub(1)).copied().unwrap_or(0);
        self.typing_style = Some(self.typing_style.unwrap_or(previous) ^ bit);
    }

    pub(super) fn edit(&mut self, action: text_editor::Action) {
        if !action.is_edit() {
            if !matches!(action, text_editor::Action::Scroll { .. }) {
                self.typing_style = None;
            }
            self.body.perform(action);
            return;
        }
        let old = self.body.text();
        let cursor = self.body.cursor();
        let position = self.position(cursor.position);
        let selection = cursor.selection.map_or(position, |selection| self.position(selection));
        let start = position.min(selection);
        let end = position.max(selection);
        let paragraph = paragraph_start(&old, start);
        if matches!(action, text_editor::Action::Edit(text_editor::Edit::Enter))
            && start == end && old[paragraph..].split('\n').next().is_none_or(|text| text.trim().is_empty())
            && self.bullets.remove(&paragraph)
        {
            return;
        }
        self.body.perform(action);
        let new = self.body.text();
        if old == new {
            return;
        }
        let new_position = self.position(self.body.cursor().position);
        let limit = start.min(new_position);
        let mut prefix = 0;
        for (a, b) in old.chars().zip(new.chars()) {
            if a != b || prefix + a.len_utf8() > limit {
                break;
            }
            prefix += a.len_utf8();
        }
        let old_tail = &old[prefix..];
        let new_tail = &new[prefix..];
        let suffix_limit = old.len().saturating_sub(end).min(new.len().saturating_sub(new_position));
        let mut suffix = 0;
        for (a, b) in old_tail.chars().rev().zip(new_tail.chars().rev()) {
            if a != b || suffix + a.len_utf8() > suffix_limit {
                break;
            }
            suffix += a.len_utf8();
        }
        self.styles.resize(old.len(), 0);
        let inherited = self.typing_style.unwrap_or_else(|| {
            self.styles.get(prefix.saturating_sub(1)).copied().unwrap_or(0)
        });
        self.styles.splice(prefix..old.len() - suffix, std::iter::repeat_n(inherited, new.len() - prefix - suffix));
        self.update_bullets(&old, &new, prefix, suffix);
    }

    pub(super) fn update_bullets(&mut self, old: &str, new: &str, prefix: usize, suffix: usize) {
        if self.bullets.is_empty() {
            return;
        }
        let old_end = old.len() - suffix;
        let new_end = new.len() - suffix;
        let inherited = self.bullets.contains(&paragraph_start(old, prefix));
        self.bullets = paragraph_starts(new).filter(|&start| {
            if start <= prefix {
                self.bullets.contains(&start)
            } else if start <= new_end && (start < new_end || inherited) {
                inherited
            } else {
                self.bullets.contains(&(old_end + start - new_end))
            }
        }).collect();
    }

    pub(super) fn description_styles(&self) -> Vec<DescriptionStyle> {
        let mut previous = None;
        self.styles.iter().copied().enumerate().filter_map(|(start, bits)| {
            if previous.replace(bits) == Some(bits) {
                return None;
            }
            Some(DescriptionStyle { start, bold: bits & 1 != 0, italic: bits & 2 != 0, underline: bits & 4 != 0 })
        }).collect()
    }

    pub(super) fn pending_link(&self) -> Option<&str> {
        let url = self.link_input.as_deref()?.trim();
        (trusted_attachment_host(url).is_some() && !self.links.iter().any(|link| link == url)).then_some(url)
    }

    pub(super) fn can_send(&self, body: &str) -> bool {
        valid_stream_draft(body) && self.selected_file.is_none()
            && self.link_input.as_ref().is_none_or(|input| input.trim().is_empty())
    }
}
