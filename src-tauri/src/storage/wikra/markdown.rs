use std::collections::HashMap;

const PROTECT_START: &str = "<!-- wikra-protect:start -->";
const PROTECT_END: &str = "<!-- wikra-protect:end -->";
const LEGACY_PROTECT_START: &str = "<!-- fmd-protect:start -->";
const LEGACY_PROTECT_END: &str = "<!-- fmd-protect:end -->";

pub(super) struct Markdown {
    metadata: HashMap<String, Vec<String>>,
    pub content_start: usize,
    pub title: Option<String>,
    pub outline: Vec<String>,
    pub footnotes: Vec<Footnote>,
    pub links: Vec<WikiLink>,
    footnote_start: Option<usize>,
    protect_end: Option<usize>,
    protection_invalid: bool,
    unclosed_fence: bool,
}

pub(super) struct Footnote {
    pub ref_label: String,
    pub target: Option<String>,
    pub label: String,
    pub quote: String,
}

pub(super) struct WikiLink {
    pub target: String,
    pub label: String,
    pub snippet: String,
}

#[derive(Default)]
pub(super) struct CodeFence(Option<(u8, usize)>);

impl CodeFence {
    // Returns true for the delimiters too, so callers only see Markdown outside code.
    pub fn contains(&mut self, line: &str) -> bool {
        let trimmed = line.trim_start_matches(' ');
        if line.len() - trimmed.len() <= 3
            && let Some(&marker @ (b'\x60' | b'~')) = trimmed.as_bytes().first()
        {
            let count = trimmed.bytes().take_while(|&byte| byte == marker).count();
            if let Some((fence, min)) = self.0 {
                if marker == fence && count >= min && trimmed[count..].trim().is_empty() {
                    self.0 = None;
                    return true;
                }
            } else if count >= 3 && (marker == b'~' || !trimmed[count..].contains('\x60')) {
                self.0 = Some((marker, count));
                return true;
            }
        }
        self.0.is_some()
    }
}

impl Markdown {
    pub fn parse(body: &str) -> Self {
        let (metadata, content_start) = frontmatter(body);
        let mut result = Self {
            metadata,
            content_start,
            title: None,
            outline: Vec::new(),
            footnotes: Vec::new(),
            links: Vec::new(),
            footnote_start: None,
            protect_end: None,
            protection_invalid: false,
            unclosed_fence: false,
        };
        let mut fence = CodeFence::default();
        let mut offset = content_start;
        let mut protect = None;
        let mut comment = false;
        let mut extending_footnote = false;
        for raw in body[content_start..].split_inclusive('\n') {
            let position = offset;
            offset += raw.len();
            let line = raw.trim_end_matches(['\r', '\n']);
            if !comment && fence.contains(line) {
                extending_footnote = false;
                continue;
            }
            if !comment {
                match line.trim() {
                    PROTECT_START | LEGACY_PROTECT_START => {
                        if protect.replace(line.trim() == PROTECT_START).is_some() {
                            result.protection_invalid = true;
                        }
                        continue;
                    }
                    PROTECT_END | LEGACY_PROTECT_END => {
                        if protect.take() == Some(line.trim() == PROTECT_END) {
                            result.protect_end = Some(position);
                        } else {
                            result.protection_invalid = true;
                        }
                        continue;
                    }
                    _ => (),
                }
            }
            if !comment && let Some((id, value)) = footnote_definition(line) {
                let outside_protection = protect.is_none();
                if outside_protection && result.footnote_start.is_none() {
                    result.footnote_start = Some(position);
                }
                if !result.footnotes.iter().any(|item| item.ref_label == id) {
                    result.footnotes.push(parse_footnote(id, value));
                    extending_footnote = true;
                } else {
                    extending_footnote = false;
                }
                continue;
            }
            if extending_footnote && (line.starts_with("    ") || line.starts_with('\t')) {
                if let Some(item) = result.footnotes.last_mut() {
                    if !item.quote.is_empty() {
                        item.quote.push(' ');
                    }
                    item.quote.push_str(line.trim());
                }
                continue;
            }
            extending_footnote = false;
            let visible = visible_text(line, &mut comment);
            if let Some((level, title)) = heading(&visible) {
                if level == 1 && result.title.is_none() {
                    result.title = Some(title.to_owned());
                } else if level == 2 {
                    result.outline.push(title.to_owned());
                }
            }
            if !line.starts_with("    ") && !line.starts_with('\t') {
                result.links.extend(wiki_links(&visible));
            }
        }
        result.protection_invalid |= protect.is_some();
        result.unclosed_fence = fence.0.is_some() || comment;
        result
    }

    pub fn value(&self, key: &str) -> Option<&str> {
        self.metadata.get(key)?.first().map(String::as_str)
    }

    pub fn list(&self, key: &str) -> &[String] {
        self.metadata.get(key).map_or(&[], Vec::as_slice)
    }

    pub fn core(&self) -> bool {
        self.value("wikra-core")
            .or_else(|| self.value("fmd-core"))
            .is_some_and(|value| value.eq_ignore_ascii_case("true"))
            || self
                .list("tags")
                .iter()
                .any(|tag| matches!(tag.trim_start_matches('#'), "wikra-core" | "fmd-core"))
    }

    pub fn managed_false(&self) -> bool {
        self.value("wikra-managed")
            .or_else(|| self.value("fmd-managed"))
            .is_some_and(|value| value.eq_ignore_ascii_case("false"))
    }

    pub fn conflicts(&self) -> Vec<String> {
        let mut ids = Vec::new();
        for key in [
            "wikra_pending_conflicts",
            "wikra-pending-conflicts",
            "fmd_pending_conflicts",
            "fmd-pending-conflicts",
        ] {
            for id in self.list(key) {
                if !ids.contains(id) {
                    ids.push(id.clone());
                }
            }
        }
        ids
    }

    pub fn protected(&self) -> bool {
        self.protect_end.is_some() || self.protection_invalid
    }

    pub fn append_section(&self, body: &str, section: &str) -> Option<String> {
        if self.protection_invalid || self.unclosed_fence {
            return None;
        }
        // If footnotes appear earlier in the page, inserting before them must stay outside
        // any protected block. Only footnotes outside such blocks set footnote_start.
        Some(insert_block(
            body,
            self.footnote_start.unwrap_or(body.len()),
            section,
        ))
    }

    pub fn append_insight(&self, body: &str, insight: &str) -> Option<String> {
        if [
            PROTECT_START,
            PROTECT_END,
            LEGACY_PROTECT_START,
            LEGACY_PROTECT_END,
        ]
        .iter()
        .any(|marker| insight.contains(marker))
            || self.protection_invalid
        {
            return None;
        }
        if let Some(end) = self.protect_end {
            return Some(insert_block(body, end, insight));
        }
        self.append_section(
            body,
            &format!("{PROTECT_START}\n## 我的理解\n\n{insight}\n{PROTECT_END}"),
        )
    }

    pub fn has_footnote(&self, id: &str) -> bool {
        self.footnotes.iter().any(|note| note.ref_label == id)
    }
}

fn insert_block(body: &str, position: usize, block: &str) -> String {
    let (before, after) = body.split_at(position);
    let lb = if body.contains("\r\n") { "\r\n" } else { "\n" };
    let block = block.trim().replace("\r\n", "\n").replace('\n', lb);
    let mut output = String::with_capacity(body.len() + block.len() + 8);
    output.push_str(before);
    if !before.is_empty() {
        if !before.ends_with('\n') {
            output.push_str(lb);
        }
        if !before
            .strip_suffix('\n')
            .is_some_and(|text| text.trim_end_matches('\r').ends_with('\n'))
        {
            output.push_str(lb);
        }
    }
    output.push_str(&block);
    output.push_str(lb);
    if !after.is_empty() && !after.starts_with(['\r', '\n']) {
        output.push_str(lb);
    }
    output.push_str(after);
    output
}

fn frontmatter(body: &str) -> (HashMap<String, Vec<String>>, usize) {
    let start = body
        .strip_prefix('\u{feff}')
        .map_or(0, |_| '\u{feff}'.len_utf8());
    let mut lines = body[start..].split_inclusive('\n');
    let Some(first) = lines.next().filter(|line| line.trim_end() == "---") else {
        return (HashMap::new(), start);
    };
    let mut offset = start + first.len();
    let mut result = HashMap::new();
    let mut current_key = None;
    for line in lines {
        offset += line.len();
        let trimmed = line.trim();
        if matches!(line.trim_end(), "---" | "...") {
            return (result, offset);
        }
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if let Some(value) = trimmed.strip_prefix("- ")
            && let Some(key) = current_key.as_ref()
        {
            let value = scalar(value);
            if !value.is_empty() {
                result
                    .entry(String::clone(key))
                    .or_insert_with(Vec::new)
                    .push(value);
            }
        } else if !line.starts_with(char::is_whitespace)
            && let Some((key, value)) = trimmed.split_once(':')
            && !key.is_empty()
            && key
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        {
            let key = key.to_ascii_lowercase();
            let values = fields(value);
            current_key = Some(key.clone());
            result.insert(key, values);
        }
    }
    (HashMap::new(), start)
}

fn fields(value: &str) -> Vec<String> {
    let value = without_yaml_comment(value).trim();
    let Some(inner) = value
        .strip_prefix('[')
        .and_then(|value| value.strip_suffix(']'))
    else {
        let value = scalar(value);
        return if value.is_empty() {
            Vec::new()
        } else {
            vec![value]
        };
    };
    let mut start = 0;
    let mut values = Vec::new();
    let mut quote = YamlQuote::default();
    for (index, ch) in inner.char_indices() {
        if !quote.update(ch, inner[start..index].trim().is_empty()) && ch == ',' {
            values.push(scalar(&inner[start..index]));
            start = index + 1;
        }
    }
    values.push(scalar(&inner[start..]));
    values.retain(|value| !value.is_empty());
    values
}

#[derive(Default)]
struct YamlQuote {
    delimiter: Option<char>,
    escaped: bool,
}

impl YamlQuote {
    fn update(&mut self, ch: char, starts_value: bool) -> bool {
        if let Some(delimiter) = self.delimiter {
            if std::mem::take(&mut self.escaped) {
                return true;
            }
            if ch == delimiter {
                self.delimiter = None;
            } else if delimiter == '"' && ch == '\\' {
                self.escaped = true;
            }
            return true;
        }
        if starts_value && matches!(ch, '\'' | '"') {
            self.delimiter = Some(ch);
            return true;
        }
        false
    }
}

fn without_yaml_comment(value: &str) -> &str {
    let mut quote = YamlQuote::default();
    let mut previous = ' ';
    let mut starts_value = true;
    for (index, ch) in value.char_indices() {
        let quoted = quote.update(ch, starts_value);
        if !quoted && ch == '#' && previous.is_whitespace() {
            return &value[..index];
        }
        if !ch.is_whitespace() {
            starts_value = !quoted && matches!(ch, '[' | ',');
        }
        previous = ch;
    }
    value
}

fn scalar(value: &str) -> String {
    let value = without_yaml_comment(value).trim();
    if let Some(inner) = value.strip_prefix('\'').and_then(|s| s.strip_suffix('\'')) {
        return inner.replace("''", "'");
    }
    if value.starts_with('"') && value.ends_with('"') {
        return serde_json::from_str(value).unwrap_or_else(|_| value.trim_matches('"').to_owned());
    }
    if matches!(value, "null" | "~") {
        return String::new();
    }
    value.to_owned()
}

pub(super) fn heading(line: &str) -> Option<(usize, &str)> {
    let line = line.trim_start_matches(' ');
    let level = line.bytes().take_while(|&byte| byte == b'#').count();
    if (1..=6).contains(&level) && line[level..].starts_with([' ', '\t']) {
        let title = line[level..].trim();
        let tail = title.trim_end_matches('#');
        let title = if tail.ends_with(char::is_whitespace) {
            tail.trim_end()
        } else {
            title
        };
        return (!title.is_empty()).then_some((level, title));
    }
    None
}

fn footnote_definition(line: &str) -> Option<(&str, &str)> {
    let trimmed = line.trim_start_matches(' ');
    if line.len() - trimmed.len() > 3 {
        return None;
    }
    let (id, value) = trimmed.strip_prefix("[^")?.split_once("]:")?;
    (!id.is_empty() && !id.contains(char::is_whitespace)).then_some((id, value.trim()))
}

fn parse_footnote(id: &str, value: &str) -> Footnote {
    let wiki = value.strip_prefix("[[").and_then(|value| {
        let (link, tail) = value.split_once("]]")?;
        let (target, label) = link_parts(link);
        Some((target, label, tail))
    });
    let markdown = || {
        let (label, rest) = value.strip_prefix('[')?.split_once("](")?;
        let (target, tail) = if let Some(rest) = rest.strip_prefix('<') {
            rest.split_once(">)")?
        } else {
            rest.split_once(')')?
        };
        Some((target.trim(), label.trim(), tail))
    };
    let (target, label, quote) = wiki.or_else(markdown).map_or_else(
        || {
            if let Some((label, quote)) = value.split_once(" —— ") {
                (None, label.trim(), quote.trim())
            } else {
                (None, id, value)
            }
        },
        |(target, label, tail)| (Some(target.to_owned()), label, tail),
    );
    let quote = quote
        .trim()
        .trim_start_matches(['—', '–', ':', '：'])
        .trim();
    let quote = quote
        .strip_prefix('“')
        .and_then(|s| s.strip_suffix('”'))
        .or_else(|| quote.strip_prefix('"').and_then(|s| s.strip_suffix('"')))
        .unwrap_or(quote);
    Footnote {
        ref_label: id.to_owned(),
        target,
        label: label.to_owned(),
        quote: quote.to_owned(),
    }
}

fn link_parts(link: &str) -> (&str, &str) {
    let (target, label) = link.split_once('|').unwrap_or((link, link));
    (target.trim(), label.trim())
}

fn visible_text(line: &str, in_comment: &mut bool) -> String {
    let mut rest = line;
    let mut visible = String::new();
    while !rest.is_empty() {
        if *in_comment {
            let Some((_, after)) = rest.split_once("-->") else {
                break;
            };
            rest = after;
            *in_comment = false;
        } else if let Some((before, after)) = rest.split_once("<!--") {
            visible.push_str(before);
            rest = after;
            *in_comment = true;
        } else {
            visible.push_str(rest);
            break;
        }
    }
    visible
}

fn wiki_links(line: &str) -> Vec<WikiLink> {
    let mut links = Vec::new();
    let mut rest = line;
    let mut code_length = 0;
    while let Some(index) = rest.find(['\x60', '[', '\\']) {
        rest = &rest[index..];
        if let Some(escaped) = rest.strip_prefix('\\') {
            rest = escaped
                .chars()
                .next()
                .map_or("", |c| &escaped[c.len_utf8()..]);
        } else if rest.starts_with('\x60') {
            let count = rest.bytes().take_while(|&b| b == b'\x60').count();
            if code_length == count {
                code_length = 0;
            } else if code_length == 0 {
                code_length = count;
            }
            rest = &rest[count..];
        } else if code_length == 0
            && let Some((link, after)) = rest.strip_prefix("[[").and_then(|s| s.split_once("]]"))
        {
            let (target, label) = link_parts(link);
            if !target.is_empty() && !target.starts_with(['#', '^']) && !target.contains('[') {
                links.push(WikiLink {
                    target: target.to_owned(),
                    label: label.to_owned(),
                    snippet: line.trim().chars().take(180).collect(),
                });
            }
            rest = after;
        } else {
            rest = &rest[1..];
        }
    }
    links
}
