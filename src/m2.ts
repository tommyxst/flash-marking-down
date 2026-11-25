export type WorkspacePage = 'records' | 'organize' | 'wiki' | 'tasks' | 'answers'

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

export type IntegrationsConfig = {
  model: ModelConfig | null
  memory: { baseUrl: string; namespace: string; enabled: boolean } | null
  credentials: CredentialReference[]
}

export type RecordEntry = {
  path: string
  capturedAt: string | null
  text: string
  contentHash: string
  titleText: string
  isCapture: boolean
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

export type RecordSelection = {
  path: string
  title: string
  contentHash: string
  isCapture?: boolean
  excerpt?: string
}

export type Evidence = {
  id: string
  sourceId?: string
  path: string
  revision: string
  startLine: number
  endLine: number
  excerpt: string
  date: string | null
  eventDate?: string | null
  movedFrom?: string
  available: boolean
  changed: boolean
  outsideRange: boolean
  isCapture?: boolean
}

export type AnalysisItem = {
  id: string
  kind: 'discussion' | 'decision' | 'action' | 'open' | 'wiki'
  text: string
  source: Evidence
  assignee: string | null
  due: string | null
}

export type Analysis = {
  id: string
  path: string
  revision: string
  status: 'running' | 'succeeded' | 'failed' | 'stale'
  error: string | null
  items: AnalysisItem[]
}

export const relationLabels = { contradiction: '意见矛盾', conditions: '适用条件不同', historical: '历史变化' }
export const disagreementStatuses = { open: '待判断', decided: '已记录当前判断', resolved: '已解决', superseded: '已被替代' }
export type Relation = keyof typeof relationLabels
export type DisagreementStatus = keyof typeof disagreementStatuses

export type PossibleDisagreement = {
  question: string
  relation: Relation
  members: { text: string; source: Evidence; claimId?: string }[]
}

export type Disagreement = PossibleDisagreement & {
  id: string
  status: DisagreementStatus
  judgment: string | null
  reason: string | null
  judgmentNeedsSources?: boolean
  sourcesChanged?: boolean
}

export const relationBadgeTone: Record<Relation, string> = {
  contradiction: 'badge-error',
  conditions: 'badge-warn',
  historical: 'badge-accent',
}

export const statusBadgeTone: Record<DisagreementStatus, string> = {
  open: 'badge-warn',
  decided: 'badge-accent',
  resolved: 'badge-success',
  superseded: 'badge-muted',
}

export type OutputKind = 'answer' | 'summary' | 'plan'

export type AnswerPreparation = {
  id: string
  endpoint: string
  query: string
  outputKind: OutputKind
  sources: Evidence[]
  disagreements: Disagreement[]
}

export type Answer = Omit<AnswerPreparation, 'endpoint'> & {
  text: string
  sourceNotice?: string
  dateBasis: 'recorded' | 'event'
  fromDate: string | null
  toDate: string | null
  skipped: number
  possibleDisagreements?: PossibleDisagreement[]
}

export type DocumentKind = 'wiki' | 'tasks'
export type DocumentMapping = {
  kind: DocumentKind
  path: string
}

export type ChangePhase = 'pending' | 'deferred' | 'rejected' | 'prepared' | 'applied' | 'conflict' | 'reverted' | 'superseded'
export type ChangeKind = 'wiki' | 'task' | 'status' | 'undo_wiki' | 'undo_task' | 'undo_status'

export const phaseBadgeTone: Record<ChangePhase, string> = {
  pending: 'badge-warn',
  deferred: 'badge-muted',
  rejected: 'badge-muted',
  prepared: 'badge-warn',
  applied: 'badge-success',
  conflict: 'badge-error',
  reverted: 'badge-accent',
  superseded: 'badge-muted',
}

export type DocumentChange = {
  id: string
  target: string
  kind: ChangeKind
  phase: ChangePhase
  baseHash: string
  before: string
  after: string
  sourcePath: string
  sourceRevision: string
  startLine: number
  endLine: number
  sourceExcerpt: string
  sourceIsCapture: boolean
}

export type WikiApplyOutcome = {
  id: string
  outcome: string
  message: string
}

export type TaskState = 'open' | 'done' | 'cancelled' | 'invalid'
export type TaskEntry = {
  id: string
  expectedLine: string
  text: string
  state: TaskState
  sourcePath: string
  sourceIsCapture: boolean
  manual: boolean
}

export type TaskList = {
  path: string
  revision: string
  entries: TaskEntry[]
  repairMessages: string[]
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
