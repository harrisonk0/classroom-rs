use super::*;

impl App {
    pub(super) fn update_deadlines(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::SetTodoFilter(filter) => self.todo_filter = filter,
            Message::SetWorkFilter(filter) => self.work_filter = filter,
            Message::FilterTodoClass(class) => self.todo_class = class.id,
            Message::ToggleTodoGroup(group) => {
                let key = (self.todo_filter, group);
                if !self.collapsed_todo_groups.remove(&key) {
                    self.collapsed_todo_groups.insert(key);
                }
            }
            Message::CalendarShift(direction) => {
                if let Some(monday) = self.calendar_monday.checked_add_signed(TimeDelta::weeks(direction.into()))
                    && monday.checked_add_days(Days::new(6)).is_some()
                {
                    self.calendar_monday = monday;
                }
            }
            Message::CalendarToday => self.calendar_monday = monday_of(Local::now().date_naive()),
            Message::CalendarFilter(class) => self.calendar_class = class.id,
            _ => {}
        }
        self.finish_update()
    }
}
