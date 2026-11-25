import { invoke } from '@tauri-apps/api/core'
import type {
  Analysis,
  Answer,
  AnswerPreparation,
  CredentialReference,
  CredentialSlot,
  Disagreement,
  DisagreementStatus,
  DocumentChange,
  DocumentKind,
  DocumentMapping,
  HotkeyStatus,
  IndexRebuildResult,
  IntegrationsConfig,
  ModelConfig,
  OpenedWorkspace,
  OutputKind,
  RecentCapture,
  RecordSource,
  Relation,
  SearchPage,
  TaskList,
  WikiApplyOutcome,
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
  saveMemoryConfig: (session: string, baseUrl: string, namespace: string, enabled: boolean, secret?: string) =>
    invoke<void>('save_memory_config', { session, baseUrl, namespace, enabled, secret }),
  testConnection: (session: string, slot: CredentialSlot) =>
    invoke<string>(slot === 'model' ? 'test_model_connection' : 'test_memory_connection', { session }),

  listAnalyses: (session: string, relativePath: string) =>
    invoke<Analysis[]>('list_analyses', { session, relativePath }),
  analyzeSource: (session: string, relativePath: string, contentHash: string, approvedEndpoint: string) =>
    invoke<Analysis>('analyze_source', { session, relativePath, contentHash, approvedEndpoint }),
  prepareAnalysisChange: (session: string, itemId: string, target: string) =>
    invoke<DocumentChange>('prepare_analysis_change', { session, itemId, target }),

  wikiTargets: (session: string) =>
    invoke<DocumentMapping[]>('wiki_targets', { session }),
  mapWikiTarget: (session: string, kind: DocumentKind, path: string) =>
    invoke<void>('map_wiki_target', { session, kind, path }),
  createWikiTarget: (session: string, path: string) =>
    invoke<void>('create_wiki_target', { session, path }),
  wikiChanges: (session: string) =>
    invoke<DocumentChange[]>('wiki_changes', { session }),
  reviewWikiChange: (session: string, id: string, decision: 'rejected' | 'deferred') =>
    invoke<void>('review_wiki_change', { session, id, decision }),
  applyWikiChanges: (session: string, ids: string[]) =>
    invoke<WikiApplyOutcome[]>('apply_wiki_changes', { session, ids }),
  previewWikiUndo: (session: string, id: string) =>
    invoke<DocumentChange>('preview_wiki_undo', { session, id }),
  rebaseWikiChange: (session: string, id: string) =>
    invoke<DocumentChange>('rebase_wiki_change', { session, id }),
  listTasks: (session: string) =>
    invoke<TaskList>('list_tasks', { session }),
  toggleTask: (session: string, path: string, taskId: string, expectedLine: string, done: boolean) =>
    invoke<void>('toggle_task', { session, path, taskId, expectedLine, done }),

  prepareAnswer: (params: {
    session: string
    query: string
    keyword: string
    dateBasis: 'recorded' | 'event'
    fromDate?: string
    toDate?: string
    outputKind: OutputKind
    anchorClaimId?: string
  }) => invoke<AnswerPreparation>('prepare_answer', params),
  answerQuery: (session: string, preparationId: string, approvedEndpoint: string) =>
    invoke<Answer>('answer_query', { session, preparationId, approvedEndpoint }),
  cancelAnswerPreparation: (session: string, preparationId: string) =>
    invoke<void>('cancel_answer_preparation', { session, preparationId }),
  copyAnswer: (session: string, answerId: string) =>
    invoke<string>('copy_answer', { session, answerId }),
  listDisagreements: (session: string) =>
    invoke<Disagreement[]>('list_disagreements', { session }),
  createDisagreement: (
    session: string,
    question: string,
    relation: Relation,
    members: { path: string; revision: string; excerpt: string }[],
  ) => invoke<Disagreement>('create_disagreement', { session, question, relation, members }),
  reviewDisagreement: (
    session: string,
    id: string,
    status: DisagreementStatus,
    judgment: string,
    reason: string,
  ) => invoke<Disagreement>('review_disagreement', { session, id, status, judgment, reason }),
}
