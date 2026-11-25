use super::*;

#[derive(Clone, Debug)]
pub(super) enum OfflineMessage {
    Save,
    Open,
    Delete,
    Finished(Result<OfflineResult, String>),
}

#[derive(Clone, Debug)]
pub(super) enum OfflineResult {
    Saved,
    Opened(Arc<Snapshot>),
    Deleted,
}

impl App {
    pub(super) fn update_offline(&mut self, message: OfflineMessage) -> Task<Message> {
        if let OfflineMessage::Finished(result) = message {
            self.offline_pending = false;
            self.error = None;
            self.notice = None;
            match result {
                Ok(OfflineResult::Saved) => self.notice = Some("Offline copy saved on this Mac.".into()),
                Ok(OfflineResult::Opened(snapshot)) => {
                    self.install_snapshot(Arc::unwrap_or_clone(snapshot));
                    self.screen = Screen::Home;
                    self.demo = false;
                }
                Ok(OfflineResult::Deleted) => {
                    if let Some(session) = &self.session
                        && self.snapshot.as_ref().is_some_and(|data| data.source == SnapshotSource::Cache)
                    {
                        self.install_snapshot(session.snapshot.clone());
                    }
                    self.notice = Some("Offline copy removed from this Mac.".into());
                }
                Err(error) => self.error = Some(error),
            }
            return self.finish_update();
        }

        if self.has_pending_request() { return Task::none(); }
        let Some(session) = self.session.clone() else { return Task::none(); };
        let Some(path) = cache_path() else {
            self.error = Some("Offline storage isn't available.".into());
            return Task::none();
        };
        let snapshot = if matches!(message, OfflineMessage::Save) {
            let Some(snapshot) = self.snapshot.clone() else { return Task::none(); };
            Some(snapshot)
        } else {
            None
        };
        self.offline_pending = true;
        self.error = None;
        self.notice = Some(match message {
            OfflineMessage::Save => "Saving the offline copy…",
            OfflineMessage::Open => "Opening the offline copy…",
            OfflineMessage::Delete => "Removing the offline copy…",
            OfflineMessage::Finished(_) => unreachable!(),
        }.into());
        Task::perform(async move {
            tokio::task::spawn_blocking(move || match message {
                OfflineMessage::Save => desktop_data::save_cache(path, &session.account, &snapshot.unwrap())
                    .map(|()| OfflineResult::Saved).map_err(|_| "The offline copy couldn't be saved."),
                OfflineMessage::Open => desktop_data::load_cache(path, &session.account)
                    .map_err(|_| "The offline copy couldn't be opened.")?
                    .map(|snapshot| OfflineResult::Opened(Arc::new(snapshot)))
                    .ok_or("There is no saved copy for that email."),
                OfflineMessage::Delete => desktop_data::clear_cache(path, &session.account)
                    .map(|()| OfflineResult::Deleted).map_err(|_| "The offline copy couldn't be removed."),
                OfflineMessage::Finished(_) => unreachable!(),
            }).await.map_err(|_| "The offline storage task stopped unexpectedly.".to_string())?
                .map_err(str::to_owned)
        }, |result| Message::Offline(OfflineMessage::Finished(result)))
    }
}
