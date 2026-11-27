use super::*;
use crate::storage::search::fts_trigram_phrase;
use crate::storage::wikra::documents::{Catalog, Document, LinkResolution, file_stem};

const MATERIAL_LIMIT: usize = 12_000;
const INDEX_LIMIT: usize = 12;

pub(crate) struct CompileContext {
    pub(in crate::storage::wikra) catalog: Catalog,
    pub(in crate::storage::wikra) candidates: Vec<usize>,
    pub(in crate::storage::wikra) source_count: usize,
    pub(in crate::storage::wikra) single_step: bool,
    pub(in crate::storage::wikra) input: serde_json::Value,
}

impl Storage {
    pub(crate) fn prepare_compilation(
        &mut self,
        sources: &[Source],
    ) -> Result<CompileContext, StorageError> {
        let mut length = 0;
        let source_count = sources
            .iter()
            .take_while(|source| {
                length += source.text.chars().count();
                length <= MATERIAL_LIMIT
            })
            .count();
        if source_count == 0 {
            return Err(invalid(
                STAGE,
                "首个待整理段落超过 12,000 字，请缩小选择范围",
            ));
        }
        let sources = &sources[..source_count];
        let material = sources
            .iter()
            .map(|source| source.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        let query = terms(&material).join(" OR ");
        if !query.is_empty() {
            self.workspace.search_reader()?.rebuild_search_index()?;
        }
        let catalog = Catalog::load(self)?;
        let candidates = shortlist(&self.connection, &catalog, &material, &query)?;
        let index_map = candidates
            .iter()
            .enumerate()
            .map(|(rank, &id)| {
                let doc = &catalog.docs[id];
                let aliases = doc
                    .parsed
                    .list("aliases")
                    .iter()
                    .take(4)
                    .map(|alias| field(alias, 48))
                    .collect::<Vec<_>>()
                    .join(", ");
                format!(
                    "c{} | {} | {} | {} | {} | {}",
                    rank + 1,
                    doc.path.replace('|', " "),
                    field(&doc.title, 120),
                    aliases,
                    field(&doc.doc_type, 24),
                    field(summary(doc), 60)
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        let schema = catalog
            .paths
            .get("schema.md")
            .or_else(|| catalog.paths.get("wiki/schema.md"))
            .map_or_else(String::new, |&id| {
                schema_slice(&catalog.docs[id], &catalog, &candidates)
            });
        let sources: Vec<_> = sources.iter().enumerate().map(|(index, source)| {
            json!({"id": format!("s{}", index + 1), "title": tidy(&source.title, 120), "text": source.text})
        }).collect();
        let sections = candidates
            .iter()
            .enumerate()
            .map(|(index, &id)| {
                (
                    format!("c{}", index + 1),
                    json!(heading_refs(&catalog.docs[id])),
                )
            })
            .collect::<serde_json::Map<_, _>>();
        let input = json!({"sources": sources, "index_map": index_map, "schema": schema, "sections": sections});
        let ambiguous_link = Markdown::parse(&material).links.iter().any(|link| {
            matches!(
                catalog.resolve_link("", &link.target),
                LinkResolution::Ambiguous(_)
            )
        });
        let single_step = !catalog.docs.iter().any(Document::knowledge_page)
            || (candidates.len() <= 3
                && input.to_string().chars().count() <= 16_000
                && !ambiguous_link
                && candidates.iter().all(|&id| {
                    let doc = &catalog.docs[id];
                    !review_page(doc)
                        && unique_name(&catalog, &doc.title, id)
                        && unique_name(&catalog, file_stem(&doc.path), id)
                        && doc
                            .parsed
                            .list("aliases")
                            .iter()
                            .all(|alias| unique_name(&catalog, alias, id))
                }));
        Ok(CompileContext {
            catalog,
            candidates,
            source_count,
            single_step,
            input,
        })
    }
}

fn field(text: &str, limit: usize) -> String {
    tidy(text, limit).replace('|', " ")
}

fn summary(doc: &Document) -> &str {
    doc.parsed
        .value("summary")
        .or_else(|| {
            doc.body[doc.parsed.content_start..]
                .lines()
                .map(str::trim)
                .find(|line| !line.is_empty() && !line.starts_with(['#', '<', '[']))
                .map(|line| line.trim_start_matches("> "))
        })
        .unwrap_or("")
}

pub(super) fn heading_refs(doc: &Document) -> Vec<serde_json::Value> {
    doc.parsed.headings.iter().filter(|heading| heading.level == 2).take(24).enumerate()
        .map(|(index, heading)| json!({"id": format!("h{}", index + 1), "title": tidy(&heading.title, 100)}))
        .collect()
}

pub(super) fn review_page(doc: &Document) -> bool {
    doc.doc_type.eq_ignore_ascii_case("decision")
        || in_directory(&doc.path, "wiki/decisions")
        || doc.parsed.core()
}

fn unique_name(catalog: &Catalog, name: &str, id: usize) -> bool {
    catalog
        .named(name)
        .iter()
        .copied()
        .filter(|&id| catalog.docs[id].knowledge_page())
        .eq([id])
}

fn contains_term(text: &str, term: &str) -> bool {
    if term.is_ascii() {
        let word = |ch: char| ch.is_ascii_alphanumeric() || ch == '_';
        term.bytes().filter(u8::is_ascii_alphanumeric).count() >= 3
            && text.match_indices(term).any(|(start, _)| {
                !text[..start].chars().next_back().is_some_and(word)
                    && !text[start + term.len()..].chars().next().is_some_and(word)
            })
    } else {
        term.chars().filter(|ch| ch.is_alphanumeric()).count() >= 2 && text.contains(term)
    }
}

fn terms(text: &str) -> Vec<String> {
    let mut result = Vec::new();
    for token in text.split(|c: char| !c.is_alphanumeric()) {
        if token.is_ascii() || token.chars().count() <= 8 {
            if let Some(phrase) = fts_trigram_phrase(token)
                && !result.contains(&phrase)
            {
                result.push(phrase);
            }
        } else {
            let chars: Vec<_> = token.chars().collect();
            for trigram in chars.windows(3).step_by(2).take(8) {
                let phrase = fts_trigram_phrase(&trigram.iter().collect::<String>())
                    .expect("three characters");
                if !result.contains(&phrase) {
                    result.push(phrase);
                }
                if result.len() == 8 {
                    break;
                }
            }
        }
        if result.len() == 8 {
            break;
        }
    }
    result
}

fn shortlist(
    conn: &Connection,
    catalog: &Catalog,
    material: &str,
    query: &str,
) -> Result<Vec<usize>, StorageError> {
    let eligible = |id: usize| {
        let doc = &catalog.docs[id];
        doc.knowledge_page() && !doc.parsed.managed_false()
    };
    let mut scores: HashMap<usize, i32> = HashMap::new();
    for link in Markdown::parse(material).links {
        if let Some(id) = catalog.resolve("", &link.target).filter(|&id| eligible(id)) {
            scores.entry(id).or_insert(100);
        }
    }
    let lower = material.to_lowercase();
    for (id, doc) in catalog
        .docs
        .iter()
        .enumerate()
        .filter(|(id, _)| eligible(*id))
    {
        let signal = |term: &str| {
            unique_name(catalog, term, id) && contains_term(&lower, &term.to_lowercase())
        };
        let points = if signal(&doc.title) {
            40
        } else if doc.parsed.list("aliases").iter().any(|term| signal(term)) {
            30
        } else {
            0
        };
        if points != 0 {
            *scores.entry(id).or_default() += points;
        }
    }
    if !query.is_empty() {
        let mut rows = conn
            .prepare(
                "SELECT relative_path FROM search_documents_fts WHERE search_documents_fts MATCH ?1
             ORDER BY bm25(search_documents_fts), relative_path",
            )
            .map_err(db)?;
        let mut rank = 0;
        for row in rows
            .query_map([query], |row| row.get::<_, String>(0))
            .map_err(db)?
        {
            let path = row.map_err(db)?;
            if let Some(&id) = catalog
                .paths
                .get(&path.to_lowercase())
                .filter(|&&id| eligible(id))
            {
                *scores.entry(id).or_default() += (35 - rank).max(10);
                rank += 1;
                if rank >= 32 {
                    break;
                }
            }
        }
    }
    let mut ranked: Vec<_> = scores.keys().copied().collect();
    ranked.sort_unstable_by_key(|id| (std::cmp::Reverse(scores[id]), *id));
    let seeds: HashSet<_> = ranked.into_iter().take(5).collect();
    let mut neighbors = HashSet::new();
    for (from, doc) in catalog
        .docs
        .iter()
        .enumerate()
        .filter(|(id, _)| eligible(*id))
    {
        for (to, kind, _, _) in catalog.relations(doc) {
            if kind != "link" || from == to || !eligible(to) {
                continue;
            }
            if seeds.contains(&from) {
                neighbors.insert(to);
            }
            if seeds.contains(&to) {
                neighbors.insert(from);
            }
        }
    }
    for id in neighbors {
        *scores.entry(id).or_default() += 15;
    }
    let mut ranked: Vec<_> = scores.keys().copied().collect();
    ranked.sort_unstable_by_key(|id| (std::cmp::Reverse(scores[id]), *id));
    ranked.truncate(INDEX_LIMIT);
    Ok(ranked)
}

fn schema_slice(schema: &Document, catalog: &Catalog, candidates: &[usize]) -> String {
    if schema.body.len() <= 4096 {
        return schema.body.clone();
    }
    let mut categories = HashSet::new();
    for &id in candidates {
        let kind = catalog.docs[id].doc_type.to_lowercase();
        categories.insert(kind.clone());
        categories.insert(super::super::documents::type_label(&kind).to_owned());
    }
    let headings: Vec<_> = schema
        .parsed
        .headings
        .iter()
        .filter(|h| h.level == 2)
        .collect();
    let mut result = String::new();
    let prefix_end = headings.first().map_or(schema.body.len(), |h| h.span.start);
    push_schema(
        &mut result,
        &schema.body[..prefix_end],
        if headings.is_empty() { 4096 } else { 1200 },
    );
    for (index, heading) in headings.iter().enumerate() {
        let lower = heading.title.to_lowercase();
        if ["全局", "通用", "global", "通则", "总则"]
            .iter()
            .any(|key| lower.contains(key))
            || categories.iter().any(|kind| lower.contains(kind))
        {
            let end = headings
                .get(index + 1)
                .map_or(schema.body.len(), |h| h.span.start);
            push_schema(&mut result, &schema.body[heading.span.start..end], 4096);
        }
    }
    result
}

fn push_schema(output: &mut String, text: &str, cap: usize) {
    let mut end = text.len().min(cap.saturating_sub(output.len()));
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    output.push_str(&text[..end]);
}
