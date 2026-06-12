const SECRET_REPLACEMENT: &str = "<SECRET>";
const BOT_TOKEN_REPLACEMENT: &str = "<BOT_TOKEN>";
const MIN_COLLECTED_SECRET_LEN: usize = 6;

pub fn redact_support_text(value: &str) -> String {
    let mut output = value.to_string();
    if let Some(home) = dirs::home_dir().map(|path| path.to_string_lossy().to_string()) {
        if !home.is_empty() {
            output = replace_case_insensitive(&output, &home, "<USER_HOME>");
        }
    }

    let mut secrets = Vec::new();
    output = redact_structured_secrets(&output, &mut secrets);
    output = redact_collected_secrets(&output, &secrets);
    output = redact_tg_links(&output);
    output = redact_bot_tokens(&output);
    output = redact_telegram_usernames(&output);
    redact_long_numbers(&output)
}

fn replace_case_insensitive(value: &str, needle: &str, replacement: &str) -> String {
    if needle.is_empty() {
        return value.to_string();
    }

    let lower_value = value.to_ascii_lowercase();
    let lower_needle = needle.to_ascii_lowercase();
    let mut output = String::with_capacity(value.len());
    let mut search_start = 0;

    while let Some(relative_index) = lower_value[search_start..].find(&lower_needle) {
        let index = search_start + relative_index;
        output.push_str(&value[search_start..index]);
        output.push_str(replacement);
        search_start = index + needle.len();
    }

    output.push_str(&value[search_start..]);
    output
}

#[derive(Debug)]
struct SecretField {
    secret_start: usize,
    secret_end: usize,
    line_value: bool,
}

fn redact_structured_secrets(value: &str, secrets: &mut Vec<String>) -> String {
    let mut output = String::with_capacity(value.len());
    let mut copy_start = 0;
    let mut index = 0;

    while index < value.len() {
        if let Some(field) =
            parse_cli_secret_field(value, index).or_else(|| parse_secret_field(value, index))
        {
            output.push_str(&value[copy_start..field.secret_start]);
            let secret = &value[field.secret_start..field.secret_end];
            if should_redact_secret(secret) {
                collect_secret_variants(secret, field.line_value, secrets);
                output.push_str(SECRET_REPLACEMENT);
            } else {
                output.push_str(secret);
            }
            copy_start = field.secret_end;
            index = field.secret_end;
        } else {
            index = next_char_index(value, index);
        }
    }

    output.push_str(&value[copy_start..]);
    output
}

fn parse_cli_secret_field(value: &str, index: usize) -> Option<SecretField> {
    if !value[index..].starts_with("--") || !is_token_boundary(value, index) {
        return None;
    }

    let bytes = value.as_bytes();
    let key_start = index + 2;
    let mut key_end = key_start;
    while key_end < value.len() && is_key_char(bytes[key_end]) {
        key_end += 1;
    }
    if key_end == key_start {
        return None;
    }

    let normalized_key = normalize_secret_key(&value[key_start..key_end]);
    if !is_sensitive_key(&normalized_key) {
        return None;
    }

    let mut value_start = key_end;
    if bytes.get(value_start) == Some(&b'=') {
        value_start += 1;
    } else if value_start < value.len() && bytes[value_start].is_ascii_whitespace() {
        value_start = skip_ascii_whitespace(value, value_start);
    } else {
        return None;
    }

    if value_start >= value.len() || value[value_start..].starts_with("--") {
        return None;
    }

    let (secret_start, secret_end) = parse_secret_value(value, value_start, false);
    if secret_start == secret_end {
        return None;
    }

    Some(SecretField {
        secret_start,
        secret_end,
        line_value: false,
    })
}

fn parse_secret_field(value: &str, index: usize) -> Option<SecretField> {
    let bytes = value.as_bytes();
    let quote = match bytes.get(index) {
        Some(b'"') => Some(b'"'),
        Some(b'\'') => Some(b'\''),
        _ => None,
    };

    if quote.is_none() && !is_field_boundary(value, index) {
        return None;
    }

    let key_start = index + quote.map_or(0, |_| 1);
    let mut key_end = key_start;
    while key_end < value.len() && is_key_char(bytes[key_end]) {
        key_end += 1;
    }
    if key_end == key_start {
        return None;
    }

    let raw_key = &value[key_start..key_end];
    let normalized_key = normalize_secret_key(raw_key);
    if !is_sensitive_key(&normalized_key) {
        return None;
    }

    let mut after_key = key_end;
    if let Some(quote) = quote {
        if bytes.get(after_key) != Some(&quote) {
            return None;
        }
        after_key += 1;
    }

    let separator_index = skip_ascii_whitespace(value, after_key);
    if !matches!(bytes.get(separator_index), Some(b':' | b'=')) {
        return None;
    }

    let value_start = skip_ascii_whitespace(value, separator_index + 1);
    if value_start >= value.len() {
        return None;
    }

    let line_value = is_line_secret_key(&normalized_key);
    let (secret_start, secret_end) = parse_secret_value(value, value_start, line_value);
    if secret_start == secret_end {
        return None;
    }

    Some(SecretField {
        secret_start,
        secret_end,
        line_value,
    })
}

fn parse_secret_value(value: &str, value_start: usize, line_value: bool) -> (usize, usize) {
    let bytes = value.as_bytes();
    if matches!(bytes.get(value_start), Some(b'"' | b'\'')) {
        let quote = bytes[value_start];
        let secret_start = value_start + 1;
        let mut secret_end = secret_start;
        let mut escaped = false;
        while secret_end < value.len() {
            let byte = bytes[secret_end];
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == quote {
                break;
            }
            secret_end += 1;
        }
        return (secret_start, secret_end);
    }

    let secret_start = value_start;
    let mut secret_end = secret_start;
    while secret_end < value.len() {
        let byte = bytes[secret_end];
        if byte == b'\r' || byte == b'\n' {
            break;
        }
        if !line_value
            && (byte.is_ascii_whitespace() || matches!(byte, b'&' | b',' | b';' | b'}' | b']'))
        {
            break;
        }
        secret_end += 1;
    }
    (secret_start, secret_end)
}

fn is_field_boundary(value: &str, index: usize) -> bool {
    if index == 0 {
        return true;
    }
    value[..index]
        .chars()
        .next_back()
        .map(|ch| !(ch.is_ascii_alphanumeric() || ch == '_' || ch == '-'))
        .unwrap_or(true)
}

fn is_key_char(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-')
}

fn normalize_secret_key(key: &str) -> String {
    key.chars()
        .filter(|ch| *ch != '_' && *ch != '-')
        .flat_map(char::to_lowercase)
        .collect()
}

fn is_sensitive_key(normalized_key: &str) -> bool {
    matches!(
        normalized_key,
        "token"
            | "bottoken"
            | "apihash"
            | "apiid"
            | "apikey"
            | "authorization"
            | "password"
            | "passcode"
            | "accesstoken"
            | "refreshtoken"
            | "secret"
            | "session"
            | "sessionid"
            | "sessiontoken"
            | "cookie"
            | "setcookie"
            | "csrf"
            | "xsrf"
            | "proxy"
    )
}

fn is_line_secret_key(normalized_key: &str) -> bool {
    matches!(normalized_key, "authorization" | "cookie" | "setcookie")
}

fn skip_ascii_whitespace(value: &str, mut index: usize) -> usize {
    let bytes = value.as_bytes();
    while index < value.len() && bytes[index].is_ascii_whitespace() {
        index += 1;
    }
    index
}

fn should_redact_secret(secret: &str) -> bool {
    let trimmed = secret.trim();
    !trimmed.is_empty()
        && !trimmed.contains(SECRET_REPLACEMENT)
        && !trimmed.contains(BOT_TOKEN_REPLACEMENT)
        && !trimmed.contains("<TG_LINK>")
        && !trimmed.contains("<TG_USERNAME>")
        && !trimmed.contains("<PHONE_OR_ID>")
}

fn collect_secret_variants(secret: &str, line_value: bool, secrets: &mut Vec<String>) {
    let trimmed = secret.trim();
    add_secret(secrets, trimmed);

    if let Some(token) = strip_auth_scheme(trimmed) {
        add_secret(secrets, token);
    }

    if line_value || trimmed.contains('=') {
        for part in trimmed.split([';', '&', ',']) {
            if let Some((_, value)) = part.split_once('=') {
                add_secret(secrets, value.trim());
            }
        }
    }
}

fn strip_auth_scheme(value: &str) -> Option<&str> {
    let trimmed = value.trim_start();
    for scheme in ["Bearer", "Basic"] {
        if trimmed.len() > scheme.len()
            && trimmed[..scheme.len()].eq_ignore_ascii_case(scheme)
            && trimmed
                .as_bytes()
                .get(scheme.len())
                .is_some_and(u8::is_ascii_whitespace)
        {
            return Some(trimmed[scheme.len()..].trim());
        }
    }
    None
}

fn add_secret(secrets: &mut Vec<String>, value: &str) {
    let trimmed = value.trim_matches(|ch: char| {
        ch.is_ascii_whitespace() || matches!(ch, '"' | '\'' | ',' | ';' | ')' | ']' | '}')
    });
    if trimmed.len() >= MIN_COLLECTED_SECRET_LEN
        && should_redact_secret(trimmed)
        && !secrets.iter().any(|secret| secret == trimmed)
    {
        secrets.push(trimmed.to_string());
    }
}

fn redact_collected_secrets(value: &str, secrets: &[String]) -> String {
    let mut sorted = secrets.to_vec();
    sorted.sort_by_key(|secret| std::cmp::Reverse(secret.len()));
    sorted.dedup();

    let mut output = value.to_string();
    for secret in sorted {
        output = output.replace(&secret, SECRET_REPLACEMENT);
    }
    output
}

fn redact_tg_links(value: &str) -> String {
    map_non_whitespace_parts(value, |part| {
        let lower = part.to_ascii_lowercase();
        if lower.contains("t.me/") || lower.contains("telegram.me/") || lower.contains("tg://") {
            preserve_trailing_punctuation(part, "<TG_LINK>")
        } else {
            part.to_string()
        }
    })
}

fn preserve_trailing_punctuation(part: &str, replacement: &str) -> String {
    let trailing = part
        .chars()
        .rev()
        .take_while(|ch| matches!(ch, '.' | ',' | ';' | ')' | ']' | '}' | '>' | '。' | '，'))
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<String>();
    if trailing.is_empty() {
        replacement.to_string()
    } else {
        format!("{replacement}{trailing}")
    }
}

fn redact_bot_tokens(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut output = String::with_capacity(value.len());
    let mut copy_start = 0;
    let mut index = 0;

    while index < value.len() {
        if let Some(token_end) = parse_bot_token(value, index) {
            output.push_str(&value[copy_start..index]);
            output.push_str(BOT_TOKEN_REPLACEMENT);
            copy_start = token_end;
            index = token_end;
        } else {
            index += if bytes[index].is_ascii() {
                1
            } else {
                next_char_index(value, index) - index
            };
        }
    }

    output.push_str(&value[copy_start..]);
    output
}

fn parse_bot_token(value: &str, index: usize) -> Option<usize> {
    if !is_token_boundary(value, index) {
        return None;
    }

    let bytes = value.as_bytes();
    let mut cursor = index;
    if value[index..].starts_with("bot") {
        cursor += 3;
    }

    let digit_start = cursor;
    while cursor < value.len() && bytes[cursor].is_ascii_digit() {
        cursor += 1;
    }
    if cursor - digit_start < 6 || bytes.get(cursor) != Some(&b':') {
        return None;
    }
    cursor += 1;

    let token_start = cursor;
    while cursor < value.len() && is_bot_token_char(bytes[cursor]) {
        cursor += 1;
    }
    if cursor - token_start < 20 {
        return None;
    }

    Some(cursor)
}

fn is_token_boundary(value: &str, index: usize) -> bool {
    if index == 0 {
        return true;
    }
    value[..index]
        .chars()
        .next_back()
        .map(|ch| !(ch.is_ascii_alphanumeric() || ch == '_' || ch == '-'))
        .unwrap_or(true)
}

fn is_bot_token_char(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-')
}

fn redact_telegram_usernames(value: &str) -> String {
    map_non_whitespace_parts(value, |part| {
        if part.starts_with('@') && part.len() > 1 {
            preserve_trailing_punctuation(part, "<TG_USERNAME>")
        } else {
            part.to_string()
        }
    })
}

fn redact_long_numbers(value: &str) -> String {
    map_non_whitespace_parts(value, |part| {
        let digits = part.chars().filter(|ch| ch.is_ascii_digit()).count();
        if digits >= 8 {
            preserve_trailing_punctuation(part, "<PHONE_OR_ID>")
        } else {
            part.to_string()
        }
    })
}

fn map_non_whitespace_parts<F>(value: &str, mut mapper: F) -> String
where
    F: FnMut(&str) -> String,
{
    let mut output = String::with_capacity(value.len());
    let mut part_start = None;

    for (index, ch) in value.char_indices() {
        if ch.is_whitespace() {
            if let Some(start) = part_start.take() {
                output.push_str(&mapper(&value[start..index]));
            }
            output.push(ch);
        } else if part_start.is_none() {
            part_start = Some(index);
        }
    }

    if let Some(start) = part_start {
        output.push_str(&mapper(&value[start..]));
    }

    output
}

fn next_char_index(value: &str, index: usize) -> usize {
    value[index..]
        .chars()
        .next()
        .map(|ch| index + ch.len_utf8())
        .unwrap_or(value.len())
}

#[cfg(test)]
mod tests {
    use super::redact_support_text;

    #[test]
    fn redacts_query_secrets_and_telegram_links() {
        let output = redact_support_text("url=https://example.test/cb?token=abc123&ok=1 tg://resolve?domain=user https://t.me/name");
        assert!(output.contains("token=<SECRET>&ok=1"));
        assert!(!output.contains("abc123"));
        assert!(!output.contains("tg://"));
        assert!(!output.contains("t.me/name"));
    }

    #[test]
    fn redacts_bot_tokens_and_usernames() {
        let output = redact_support_text("bot123456789:ABCDEFGHIJKLMNOPQRSTUVWXYZ @someone");
        assert!(output.contains("<BOT_TOKEN>"));
        assert!(output.contains("<TG_USERNAME>"));
        assert!(!output.contains("ABCDEFGHIJKLMNOPQRSTUVWXYZ"));
    }

    #[test]
    fn redacts_bare_bot_tokens_in_urls() {
        let output = redact_support_text("https://api.telegram.org/bot123456789:ABCDEFGHIJKLMNOPQRSTUVWXYZ/sendMessage 123456789:ABCDEFGHIJKLMNOPQRSTUVWXYZ");
        assert_eq!(output.matches("<BOT_TOKEN>").count(), 2);
        assert!(!output.contains("123456789:ABCDEFGHIJKLMNOPQRSTUVWXYZ"));
    }

    #[test]
    fn redacts_json_style_secrets() {
        let output = redact_support_text(r#"{"api_hash":"abcdef123456","password":"secret"}"#);
        assert!(output.contains("\"api_hash\":\"<SECRET>\""));
        assert!(output.contains("\"password\":\"<SECRET>\""));
        assert!(!output.contains("abcdef123456"));
        assert!(!output.contains("secret"));
    }

    #[test]
    fn redacts_pretty_json_and_spaced_key_values() {
        let input = "{\n  \"api_hash\": \"abcdef123456\",\n  password : very-secret-value,\n  api_id = 1234567\n}";
        let output = redact_support_text(input);
        assert!(output.contains("\"api_hash\": \"<SECRET>\""));
        assert!(output.contains("password : <SECRET>"));
        assert!(output.contains("api_id = <SECRET>"));
        assert!(!output.contains("abcdef123456"));
        assert!(!output.contains("very-secret-value"));
        assert!(!output.contains("1234567"));
    }

    #[test]
    fn redacts_authorization_and_cookie_headers() {
        let input = "Authorization: Bearer bearer-token-value\nCookie: session=abc123456; csrf=csrf-token-value\nSet-Cookie: sid=session-token-value; Path=/";
        let output = redact_support_text(input);
        assert!(output.contains("Authorization: <SECRET>"));
        assert!(output.contains("Cookie: <SECRET>"));
        assert!(output.contains("Set-Cookie: <SECRET>"));
        assert!(!output.contains("bearer-token-value"));
        assert!(!output.contains("abc123456"));
        assert!(!output.contains("csrf-token-value"));
        assert!(!output.contains("session-token-value"));
    }

    #[test]
    fn redacts_repeated_secret_values() {
        let output = redact_support_text(
            "password=repeated-secret-value\nretry repeated-secret-value later",
        );
        assert_eq!(output.matches("<SECRET>").count(), 2);
        assert!(!output.contains("repeated-secret-value"));
    }

    #[test]
    fn redacts_space_separated_cli_secret_flags() {
        let output = redact_support_text("tdl download --api-hash abcdef123456 --api-id 1234567 --password pass-value --proxy socks5://user:pass@example.test:1080 --limit 10");
        assert!(output.contains("--api-hash <SECRET>"));
        assert!(output.contains("--api-id <SECRET>"));
        assert!(output.contains("--password <SECRET>"));
        assert!(output.contains("--proxy <SECRET>"));
        assert!(output.contains("--limit 10"));
        assert!(!output.contains("abcdef123456"));
        assert!(!output.contains("1234567"));
        assert!(!output.contains("pass-value"));
        assert!(!output.contains("user:pass@example.test"));
    }

    #[test]
    fn redacts_quoted_secret_with_escaped_quote() {
        let output = redact_support_text(r#"{"password":"abc\"def-secret"}"#);
        assert!(output.contains("\"password\":\"<SECRET>\""));
        assert!(!output.contains("def-secret"));
    }

    #[test]
    fn redaction_is_idempotent() {
        let once = redact_support_text(
            "Authorization: Bearer bearer-token-value\npassword = repeated-secret-value",
        );
        let twice = redact_support_text(&once);
        assert_eq!(once, twice);
    }

    #[test]
    fn keeps_non_secret_log_lines_readable() {
        let input = "download completed: file_name=holiday-video.mp4 status=ok";
        assert_eq!(redact_support_text(input), input);
    }

    #[test]
    fn redacts_long_numbers_with_punctuation() {
        let output = redact_support_text("chat=1234567890, phone +8613800000000.");
        assert!(output.contains("<PHONE_OR_ID>,"));
        assert!(output.contains("<PHONE_OR_ID>."));
        assert!(!output.contains("1234567890"));
    }
}
