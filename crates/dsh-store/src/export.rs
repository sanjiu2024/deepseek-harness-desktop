//! Session export.
//!
//! A transcript is written the way a person reads it, not as a JSON dump — the log already
//! *is* a JSON dump, so exporting one again would achieve nothing.

use crate::SessionHistory;

/// Render a stored session as Markdown.
pub fn render_markdown(history: &SessionHistory) -> String {
    let mut out = String::new();
    out.push_str("# DeepSeek Harness 会话\n\n");
    out.push_str("- 会话 id：`");
    out.push_str(&history.header.id);
    out.push_str("`\n");
    if let Some(cwd) = &history.header.cwd {
        out.push_str("- 工作目录：`");
        out.push_str(cwd);
        out.push_str("`\n");
    }
    out.push_str("- 事件数：");
    out.push_str(&history.events.len().to_string());
    out.push_str("\n\n");

    for event in &history.events {
        match event.kind.as_str() {
            "user/message" => {
                if let Some(text) = message_text(&event.data) {
                    out.push_str("## 你\n\n");
                    out.push_str(&text);
                    out.push_str("\n\n");
                }
            }
            "assistant/message" => {
                let payload = event.data.get("message").unwrap_or(&event.data);
                if let Some(text) = message_text(payload) {
                    out.push_str("## 助手\n\n");
                    out.push_str(&text);
                    out.push_str("\n\n");
                }
            }
            "tool/call" => {
                let name = event
                    .data
                    .get("name")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("tool");
                let arguments = event
                    .data
                    .get("arguments")
                    .map(ToString::to_string)
                    .unwrap_or_default();
                out.push_str("- 调用 `");
                out.push_str(name);
                out.push_str("` `");
                out.push_str(&truncate(&arguments, ARGUMENT_PREVIEW));
                out.push_str("`\n");
                // The call id is what pairs a result with its call; without it the transcript
                // shows a question and never the answer.
                if let Some(call_id) = event.data.get("callId").and_then(serde_json::Value::as_str)
                {
                    out.push_str("  <!-- ");
                    out.push_str(call_id);
                    out.push_str(" -->\n");
                }
            }
            "tool/result" => {
                // Results were missing from the transcript entirely: the export listed what the
                // model asked for and never what came back, which reads like a plan nobody ran.
                if let Some(text) = message_text(event.data.get("message").unwrap_or(&event.data)) {
                    let body = truncate(&text, RESULT_PREVIEW);
                    out.push_str("  ```\n");
                    for line in body.lines() {
                        out.push_str("  ");
                        out.push_str(line);
                        out.push('\n');
                    }
                    out.push_str("  ```\n");
                }
            }
            _ => {}
        }
    }
    out
}

/// How much of a tool's arguments to keep. The arguments are usually a command line; a
/// serialised megabyte of JSON helps nobody read the transcript.
const ARGUMENT_PREVIEW: usize = 300;

/// How much of a tool's output to keep.
///
/// A transcript is for reading, not for archival — the log is the archive. Truncating keeps an
/// export openable in a text editor after a session with a few hundred tool calls.
const RESULT_PREVIEW: usize = 2000;

/// Cut `text` to `limit` **characters**, marking that something was removed.
fn truncate(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        return text.to_owned();
    }
    let cut: String = text.chars().take(limit).collect();
    format!("{cut}…（截断）")
}

/// Best-effort text: a plain string, or the concatenated text blocks of a message.
fn message_text(data: &serde_json::Value) -> Option<String> {
    let content = data.get("content")?;
    if let Some(text) = content.as_str() {
        return (!text.trim().is_empty()).then(|| text.to_owned());
    }
    // A reasoning block carries a  field too. Filtering on the key alone would
    // put the model's thinking into the saved transcript, which is not what the user asked
    // to export.
    let joined = content
        .as_array()?
        .iter()
        .filter(|block| block.get("type").and_then(serde_json::Value::as_str) == Some("text"))
        .filter_map(|block| block.get("text").and_then(serde_json::Value::as_str))
        .collect::<Vec<_>>()
        .join("\n\n");
    (!joined.trim().is_empty()).then_some(joined)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn history(raw: &str) -> SessionHistory {
        let mut lines = raw.lines();
        let header: crate::SessionHeader =
            serde_json::from_str(lines.next().expect("header")).expect("header json");
        let events: Vec<crate::SessionEvent> = lines
            .map(|line| serde_json::from_str(line).expect("event json"))
            .collect();
        SessionHistory {
            total: events.len(),
            truncated: false,
            header,
            events,
        }
    }

    #[test]
    fn the_transcript_reads_like_a_conversation() {
        let markdown = render_markdown(&history(
            r#"{"type":"session","version":4,"id":"abc","createdAt":1,"cwd":"D:\\repo","isSeeded":false,"delegationDepth":0}
{"type":"user/message","seq":0,"time":2,"data":{"content":[{"type":"text","text":"问题"}]}}
{"type":"assistant/message","seq":1,"time":3,"data":{"message":{"role":"assistant","content":[{"type":"text","text":"回答"}]}}}
{"type":"tool/call","seq":2,"time":4,"data":{"name":"pwsh","arguments":"{}"}}"#,
        ));

        assert!(markdown.contains("## 你\n\n问题"), "{markdown}");
        assert!(markdown.contains("## 助手\n\n回答"), "{markdown}");
        assert!(markdown.contains("调用 `pwsh`"), "{markdown}");
        assert!(markdown.contains("D:\\repo"), "{markdown}");
    }

    #[test]
    fn reasoning_blocks_do_not_leak_into_the_transcript() {
        // Thinking is useful live and noise in a saved transcript.
        let markdown = render_markdown(&history(
            r#"{"type":"session","version":4,"id":"a","createdAt":1,"isSeeded":false,"delegationDepth":0}
{"type":"assistant/message","seq":0,"time":2,"data":{"message":{"content":[{"type":"reasoning","text":"想了很多"},{"type":"text","text":"结论"}]}}}"#,
        ));
        assert!(!markdown.contains("想了很多"), "{markdown}");
        assert!(markdown.contains("结论"));
    }
}

#[cfg(test)]
mod tool_tests {
    use super::*;

    fn history(raw: &str) -> SessionHistory {
        let mut lines = raw.lines();
        let header: crate::SessionHeader =
            serde_json::from_str(lines.next().expect("header")).expect("header json");
        let events: Vec<crate::SessionEvent> = lines
            .map(|line| serde_json::from_str(line).expect("event json"))
            .collect();
        SessionHistory {
            total: events.len(),
            truncated: false,
            header,
            events,
        }
    }

    #[test]
    fn a_tool_result_appears_under_its_call() {
        // The first version listed calls and never results: the transcript read like a plan
        // nobody ran.
        let markdown = render_markdown(&history(
            r#"{"type":"session","version":4,"id":"a","createdAt":1,"isSeeded":false,"delegationDepth":0}
{"type":"tool/call","seq":0,"time":2,"data":{"callId":"c1","name":"pwsh","arguments":"{\"command\":\"ls\"}"}}
{"type":"tool/result","seq":1,"time":3,"data":{"message":{"toolCallId":"c1","content":[{"type":"text","text":"file-a.txt\nfile-b.txt"}]}}}"#,
        ));

        assert!(markdown.contains("调用 `pwsh`"), "{markdown}");
        assert!(
            markdown.contains("file-a.txt"),
            "the output is in the transcript: {markdown}"
        );
    }

    #[test]
    fn a_huge_output_is_truncated_rather_than_dumped() {
        let huge = "x".repeat(5000);
        let markdown = render_markdown(&history(&format!(
            r#"{{"type":"session","version":4,"id":"a","createdAt":1,"isSeeded":false,"delegationDepth":0}}
{{"type":"tool/result","seq":0,"time":2,"data":{{"message":{{"content":[{{"type":"text","text":"{huge}"}}]}}}}}}"#
        )));

        assert!(markdown.contains("（截断）"));
        assert!(
            markdown.chars().count() < 4000,
            "an export should stay openable in an editor"
        );
    }
}
