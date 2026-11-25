use super::*;
use iced::widget::column;

impl App {
    pub(super) fn your_work(&self, id: i64) -> Element<'_, Message> {
        let items = self.snapshot.as_ref().and_then(|snapshot| snapshot.coursework_for(id));
        let filters = row(WorkFilter::ALL.into_iter().map(|filter| {
            let active = self.work_filter == filter;
            button(filter.label()).padding([8, 12]).style(move |_, status| nav_style(active, status))
                .on_press(Message::SetWorkFilter(filter)).into()
        })).spacing(6).wrap().vertical_spacing(6);
        let mut body = column![
            button("‹ Classwork").padding([7, 9]).style(quiet_style).on_press(Message::SetTab(Tab::Classwork)),
            text("Your work").font(medium()).size(21).color(FG),
            filters,
        ].spacing(12);
        if self.snapshot.as_ref().is_some_and(|data| data.source == SnapshotSource::Cache) {
            body = body.push(text("Submission status and grades are available when connected.").size(14).color(MUTED));
        }
        if let Some(Some(grade)) = self.overall_grades.get(&id) {
            let method = match grade.calculation {
                GradeCalculation::TotalPoints => "Total points",
                GradeCalculation::WeightedCategories => "Weighted categories",
            };
            body = body.push(container(column![
                text(format!("Overall grade · {:.2}%", grade.percent)).size(16).font(medium()).color(FG),
                text(method).size(14).color(MUTED),
            ].spacing(5)).padding(12).width(Fill).style(|_| outline_surface(8.0)));
        } else if let Some(status) = self.page_read_status(
            PageRead::Detail(DetailRead::OverallGrade(id)),
            self.overall_grades.contains_key(&id), "Checking for an overall grade…",
        ) {
            body = body.push(status);
        }
        let status = match items {
            None => self.page_read_status(PageRead::Coursework(id), false, "Loading classwork…"),
            Some(items) if items.iter().any(has_submission) => self.page_read_status(
                PageRead::Detail(DetailRead::Submissions(id)),
                self.submissions_checked.contains(&id),
                "Loading your work…",
            ),
            _ => None,
        };
        let pending = status.is_some();
        if let Some(status) = status {
            body = body.push(status);
        }
        let mut visible = false;
        for item in items.unwrap_or_default().iter()
            .filter(|item| has_submission(item) && self.work_filter.includes(item))
        {
            visible = true;
            body = body.push(coursework_row(item, None));
        }
        if !visible && !pending {
            body = body.push(empty_panel(
                Icon::Check, "No matching work", "There are no assignments or questions that match this filter.",
            ));
        }
        body.width(Fill).into()
    }

    pub(super) fn visible_rubric(&self, course: i64, item: i64) -> Option<&Rubric> {
        if self.screen != Screen::Item(course) || self.item_id.as_deref().and_then(|id| id.parse().ok()) != Some(item) {
            return None;
        }
        self.rubrics.get(&(course, item))?.as_ref()
    }

    pub(super) fn assignment_rubric<'a>(&'a self, course: i64, item_id: i64, item: &'a Coursework) -> Option<Element<'a, Message>> {
        let key = (course, item_id);
        if let Some(status) = self.page_read_status(
            PageRead::Detail(DetailRead::Rubric(course, item_id)), self.rubrics.contains_key(&key), "Checking for a rubric…",
        ) {
            return Some(status);
        }
        let rubric = self.visible_rubric(course, item_id)?;
        let mut panel = column![button(row![
            text("Rubric").size(19).font(medium()).color(FG),
            space().width(Fill),
            text(if self.rubric_open { "⌃" } else { "⌄" }).size(17).color(MUTED),
        ].spacing(8).align_y(Alignment::Center)).style(quiet_style).width(Fill)
            .padding([8, 10]).on_press(Message::ToggleRubric(course, item_id))].spacing(9);
        if self.rubric_open {
            if let Some(title) = &rubric.title {
                panel = panel.push(text(title).size(16).color(FG));
            }
            if !rubric.criteria.is_empty() {
                let all_open = self.open_criteria.len() == rubric.criteria.len();
                panel = panel.push(button(if all_open { "Collapse criteria" } else { "Expand criteria" })
                    .style(quiet_style).padding([7, 10])
                    .on_press(Message::SetRubricCriteria(course, item_id, !all_open)));
            }
            for (index, criterion) in rubric.criteria.iter().enumerate() {
                let score = item.submission.as_ref().and_then(|submission| {
                    submission.rubric_scores.get(criterion.id.as_ref()?)
                });
                let open = self.open_criteria.contains(&index);
                let mut heading = row![text(&criterion.title).font(medium()).size(15).width(Fill)
                    .wrapping(text::Wrapping::WordOrGlyph)].spacing(8).align_y(Alignment::Center);
                if let Some(points) = score.and_then(|score| score.points) {
                    heading = heading.push(pill(format!("{points} points"), ACTIVE_BG, ACTIVE_FG));
                }
                if !open && let Some(rating_id) = score.and_then(|score| score.rating_id.as_ref())
                    && let Some(rating) = criterion.ratings.iter().find(|rating| rating.id.as_ref() == Some(rating_id))
                    && !rating.title.is_empty()
                {
                    heading = heading.push(text(&rating.title).size(13).color(ACTIVE_FG));
                }
                heading = heading.push(text(if open { "⌃" } else { "⌄" }).size(17).color(MUTED));
                let mut section = column![button(heading).width(Fill).padding([8, 10]).style(quiet_style)
                    .on_press(Message::ToggleRubricCriterion(course, item_id, index))].spacing(6);
                if open {
                    if !criterion.description.is_empty() {
                        section = section.push(text(&criterion.description).size(14).color(FG)
                            .width(Fill).wrapping(text::Wrapping::WordOrGlyph));
                    }
                    for (rank, rating) in criterion.ratings.iter().enumerate() {
                        let selected = rating.id.as_ref().is_some_and(|id| {
                            score.and_then(|score| score.rating_id.as_ref()) == Some(id)
                        });
                        let title = if rating.title.is_empty() { format!("Level {}", rank + 1) } else { rating.title.clone() };
                        let mut level_heading = row![text(title).font(medium()).size(14).width(Fill)]
                            .spacing(8).align_y(Alignment::Center);
                        if let Some(points) = rating.points {
                            level_heading = level_heading.push(text(format!("{points} points")).size(13).color(MUTED));
                        }
                        if selected {
                            level_heading = level_heading.push(pill("Selected level", ACTIVE_BG, ACTIVE_FG));
                        }
                        let mut level = column![level_heading].spacing(6);
                        if !rating.description.is_empty() {
                            level = level.push(text(&rating.description).size(14).width(Fill)
                                .wrapping(text::Wrapping::WordOrGlyph));
                        }
                        section = section.push(container(level).padding(8).width(Fill)
                            .style(move |_| surface(if selected { ACTIVE_BG } else { SOFT }, 7.0)));
                    }
                }
                panel = panel.push(container(section).width(Fill).padding(8).style(|_| outline_surface(8.0)));
            }
        }
        Some(container(panel).width(Fill).padding(9).style(|_| outline_surface(9.0)).into())
    }
}
