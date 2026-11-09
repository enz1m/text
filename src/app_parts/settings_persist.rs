//! Settings persistence.

use super::*;

impl TextApp {
    pub(crate) fn save_settings(&mut self) -> Task<Message> {
        if !self.settings_persistence_enabled {
            return Task::none();
        }
        // A wrapped generation would let a stale `SettingsSaved` clear fresh
        // debt; on exhaustion settings apply in memory but are not saved.
        let Some(generation) = self.settings_generation.checked_add(1) else {
            self.settings_retry_blocked = true;
            self.picker.failure = Some(enzui_core::localization::text(
                "text-settings-save-generations-are-exhausted-restart-the-ce43f9",
            ));
            return Task::none();
        };
        self.settings_generation = generation;
        self.settings_debt = Some((self.settings_generation, self.model.settings.clone()));
        self.settings_retry_blocked = false;
        self.dispatch_settings_debt()
    }

    pub(crate) fn queue_settings_save(&mut self) -> Task<Message> {
        if !self.settings_persistence_enabled {
            return Task::none();
        }
        self.settings_retry_blocked = false;
        if self
            .settings_inflight
            .as_ref()
            .is_some_and(|(_, settings)| settings == &self.model.settings)
            || self
                .settings_debt
                .as_ref()
                .is_some_and(|(_, settings)| settings == &self.model.settings)
        {
            return self.dispatch_settings_debt();
        }
        self.save_settings()
    }

    pub(crate) fn dispatch_settings_debt(&mut self) -> Task<Message> {
        if !self.settings_persistence_enabled
            || self.settings_inflight.is_some()
            || self.settings_retry_blocked
        {
            return Task::none();
        }
        let Some((generation, settings)) = self.settings_debt.take() else {
            return Task::none();
        };
        let path = self.settings_config_path.clone();
        let task_settings = settings.clone();
        let task = self
            .blocking_tasks
            .submit(generation, move |_| task_settings.save_to(&path));
        self.settings_inflight = Some((generation, settings));
        Task::perform(task, move |settlement| Message::SettingsSaved {
            generation,
            result: settlement.into_result(),
        })
    }

    pub(crate) fn settings_saved(
        &mut self,
        generation: u64,
        result: Result<bool, BlockingTaskError>,
    ) -> Task<Message> {
        let Some((active_generation, settings)) = self.settings_inflight.take() else {
            return Task::none();
        };
        if active_generation != generation {
            self.settings_inflight = Some((active_generation, settings));
            return Task::none();
        }
        match result {
            Ok(true) => {}
            Ok(false) => {
                self.settings_debt = Some((generation, settings));
                self.settings_retry_blocked = true;
                self.picker.failure = Some(enzui_core::localization::text(
                    "text-settings-were-not-saved-retry-the-settings-c0b9cf",
                ));
                self.veto_close_for_durability(enzui_core::localization::text(
                    "text-close-was-cancelled-because-settings-could-not-189447",
                ));
            }
            Err(BlockingTaskError::Busy) => {
                self.settings_debt = Some((generation, settings));
                self.blocking_retry_pending = true;
            }
            Err(BlockingTaskError::Cancelled)
                if self
                    .settings_debt
                    .as_ref()
                    .is_some_and(|(new_generation, newer)| {
                        *new_generation > generation && newer == &settings
                    }) => {}
            Err(
                BlockingTaskError::Cancelled
                | BlockingTaskError::Shutdown
                | BlockingTaskError::Panicked,
            ) => {
                self.settings_debt = Some((generation, settings));
                self.settings_retry_blocked = true;
                self.picker.failure = Some(enzui_core::localization::text(
                    "text-settings-persistence-stopped-retry-the-settings-change-78489f",
                ));
                self.veto_close_for_durability(enzui_core::localization::text(
                    "text-close-was-cancelled-because-settings-persistence-stopped-a070f0",
                ));
            }
            Err(_) => {
                self.settings_debt = Some((generation, settings));
                self.settings_retry_blocked = true;
                self.picker.failure = Some(enzui_core::localization::text(
                    "text-settings-persistence-stopped-retry-the-settings-change-78489f",
                ));
                self.veto_close_for_durability(enzui_core::localization::text(
                    "text-close-was-cancelled-because-settings-persistence-stopped-a070f0",
                ));
            }
        }
        Task::none()
    }
}
