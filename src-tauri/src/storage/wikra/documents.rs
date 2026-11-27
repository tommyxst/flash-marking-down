use std::collections::BTreeSet;

use crate::dto::wikra::*;

use super::markdown::{Footnote, Markdown, match_quote, paragraph_slices};
use super::*;

const STAGE: &str = "wikra_document";

struct Clip {
    id: String,
    title: String,
    url: Option<String>,
    saved_at: String,
}

pub(crate) struct Document {
    pub path: String,
    pub title: String,
    pub doc_type: String,
    pub raw: bool,
    clip: Option<Clip>,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
    pub pending_conflict_ids: Vec<String>,
    pub body: String,
    pub parsed: Markdown,
}

impl Document {
    fn is_clip(&self) -> bool {
        self.raw && self.doc_type == "clip"
    }

    fn source_url(&self) -> Option<String> {
        self.clip
            .as_ref()
            .and_then(|clip| clip.url.clone())
            .or_else(|| {
                ["source_url", "source-url", "sourceurl", "url", "source"]
                    .into_iter()
                    .filter_map(|key| self.parsed.value(key))
                    .find_map(crate::platform::parse_http_url)
                    .map(|url| url.to_string())
            })
    }

    pub(crate) fn meta(&self) -> bool {
        meta_path(&self.path)
    }

    pub(crate) fn knowledge_page(&self) -> bool {
        !self.raw && !self.meta() && in_directory(&self.path, "wiki")
    }
}

pub(crate) fn meta_path(path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or_default();
    ["index.md", "log.md", "SCHEMA.md", "SKILL.md"]
        .iter()
        .any(|meta| name.eq_ignore_ascii_case(meta))
}

pub(crate) struct Catalog {
    raw_dir: String,
    pub docs: Vec<Document>,
    pub paths: HashMap<String, usize>,
    pub names: HashMap<String, Vec<usize>>,
    pub unreadable: usize,
}

pub(crate) enum LinkResolution {
    Unique(usize),
    Ambiguous(Vec<usize>),
    MissingAnchor(usize),
    Missing,
}

impl LinkResolution {
    pub(super) fn page(self) -> Option<usize> {
        match self {
            Self::Unique(id) | Self::MissingAnchor(id) => Some(id),
            _ => None,
        }
    }
}

impl Catalog {
    pub(crate) fn load(storage: &Storage) -> Result<Self, StorageError> {
        let files = storage.workspace_files();
        let (entries, unreadable) = files
            .markdown_entries()
            .map_err(|err| source_error(STAGE, err))?;
        Self::load_from(
            &files,
            Some(&storage.connection),
            storage.raw_dir(),
            storage.capture_dir(),
            entries,
            unreadable,
        )
    }

    pub(crate) fn load_from(
        files: &WorkspaceFiles,
        conn: Option<&Connection>,
        raw_dir: &str,
        capture_dir: &str,
        mut entries: Vec<crate::workspace::MarkdownEntry>,
        mut unreadable: usize,
    ) -> Result<Self, StorageError> {
        let clips_dir = format!("{raw_dir}/clips");
        let db = |err| database_error(STAGE, None, err);
        let mut captured = HashMap::new();
        let mut clips = HashMap::new();
        let mut conflicts: HashMap<String, BTreeSet<String>> = HashMap::new();
        if let Some(conn) = conn {
            let mut notes = conn.prepare(
            "SELECT relative_path, min(captured_at) FROM notes WHERE draft_id <> 'manual' GROUP BY relative_path",
        ).map_err(db)?;
            captured = notes
                .query_map([], |row| {
                    Ok((row.get::<_, String>(0)?.to_lowercase(), row.get(1)?))
                })
                .map_err(db)?
                .collect::<Result<_, _>>()
                .map_err(db)?;
            let mut query = conn
                .prepare("SELECT raw_path, id, title, url, saved_at FROM inbox_items")
                .map_err(db)?;
            clips = query
                .query_map([], |row| {
                    Ok((
                        row.get::<_, String>(0)?.to_lowercase(),
                        Clip {
                            id: row.get(1)?,
                            title: row.get(2)?,
                            url: row.get(3)?,
                            saved_at: row.get(4)?,
                        },
                    ))
                })
                .map_err(db)?
                .collect::<Result<_, _>>()
                .map_err(db)?;

            let mut query = conn
                .prepare(
                    "SELECT id, current_path, incoming_path FROM conflicts WHERE status = 'open'",
                )
                .map_err(db)?;
            let rows = query
                .query_map([], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, Option<String>>(1)?,
                        row.get::<_, Option<String>>(2)?,
                    ))
                })
                .map_err(db)?;
            for (id, current, incoming) in rows.filter_map(Result::ok) {
                for path in [current, incoming].into_iter().flatten() {
                    conflicts
                        .entry(physical_path(&path).to_lowercase())
                        .or_default()
                        .insert(id.clone());
                }
            }
        }
        entries.sort_unstable_by(|a, b| a.relative_path.cmp(&b.relative_path));
        let mut docs = Vec::with_capacity(entries.len());
        let mut paths = HashMap::with_capacity(entries.len());
        let mut names: HashMap<String, Vec<usize>> = HashMap::new();
        let memory;
        let timestamp_conn = if let Some(conn) = conn {
            conn
        } else {
            memory = Connection::open_in_memory().map_err(db)?;
            &memory
        };
        let mut timestamp = timestamp_conn
            .prepare("SELECT strftime('%Y-%m-%dT%H:%M:%SZ', ?1, 'unixepoch')")
            .map_err(db)?;
        for entry in entries {
            let path = entry.relative_path;
            let Ok(body) = files
                .read_markdown(&path)
                .and_then(|bytes| String::from_utf8(bytes).map_err(io::Error::other))
            else {
                unreadable += 1;
                continue;
            };
            let key = path.to_lowercase();
            let parsed = Markdown::parse(&body);
            let clip = clips.remove(&key);
            let raw = in_directory(&path, raw_dir)
                || in_directory(&path, capture_dir)
                || captured.contains_key(&key)
                || clip.is_some();
            let doc_type = if raw {
                if clip.is_some() || in_directory(&path, &clips_dir) {
                    "clip"
                } else {
                    "note"
                }
            } else {
                parsed
                    .value("type")
                    .or_else(|| parsed.value("doctype"))
                    .unwrap_or_else(|| default_doc_type(&path))
            }
            .to_owned();
            let stem = file_stem(&path);
            let title = clip
                .as_ref()
                .map(|clip| clip.title.as_str())
                .or_else(|| parsed.value("title"))
                .or(parsed.title.as_deref())
                .unwrap_or(stem)
                .to_owned();
            let modified = timestamp
                .query_row([entry.mtime_ns / 10_000_000 - 11_644_473_600], |row| {
                    row.get(0)
                })
                .map_err(db)?;
            let updated_at = parsed
                .value("updated")
                .or_else(|| parsed.value("updatedat"))
                .map(str::to_owned)
                .or(modified);
            let created_at = clip
                .as_ref()
                .map(|clip| clip.saved_at.clone())
                .or_else(|| captured.get(&key).cloned())
                .or_else(|| {
                    parsed
                        .value("created")
                        .or_else(|| parsed.value("createdat"))
                        .map(str::to_owned)
                })
                .or_else(|| {
                    timestamp
                        .query_row(
                            [entry.created_filetime / 10_000_000 - 11_644_473_600],
                            |row| row.get(0),
                        )
                        .ok()
                        .flatten()
                });
            let mut pending_conflict_ids = parsed.conflicts();
            for id in conflicts.remove(&key).into_iter().flatten() {
                if !pending_conflict_ids.contains(&id) {
                    pending_conflict_ids.push(id);
                }
            }
            let index = docs.len();
            for name in std::iter::once(stem)
                .chain(std::iter::once(title.as_str()))
                .chain(parsed.list("aliases").iter().map(String::as_str))
            {
                let matching = names.entry(name.to_lowercase()).or_default();
                if matching.last() != Some(&index) {
                    matching.push(index);
                }
            }
            paths.insert(key, index);
            docs.push(Document {
                path,
                title,
                doc_type,
                raw,
                clip,
                created_at,
                updated_at,
                body,
                parsed,
                pending_conflict_ids,
            });
        }
        Ok(Self {
            raw_dir: raw_dir.to_owned(),
            docs,
            paths,
            names,
            unreadable,
        })
    }

    pub(crate) fn resolve(&self, source: &str, target: &str) -> Option<usize> {
        match self.resolve_link(source, target) {
            LinkResolution::Unique(id) => Some(id),
            _ => None,
        }
    }

    pub(crate) fn resolve_link(&self, source: &str, target: &str) -> LinkResolution {
        let target = physical_path(target.trim().trim_matches(['<', '>']));
        let (target, anchor) = target.split_once('#').map_or_else(
            || {
                target
                    .rsplit_once('^')
                    .filter(|(_, id)| {
                        !id.is_empty() && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
                    })
                    .map_or((target, None), |(path, id)| (path, Some(format!("^{id}"))))
            },
            |(path, anchor)| (path, Some(percent_decode(anchor))),
        );
        let matching = if target.is_empty() {
            self.paths
                .get(&source.to_lowercase())
                .copied()
                .into_iter()
                .collect()
        } else {
            self.match_path(source, target)
        };
        match matching.as_slice() {
            [] => LinkResolution::Missing,
            &[id]
                if anchor
                    .as_ref()
                    .is_some_and(|anchor| !self.docs[id].parsed.has_anchor(anchor)) =>
            {
                LinkResolution::MissingAnchor(id)
            }
            &[id] => LinkResolution::Unique(id),
            _ => LinkResolution::Ambiguous(matching),
        }
    }

    pub(super) fn named(&self, name: &str) -> &[usize] {
        self.names
            .get(&name.to_lowercase())
            .map_or(&[], Vec::as_slice)
    }

    pub(super) fn stem_matches(&self, stem: &str) -> Vec<usize> {
        let key = stem.to_lowercase();
        self.named(stem)
            .iter()
            .copied()
            .filter(|&id| file_stem(&self.docs[id].path).to_lowercase() == key)
            .collect()
    }

    fn match_path(&self, source: &str, target: &str) -> Vec<usize> {
        let decoded = percent_decode(target);
        [target, decoded.as_str()]
            .into_iter()
            .find_map(|target| {
                if target.is_empty() || target.contains(':') || target.starts_with(['/', '\\']) {
                    return None;
                }
                let explicit = target.contains(['/', '\\']);
                if !explicit {
                    let stem = file_stem(target);
                    let files = self.stem_matches(stem);
                    if !files.is_empty() {
                        return Some(files);
                    }
                    let names = self.named(stem);
                    if !names.is_empty() {
                        return Some(names.to_vec());
                    }
                }
                let relative = join_reference(parent_dir(source), target);
                let root = (!target.starts_with('.'))
                    .then(|| join_reference("", target))
                    .flatten();
                let root_first = target.starts_with("wiki/")
                    || target
                        .strip_prefix(self.raw_dir.as_str())
                        .is_some_and(|suffix| suffix.starts_with('/'));
                let options = if root_first {
                    [root, relative]
                } else {
                    [relative, root]
                };
                if let Some(id) = options
                    .into_iter()
                    .flatten()
                    .find_map(|key| self.paths.get(&key.to_lowercase()).copied())
                {
                    return Some(vec![id]);
                }
                if !explicit || target.starts_with('.') {
                    return None;
                }
                let suffix = format!("/{}", join_reference("", target)?.to_lowercase());
                let mut matches: Vec<_> = self
                    .paths
                    .iter()
                    .filter(|(path, _)| path.ends_with(&suffix))
                    .map(|(_, id)| *id)
                    .collect();
                matches.sort_unstable();
                (!matches.is_empty()).then_some(matches)
            })
            .unwrap_or_default()
    }

    pub(crate) fn citation(&self, doc: &Document, note: &Footnote) -> DocCitationDto {
        let raw_path = note
            .target
            .as_deref()
            .and_then(|target| self.resolve_link(&doc.path, target).page())
            .map(|id| &self.docs[id])
            .filter(|doc| doc.raw)
            .map(|doc| doc.path.clone());
        DocCitationDto {
            id: format!("{}#{}", doc.path, note.ref_label),
            ref_label: note.ref_label.clone(),
            raw_path: raw_path.unwrap_or_default(),
            label: note.label.clone(),
            quote: note.quote.clone(),
        }
    }

    pub(crate) fn citation_context(
        &self,
        files: &WorkspaceFiles,
        page: &Document,
        note: &Footnote,
        allows: impl FnOnce(&str) -> bool,
    ) -> CitationContextDto {
        let mut response = CitationContextDto {
            raw_path: self.citation(page, note).raw_path,
            quote: note.quote.clone(),
            verified: false,
            paragraph: None,
            prev: None,
            next: None,
            reason: None,
        };
        let marker = note
            .target
            .as_deref()
            .and_then(|target| target.split_once(NOTE_ANCHOR))
            .map(|(_, marker)| marker);
        let raw = files
            .markdown_file(&response.raw_path)
            .ok()
            .filter(|(key, _)| key.eq_ignore_ascii_case(&response.raw_path))
            .and_then(|(_, path)| fs::read_to_string(path).ok());
        let Some(raw) = raw
            .as_deref()
            .and_then(|body| marker.map_or(Some(body), |marker| block_body(body, marker)))
        else {
            response.reason = Some("原文缺失");
            return response;
        };
        let paragraphs = paragraph_slices(raw);
        let matched = paragraphs
            .iter()
            .enumerate()
            .find_map(|(index, paragraph)| {
                match_quote(paragraph, &note.quote).map(|quote| (index, quote))
            });
        response.verified = matched.is_some();
        if !allows(&response.raw_path) {
            response.reason = Some("raw 未授权");
        } else if let Some((index, quote)) = matched {
            let current = paragraphs[index];
            let start = quote.as_ptr() as usize - current.as_ptr() as usize;
            let preceding: String = current[..start]
                .chars()
                .rev()
                .take(240)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect();
            let current: String = preceding
                .chars()
                .chain(current[start..].chars())
                .take(800)
                .collect();
            response.paragraph = Some(current);
            response.prev = index.checked_sub(1).map(|i| {
                paragraphs[i]
                    .chars()
                    .rev()
                    .take(198)
                    .collect::<Vec<_>>()
                    .into_iter()
                    .rev()
                    .collect()
            });
            response.next = paragraphs
                .get(index + 1)
                .map(|next| next.chars().take(198).collect());
        } else {
            response.reason = Some("引文与原文不符");
        }
        response
    }

    pub(crate) fn relations<'a>(
        &'a self,
        doc: &'a Document,
    ) -> impl Iterator<Item = (usize, &'static str, &'a str, &'a str)> + 'a {
        doc.parsed
            .links
            .iter()
            .filter(|_| !doc.meta())
            .filter_map(move |link| {
                self.resolve_link(&doc.path, &link.target)
                    .page()
                    .map(|to| (to, "link", link.label.as_str(), link.snippet.as_str()))
            })
            .chain(
                doc.parsed
                    .footnotes
                    .iter()
                    .filter(|_| !doc.meta())
                    .filter_map(move |note| {
                        self.resolve_link(&doc.path, note.target.as_deref()?)
                            .page()
                            .map(|to| (to, "citation", note.label.as_str(), note.quote.as_str()))
                    }),
            )
    }

    pub(crate) fn incoming(&self, wiki_only: bool) -> Vec<Vec<DocBacklinkDto>> {
        let mut incoming = (0..self.docs.len()).map(|_| Vec::new()).collect::<Vec<_>>();
        for (from, doc) in self
            .docs
            .iter()
            .enumerate()
            .filter(|(_, doc)| !wiki_only || !doc.raw)
        {
            let mut seen = HashSet::new();
            for (to, _, _, snippet) in self.relations(doc) {
                if from != to && seen.insert(to) {
                    incoming[to].push(DocBacklinkDto {
                        page_path: doc.path.clone(),
                        title: doc.title.clone(),
                        excerpt: snippet.to_owned(),
                    });
                }
            }
        }
        incoming
    }
}

impl Storage {
    pub(crate) fn list_wikilink_candidates(
        &self,
    ) -> Result<Vec<WikilinkCandidateDto>, StorageError> {
        let catalog = Catalog::load(self)?;
        Ok(catalog
            .docs
            .iter()
            .enumerate()
            .filter(|(_, doc)| !doc.raw && !doc.meta())
            .map(|(id, doc)| {
                let stem = file_stem(&doc.path);
                let ambiguous_stem = catalog.stem_matches(stem) != [id];
                let target = if ambiguous_stem {
                    let path = &doc.path[..doc.path.len() - 3];
                    path.rmatch_indices('/')
                        .map(|(index, _)| &path[index + 1..])
                        .skip(1)
                        .find(|candidate| {
                            let name = format!("{candidate}.md").to_lowercase();
                            catalog
                                .paths
                                .keys()
                                .filter(|key| **key == name || key.ends_with(&format!("/{name}")))
                                .count()
                                == 1
                        })
                        .unwrap_or(path)
                } else {
                    stem
                };
                let link_text = if target == doc.title {
                    format!("[[{target}]]")
                } else {
                    format!("[[{target}|{}]]", doc.title.replace(['|', '[', ']'], " "))
                };
                WikilinkCandidateDto {
                    path: doc.path.clone(),
                    stem: stem.to_owned(),
                    title: doc.title.clone(),
                    aliases: doc.parsed.list("aliases").to_vec(),
                    folder: parent_dir(&doc.path).to_owned(),
                    type_label: type_label(&doc.doc_type),
                    link_text,
                    ambiguous_stem,
                }
            })
            .collect())
    }

    pub(crate) fn resolve_wikilink(
        &self,
        from_path: &str,
        target: &str,
    ) -> Result<WikilinkResolutionDto, StorageError> {
        let catalog = Catalog::load(self)?;
        let from = if from_path.is_empty() {
            String::new()
        } else {
            crate::workspace::normalize_workspace_subdir(Path::new(from_path))
                .map_err(|_| invalid(STAGE, "链接起点不在工作区"))?
        };
        let target = target
            .strip_prefix("[[")
            .and_then(|s| s.strip_suffix("]]"))
            .unwrap_or(target);
        let target = target.split_once('|').map_or(target, |(name, _)| name);
        let (status, matching) = match catalog.resolve_link(&from, target) {
            LinkResolution::Unique(i) => ("unique", vec![i]),
            LinkResolution::Ambiguous(ids) => ("ambiguous", ids),
            _ => ("missing", Vec::new()),
        };
        Ok(WikilinkResolutionDto {
            status,
            matches: matching
                .into_iter()
                .map(|id| {
                    let doc = &catalog.docs[id];
                    WikilinkMatchDto {
                        path: doc.path.clone(),
                        title: doc.title.clone(),
                        type_label: type_label(&doc.doc_type),
                        updated_at: doc.updated_at.clone(),
                    }
                })
                .collect(),
        })
    }

    pub(crate) fn read_citation_context(
        &self,
        page_path: &str,
        ref_label: &str,
    ) -> Result<CitationContextDto, StorageError> {
        let catalog = Catalog::load(self)?;
        let key = self
            .workspace
            .files
            .markdown_key(page_path)
            .map_err(|err| source_error(STAGE, err))?;
        let doc = catalog
            .paths
            .get(&key.to_lowercase())
            .map(|&i| &catalog.docs[i])
            .filter(|doc| !doc.raw)
            .ok_or_else(|| invalid(STAGE, "找不到知识页"))?;
        let note = doc
            .parsed
            .footnotes
            .iter()
            .find(|note| note.ref_label == ref_label)
            .ok_or_else(|| invalid(STAGE, "找不到该页引文"))?;
        Ok(catalog.citation_context(&self.workspace.files, doc, note, |_| true))
    }

    pub(crate) fn list_library_tree(&self) -> Result<LibraryTreeDto, StorageError> {
        let catalog = Catalog::load(self)?;
        let incoming = catalog.incoming(true);
        let mut wiki_pages = Vec::new();
        let mut raw_files = Vec::new();
        for (doc, backlinks) in catalog.docs.into_iter().zip(incoming) {
            if doc.raw {
                let is_clip = doc.is_clip();
                raw_files.push(RawFileSummaryDto {
                    id: doc
                        .clip
                        .as_ref()
                        .map_or_else(|| format!("file:{}", doc.path), |clip| clip.id.clone()),
                    kind: if is_clip { "clip" } else { "note" },
                    kind_label: raw_label(is_clip),
                    source_url: doc.source_url(),
                    path: doc.path,
                    title: doc.title,
                    created_at: doc.created_at,
                    linked_page_paths: backlinks
                        .into_iter()
                        .map(|backlink| backlink.page_path)
                        .collect(),
                });
            } else {
                wiki_pages.push(LibraryDocSummaryDto {
                    title: doc.title,
                    folder: parent_dir(&doc.path).to_owned(),
                    path: doc.path,
                    type_label: type_label(&doc.doc_type),
                    doc_type: doc.doc_type,
                    tags: doc.parsed.list("tags").to_vec(),
                    aliases: doc.parsed.list("aliases").to_vec(),
                    status: doc.parsed.value("status").map(str::to_owned),
                    updated_at: doc.updated_at,
                    citation_count: doc.parsed.footnotes.len(),
                    pending_conflict_count: doc.pending_conflict_ids.len(),
                    wikra_core: doc.parsed.core(),
                });
            }
        }
        Ok(LibraryTreeDto {
            raw_dir: self.raw_dir().to_owned(),
            wiki_pages,
            raw_files,
        })
    }

    pub(crate) fn read_document_detail(
        &self,
        relative_path: &str,
    ) -> Result<DocumentDetailDto, StorageError> {
        let key = self
            .workspace
            .files
            .markdown_key(relative_path)
            .map_err(|err| source_error(STAGE, err))?;
        let mut catalog = Catalog::load(self)?;
        let index = *catalog
            .paths
            .get(&key.to_lowercase())
            .ok_or_else(|| source_error(STAGE, "file disappeared or is not UTF-8 Markdown"))?;
        let backlinks = catalog.incoming(false).swap_remove(index);
        let doc = &catalog.docs[index];
        let citations = doc
            .parsed
            .footnotes
            .iter()
            .map(|note| catalog.citation(doc, note))
            .collect();
        let linked_wiki_paths = if doc.raw {
            backlinks
                .iter()
                .filter(|backlink| {
                    let source = catalog.paths[&backlink.page_path.to_lowercase()];
                    !catalog.docs[source].raw
                })
                .map(|backlink| backlink.page_path.clone())
                .collect()
        } else {
            Vec::new()
        };
        let source_url = doc.source_url();
        let doc = catalog.docs.swap_remove(index);
        let tasks = wiki::document_tasks(&doc.body);
        Ok(DocumentDetailDto {
            kind: if doc.raw { "raw" } else { "wiki" },
            path: doc.path,
            title: doc.title,
            content_hash: revision_hash(doc.body.as_bytes()),
            body: doc.body,
            updated_at: doc.updated_at,
            type_label: type_label(&doc.doc_type),
            doc_type: doc.doc_type,
            aliases: doc.parsed.list("aliases").to_vec(),
            status: doc.parsed.value("status").map(str::to_owned),
            supersedes: doc.parsed.list("supersedes").to_vec(),
            tags: doc.parsed.list("tags").to_vec(),
            wikra_core: doc.parsed.core(),
            source_url,
            pending_conflict_ids: doc.pending_conflict_ids,
            has_protected_section: doc.parsed.protected(),
            outline: doc.parsed.outline,
            tasks,
            citations,
            backlinks,
            linked_wiki_paths,
        })
    }

    pub(crate) fn create_markdown_document(
        &mut self,
        path: &str,
        body: &str,
    ) -> Result<String, StorageError> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|err| database_error(STAGE, None, err))?;
        let edit = MarkdownEdit::load(&self.workspace.files, path, true, STAGE)?;
        if edit.before.is_some() {
            return Err(conflict(STAGE, "文件已存在，请打开原文件或使用新文件名"));
        }
        edit.commit(tx, Some(body), STAGE)
    }

    pub(crate) fn save_markdown_document(
        &mut self,
        path: &str,
        expected_hash: &str,
        body: &str,
    ) -> Result<String, StorageError> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|err| database_error(STAGE, None, err))?;
        let edit = MarkdownEdit::load(&self.workspace.files, path, false, STAGE)?;
        if edit
            .before
            .as_ref()
            .is_none_or(|before| revision_hash(before.as_bytes()) != expected_hash)
        {
            return Err(conflict(STAGE, "Markdown 已在外部被修改，请刷新后重试"));
        }
        ensure_editable(&tx, &edit.key, STAGE)?;
        edit.commit(tx, Some(body), STAGE)
    }

    pub(crate) fn append_human_understanding(
        &mut self,
        path: &str,
        insight: &str,
    ) -> Result<String, StorageError> {
        if insight.trim().is_empty() {
            return Err(invalid(STAGE, "请先填写你的理解"));
        }
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|err| database_error(STAGE, None, err))?;
        let edit = MarkdownEdit::load(&self.workspace.files, path, false, STAGE)?;
        if in_directory(&edit.key, &self.workspace.raw_dir)
            || in_directory(&edit.key, &self.workspace.capture_dir)
            || is_capture(&tx, &edit.key)
        {
            return Err(invalid(STAGE, "请选择一篇知识页保存理解"));
        }
        ensure_editable(&tx, &edit.key, STAGE)?;
        let before = edit.before.as_deref().expect("existing Markdown");
        let after = Markdown::parse(before)
            .append_insight(before, insight)
            .ok_or_else(|| conflict(STAGE, "请检查人工保护标记和 Markdown 区块是否完整"))?;
        edit.commit(tx, Some(&after), STAGE)
    }

    pub(crate) fn get_knowledge_graph(
        &self,
        include_meta: bool,
    ) -> Result<KnowledgeGraphDto, StorageError> {
        let catalog = Catalog::load(self)?;
        let mut edges = Vec::new();
        let mut gaps = Vec::new();
        let mut degree = vec![0; catalog.docs.len()];
        let mut incoming = vec![HashSet::new(); catalog.docs.len()];
        let mut evidence: HashMap<(usize, &str), BTreeSet<usize>> = HashMap::new();
        let mut linked = HashSet::new();
        let included = |doc: &Document| include_meta || !doc.meta();
        let mut seen_edges = HashSet::new();
        let mut seen_gaps = HashSet::new();
        for (from, doc) in catalog
            .docs
            .iter()
            .enumerate()
            .filter(|(_, doc)| included(doc))
        {
            for (reference, (to, kind, label, snippet)) in catalog.relations(doc).enumerate() {
                let target = &catalog.docs[to];
                if from == to
                    || !included(target)
                    || doc.raw && target.raw
                    || !seen_edges.insert((from, to, kind, label, snippet))
                {
                    continue;
                }
                degree[from] += 1;
                degree[to] += 1;
                if !doc.raw && !doc.meta() {
                    incoming[to].insert(from);
                    if kind == "citation" && target.raw && !snippet.is_empty() {
                        evidence.entry((to, snippet)).or_default().insert(from);
                    }
                }
                if kind == "link" {
                    linked.insert((from.min(to), from.max(to)));
                }
                edges.push(GraphEdgeDto {
                    id: format!("{kind}:{}:{}:{reference}", doc.path, target.path),
                    from: doc.path.clone(),
                    to: target.path.clone(),
                    kind,
                    label: label.to_owned(),
                    snippet: snippet.to_owned(),
                });
            }
            if doc.raw || doc.meta() {
                continue;
            }
            for link in &doc.parsed.links {
                if catalog
                    .resolve_link(&doc.path, &link.target)
                    .page()
                    .is_none()
                {
                    let id = format!("missing:{}:{}", doc.path, link.target);
                    if !seen_gaps.insert(id.clone()) {
                        continue;
                    }
                    gaps.push(GraphGapDto {
                        id,
                        title: link.label.clone(),
                        reason: format!("《{}》的双链找不到目标", doc.title),
                        question: format!(
                            "应为「{}」建立新知识页，还是链接到已有页面？",
                            link.label
                        ),
                        from_path: doc.path.clone(),
                        to_path: String::new(),
                    });
                }
            }
        }
        let mut evidence: Vec<_> = evidence.into_iter().collect();
        evidence.sort_unstable_by_key(|(source, _)| *source);
        for ((source, _), pages) in evidence {
            let pages: Vec<_> = pages.into_iter().collect();
            for pair in pages.windows(2) {
                if linked.contains(&(pair[0], pair[1])) {
                    continue;
                }
                let (a, b) = (&catalog.docs[pair[0]], &catalog.docs[pair[1]]);
                let id = format!("evidence:{}:{}", a.path, b.path);
                if !seen_gaps.insert(id.clone()) {
                    continue;
                }
                gaps.push(GraphGapDto {
                    id,
                    title: format!("{} · {}", a.title, b.title),
                    reason: format!("两页都引用了《{}》的同一段原文", catalog.docs[source].title),
                    question: format!("这份原文如何联系《{}》和《{}》的结论？", a.title, b.title),
                    from_path: a.path.clone(),
                    to_path: b.path.clone(),
                });
            }
        }
        let dismissed = self.dismissed_ids(false)?;
        gaps.retain(|gap| !dismissed.contains(&gap.id));
        let nodes = catalog
            .docs
            .iter()
            .enumerate()
            .filter(|(id, doc)| included(doc) && (!doc.raw || degree[*id] > 0))
            .map(|(id, doc)| GraphNodeDto {
                id: doc.path.clone(),
                title: doc.title.clone(),
                doc_type: doc.doc_type.clone(),
                degree: degree[id],
                is_hub: !doc.raw && incoming[id].len() >= 3,
                is_orphan: !doc.raw && !doc.meta() && degree[id] == 0,
            })
            .collect();
        Ok(KnowledgeGraphDto { nodes, edges, gaps })
    }

    pub(crate) fn global_search(
        &self,
        query: &str,
        limit: usize,
    ) -> Result<Vec<GlobalSearchHitDto>, StorageError> {
        let query = query.trim().to_lowercase();
        if query.is_empty() || limit == 0 {
            return Ok(Vec::new());
        }
        let catalog = Catalog::load(self)?;
        let has_wiki_dir = catalog
            .docs
            .iter()
            .any(|doc| !doc.raw && in_directory(&doc.path, "wiki"));
        let mut matches = Vec::new();
        for doc in &catalog.docs {
            let (kind, kind_label) = if !doc.raw
                && (in_directory(&doc.path, "wiki") || (!has_wiki_dir && !doc.meta()))
            {
                ("wiki", "知识页")
            } else if doc.raw
                && (in_directory(&doc.path, self.raw_dir())
                    || in_directory(&doc.path, self.capture_dir())
                    || doc.clip.is_some())
            {
                ("raw", raw_label(doc.is_clip()))
            } else {
                continue;
            };
            let title_match = doc.title.to_lowercase().contains(&query);
            let excerpt = find_excerpt(&doc.body, &query);
            if title_match || excerpt.is_some() {
                matches.push((
                    if title_match { 0 } else { 2 },
                    GlobalSearchHitDto {
                        kind,
                        kind_label,
                        title: doc.title.clone(),
                        subtitle: excerpt.unwrap_or_else(|| doc.path.clone()),
                        target_path: doc.path.clone(),
                        citation_id: None,
                    },
                ));
            }
            if kind == "wiki" {
                for note in &doc.parsed.footnotes {
                    if let Some(excerpt) = find_excerpt(&note.quote, &query).or_else(|| {
                        note.label
                            .to_lowercase()
                            .contains(&query)
                            .then(|| note.quote.clone())
                    }) {
                        matches.push((
                            1,
                            GlobalSearchHitDto {
                                kind: "citation",
                                kind_label: "原文引文",
                                title: note.label.clone(),
                                subtitle: excerpt,
                                target_path: doc.path.clone(),
                                citation_id: Some(format!("{}#{}", doc.path, note.ref_label)),
                            },
                        ));
                    }
                }
            }
        }
        matches.sort_by_key(|(rank, _)| *rank);
        Ok(matches
            .into_iter()
            .take(limit)
            .map(|(_, hit)| hit)
            .collect())
    }
}

pub(crate) fn in_directory(path: &str, dir: &str) -> bool {
    path.get(..dir.len())
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case(dir))
        && (path.len() == dir.len() || path.as_bytes().get(dir.len()) == Some(&b'/'))
}

pub(crate) fn parent_dir(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |(parent, _)| parent)
}

pub(crate) fn file_stem(path: &str) -> &str {
    let name = path.rsplit('/').next().unwrap_or(path);
    name.rsplit_once('.')
        .filter(|(_, ext)| ext.eq_ignore_ascii_case("md"))
        .map_or(name, |(stem, _)| stem)
}

fn join_reference(parent: &str, target: &str) -> Option<String> {
    let mut parts: Vec<_> = parent.split('/').filter(|s| !s.is_empty()).collect();
    for part in target.split(['/', '\\']) {
        match part {
            ".." => {
                parts.pop()?;
            }
            "." | "" => (),
            other => parts.push(other),
        }
    }
    let mut path = parts.join("/");
    if !path.to_ascii_lowercase().ends_with(".md") {
        path.push_str(".md");
    }
    Some(path)
}

fn percent_decode(value: &str) -> String {
    let mut input = value.as_bytes();
    let mut result = Vec::with_capacity(input.len());
    while let [first, rest @ ..] = input {
        if *first == b'%'
            && let [a, b, after @ ..] = rest
            && let (Some(a), Some(b)) = ((*a as char).to_digit(16), (*b as char).to_digit(16))
        {
            result.push((a * 16 + b) as u8);
            input = after;
        } else {
            result.push(*first);
            input = rest;
        }
    }
    String::from_utf8(result).unwrap_or_else(|_| value.to_owned())
}

fn default_doc_type(path: &str) -> &'static str {
    for (dir, kind) in [
        ("wiki/decisions", "decision"),
        ("wiki/concepts", "concept"),
        ("wiki/syntheses", "synthesis"),
        ("wiki/entities", "entity"),
    ] {
        if in_directory(path, dir) {
            return kind;
        }
    }
    "page"
}

pub(crate) fn type_label(doc_type: &str) -> &'static str {
    match doc_type {
        "decision" => "决策",
        "concept" => "概念",
        "synthesis" => "综述",
        "entity" => "实体",
        "note" => "随手笔记",
        "clip" => "收藏原文",
        _ => "知识页",
    }
}

fn raw_label(clip: bool) -> &'static str {
    if clip { "收藏原文" } else { "随手笔记" }
}

pub(crate) fn find_excerpt(text: &str, lowercase_query: &str) -> Option<String> {
    let lowercase = text.to_lowercase();
    let start = lowercase.find(lowercase_query)?;
    let position = lowercase[..start].chars().count();
    Some(
        text.chars()
            .skip(position.saturating_sub(28))
            .take(160)
            .collect(),
    )
}
