// Implements `oxidant_core::UiBridge` for the GUI host.
//
// `GuiBridge` is created once per agent-loop spawn in
// `chat_input::spawn_agent_inner` and threaded into the tool
// dispatch via `ToolContext.ui`. The bridge posts pending questions
// into the matching window's `PerWindowState` and awaits the answer
// on a oneshot — the user's click fulfils it via
// `UserQuestionPanel::render`. See spec/contracts/tool.md "UiBridge"
// and spec/tools/ask-user.md.

use std::sync::{Arc, Mutex as StdMutex};

use async_trait::async_trait;

use oxidant_core::{ExplorationId, UiBridge};

use crate::app::{PendingUserQuestion, SharedState};

pub struct GuiBridge {
    pub state: Arc<StdMutex<SharedState>>,
    pub view_id: ExplorationId,
    pub egui_ctx: egui::Context,
}

#[async_trait]
impl UiBridge for GuiBridge {
    async fn ask_user(
        &self,
        question: String,
        options: Vec<String>,
        allow_freeform: bool,
    ) -> anyhow::Result<String> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        {
            let mut s = self
                .state
                .lock()
                .map_err(|_| anyhow::anyhow!("SharedState poisoned"))?;
            s.window_mut(self.view_id).pending_question = Some(PendingUserQuestion {
                question,
                options,
                allow_freeform,
                answer_tx: tx,
            });
        }
        // Wake the GUI so the modal renders this frame.
        self.egui_ctx.request_repaint();
        rx.await
            .map_err(|_| anyhow::anyhow!("user cancelled or window closed"))
    }
}
