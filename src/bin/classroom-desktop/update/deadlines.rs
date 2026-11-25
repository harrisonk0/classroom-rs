use super::*;

#[derive(Clone)]
pub(in crate::desktop) enum DeadlinesMessage {
    SetTodoFilter(TodoFilter),
    SetWorkFilter(WorkFilter),
    FilterTodoClass(ClassFilterOption),
    ToggleTodoGroup(TodoGroup),
    CalendarShift(i32),
    CalendarToday,
    ClockTick(DateTime<Local>),
    CalendarFilter(ClassFilterOption),
}

impl App {
    pub(super) fn update_deadlines(&mut self, message: DeadlinesMessage) -> Task<Message> {
        match message {
            DeadlinesMessage::SetTodoFilter(filter) => self.deadlines.todo_filter = filter,
            DeadlinesMessage::SetWorkFilter(filter) => self.student_work.work_filter = filter,
            DeadlinesMessage::FilterTodoClass(class) => self.deadlines.todo_class = class.id,
            DeadlinesMessage::ToggleTodoGroup(group) => {
                let key = (self.deadlines.todo_filter, group);
                if !self.deadlines.collapsed_todo_groups.remove(&key) {
                    self.deadlines.collapsed_todo_groups.insert(key);
                }
            }
            DeadlinesMessage::CalendarShift(direction) => {
                if let Some(monday) = self.deadlines.calendar_monday.checked_add_signed(TimeDelta::weeks(direction.into()))
                    && monday.checked_add_days(Days::new(6)).is_some()
                {
                    self.deadlines.calendar_monday = monday;
                }
            }
            DeadlinesMessage::ClockTick(now) => self.deadlines.now = now,
            DeadlinesMessage::CalendarToday => self.deadlines.calendar_monday = monday_of(Local::now().date_naive()),
            DeadlinesMessage::CalendarFilter(class) => self.deadlines.calendar_class = class.id,
        }
        self.finish_update()
    }
}
