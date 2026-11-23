//! Leaving a drawing whose steps the room has not taken
//! (`SPEC-collab-offline-sync.md` §5.6).
//!
//! Quitting, or closing the session's drawing, while steps wait in the
//! queue asks once whether to go. It is asked last, after the questions
//! about block edits and unsaved changes, at the point where the way out
//! would otherwise go on. A script cannot answer a dialog: it is told on
//! the command line, and goes on.

use iced::Task;

use crate::app::{Message, ModalKind, OpenCADStudio, PendingClose};
use crate::ui::window::pid_collab::unsynced_prompt;

/// What the person was doing when the question came up, so that 退出 (关闭)
/// can go on with it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Leaving {
    Quit,
    /// Closing the drawing of tab `tab`; `discard` when its unsaved changes
    /// were let go first.
    Close {
        tab: u64,
        discard: bool,
    },
}

/// The question while it is up: what it asks before, and how many steps it
/// says are queued.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct UnsyncedQuestion {
    pub(crate) leaving: Leaving,
    pub(crate) steps: usize,
}

impl OpenCADStudio {
    /// How many steps `leaving` would leave the room without: the session's
    /// queue, on a quit or on closing the session's drawing. `None` when
    /// there are none, or no collaboration is configured.
    fn steps_left_behind(&self, leaving: Leaving) -> Option<usize> {
        if !self.collab.wanted() {
            return None;
        }
        if let Leaving::Close { tab, .. } = leaving {
            if self.collab.document != Some(tab) {
                return None;
            }
        }
        Some(self.collab.outbox.len()).filter(|&steps| steps > 0)
    }

    /// Before `leaving`: the task that puts the question up, or `None` to go
    /// on — nothing is left behind, the question has been answered, or a
    /// script is running, which is told on the command line instead.
    pub(in crate::app) fn ask_before_leaving(&mut self, leaving: Leaving) -> Option<Task<Message>> {
        if self.leaving_anyway {
            return None;
        }
        let steps = self.steps_left_behind(leaving)?;
        if self.script_running {
            self.command_line.push_warning(&unsynced_prompt(steps));
            return None;
        }
        self.unsynced_question = Some(UnsyncedQuestion { leaving, steps });
        self.active_modal = Some(ModalKind::CollabUnsynced);
        Some(self.raise_main_window())
    }

    /// The end of a quit: the question about steps not synced, or the exit.
    pub(in crate::app) fn quit_unless_unsynced(&mut self) -> Task<Message> {
        match self.ask_before_leaving(Leaving::Quit) {
            Some(ask) => ask,
            None => self.exit_app(),
        }
    }

    /// 退出 / 关闭: go on with what the question was asked about, without
    /// asking it again.
    pub(in crate::app) fn on_unsynced_leave(&mut self) -> Task<Message> {
        let Some(question) = self.unsynced_question.take() else {
            return Task::none();
        };
        let closed = self.close_unsynced_question();
        self.leaving_anyway = true;
        let go = match question.leaving {
            Leaving::Quit => self.exit_app(),
            Leaving::Close { tab, discard: true } => {
                self.pending_close = Some(PendingClose::Tab(tab));
                self.on_unsaved_dialog_discard()
            }
            Leaving::Close {
                tab,
                discard: false,
            } => match self.tabs.iter().position(|open| open.id == tab) {
                Some(idx) => {
                    let close = self.on_tab_close(idx);
                    Task::batch([close, self.continue_tab_close_queue()])
                }
                None => self.continue_tab_close_queue(),
            },
        };
        self.leaving_anyway = false;
        Task::batch([closed, go])
    }

    /// 取消: stay. Drawings a close-all still had to close stay open, as
    /// they do on 取消 in the question about unsaved changes.
    pub(in crate::app) fn on_unsynced_cancel(&mut self) -> Task<Message> {
        self.unsynced_question = None;
        self.pending_tab_closes.clear();
        self.close_unsynced_question()
    }

    fn close_unsynced_question(&mut self) -> Task<Message> {
        if self.focus_dialog(ModalKind::CollabUnsynced) {
            self.active_modal = None;
            self.reset_modal_geometry();
        }
        Task::none()
    }
}
