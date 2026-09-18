use serde_json::{json, Value};
use sha2::{Digest, Sha256};

const TOOL_FRAME_BYTES: usize = 64 * 1024;
const TERMINAL_BYTES: usize = 8 * 1024;

// Independent of the raw prefix: a long stream must not erase its terminal
// usage or the public tool deltas needed to diagnose that stream.
#[derive(Default)]
pub(super) struct StreamCapture {
    frames: usize,
    bytes: usize,
    digest: Sha256,
    tool_frames: Vec<Value>,
    tool_bytes: usize,
    omitted_tool_frames: usize,
    terminal: Value,
    terminal_truncated: bool,
}

impl StreamCapture {
    pub(super) fn observe(&mut self, safe_frame: &Value) {
        let encoded = safe_frame.to_string();
        let index = self.frames;
        self.frames = self.frames.saturating_add(1);
        self.bytes = self.bytes.saturating_add(encoded.len());
        self.digest.update(encoded.as_bytes());
        self.digest.update(b"\n");
        let mut tools = Vec::new();
        let mut finished = Vec::new();
        if let Some(choices) = safe_frame.get("choices").and_then(Value::as_array) {
            for choice in choices {
                if let Some(calls) = choice.pointer("/delta/tool_calls") {
                    if !calls.is_null() {
                        tools.push(
                            json!({"index":choice.get("index"), "delta":{"tool_calls":calls}}),
                        );
                    }
                }
                if let Some(reason) = choice.get("finish_reason").filter(|v| !v.is_null()) {
                    finished.push(json!({"index":choice.get("index"), "finish_reason":reason}));
                }
            }
        }
        if !tools.is_empty() {
            let record = json!({"frame_index":index, "choices":tools});
            let size = record.to_string().len();
            if self.tool_bytes.saturating_add(size) <= TOOL_FRAME_BYTES {
                self.tool_bytes += size;
                self.tool_frames.push(record);
            } else {
                self.omitted_tool_frames = self.omitted_tool_frames.saturating_add(1);
            }
        }
        let mut terminal = self.terminal.clone();
        if !finished.is_empty() {
            terminal["choices"] = json!(finished);
        }
        if let Some(usage) = safe_frame.get("usage").filter(|v| !v.is_null()) {
            terminal["usage"] = usage.clone();
        }
        if terminal.to_string().len() <= TERMINAL_BYTES {
            self.terminal = terminal;
        } else {
            self.terminal_truncated = true;
        }
    }

    pub(super) fn record(&self, retained_raw_frames: usize, stream_complete: bool) -> Value {
        json!({
            "record_type":"public_stream_evidence",
            "schema_version":1,
            "source":"sanitized_provider_frames",
            "projection":"tool_deltas_and_terminal_fields",
            "stream_complete":stream_complete,
            "source_frames":self.frames,
            "source_bytes":self.bytes,
            "source_sha256":format!("{:x}", self.digest.clone().finalize()),
            "raw_prefix_truncated":retained_raw_frames < self.frames,
            "tool_frames":self.tool_frames,
            "omitted_tool_frames":self.omitted_tool_frames,
            "tool_frames_complete":self.omitted_tool_frames == 0 && stream_complete,
            "terminal":self.terminal,
            "terminal_truncated":self.terminal_truncated,
        })
    }
}

#[cfg(test)]
#[path = "stream_capture_tests.rs"]
mod tests;
