use super::*;

fn payload_uses_code_block(text: &str) -> bool {
    let (body, _) = telegram_text_payload(text);
    body.contains("<pre>") && body.contains("<code>")
}

#[test]
fn news_list_text_not_wrapped_as_code() {
    let text = r#"sources_ok=3 sources_failed=0 items=5
1. [techcrunch] 标题A
   🧾 Summary:
   From techcrunch.com.
   Topic: Tech & Ecosystem.
   🔗 https://example.com/a

2. [theverge] 标题B
   🧾 Summary:
   From theverge.com.
   🔗 https://example.com/b"#;
    assert!(
        !payload_uses_code_block(text),
        "新闻列表不应被包成 <pre><code>"
    );
}

#[test]
fn rss_summary_not_multiline_code() {
    let text = r#"1. [feed][host] Some headline
   🧾 Summary:
   From host.
   Topic: Other.
   🔗 https://link"#;
    assert!(
        !looks_like_multiline_code(text),
        "RSS 摘要不应被判成 multiline code"
    );
}

#[test]
fn shell_command_block_still_wrapped_as_code() {
    let text = r#"$ cargo build --release
$ ./target/release/bin"#;
    assert!(
        payload_uses_code_block(text),
        "真正的 shell 命令块仍应包成 code block"
    );
}

#[test]
fn fenced_code_block_still_wrapped_as_code() {
    let text = r#"```rust
fn main() {
    println!("hello");
}
```"#;
    assert!(
        payload_uses_code_block(text),
        "真正的 fenced code block 仍应包成 code block"
    );
}

#[test]
fn natural_language_summary_not_code() {
    let text = r#"这是一段普通自然语言多行摘要。
第二行说明。
第三行。"#;
    assert!(
        !payload_uses_code_block(text),
        "普通自然语言多行摘要应按普通文本发送"
    );
}

#[test]
fn should_never_format_as_code_detects_rss_header() {
    assert!(should_never_format_as_code(
        "sources_ok=2 sources_failed=1 items=3\n1. a\n2. b"
    ));
}

#[test]
fn should_never_format_as_code_detects_numbered_list() {
    assert!(should_never_format_as_code("1. First\n2. Second\n3. Third"));
}

#[test]
fn command_example_line_uses_structural_separator_rule() {
    assert!(looks_like_command_example_line(
        "示例：/rustclaw config show"
    ));
    assert!(looks_like_command_example_line("Example: /help"));
    assert!(!looks_like_command_example_line("Example: plain text"));
}
