<script setup lang="ts">
import { computed, ref } from 'vue'
import type { UiError } from '../api/errors'
import ErrorNotice from './ErrorNotice.vue'
import UiIcon from '../UiIcon.vue'
import {
  recordTitle,
  useSourcePresentation,
  type IndexFeedback,
  type LeftTab,
  type MemoryConfig,
  type MemorySyncStatus,
  type NoteSummary,
  type RecordEntry,
  type TaskEntry,
  type TaskList,
} from '../m2'

const props = defineProps<{
  hasWorkspace: boolean
  opening: boolean
  desktop: boolean
  leftTab: LeftTab
  searchActive: boolean
  invalidDateRange: boolean
  fromDate: string
  toDate: string
  notes: NoteSummary[]
  memoryConfig: MemoryConfig | null
  notesLoading: boolean
  notesError: UiError | null
  searchPage?: { records: RecordEntry[]; hasMore: boolean; skipped: number }
  searchLoading: boolean
  searchError?: { error: UiError; append: boolean }
  indexFeedback?: IndexFeedback
  selectedLocator?: string
  taskList?: TaskList
  tasksLoading: boolean
  tasksError: UiError | null
  taskBusyId: string
  workspaceFiles: string[]
}>()

const emit = defineEmits<{
  'update:leftTab': [tab: LeftTab]
  'update:fromDate': [value: string]
  'update:toDate': [value: string]
  clearSearch: []
  rebuildIndex: []
  openSettings: []
  openCapture: []
  retryNotes: []
  retrySearch: [append: boolean]
  loadMore: []
  selectNote: [locator: string, focusDetail: boolean]
  toggleTask: [task: TaskEntry, done: boolean]
  refreshTasks: []
  mapTaskTarget: [path: string]
  pickTaskTarget: []
  openExternal: [path: string]
}>()

const { showSourcePath, sourceMarkdown } = useSourcePresentation()
const dateLabel = new Intl.DateTimeFormat('zh-CN', { dateStyle: 'full' })
const timeLabel = new Intl.DateTimeFormat('zh-CN', { timeStyle: 'short', hour12: false })

const taskMapInput = ref('')
const showTaskTargetForm = ref(false)

const openTaskCount = computed(() =>
  props.taskList?.entries.filter((item) => item.state === 'open').length ?? 0,
)

type TimelineRecord = RecordEntry & { syncStatus?: MemorySyncStatus }

const syncStatusByLocator = computed(() => {
  const statuses = new Map<string, MemorySyncStatus>()
  const config = props.memoryConfig
  if (!config?.enabled || !(config.syncResources || config.commitMemories)) return statuses
  for (const note of props.notes) {
    const routes = [
      config.syncResources ? note.ovResourceStatus : undefined,
      config.commitMemories ? note.ovMemoryStatus : undefined,
    ]
    statuses.set(note.locator, routes.includes('failed') ? 'failed' : routes.includes('pending') ? 'pending' : 'synced')
  }
  return statuses
})

const displayedRecords = computed<TimelineRecord[]>(() => {
  if (props.searchActive) {
    return (props.searchPage?.records ?? []).map((record) => ({
      ...record,
      syncStatus: syncStatusByLocator.value.get(keyForRecord(record)),
    }))
  }
  return props.notes.map((n) => ({
    id: n.id,
    path: n.path,
    locator: n.locator,
    capturedAt: n.capturedAt,
    text: n.bodyPreview,
    contentHash: n.contentHash,
    titleText: n.title || n.bodyPreview,
    isCapture: n.isCapture,
    organizeStatus: n.organizeStatus,
    syncStatus: syncStatusByLocator.value.get(n.locator),
    openConflictCount: n.openConflictCount,
  }))
})

const groupedRecords = computed(() => {
  const days = new Map<string, TimelineRecord[]>()
  for (const record of displayedRecords.value) {
    const date = record.capturedAt ? dateLabel.format(new Date(record.capturedAt)) : '记录日期未知'
    const day = days.get(date) ?? []
    day.push(record)
    days.set(date, day)
  }
  return days
})

const syncIndicators: Record<MemorySyncStatus, { tone: string; label: string }> = {
  pending: { tone: '', label: 'OV 待同步' },
  synced: { tone: 'good', label: 'OV 已同步' },
  failed: { tone: 'bad', label: 'OV 同步失败' },
}

function organizeDotClass(status?: string) {
  if (status === 'done') return 'good'
  if (status === 'running' || status === 'queued') return 'warn'
  if (status === 'failed') return 'bad'
  return ''
}

function organizeDotTitle(status?: string) {
  if (status === 'done') return '已完成整理'
  if (status === 'running') return '正在后台整理'
  if (status === 'queued') return '等待整理'
  if (status === 'failed') return '整理失败（点击查看或重试）'
  return '未整理'
}

function keyForRecord(record: RecordEntry) {
  return record.locator ?? record.path
}

function submitTaskMapping() {
  const path = taskMapInput.value.trim()
  if (!path) return
  emit('mapTaskTarget', path)
  taskMapInput.value = ''
  showTaskTargetForm.value = false
}
</script>

<template>
  <aside id="timeline" class="pane pane-left" aria-label="记录与待办栏">
    <header class="pane-tabs" role="tablist" aria-label="左栏视图切换">
      <button
        type="button"
        role="tab"
        :aria-selected="leftTab === 'notes'"
        :class="{ active: leftTab === 'notes' }"
        @click="emit('update:leftTab', 'notes')"
      >
        <UiIcon name="history" />
        <span>{{ searchActive ? '搜索' : '记录' }}</span>
        <span class="tab-count">{{ displayedRecords.length }}</span>
      </button>
      <button
        type="button"
        role="tab"
        :aria-selected="leftTab === 'tasks'"
        :class="{ active: leftTab === 'tasks' }"
        @click="emit('update:leftTab', 'tasks')"
      >
        <UiIcon name="tasks" />
        <span>待办</span>
        <span class="tab-count">{{ openTaskCount }}</span>
      </button>
    </header>

    <div v-show="leftTab === 'notes'" class="pane-scroll">
      <div v-if="hasWorkspace && (searchActive || indexFeedback)" class="left-toolbar">
        <div class="actions">
          <button v-if="searchActive" type="button" class="quiet compact-btn" @click="emit('clearSearch')">清除筛选</button>
          <button
            type="button"
            class="quiet compact-btn"
            :disabled="opening || indexFeedback?.state === 'building'"
            @click="emit('rebuildIndex')"
          >
            {{ indexFeedback?.state === 'failed' ? '重试索引' : '重建索引' }}
          </button>
        </div>
        <p v-if="indexFeedback?.state === 'building'" class="hint" role="status">正在重建索引…</p>
        <p v-else-if="indexFeedback?.state === 'ready'" class="notice success compact-notice" role="status">
          已收录 {{ indexFeedback.indexed }} 份文档<template v-if="indexFeedback.skipped">（跳过 {{ indexFeedback.skipped }} 项）</template>
        </p>
        <ErrorNotice v-else-if="indexFeedback?.state === 'failed'" :error="indexFeedback.error" prefix="索引失败：" />
      </div>

      <p v-if="opening && !hasWorkspace" class="empty-panel compact" role="status">正在打开工作区…</p>
      <div v-else-if="!hasWorkspace" class="empty-panel compact">
        <UiIcon name="folder" />
        <h2>打开本地工作区</h2>
        <p class="hint">选择 Markdown 文件夹即可开始。</p>
        <button type="button" class="primary" @click="emit('openSettings')">选择工作区</button>
      </div>
      <p v-else-if="invalidDateRange" class="notice error" role="alert">开始日期不能晚于结束日期。</p>
      <template v-else-if="searchActive">
        <div v-if="searchError" class="empty-panel compact">
          <ErrorNotice :error="searchError.error" prefix="搜索失败：" />
          <button type="button" @click="emit('retrySearch', searchError.append)">重试搜索</button>
        </div>
        <p v-else-if="searchLoading && !searchPage" class="empty-panel compact" role="status">正在搜索工作区…</p>
        <div v-else-if="searchPage && !displayedRecords.length" class="empty-panel compact">
          <UiIcon name="search" />
          <h2>没有匹配的记录</h2>
          <p class="hint">调整关键词或日期范围后再试。</p>
        </div>
      </template>
      <div v-else-if="notesError" class="empty-panel compact">
        <ErrorNotice :error="notesError" />
        <button type="button" @click="emit('retryNotes')">重试读取</button>
      </div>
      <p v-else-if="notesLoading && !notes.length" class="empty-panel compact" role="status">正在读取最近记录…</p>
      <div v-else-if="!notes.length" class="empty-panel compact">
        <UiIcon name="history" />
        <h2>还没有随手记</h2>
        <button type="button" class="primary" @click="emit('openCapture')">写第一条笔记</button>
      </div>

      <section v-for="[date, day] in groupedRecords" :key="date" class="timeline-day" :aria-label="date">
        <h2 class="day-heading"><span class="timeline-dot" />{{ date }}</h2>
        <ul>
          <li v-for="record in day" :key="keyForRecord(record)">
            <button
              type="button"
              class="record-card compact-card"
              :aria-pressed="selectedLocator === keyForRecord(record)"
              @click="emit('selectNote', keyForRecord(record), $event.detail === 0)"
            >
              <span class="record-heading">
                <span
                  v-if="!searchActive"
                  class="status-dot"
                  :class="organizeDotClass(record.organizeStatus)"
                  :title="organizeDotTitle(record.organizeStatus)"
                />
                <strong>{{ recordTitle(record) }}</strong>
                <span v-if="record.openConflictCount" class="mini-flag" title="存在待处理观点冲突">⚡{{ record.openConflictCount }}</span>
                <time v-if="record.capturedAt" :datetime="record.capturedAt">{{ timeLabel.format(new Date(record.capturedAt)) }}</time>
              </span>
              <span v-if="record.text && record.text !== recordTitle(record)" class="record-excerpt">{{ sourceMarkdown(record.text) }}</span>
              <code v-if="showSourcePath(record.path, record.isCapture)" class="record-path">{{ record.path }}</code>
              <span v-if="record.syncStatus" class="record-sync-state">
                <span class="status-dot" :class="syncIndicators[record.syncStatus].tone" aria-hidden="true" />
                {{ syncIndicators[record.syncStatus].label }}
              </span>
            </button>
          </li>
        </ul>
      </section>

      <button
        v-if="searchActive && searchPage?.hasMore && !searchError"
        class="load-more"
        type="button"
        :aria-disabled="searchLoading || undefined"
        @click="emit('loadMore')"
      >
        {{ searchLoading ? '正在加载…' : '加载更多' }}
      </button>
    </div>

    <div v-show="leftTab === 'tasks'" class="pane-scroll">
      <div v-if="hasWorkspace" class="left-toolbar">
        <div class="actions">
          <button type="button" class="quiet compact-btn" :disabled="tasksLoading" @click="emit('refreshTasks')">刷新</button>
          <button type="button" class="quiet compact-btn" @click="showTaskTargetForm = !showTaskTargetForm">
            {{ taskList?.path ? '清单文件' : '映射清单' }}
          </button>
          <button
            v-if="desktop && taskList?.path"
            type="button"
            class="quiet compact-btn"
            @click="emit('openExternal', taskList.path)"
          >
            <UiIcon name="external" />打开
          </button>
        </div>
        <p v-if="taskList?.path" class="hint path-hint"><code>{{ taskList.path }}</code></p>
        <form v-if="showTaskTargetForm" class="m2-form compact-form" @submit.prevent="submitTaskMapping">
          <div class="folder-field">
            <input
              v-model="taskMapInput"
              list="left-task-md-files"
              placeholder="输入或选择任务清单 .md 路径"
              spellcheck="false"
            >
            <button v-if="desktop" type="button" class="compact-btn" @click="emit('pickTaskTarget')">浏览</button>
          </div>
          <datalist id="left-task-md-files">
            <option v-for="file in workspaceFiles" :key="file" :value="file" />
          </datalist>
          <button type="submit" class="compact-btn" :disabled="!taskMapInput.trim()">绑定清单</button>
        </form>
      </div>

      <ErrorNotice :error="tasksError" />
      <p v-if="tasksLoading && !taskList" class="hint" role="status">正在读取待办…</p>
      <template v-else-if="taskList">
        <ul v-if="taskList.repairMessages.length" class="m2-list">
          <li v-for="(msg, i) in taskList.repairMessages" :key="i" class="notice error compact-notice">{{ msg }}</li>
        </ul>
        <p v-if="!taskList.entries.length" class="empty-panel compact">
          暂无待办。记录中写 `- [ ] 待办` 或后台整理提取后会自动出现。
        </p>
        <ul class="m2-list">
          <li
            v-for="(task, idx) in taskList.entries"
            :key="task.id ? `${task.path}:${task.id}` : idx"
            class="task-row"
            :class="{ 'task-row-done': task.state === 'done' }"
          >
            <label class="m2-check task-check">
              <input
                type="checkbox"
                :checked="task.state === 'done'"
                :disabled="taskBusyId === task.id || !task.id || task.state === 'invalid' || task.state === 'cancelled'"
                @change="emit('toggleTask', task, ($event.target as HTMLInputElement).checked)"
              >
              <span class="task-text">{{ task.text }}</span>
            </label>
            <div class="task-meta">
              <span v-if="task.assignee" class="badge badge-muted">{{ task.assignee }}</span>
              <span v-if="task.due" class="badge badge-muted">{{ task.due }}</span>
              <button
                v-if="task.sourcePath || task.path"
                type="button"
                class="quiet link-btn"
                @click="emit('selectNote', task.sourcePath || task.path, true)"
              >
                {{ showSourcePath(task.sourcePath || task.path, task.sourceIsCapture) ? (task.sourcePath || task.path) : '查看原记录' }}
              </button>
            </div>
          </li>
        </ul>
      </template>
    </div>
  </aside>
</template>
