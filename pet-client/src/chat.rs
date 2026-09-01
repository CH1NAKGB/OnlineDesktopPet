use shared_protocol::MAX_TEXT_CHARS;
use tinyfiledialogs::MessageBoxIcon;

pub fn compose(partner_id: Option<&str>) -> Option<String> {
    let Some(partner_id) = partner_id else {
        tinyfiledialogs::message_box_ok(
            "Chick Link",
            "尚未配对。请先设置 CHICK_PARTNER_ID。",
            MessageBoxIcon::Info,
        );
        return None;
    };
    let prompt = format!("发送给 {partner_id}（最多 {MAX_TEXT_CHARS} 字）");
    let text = tinyfiledialogs::input_box("Chick Link 聊天", &prompt, "")?;
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    Some(text.chars().take(MAX_TEXT_CHARS).collect())
}

pub fn show_incoming(sender_id: &str, text: &str) {
    tinyfiledialogs::message_box_ok(
        &format!("来自 {sender_id} 的消息"),
        text,
        MessageBoxIcon::Info,
    );
}
