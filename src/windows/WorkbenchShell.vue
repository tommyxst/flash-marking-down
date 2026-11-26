<script setup lang="ts">
import { isTauri } from '@tauri-apps/api/core'
import { getCurrentWindow } from '@tauri-apps/api/window'
import { computed, defineAsyncComponent, nextTick, onMounted, onUnmounted, ref, shallowRef, watch } from 'vue'
import { api } from '../api/client'
import { parseAppError, type UiError } from '../api/errors'
import ErrorNotice from '../components/ErrorNotice.vue'
import RecordDetail from '../components/RecordDetail.vue'
import RecordTimeline from '../components/RecordTimeline.vue'
import WorkspaceSettingsDialog from '../components/WorkspaceSettingsDialog.vue'
import UiIcon from '../UiIcon.vue'
import type {
  CitationItem,
  DocumentMapping,
  DrawerCounts,
  DrawerTab,
  HotkeyStatus,
  IndexFeedback,
  IntegrationsConfig,
  LeftTab,
  MemoryConfig,
  MemoryPreview,
  MemorySource,
  ModelConfig,
  NoteDetail,
  NoteSummary,
  OpenedWorkspace,
  RecordEntry,
  RecoveryErrorDto,
  TaskEntry,
  TaskList,
  WikiDocument,
} from '../m2'

const M2Answers = defineAsyncComponent(() => import('../M2Answers.vue'))
const M2Workbench = defineAsyncComponent(() => import('../M2Workbench.vue'))

const desktopWindow = isTauri() ? getCurrentWindow() : undefined
const workspace = ref<{ session: string; root: string; captureDir: string }>()
const opening = ref(true)
const workspaceError = ref<UiError | null>(null)
const workspaceFormOpen = ref(false)
const recoveredPaths = ref<string[]>([])
const recoveryErrors = ref<RecoveryErrorDto[]>([])
const windowError = ref<UiError | null>(null)
const isMaximized = ref(false)

const settingsDialog = ref<InstanceType<typeof WorkspaceSettingsDialog>>()
const recordDetail = ref<InstanceType<typeof RecordDetail>>()
const searchInput = ref<HTMLInputElement>()

const modelConfig = ref<ModelConfig | null>(null)
const memoryConfig = ref<MemoryConfig | null>(null)
const hotkey = ref<HotkeyStatus>()
const hotkeyBusy = ref(false)
const hotkeyError = ref<UiError | null>(null)

const leftTab = ref<LeftTab>('notes')
const chatOpen = ref(true)
const drawerOpen = ref(false)
const drawerTab = ref<DrawerTab>('conflicts')
const drawerCounts = ref<DrawerCounts>({ openConflicts: 0, pendingWiki: 0 })
const wikiTargets = ref<DocumentMapping[]>([])
const workspaceFiles = ref<string[]>([])
const prefillConflict = ref<{ topic: string; incomingPath: string; incomingText: string } | null>(null)
const wikiReviewId = ref<string>()

const wikiDocuments = ref<WikiDocument[]>([])
const wikiLoading = ref(false)
const wikiError = ref<UiError | null>(null)
const wikiCreateBusy = ref(false)
const wikiCreateError = ref<UiError | null>(null)
const wikiTargetBusy = ref(false)
const wikiTargetError = ref<{ path: string; error: UiError }>()

const query = ref('')
const datePreset = ref<'all' | 'today' | '7d' | 'custom'>('all')
const fromDate = ref('')
const toDate = ref('')

const notes = ref<NoteSummary[]>([])
const notesLoading = ref(false)
const notesError = ref<UiError | null>(null)
const selectedLocator = ref('')
const activeDetail = ref<NoteDetail>()
const detailLoading = ref(false)
const detailError = ref<UiError | null>(null)
const organizing = ref(false)
const memoryPreview = shallowRef<MemoryPreview>()

const searchPage = ref<{ records: RecordEntry[]; hasMore: boolean; skipped: number }>()
const searchLoading = ref(false)
const searchError = ref<{ error: UiError; append: boolean }>()
const indexFeedback = ref<IndexFeedback>()

const taskList = ref<TaskList>()
const tasksLoading = ref(false)
const tasksError = ref<UiError | null>(null)
const taskBusyId = ref('')

let searchRequest = 0
let searchTimer: number | undefined
let detailRequest = 0
let wikiRequest = 0
let drawerRequest = 0
const unlisteners: (() => void)[] = []
let disposed = false

const folderLabel = computed(() => {
  const parts = workspace.value?.root.split(/[\\/]+/).filter(Boolean)
  return parts?.at(-1) ?? ''
})
const activeHotkeyLabel = computed(() => {
  const status = hotkey.value
  if (status?.active) return status.choices.find((choice) => choice.id === status.id)?.label
  return undefined
})
const modelReady = computed(() => Boolean(modelConfig.value?.ready))
const keyword = computed(() => query.value.trim())
const searchActive = computed(() => Boolean(keyword.value || fromDate.value || toDate.value))
const invalidDateRange = computed(() => Boolean(fromDate.value && toDate.value && fromDate.value > toDate.value))
const totalDrawerCount = computed(() => drawerCounts.value.openConflicts + drawerCounts.value.pendingWiki)

function localIsoDate(daysOffset = 0) {
  const d = new Date()
  d.setDate(d.getDate() + daysOffset)
  const y = d.getFullYear()
  const m = String(d.getMonth() + 1).padStart(2, '0')
  const day = String(d.getDate()).padStart(2, '0')
  return `${y}-${m}-${day}`
}

function applyDatePreset(preset: 'all' | 'today' | '7d' | 'custom') {
  datePreset.value = preset
  if (preset === 'all') {
    fromDate.value = toDate.value = ''
  } else if (preset === 'today') {
    const today = localIsoDate(0)
    fromDate.value = today
    toDate.value = today
  } else if (preset === '7d') {
    fromDate.value = localIsoDate(-6)
    toDate.value = localIsoDate(0)
  }
}

function trackUnlisten(unlisten: () => void) {
  if (disposed) unlisten()
  else unlisteners.push(unlisten)
}

function applyWorkspace(opened: OpenedWorkspace) {
  workspace.value = { session: opened.session, root: opened.root, captureDir: opened.captureDir }
  recoveredPaths.value = opened.recovered
  recoveryErrors.value = opened.recoveryErrors
  workspaceError.value = null
  workspaceFormOpen.value = false
  modelConfig.value = null
  memoryConfig.value = null
  clearSearch()
  detailRequest++
  selectedLocator.value = ''
  activeDetail.value = undefined
  detailLoading.value = false
  detailError.value = null
  memoryPreview.value = undefined
  notes.value = []
  wikiDocuments.value = []
  wikiTargets.value = []
  workspaceFiles.value = []
  wikiCreateBusy.value = false
  wikiCreateError.value = null
  wikiTargetBusy.value = false
  wikiTargetError.value = undefined
  wikiReviewId.value = undefined
  taskList.value = undefined
  indexFeedback.value = undefined
  void refreshAllWorkspaceData(opened.session)
}

async function refreshAllWorkspaceData(session: string) {
  await Promise.all([
    refreshIntegrationsConfig(session),
    refreshNotes(),
    refreshTasks(),
    refreshWikiData(),
  ])
}

async function refreshDrawerData() {
  const session = workspace.value?.session
  if (!session) return
  const req = ++drawerRequest
  const [counts, targets, files] = await Promise.allSettled([
    api.drawerCounts(session),
    api.wikiTargets(session),
    api.listWorkspaceMarkdownFiles(session),
  ])
  if (workspace.value?.session !== session || req !== drawerRequest) return
  if (counts.status === 'fulfilled') drawerCounts.value = counts.value
  if (targets.status === 'fulfilled') wikiTargets.value = targets.value
  if (files.status === 'fulfilled') workspaceFiles.value = files.value
}

async function refreshWikiDocuments() {
  const session = workspace.value?.session
  if (!session) return
  const req = ++wikiRequest
  const current = () => req === wikiRequest && workspace.value?.session === session
  wikiLoading.value = true
  wikiError.value = null
  try {
    const documents = await api.listWikiDocuments(session)
    if (current()) wikiDocuments.value = documents
  } catch (error) {
    if (current()) wikiError.value = parseAppError(error)
  } finally {
    if (current()) wikiLoading.value = false
  }
}

function refreshWikiData() {
  return Promise.all([refreshWikiDocuments(), refreshDrawerData()])
}

function refreshAfterDrawerChange() {
  void refreshWikiData()
  void refreshNotes(false)
  if (activeDetail.value) void reloadCurrentDetail()
}

onMounted(async () => {
  window.addEventListener('keydown', handleWindowKey)
  try {
    if (desktopWindow) {
      trackUnlisten(await desktopWindow.listen('capture-closed', () => {
        void refreshNotes()
        void refreshTasks()
      }))
      trackUnlisten(await desktopWindow.listen<{ noteId: string }>('note-updated', ({ payload }) => {
        void refreshNotes(false)
        void refreshTasks()
        if (activeDetail.value && (activeDetail.value.id === payload.noteId || activeDetail.value.locator === selectedLocator.value)) {
          void loadDetail(activeDetail.value.locator)
        }
      }))
      trackUnlisten(await desktopWindow.listen('drawer-badge-updated', refreshAfterDrawerChange))
      trackUnlisten(await desktopWindow.onResized(() => { void syncMaximized() }))
      void syncMaximized()
      void updateHotkey()
    }
    const opened = await api.resumeWorkspace()
    if (opened) applyWorkspace(opened)
  } catch (error) {
    workspaceError.value = parseAppError(error)
  } finally {
    opening.value = false
    if (!workspace.value) workspaceFormOpen.value = true
  }
})

onUnmounted(() => {
  disposed = true
  window.removeEventListener('keydown', handleWindowKey)
  window.clearTimeout(searchTimer)
  for (const unlisten of unlisteners.splice(0)) unlisten()
})

watch(
  () => ({ session: workspace.value?.session, keyword: keyword.value, fromDate: fromDate.value, toDate: toDate.value }),
  ({ session }, _, onCleanup) => {
    searchPage.value = undefined
    searchError.value = undefined
    searchLoading.value = Boolean(session && searchActive.value && !invalidDateRange.value)
    if (!searchLoading.value) return
    leftTab.value = 'notes'
    searchTimer = window.setTimeout(() => { void refreshSearch() }, 200)
    onCleanup(() => window.clearTimeout(searchTimer))
  },
)

async function syncMaximized() {
  if (!desktopWindow) return
  try {
    isMaximized.value = await api.isMainMaximized()
  } catch {
    // Ignore transient resize query errors.
  }
}

async function desktopAction(action: 'drag' | 'minimize' | 'maximize' | 'close') {
  if (!desktopWindow) return
  if (workspaceFormOpen.value) settingsDialog.value?.setWindowError(null)
  else windowError.value = null
  try {
    await api.desktopWindowAction(action)
    if (action === 'maximize') void syncMaximized()
  } catch (error) {
    const parsed = parseAppError(error)
    if (workspaceFormOpen.value) settingsDialog.value?.setWindowError(parsed)
    else windowError.value = parsed
  }
}

async function updateHotkey(choice?: string) {
  if (hotkeyBusy.value) return
  hotkeyBusy.value = true
  hotkeyError.value = null
  try {
    const result = choice === undefined ? await api.getHotkey() : await api.setHotkey(choice)
    hotkey.value = result
    hotkeyError.value = result.error ? { safeMessage: result.error } : null
  } catch (error) {
    hotkeyError.value = parseAppError(error)
  } finally {
    hotkeyBusy.value = false
  }
}

async function refreshModelConfig(session: string) {
  try {
    const config = await api.getModelConfig(session)
    if (workspace.value?.session === session) modelConfig.value = config
  } catch {
    // Errors are shown in settings dialog.
  }
}

function applyIntegrationsConfig(config: IntegrationsConfig) {
  modelConfig.value = config.model
  memoryConfig.value = config.memory
}

async function refreshIntegrationsConfig(session: string) {
  try {
    const config = await api.getIntegrationsConfig(session)
    if (workspace.value?.session === session) applyIntegrationsConfig(config)
  } catch {
    await refreshModelConfig(session)
  }
}

function showSettings() {
  workspaceFormOpen.value = true
}

function showHotkeySettings() {
  showSettings()
  settingsDialog.value?.focusHotkey()
}

async function showCapture() {
  const session = workspace.value?.session
  if (!session) {
    showSettings()
    return
  }
  windowError.value = null
  try {
    await api.showCaptureWindow(session)
  } catch (error) {
    windowError.value = parseAppError(error)
  }
}

async function refreshNotes(refreshSelected = true) {
  const session = workspace.value?.session
  if (!session) return
  notesLoading.value = true
  notesError.value = null
  try {
    const list = await api.listNotes(session)
    if (workspace.value?.session !== session) return
    notes.value = list
    if (!selectedLocator.value && list.length) {
      selectedLocator.value = list[0].locator
      await loadDetail(selectedLocator.value)
    } else if (refreshSelected && selectedLocator.value && !detailLoading.value && !searchActive.value) {
      await loadDetail(selectedLocator.value)
    }
  } catch (error) {
    if (workspace.value?.session === session) notesError.value = parseAppError(error)
  } finally {
    if (workspace.value?.session === session) notesLoading.value = false
  }
}

async function loadDetail(locator: string) {
  const session = workspace.value?.session
  if (!session || !locator) return
  const req = ++detailRequest
  detailLoading.value = true
  detailError.value = null
  try {
    const detail = await api.getNoteDetail(session, locator)
    if (req === detailRequest && workspace.value?.session === session) {
      activeDetail.value = detail
      detailError.value = null
      return detail
    }
  } catch (error) {
    if (req === detailRequest && workspace.value?.session === session) {
      detailError.value = parseAppError(error)
    }
  } finally {
    if (req === detailRequest) detailLoading.value = false
  }
}

function reloadCurrentDetail() {
  return loadDetail(activeDetail.value?.locator ?? selectedLocator.value)
}

function onNoteSaved(updated: NoteDetail) {
  if (!updated.isCapture) {
    void refreshWikiDocuments()
    void refreshSearch()
  }
  const current = activeDetail.value
  if (current?.locator !== updated.locator) return
  if (current.contentHash !== updated.contentHash) activeDetail.value = updated
  void reloadCurrentDetail()
  void refreshNotes(false)
  void refreshTasks()
  void refreshDrawerData()
}

async function selectNote(locator: string, focusDetail: boolean) {
  const session = workspace.value?.session
  memoryPreview.value = undefined
  if (selectedLocator.value !== locator) activeDetail.value = undefined
  selectedLocator.value = locator
  const detail = await loadDetail(locator)
  if (memoryPreview.value || selectedLocator.value !== locator || workspace.value?.session !== session) return
  if (detail) selectedLocator.value = detail.locator
  if (focusDetail) recordDetail.value?.focus()
  return detail
}

async function openLocalSource(path: string) {
  leftTab.value = wikiDocuments.value.some((doc) => doc.path === path) ? 'wiki' : 'notes'
  const detail = await selectNote(path, true)
  if (!detail) return
  const captureDir = workspace.value?.captureDir
  const isCapture = detail.isCapture || (captureDir && detail.path.startsWith(`${captureDir}/`)) ||
    notes.value.some((note) => note.path === detail.path)
  leftTab.value = isCapture ? 'notes' : 'wiki'
}

async function createWikiPage(path: string) {
  const session = workspace.value?.session
  if (!session || wikiCreateBusy.value) return
  const normalizedPath = path.replaceAll('\\', '/')
  wikiCreateBusy.value = true
  wikiCreateError.value = null
  try {
    await api.createWikiTarget(session, normalizedPath)
    if (workspace.value?.session !== session) return
    await refreshWikiData()
    if (workspace.value?.session !== session) return
    leftTab.value = 'wiki'
    const detail = await selectNote(normalizedPath, true)
    await nextTick()
    if (detail && workspace.value?.session === session && activeDetail.value?.locator === detail.locator) {
      recordDetail.value?.startEditing()
    }
  } catch (error) {
    if (workspace.value?.session === session) wikiCreateError.value = parseAppError(error)
  } finally {
    if (workspace.value?.session === session) wikiCreateBusy.value = false
  }
}

async function toggleWikiTarget(path: string, mapped: boolean) {
  const session = workspace.value?.session
  if (!session || wikiTargetBusy.value) return
  wikiTargetBusy.value = true
  wikiTargetError.value = undefined
  try {
    if (mapped) await api.unmapWikiTarget(session, path)
    else await api.mapWikiTarget(session, 'wiki', path)
    if (workspace.value?.session === session) await refreshWikiData()
  } catch (error) {
    if (workspace.value?.session === session) wikiTargetError.value = { path, error: parseAppError(error) }
  } finally {
    if (workspace.value?.session === session) wikiTargetBusy.value = false
  }
}

async function openMemoryPreview({ uri, kind, title }: MemorySource) {
  const session = workspace.value?.session
  if (!session) return
  const request: MemoryPreview = { state: 'opening', uri, kind, title }
  memoryPreview.value = request
  void nextTick(() => {
    if (memoryPreview.value === request) recordDetail.value?.focus()
  })
  try {
    const item = await api.readMemoryItem(session, uri)
    if (memoryPreview.value === request) memoryPreview.value = { ...item, state: 'opened' }
  } catch (error) {
    if (memoryPreview.value === request) {
      memoryPreview.value = { ...request, state: 'failed', error: parseAppError(error) }
    }
  }
}

function openCitation(citation: CitationItem) {
  if (citation.kind === 'local') openLocalSource(citation.path)
  else void openMemoryPreview({ uri: citation.path, title: citation.title, kind: citation.kind })
}

function returnToNote() {
  memoryPreview.value = undefined
  void nextTick(() => recordDetail.value?.focus())
}

async function triggerOrganize(idOrPath: string) {
  const session = workspace.value?.session
  if (!session || organizing.value) return
  const locator = selectedLocator.value
  organizing.value = true
  detailError.value = null
  try {
    await api.triggerNoteOrganize(session, idOrPath)
    if (workspace.value?.session !== session) return
    if (selectedLocator.value === locator) void reloadCurrentDetail()
    await refreshNotes(false)
  } catch (error) {
    if (workspace.value?.session === session && selectedLocator.value === locator) {
      detailError.value = parseAppError(error)
    }
  } finally {
    organizing.value = false
  }
}

async function refreshTasks() {
  const session = workspace.value?.session
  if (!session) return
  tasksLoading.value = true
  tasksError.value = null
  try {
    const list = await api.listTasks(session)
    if (workspace.value?.session === session) taskList.value = list
  } catch (error) {
    if (workspace.value?.session === session) tasksError.value = parseAppError(error)
  } finally {
    if (workspace.value?.session === session) tasksLoading.value = false
  }
}

async function toggleTask(path: string, taskId: string, expectedLine: string, done: boolean) {
  const session = workspace.value?.session
  if (!session || taskBusyId.value) return
  taskBusyId.value = taskId
  tasksError.value = null
  try {
    await api.toggleTask(session, path, taskId, expectedLine, done)
    await refreshTasks()
    if (activeDetail.value) await loadDetail(activeDetail.value.locator)
  } catch (error) {
    tasksError.value = parseAppError(error)
    await refreshTasks()
  } finally {
    taskBusyId.value = ''
  }
}

async function mapTaskTarget(path: string) {
  const session = workspace.value?.session
  if (!session) return
  tasksError.value = null
  try {
    await api.mapWikiTarget(session, 'tasks', path)
    await Promise.all([refreshTasks(), refreshWikiData()])
  } catch (error) {
    tasksError.value = parseAppError(error)
  }
}

async function pickTaskTarget() {
  const session = workspace.value?.session
  if (!session) return
  try {
    const selected = await api.pickWorkspaceMarkdown(session, 'tasks')
    if (selected) await mapTaskTarget(selected)
  } catch (error) {
    tasksError.value = parseAppError(error)
  }
}

function clearSearch() {
  query.value = ''
  applyDatePreset('all')
}

function localMidnight(date: string, nextDay = false) {
  if (nextDay) {
    const following = new Date(`${date}T00:00:00Z`)
    following.setUTCDate(following.getUTCDate() + 1)
    date = following.toISOString().split('T', 1)[0]
  }
  return new Date(`${date}T00:00:00`).toISOString()
}

async function refreshSearch(append = false) {
  const session = workspace.value?.session
  if (!session || !searchActive.value || invalidDateRange.value) return
  const previous = append ? searchPage.value : undefined
  if (append && (!previous?.hasMore || searchLoading.value)) return
  window.clearTimeout(searchTimer)
  const req = ++searchRequest
  searchLoading.value = true
  searchError.value = undefined
  try {
    const page = await api.searchRecords({
      session,
      query: keyword.value,
      fromUtc: fromDate.value ? localMidnight(fromDate.value) : undefined,
      untilUtc: toDate.value ? localMidnight(toDate.value, true) : undefined,
      offset: previous?.records.length ?? 0,
    })
    if (req === searchRequest && workspace.value?.session === session) {
      const records: RecordEntry[] = page.hits.map((hit) => ({
        path: hit.path,
        locator: hit.path,
        capturedAt: hit.capturedAt,
        text: hit.preview,
        contentHash: hit.contentHash,
        titleText: hit.titleText,
        isCapture: hit.isCapture,
      }))
      searchPage.value = {
        records: previous ? previous.records.concat(records) : records,
        hasMore: page.hasMore,
        skipped: page.skipped,
      }
    }
  } catch (error) {
    if (req === searchRequest) searchError.value = { error: parseAppError(error), append }
  } finally {
    if (req === searchRequest) searchLoading.value = false
  }
}

async function rebuildSearchIndex() {
  const session = workspace.value?.session
  if (!session || indexFeedback.value?.state === 'building') return
  indexFeedback.value = { state: 'building' }
  try {
    const { indexed, skipped } = await api.rebuildSearchIndex(session)
    if (workspace.value?.session === session) {
      indexFeedback.value = { state: 'ready', indexed, skipped }
      void refreshSearch()
      void refreshWikiDocuments()
    }
  } catch (error) {
    if (workspace.value?.session === session) {
      indexFeedback.value = { state: 'failed', error: parseAppError(error) }
    }
  }
}

function openDrawer(tab: DrawerTab, suggestionId?: string) {
  wikiReviewId.value = suggestionId
  drawerTab.value = tab
  drawerOpen.value = true
}

function openConflictFromNote(detail: NoteDetail) {
  prefillConflict.value = {
    topic: detail.title || '观点分歧',
    incomingPath: detail.locator,
    incomingText: detail.body.slice(0, 480),
  }
  openDrawer('conflicts')
}

async function openExternal(path: string) {
  const session = workspace.value?.session
  if (!session) return
  try {
    await api.openWorkspaceFile(session, path, 'open')
  } catch (error) {
    windowError.value = parseAppError(error)
  }
}

function handleWindowKey(event: KeyboardEvent) {
  if (event.isComposing || event.keyCode === 229) return
  if (!workspaceFormOpen.value && event.ctrlKey && event.key.toLowerCase() === 'k') {
    event.preventDefault()
    leftTab.value = 'notes'
    void nextTick(() => searchInput.value?.focus())
  }
}
</script>

<template>
  <div class="shell-root">
    <header class="topbar">
      <div class="brand-compact" @mousedown.left="desktopWindow && desktopAction('drag')" @dblclick="desktopWindow && desktopAction('maximize')">
        <span class="brand-icon"><UiIcon name="terminal" /></span>
        <div class="brand-text">
          <span class="brand-name">Flash Marking Down</span>
          <small v-if="workspace" :title="workspace.root">{{ folderLabel }}</small>
        </div>
      </div>

      <label class="search-box">
        <UiIcon name="search" />
        <span class="visually-hidden">搜索工作区 Markdown</span>
        <input
          ref="searchInput"
          v-model="query"
          type="search"
          placeholder="搜索笔记与 Markdown…"
          :disabled="!workspace || opening"
          aria-keyshortcuts="Control+k"
        >
        <kbd aria-hidden="true">Ctrl+K</kbd>
      </label>

      <div v-if="workspace" class="date-pills" role="group" aria-label="快捷日期筛选">
        <button type="button" class="quiet compact-btn" :class="{ active: datePreset === 'all' }" @click="applyDatePreset('all')">全部</button>
        <button type="button" class="quiet compact-btn" :class="{ active: datePreset === 'today' }" @click="applyDatePreset('today')">今日</button>
        <button type="button" class="quiet compact-btn" :class="{ active: datePreset === '7d' }" @click="applyDatePreset('7d')">近 7 天</button>
        <template v-if="datePreset === 'custom'">
          <input v-model="fromDate" type="date" aria-label="开始日期" class="compact-date">
          <input v-model="toDate" type="date" aria-label="结束日期" class="compact-date">
        </template>
        <button type="button" class="quiet compact-btn" :class="{ active: datePreset === 'custom' }" @click="applyDatePreset('custom')">自选</button>
      </div>

      <div class="topbar-actions">
        <button type="button" class="primary compact-btn" :disabled="opening" @click="showCapture">
          <UiIcon name="bolt" />随手记
          <kbd v-if="activeHotkeyLabel" aria-hidden="true">{{ activeHotkeyLabel }}</kbd>
        </button>

        <button
          v-if="workspace"
          type="button"
          class="compact-btn"
          :class="totalDrawerCount ? 'badge-warn-btn' : 'quiet'"
          @click="openDrawer(drawerCounts.openConflicts || !drawerCounts.pendingWiki ? 'conflicts' : 'wiki')"
        >
          <UiIcon name="conflict" />冲突/变更 ({{ totalDrawerCount }})
        </button>

        <button
          v-if="workspace"
          type="button"
          class="quiet compact-btn"
          :aria-pressed="chatOpen"
          title="展开或收起右侧记忆对话"
          @click="chatOpen = !chatOpen"
        >
          <UiIcon name="sidebar" />对话
        </button>

        <button type="button" class="quiet compact-btn" :disabled="opening" @click="showSettings">
          <UiIcon name="settings" />设置
        </button>
      </div>

      <div v-if="desktopWindow" class="window-drag" aria-hidden="true" @mousedown.left="desktopAction('drag')" @dblclick="desktopAction('maximize')" />
      <div v-if="desktopWindow" class="window-controls" role="group" aria-label="工作台窗口">
        <button type="button" aria-label="最小化" @click="desktopAction('minimize')"><UiIcon name="minimize" /></button>
        <button type="button" :aria-label="isMaximized ? '还原' : '最大化'" @click="desktopAction('maximize')"><UiIcon :name="isMaximized ? 'restore' : 'maximize'" /></button>
        <button type="button" aria-label="关闭窗口" @click="desktopAction('close')"><UiIcon name="close" /></button>
      </div>
    </header>

    <div v-if="windowError || hotkeyError || recoveredPaths.length || recoveryErrors.length" class="banner-strip">
      <ErrorNotice :error="windowError" />
      <div v-if="hotkeyError" class="notice error hotkey-notice">
        <span role="alert">{{ hotkeyError.safeMessage }}。仍可从工作台或托盘打开随手记。</span>
        <button type="button" class="quiet compact-btn" @click="showHotkeySettings">更改快捷键</button>
      </div>
      <p v-if="recoveredPaths.length" class="notice success compact-notice" role="status">已恢复 {{ recoveredPaths.length }} 条随手记</p>
      <p v-for="item in recoveryErrors" :key="item.requestId" class="notice error compact-notice" role="alert">恢复遇到问题：{{ item.message }}</p>
    </div>

    <main class="three-pane-grid" :class="{ 'chat-collapsed': !chatOpen || !workspace }">
      <RecordTimeline
        :key="workspace?.session"
        v-model:left-tab="leftTab"
        :has-workspace="Boolean(workspace)"
        :opening="opening"
        :desktop="Boolean(desktopWindow)"
        :search-active="searchActive"
        :invalid-date-range="invalidDateRange"
        :from-date="fromDate"
        :to-date="toDate"
        :notes="notes"
        :memory-config="memoryConfig"
        :notes-loading="notesLoading"
        :notes-error="notesError"
        :wiki-documents="wikiDocuments"
        :wiki-loading="wikiLoading"
        :wiki-error="wikiError"
        :wiki-create-busy="wikiCreateBusy"
        :wiki-create-error="wikiCreateError"
        :search-page="searchPage"
        :search-loading="searchLoading"
        :search-error="searchError"
        :index-feedback="indexFeedback"
        :selected-locator="selectedLocator"
        :task-list="taskList"
        :tasks-loading="tasksLoading"
        :tasks-error="tasksError"
        :task-busy-id="taskBusyId"
        :workspace-files="workspaceFiles"
        @update:from-date="fromDate = $event"
        @update:to-date="toDate = $event"
        @clear-search="clearSearch(); searchInput?.focus()"
        @rebuild-index="rebuildSearchIndex"
        @open-settings="showSettings"
        @open-capture="showCapture"
        @retry-notes="refreshNotes"
        @retry-search="refreshSearch($event)"
        @load-more="refreshSearch(true)"
        @select-note="selectNote"
        @refresh-wiki="refreshWikiDocuments"
        @create-wiki-page="createWikiPage"
        @toggle-task="(task: TaskEntry, done: boolean) => toggleTask(task.path, task.id, task.expectedLine, done)"
        @refresh-tasks="refreshTasks"
        @map-task-target="mapTaskTarget"
        @pick-task-target="pickTaskTarget"
        @open-external="openExternal"
      />

      <RecordDetail
        ref="recordDetail"
        :session="workspace?.session"
        :detail="activeDetail"
        :loading="detailLoading"
        :error="detailError"
        :opening="opening"
        :desktop="Boolean(desktopWindow)"
        :model-ready="modelReady"
        :organizing="organizing"
        :memory-config="memoryConfig"
        :memory-preview="memoryPreview"
        :wiki-targets="wikiTargets"
        :wiki-target-busy="wikiTargetBusy"
        :wiki-target-error="wikiTargetError && wikiTargetError.path === activeDetail?.path ? wikiTargetError.error : null"
        :task-busy-id="taskBusyId"
        @note-saved="onNoteSaved"
        @reload-detail="reloadCurrentDetail"
        @trigger-organize="triggerOrganize"
        @toggle-task="toggleTask"
        @open-conflicts="openDrawer('conflicts')"
        @open-wiki="openDrawer('wiki', $event)"
        @toggle-wiki-target="toggleWikiTarget"
        @open-settings="showSettings"
        @wiki-suggested="refreshAfterDrawerChange"
        @create-conflict-from-note="openConflictFromNote"
        @return-to-note="returnToNote"
        @retry-memory-preview="memoryPreview && openMemoryPreview(memoryPreview)"
      />

      <M2Answers
        v-if="workspace && chatOpen"
        :key="workspace.session"
        :session="workspace.session"
        :model-ready="modelReady"
        :memory-enabled="Boolean(memoryConfig?.enabled)"
        @open-citation="openCitation"
        @open-conflicts="openDrawer('conflicts')"
        @open-settings="showSettings"
        @focus-search="searchInput?.focus()"
      />
    </main>

    <M2Workbench
      v-if="workspace"
      v-model:open="drawerOpen"
      v-model:tab="drawerTab"
      :session="workspace.session"
      :counts="drawerCounts"
      :wiki-targets="wikiTargets"
      :workspace-files="workspaceFiles"
      :desktop="Boolean(desktopWindow)"
      :prefill-conflict="prefillConflict"
      :wiki-review-id="wikiReviewId"
      @open-note="openLocalSource"
      @changed="refreshAfterDrawerChange"
      @targets-changed="refreshWikiData"
      @clear-prefill="prefillConflict = null"
    />

    <WorkspaceSettingsDialog
      ref="settingsDialog"
      v-model:open="workspaceFormOpen"
      :desktop="Boolean(desktopWindow)"
      :opening="opening"
      :workspace="workspace"
      :initial-workspace-error="workspaceError"
      :hotkey="hotkey"
      :hotkey-busy="hotkeyBusy"
      :hotkey-error="hotkeyError"
      @workspace-opened="applyWorkspace"
      @hotkey-request="updateHotkey"
      @integrations-updated="applyIntegrationsConfig"
      @desktop-action="desktopAction"
    />
  </div>
</template>
