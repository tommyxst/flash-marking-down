use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Write};
use std::os::windows::process::CommandExt;
use std::process::{Command, Stdio};
use std::time::Duration;

use serde_json::{Value, json};

use crate::dto::wikra::*;
use crate::mcp::{TOOL_NAMES, VERSIONS, settings_allow};
use crate::platform::display_local_path;

use super::documents::in_directory;
use super::markdown::{Markdown, line_ending};
use super::organize::apply::managed_block;
use super::*;

const STAGE: &str = "wikra_agent";
const SKILL_PATH: &str = "SKILL.md";
const SKILL_NAME: &str = "wikra-lookup";
const SKILL_START: &str = "<!-- wikra-skill:start -->";
const SKILL_END: &str = "<!-- wikra-skill:end -->";
const SKILL_BLOCK: &str = "<!-- wikra-skill:start -->
## 从 Wikra 查证知识

1. 用 Wikra MCP 的 search 找到相关知识页，再用 read_page 阅读。优先处理页首的冲突提醒。
2. 需要证据时，将页面返回的 citationRef 原样传给 get_evidence，检查 verified，注明知识页和短引出处。
3. 默认权限仅开放 Wiki 及其中已有短引。只有用户在 Wikra 中授权的原文目录可检索、读取或返回上下文。
4. 可用资源为 wikra://vault/schema 和 wikra://vault/index。遇到未核验的短引或待核对冲突时，向用户说明依据。
<!-- wikra-skill:end -->";

fn db(err: rusqlite::Error) -> StorageError {
    database_error(STAGE, None, err)
}

fn mcp_exe() -> io::Result<PathBuf> {
    Ok(std::env::current_exe()?.with_file_name("wikra-mcp.exe"))
}

fn powershell_argument(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

impl Storage {
    pub(crate) fn get_agent_integration(&self) -> Result<AgentIntegrationDto, StorageError> {
        let files = &self.workspace.files;
        let exe = mcp_exe().map_err(|err| source_error(STAGE, err))?;
        let vault = display_local_path(files.root_path());
        let command = display_local_path(&exe);
        let snippets = McpSnippetsDto {
            claude_cli: format!(
                "claude mcp add --transport stdio wikra -- {} --vault {}",
                powershell_argument(&command),
                powershell_argument(&vault)
            ),
            mcp_json: serde_json::to_string_pretty(&json!({"mcpServers":{"wikra":{
                "command":command,"args":["--vault",vault]
            }}}))
            .expect("JSON value"),
            codex_toml: format!(
                "[mcp_servers.wikra]\ncommand = {}\nargs = [\"--vault\", {}]\n",
                json!(command),
                json!(vault)
            ),
        };
        let (paths, _) = files
            .markdown_paths()
            .map_err(|err| source_error(STAGE, err))?;
        let mut dirs: BTreeMap<String, usize> = [self.raw_dir(), self.capture_dir()]
            .into_iter()
            .map(|dir| (dir.into(), 0))
            .collect();
        for path in paths {
            let mut parent = path.as_str();
            while let Some((dir, _)) = parent.rsplit_once('/') {
                if !in_directory(dir, self.raw_dir()) && !in_directory(dir, self.capture_dir()) {
                    break;
                }
                *dirs.entry(dir.to_owned()).or_default() += 1;
                parent = dir;
            }
        }
        let raw_dirs = dirs
            .into_iter()
            .map(|(path, file_count)| {
                let is_capture_dir = in_directory(&path, self.capture_dir());
                RawDirOptionDto {
                    path,
                    file_count,
                    is_capture_dir,
                }
            })
            .collect();
        let raw_allow = settings_allow(Some(&self.connection))
            .into_iter()
            .filter(|folder| {
                in_directory(folder, self.raw_dir()) || in_directory(folder, self.capture_dir())
            })
            .collect();
        Ok(AgentIntegrationDto {
            mcp_exe_path: command,
            mcp_exe_exists: exe.is_file(),
            vault_root: vault,
            raw_dir: self.raw_dir().to_owned(),
            raw_dirs,
            raw_allow,
            snippets,
            skill_path: display_local_path(&files.root_path().join(SKILL_PATH)),
            skill_exists: files.markdown_file(SKILL_PATH).is_ok(),
        })
    }

    pub(crate) fn set_mcp_raw_allow(
        &self,
        folders: &[String],
    ) -> Result<Vec<String>, StorageError> {
        let allowed = |folder: &str| {
            in_directory(folder, self.raw_dir()) || in_directory(folder, self.capture_dir())
        };
        let mut selected = Vec::new();
        for folder in folders {
            let requested = normalize_workspace_subdir(Path::new(folder))
                .map_err(|_| invalid(STAGE, "请选择原文或随手记目录"))?;
            let canonical = self
                .workspace
                .files
                .directory_key(Path::new(&requested))
                .map_err(|_| invalid(STAGE, "目录不存在，或不在工作区内"))?;
            if !allowed(&requested) || !allowed(&canonical) {
                return Err(invalid(STAGE, "只可授权原文或随手记目录及其子目录"));
            }
            if !selected
                .iter()
                .any(|old: &String| old.eq_ignore_ascii_case(&canonical))
            {
                selected.push(canonical);
            }
        }
        self.connection
            .execute(
                UPSERT_WORKSPACE_SETTING,
                params!["mcp_raw_allow", json!(selected).to_string()],
            )
            .map_err(db)?;
        Ok(selected)
    }

    pub(crate) fn export_vault_skill(&self) -> Result<ExportSkillResultDto, StorageError> {
        let edit = MarkdownEdit::load(&self.workspace.files, SKILL_PATH, true, STAGE)?;
        if edit.key != SKILL_PATH || is_capture(&self.connection, &edit.key) {
            return Err(invalid(STAGE, "技能文件位置已被占用"));
        }
        ensure_editable(&self.connection, &edit.key, STAGE)?;
        let before = edit.before.as_deref().unwrap_or("");
        let parsed = Markdown::parse(before);
        if parsed.blocked() {
            return Err(conflict(
                STAGE,
                "请先修复 SKILL.md 中未闭合的 Markdown 区块",
            ));
        }
        let lb = line_ending(before);
        let bom = if before.starts_with('\u{feff}') { 3 } else { 0 };
        let named = match parsed.value("name") {
            Some(SKILL_NAME) => before.to_owned(),
            Some(_) => return Err(conflict(STAGE, "SKILL.md 已用于另一项技能")),
            None if parsed.content_start > bom => {
                let front = &before[bom..parsed.content_start];
                let value = front.trim_end_matches(['\r', '\n']);
                let offset = bom + value.rfind('\n').map_or(0, |i| i + 1);
                format!(
                    "{}name: {SKILL_NAME}{lb}{}",
                    &before[..offset],
                    &before[offset..]
                )
            }
            None if before[bom..]
                .lines()
                .next()
                .is_some_and(|line| line.trim_end() == "---") =>
            {
                return Err(conflict(STAGE, "请先修复 SKILL.md 中未闭合的 Frontmatter"));
            }
            None => format!(
                "{}---{lb}name: {SKILL_NAME}{lb}description: 在 Wikra 知识库检索、阅读并核验短引{lb}---{lb}{}",
                &before[..bom],
                &before[bom..]
            ),
        };
        let after = managed_block(&named, SKILL_BLOCK, SKILL_START, SKILL_END)
            .ok_or_else(|| conflict(STAGE, "请检查 SKILL.md 托管块及人工保护标记"))?;
        let action = if before == after {
            "unchanged"
        } else {
            // The current file bytes are the expected version for the atomic replacement.
            write_edit(&edit.path, edit.before.as_deref(), Some(&after)).map_err(|err| {
                if err.kind() == io::ErrorKind::AlreadyExists {
                    conflict(STAGE, "SKILL.md 已在外部被修改，请重试")
                } else {
                    source_error(STAGE, err)
                }
            })?;
            if edit.before.is_some() {
                "updated"
            } else {
                "created"
            }
        };
        Ok(ExportSkillResultDto {
            path: edit.key,
            action,
        })
    }

    pub(crate) fn test_mcp_stdio(&self) -> McpTestResultDto {
        let result = mcp_exe()
            .map_err(|_| "无法定位 Wikra MCP 程序")
            .and_then(|exe| test_stdio(&exe, self.workspace.files.root_path()));
        match result {
            Ok((version, tools)) => McpTestResultDto {
                ok: true,
                protocol_version: Some(version),
                tools,
                error: None,
            },
            Err(message) => McpTestResultDto {
                error: Some(message.into()),
                ..Default::default()
            },
        }
    }
}

fn test_stdio(exe: &Path, vault: &Path) -> Result<(String, Vec<String>), &'static str> {
    if !exe.is_file() {
        return Err("应用目录未找到 wikra-mcp.exe");
    }
    // CREATE_NO_WINDOW keeps the stdio child from opening a console in the desktop app.
    let mut child = Command::new(exe)
        .arg("--vault")
        .arg(vault)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .creation_flags(0x0800_0000)
        .spawn()
        .map_err(|_| "无法启动 Wikra MCP")?;
    let stdin = child.stdin.take().expect("piped stdin");
    let stdout = child.stdout.take().expect("piped stdout");
    let (send, receive) = std::sync::mpsc::channel();
    let worker = std::thread::spawn(move || {
        let _ = send.send(handshake(stdin, BufReader::new(stdout)));
    });
    let result = receive
        .recv_timeout(Duration::from_secs(5))
        .unwrap_or(Err("Wikra MCP stdio 握手超时"));
    if child.try_wait().ok().flatten().is_none() {
        let _ = child.kill();
    }
    let _ = child.wait();
    let _ = worker.join();
    result
}

fn handshake(
    mut input: impl Write,
    mut output: impl BufRead,
) -> Result<(String, Vec<String>), &'static str> {
    let send = |input: &mut dyn Write, message: Value| {
        writeln!(input, "{message}")
            .and_then(|_| input.flush())
            .map_err(|_| "无法写入 MCP 请求")
    };
    let receive = |output: &mut dyn BufRead, id: u32| -> Result<Value, &'static str> {
        let mut line = String::new();
        output
            .read_line(&mut line)
            .map_err(|_| "无法读取 MCP 响应")?;
        let response: Value = serde_json::from_str(&line).map_err(|_| "MCP 响应不是有效 JSON")?;
        if response["jsonrpc"] != "2.0" || response["id"] != id || !response["result"].is_object() {
            return Err("MCP 响应与请求不匹配");
        }
        Ok(response)
    };
    send(
        &mut input,
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{
            "protocolVersion":VERSIONS[0],"capabilities":{},"clientInfo":{"name":"wikra","version":env!("CARGO_PKG_VERSION")}
        }}),
    )?;
    let hello = receive(&mut output, 1)?;
    let version = hello["result"]["protocolVersion"]
        .as_str()
        .filter(|version| VERSIONS.contains(version))
        .ok_or("MCP 返回了不支持的协议版本")?;
    send(
        &mut input,
        json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
    )?;
    send(
        &mut input,
        json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}),
    )?;
    let list = receive(&mut output, 2)?;
    let tools: Vec<_> = list["result"]["tools"]
        .as_array()
        .ok_or("MCP 工具列表缺失")?
        .iter()
        .filter_map(|tool| tool["name"].as_str().map(str::to_owned))
        .collect();
    if !TOOL_NAMES
        .iter()
        .all(|name| tools.iter().any(|tool| tool == name))
    {
        return Err("MCP 未提供预期的查询工具");
    }
    Ok((version.to_owned(), tools))
}
