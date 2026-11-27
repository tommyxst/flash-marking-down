use std::collections::HashMap;
use std::ops::Range;

const PROTECT_START: &str = "<!-- wikra-protect:start -->";
const PROTECT_END: &str = "<!-- wikra-protect:end -->";
const LEGACY_PROTECT_START: &str = "<!-- fmd-protect:start -->";
const LEGACY_PROTECT_END: &str = "<!-- fmd-protect:end -->";

pub(crate) struct Markdown {
    metadata: HashMap<String, Vec<String>>,
    pub content_start: usize,
    pub title: Option<String>,
    pub outline: Vec<String>,
    pub footnotes: Vec<Footnote>,
    pub links: Vec<WikiLink>,
    pub headings: Vec<Heading>,
    pub blocks: std::collections::HashSet<String>,
    pub protections: Vec<Range<usize>>,
    pub opaque: Vec<Range<usize>>,
    pub managed_markers: Vec<Range<usize>>,
    pub text_chars: usize,
    footnote_start: Option<usize>,
    protect_end: Option<usize>,
    protection_invalid: bool,
    unclosed_fence: bool,
}

pub(crate) struct Footnote {
    pub ref_label: String,
    pub target: Option<String>,
    pub label: String,
    pub quote: String,
    pub line: usize,
}

pub(crate) struct WikiLink {
    pub target: String,
    pub label: String,
    pub snippet: String,
    pub span: Range<usize>,
    pub line: usize,
}

pub(crate) struct Heading {
    pub level: usize,
    pub title: String,
    pub slug: String,
    pub span: Range<usize>,
    pub body_start: usize,
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
            headings: Vec::new(),
            blocks: Default::default(),
            protections: Vec::new(),
            opaque: Vec::new(),
            managed_markers: Vec::new(),
            text_chars: 0,
            footnote_start: None,
            protect_end: None,
            protection_invalid: false,
            unclosed_fence: false,
        };
        let mut fence = CodeFence::default();
        let mut offset = content_start;
        let mut protect = None;
        let mut comment = None;
        let mut code_end = 0;
        let first_line = body[..content_start]
            .bytes()
            .filter(|&b| b == b'\n')
            .count();
        let mut extending_footnote = false;
        for (number, raw) in body[content_start..].split_inclusive('\n').enumerate() {
            let position = offset;
            offset += raw.len();
            let line = raw.trim_end_matches(['\r', '\n']);
            let outside_inline = code_end <= position;
            if outside_inline && comment.is_none() && fence.contains(line) {
                result.opaque.push(position..offset);
                extending_footnote = false;
                continue;
            }
            if outside_inline && comment.is_none() {
                match line.trim() {
                    PROTECT_START | LEGACY_PROTECT_START => {
                        if protect
                            .replace((position, line.trim() == PROTECT_START))
                            .is_some()
                        {
                            result.protection_invalid = true;
                        }
                        continue;
                    }
                    PROTECT_END | LEGACY_PROTECT_END => {
                        if let Some((start, modern)) = protect.take()
                            && modern == (line.trim() == PROTECT_END)
                        {
                            result.protect_end = Some(position);
                            result.protections.push(start..offset);
                        } else {
                            result.protection_invalid = true;
                        }
                        continue;
                    }
                    marker @ ("<!-- wikra-index:start -->"
                    | "<!-- wikra-index:end -->"
                    | "<!-- wikra-skill:start -->"
                    | "<!-- wikra-skill:end -->")
                        if !line.starts_with("    ") && !line.starts_with('\t') =>
                    {
                        let start = position + line.len() - line.trim_start().len();
                        result.managed_markers.push(start..start + marker.len());
                        result.opaque.push(position..offset);
                        continue;
                    }
                    _ => (),
                }
            }
            let previous_comment = comment;
            let previous_code_end = code_end;
            let (visible, had_comment) =
                visible_text(body, position, line, &mut comment, &mut code_end);
            if let Some((id, value)) = footnote_definition(&visible) {
                comment = previous_comment;
                code_end = previous_code_end;
                let outside_protection = protect.is_none();
                if outside_protection && result.footnote_start.is_none() {
                    result.footnote_start = Some(position);
                }
                if !result.footnotes.iter().any(|item| item.ref_label == id) {
                    let original_value =
                        footnote_definition(line).map_or(value, |(_, value)| value);
                    let mut item = parse_footnote(id, original_value);
                    item.line = first_line + number + 1;
                    result.footnotes.push(item);
                    extending_footnote = true;
                } else {
                    extending_footnote = false;
                }
                continue;
            }
            if had_comment || line.starts_with("    ") || line.starts_with('\t') {
                result.opaque.push(position..offset);
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
            let heading_line = if outside_inline && !had_comment {
                line
            } else {
                &visible
            };
            if let Some((level, title)) = heading(heading_line) {
                result.headings.push(Heading {
                    level,
                    title: title.to_owned(),
                    slug: heading_slug(title),
                    span: position..offset,
                    body_start: offset,
                });
                if level == 1 && result.title.is_none() {
                    result.title = Some(title.to_owned());
                } else if level == 2 {
                    result.outline.push(title.to_owned());
                }
            } else if !line.starts_with("    ") && !line.starts_with('\t') {
                result.text_chars += visible.chars().filter(|ch| !ch.is_whitespace()).count();
            }
            if !line.starts_with("    ") && !line.starts_with('\t') {
                if let Some((_, id)) = visible.trim_end().rsplit_once('^')
                    && !id.is_empty()
                    && id
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
                {
                    let prefix = visible.trim_end().strip_suffix(id).unwrap_or_default();
                    if prefix
                        .strip_suffix('^')
                        .is_some_and(|p| p.is_empty() || p.ends_with(char::is_whitespace))
                    {
                        result.blocks.insert(id.to_owned());
                    }
                }
                for mut link in wiki_links(&visible) {
                    link.span = link.span.start + position..link.span.end + position;
                    link.line = first_line + number + 1;
                    result.links.push(link);
                }
            }
        }
        result.protection_invalid |= protect.is_some();
        result.unclosed_fence = fence.0.is_some() || comment.is_some();
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

    pub(crate) fn blocked(&self) -> bool {
        self.protection_invalid || self.unclosed_fence
    }

    pub(crate) fn intersects_protection(&self, range: &Range<usize>) -> bool {
        self.protections
            .iter()
            .any(|protected| range.end > protected.start && range.start < protected.end)
    }

    pub(crate) fn has_anchor(&self, anchor: &str) -> bool {
        if let Some(block) = anchor.strip_prefix('^') {
            return self.blocks.contains(block);
        }
        let slug = heading_slug(anchor);
        self.headings.iter().any(|heading| {
            heading.title.eq_ignore_ascii_case(anchor) || (!slug.is_empty() && heading.slug == slug)
        })
    }

    pub(crate) fn section_range(&self, index: usize, body_len: usize) -> Range<usize> {
        let heading = &self.headings[index];
        let end = self.headings[index + 1..]
            .iter()
            .find(|next| next.level <= heading.level)
            .map_or(body_len, |next| next.span.start);
        let end = self
            .footnote_start
            .filter(|&start| start >= heading.body_start)
            .map_or(end, |start| end.min(start));
        heading.span.start..end
    }

    pub(crate) fn append_link_item(
        &self,
        body: &str,
        item: &str,
        separate: bool,
    ) -> Option<String> {
        self.append_to_section(body, "相关链接", item, separate)
    }

    pub(crate) fn append_to_section(
        &self,
        body: &str,
        name: &str,
        text: &str,
        separate: bool,
    ) -> Option<String> {
        if self.blocked() {
            return None;
        }
        let mut matches = self
            .headings
            .iter()
            .enumerate()
            .filter(|(_, heading)| heading.level == 2 && heading.title == name);
        let Some((index, _)) = matches.next() else {
            return self.append_section(body, &format!("## {name}\n\n{text}"));
        };
        if matches.next().is_some() {
            return separate
                .then(|| {
                    let mut heading = format!("{name}（补充）");
                    let mut number = 2;
                    while self
                        .headings
                        .iter()
                        .any(|h| h.level == 2 && h.title == heading)
                    {
                        heading = format!("{name}（补充 {number}）");
                        number += 1;
                    }
                    self.append_section(body, &format!("## {heading}\n\n{text}"))
                })
                .flatten();
        }
        let range = self.section_range(index, body.len());
        if self.intersects_protection(&self.headings[index].span) {
            return None;
        }
        // Insert before trailing blank lines and never inside a fenced or protected block.
        let end = range.start
            + body[range.clone()]
                .trim_end_matches(['\r', '\n', ' ', '\t'])
                .len();
        let end = if end == range.end {
            end
        } else {
            body[end..range.end]
                .find('\n')
                .map_or(range.end, |n| end + n + 1)
        };
        let point = end..end;
        (!self.blocked() && !self.intersects_protection(&point))
            .then(|| insert_block(body, end, text))
    }

    pub(crate) fn append_footnotes(&self, body: &str, notes: &str) -> Option<String> {
        (!self.blocked()).then(|| insert_block(body, body.len(), notes))
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

pub(crate) fn line_ending(body: &str) -> &'static str {
    let newline_count = body.bytes().filter(|&byte| byte == b'\n').count();
    let windows_count = body.matches("\r\n").count();
    if windows_count > 0 && windows_count >= newline_count - windows_count {
        "\r\n"
    } else {
        "\n"
    }
}

pub(super) fn insert_block(body: &str, position: usize, block: &str) -> String {
    let (before, after) = body.split_at(position);
    let lb = line_ending(body);
    let block = block.trim().replace("\r\n", "\n").replace('\n', lb);
    let mut output = String::with_capacity(body.len() + block.len() + 8);
    output.push_str(before);
    if !before.is_empty() && before != "\u{feff}" {
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

pub(crate) fn heading_slug(title: &str) -> String {
    title
        .trim()
        .to_lowercase()
        .chars()
        .filter_map(|c| {
            if c.is_alphanumeric() || matches!(c, '-' | '_') {
                Some(c)
            } else if c.is_whitespace() {
                Some('-')
            } else {
                None
            }
        })
        .collect()
}

pub(crate) fn match_quote<'a>(source: &'a str, quote: &str) -> Option<&'a str> {
    let quote = quote.trim();
    if quote.is_empty() || quote.contains(['\r', '\n']) || quote.chars().count() > 240 {
        return None;
    }
    if let Some(start) = source.find(quote) {
        return source.get(start..start + quote.len());
    }
    let needle = quote.split_whitespace().collect::<Vec<_>>().join(" ");
    for line in source.split(['\r', '\n']) {
        let mut folded = String::new();
        let mut offsets = Vec::new();
        let mut space = None;
        for (index, ch) in line.char_indices() {
            if ch.is_whitespace() {
                if !folded.is_empty() {
                    space.get_or_insert(index);
                }
            } else {
                if let Some(index) = space.take() {
                    offsets.push(index);
                    folded.push(' ');
                }
                offsets.extend(index..index + ch.len_utf8());
                folded.push(ch);
            }
        }
        for (start, _) in folded.match_indices(&needle) {
            let end = start + needle.len() - 1;
            let original = &line[offsets[start]..offsets[end] + 1];
            if original.chars().count() <= 240 {
                return Some(original);
            }
        }
    }
    None
}

pub(crate) fn paragraph_slices(text: &str) -> Vec<&str> {
    let mut paragraphs = Vec::new();
    let mut start = 0;
    let mut position = 0;
    for line in text.split_inclusive('\n') {
        if line.trim().is_empty() {
            if start < position {
                paragraphs.push(text[start..position].trim());
            }
            start = position + line.len();
        }
        position += line.len();
    }
    if start < text.len() {
        paragraphs.push(text[start..].trim());
    }
    paragraphs
}

pub(crate) fn upsert_frontmatter_list(body: &str, key: &str, value: &str) -> Option<String> {
    let parsed = Markdown::parse(body);
    if parsed
        .list(key)
        .iter()
        .any(|item| item.eq_ignore_ascii_case(value))
    {
        return Some(body.to_owned());
    }
    let value = serde_json::to_string(value).ok()?;
    let lb = line_ending(body);
    let bom = if body.starts_with('\u{feff}') { 3 } else { 0 };
    if parsed.content_start == bom {
        if body[bom..]
            .lines()
            .next()
            .is_some_and(|line| line.trim_end() == "---")
        {
            return None;
        }
        return Some(format!(
            "{}---{lb}{key}: [{value}]{lb}---{lb}{}",
            &body[..bom],
            &body[bom..]
        ));
    }
    let lines: Vec<_> = body[bom..parsed.content_start]
        .split_inclusive('\n')
        .scan(bom, |offset, line| {
            let start = *offset;
            *offset += line.len();
            Some((start, line))
        })
        .collect();
    let (closing, _) = *lines.last()?;
    for (index, &(offset, line)) in lines.iter().enumerate().skip(1) {
        let Some((name, rest)) = line.trim_end_matches(['\r', '\n']).split_once(':') else {
            continue;
        };
        if !name.eq_ignore_ascii_case(key) {
            continue;
        }
        let field = without_yaml_comment(rest).trim_end();
        let end = offset + name.len() + 1 + field.len();
        let begin = end - field.trim_start().len();
        let field = &body[begin..end];
        if let Some(inner) = field.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
            let point = begin + 1 + inner.trim_end().len();
            let separator = if inner.trim().is_empty() {
                ""
            } else if inner.trim_end().ends_with(',') {
                " "
            } else {
                ", "
            };
            return Some(format!(
                "{}{separator}{value}{}",
                &body[..point],
                &body[point..]
            ));
        }
        if !field.is_empty() {
            if matches!(
                field.chars().next(),
                Some('[' | '{' | '|' | '>' | '&' | '*')
            ) {
                return None;
            }
            let old = if parsed.list(key).is_empty() {
                String::new()
            } else {
                format!("{field}, ")
            };
            return Some(format!("{}[{old}{value}]{}", &body[..begin], &body[end..]));
        }
        let mut point = offset + line.len();
        let mut indent = "  ";
        for &(start, next) in &lines[index + 1..] {
            let trimmed = next.trim_start();
            if trimmed.starts_with("- ") {
                indent = &next[..next.len() - trimmed.len()];
                point = start + next.len();
            } else if !trimmed.is_empty() && !trimmed.starts_with('#') {
                if next.starts_with(char::is_whitespace) {
                    return None;
                }
                break;
            }
        }
        return Some(format!(
            "{}{indent}- {value}{lb}{}",
            &body[..point],
            &body[point..]
        ));
    }
    Some(format!(
        "{}{key}: [{value}]{lb}{}",
        &body[..closing],
        &body[closing..]
    ))
}

pub(crate) fn remove_conflict_marker(body: &str, id: &str) -> Option<String> {
    let parsed = Markdown::parse(body);
    let mut remove = Vec::new();
    let bom = if body.starts_with('\u{feff}') { 3 } else { 0 };
    let mut in_field = false;
    let mut offset = bom;
    for raw in body[bom..parsed.content_start].split_inclusive('\n') {
        let begin = offset;
        offset += raw.len();
        let line = raw.trim_end_matches(['\r', '\n']);
        let trimmed = line.trim_start();
        if let Some(value) = trimmed.strip_prefix("- ") {
            if in_field && scalar(value) == id {
                let prefix = line.len() - trimmed.len();
                let comment = &value[without_yaml_comment(value).len()..];
                if comment.is_empty() {
                    remove.push((begin..offset, ""));
                } else {
                    remove.push((
                        begin + prefix..begin + prefix + 2 + value.len() - comment.len(),
                        "",
                    ));
                }
            }
            continue;
        }
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        in_field = false;
        let Some((key, rest)) = line.split_once(':') else {
            continue;
        };
        if !matches!(
            key.to_ascii_lowercase().as_str(),
            "wikra_pending_conflicts"
                | "wikra-pending-conflicts"
                | "fmd_pending_conflicts"
                | "fmd-pending-conflicts"
        ) {
            continue;
        }
        let value = without_yaml_comment(rest).trim();
        in_field = value.is_empty();
        let start = begin + key.len() + 1 + rest.len() - rest.trim_start().len();
        if let Some(inner) = value.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
            let mut quote = YamlQuote::default();
            let mut from = 0;
            let mut edits = Vec::new();
            for (i, c) in inner
                .char_indices()
                .chain(std::iter::once((inner.len(), ',')))
            {
                if c == ',' && !quote.update(c, inner[from..i].trim().is_empty()) {
                    let item = &inner[from..i];
                    if scalar(item) == id {
                        let first = from + item.len() - item.trim_start().len();
                        let last = from + item.trim_end().len();
                        let range = if i < inner.len() {
                            first..i + 1
                        } else if from > 0 {
                            from - 1..last
                        } else {
                            first..last
                        };
                        edits.push(range);
                    }
                    from = (i + 1).min(inner.len());
                } else if c != ',' {
                    quote.update(c, inner[from..i].trim().is_empty());
                }
            }
            let mut merged: Vec<Range<usize>> = Vec::new();
            for range in edits {
                if let Some(last) = merged.last_mut().filter(|last| last.end >= range.start) {
                    last.end = last.end.max(range.end);
                } else {
                    merged.push(range);
                }
            }
            remove.extend(
                merged
                    .into_iter()
                    .map(|range| (start + 1 + range.start..start + 1 + range.end, "")),
            );
        } else if scalar(value) == id {
            remove.push((start..start + value.len(), "[]"));
        }
    }
    let mut result = body.to_owned();
    for (range, replacement) in remove.into_iter().rev() {
        result.replace_range(range, replacement);
    }
    (!Markdown::parse(&result)
        .conflicts()
        .iter()
        .any(|value| value == id))
    .then_some(result)
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
    let trimmed = line.trim_start_matches(' ');
    if line.len() - trimmed.len() > 3 {
        return None;
    }
    let line = trimmed;
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
        line: 0,
    }
}

fn link_parts(link: &str) -> (&str, &str) {
    let (target, label) = link.split_once('|').unwrap_or((link, link));
    (target.trim(), label.trim())
}

fn closing_ticks(tail: &str, count: usize) -> Option<usize> {
    let mut offset = 0;
    for line in tail.split_inclusive('\n') {
        if offset > 0
            && (line.trim().is_empty()
                || heading(line).is_some()
                || CodeFence::default().contains(line))
        {
            return None;
        }
        let mut rest = line;
        while let Some(start) = rest.find('\x60') {
            let len = rest[start..]
                .bytes()
                .take_while(|&byte| byte == b'\x60')
                .count();
            rest = &rest[start + len..];
            if len == count {
                return Some(offset + line.len() - rest.len());
            }
        }
        offset += line.len();
    }
    None
}

fn visible_text(
    body: &str,
    position: usize,
    line: &str,
    in_comment: &mut Option<&'static str>,
    code_end: &mut usize,
) -> (String, bool) {
    let mut rest = line;
    let mut visible = String::with_capacity(line.len());
    let mut had_comment = in_comment.is_some();
    while !rest.is_empty() {
        let offset = position + line.len() - rest.len();
        if *code_end > offset {
            let len = (*code_end - offset).min(rest.len());
            visible.extend(std::iter::repeat_n(' ', len));
            rest = &rest[len..];
            continue;
        }
        if let Some(end) = *in_comment {
            let end = rest.find(end).map(|index| index + end.len());
            let len = end.unwrap_or(rest.len());
            visible.extend(std::iter::repeat_n(' ', len));
            rest = &rest[len..];
            if end.is_some() {
                *in_comment = None;
            }
            continue;
        }
        let Some(index) = rest.find(['%', '<', '\x60', '\\']) else {
            visible.push_str(rest);
            break;
        };
        visible.push_str(&rest[..index]);
        rest = &rest[index..];
        if let Some(after) = rest.strip_prefix("<!--") {
            visible.push_str("    ");
            *in_comment = Some("-->");
            had_comment = true;
            rest = after;
        } else if let Some(after) = rest.strip_prefix("%%") {
            visible.push_str("  ");
            *in_comment = Some("%%");
            had_comment = true;
            rest = after;
        } else if rest.starts_with('\x60') {
            let count = rest.bytes().take_while(|&byte| byte == b'\x60').count();
            let after_open = offset + index + count;
            if let Some(len) = closing_ticks(&body[after_open..], count) {
                *code_end = after_open + len;
            } else {
                visible.push_str(&rest[..count]);
                rest = &rest[count..];
            }
        } else {
            let len = if rest.starts_with('\\') {
                1 + rest[1..].chars().next().map_or(0, char::len_utf8)
            } else {
                1
            };
            visible.push_str(&rest[..len]);
            rest = &rest[len..];
        }
    }
    (visible, had_comment)
}

pub(super) fn wiki_links(line: &str) -> Vec<WikiLink> {
    let mut links = Vec::new();
    let mut rest = line;
    while let Some(index) = rest.find(['\x60', '[', '\\']) {
        rest = &rest[index..];
        if let Some(escaped) = rest.strip_prefix('\\') {
            rest = escaped
                .chars()
                .next()
                .map_or("", |c| &escaped[c.len_utf8()..]);
        } else if rest.starts_with('\x60') {
            let count = rest.bytes().take_while(|&b| b == b'\x60').count();
            let end = count + closing_ticks(&rest[count..], count).unwrap_or(0);
            rest = &rest[end..];
        } else if let Some((link, after)) = rest.strip_prefix("[[").and_then(|s| s.split_once("]]"))
        {
            let (target, label) = link_parts(link);
            if !target.is_empty() && !target.contains('[') {
                links.push(WikiLink {
                    target: target.to_owned(),
                    label: label.to_owned(),
                    snippet: line.trim().chars().take(180).collect(),
                    span: line.len() - rest.len()..line.len() - after.len(),
                    line: 0,
                });
            }
            rest = after;
        } else {
            rest = &rest[1..];
        }
    }
    links
}
