use std::collections::HashSet;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::Duration;

use crate::{events, output, task};

pub(crate) struct ChatBackgroundFollowers {
    output_tx: Sender<String>,
    completion_tx: Sender<(String, u64)>,
    completion_rx: Receiver<(String, u64)>,
    active_task_ids: HashSet<String>,
}

impl ChatBackgroundFollowers {
    pub(crate) fn new(output_tx: Sender<String>) -> Self {
        let (completion_tx, completion_rx) = mpsc::channel();
        Self {
            output_tx,
            completion_tx,
            completion_rx,
            active_task_ids: HashSet::new(),
        }
    }

    pub(crate) fn start(&mut self, base_url: &str, key: &str, task_id: &str, cursor: u64) {
        if !self.active_task_ids.insert(task_id.to_string()) {
            return;
        }
        let base_url = base_url.to_string();
        let key = key.to_string();
        let task_id = task_id.to_string();
        let output_tx = self.output_tx.clone();
        let completion_tx = self.completion_tx.clone();
        std::thread::spawn(move || {
            let cursor = follow_until_settled(&base_url, &key, &task_id, cursor, &output_tx);
            let _ = completion_tx.send((task_id, cursor));
        });
    }

    pub(crate) fn reap(&mut self) -> Vec<(String, u64)> {
        let mut completed = Vec::new();
        while let Ok((task_id, cursor)) = self.completion_rx.try_recv() {
            self.active_task_ids.remove(&task_id);
            completed.push((task_id, cursor));
        }
        completed
    }
}

fn follow_until_settled(
    base_url: &str,
    key: &str,
    task_id: &str,
    initial_cursor: u64,
    output_tx: &Sender<String>,
) -> u64 {
    let mut cursor = initial_cursor;
    let mut consecutive_errors = 0_u8;
    let mut presentation = crate::assistant_presentation::AssistantPresentationReducer::default();
    let mut presentation_line_open = false;
    loop {
        let followed = events::follow_task_events_with_timeout(
            base_url,
            key,
            task_id,
            cursor,
            Some(Duration::from_secs(30)),
            |raw_event| {
                if let Some(seq) = events::task_event_seq(raw_event) {
                    cursor = cursor.max(seq);
                }
                if let Some(event) = crate::assistant_presentation::decode(raw_event)? {
                    match presentation.apply(event)? {
                        crate::assistant_presentation::PresentationUpdate::Delta(content) => {
                            presentation_line_open = !content.ends_with('\n');
                            let _ = output_tx.send(content);
                        }
                        crate::assistant_presentation::PresentationUpdate::Completed
                        | crate::assistant_presentation::PresentationUpdate::Aborted => {
                            if presentation_line_open {
                                let _ = output_tx.send("\n".to_string());
                                presentation_line_open = false;
                            }
                        }
                        crate::assistant_presentation::PresentationUpdate::Started
                        | crate::assistant_presentation::PresentationUpdate::Replaced
                        | crate::assistant_presentation::PresentationUpdate::Duplicate
                        | crate::assistant_presentation::PresentationUpdate::Superseded => {}
                    }
                } else if let Some(line) = events::live_task_event_output_line(
                    raw_event,
                    events::LiveEventOutputMode::Compact,
                    &events::EventFilters::default(),
                )? {
                    let _ = output_tx.send(format!("{line}\n"));
                }
                Ok(!events::task_event_is_terminal(raw_event)
                    && !events::task_event_is_background(raw_event))
            },
        );
        match followed {
            Ok(()) => match task::get_task_status(base_url, key, task_id) {
                Ok(status) if status.is_terminal() || status.is_background_waiting() => {
                    let presentation_matches =
                        presentation.completed_matches(status.result_text.as_deref());
                    let mut lines = if presentation_matches {
                        output::task_status_lines_without_result(
                            &status,
                            false,
                            &events::EventFilters::default(),
                        )
                    } else {
                        output::task_status_lines(&status, false, &events::EventFilters::default())
                    };
                    if let Some(error) = status.error_text.as_deref() {
                        lines.push(format!("error: {error}"));
                    }
                    let _ = output_tx.send(format!("{}\n", lines.join("\n")));
                    return cursor;
                }
                Ok(_) => consecutive_errors = 0,
                Err(error) => {
                    consecutive_errors = consecutive_errors.saturating_add(1);
                    if consecutive_errors >= 5 {
                        let _ = output_tx.send(format!(
                            "error_code=chat_background_follow_failed task_id={task_id} detail={error}\n"
                        ));
                        return cursor;
                    }
                }
            },
            Err(error) if events::task_event_stream_timed_out(&error) => {
                consecutive_errors = 0;
            }
            Err(error) => {
                consecutive_errors = consecutive_errors.saturating_add(1);
                if consecutive_errors >= 5 {
                    let _ = output_tx.send(format!(
                        "error_code=chat_background_follow_failed task_id={task_id} detail={error}\n"
                    ));
                    return cursor;
                }
                std::thread::sleep(Duration::from_millis(250));
            }
        }
    }
}

pub(crate) fn output_channel() -> (Sender<String>, Receiver<String>) {
    mpsc::channel()
}

#[cfg(test)]
#[path = "chat_background_tests.rs"]
mod tests;
