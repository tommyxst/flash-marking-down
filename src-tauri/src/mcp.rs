use std::collections::HashMap;
use std::ffi::OsStr;
use std::fs;
use std::io::{self, BufRead, Read, Write};
use std::path::{Path, PathBuf};

use rusqlite::{Connection, OpenFlags, OptionalExtension};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::storage::wikra::documents::{Catalog, Document, find_excerpt, in_directory};
use crate::storage::wikra::organize::apply::index_block;
use crate::storage::{APPLICATION_ID, SCHEMA_VERSION, physical_path};
use crate::workspace::{
    DEFAULT_CAPTURE_DIR, DEFAULT_RAW_DIR, WorkspaceFiles, normalize_workspace_subdir,
};

pub(crate) const VERSIONS: [&str; 3] = ["2025-11-25", "2025-06-18", "2025-03-26"];
pub(crate) const TOOL_NAMES: [&str; 3] = ["search", "read_page", "get_evidence"];
const SCHEMA_URI: &str = "wikra://vault/schema";
const INDEX_URI: &str = "wikra://vault/index";

pub fn main_stdio() -> std::process::ExitCode {
    let mut args = std::env::args_os().skip(1);
    let root = match args.next().as_deref() {
        Some(arg) if arg == OsStr::new("--vault") => args.next(),
        None => std::env::var_os("WIKRA_VAULT"),
        _ => None,
    };
    let Some(root) = root.filter(|_| args.next().is_none()) else {
        eprintln!("Usage: wikra-mcp --vault <absolute path>");
        return std::process::ExitCode::FAILURE;
    };
    let result = Server::open(Path::new(&root))
        .and_then(|mut server| server.serve(io::stdin().lock(), io::stdout().lock()));
    if result.is_err() {
        eprintln!("Wikra MCP: 无法打开知识库或 stdio 已断开");
        std::process::ExitCode::FAILURE
    } else {
        std::process::ExitCode::SUCCESS
    }
}

struct Server {
    files: WorkspaceFiles,
    cache: Option<Cache>,
}

struct Cache {
    signature: (usize, i64, u128),
    raw_dir: String,
    capture_dir: String,
    db: bool,
    catalog: Catalog,
}

impl Server {
    fn open(root: &Path) -> io::Result<Self> {
        Ok(Self {
            files: WorkspaceFiles::open_read_only(root)?,
            cache: None,
        })
    }

    fn serve(&mut self, reader: impl BufRead, mut writer: impl Write) -> io::Result<()> {
        for message in reader.lines() {
            let response = match serde_json::from_str::<Value>(&message?) {
                Ok(request) => self.request(request),
                Err(_) => Some(rpc_error(Value::Null, -32700, "Parse error")),
            };
            if let Some(response) = response {
                serde_json::to_writer(&mut writer, &response)?;
                writer.write_all(b"\n")?;
                writer.flush()?;
            }
        }
        Ok(())
    }

    fn request(&mut self, request: Value) -> Option<Value> {
        let id = request.get("id");
        if request["jsonrpc"] != "2.0"
            || request["method"].as_str().is_none()
            || id.is_some_and(|id| !id.is_string() && !id.is_number())
        {
            return Some(rpc_error(Value::Null, -32600, "Invalid Request"));
        }
        let method = request["method"].as_str()?;
        let id = id?.clone(); // JSON-RPC notifications, including initialized, have no response.
        let params = &request["params"];
        if !params.is_null() && !params.is_object() {
            return Some(rpc_error(id, -32602, "Invalid params"));
        }
        let result = match method {
            "initialize" => {
                let Some(version) = params["protocolVersion"].as_str() else {
                    return Some(rpc_error(id, -32602, "Invalid params"));
                };
                let protocol = VERSIONS
                    .iter()
                    .find(|&&supported| supported == version)
                    .copied()
                    .unwrap_or(VERSIONS[0]);
                Ok(
                    json!({"protocolVersion":protocol, "capabilities":{"tools":{},"resources":{}},
                    "serverInfo":{"name":"wikra","version":env!("CARGO_PKG_VERSION")}}),
                )
            }
            "ping" => Ok(json!({})),
            "tools/list" => Ok(json!({"tools":tools()})),
            "tools/call" => self.call(params).map_err(|error| error.code()),
            "resources/list" => Ok(json!({"resources":[
                {"uri":SCHEMA_URI,"name":"Vault schema","mimeType":"text/markdown"},
                {"uri":INDEX_URI,"name":"Vault index","mimeType":"text/markdown"}
            ]})),
            "resources/read" => self.resource(params).map_err(|error| error.code()),
            _ => Err((-32601, "Method not found")),
        };
        Some(match result {
            Ok(result) => json!({"jsonrpc":"2.0","id":id,"result":result}),
            Err((code, message)) => rpc_error(id, code, message),
        })
    }

    fn view(&mut self) -> Result<View<'_>, ToolError> {
        let mut db = readonly_database(&self.files);
        let (entries, unreadable) = self
            .files
            .markdown_entries()
            .map_err(|_| ToolError::Unavailable)?;
        let signature = (
            entries.len(),
            entries.iter().map(|e| e.mtime_ns).max().unwrap_or(0),
            entries.iter().map(|e| e.file_size as u128).sum(),
        );
        let (mut raw_dir, mut capture_dir) = dirs(db.as_ref());
        let reuse = self.cache.as_ref().is_some_and(|cache| {
            cache.signature == signature
                && cache.raw_dir == raw_dir
                && cache.capture_dir == capture_dir
                && cache.db == db.is_some()
        });
        if !reuse {
            let catalog = match Catalog::load_from(
                &self.files,
                db.as_ref(),
                &raw_dir,
                &capture_dir,
                entries,
                unreadable,
            ) {
                Ok(catalog) => catalog,
                Err(_) if db.take().is_some() => {
                    (raw_dir, capture_dir) = dirs(None);
                    let (entries, unreadable) = self
                        .files
                        .markdown_entries()
                        .map_err(|_| ToolError::Unavailable)?;
                    Catalog::load_from(
                        &self.files,
                        None,
                        &raw_dir,
                        &capture_dir,
                        entries,
                        unreadable,
                    )
                    .map_err(|_| ToolError::Unavailable)?
                }
                Err(_) => return Err(ToolError::Unavailable),
            };
            self.cache = Some(Cache {
                signature,
                raw_dir,
                capture_dir,
                db: db.is_some(),
                catalog,
            });
        }
        let cache = self.cache.as_ref().expect("catalog populated");
        let authorized = settings_allow(db.as_ref())
            .into_iter()
            .filter(|folder| {
                in_directory(folder, &cache.raw_dir) || in_directory(folder, &cache.capture_dir)
            })
            .collect();
        let (conflicts, registered_raw) = database_metadata(db.as_ref());
        Ok(View {
            files: &self.files,
            cache,
            authorized,
            conflicts,
            registered_raw,
            db: db.is_some(),
        })
    }

    fn call(&mut self, params: &Value) -> Result<Value, ToolError> {
        let name = params["name"].as_str().ok_or(ToolError::Params)?;
        let empty = json!({});
        let args = params.get("arguments").unwrap_or(&empty);
        if !args.is_object() {
            return Err(ToolError::Params);
        }
        let response = match name {
            "search" => {
                let args: Search = parse_args(args)?;
                if args.query.trim().is_empty() || !(1..=20).contains(&args.limit) {
                    return Err(ToolError::Params);
                }
                self.view().and_then(|view| view.search(args))
            }
            "read_page" => {
                let args: ReadPage = parse_args(args)?;
                if args.path.is_some() == args.title.is_some() || args.max_chars == 0 {
                    return Err(ToolError::Params);
                }
                self.view().and_then(|view| view.read(args))
            }
            "get_evidence" => {
                let args: Evidence = parse_args(args)?;
                self.view()
                    .and_then(|view| view.evidence(&args.citation_ref))
            }
            _ => return Err(ToolError::Params),
        };
        match response {
            Ok((value, prefix)) => {
                let text = format!(
                    "{prefix}{}",
                    serde_json::to_string_pretty(&value).expect("JSON value")
                );
                Ok(json!({"content":[{"type":"text","text":text}], "structuredContent":value}))
            }
            Err(ToolError::Params) => Err(ToolError::Params),
            Err(error) => {
                Ok(json!({"isError":true,"content":[{"type":"text","text":error.message()}]}))
            }
        }
    }

    fn resource(&mut self, params: &Value) -> Result<Value, ToolError> {
        let uri = params["uri"].as_str().ok_or(ToolError::Params)?;
        let view = self.view()?;
        let catalog = &view.cache.catalog;
        let text = match uri {
            SCHEMA_URI => {
                let id = catalog
                    .paths
                    .get("schema.md")
                    .or_else(|| catalog.paths.get("wiki/schema.md"))
                    .ok_or(ToolError::Unavailable)?;
                let doc = &catalog.docs[*id];
                if view.is_raw(doc) {
                    return Err(ToolError::Unavailable);
                }
                doc.body.clone()
            }
            INDEX_URI => index_block(catalog, true, |doc| !view.is_raw(doc)),
            _ => return Err(ToolError::Unavailable),
        };
        Ok(json!({"contents":[{"uri":uri,"mimeType":"text/markdown","text":text}]}))
    }
}

fn rpc_error(id: Value, code: i32, message: &str) -> Value {
    json!({"jsonrpc":"2.0","id":id,"error":{"code":code,"message":message}})
}

#[derive(Clone, Copy)]
enum ToolError {
    Params,
    Unavailable,
    Ambiguous,
}

impl ToolError {
    fn message(self) -> &'static str {
        match self {
            Self::Params => "Invalid params",
            Self::Unavailable => "页面不存在、不可读取或未获得授权",
            Self::Ambiguous => "名称对应多篇页面，请使用路径",
        }
    }

    fn code(self) -> (i32, &'static str) {
        (
            if matches!(self, Self::Params) {
                -32602
            } else {
                -32002
            },
            self.message(),
        )
    }
}

fn parse_args<T: serde::de::DeserializeOwned>(value: &Value) -> Result<T, ToolError> {
    serde_json::from_value(value.clone()).map_err(|_| ToolError::Params)
}

fn readonly_database(files: &WorkspaceFiles) -> Option<Connection> {
    let path = files.existing_database_path().ok()?;
    let mut header = [0; 20];
    fs::File::open(path).ok()?.read_exact(&mut header).ok()?;
    let mut flags = OpenFlags::SQLITE_OPEN_READ_ONLY;
    let conn = if header[18..20] == [2, 2] {
        let sidecar = |suffix: &str| {
            let mut name = path.as_os_str().to_os_string();
            name.push(suffix);
            PathBuf::from(name)
        };
        let wal = sidecar("-wal");
        let shm = sidecar("-shm");
        match (wal.try_exists().ok()?, shm.try_exists().ok()?) {
            (true, true) => Connection::open_with_flags(path, flags).ok()?,
            (false, false) => {
                // SQLite may otherwise create WAL sidecars even on a read-only connection.
                let shell = crate::platform::display_local_path(path);
                let mut uri = tauri::Url::from_file_path(&shell).ok()?;
                uri.set_query(Some("immutable=1"));
                flags |= OpenFlags::SQLITE_OPEN_URI;
                let conn = Connection::open_with_flags(uri.as_str(), flags).ok()?;
                if wal.try_exists().ok()? || shm.try_exists().ok()? {
                    return None;
                }
                conn
            }
            _ => return None,
        }
    } else {
        Connection::open_with_flags(path, flags).ok()?
    };
    conn.execute_batch("PRAGMA query_only = ON; PRAGMA busy_timeout = 2000;")
        .ok()?;
    let application = conn
        .pragma_query_value(None, "application_id", |row| row.get::<_, i64>(0))
        .ok()?;
    let version = conn
        .pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
        .ok()?;
    (application == APPLICATION_ID && version == SCHEMA_VERSION).then_some(conn)
}

fn setting(conn: Option<&Connection>, key: &str) -> Option<String> {
    conn?
        .query_row(
            "SELECT value FROM workspace_settings WHERE key = ?1",
            [key],
            |row| row.get(0),
        )
        .optional()
        .ok()
        .flatten()
}

pub(crate) fn settings_allow(conn: Option<&Connection>) -> Vec<String> {
    setting(conn, "mcp_raw_allow")
        .and_then(|json| serde_json::from_str::<Vec<String>>(&json).ok())
        .unwrap_or_default()
        .into_iter()
        .filter_map(|folder| normalize_workspace_subdir(Path::new(&folder)).ok())
        .collect()
}

fn dirs(conn: Option<&Connection>) -> (String, String) {
    let get = |key, fallback: &str| {
        setting(conn, key)
            .and_then(|value| normalize_workspace_subdir(Path::new(&value)).ok())
            .unwrap_or_else(|| fallback.into())
    };
    (
        get("raw_dir", DEFAULT_RAW_DIR),
        get("capture_dir", DEFAULT_CAPTURE_DIR),
    )
}

type ConflictTopics = HashMap<String, Vec<(String, String)>>;

fn database_metadata(
    conn: Option<&Connection>,
) -> (ConflictTopics, std::collections::HashSet<String>) {
    let mut topics = HashMap::<_, Vec<_>>::new();
    let mut raw = std::collections::HashSet::new();
    if let Some(conn) = conn {
        if let Ok(mut statement) = conn.prepare("SELECT id, topic, current_path, incoming_path FROM conflicts WHERE status = 'open' ORDER BY id")
            && let Ok(rows) = statement.query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?, row.get::<_, Option<String>>(3)?)))
        {
            for (id, topic, current, incoming) in rows.filter_map(Result::ok) {
                for path in current.into_iter().chain(incoming) {
                    topics.entry(physical_path(&path).to_lowercase()).or_default().push((id.clone(), topic.clone()));
                }
            }
        }
        if let Ok(mut statement) = conn.prepare("SELECT raw_path FROM inbox_items UNION SELECT relative_path FROM notes WHERE draft_id <> 'manual'")
            && let Ok(rows) = statement.query_map([], |row| row.get::<_, String>(0))
        {
            raw.extend(rows.filter_map(Result::ok).map(|path| path.to_lowercase()));
        }
    }
    (topics, raw)
}

struct View<'a> {
    files: &'a WorkspaceFiles,
    cache: &'a Cache,
    authorized: Vec<String>,
    conflicts: ConflictTopics,
    registered_raw: std::collections::HashSet<String>,
    db: bool,
}

type ToolResult = Result<(Value, String), ToolError>;

impl View<'_> {
    fn is_raw(&self, doc: &Document) -> bool {
        doc.raw || self.registered_raw.contains(&doc.path.to_lowercase())
    }

    fn allows_raw(&self, path: &str) -> bool {
        !path.is_empty()
            && self
                .authorized
                .iter()
                .any(|folder| in_directory(path, folder))
    }

    fn visible(&self, doc: &Document) -> bool {
        if self.is_raw(doc) {
            self.allows_raw(&doc.path) && !doc.meta()
        } else {
            !doc.meta()
        }
    }

    fn by_path(&self, path: &str) -> Result<&Document, ToolError> {
        let key = self
            .files
            .markdown_key(path)
            .map_err(|_| ToolError::Unavailable)?;
        self.cache
            .catalog
            .paths
            .get(&key.to_lowercase())
            .map(|&id| &self.cache.catalog.docs[id])
            .filter(|doc| self.visible(doc))
            .ok_or(ToolError::Unavailable)
    }

    fn pending(&self, doc: &Document) -> Vec<Value> {
        let mut seen = std::collections::HashSet::new();
        let database = self
            .conflicts
            .get(&doc.path.to_lowercase())
            .into_iter()
            .flatten()
            .filter(|(id, _)| seen.insert(id.clone()))
            .map(|(id, topic)| json!({"id":id,"topic":topic}))
            .collect::<Vec<_>>();
        database
            .into_iter()
            .chain(
                doc.parsed
                    .conflicts()
                    .into_iter()
                    .filter(|id| seen.insert(id.clone()))
                    .map(|id| json!({"id":id,"topic":"待核对冲突"})),
            )
            .collect()
    }

    fn search(&self, request: Search) -> ToolResult {
        let folder = request
            .folder
            .as_deref()
            .map(|folder| normalize_workspace_subdir(Path::new(folder)))
            .transpose()
            .map_err(|_| ToolError::Params)?;
        let query = request.query.trim().to_lowercase();
        let mut hits = Vec::new();
        for doc in self
            .cache
            .catalog
            .docs
            .iter()
            .filter(|doc| self.visible(doc))
        {
            if request
                .kind
                .as_ref()
                .is_some_and(|kind| !kind.eq_ignore_ascii_case(&doc.doc_type))
                || folder
                    .as_ref()
                    .is_some_and(|folder| !in_directory(&doc.path, folder))
            {
                continue;
            }
            let title = doc.title.to_lowercase();
            let best = title == query;
            let title_match = title.contains(&query)
                || doc
                    .parsed
                    .list("aliases")
                    .iter()
                    .any(|alias| alias.to_lowercase().contains(&query));
            let body = &doc.body[doc.parsed.content_start..];
            let excerpt = find_excerpt(body, &query);
            if title_match || excerpt.is_some() {
                hits.push((
                    !best,
                    !title_match,
                    json!({"path":doc.path,"title":doc.title,"type":doc.doc_type,
                    "snippet":excerpt.unwrap_or_else(|| body.trim().chars().take(160).collect()),
                    "pendingConflictCount":self.pending(doc).len()}),
                ));
            }
        }
        hits.sort_by_key(|(exact, title, _)| (*exact, *title));
        let hits: Vec<_> = hits
            .into_iter()
            .take(request.limit)
            .map(|(_, _, hit)| hit)
            .collect();
        Ok((
            json!({"hits":hits,"indexState":if self.db {"db"} else {"files_only"}}),
            String::new(),
        ))
    }

    fn read(&self, request: ReadPage) -> ToolResult {
        let catalog = &self.cache.catalog;
        let doc = if let Some(path) = request.path.as_deref() {
            self.by_path(path)?
        } else {
            let title = request
                .title
                .as_deref()
                .unwrap_or_default()
                .trim()
                .to_lowercase();
            let mut matching = catalog
                .names
                .get(&title)
                .into_iter()
                .flatten()
                .map(|&id| &catalog.docs[id])
                .filter(|doc| self.visible(doc));
            let page = matching.next().ok_or(ToolError::Unavailable)?;
            if matching.next().is_some() {
                return Err(ToolError::Ambiguous);
            }
            page
        };
        let pending = self.pending(doc);
        let mut warnings = Vec::new();
        if !pending.is_empty() {
            warnings.push(format!("这页有 {} 处待核对冲突", pending.len()));
        }
        if doc.parsed.blocked() {
            warnings.push("页面含未闭合的 Markdown 结构".into());
        }
        let prefix = if warnings.is_empty() {
            String::new()
        } else {
            format!("{}\n\n", warnings.join("\n"))
        };
        let body = &doc.body[doc.parsed.content_start..];
        let end = body
            .char_indices()
            .nth(request.max_chars)
            .map_or(body.len(), |(i, _)| i);
        let citations: Vec<_> = doc.parsed.footnotes.iter().map(|note| {
            let citation = catalog.citation(doc, note);
            json!({"citationRef":citation.id,"label":citation.ref_label,"rawPath":citation.raw_path,"quote":citation.quote})
        }).collect();
        let backlinks: Vec<_> = catalog
            .docs
            .iter()
            .filter(|source| {
                source.path != doc.path
                    && self.visible(source)
                    && catalog
                        .relations(source)
                        .any(|(to, _, _, _)| catalog.docs[to].path == doc.path)
            })
            .take(20)
            .map(|source| json!({"path":source.path,"title":source.title}))
            .collect();
        let frontmatter = &doc.body[..doc.parsed.content_start];
        Ok((
            json!({"path":doc.path,"title":doc.title,"frontmatter":frontmatter,
            "body":&body[..end],"truncated":end < body.len(),"warnings":warnings,
            "pendingConflicts":pending,"citations":citations,"backlinks":backlinks}),
            prefix,
        ))
    }

    fn evidence(&self, reference: &str) -> ToolResult {
        let (path, label) = reference
            .rsplit_once('#')
            .filter(|(path, label)| !path.is_empty() && !label.is_empty())
            .ok_or(ToolError::Params)?;
        let doc = self.by_path(path)?;
        if self.is_raw(doc) {
            return Err(ToolError::Unavailable);
        }
        let note = doc
            .parsed
            .footnotes
            .iter()
            .find(|note| note.ref_label == label)
            .ok_or(ToolError::Unavailable)?;
        let citation = self
            .cache
            .catalog
            .citation_context(self.files, doc, note, |path| self.allows_raw(path));
        let context = citation.paragraph.as_ref().map(|body| {
            citation
                .prev
                .as_deref()
                .into_iter()
                .chain(std::iter::once(body.as_str()))
                .chain(citation.next.as_deref())
                .collect::<Vec<_>>()
                .join("\n\n")
                .chars()
                .take(1200)
                .collect::<String>()
        });
        let mut result = json!({"citationRef":reference,"rawPath":citation.raw_path,"quote":citation.quote,
            "verified":citation.verified,"context":if context.is_some() {"paragraph"} else {"quote"},"paragraph":context});
        if let Some(reason @ ("raw 未授权" | "原文缺失")) = citation.reason {
            result["reason"] = json!(reason);
        }
        Ok((result, String::new()))
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Search {
    query: String,
    #[serde(rename = "type")]
    kind: Option<String>,
    folder: Option<String>,
    #[serde(default = "default_limit")]
    limit: usize,
}

fn default_limit() -> usize {
    8
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct ReadPage {
    path: Option<String>,
    title: Option<String>,
    #[serde(default = "default_max_chars")]
    max_chars: usize,
}

fn default_max_chars() -> usize {
    20_000
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Evidence {
    citation_ref: String,
}

fn tools() -> Value {
    json!([
        {"name":"search","description":"搜索已授权的知识库 Markdown","inputSchema":
         {"type":"object","properties":{"query":{"type":"string"},"type":{"type":"string"},"folder":{"type":"string"},"limit":{"type":"integer","minimum":1,"maximum":20,"default":8}},"required":["query"],"additionalProperties":false}},
        {"name":"read_page","description":"按路径或唯一标题读取页面，先查看冲突提醒","inputSchema":
         {"type":"object","properties":{"path":{"type":"string"},"title":{"type":"string"},"maxChars":{"type":"integer","minimum":1,"default":20000}},
          "oneOf":[{"required":["path"]},{"required":["title"]}],"additionalProperties":false}},
        {"name":"get_evidence","description":"核验知识页短引并返回已授权的原文上下文","inputSchema":
         {"type":"object","properties":{"citationRef":{"type":"string"}},"required":["citationRef"],"additionalProperties":false}}
    ])
}
