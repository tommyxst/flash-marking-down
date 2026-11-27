import { invoke } from '@tauri-apps/api/core'
import type {
  AgentIntegration,
  CaptureCommitMode,
  CaptureCommitResult,
  ChangesetDetail,
  ChangesetItem,
  ChatTurn,
  CitationContext,
  ConflictItem,
  ConflictStatus,
  ConversationDetail,
  ConversationSummary,
  CredentialReference,
  CredentialSlot,
  DocumentDetail,
  DocumentKind,
  DocumentMapping,
  DocWidthMode,
  DrawerCounts,
  ExportSkillResult,
  GlobalSearchHit,
  HotkeyStatus,
  InboxItemDetail,
  InboxItemSummary,
  IndexRebuildResult,
  IntegrationsConfig,
  KnowledgeGraph,
  LibraryTree,
  LintFinding,
  LintFixResult,
  LintReport,
  McpTestResult,
  MemoryConfig,
  MemoryItem,
  ModelConfig,
  NoteDetail,
  NoteSummary,
  OpenedWorkspace,
  OrganizeOverview,
  RecentCapture,
  RecordSource,
  RevertChangesetResult,
  SearchPage,
  SemanticLintPreview,
  SemanticLintScope,
  TaskList,
  TeachbackTurn,
  ThemeName,
  WikiDocument,
  WikilinkCandidate,
  WikilinkResolution,
  WikiSuggestion,
} from '../m2'

export interface AppSettings {
  version: number
  lastWorkspaceRoot: string | null
  lastCaptureDir: string
  hotkey: string
  autoOrganize: boolean
  autoSyncMemory: boolean
  theme: ThemeName
  docWidth: DocWidthMode
  allowNoteFulltextToModel: boolean
  quietOrganize: boolean
  bufferSeconds: number
}

export type AppSettingsPatch = Partial<
  Pick<
    AppSettings,
    | 'autoOrganize'
    | 'autoSyncMemory'
    | 'theme'
    | 'docWidth'
    | 'allowNoteFulltextToModel'
    | 'quietOrganize'
    | 'bufferSeconds'
  >
>

export const api = {
  getAppSettings: () => invoke<AppSettings>('get_app_settings'),
  saveAppSettings: (changes: AppSettingsPatch) =>
    invoke<AppSettings>('save_app_settings', changes),
  openWorkspace: (root: string, captureDir: string) =>
    invoke<OpenedWorkspace>('open_workspace', { root, captureDir }),
  initStandardVault: (root: string) =>
    invoke<OpenedWorkspace>('init_standard_vault', { root }),
  resumeWorkspace: () =>
    invoke<OpenedWorkspace | null>('resume_workspace'),
  loadCaptureWorkspace: () =>
    invoke<OpenedWorkspace>('load_capture_workspace'),
  showCaptureWindow: (session: string) =>
    invoke<void>('show_capture_window', { session }),
  closeCaptureWindow: (exit?: boolean) =>
    invoke<void>('close_capture_window', { exit }),
  persistDraft: (session: string, revision: number, body: string) =>
    invoke<number>('persist_draft', { session, revision, body }),
  commitCapture: (session: string, requestId: string, revision: number, body: string) =>
    invoke<string>('commit_capture', { session, requestId, revision, body }),
  commitCaptureV2: (
    session: string,
    requestId: string,
    revision: number,
    body: string,
    mode: CaptureCommitMode,
  ) => invoke<CaptureCommitResult>('commit_capture_v2', { session, requestId, revision, body, mode }),
  confirmCapture: (session: string, requestId: string) =>
    invoke<boolean>('confirm_capture', { session, requestId }),
  listRecentCaptures: (session: string) =>
    invoke<RecentCapture[]>('list_recent_captures', { session }),
  searchRecords: (params: { session: string; query: string; fromUtc?: string; untilUtc?: string; offset: number }) =>
    invoke<SearchPage>('search_records', params),
  globalSearch: (session: string, query: string, limit?: number) =>
    invoke<GlobalSearchHit[]>('global_search', { session, query, limit }),
  rebuildSearchIndex: (session: string) =>
    invoke<IndexRebuildResult>('rebuild_search_index', { session }),
  openRecordSource: (session: string, relativePath: string, contentHash: string) =>
    invoke<RecordSource>('open_record_source', { session, relativePath, contentHash }),

  // Unified Library & Document Viewer (Wikra v2.0)
  listLibraryTree: (session: string) =>
    invoke<LibraryTree>('list_library_tree', { session }),
  readDocumentDetail: (session: string, relativePath: string) =>
    invoke<DocumentDetail>('read_document_detail', { session, relativePath }),
  saveMarkdownDocument: (session: string, relativePath: string, expectedHash: string, body: string) =>
    invoke<DocumentDetail>('save_markdown_document', { session, relativePath, expectedHash, body }),
  createMarkdownDocument: (session: string, relativePath: string, initialBody: string) =>
    invoke<DocumentDetail>('create_markdown_document', { session, relativePath, initialBody }),
  appendHumanUnderstanding: (session: string, relativePath: string, insightText: string) =>
    invoke<DocumentDetail>('append_human_understanding', { session, relativePath, insightText }),

  // Inbox Reader & Compilation (Wikra v2.0)
  listInboxItems: (session: string) =>
    invoke<InboxItemSummary[]>('list_inbox_items', { session }),
  getInboxItem: (session: string, id: string) =>
    invoke<InboxItemDetail>('get_inbox_item', { session, id }),
  ingestInboxInput: (session: string, input: string, titleHint?: string) =>
    invoke<InboxItemDetail>('ingest_inbox_input', { session, input, titleHint }),
  completeInboxStub: (session: string, id: string, pastedBody: string) =>
    invoke<InboxItemDetail>('complete_inbox_stub', { session, id, pastedBody }),
  toggleInboxParagraph: (session: string, id: string, paragraphId: string) =>
    invoke<InboxItemDetail>('toggle_inbox_paragraph', { session, id, paragraphId }),
  compileInboxItem: (session: string, id: string, mode: 'excerpt' | 'full' | 'archive') =>
    invoke<InboxItemDetail>('compile_inbox_item', { session, id, mode }),
  openExternalUrl: (url: string) =>
    invoke<void>('open_external_url', { url }),

  // Organize Overview, 60s Buffer Queue, Reversible Changesets & Active Recall
  listOrganizeOverview: (session: string) =>
    invoke<OrganizeOverview>('list_organize_overview', { session }),
  cancelBufferItem: (session: string, itemId: string) =>
    invoke<OrganizeOverview>('cancel_buffer_item', { session, itemId }),
  flushBufferNow: (session: string) =>
    invoke<OrganizeOverview>('flush_buffer_now', { session }),
  getChangesetDetail: (session: string, changesetId: string) =>
    invoke<ChangesetDetail>('get_changeset_detail', { session, changesetId }),
  decideReview: (
    session: string,
    changesetId: string,
    path: string,
    decision: 'apply' | 'discard',
  ) => invoke<ChangesetItem>('decide_review', { session, changesetId, path, decision }),
  revertChangeset: (session: string, changesetId: string) =>
    invoke<RevertChangesetResult>('revert_changeset', { session, changesetId }),
  dismissSuggestion: (session: string, suggestionId: string) =>
    invoke<void>('dismiss_suggestion', { session, suggestionId }),
  undismissSuggestion: (session: string, suggestionId: string) =>
    invoke<void>('undismiss_suggestion', { session, suggestionId }),

  // P0 Hybrid Lint (L1 Structure + L2 Semantic)
  runStructureLint: (session: string) =>
    invoke<LintReport>('run_structure_lint', { session }),
  applyLintFix: (session: string, findingIds: string[]) =>
    invoke<LintFixResult>('apply_lint_fix', { session, findingIds }),
  previewSemanticLint: (session: string, scope: SemanticLintScope) =>
    invoke<SemanticLintPreview>('preview_semantic_lint', { session, scope }),
  runSemanticLint: (session: string, scope: SemanticLintScope) =>
    invoke<LintFinding[]>('run_semantic_lint', { session, scope }),
  escalateLintToConflict: (session: string, findingId: string) =>
    invoke<ConflictItem>('escalate_lint_to_conflict', { session, findingId }),

  // P0 Wikilinks & Citation Context
  listWikilinkCandidates: (session: string) =>
    invoke<WikilinkCandidate[]>('list_wikilink_candidates', { session }),
  resolveWikilink: (session: string, fromPath: string, target: string) =>
    invoke<WikilinkResolution>('resolve_wikilink', { session, fromPath, target }),
  readCitationContext: (session: string, pagePath: string, refLabel: string) =>
    invoke<CitationContext>('read_citation_context', { session, pagePath, refLabel }),

  // P0 External Agent Integration (Read-Only MCP & SKILL.md)
  getAgentIntegration: (session: string) =>
    invoke<AgentIntegration>('get_agent_integration', { session }),
  setMcpRawAllow: (session: string, folders: string[]) =>
    invoke<string[]>('set_mcp_raw_allow', { session, folders }),
  exportVaultSkill: (session: string) =>
    invoke<ExportSkillResult>('export_vault_skill', { session }),
  testMcpStdio: (session: string) =>
    invoke<McpTestResult>('test_mcp_stdio', { session }),

  teachbackTurn: (session: string, pagePath: string, step: number, userAnswer?: string) =>
    invoke<TeachbackTurn>('teachback_turn', { session, pagePath, step, userAnswer }),
  saveAnswerToWiki: (session: string, conversationId: string, messageId: string, targetPath?: string) =>
    invoke<DocumentDetail>('save_answer_to_wiki', { session, conversationId, messageId, targetPath }),

  // Interactive Knowledge Graph
  getKnowledgeGraph: (session: string, includeMetaPages: boolean) =>
    invoke<KnowledgeGraph>('get_knowledge_graph', { session, includeMetaPages }),
  dismissGraphGap: (session: string, gapId: string) =>
    invoke<void>('dismiss_graph_gap', { session, gapId }),

  pickWorkspaceFolder: (kind: 'root' | 'capture' | 'raw', root: string, captureDir: string) =>
    invoke<string | null>('pick_workspace_folder', { kind, root, captureDir }),
  updateWorkspaceDirs: (session: string, rawDir: string, captureDir?: string) =>
    invoke<OpenedWorkspace>('update_workspace_dirs', { session, rawDir, captureDir }),
  pickWorkspaceMarkdown: (session: string, kind: DocumentKind) =>
    invoke<string | null>('pick_workspace_markdown', { session, kind }),
  listWorkspaceMarkdownFiles: (session: string) =>
    invoke<string[]>('list_workspace_markdown_files', { session }),
  openWorkspaceFile: (session: string, relativePath: string, mode: 'open' | 'reveal') =>
    invoke<void>('open_workspace_file', { session, relativePath, mode }),
  readWorkspaceImage: (session: string, basePath: string | undefined, imagePath: string) =>
    invoke<string>('read_workspace_image', { session, basePath, imagePath }),
  desktopWindowAction: (action: 'drag' | 'minimize' | 'maximize' | 'close') =>
    invoke<void>('desktop_window_action', { action }),
  isMainMaximized: () =>
    invoke<boolean>('is_main_maximized'),

  getHotkey: () => invoke<HotkeyStatus>('get_hotkey'),
  setHotkey: (id: string) => invoke<HotkeyStatus>('set_hotkey', { id }),

  setCredential: (session: string, slot: CredentialSlot, linked: boolean) =>
    invoke<CredentialReference>('set_credential', { session, slot, linked }),
  getModelConfig: (session: string) =>
    invoke<ModelConfig | null>('get_model_config', { session }),
  getIntegrationsConfig: (session: string) =>
    invoke<IntegrationsConfig>('get_integrations_config', { session }),
  saveModelConfig: (session: string, endpoint: string, model: string, enabled: boolean, secret?: string) =>
    invoke<void>('save_model_config', { session, endpoint, model, enabled, secret }),
  saveMemoryConfig: (session: string, config: MemoryConfig, secret?: string) =>
    invoke<void>('save_memory_config', { session, ...config, secret }),
  testConnection: (session: string, slot: CredentialSlot) =>
    invoke<string>(slot === 'model' ? 'test_model_connection' : 'test_memory_connection', { session }),
  listRemoteModels: (session: string, endpoint: string, secret?: string) =>
    invoke<string[]>('list_remote_models', { session, endpoint, secret }),
  retryMemorySync: (session: string, noteId?: string) =>
    invoke<number>('retry_memory_sync', { session, noteId }),
  readMemoryItem: (session: string, uri: string) =>
    invoke<MemoryItem>('read_memory_item', { session, uri }),

  listNotes: (session: string) =>
    invoke<NoteSummary[]>('list_notes', { session }),
  getNoteDetail: (session: string, idOrPath: string) =>
    invoke<NoteDetail>('get_note_detail', { session, idOrPath }),
  updateNote: (params: {
    session: string
    idOrPath: string
    expectedHash: string
    body: string
    reorganize?: boolean
  }) => invoke<NoteDetail>('update_note', params),
  triggerNoteOrganize: (session: string, idOrPath: string) =>
    invoke<NoteDetail>('trigger_note_organize', { session, idOrPath }),

  drawerCounts: (session: string) =>
    invoke<DrawerCounts>('drawer_counts', { session }),
  listConflicts: (session: string) =>
    invoke<ConflictItem[]>('list_conflicts', { session }),
  createConflict: (params: {
    session: string
    topic: string
    currentLabel?: string
    currentPath?: string
    currentText: string
    incomingLabel?: string
    incomingPath?: string
    incomingText: string
  }) => invoke<ConflictItem>('create_conflict', params),
  resolveConflict: (params: {
    session: string
    id: string
    status: ConflictStatus
    resolutionNote?: string
    wikiTarget?: string
  }) => invoke<ConflictItem>('resolve_conflict', params),

  listWikiDocuments: (session: string) =>
    invoke<WikiDocument[]>('list_wiki_documents', { session }),
  wikiTargets: (session: string) =>
    invoke<DocumentMapping[]>('wiki_targets', { session }),
  mapWikiTarget: (session: string, kind: DocumentKind, path: string) =>
    invoke<void>('map_wiki_target', { session, kind, path }),
  unmapWikiTarget: (session: string, path: string) =>
    invoke<void>('unmap_wiki_target', { session, path }),
  createWikiTarget: (session: string, path: string) =>
    invoke<void>('create_wiki_target', { session, path }),
  listWikiSuggestions: (session: string) =>
    invoke<WikiSuggestion[]>('list_wiki_suggestions', { session }),
  createWikiSuggestion: (params: {
    session: string
    noteId?: string
    targetPath: string
    content: string
    reason: string
  }) => invoke<WikiSuggestion>('create_wiki_suggestion', params),
  applyWikiSuggestion: (session: string, id: string) =>
    invoke<WikiSuggestion>('apply_wiki_suggestion', { session, id }),
  recompareWikiSuggestion: (session: string, id: string) =>
    invoke<WikiSuggestion>('recompare_wiki_suggestion', { session, id }),
  rejectWikiSuggestion: (session: string, id: string) =>
    invoke<void>('reject_wiki_suggestion', { session, id }),
  undoWikiSuggestion: (session: string, id: string) =>
    invoke<WikiSuggestion>('undo_wiki_suggestion', { session, id }),

  listTasks: (session: string) =>
    invoke<TaskList>('list_tasks', { session }),
  toggleTask: (session: string, path: string, taskId: string, expectedLine: string, done: boolean) =>
    invoke<void>('toggle_task', { session, path, taskId, expectedLine, done }),

  listConversations: (session: string) =>
    invoke<ConversationSummary[]>('list_conversations', { session }),
  getConversation: (session: string, id: string) =>
    invoke<ConversationDetail>('get_conversation', { session, id }),
  deleteConversation: (session: string, id: string) =>
    invoke<void>('delete_conversation', { session, id }),
  sendChatMessage: (session: string, conversationId: string | undefined, message: string) =>
    invoke<ChatTurn>('send_chat_message', { session, conversationId, message }),
}
