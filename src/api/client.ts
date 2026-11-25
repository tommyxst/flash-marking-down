import { invoke } from '@tauri-apps/api/core'
import type {
  ChatTurn,
  ConflictItem,
  ConflictStatus,
  ConversationDetail,
  ConversationSummary,
  CredentialReference,
  CredentialSlot,
  DocumentKind,
  DocumentMapping,
  DrawerCounts,
  HotkeyStatus,
  IndexRebuildResult,
  IntegrationsConfig,
  MemoryConfig,
  MemoryItem,
  ModelConfig,
  NoteDetail,
  NoteSummary,
  OpenedWorkspace,
  RecentCapture,
  RecordSource,
  SearchPage,
  TaskList,
  WikiSuggestion,
} from '../m2'

export interface AppSettings {
  version: number
  lastWorkspaceRoot: string | null
  lastCaptureDir: string
  hotkey: string
  autoOrganize: boolean
  autoSyncMemory: boolean
}

export const api = {
  getAppSettings: () => invoke<AppSettings>('get_app_settings'),
  saveAppSettings: (changes: Partial<Pick<AppSettings, 'autoOrganize' | 'autoSyncMemory'>>) =>
    invoke<AppSettings>('save_app_settings', changes),
  openWorkspace: (root: string, captureDir: string) =>
    invoke<OpenedWorkspace>('open_workspace', { root, captureDir }),
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
  confirmCapture: (session: string, requestId: string) =>
    invoke<boolean>('confirm_capture', { session, requestId }),
  listRecentCaptures: (session: string) =>
    invoke<RecentCapture[]>('list_recent_captures', { session }),
  searchRecords: (params: { session: string; query: string; fromUtc?: string; untilUtc?: string; offset: number }) =>
    invoke<SearchPage>('search_records', params),
  rebuildSearchIndex: (session: string) =>
    invoke<IndexRebuildResult>('rebuild_search_index', { session }),
  openRecordSource: (session: string, relativePath: string, contentHash: string) =>
    invoke<RecordSource>('open_record_source', { session, relativePath, contentHash }),

  pickWorkspaceFolder: (kind: 'root' | 'capture', root: string, captureDir: string) =>
    invoke<string | null>('pick_workspace_folder', { kind, root, captureDir }),
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
