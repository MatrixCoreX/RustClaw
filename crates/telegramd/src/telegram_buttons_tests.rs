use super::{build_url_button_markup, extract_url_buttons_from_text};

#[test]
fn extracts_only_prefixed_button_lines_and_strips_them_from_text() {
    let parsed = extract_url_buttons_from_text(
        "商户 A\nBUTTON: 使用高德导航：https://a.example\nBUTTON: 使用高德导航：https://b.example",
    );
    assert_eq!(parsed.text_without_buttons, "商户 A");
    assert_eq!(parsed.buttons.len(), 2);
    assert_eq!(parsed.buttons[0].label, "使用高德导航");
    assert_eq!(parsed.buttons[1].label, "使用高德导航 2");
}

#[test]
fn ignores_non_url_lines() {
    let parsed =
        extract_url_buttons_from_text("说明文字\nBUTTON: 使用高德导航：not-a-url\n继续展示");
    assert_eq!(
        parsed.text_without_buttons,
        "说明文字\nBUTTON: 使用高德导航：not-a-url\n继续展示"
    );
    assert!(parsed.buttons.is_empty());
}

#[test]
fn keeps_plain_label_url_lines_as_text() {
    let parsed = extract_url_buttons_from_text(
        "说明文字\n使用高德导航：https://uri.amap.com/navigation?to=1,2,test",
    );
    assert_eq!(
        parsed.text_without_buttons,
        "说明文字\n使用高德导航：https://uri.amap.com/navigation?to=1,2,test"
    );
    assert!(parsed.buttons.is_empty());
}

#[test]
fn builds_markup_only_for_valid_urls() {
    let parsed = extract_url_buttons_from_text(
        "BUTTON: 使用高德导航：https://uri.amap.com/navigation?to=1,2,test",
    );
    let markup = build_url_button_markup(&parsed.buttons);
    assert!(markup.is_some());
}
