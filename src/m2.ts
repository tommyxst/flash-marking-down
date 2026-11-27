export type WorkspacePage = 'records' | 'organize' | 'wiki' | 'tasks' | 'answers'
export type LeftTab = 'notes' | 'wiki' | 'tasks'
export type DrawerTab = 'conflicts' | 'wiki'

export type WikiDocument = {
  path: string
  title: string
  preview: string
  folder: string
  mapped: boolean
  pendingSuggestions: number
  conflictSuggestions: number
  updatedAt: string | null
}

export type DraftDto = {
  revision: number
  body: string | null
}

export type RecoveryErrorDto = {
  requestId: string
  message: string
}

export type OpenedWorkspace = {
  session: string
  root: string
  captureDir: string
  rawDir: string
  draft: DraftDto
  recovered: string[]
  recoveryErrors: RecoveryErrorDto[]
  pendingRequestId: string | null
}

export type RecentCapture = {
  path: string
  blockId: string | null
  capturedAt: string
  body: string
  contentHash: string
  isCapture: boolean
}

export type SearchHit = {
  path: string
  capturedAt: string | null
  contentHash: string
  preview: string
  titleText: string
  isCapture: boolean
}

export type SearchPage = {
  hits: SearchHit[]
  hasMore: boolean
  skipped: number
}

export type IndexRebuildResult = {
  indexed: number
  skipped: number
}

export type RecordSource = {
  state: 'current' | 'changed'
  contentHash: string
  body: string
  isCapture: boolean
}

export type HotkeyChoice = {
  id: string
  label: string
}

export type HotkeyStatus = {
  id: string
  active: boolean
  error: string | null
  choices: HotkeyChoice[]
}

export type CredentialSlot = 'model' | 'memory'
export type CredentialState = 'unlinked' | 'linked' | 'missing' | 'unavailable' | 'invalid'
export type CredentialReference = {
  slot: CredentialSlot
  target: string
  state: CredentialState
}

export type ModelConfig = {
  endpoint: string
  model: string
  enabled: boolean
  ready: boolean
  requiresCredential: boolean
}

export type MemoryConfig = {
  baseUrl: string
  namespace: string
  enabled: boolean
  syncResources: boolean
  commitMemories: boolean
}

export type IntegrationsConfig = {
  model: ModelConfig | null
  memory: MemoryConfig | null
  credentials: CredentialReference[]
}

export type MemorySyncStatus = 'pending' | 'synced' | 'failed'

export type RecordEntry = {
  id?: string
  path: string
  locator?: string
  capturedAt: string | null
  text: string
  contentHash: string
  titleText: string
  isCapture: boolean
  organizeStatus?: string
  openConflictCount?: number
}

export function recordTitle(record: Pick<RecordEntry, 'titleText'>): string {
  return record.titleText.trimStart().split('\n', 1)[0]?.replace(/^#{1,6}\s+/, '').trim() || '未命名记录'
}

export type SourceView =
  | { state: 'opening' }
  | { state: 'opened'; changed: boolean; contentHash: string; body: string; isCapture: boolean }
  | { state: 'failed'; error: import('./api/errors').UiError }

export type IndexFeedback =
  | { state: 'building' }
  | { state: 'ready'; indexed: number; skipped: number }
  | { state: 'failed'; error: import('./api/errors').UiError }

export type OrganizedTask = {
  id: string
  text: string
  assignee: string | null
  due: string | null
  done: boolean
  line: number | null
}

export type OrganizeResult = {
  summary: string
  decisions: string[]
  tasks: OrganizedTask[]
  sourceChanged: boolean
  lastError: string | null
}

export type NoteSummary = {
  id: string
  path: string
  locator: string
  blockId: string | null
  capturedAt: string
  title: string
  bodyPreview: string
  contentHash: string
  isCapture: boolean
  organizeStatus: 'idle' | 'queued' | 'running' | 'done' | 'failed' | string
  ovResourceStatus: MemorySyncStatus
  ovMemoryStatus: MemorySyncStatus
  openConflictCount: number
}

export type ConflictStatus = 'open' | 'keep_current' | 'accept_incoming' | 'keep_both'

export type ConflictItem = {
  id: string
  topic: string
  currentLabel: string
  currentPath: string | null
  currentText: string
  incomingLabel: string
  incomingPath: string | null
  incomingText: string
  status: ConflictStatus
  resolutionNote: string | null
  createdAt: string
  resolvedAt: string | null
}

export type WikiSuggestionStatus = 'pending' | 'applied' | 'rejected' | 'conflict' | 'reverted'

export type WikiSuggestion = {
  id: string
  noteId: string | null
  targetPath: string
  baseHash: string
  beforeText: string
  afterText: string
  reason: string
  status: WikiSuggestionStatus
  createdAt: string
}

export type NoteDetail = {
  id: string
  path: string
  locator: string
  blockId: string | null
  capturedAt: string | null
  title: string
  body: string
  contentHash: string
  isCapture: boolean
  organizeStatus: string
  organize: OrganizeResult | null
  organizeError: string | null
  ovResourceStatus: MemorySyncStatus
  ovResourceUri: string | null
  ovMemoryStatus: MemorySyncStatus
  ovError: string | null
  conflicts: ConflictItem[]
  wikiSuggestions: WikiSuggestion[]
}

export type DrawerCounts = {
  openConflicts: number
  pendingWiki: number
}

export type DocumentKind = 'wiki' | 'tasks'
export type DocumentMapping = {
  kind: DocumentKind
  path: string
}

export type TaskState = 'open' | 'done' | 'cancelled' | 'invalid'
export type TaskEntry = {
  id: string
  path: string
  expectedLine: string
  text: string
  state: TaskState
  sourcePath: string
  sourceIsCapture: boolean
  manual: boolean
  assignee: string | null
  due: string | null
  noteId: string | null
}

export type TaskList = {
  path: string
  revision: string
  entries: TaskEntry[]
  repairMessages: string[]
}

export type CitationKind = 'local' | 'ov_memory' | 'ov_resource'

export const citationKindLabels: Record<CitationKind, string> = {
  local: '[本地]',
  ov_memory: '[OV记忆]',
  ov_resource: '[OV资源]',
}

export type CitationItem = {
  index: number
  kind: CitationKind
  path: string
  title: string
  snippet: string
  capturedAt: string | null
}

export type MemoryItem = {
  uri: string
  kind: Exclude<CitationKind, 'local'>
  title: string
  content: string
}

export type MemorySource = Omit<MemoryItem, 'content'>

export type MemoryPreview = MemorySource & (
  | { state: 'opening' }
  | { state: 'opened'; content: string }
  | { state: 'failed'; error: import('./api/errors').UiError }
)

export type ConversationSummary = {
  id: string
  title: string
  createdAt: string
  updatedAt: string
}

export type ChatMessage = {
  id: string
  conversationId: string
  role: 'user' | 'assistant' | string
  content: string
  citations: CitationItem[]
  relatedConflicts: ConflictItem[]
  createdAt: string
  notice?: string | null
}

export type ConversationDetail = {
  conversation: ConversationSummary
  messages: ChatMessage[]
}

export type ChatTurn = {
  conversation: ConversationSummary
  userMessage: ChatMessage
  assistantMessage: ChatMessage
}

export const conflictStatusLabels: Record<ConflictStatus, string> = {
  open: '待解决',
  keep_current: '保留当前',
  accept_incoming: '采纳新记录',
  keep_both: '保留双方',
}

export const conflictStatusTone: Record<ConflictStatus, string> = {
  open: 'badge-warn',
  keep_current: 'badge-accent',
  accept_incoming: 'badge-success',
  keep_both: 'badge-accent',
}

export const wikiStatusLabels: Record<WikiSuggestionStatus, string> = {
  pending: '待审阅',
  applied: '已采纳',
  rejected: '已拒绝',
  conflict: '版本冲突',
  reverted: '已撤销',
}

export const wikiStatusTone: Record<WikiSuggestionStatus, string> = {
  pending: 'badge-warn',
  applied: 'badge-success',
  rejected: 'badge-muted',
  conflict: 'badge-error',
  reverted: 'badge-accent',
}

const GENERATED_CAPTURE_FILE = /^(?:[0-9a-f]{32}\.md|[^\\/]+\.md#fmd-note:[0-9a-f]{8,16})$/i

export function isGeneratedCaptureFile(path: string): boolean {
  const base = path.split(/[\\/]/).pop() ?? ''
  return GENERATED_CAPTURE_FILE.test(base)
}

export function presentSourceMarkdown(text: string): string {
  return text.replace(/[ \t]+<!-- fmd-note:[0-9a-f]{8,16} -->$/gim, '').replace(
    /(^| · )来源：([^\r\n]+?)（第 (\d+–\d+ 行)） <!-- fmd-source:[0-9a-f]{32} fmd-revision:[0-9a-f]{64} -->/gm,
    (citation, prefix: string, path: string, lines: string) =>
      isGeneratedCaptureFile(path) ? `${prefix}来源：随手记（第 ${lines}）` : citation,
  )
}

export function useSourcePresentation() {
  return {
    showSourcePath: (path: string | undefined, isCapture?: boolean) =>
      Boolean(path && !(isCapture ?? isGeneratedCaptureFile(path))),
    sourceMarkdown: (text: string) => presentSourceMarkdown(text),
  }
}

// ============================================================================
// Wikra v2.0 Frozen DTO Contracts (§2–§3)
// ============================================================================

export type ThemeName = 'slate' | 'paper'
export type DocWidthMode = 'readable' | 'full'
export type CaptureCommitMode = 'only' | 'queue' | 'now'

export interface LibraryDocSummary {
  path: string
  title: string
  folder: string
  docType: 'decision' | 'concept' | 'synthesis' | 'entity' | 'note' | string
  typeLabel: string
  tags: string[]
  updatedAt: string | null
  citationCount: number
  pendingConflictCount: number
  wikraCore: boolean
}

export interface RawFileSummary {
  id: string
  path: string
  title: string
  kind: 'note' | 'clip'
  kindLabel: '随手笔记' | '收藏原文' | string
  createdAt: string | null
  sourceUrl: string | null
  linkedPagePaths: string[]
}

export interface LibraryTree {
  rawDir: string
  wikiPages: LibraryDocSummary[]
  rawFiles: RawFileSummary[]
}

export interface DocCitation {
  id: string
  refLabel: string
  rawPath: string
  label: string
  quote: string
}

export interface DocBacklink {
  pagePath: string
  title: string
  excerpt: string
}

export interface DocTaskItem {
  id: string
  lineNumber: number
  expectedLine: string
  text: string
  done: boolean
}

export interface DocumentDetail {
  kind: 'wiki' | 'raw'
  path: string
  title: string
  body: string
  contentHash: string
  updatedAt: string | null
  docType: string
  typeLabel: string
  aliases: string[]
  tags: string[]
  wikraCore: boolean
  sourceUrl: string | null
  pendingConflictIds: string[]
  outline: string[]
  tasks: DocTaskItem[]
  citations: DocCitation[]
  backlinks: DocBacklink[]
  linkedWikiPaths: string[]
  hasProtectedSection: boolean
}

export interface InboxParagraph {
  id: string
  heading: string
  text: string
  highlighted: boolean
  compiled: boolean
}

export interface InboxItemSummary {
  id: string
  title: string
  domain: string
  sourceType: string
  url: string | null
  rawPath: string
  savedAt: string
  readTime: string
  fetchStatus: 'full' | 'stub'
  state: 'unread' | 'reading' | 'compiled' | 'archived'
  wordCount: number
  highlightCount: number
}

export interface InboxItemDetail extends InboxItemSummary {
  summaryPoints: string[]
  relatedPagePaths: string[]
  paragraphs: InboxParagraph[]
}

export interface ChangesetOp {
  label: string
  targetPath: string
  targetTitle: string
  outcome: 'applied' | 'pending_review' | 'reverted' | string
}

export interface ChangesetItem {
  id: string
  time: string
  trigger: string
  modelAlias: string
  status: 'applied' | 'partial' | 'reverted'
  summary: string
  pagePaths: string[]
  ops: ChangesetOp[]
}

export interface BufferQueueItem {
  id: string
  noteId: string
  path: string
  title: string
  preview: string
  queuedAt: string
}

export interface OrganizeSuggestion {
  id: string
  kind: 'review' | 'orphan' | string
  kindLabel: string
  title: string
  detail: string
  actionLabel: string
  pagePath: string
}

export interface OrganizeOverview {
  conflicts: ConflictItem[]
  changesets: ChangesetItem[]
  suggestions: OrganizeSuggestion[]
  bufferQueue: BufferQueueItem[]
  bufferSecondsLeft: number
}

export interface CaptureCommitResult {
  path: string
  noteId: string
  queued: boolean
  bufferSecondsLeft: number
}

export interface TeachbackTurn {
  step: number
  totalSteps: number
  prompt: string
  feedback: string
  suggestedInsight: string | null
}

export interface GraphNode {
  id: string
  title: string
  docType: string
  degree: number
  isHub: boolean
  isOrphan: boolean
}

export interface GraphEdge {
  id: string
  from: string
  to: string
  kind: 'link' | 'citation'
  label: string
  snippet: string
}

export interface GraphGap {
  id: string
  title: string
  reason: string
  question: string
  fromPath: string
  toPath: string
}

export interface KnowledgeGraph {
  nodes: GraphNode[]
  edges: GraphEdge[]
  gaps: GraphGap[]
}

export interface GlobalSearchHit {
  kind: 'wiki' | 'raw' | 'citation'
  kindLabel: '知识页' | '随手笔记' | '收藏原文' | '原文引文' | string
  title: string
  subtitle: string
  targetPath: string
  citationId: string | null
}

