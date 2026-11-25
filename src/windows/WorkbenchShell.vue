<script setup lang="ts">
import { getVersion } from '@tauri-apps/api/app'
import { isTauri } from '@tauri-apps/api/core'
import { getCurrentWindow } from '@tauri-apps/api/window'
import { computed, defineAsyncComponent, nextTick, onMounted, onUnmounted, ref, watch } from 'vue'
import { api } from '../api/client'
import { parseAppError, type UiError } from '../api/errors'
import ErrorNotice from '../components/ErrorNotice.vue'
import RecordDetail from '../components/RecordDetail.vue'
import RecordTimeline from '../components/RecordTimeline.vue'
import WorkspaceSettingsDialog from '../components/WorkspaceSettingsDialog.vue'
import UiIcon from '../UiIcon.vue'
import type {
  HotkeyStatus,
  IndexFeedback,
  ModelConfig,
  OpenedWorkspace,
  RecordEntry,
  RecordSelection,
  RecoveryErrorDto,
  WorkspacePage,
} from '../m2'

const M2Workbench = defineAsyncComponent(() => import('../M2Workbench.vue'))

const desktopWindow = isTauri() ? getCurrentWindow() : undefined
const workspace = ref<{ session: string; root: string; captureDir: string }>()
const buildStatus = ref('正在读取桌面版本…')
const opening = ref(true)
const workspaceError = ref<UiError | null>(null)
const workspaceFormOpen = ref(false)
const recoveredPaths = ref<string[]>([])
const recoveryErrors = ref<RecoveryErrorDto[]>([])
const savedPath = ref('')
const windowError = ref<UiError | null>(null)
const isMaximized = ref(false)

const settingsDialog = ref<InstanceType<typeof WorkspaceSettingsDialog>>()
const recordDetail = ref<InstanceType<typeof RecordDetail>>()
const search = ref<HTMLInputElement>()

const modelConfig = ref<ModelConfig | null>(null)
const hotkey = ref<HotkeyStatus>()
const hotkeyBusy = ref(false)
const hotkeyError = ref<UiError | null>(null)

const workbenchPage = ref<WorkspacePage>('records')
const organizeRecord = ref<RecordSelection>()
const disagreementSources = ref<[RecordSelection | null, RecordSelection | null]>([null, null])

const query = ref('')
const fromDate = ref('')
const toDate = ref('')
const recentRecords = ref<RecordEntry[]>([])
const recordsLoading = ref(false)
const recordsError = ref<UiError | null>(null)
const selectedRecentPath = ref('')
let recentRequest = 0

const selectedSearchPath = ref('')
const searchPage = ref<{ records: RecordEntry[]; hasMore: boolean; skipped: number }>()
const searchLoading = ref(false)
const searchError = ref<{ error: UiError; append: boolean }>()
const indexFeedback = ref<IndexFeedback>()
let searchRequest = 0
let searchTimer: number | undefined
const searchDelayMs = 200

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
const searchCriteria = computed(() => ({
  session: workspace.value?.session,
  keyword: keyword.value,
  fromDate: fromDate.value,
  toDate: toDate.value,
}))
const searchActive = computed(() => Boolean(keyword.value || fromDate.value || toDate.value))
const invalidDateRange = computed(() => Boolean(fromDate.value && toDate.value && fromDate.value > toDate.value))
const displayedRecords = computed(() => searchActive.value ? searchPage.value?.records ?? [] : recentRecords.value)
const selectedRecord = computed(() => {
  const selectedPath = searchActive.value ? selectedSearchPath.value : selectedRecentPath.value
  return displayedRecords.value.find((record) => record.path === selectedPath) ?? displayedRecords.value[0]
})

function trackUnlisten(unlisten: () => void) {
  if (disposed) unlisten()
  else unlisteners.push(unlisten)
}

function applyWorkspace(opened: OpenedWorkspace) {
  workspace.value = { session: opened.session, root: opened.root, captureDir: opened.captureDir }
  recoveredPaths.value = opened.recovered
  recoveryErrors.value = opened.recoveryErrors
  savedPath.value = ''
  workspaceError.value = null
  workspaceFormOpen.value = false
  workbenchPage.value = 'records'
  organizeRecord.value = undefined
  disagreementSources.value = [null, null]
  modelConfig.value = null
  clearSearch()
  selectedRecentPath.value = ''
  recentRecords.value = []
  indexFeedback.value = undefined
  void refreshModelConfig(opened.session)
  void refreshRecords()
}

onMounted(async () => {
  window.addEventListener('keydown', handleWindowKey)
  void getVersion().then(
    (value) => { buildStatus.value = `开发构建 ${value}` },
    () => { buildStatus.value = '请从 Tauri 启动应用' },
  )
  try {
    if (desktopWindow) {
      trackUnlisten(await desktopWindow.listen('capture-closed', () => { void refreshRecords(true) }))
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

watch(searchCriteria, ({ session }, _, onCleanup) => {
  searchPage.value = undefined
  searchError.value = undefined
  selectedSearchPath.value = ''
  searchLoading.value = Boolean(session && searchActive.value && !invalidDateRange.value)
  if (!searchLoading.value) return

  searchTimer = window.setTimeout(() => { void refreshSearch() }, searchDelayMs)
  onCleanup(() => window.clearTimeout(searchTimer))
})

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
    // Errors are surfaced inside WorkspaceSettingsDialog if user inspects model config.
  }
}

function showSettings() {
  workspaceFormOpen.value = true
}

function showModelSettings() {
  showSettings()
  settingsDialog.value?.focusModel()
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

async function refreshRecords(afterCapture = false) {
  const session = workspace.value?.session
  if (!session) return
  const request = ++recentRequest
  const isCurrent = () => request === recentRequest && workspace.value?.session === session
  recordsLoading.value = true
  recordsError.value = null
  try {
    const recent = await api.listRecentCaptures(session)
    if (isCurrent()) {
      const records = recent.map((item) => ({
        path: item.blockId ? item.path + '#fmd-note:' + item.blockId : item.path,
        capturedAt: item.capturedAt,
        text: item.body,
        contentHash: item.contentHash,
        titleText: item.body,
        isCapture: item.isCapture,
      }))
      const latest = records[0]?.path
      if (afterCapture && latest && recentRecords.value[0]?.path !== latest) {
        savedPath.value = latest
        void refreshSearch()
      }
      recentRecords.value = records
    }
  } catch (error) {
    if (isCurrent()) recordsError.value = parseAppError(error)
  } finally {
    if (isCurrent()) recordsLoading.value = false
  }
}

function clearSearch() {
  query.value = fromDate.value = toDate.value = ''
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
  const criteria = searchCriteria.value
  const { session, keyword, fromDate, toDate } = criteria
  if (!session || !searchActive.value || invalidDateRange.value) return
  const previous = append ? searchPage.value : undefined
  if (append && (!previous?.hasMore || searchLoading.value)) return

  window.clearTimeout(searchTimer)
  const request = ++searchRequest
  const isCurrent = () => request === searchRequest && criteria === searchCriteria.value
  searchLoading.value = true
  searchError.value = undefined
  try {
    const page = await api.searchRecords({
      session,
      query: keyword,
      fromUtc: fromDate ? localMidnight(fromDate) : undefined,
      untilUtc: toDate ? localMidnight(toDate, true) : undefined,
      offset: previous?.records.length ?? 0,
    })
    if (isCurrent()) {
      const records: RecordEntry[] = page.hits.map((hit) => ({
        path: hit.path,
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
    if (isCurrent()) searchError.value = { error: parseAppError(error), append }
  } finally {
    if (isCurrent()) searchLoading.value = false
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
    }
  } catch (error) {
    if (workspace.value?.session === session) {
      indexFeedback.value = { state: 'failed', error: parseAppError(error) }
    }
  }
}

function selectRecord(path: string, focusDetail = false) {
  if (searchActive.value) selectedSearchPath.value = path
  else selectedRecentPath.value = path
  if (focusDetail || window.matchMedia('(max-width: 60rem)').matches) recordDetail.value?.focus()
}

function moveRecordSelection(delta: number) {
  const list = displayedRecords.value
  if (!list.length) return
  const index = list.findIndex((item) => item.path === selectedRecord.value?.path)
  const nextIndex = index < 0 ? 0 : Math.max(0, Math.min(list.length - 1, index + delta))
  selectRecord(list[nextIndex].path)
}

function focusSelectedDetail() {
  if (selectedRecord.value) recordDetail.value?.focus()
}

function showSavedRecord() {
  workbenchPage.value = 'records'
  clearSearch()
  const path = savedPath.value
  savedPath.value = ''
  void nextTick(() => selectRecord(path, true))
}

function showRecords() {
  workbenchPage.value = 'records'
  void nextTick(() => search.value?.focus())
}

function onOrganize(selection: RecordSelection) {
  organizeRecord.value = selection
  workbenchPage.value = 'organize'
}

function applyDisagreementSource(material: RecordSelection, slotIndex?: number) {
  const targetSlot = slotIndex ?? disagreementSources.value.findIndex((item) => item === null)
  const index = targetSlot >= 0 ? targetSlot : 0
  disagreementSources.value[index] = material
  workbenchPage.value = 'answers'
}

function handleWindowKey(event: KeyboardEvent) {
  if (event.isComposing || event.keyCode === 229) return
  if (!workspaceFormOpen.value && event.ctrlKey && event.key.toLowerCase() === 'k') {
    event.preventDefault()
    showRecords()
  }
}
</script>

<template>
  <div class="workbench">
    <aside class="sidebar" aria-label="工作台导航">
      <header class="brand" @mousedown.left="desktopWindow && desktopAction('drag')" @dblclick="desktopWindow && desktopAction('maximize')">
        <span class="brand-icon"><UiIcon name="terminal" /></span>
        <div><span class="brand-name">Flash Marking Down</span><small>{{ buildStatus }}</small></div>
      </header>
      <nav class="sidebar-nav">
        <button type="button" :class="{ 'nav-current': workbenchPage === 'records' }" :aria-current="workbenchPage === 'records' ? 'page' : undefined" @click="workbenchPage = 'records'; clearSearch()"><UiIcon name="history" />时间线与记录</button>
        <button type="button" :disabled="!workspace || opening" :class="{ 'nav-current': workbenchPage === 'organize' }" :aria-current="workbenchPage === 'organize' ? 'page' : undefined" @click="workbenchPage = 'organize'"><UiIcon name="organize" />整理建议</button>
        <button type="button" :disabled="!workspace || opening" :class="{ 'nav-current': workbenchPage === 'wiki' }" :aria-current="workbenchPage === 'wiki' ? 'page' : undefined" @click="workbenchPage = 'wiki'"><UiIcon name="file" />Wiki 变更</button>
        <button type="button" :disabled="!workspace || opening" :class="{ 'nav-current': workbenchPage === 'tasks' }" :aria-current="workbenchPage === 'tasks' ? 'page' : undefined" @click="workbenchPage = 'tasks'"><UiIcon name="tasks" />任务清单</button>
        <button type="button" :disabled="!workspace || opening" :class="{ 'nav-current': workbenchPage === 'answers' }" :aria-current="workbenchPage === 'answers' ? 'page' : undefined" @click="workbenchPage = 'answers'"><UiIcon name="question" />问答与分歧</button>
        <button type="button" :disabled="opening" @click="showCapture"><UiIcon name="bolt" />随手记</button>
        <button type="button" :disabled="opening" @click="showSettings"><UiIcon name="settings" />工作区设置</button>
      </nav>
      <div v-if="workspace" class="sidebar-workspace">
        <p><UiIcon name="folder" /> <strong :title="workspace.root">{{ folderLabel }}</strong></p>
        <small><span class="status-dot good" />本地工作区</small>
      </div>
    </aside>

    <div class="workbench-body">
      <header class="topbar">
        <label class="search-box">
          <UiIcon name="search" />
          <span class="visually-hidden">搜索工作区 Markdown</span>
          <input
            ref="search"
            v-model="query"
            type="search"
            placeholder="搜索工作区 Markdown…"
            :disabled="!workspace || opening"
            aria-keyshortcuts="Control+k"
            @input="workbenchPage = 'records'"
            @keydown.down.prevent="moveRecordSelection(1)"
            @keydown.up.prevent="moveRecordSelection(-1)"
            @keydown.enter="focusSelectedDetail"
          >
          <kbd aria-hidden="true">Ctrl+K</kbd>
        </label>
        <button type="button" class="new-capture" :disabled="opening" @click="showCapture">
          <UiIcon name="bolt" />随手记
          <kbd v-if="activeHotkeyLabel" aria-hidden="true">{{ activeHotkeyLabel }}</kbd>
        </button>
        <div v-if="desktopWindow" class="window-drag" aria-hidden="true" @mousedown.left="desktopAction('drag')" @dblclick="desktopAction('maximize')" />
        <div v-if="desktopWindow" class="window-controls" role="group" aria-label="工作台窗口">
          <button type="button" aria-label="最小化工作台" title="最小化" @click="desktopAction('minimize')"><UiIcon name="minimize" /></button>
          <button type="button" :aria-label="isMaximized ? '还原工作台' : '最大化工作台'" :title="isMaximized ? '还原' : '最大化'" @click="desktopAction('maximize')"><UiIcon :name="isMaximized ? 'restore' : 'maximize'" /></button>
          <button type="button" aria-label="关闭工作台（仍在托盘运行）" title="关闭窗口（仍在托盘运行）" @click="desktopAction('close')"><UiIcon name="close" /></button>
        </div>
      </header>

      <main class="workbench-main">
        <ErrorNotice :error="windowError" />
        <div v-if="hotkeyError" class="notice error hotkey-notice">
          <span role="alert">{{ hotkeyError.safeMessage }}。仍可从工作台或托盘打开随手记。</span>
          <button type="button" class="quiet" @click="showHotkeySettings">更改快捷键</button>
        </div>
        <div v-if="savedPath" class="notice success save-receipt">
          <span role="status">已保存随手记</span>
          <div class="actions">
            <button type="button" class="quiet" @click="showSavedRecord">查看记录</button>
            <button type="button" class="quiet" aria-label="关闭已保存提示" title="关闭提示" @click="savedPath = ''"><UiIcon name="close" /></button>
          </div>
        </div>
        <p v-if="recoveredPaths.length" class="notice success" role="status">已恢复 {{ recoveredPaths.length }} 条随手记</p>
        <p v-for="item in recoveryErrors" :key="item.requestId" class="notice error" role="alert">恢复遇到问题。{{ item.message }}</p>

        <div v-show="workbenchPage === 'records'" class="timeline-layout">
          <RecordTimeline
            :has-workspace="Boolean(workspace)"
            :opening="opening"
            :model-ready="modelReady"
            :search-active="searchActive"
            :invalid-date-range="invalidDateRange"
            :from-date="fromDate"
            :to-date="toDate"
            :recent-records="recentRecords"
            :records-loading="recordsLoading"
            :records-error="recordsError"
            :search-page="searchPage"
            :search-loading="searchLoading"
            :search-error="searchError"
            :index-feedback="indexFeedback"
            :selected-path="selectedRecord?.path"
            @update:from-date="fromDate = $event"
            @update:to-date="toDate = $event"
            @clear-search="clearSearch(); search?.focus()"
            @rebuild-index="rebuildSearchIndex"
            @open-settings="showSettings"
            @open-capture="showCapture"
            @retry-records="refreshRecords()"
            @retry-search="refreshSearch($event)"
            @load-more="refreshSearch(true)"
            @select="selectRecord"
          />

          <RecordDetail
            ref="recordDetail"
            :session="workspace?.session"
            :record="selectedRecord"
            :opening="opening"
            :search-active="searchActive"
            :desktop="Boolean(desktopWindow)"
            :disagreement-slots-full="disagreementSources.every(Boolean)"
            @organize="onOrganize"
            @add-disagreement-source="applyDisagreementSource($event)"
          />
        </div>

        <M2Workbench
          v-if="workspace"
          v-show="workbenchPage !== 'records'"
          :key="workspace.session"
          :session="workspace.session"
          :page="workbenchPage"
          :record="organizeRecord"
          :sources="disagreementSources"
          :model="modelConfig"
          :model-ready="modelReady"
          @navigate="workbenchPage = $event"
          @show-records="showRecords"
          @show-settings="showModelSettings"
          @remove-source="disagreementSources[$event] = null"
          @clear-sources="disagreementSources = [null, null]"
          @select-disagreement-source="applyDisagreementSource"
        />
      </main>
    </div>

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
      :model-config="modelConfig"
      @workspace-opened="applyWorkspace"
      @hotkey-request="updateHotkey"
      @model-updated="modelConfig = $event"
      @desktop-action="desktopAction"
    />
  </div>
</template>
