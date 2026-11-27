<script setup lang="ts">
import { isTauri } from '@tauri-apps/api/core'
import { getCurrentWindow } from '@tauri-apps/api/window'
import { computed, nextTick, onMounted, onUnmounted, ref, watch } from 'vue'
import { api, type AppSettings } from '../api/client'
import { parseAppError, type UiError } from '../api/errors'
import UiIcon from '../UiIcon.vue'
import BufferQueueModal from '../components/wikra/BufferQueueModal.vue'
import ConflictReviewView from '../components/wikra/ConflictReviewView.vue'
import InboxReaderView from '../components/wikra/InboxReaderView.vue'
import InboxSidebar from '../components/wikra/InboxSidebar.vue'
import KnowledgeGraphView from '../components/wikra/KnowledgeGraphView.vue'
import LibrarySidebar from '../components/wikra/LibrarySidebar.vue'
import OrganizeView from '../components/wikra/OrganizeView.vue'
import RawDocumentView from '../components/wikra/RawDocumentView.vue'
import RightCompanionPane from '../components/wikra/RightCompanionPane.vue'
import SettingsModal from '../components/wikra/SettingsModal.vue'
import VaultOnboardingModal from '../components/wikra/VaultOnboardingModal.vue'
import WikiDocumentView from '../components/wikra/WikiDocumentView.vue'
import type {
  ChangesetItem,
  ConflictItem,
  ConflictStatus,
  DocCitation,
  DocTaskItem,
  DocumentDetail,
  DocWidthMode,
  GlobalSearchHit,
  GraphGap,
  HotkeyStatus,
  InboxItemDetail,
  InboxItemSummary,
  IntegrationsConfig,
  KnowledgeGraph,
  LibraryTree,
  LintReport,
  MemoryConfig,
  ModelConfig,
  OpenedWorkspace,
  OrganizeOverview,
  ThemeName,
  WikilinkCandidate,
} from '../m2'
import {
  applyBodyPreferences,
  formatCountdown,
  isUserTypingInEditor,
  slugifyMarkdownPath,
} from '../utils/wikra'

type DocTab = {
  id: string
  kind: 'page' | 'raw' | 'inbox' | 'review'
  ref: string
  label: string
}

type ToastItem = {
  id: number
  message: string
  actions?: { label: string; onClick: () => void }[]
}

const desktopWindow = isTauri() ? getCurrentWindow() : undefined

// ============================================================================
// App Settings, Theme & Window State
// ============================================================================
const settings = ref<AppSettings>({
  version: 1,
  lastWorkspaceRoot: null,
  lastCaptureDir: 'raw/notes',
  hotkey: '',
  autoOrganize: true,
  autoSyncMemory: false,
  theme: 'slate',
  docWidth: 'readable',
  allowNoteFulltextToModel: true,
  quietOrganize: true,
  bufferSeconds: 60,
})

const workspace = ref<{ session: string; root: string; captureDir: string; rawDir: string }>()
const opening = ref(true)
const isMaximized = ref(false)
const viewportWidth = ref(window.innerWidth)

const modelConfig = ref<ModelConfig | null>(null)
const memoryConfig = ref<MemoryConfig | null>(null)
const hotkey = ref<HotkeyStatus>()
const hotkeyBusy = ref(false)
const hotkeyError = ref<UiError | null>(null)

// ============================================================================
// Navigation, Tabs & Responsive Layout State
// ============================================================================
const activeWorkspace = ref<'docs' | 'organize' | 'graph'>('docs')
const docLeftMode = ref<'library' | 'inbox'>('library')
const libraryFilter = ref<'all' | 'wiki' | 'raw'>('all')
const inboxFilter = ref<'pending' | 'archived' | 'all'>('pending')
const inboxReadMode = ref<'full' | 'highlights'>('full')
const editorMode = ref<'preview' | 'source'>('preview')

const openDocTabs = ref<DocTab[]>([])
const activeDocTabId = ref('')
const activePagePath = ref('')
const activeRawPath = ref('')
const selectedInboxId = ref('')

const rightOpenByView = ref<Record<string, boolean>>({
  library: true,
  raw: true,
  inbox: true,
  organize: false,
  review: false,
  graph: false,
})
const rightTab = ref<'info' | 'chat'>('info')
const chatMode = ref<'query' | 'review'>('query')
const recallCurtainPagePath = ref<string | null>(null)
const pendingAskPrompt = ref<string | null>(null)

// Modals
const settingsOpen = ref(false)
const onboardingOpen = ref(false)
const bufferModalOpen = ref(false)

// ============================================================================
// Domain Data State
// ============================================================================
const libraryTree = ref<LibraryTree>({ rawDir: 'raw', wikiPages: [], rawFiles: [] })
const libraryLoading = ref(false)
const libraryError = ref<UiError | null>(null)
const creatingPage = ref(false)

const activeDoc = ref<DocumentDetail | null>(null)
const docLoading = ref(false)
const docError = ref<UiError | null>(null)
const externalChangedPaths = ref<Record<string, boolean>>({})

const evidenceByPage = ref<Record<string, DocCitation | null>>({})
const evidenceRawDoc = ref<DocumentDetail | null>(null)
const evidenceLoading = ref(false)
const reorganizingRaw = ref(false)

const inboxItems = ref<InboxItemSummary[]>([])
const inboxLoading = ref(false)
const activeInbox = ref<InboxItemDetail | null>(null)
const inboxDetailLoading = ref(false)
const inboxError = ref<UiError | null>(null)
const ingestingInbox = ref(false)
const compilingInbox = ref(false)
const completingStub = ref(false)
const togglingParaId = ref('')

const organizeOverview = ref<OrganizeOverview | null>(null)
const organizeLoading = ref(false)
const organizeError = ref<UiError | null>(null)
const bufferSecondsLeft = ref(0)
const bufferBusy = ref(false)
const revertingId = ref('')
const resolvingConflict = ref(false)
const conflictError = ref<UiError | null>(null)

// P0: Structure Lint cache, Wikilink candidates, Rollback & child-revert feedback
const wikilinkCandidates = ref<WikilinkCandidate[]>([])
const lintReport = ref<LintReport | null>(null)
const lintLoading = ref(false)
const lintError = ref<UiError | null>(null)
const lintFilterPagePath = ref<string | null>(null)
const rollbackFeedbackByCs = ref<Record<string, string>>({})
const revertBlockedByCs = ref<Record<string, { message: string; childIds: string[] }>>({})

const knowledgeGraph = ref<KnowledgeGraph | null>(null)
const graphLoading = ref(false)
const graphError = ref<UiError | null>(null)
const includeMetaPages = ref(false)

// Global Search (Ctrl+K)
const searchInputEl = ref<HTMLInputElement>()
const searchQuery = ref('')
const searchOpen = ref(false)
const searchHits = ref<GlobalSearchHit[]>([])
const searchBusy = ref(false)

// Toasts
const toasts = ref<ToastItem[]>([])
let nextToastId = 1

let searchDebounce: number | undefined
let bufferTickTimer: number | undefined
let docRequestSeq = 0
let inboxRequestSeq = 0
const unlisteners: (() => void)[] = []
let disposed = false

// ============================================================================
// Computed Selectors
// ============================================================================
const vaultName = computed(() => {
  const parts = workspace.value?.root.split(/[\\/]+/).filter(Boolean)
  return parts?.at(-1) || '未选择知识库'
})

const effectiveRawDir = computed(() => libraryTree.value.rawDir || workspace.value?.rawDir || 'raw')

const activeHotkeyLabel = computed(() => {
  const h = hotkey.value
  if (h?.active) return h.choices.find((c) => c.id === h.id)?.label
  return 'Win+Alt+N'
})

const currentTab = computed(() => openDocTabs.value.find((t) => t.id === activeDocTabId.value) || null)

const currentViewMode = computed<'library' | 'raw' | 'inbox' | 'organize' | 'review' | 'graph'>(() => {
  if (activeWorkspace.value === 'organize') return 'organize'
  if (activeWorkspace.value === 'graph') return 'graph'
  if (currentTab.value?.kind === 'review') return 'review'
  if (currentTab.value?.kind === 'inbox') return 'inbox'
  if (currentTab.value?.kind === 'raw') return 'raw'
  return 'library'
})

const isNarrow = computed(() => viewportWidth.value < 1240)
const isRightOpen = computed(() => Boolean(rightOpenByView.value[currentViewMode.value]))
const showLeftPane = computed(
  () =>
    Boolean(workspace.value)
    && activeWorkspace.value === 'docs'
    && currentViewMode.value !== 'review'
    && !(isNarrow.value && isRightOpen.value),
)
const showRightPane = computed(() => Boolean(workspace.value) && isRightOpen.value)

const openConflicts = computed(() =>
  (organizeOverview.value?.conflicts || []).filter((c) => c.status === 'open'),
)
const pendingReviewPageCount = computed(() => {
  const keys = new Set<string>()
  for (const cs of organizeOverview.value?.changesets || []) {
    if (cs.status === 'reverted') continue
    for (const op of cs.ops || []) {
      if (op.outcome === 'pending_review') {
        keys.add(`${cs.id}::${op.targetPath}`)
      }
    }
  }
  return keys.size
})
const organizeAttentionCount = computed(() => openConflicts.value.length + pendingReviewPageCount.value)
const activePageLintFindings = computed(() => {
  const docPath = activeDoc.value?.kind === 'wiki' ? activeDoc.value.path : ''
  if (!docPath || !lintReport.value) return []
  return lintReport.value.findings.filter(
    (f) => !f.dismissed && f.pagePath === docPath && (f.severity === 'error' || f.severity === 'warn'),
  )
})
const inboxPendingCount = computed(() =>
  inboxItems.value.filter((i) => i.state === 'unread' || i.state === 'reading').length,
)

const activeConflict = computed(() => {
  if (currentTab.value?.kind !== 'review') return null
  const cid = currentTab.value.ref
  return (organizeOverview.value?.conflicts || []).find((c) => c.id === cid) || openConflicts.value[0] || null
})

const wikiTitleMap = computed(() => {
  const map: Record<string, string> = {}
  for (const p of libraryTree.value.wikiPages) map[p.path] = p.title
  return map
})

const currentEvidenceCitation = computed(() =>
  activeDoc.value ? (evidenceByPage.value[activeDoc.value.path] ?? null) : null,
)

const teachbackPageTitle = computed(() => {
  if (!recallCurtainPagePath.value) return ''
  return wikiTitleMap.value[recallCurtainPagePath.value] || activeDoc.value?.title || recallCurtainPagePath.value
})

const isDark = computed(() => settings.value.theme !== 'paper')
const modelStatusLabel = computed(() =>
  modelConfig.value?.enabled && modelConfig.value?.model
    ? modelConfig.value.model
    : '未启用模型',
)
const memoryStatusLabel = computed(() =>
  memoryConfig.value?.enabled ? 'OpenViking 本地' : '已停用',
)

// ============================================================================
// Toast & Helpers
// ============================================================================
function showToast(message: string, actions?: { label: string; onClick: () => void }[]) {
  const id = nextToastId++
  toasts.value = [...toasts.value, { id, message, actions }]
  window.setTimeout(() => {
    toasts.value = toasts.value.filter((t) => t.id !== id)
  }, actions?.length ? 4600 : 2800)
}

function runToastAction(toastId: number, fn: () => void) {
  toasts.value = toasts.value.filter((t) => t.id !== toastId)
  fn()
}

function trackUnlisten(fn: () => void) {
  if (disposed) fn()
  else unlisteners.push(fn)
}

function setRightOpen(val: boolean) {
  const mode = currentViewMode.value
  rightOpenByView.value = { ...rightOpenByView.value, [mode]: val }
  if (val && isNarrow.value && mode === 'library' && activeDoc.value) {
    evidenceByPage.value = { ...evidenceByPage.value, [activeDoc.value.path]: null }
  }
}

function resetRecallIfDifferentPage(targetPath?: string) {
  if (recallCurtainPagePath.value && recallCurtainPagePath.value !== targetPath) {
    recallCurtainPagePath.value = null
    chatMode.value = 'query'
  }
}

function syncBufferTimer() {
  if (bufferSecondsLeft.value > 0 && !bufferTickTimer) {
    bufferTickTimer = window.setInterval(() => {
      if (bufferSecondsLeft.value > 1) {
        bufferSecondsLeft.value -= 1
      } else {
        bufferSecondsLeft.value = 0
        if (bufferTickTimer) {
          window.clearInterval(bufferTickTimer)
          bufferTickTimer = undefined
        }
        if (workspace.value) void refreshOrganizeOverview(true)
      }
    }, 1000)
  } else if (bufferSecondsLeft.value <= 0 && bufferTickTimer) {
    window.clearInterval(bufferTickTimer)
    bufferTickTimer = undefined
  }
}

// ============================================================================
// Theme & Preference Persistence
// ============================================================================
async function loadAppSettings() {
  try {
    const loaded = await api.getAppSettings()
    settings.value = {
      ...settings.value,
      ...loaded,
      theme: loaded.theme === 'paper' ? 'paper' : 'slate',
      docWidth: loaded.docWidth === 'full' ? 'full' : 'readable',
    }
    applyBodyPreferences(settings.value.theme, settings.value.docWidth)
  } catch {
    applyBodyPreferences(settings.value.theme, settings.value.docWidth)
  }
}

async function toggleTheme() {
  const next: ThemeName = isDark.value ? 'paper' : 'slate'
  settings.value = { ...settings.value, theme: next }
  applyBodyPreferences(next, settings.value.docWidth)
  try {
    const saved = await api.saveAppSettings({ theme: next })
    settings.value = { ...settings.value, ...saved }
  } catch {
    // Keep visual update even if settings persist fails.
  }
}

async function toggleDocWidth() {
  const next: DocWidthMode = settings.value.docWidth === 'readable' ? 'full' : 'readable'
  settings.value = { ...settings.value, docWidth: next }
  applyBodyPreferences(settings.value.theme, next)
  try {
    const saved = await api.saveAppSettings({ docWidth: next })
    settings.value = { ...settings.value, ...saved }
  } catch {
    // Keep visual update.
  }
}

function handleSettingsSaved(saved: AppSettings) {
  settings.value = { ...settings.value, ...saved }
  applyBodyPreferences(settings.value.theme, settings.value.docWidth)
}

async function handleWorkspaceDirsUpdated(updated: OpenedWorkspace) {
  if (!workspace.value) return
  const nextRawDir = updated.rawDir || 'raw'
  workspace.value = {
    ...workspace.value,
    session: updated.session || workspace.value.session,
    root: updated.root || workspace.value.root,
    rawDir: nextRawDir,
    captureDir: updated.captureDir,
  }
  showToast(`已更新目录设置：原始文件位于 ${nextRawDir}/`)
  await Promise.all([refreshLibraryTree(), refreshOrganizeOverview(true)])
  const rawSet = new Set(libraryTree.value.rawFiles.map((r) => r.path))
  const wikiMap = new Map(libraryTree.value.wikiPages.map((p) => [p.path, p.title]))
  const rawMap = new Map(libraryTree.value.rawFiles.map((r) => [r.path, r.title]))
  let nextActiveId = activeDocTabId.value
  openDocTabs.value = openDocTabs.value.map((tab) => {
    if (tab.kind !== 'page' && tab.kind !== 'raw') return tab
    const isNowRaw = rawSet.has(tab.ref)
    const nextKind: 'page' | 'raw' = isNowRaw ? 'raw' : 'page'
    const nextId = `${nextKind}:${tab.ref}`
    if (tab.id === activeDocTabId.value) nextActiveId = nextId
    const title = isNowRaw
      ? `原文：${(rawMap.get(tab.ref) || tab.ref).slice(0, 14)}`
      : (wikiMap.get(tab.ref) || tab.label)
    return { ...tab, id: nextId, kind: nextKind, label: title }
  })
  activeDocTabId.value = nextActiveId
  if (activeDoc.value) {
    await loadDocument(activeDoc.value.path)
  }
}

// ============================================================================
// Workspace Lifecycle & Data Loading
// ============================================================================
function applyWorkspace(opened: OpenedWorkspace) {
  workspace.value = {
    session: opened.session,
    root: opened.root,
    captureDir: opened.captureDir,
    rawDir: opened.rawDir || 'raw',
  }
  onboardingOpen.value = false
  activeWorkspace.value = 'docs'
  docLeftMode.value = 'library'
  openDocTabs.value = []
  activeDocTabId.value = ''
  activePagePath.value = ''
  activeRawPath.value = ''
  selectedInboxId.value = ''
  activeDoc.value = null
  activeInbox.value = null
  evidenceByPage.value = {}
  evidenceRawDoc.value = null
  externalChangedPaths.value = {}
  recallCurtainPagePath.value = null
  chatMode.value = 'query'
  knowledgeGraph.value = null
  searchQuery.value = ''
  searchOpen.value = false
  searchHits.value = []
  wikilinkCandidates.value = []
  lintReport.value = null
  lintError.value = null
  lintFilterPagePath.value = null
  rollbackFeedbackByCs.value = {}
  revertBlockedByCs.value = {}

  if (opened.recovered.length > 0) {
    showToast(`已自动恢复 ${opened.recovered.length} 条未完成随手记`)
  }
  void refreshAllData(opened.session, true)
}

async function refreshAllData(session: string, openInitialDoc = false) {
  await Promise.all([
    refreshIntegrations(session),
    refreshLibraryTree(openInitialDoc),
    refreshInboxList(),
    refreshOrganizeOverview(),
    refreshWikilinkCandidates(session),
  ])
  // Background L1 structure lint cache so wiki toolbar badge appears without blocking open.
  void refreshLintReport(true)
}

async function refreshWikilinkCandidates(sessionOverride?: string) {
  const session = sessionOverride || workspace.value?.session
  if (!session) return
  try {
    const list = await api.listWikilinkCandidates(session)
    if (workspace.value?.session === session) {
      wikilinkCandidates.value = list
    }
  } catch {
    // Non-fatal: keep existing candidates.
  }
}

async function refreshLintReport(fromBackground = false) {
  const session = workspace.value?.session
  if (!session) return
  if (!fromBackground) {
    lintLoading.value = true
    lintError.value = null
  }
  try {
    const report = await api.runStructureLint(session)
    if (workspace.value?.session === session) {
      lintReport.value = report
    }
  } catch (err) {
    if (!fromBackground && workspace.value?.session === session) {
      lintError.value = parseAppError(err)
    }
  } finally {
    if (!fromBackground && workspace.value?.session === session) {
      lintLoading.value = false
    }
  }
}

async function refreshIntegrations(session: string) {
  try {
    const config = await api.getIntegrationsConfig(session)
    if (workspace.value?.session === session) {
      modelConfig.value = config.model
      memoryConfig.value = config.memory
    }
  } catch {
    // Errors surfaced in settings modal.
  }
}

async function refreshLibraryTree(openInitial = false) {
  const session = workspace.value?.session
  if (!session) return
  libraryLoading.value = true
  libraryError.value = null
  try {
    const tree = await api.listLibraryTree(session)
    if (workspace.value?.session !== session) return
    libraryTree.value = tree
    if (tree.rawDir && workspace.value && workspace.value.rawDir !== tree.rawDir) {
      workspace.value = { ...workspace.value, rawDir: tree.rawDir }
    }
    void refreshWikilinkCandidates(session)
    if (openInitial && !openDocTabs.value.length) {
      if (tree.wikiPages.length > 0) {
        void openWikiPage(tree.wikiPages[0].path)
      } else if (tree.rawFiles.length > 0) {
        void openRawFile(tree.rawFiles[0].path)
      }
    }
  } catch (err) {
    if (workspace.value?.session === session) libraryError.value = parseAppError(err)
  } finally {
    if (workspace.value?.session === session) libraryLoading.value = false
  }
}

async function loadDocument(path: string, fromBackground = false) {
  const session = workspace.value?.session
  if (!session || !path) return

  // Guardrail: never clobber an active editor session during background events!
  if (fromBackground && editorMode.value === 'source' && isUserTypingInEditor() && activeDoc.value?.path === path) {
    externalChangedPaths.value = { ...externalChangedPaths.value, [path]: true }
    return
  }

  const seq = ++docRequestSeq
  if (!fromBackground) {
    docLoading.value = true
    docError.value = null
  }
  try {
    const detail = await api.readDocumentDetail(session, path)
    if (seq !== docRequestSeq || workspace.value?.session !== session) return
    if (fromBackground && editorMode.value === 'source' && activeDoc.value?.path === path && activeDoc.value.contentHash !== detail.contentHash) {
      externalChangedPaths.value = { ...externalChangedPaths.value, [path]: true }
      return
    }
    activeDoc.value = detail
    externalChangedPaths.value = { ...externalChangedPaths.value, [path]: false }
    // Sync tab label if title changed
    const tabId = `${detail.kind === 'raw' ? 'raw' : 'page'}:${detail.path}`
    openDocTabs.value = openDocTabs.value.map((t) =>
      t.id === tabId
        ? { ...t, label: detail.kind === 'raw' ? `原文：${detail.title.slice(0, 14)}` : detail.title }
        : t,
    )
  } catch (err) {
    if (seq === docRequestSeq && !fromBackground) {
      docError.value = parseAppError(err)
    }
  } finally {
    if (seq === docRequestSeq) docLoading.value = false
  }
}

async function refreshInboxList() {
  const session = workspace.value?.session
  if (!session) return
  inboxLoading.value = true
  try {
    const items = await api.listInboxItems(session)
    if (workspace.value?.session === session) {
      inboxItems.value = items
      if (!selectedInboxId.value && items.length > 0) {
        selectedInboxId.value = items[0].id
      }
    }
  } catch {
    // Handled on inbox open.
  } finally {
    if (workspace.value?.session === session) inboxLoading.value = false
  }
}

async function loadInboxDetail(id: string) {
  const session = workspace.value?.session
  if (!session || !id) return
  const seq = ++inboxRequestSeq
  inboxDetailLoading.value = true
  inboxError.value = null
  try {
    const detail = await api.getInboxItem(session, id)
    if (seq === inboxRequestSeq && workspace.value?.session === session) {
      activeInbox.value = detail
    }
  } catch (err) {
    if (seq === inboxRequestSeq) inboxError.value = parseAppError(err)
  } finally {
    if (seq === inboxRequestSeq) inboxDetailLoading.value = false
  }
}

async function refreshOrganizeOverview(fromBackground = false) {
  const session = workspace.value?.session
  if (!session) return
  if (!fromBackground) {
    organizeLoading.value = true
    organizeError.value = null
  }
  try {
    const ov = await api.listOrganizeOverview(session)
    if (workspace.value?.session !== session) return
    organizeOverview.value = ov
    bufferSecondsLeft.value = ov.bufferSecondsLeft
    syncBufferTimer()
  } catch (err) {
    if (!fromBackground && workspace.value?.session === session) {
      organizeError.value = parseAppError(err)
    }
  } finally {
    if (!fromBackground && workspace.value?.session === session) {
      organizeLoading.value = false
    }
  }
}

async function refreshKnowledgeGraph() {
  const session = workspace.value?.session
  if (!session) return
  graphLoading.value = true
  graphError.value = null
  try {
    const g = await api.getKnowledgeGraph(session, includeMetaPages.value)
    if (workspace.value?.session === session) {
      knowledgeGraph.value = g
    }
  } catch (err) {
    if (workspace.value?.session === session) graphError.value = parseAppError(err)
  } finally {
    if (workspace.value?.session === session) graphLoading.value = false
  }
}

// ============================================================================
// Navigation & Tab Actions
// ============================================================================
function openDocTab(tab: DocTab) {
  activeWorkspace.value = 'docs'
  if (!openDocTabs.value.some((t) => t.id === tab.id)) {
    openDocTabs.value = [...openDocTabs.value, tab]
  }
  activeDocTabId.value = tab.id
  if (tab.kind === 'page') {
    resetRecallIfDifferentPage(tab.ref)
    docLeftMode.value = 'library'
    activePagePath.value = tab.ref
    void loadDocument(tab.ref)
  } else if (tab.kind === 'raw') {
    resetRecallIfDifferentPage(undefined)
    docLeftMode.value = 'library'
    activeRawPath.value = tab.ref
    void loadDocument(tab.ref)
  } else if (tab.kind === 'inbox') {
    resetRecallIfDifferentPage(undefined)
    docLeftMode.value = 'inbox'
    selectedInboxId.value = tab.ref
    void loadInboxDetail(tab.ref)
  } else if (tab.kind === 'review') {
    resetRecallIfDifferentPage(undefined)
    rightOpenByView.value = { ...rightOpenByView.value, review: false }
  }
}

function closeDocTab(tabId: string) {
  const idx = openDocTabs.value.findIndex((t) => t.id === tabId)
  if (idx < 0) return
  const next = openDocTabs.value.filter((t) => t.id !== tabId)
  openDocTabs.value = next
  if (next.length === 0) {
    activeDocTabId.value = ''
    activeDoc.value = null
    activeInbox.value = null
    resetRecallIfDifferentPage(undefined)
    return
  }
  if (activeDocTabId.value === tabId) {
    openDocTab(next[Math.max(0, idx - 1)])
  }
}

async function openWikiPage(path: string) {
  const summary = libraryTree.value.wikiPages.find((p) => p.path === path)
  if (!summary && libraryTree.value.rawFiles.some((r) => r.path === path)) {
    await openRawFile(path)
    return
  }
  const label = summary?.title || path.split('/').pop()?.replace(/\.md$/i, '') || path
  openDocTab({ id: `page:${path}`, kind: 'page', ref: path, label })
}

async function openRawFile(path: string) {
  const summary = libraryTree.value.rawFiles.find((r) => r.path === path)
  const rawTitle = summary?.title || path.split('/').pop()?.replace(/\.md$/i, '') || path
  openDocTab({ id: `raw:${path}`, kind: 'raw', ref: path, label: `原文：${rawTitle.slice(0, 14)}` })
}

function openInboxItem(id: string) {
  const item = inboxItems.value.find((i) => i.id === id)
  const label = item?.title || '收件箱文章'
  openDocTab({ id: `inbox:${id}`, kind: 'inbox', ref: id, label })
}

function openConflictReview(conflictId?: string) {
  const target =
    (conflictId ? organizeOverview.value?.conflicts.find((c) => c.id === conflictId) : null)
    || openConflicts.value[0]
    || organizeOverview.value?.conflicts[0]
  if (!target) {
    switchWorkspace('organize')
    return
  }
  openDocTab({
    id: `review:${target.id}`,
    kind: 'review',
    ref: target.id,
    label: `对比确认 · ${target.topic.slice(0, 12)}`,
  })
}

function switchWorkspace(ws: 'library' | 'inbox' | 'organize' | 'graph') {
  if (ws === 'organize') {
    activeWorkspace.value = 'organize'
    resetRecallIfDifferentPage(undefined)
    void refreshOrganizeOverview()
    void refreshLintReport(!lintReport.value ? false : true)
    return
  }
  if (ws === 'graph') {
    activeWorkspace.value = 'graph'
    resetRecallIfDifferentPage(undefined)
    void refreshKnowledgeGraph()
    return
  }
  if (ws === 'inbox') {
    activeWorkspace.value = 'docs'
    docLeftMode.value = 'inbox'
    resetRecallIfDifferentPage(undefined)
    void refreshInboxList()
    if (selectedInboxId.value) {
      openInboxItem(selectedInboxId.value)
    } else if (inboxItems.value.length > 0) {
      openInboxItem(inboxItems.value[0].id)
    } else {
      activeDocTabId.value = 'inbox:__empty__'
      openDocTabs.value = openDocTabs.value.filter((t) => t.id !== 'inbox:__empty__')
      openDocTabs.value = [...openDocTabs.value, { id: 'inbox:__empty__', kind: 'inbox', ref: '', label: '收件箱' }]
      activeInbox.value = null
    }
    return
  }
  // library
  activeWorkspace.value = 'docs'
  docLeftMode.value = 'library'
  if (activePagePath.value) {
    void openWikiPage(activePagePath.value)
  } else if (libraryTree.value.wikiPages.length > 0) {
    void openWikiPage(libraryTree.value.wikiPages[0].path)
  } else if (libraryTree.value.rawFiles.length > 0) {
    void openRawFile(libraryTree.value.rawFiles[0].path)
  }
}

// ============================================================================
// Document & Evidence Interactions
// ============================================================================
async function handleCreateWikiPage(nameOrPath: string) {
  const session = workspace.value?.session
  if (!session || creatingPage.value) return
  const relPath = slugifyMarkdownPath(nameOrPath)
  if (!relPath) return
  const title = nameOrPath.replace(/\.md$/i, '').split('/').pop()?.trim() || '新建知识页'
  const initialBody = `---\ntype: concept\ntags: []\nwikra-core: false\n---\n\n# ${title}\n\n## 核心要点\n\n`
  creatingPage.value = true
  try {
    const created = await api.createMarkdownDocument(session, relPath, initialBody)
    await refreshLibraryTree()
    editorMode.value = 'source'
    openDocTab({ id: `page:${created.path}`, kind: 'page', ref: created.path, label: created.title })
    activeDoc.value = created
    showToast(`已新建知识页：${created.title}`)
  } catch (err) {
    showToast(parseAppError(err).safeMessage)
  } finally {
    creatingPage.value = false
  }
}

function handleDocumentSaved(updated: DocumentDetail) {
  activeDoc.value = updated
  externalChangedPaths.value = { ...externalChangedPaths.value, [updated.path]: false }
  void refreshLibraryTree()
  void refreshLintReport(true)
  showToast('已保存修改至本地 Markdown 文件')
}

function handleOpenPageLint(path: string) {
  lintFilterPagePath.value = path
  activeWorkspace.value = 'organize'
  resetRecallIfDifferentPage(undefined)
  void refreshOrganizeOverview()
  void refreshLintReport(false)
}

async function handleToggleTask(task: DocTaskItem) {
  const session = workspace.value?.session
  const doc = activeDoc.value
  if (!session || !doc) return
  try {
    await api.toggleTask(session, doc.path, task.id, task.expectedLine, !task.done)
    await loadDocument(doc.path)
    showToast('已更新待办状态')
  } catch (err) {
    showToast(parseAppError(err).safeMessage)
  }
}

async function openEvidenceSplit(citation: DocCitation) {
  const pagePath = activeDoc.value?.path || activePagePath.value
  if (!pagePath) return
  evidenceByPage.value = { ...evidenceByPage.value, [pagePath]: citation }
  if (viewportWidth.value < 1680) {
    rightOpenByView.value = { ...rightOpenByView.value, library: false }
  }
  evidenceRawDoc.value = null
  if (citation.rawPath && workspace.value?.session) {
    evidenceLoading.value = true
    try {
      evidenceRawDoc.value = await api.readDocumentDetail(workspace.value.session, citation.rawPath)
    } catch {
      evidenceRawDoc.value = null
    } finally {
      evidenceLoading.value = false
    }
  }
}

function toggleSplitOrigin() {
  const doc = activeDoc.value
  if (!doc) return
  if (evidenceByPage.value[doc.path]) {
    evidenceByPage.value = { ...evidenceByPage.value, [doc.path]: null }
    evidenceRawDoc.value = null
    return
  }
  if (doc.citations.length > 0) {
    void openEvidenceSplit(doc.citations[0])
  }
}

function closeEvidenceSplit() {
  if (!activeDoc.value) return
  evidenceByPage.value = { ...evidenceByPage.value, [activeDoc.value.path]: null }
  evidenceRawDoc.value = null
}

function handleOpenWikilink(target: string) {
  const normalized = target.trim().toLowerCase()
  const matchedWiki = libraryTree.value.wikiPages.find(
    (p) =>
      p.path.toLowerCase() === normalized
      || p.path.toLowerCase() === `${normalized}.md`
      || p.title.toLowerCase() === normalized
      || p.path.toLowerCase().endsWith(`/${normalized}.md`),
  )
  if (matchedWiki) {
    void openWikiPage(matchedWiki.path)
    return
  }
  const matchedRaw = libraryTree.value.rawFiles.find(
    (r) =>
      r.path.toLowerCase() === normalized
      || r.path.toLowerCase() === `${normalized}.md`
      || r.title.toLowerCase() === normalized,
  )
  if (matchedRaw) {
    void openRawFile(matchedRaw.path)
    return
  }
  showToast(`未找到本地文档「${target}」`)
}

async function handleOpenObsidian(path: string) {
  const session = workspace.value?.session
  if (!session) return
  try {
    await api.openWorkspaceFile(session, path, 'open')
    showToast('已用系统关联程序打开本地文件')
  } catch (err) {
    showToast(parseAppError(err).safeMessage)
  }
}

async function handleOpenExternalUrl(url: string) {
  try {
    await api.openExternalUrl(url)
  } catch (err) {
    showToast(parseAppError(err).safeMessage)
  }
}

async function handleReorganizeRaw(path: string) {
  const session = workspace.value?.session
  if (!session || reorganizingRaw.value) return
  reorganizingRaw.value = true
  try {
    await api.triggerNoteOrganize(session, path)
    showToast('已重新发起整理，完成后会自动同步知识页')
    void refreshOrganizeOverview(true)
  } catch (err) {
    showToast(parseAppError(err).safeMessage)
  } finally {
    reorganizingRaw.value = false
  }
}

// ============================================================================
// Inbox Actions
// ============================================================================
async function handleInboxIngest(input: string) {
  const session = workspace.value?.session
  if (!session || ingestingInbox.value) return
  ingestingInbox.value = true
  inboxError.value = null
  try {
    const created = await api.ingestInboxInput(session, input)
    await Promise.all([refreshInboxList(), refreshLibraryTree()])
    // Remove placeholder empty tab if present
    openDocTabs.value = openDocTabs.value.filter((t) => t.id !== 'inbox:__empty__')
    openDocTab({ id: `inbox:${created.id}`, kind: 'inbox', ref: created.id, label: created.title })
    activeInbox.value = created
    showToast(
      created.fetchStatus === 'stub'
        ? '该网页需登录后查看，已保存链接等待粘贴正文'
        : '已收入收件箱，可直接阅读原文并标重点',
    )
  } catch (err) {
    inboxError.value = parseAppError(err)
  } finally {
    ingestingInbox.value = false
  }
}

async function handleToggleInboxParagraph(paragraphId: string) {
  const session = workspace.value?.session
  const item = activeInbox.value
  if (!session || !item || togglingParaId.value) return
  togglingParaId.value = paragraphId
  try {
    const updated = await api.toggleInboxParagraph(session, item.id, paragraphId)
    activeInbox.value = updated
    await refreshInboxList()
  } catch (err) {
    inboxError.value = parseAppError(err)
  } finally {
    togglingParaId.value = ''
  }
}

async function handleCompileInbox(mode: 'excerpt' | 'full' | 'archive') {
  const session = workspace.value?.session
  const item = activeInbox.value
  if (!session || !item || compilingInbox.value) return
  compilingInbox.value = true
  inboxError.value = null
  try {
    const updated = await api.compileInboxItem(session, item.id, mode)
    activeInbox.value = updated
    await Promise.all([refreshInboxList(), refreshLibraryTree(), refreshOrganizeOverview(true)])
    void refreshLintReport(true)
    if (mode === 'archive') {
      showToast('已存档，原文仍保留在知识库「原文」中可随时阅读')
    } else if (updated.compileSummary) {
      const s = updated.compileSummary
      const parts = [`已写入 ${s.applied} 处`]
      if (s.pending > 0) parts.push(`${s.pending} 处等待你确认`)
      if (s.remainingParagraphs > 0) parts.push(`剩余 ${s.remainingParagraphs} 段可继续提炼`)
      showToast(`已完成提炼：${parts.join('，')}`, [
        { label: '查看整理记录', onClick: () => switchWorkspace('organize') },
      ])
    } else {
      showToast('已完成提炼并更新知识库', [
        { label: '查看整理记录', onClick: () => switchWorkspace('organize') },
      ])
    }
  } catch (err) {
    inboxError.value = parseAppError(err)
  } finally {
    compilingInbox.value = false
  }
}

async function handleCompleteStub(pastedBody: string) {
  const session = workspace.value?.session
  const item = activeInbox.value
  if (!session || !item || completingStub.value) return
  completingStub.value = true
  inboxError.value = null
  try {
    const updated = await api.completeInboxStub(session, item.id, pastedBody)
    activeInbox.value = updated
    await Promise.all([refreshInboxList(), refreshLibraryTree()])
    showToast('已存为本地原文，现在可阅读并标重点提炼')
  } catch (err) {
    inboxError.value = parseAppError(err)
  } finally {
    completingStub.value = false
  }
}

// ============================================================================
// Organize, Buffer Queue, Conflicts & Graph Actions
// ============================================================================
async function handleCancelBufferItem(itemId: string) {
  const session = workspace.value?.session
  if (!session || bufferBusy.value) return
  bufferBusy.value = true
  try {
    const ov = await api.cancelBufferItem(session, itemId)
    organizeOverview.value = ov
    bufferSecondsLeft.value = ov.bufferSecondsLeft
    syncBufferTimer()
    showToast('已移出本次整理队列（本地原文保留）')
  } catch (err) {
    showToast(parseAppError(err).safeMessage)
  } finally {
    bufferBusy.value = false
  }
}

async function handleFlushBufferNow() {
  const session = workspace.value?.session
  if (!session || bufferBusy.value) return
  bufferBusy.value = true
  try {
    const ov = await api.flushBufferNow(session)
    organizeOverview.value = ov
    bufferSecondsLeft.value = ov.bufferSecondsLeft
    syncBufferTimer()
    bufferModalOpen.value = false
    showToast('已立即触发合并整理')
    void refreshLibraryTree()
  } catch (err) {
    showToast(parseAppError(err).safeMessage)
  } finally {
    bufferBusy.value = false
  }
}

async function handleRevertChangeset(id: string) {
  const session = workspace.value?.session
  if (!session || revertingId.value) return
  revertingId.value = id
  try {
    const res = await api.revertChangeset(session, id)

    // Clear blocked error for this changeset and any parent changeset unblocked by reverting this child.
    const nextBlocked = { ...revertBlockedByCs.value }
    delete nextBlocked[id]
    for (const [parentId, entry] of Object.entries(nextBlocked)) {
      const remaining = entry.childIds.filter((cid) => cid !== id)
      if (remaining.length !== entry.childIds.length) {
        if (remaining.length === 0) delete nextBlocked[parentId]
        else nextBlocked[parentId] = { ...entry, childIds: remaining }
      }
    }
    revertBlockedByCs.value = nextBlocked

    const restoredUnique = Array.from(new Set(res.restored || []))
    const keptUnique = Array.from(new Set(res.keptModified || []))
    const pageTitleMap = new Map<string, string>()
    for (const p of res.changeset.pages || []) pageTitleMap.set(p.path, p.title)
    const resolveTitle = (path: string) =>
      pageTitleMap.get(path)
      || wikiTitleMap.value[path]
      || path.split('/').pop()?.replace(/\.md$/i, '')
      || path

    let feedback = ''
    if (restoredUnique.length > 0 && keptUnique.length > 0) {
      const keptLabels = keptUnique.map((p) => `《${resolveTitle(p)}》`).join('、')
      feedback = `已还原 ${restoredUnique.length} 篇；${keptLabels}因你后续手动修改，已原样保留。`
    } else if (restoredUnique.length === 0 && keptUnique.length > 0) {
      const keptLabels = keptUnique.map((p) => `《${resolveTitle(p)}》`).join('、')
      feedback = `${keptLabels}因你后续手动修改，已原样保留。`
    } else if (restoredUnique.length > 0) {
      feedback = `已还原 ${restoredUnique.length} 篇知识页（你手动写的内容保持不动）。`
    } else {
      feedback = '已撤销此次整理并清理待确认项。'
    }

    rollbackFeedbackByCs.value = { ...rollbackFeedbackByCs.value, [id]: feedback }
    await Promise.all([refreshOrganizeOverview(), refreshLibraryTree()])
    void refreshLintReport(true)
    if (activeDoc.value) void loadDocument(activeDoc.value.path)
    showToast(feedback)
  } catch (err) {
    const parsed = parseAppError(err)
    const childIds = (organizeOverview.value?.changesets || [])
      .filter((c) => c.parentId === id && c.status !== 'reverted')
      .map((c) => c.id)
    revertBlockedByCs.value = {
      ...revertBlockedByCs.value,
      [id]: { message: parsed.safeMessage, childIds },
    }
    if (activeWorkspace.value !== 'organize') {
      showToast(parsed.safeMessage, [
        { label: '去整理页查看', onClick: () => switchWorkspace('organize') },
      ])
    }
  } finally {
    revertingId.value = ''
  }
}

function handleLintUpdated(report: LintReport) {
  lintReport.value = report
  void Promise.all([refreshOrganizeOverview(true), refreshLibraryTree()])
  if (activeDoc.value) void loadDocument(activeDoc.value.path, true)
}

async function handleReviewDecided(payload: {
  decision: 'apply' | 'discard'
  changeset: ChangesetItem
  pageTitle: string
}) {
  await Promise.all([refreshOrganizeOverview(), refreshLibraryTree()])
  void refreshLintReport(true)
  if (activeDoc.value) void loadDocument(activeDoc.value.path, true)
  showToast(
    payload.decision === 'apply'
      ? `已采纳并写入《${payload.pageTitle}》，随时可一键撤销`
      : `已丢弃《${payload.pageTitle}》的待审改动`,
  )
}

async function handleEscalatedConflict(_conflict: ConflictItem) {
  await Promise.all([refreshOrganizeOverview(), refreshLibraryTree()])
  showToast('已转为观点冲突，可在上方卡片左右对比确认')
}

async function handleResolveConflict(payload: {
  id: string
  status: ConflictStatus
  resolutionNote?: string
  wikiTarget?: string
}) {
  const session = workspace.value?.session
  if (!session || resolvingConflict.value) return
  resolvingConflict.value = true
  conflictError.value = null
  try {
    await api.resolveConflict({ session, ...payload })
    await Promise.all([refreshOrganizeOverview(), refreshLibraryTree()])
    showToast('已确认冲突结论并同步知识库')
  } catch (err) {
    conflictError.value = parseAppError(err)
  } finally {
    resolvingConflict.value = false
  }
}

async function handleDismissGraphGap(gapId: string) {
  const session = workspace.value?.session
  if (!session) return
  try {
    await api.dismissGraphGap(session, gapId)
    await refreshKnowledgeGraph()
    showToast('已忽略该关联建议')
  } catch (err) {
    showToast(parseAppError(err).safeMessage)
  }
}

function handleAskGraphGap(gap: GraphGap) {
  rightOpenByView.value = { ...rightOpenByView.value, graph: true }
  rightTab.value = 'chat'
  chatMode.value = 'query'
  pendingAskPrompt.value = gap.question
}

// ============================================================================
// Active Recall ("让我讲一遍") & Chat Helpers
// ============================================================================
function startTeachback(pagePath: string) {
  recallCurtainPagePath.value = pagePath
  chatMode.value = 'review'
  rightTab.value = 'chat'
  rightOpenByView.value = { ...rightOpenByView.value, library: true }
  editorMode.value = 'preview'
  void openWikiPage(pagePath)
}

function exitTeachback() {
  recallCurtainPagePath.value = null
  chatMode.value = 'query'
}

function handleUnderstandingAppended(updated: DocumentDetail) {
  activeDoc.value = updated
  void refreshLibraryTree()
  showToast('已写入受手动保护的「我的理解」')
}

function handleAnswerSaved(created: DocumentDetail) {
  void refreshLibraryTree()
  showToast(`已沉淀为知识页：${created.title}`, [
    { label: '打开查看', onClick: () => void openWikiPage(created.path) },
  ])
}

// ============================================================================
// Global Cross-View Search (Ctrl+K)
// ============================================================================
watch(searchQuery, (q) => {
  window.clearTimeout(searchDebounce)
  const trimmed = q.trim()
  if (!trimmed || !workspace.value?.session) {
    searchHits.value = []
    searchOpen.value = false
    return
  }
  searchOpen.value = true
  searchBusy.value = true
  const session = workspace.value.session
  searchDebounce = window.setTimeout(async () => {
    try {
      const hits = await api.globalSearch(session, trimmed, 8)
      if (searchQuery.value.trim() === trimmed) searchHits.value = hits
    } catch {
      searchHits.value = []
    } finally {
      searchBusy.value = false
    }
  }, 150)
})

async function selectSearchHit(hit: GlobalSearchHit) {
  searchOpen.value = false
  if (hit.kind === 'raw') {
    await openRawFile(hit.targetPath)
    return
  }
  await openWikiPage(hit.targetPath)
  if (hit.citationId) {
    await loadDocument(hit.targetPath)
    const matched = activeDoc.value?.citations.find((c) => c.id === hit.citationId)
    if (matched) {
      void openEvidenceSplit(matched)
    }
  }
}

function askFromGlobalSearch() {
  const q = searchQuery.value.trim()
  if (!q) return
  searchOpen.value = false
  setRightOpen(true)
  rightTab.value = 'chat'
  chatMode.value = 'query'
  pendingAskPrompt.value = q
}

// ============================================================================
// Desktop Window Controls, Hotkey & Shortcuts
// ============================================================================
async function syncMaximized() {
  if (!desktopWindow) return
  try {
    isMaximized.value = await api.isMainMaximized()
  } catch {
    // Ignore transient query errors.
  }
}

async function desktopAction(action: 'drag' | 'minimize' | 'maximize' | 'close') {
  if (!desktopWindow) return
  try {
    await api.desktopWindowAction(action)
    if (action === 'maximize') void syncMaximized()
  } catch (err) {
    showToast(parseAppError(err).safeMessage)
  }
}

async function updateHotkey(choice?: string) {
  if (hotkeyBusy.value) return
  hotkeyBusy.value = true
  hotkeyError.value = null
  try {
    const res = choice === undefined ? await api.getHotkey() : await api.setHotkey(choice)
    hotkey.value = res
    hotkeyError.value = res.error ? { safeMessage: res.error } : null
  } catch (err) {
    hotkeyError.value = parseAppError(err)
  } finally {
    hotkeyBusy.value = false
  }
}

async function openQuickCapture() {
  const session = workspace.value?.session
  if (!session) {
    onboardingOpen.value = true
    return
  }
  try {
    await api.showCaptureWindow(session)
  } catch (err) {
    showToast(parseAppError(err).safeMessage)
  }
}

function handleGlobalKeydown(e: KeyboardEvent) {
  if (e.isComposing || e.keyCode === 229) return
  if (settingsOpen.value || onboardingOpen.value || bufferModalOpen.value) return
  if (e.ctrlKey && e.key.toLowerCase() === 'k') {
    e.preventDefault()
    searchInputEl.value?.focus()
    if (searchQuery.value.trim()) searchOpen.value = true
  } else if (e.ctrlKey && e.key.toLowerCase() === 'e' && currentViewMode.value === 'library' && activeDoc.value) {
    e.preventDefault()
    editorMode.value = editorMode.value === 'preview' ? 'source' : 'preview'
  } else if (e.ctrlKey && e.key.toLowerCase() === 'j' && workspace.value) {
    e.preventDefault()
    setRightOpen(!isRightOpen.value)
  } else if (e.ctrlKey && e.key.toLowerCase() === 'g' && workspace.value) {
    e.preventDefault()
    switchWorkspace('graph')
  }
}

function handleResize() {
  viewportWidth.value = window.innerWidth
}

function handleClickOutsideSearch(e: MouseEvent) {
  if (!searchOpen.value) return
  const target = e.target as HTMLElement | null
  if (!target?.closest('.global-search-wrap')) {
    searchOpen.value = false
  }
}

// ============================================================================
// Lifecycle & Backend Event Listeners (Focus-Safe!)
// ============================================================================
onMounted(async () => {
  window.addEventListener('keydown', handleGlobalKeydown)
  window.addEventListener('resize', handleResize)
  window.addEventListener('mousedown', handleClickOutsideSearch)

  await loadAppSettings()

  try {
    if (desktopWindow) {
      trackUnlisten(await desktopWindow.listen('capture-closed', () => {
        void refreshLibraryTree()
        void refreshOrganizeOverview(true)
      }))
      trackUnlisten(await desktopWindow.listen<{ bufferSecondsLeft?: number }>('wikra-buffer-updated', ({ payload }) => {
        if (typeof payload?.bufferSecondsLeft === 'number') {
          bufferSecondsLeft.value = payload.bufferSecondsLeft
          syncBufferTimer()
        }
        void refreshOrganizeOverview(true)
        void refreshLibraryTree()
      }))
      trackUnlisten(await desktopWindow.listen<{ summary?: string; changesetId?: string }>('wikra-organize-finished', ({ payload }) => {
        void refreshOrganizeOverview(true)
        void refreshLibraryTree()
        void refreshLintReport(true)
        if (activeDoc.value) void loadDocument(activeDoc.value.path, true)
        if (payload?.summary) {
          showToast(payload.summary, [
            { label: '查看改动', onClick: () => switchWorkspace('organize') },
            ...(payload.changesetId
              ? [{ label: '撤销', onClick: () => void handleRevertChangeset(payload.changesetId!) }]
              : []),
          ])
        }
      }))
      trackUnlisten(await desktopWindow.listen('note-updated', () => {
        void refreshLibraryTree()
        void refreshOrganizeOverview(true)
        if (activeDoc.value) void loadDocument(activeDoc.value.path, true)
      }))
      trackUnlisten(await desktopWindow.listen('drawer-badge-updated', () => {
        void refreshOrganizeOverview(true)
        void refreshLibraryTree()
        if (activeDoc.value) void loadDocument(activeDoc.value.path, true)
      }))
      trackUnlisten(await desktopWindow.onResized(() => { void syncMaximized() }))
      void syncMaximized()
      void updateHotkey()
    }

    const opened = await api.resumeWorkspace()
    if (opened) {
      applyWorkspace(opened)
    }
  } catch (err) {
    showToast(parseAppError(err).safeMessage)
  } finally {
    opening.value = false
  }
})

onUnmounted(() => {
  disposed = true
  window.removeEventListener('keydown', handleGlobalKeydown)
  window.removeEventListener('resize', handleResize)
  window.removeEventListener('mousedown', handleClickOutsideSearch)
  window.clearTimeout(searchDebounce)
  if (bufferTickTimer) window.clearInterval(bufferTickTimer)
  for (const fn of unlisteners.splice(0)) fn()
})
</script>

<template>
  <div class="app-shell">
    <!-- Frameless Top Bar with Dual Explicit Drag Hotzones -->
    <header class="topbar">
      <div class="brand-mark">
        <div class="brand-glyph"><UiIcon name="bolt" size="sm" /></div>
        <span class="brand-title">Wikra</span>
        <button
          type="button"
          class="vault-chip"
          :title="workspace?.root || '点击新建或打开本地知识库'"
          @click="onboardingOpen = true"
        >{{ vaultName }}</button>
      </div>

      <div
        class="drag-spacer"
        title="按住此处拖动桌面窗口"
        @mousedown.left="desktopWindow && desktopAction('drag')"
        @dblclick="desktopWindow && desktopAction('maximize')"
      />

      <div class="global-search-wrap">
        <div class="global-search" role="search">
          <UiIcon name="search" size="sm" />
          <input
            ref="searchInputEl"
            v-model="searchQuery"
            type="text"
            placeholder="搜索知识页、原文或直接提问…"
            aria-label="全局搜索"
            :disabled="!workspace"
            @focus="searchQuery.trim() && (searchOpen = true)"
            @keydown.esc.stop="searchOpen = false; searchInputEl?.blur()"
            @keydown.enter.prevent="askFromGlobalSearch"
          >
          <kbd>Ctrl+K</kbd>
        </div>

        <div v-if="searchOpen && searchQuery.trim()" class="search-popover" role="listbox" aria-label="全局搜索结果">
          <div v-if="searchBusy && !searchHits.length" class="hint" style="padding:6px 8px;">正在检索知识页、原文与引文…</div>
          <button
            v-for="(h, i) in searchHits"
            :key="`${h.targetPath}-${h.citationId || i}`"
            type="button"
            class="search-hit"
            @click="selectSearchHit(h)"
          >
            <div style="display:flex;align-items:center;gap:6px;">
              <span class="pill" :class="{ accent: h.kind === 'citation' }">{{ h.kindLabel }}</span>
              <strong style="font-size:12.5px;">{{ h.title }}</strong>
            </div>
            <span style="font-size:11.5px;color:var(--muted);overflow:hidden;text-overflow:ellipsis;white-space:nowrap;">
              {{ h.subtitle }}
            </span>
          </button>
          <div v-if="!searchBusy && !searchHits.length" class="hint" style="padding:6px 8px;">
            未找到本地匹配内容
          </div>
          <button
            type="button"
            class="search-hit"
            style="border-top:1px solid var(--line-subtle);margin-top:2px;padding-top:8px;"
            @click="askFromGlobalSearch"
          >
            <strong style="font-size:12.5px;color:var(--accent);">直接问 AI：「{{ searchQuery.trim() }}」 →</strong>
          </button>
        </div>
      </div>

      <div
        class="drag-spacer"
        title="按住此处拖动桌面窗口"
        @mousedown.left="desktopWindow && desktopAction('drag')"
        @dblclick="desktopWindow && desktopAction('maximize')"
      />

      <div class="topbar-actions">
        <button
          type="button"
          class="btn primary sm"
          :disabled="opening"
          :title="`随时新建笔记 (${activeHotkeyLabel})`"
          @click="openQuickCapture"
        >
          <UiIcon name="plus" size="sm" />
          <span>随手记</span>
          <kbd class="hud-kbd-inline">{{ activeHotkeyLabel }}</kbd>
        </button>

        <button
          v-if="workspace && openConflicts.length > 0"
          type="button"
          class="btn sm warn-outline"
          title="有观点冲突等待你确认"
          @click="openConflictReview()"
        >
          <UiIcon name="conflict" size="sm" />
          <span>有冲突 ({{ openConflicts.length }})</span>
        </button>

        <button
          v-if="workspace"
          type="button"
          class="btn sm"
          :class="showRightPane ? 'active-toggle' : 'quiet'"
          :aria-pressed="showRightPane"
          title="显示/隐藏右侧栏 (Ctrl+J)"
          @click="setRightOpen(!isRightOpen)"
        >
          <UiIcon name="sidebar" size="sm" />
          <span>侧栏</span>
        </button>

        <div v-if="desktopWindow" class="win-controls" aria-label="窗口控制">
          <button type="button" class="win-btn" title="最小化到任务栏" aria-label="最小化" @click="desktopAction('minimize')">
            <UiIcon name="minimize" size="sm" />
          </button>
          <button
            type="button"
            class="win-btn"
            :title="isMaximized ? '还原窗口' : '最大化窗口'"
            :aria-label="isMaximized ? '还原' : '最大化'"
            @click="desktopAction('maximize')"
          >
            <UiIcon :name="isMaximized ? 'restore' : 'maximize'" size="sm" />
          </button>
          <button type="button" class="win-btn close" title="隐藏到系统托盘" aria-label="隐藏到系统托盘" @click="desktopAction('close')">
            <UiIcon name="close" size="sm" />
          </button>
        </div>
      </div>
    </header>

    <!-- Main Workspace Body -->
    <div class="workspace-body">
      <nav class="nav-rail" aria-label="主导航">
        <div class="nav-group">
          <button
            type="button"
            class="nav-item"
            :class="{ active: activeWorkspace === 'docs' && docLeftMode === 'library' }"
            title="知识库（浏览知识页与全部原文）"
            :disabled="!workspace"
            @click="switchWorkspace('library')"
          >
            <UiIcon name="book" size="lg" />
            <span class="nav-label">知识库</span>
          </button>

          <button
            type="button"
            class="nav-item"
            :class="{ active: activeWorkspace === 'docs' && docLeftMode === 'inbox' }"
            title="收件箱（阅读收藏的文章网页并提炼重点）"
            :disabled="!workspace"
            @click="switchWorkspace('inbox')"
          >
            <UiIcon name="inbox" size="lg" />
            <span class="nav-label">收件箱</span>
            <span v-if="inboxPendingCount > 0" class="nav-badge">{{ inboxPendingCount }}</span>
          </button>

          <button
            type="button"
            class="nav-item"
            :class="{ active: activeWorkspace === 'organize' }"
            title="整理（查看 AI 整理记录、体检与需要你确认的冲突）"
            :disabled="!workspace"
            @click="switchWorkspace('organize')"
          >
            <UiIcon name="sparkles" size="lg" />
            <span class="nav-label">整理</span>
            <span v-if="organizeAttentionCount > 0" class="nav-badge warn">{{ organizeAttentionCount }}</span>
          </button>

          <button
            type="button"
            class="nav-item"
            :class="{ active: activeWorkspace === 'graph' }"
            title="知识图谱 (Ctrl+G)"
            :disabled="!workspace"
            @click="switchWorkspace('graph')"
          >
            <UiIcon name="graph" size="lg" />
            <span class="nav-label">图谱</span>
          </button>
        </div>

        <div class="nav-group">
          <button
            type="button"
            class="nav-item"
            :title="isDark ? '切换为浅色主题（Paper Daylight）' : '切换为深色主题（Cognitive Slate）'"
            @click="toggleTheme"
          >
            <UiIcon :name="isDark ? 'sun' : 'moon'" size="lg" />
            <span class="nav-label">{{ isDark ? '浅色' : '深色' }}</span>
          </button>
          <button
            type="button"
            class="nav-item"
            title="设置（外观、AI 模型、OpenViking 与目录）"
            @click="settingsOpen = true"
          >
            <UiIcon name="settings" size="lg" />
            <span class="nav-label">设置</span>
          </button>
        </div>
      </nav>

      <!-- First-Run Welcome Screen when no workspace is open -->
      <main v-if="opening" class="center-stage">
        <div class="empty-stage">
          <p>正在恢复上次打开的知识库…</p>
        </div>
      </main>

      <main v-else-if="!workspace" class="center-stage">
        <VaultOnboardingModal
          :open="true"
          inline
          :desktop="Boolean(desktopWindow)"
          :can-dismiss="false"
          @workspace-opened="applyWorkspace"
        />
      </main>

      <!-- Active Workspace 3-Pane Layout -->
      <template v-else>
        <LibrarySidebar
          v-if="showLeftPane && docLeftMode === 'library'"
          v-model:filter="libraryFilter"
          :tree="libraryTree"
          :vault-root="workspace.root"
          :raw-dir="effectiveRawDir"
          :loading="libraryLoading"
          :active-tab-id="activeDocTabId"
          :creating="creatingPage"
          :error="libraryError"
          @open-wiki="openWikiPage"
          @open-raw="openRawFile"
          @create-page="handleCreateWikiPage"
        />

        <InboxSidebar
          v-else-if="showLeftPane && docLeftMode === 'inbox'"
          v-model:filter="inboxFilter"
          :items="inboxItems"
          :loading="inboxLoading"
          :active-tab-id="activeDocTabId"
          @open-inbox="openInboxItem"
        />

        <main class="center-stage">
          <div v-if="activeWorkspace === 'docs' && openDocTabs.length > 0" class="tab-strip" role="tablist" aria-label="已打开文档标签">
            <div
              v-for="t in openDocTabs"
              :key="t.id"
              class="doc-tab"
              :class="{ active: activeDocTabId === t.id }"
              role="tab"
              :aria-selected="activeDocTabId === t.id"
              :title="t.label"
              @click="openDocTab(t)"
            >
              <span class="tab-label">{{ t.label }}</span>
              <button
                type="button"
                class="tab-close"
                aria-label="关闭标签"
                @click.stop="closeDocTab(t.id)"
              >&times;</button>
            </div>
          </div>

          <OrganizeView
            v-if="activeWorkspace === 'organize'"
            :session="workspace.session"
            :overview="organizeOverview"
            :loading="organizeLoading"
            :error="organizeError"
            :buffer-seconds-left="bufferSecondsLeft"
            :reverting-id="revertingId"
            :lint-report="lintReport"
            :lint-loading="lintLoading"
            :lint-error="lintError"
            :filter-page-path="lintFilterPagePath"
            :active-wiki-path="activePagePath"
            :wiki-pages="libraryTree.wikiPages"
            :rollback-feedback-by-cs="rollbackFeedbackByCs"
            :revert-blocked-by-cs="revertBlockedByCs"
            @go-library="switchWorkspace('library')"
            @open-capture="openQuickCapture"
            @open-conflict="openConflictReview"
            @open-wiki="openWikiPage"
            @open-buffer-queue="bufferModalOpen = true"
            @flush-buffer="handleFlushBufferNow"
            @revert-changeset="handleRevertChangeset"
            @refresh-lint="refreshLintReport(false)"
            @clear-lint-filter="lintFilterPagePath = null"
            @lint-updated="handleLintUpdated"
            @review-decided="handleReviewDecided"
            @escalated-conflict="handleEscalatedConflict"
            @toast="showToast"
          />

          <KnowledgeGraphView
            v-else-if="activeWorkspace === 'graph'"
            :graph="knowledgeGraph"
            :loading="graphLoading"
            :error="graphError"
            :include-meta-pages="includeMetaPages"
            @update:include-meta-pages="includeMetaPages = $event; refreshKnowledgeGraph()"
            @open-wiki="openWikiPage"
            @dismiss-gap="handleDismissGraphGap"
            @ask-gap="handleAskGraphGap"
          />

          <ConflictReviewView
            v-else-if="currentViewMode === 'review'"
            :conflict="activeConflict"
            :wiki-pages="libraryTree.wikiPages"
            :resolving="resolvingConflict"
            :error="conflictError"
            @open-wiki="openWikiPage"
            @resolve="handleResolveConflict"
          />

          <InboxReaderView
            v-else-if="currentViewMode === 'inbox'"
            v-model:read-mode="inboxReadMode"
            :item="activeInbox"
            :loading="inboxDetailLoading"
            :error="inboxError"
            :doc-width="settings.docWidth"
            :ingesting="ingestingInbox"
            :compiling="compilingInbox"
            :completing-stub="completingStub"
            :toggling-para-id="togglingParaId"
            @toggle-width="toggleDocWidth"
            @ingest="handleInboxIngest"
            @open-external-url="handleOpenExternalUrl"
            @toggle-paragraph="handleToggleInboxParagraph"
            @compile="handleCompileInbox"
            @complete-stub="handleCompleteStub"
          />

          <RawDocumentView
            v-else-if="currentViewMode === 'raw'"
            :session="workspace.session"
            :doc="activeDoc"
            :loading="docLoading"
            :error="docError"
            :doc-width="settings.docWidth"
            :wiki-title-map="wikiTitleMap"
            :organizing="reorganizingRaw"
            @toggle-width="toggleDocWidth"
            @open-external-url="handleOpenExternalUrl"
            @open-wiki="openWikiPage"
            @reorganize="handleReorganizeRaw"
          />

          <WikiDocumentView
            v-else
            v-model:editor-mode="editorMode"
            :session="workspace.session"
            :doc="activeDoc"
            :loading="docLoading"
            :error="docError"
            :doc-width="settings.docWidth"
            :recall-curtain-page-path="recallCurtainPagePath"
            :evidence-citation="currentEvidenceCitation"
            :evidence-raw-doc="evidenceRawDoc"
            :evidence-loading="evidenceLoading"
            :external-changed="Boolean(activeDoc && externalChangedPaths[activeDoc.path])"
            :wikilink-candidates="wikilinkCandidates"
            :page-lint-findings="activePageLintFindings"
            @toggle-width="toggleDocWidth"
            @toggle-split-origin="toggleSplitOrigin"
            @open-evidence="openEvidenceSplit"
            @close-evidence="closeEvidenceSplit"
            @open-raw="openRawFile"
            @open-wikilink="handleOpenWikilink"
            @open-wiki-path="openWikiPage"
            @open-page-lint="handleOpenPageLint"
            @open-conflict="openConflictReview"
            @start-teachback="startTeachback"
            @exit-curtain="exitTeachback"
            @open-obsidian="handleOpenObsidian"
            @open-external-url="handleOpenExternalUrl"
            @document-saved="handleDocumentSaved"
            @reload-doc="loadDocument($event, false)"
            @dismiss-external="activeDoc && (externalChangedPaths = { ...externalChangedPaths, [activeDoc.path]: false })"
            @toggle-task="handleToggleTask"
            @toast="showToast"
          />
        </main>

        <RightCompanionPane
          v-if="showRightPane"
          v-model:active-tab="rightTab"
          :session="workspace.session"
          :mode="currentViewMode"
          :active-doc="activeDoc"
          :active-inbox="activeInbox"
          :wiki-title-map="wikiTitleMap"
          :raw-dir="effectiveRawDir"
          :raw-files="libraryTree.rawFiles"
          :changesets="organizeOverview?.changesets || []"
          :chat-mode="chatMode"
          :teachback-page-path="recallCurtainPagePath"
          :teachback-page-title="teachbackPageTitle"
          :pending-ask-prompt="pendingAskPrompt"
          @open-evidence="openEvidenceSplit"
          @open-wiki="openWikiPage"
          @open-raw="openRawFile"
          @go-organize="switchWorkspace('organize')"
          @revert-changeset="handleRevertChangeset"
          @start-teachback="startTeachback"
          @exit-teachback="exitTeachback"
          @understanding-appended="handleUnderstandingAppended"
          @answer-saved="handleAnswerSaved"
          @clear-pending-ask="pendingAskPrompt = null"
          @toast="showToast"
        />
      </template>
    </div>

    <!-- Calm Minimal Bottom Status Bar -->
    <footer class="statusbar">
      <div class="status-group">
        <span v-if="workspace">
          本地库：{{ vaultName }}（{{ libraryTree.wikiPages.length }} 篇知识 · {{ libraryTree.rawFiles.length }} 篇原文）
        </span>
        <span v-else>未打开本地知识库</span>
      </div>

      <div class="status-group">
        <span v-if="bufferSecondsLeft > 0 || (organizeOverview?.bufferQueue.length || 0) > 0" class="status-slot-buffer">
          <span>
            待整理（{{ organizeOverview?.bufferQueue.length || 1 }} 条）
            <template v-if="bufferSecondsLeft > 0">
              · <strong class="mono">{{ formatCountdown(bufferSecondsLeft) }}</strong>
            </template>
          </span>
          <button type="button" class="btn quiet xs" @click="bufferModalOpen = true">查看/撤回</button>
          <button type="button" class="btn quiet xs" style="color:var(--accent);" @click="handleFlushBufferNow">立即整理</button>
        </span>

        <span>
          <span class="status-dot" :class="{ off: !modelConfig?.enabled }" />
          AI：{{ modelStatusLabel }}
        </span>
        <span>
          <span class="status-dot" :class="{ off: !memoryConfig?.enabled }" />
          记忆：{{ memoryStatusLabel }}
        </span>
      </div>
    </footer>

    <!-- Modals -->
    <SettingsModal
      :open="settingsOpen"
      :desktop="Boolean(desktopWindow)"
      :session="workspace?.session || ''"
      :vault-root="workspace?.root || ''"
      :raw-dir="effectiveRawDir"
      :capture-dir="workspace?.captureDir || ''"
      :settings="settings"
      :hotkey="hotkey"
      :hotkey-busy="hotkeyBusy"
      :hotkey-error="hotkeyError"
      @close="settingsOpen = false"
      @settings-saved="handleSettingsSaved"
      @integrations-updated="(cfg: IntegrationsConfig) => { modelConfig = cfg.model; memoryConfig = cfg.memory }"
      @workspace-dirs-updated="handleWorkspaceDirsUpdated"
      @hotkey-request="updateHotkey"
      @open-onboarding="settingsOpen = false; onboardingOpen = true"
      @index-rebuilt="refreshLibraryTree()"
    />

    <VaultOnboardingModal
      :open="onboardingOpen"
      :desktop="Boolean(desktopWindow)"
      :can-dismiss="Boolean(workspace)"
      :current-root="workspace?.root"
      :current-capture-dir="workspace?.captureDir"
      @close="onboardingOpen = false"
      @workspace-opened="applyWorkspace"
    />

    <BufferQueueModal
      :open="bufferModalOpen"
      :items="organizeOverview?.bufferQueue || []"
      :seconds-left="bufferSecondsLeft"
      :busy="bufferBusy"
      @close="bufferModalOpen = false"
      @cancel-item="handleCancelBufferItem"
      @flush-now="handleFlushBufferNow"
    />

    <!-- Top-Center Toast Region -->
    <div class="toast-region" aria-live="polite">
      <div v-for="t in toasts" :key="t.id" class="toast">
        <span>{{ t.message }}</span>
        <button
          v-for="(act, idx) in t.actions || []"
          :key="idx"
          type="button"
          class="btn quiet xs"
          style="color:var(--accent);"
          @click="runToastAction(t.id, act.onClick)"
        >{{ act.label }}</button>
      </div>
    </div>
  </div>
</template>
