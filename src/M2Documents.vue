<script setup lang="ts">
import { isTauri } from '@tauri-apps/api/core'
import { computed, onMounted, onUnmounted, ref, watch } from 'vue'
import { api } from './api/client'
import { parseAppError, type UiError } from './api/errors'
import ErrorNotice from './components/ErrorNotice.vue'
import DiffViewer from './DiffViewer.vue'
import M2Evidence from './M2Evidence.vue'
import UiIcon from './UiIcon.vue'
import type {
  ChangeKind,
  ChangePhase,
  DocumentChange,
  DocumentMapping,
  Evidence,
  RecordSelection,
  TaskEntry,
  TaskList,
  TaskState,
  WikiApplyOutcome,
} from './m2'
import { phaseBadgeTone, useSourcePresentation } from './m2'

const props = defineProps<{
  session: string
  mode: 'wiki' | 'tasks' | null
  mappings: DocumentMapping[]
  preview?: DocumentChange
}>()
const emit = defineEmits<{ showOrganize: []; selectDisagreementSource: [source: RecordSelection] }>()
const { showSourcePath } = useSourcePresentation()
const desktop = isTauri()

const phaseLabels: Record<ChangePhase, string> = {
  pending: '待审阅', deferred: '稍后处理', rejected: '已拒绝', prepared: '待核对写入结果',
  applied: '已采用', conflict: '版本冲突', reverted: '已撤销', superseded: '已由新版预览替代',
}
const kindLabels: Record<ChangeKind, string> = {
  wiki: 'Wiki 修改', task: '加入任务清单', status: '任务状态修改',
  undo_wiki: '撤销 Wiki 修改', undo_task: '撤销加入任务', undo_status: '撤销任务状态修改',
}
const stateLabels: Record<TaskState, string> = { open: '待办', done: '已完成', cancelled: '已取消', invalid: '待修复' }
const stateBadgeTone: Record<TaskState, string> = { open: 'badge-accent', done: 'badge-success', cancelled: 'badge-muted', invalid: 'badge-error' }

const changes = ref<DocumentChange[]>([])
const selectedId = ref('')
const chosen = ref<string[]>([])
const changesLoading = ref(false)
const outcomes = ref<WikiApplyOutcome[]>([])
const taskList = ref<TaskList>()
const taskLoading = ref(false)
const busy = ref(false)
const error = ref<UiError | null>(null)
const feedback = ref('')

const taskTarget = computed(() => props.mappings.find((m) => m.kind === 'tasks')?.path)
const selected = computed(() => changes.value.find((change) => change.id === selectedId.value) ?? changes.value[0])
const editable = computed(() => selected.value && canReview(selected.value))
const changeSource = computed<Evidence | undefined>(() => {
  const item = selected.value
  if (!item) return undefined
  return {
    id: item.id,
    path: item.sourcePath,
    revision: item.sourceRevision,
    startLine: item.startLine,
    endLine: item.endLine,
    excerpt: item.sourceExcerpt,
    date: null,
    available: true,
    changed: false,
    outsideRange: false,
    isCapture: item.sourceIsCapture,
  }
})

async function openTargetExternal(path: string, mode: 'open' | 'reveal') {
  error.value = null
  try {
    await api.openWorkspaceFile(props.session, path, mode)
  } catch (failure) {
    error.value = parseAppError(failure)
  }
}

watch(() => props.preview, (change) => { if (change) showChange(change) })
watch(() => props.mode, (mode) => {
  error.value = null
  feedback.value = ''
  if (mode === 'wiki') void loadChanges()
})
watch(() => [props.mode, taskTarget.value] as const, ([mode, target], previous) => {
  if (target !== previous?.[1]) taskList.value = undefined
  if (mode === 'tasks' && target) void loadTasks()
})

onMounted(() => window.addEventListener('focus', refreshFromDisk))
onUnmounted(() => window.removeEventListener('focus', refreshFromDisk))

function refreshFromDisk() {
  if (props.mode === 'tasks' && taskTarget.value && !busy.value) void loadTasks()
}

function showChange(change: DocumentChange) {
  changes.value = [change, ...changes.value.filter((item) => item.id !== change.id)]
  selectedId.value = change.id
}

function canReview(change: DocumentChange) {
  return change.phase === 'pending' || change.phase === 'deferred'
}

async function loadChanges() {
  if (changesLoading.value) return
  changesLoading.value = true
  try {
    changes.value = await api.wikiChanges(props.session)
    chosen.value = chosen.value.filter((id) => changes.value.some((change) => change.id === id && canReview(change)))
  } catch (failure) {
    error.value = parseAppError(failure)
  } finally {
    changesLoading.value = false
  }
}

async function refreshChanges() {
  error.value = null
  await loadChanges()
}

async function applyChanges(ids: string[]) {
  if (!ids.length || busy.value || changesLoading.value) return
  busy.value = true
  outcomes.value = []
  error.value = null
  feedback.value = ''
  try {
    outcomes.value = await api.applyWikiChanges(props.session, ids)
    await loadChanges()
  } catch (failure) {
    error.value = parseAppError(failure)
  } finally {
    busy.value = false
  }
}

async function decide(id: string, decision: 'rejected' | 'deferred') {
  if (busy.value) return
  busy.value = true
  error.value = null
  feedback.value = ''
  try {
    await api.reviewWikiChange(props.session, id, decision)
    feedback.value = decision === 'rejected' ? '已拒绝此变更。' : '已保留预览，稍后可继续审阅。'
    await loadChanges()
  } catch (failure) {
    error.value = parseAppError(failure)
  } finally {
    busy.value = false
  }
}

async function generatePreview(id: string, command: 'preview_wiki_undo' | 'rebase_wiki_change') {
  if (busy.value) return
  busy.value = true
  error.value = null
  feedback.value = ''
  try {
    const change = command === 'preview_wiki_undo'
      ? await api.previewWikiUndo(props.session, id)
      : await api.rebaseWikiChange(props.session, id)
    showChange(change)
    await loadChanges()
  } catch (failure) {
    error.value = parseAppError(failure)
  } finally {
    busy.value = false
  }
}

async function loadTasks() {
  const target = taskTarget.value
  if (!target || taskLoading.value) return
  taskLoading.value = true
  try {
    const list = await api.wikiTasks(props.session)
    if (taskTarget.value === target) taskList.value = list
  } catch (failure) {
    if (taskTarget.value === target) {
      taskList.value = undefined
      error.value = parseAppError(failure)
    }
  } finally {
    taskLoading.value = false
    if (taskTarget.value !== target && props.mode === 'tasks') void loadTasks()
  }
}

async function refreshTasks() {
  error.value = null
  await loadTasks()
}

async function toggleTask(task: TaskEntry, event: Event) {
  const checkbox = event.currentTarget as HTMLInputElement
  checkbox.checked = task.state === 'done'
  const list = taskList.value
  if (!list || busy.value || taskLoading.value || task.manual || !task.id || (task.state !== 'open' && task.state !== 'done')) return
  const state = task.state === 'open' ? 'done' : 'open'
  busy.value = true
  error.value = null
  feedback.value = ''
  try {
    const prepared = await api.prepareTaskStatus(props.session, task.id, list.revision, state)
    const [result] = await api.applyWikiChanges(props.session, [prepared.id])
    if (result?.outcome !== 'applied') error.value = { safeMessage: result?.message || '任务状态未写入，请刷新清单。' }
    else feedback.value = '已保存任务状态。'
  } catch (failure) {
    error.value = parseAppError(failure)
  } finally {
    await loadTasks()
    busy.value = false
  }
}

function targetFor(id: string) {
  return changes.value.find((change) => change.id === id)?.target ?? ''
}
</script>

<template>
  <section v-if="mode === 'wiki'" class="m2-stack" aria-label="审阅文件变更">
    <div class="m2-card-header">
      <div class="actions">
        <button type="button" class="quiet" :disabled="busy || changesLoading" @click="refreshChanges">刷新变更列表</button>
        <button v-if="chosen.length" type="button" class="primary" :disabled="busy || changesLoading" @click="applyChanges(chosen)">采用所选 {{ chosen.length }} 项</button>
      </div>
    </div>
    <p v-if="busy || changesLoading" class="hint" role="status">{{ busy ? '正在处理文件变更…' : '正在读取变更…' }}</p>
    <ErrorNotice :error="error" />
    <p v-if="feedback" class="notice success" role="status">{{ feedback }}</p>
    <ul v-if="outcomes.length" class="m2-list" aria-label="逐项处理结果">
      <li v-for="item in outcomes" :key="item.id" class="notice" :class="item.outcome === 'applied' ? 'success' : 'error'" role="status">
        <code>{{ targetFor(item.id) }}</code>　{{ item.outcome === 'applied' ? '已应用' : item.message || '变更未应用，请刷新核对' }}
      </li>
    </ul>
    <div v-if="!changes.length && !changesLoading && !error" class="empty-panel compact">
      <p>还没有待审阅的文件变更。整理记录后可预览 Wiki 修改或加入任务清单。</p>
      <button class="quiet" type="button" @click="emit('showOrganize')">查看整理建议</button>
    </div>
    <div v-if="selected && changeSource" class="m2-review-layout">
      <ul class="m2-list" aria-label="文件变更列表">
        <li v-for="change in changes" :key="change.id">
          <button type="button" class="record-card" :aria-pressed="selected.id === change.id" @click="selectedId = change.id">
            <div class="m2-card-header">
              <strong>{{ kindLabels[change.kind] }}</strong>
              <span class="badge" :class="phaseBadgeTone[change.phase]">{{ phaseLabels[change.phase] }}</span>
            </div>
            <code class="record-path" :title="change.target">{{ change.target }}</code>
          </button>
        </li>
      </ul>
      <article class="detail-card m2-stack">
        <header class="m2-card-header">
          <h2>{{ kindLabels[selected.kind] }}</h2>
          <span class="badge" :class="phaseBadgeTone[selected.phase]">{{ phaseLabels[selected.phase] }}</span>
        </header>
        <div class="m2-card-header">
          <div class="path-bar"><code>{{ selected.target }}</code></div>
          <div v-if="desktop" class="actions">
            <button type="button" class="quiet" @click="openTargetExternal(selected.target, 'open')"><UiIcon name="external" />外部打开</button>
          </div>
        </div>
        <M2Evidence
          :session="session"
          :evidence="changeSource"
          @select-disagreement-source="emit('selectDisagreementSource', $event)"
        />
        <details><summary>基准版本</summary><code>{{ selected.baseHash }}</code></details>
        <DiffViewer :before="selected.before" :after="selected.after" />
        <template v-if="editable">
          <div class="actions">
            <button class="primary" type="button" :disabled="busy || changesLoading" @click="applyChanges([selected.id])">{{ selected.kind.startsWith('undo_') ? '确认撤销' : '采用此项' }}</button>
            <button type="button" :disabled="busy || changesLoading" @click="decide(selected.id, 'rejected')">拒绝</button>
            <button v-if="selected.phase === 'pending'" type="button" class="quiet" :disabled="busy || changesLoading" @click="decide(selected.id, 'deferred')">稍后处理</button>
          </div>
          <label class="m2-check"><input v-model="chosen" type="checkbox" :value="selected.id" :disabled="busy || changesLoading">加入本次批量采用</label>
        </template>
        <div v-else-if="selected.phase === 'conflict'" class="actions">
          <button type="button" :disabled="busy || changesLoading" @click="generatePreview(selected.id, 'rebase_wiki_change')">读取当前版本并重新比较</button>
        </div>
        <div v-else-if="selected.phase === 'applied' && !selected.kind.startsWith('undo_')" class="actions">
          <button type="button" :disabled="busy || changesLoading" @click="generatePreview(selected.id, 'preview_wiki_undo')">查看撤销预览</button>
        </div>
      </article>
    </div>
  </section>

  <section v-else-if="mode === 'tasks'" class="m2-stack" aria-label="Markdown 任务清单">
    <div v-if="taskTarget" class="m2-card-header">
      <code class="m2-path">{{ taskTarget }}</code>
      <div class="actions">
        <button v-if="desktop" type="button" class="quiet" @click="openTargetExternal(taskTarget, 'open')"><UiIcon name="external" />外部编辑清单</button>
        <button type="button" class="quiet" :disabled="busy || taskLoading" @click="refreshTasks">刷新清单</button>
      </div>
    </div>
    <p v-if="taskLoading || busy" class="hint" role="status">{{ busy ? '正在写入任务状态并回读清单…' : '正在读取 Markdown 清单…' }}</p>
    <ErrorNotice :error="error" />
    <p v-if="feedback" class="notice success" role="status">{{ feedback }}</p>
    <p v-if="!taskTarget" class="empty-panel compact">先映射一个 Markdown 任务清单文件。采用的候选任务会写入该清单。</p>
    <template v-else-if="taskList">
      <ul v-if="taskList.repairMessages.length" class="m2-list" aria-label="任务清单待修复项">
        <li v-for="(message, index) in taskList.repairMessages" :key="index" class="notice error">{{ message }}</li>
      </ul>
      <p v-if="!taskList.entries.length && !taskLoading" class="empty-panel compact">任务清单为空。</p>
      <ul class="m2-list">
        <li v-for="(task, index) in taskList.entries" :key="task.id || `manual-${index}`" class="detail-card m2-task" :class="{ 'm2-task-done': task.state === 'done' }">
          <label class="m2-check">
            <input
              type="checkbox"
              :checked="task.state === 'done'"
              :disabled="busy || taskLoading || task.manual || !task.id || task.state === 'invalid' || task.state === 'cancelled'"
              :title="task.manual && task.state !== 'invalid' ? '手工任务请在原 Markdown 文件中编辑' : undefined"
              @change="toggleTask(task, $event)"
            >
            <strong>{{ task.text }}</strong>
          </label>
          <div class="actions">
            <span class="badge" :class="stateBadgeTone[task.state]">{{ stateLabels[task.state] }}</span>
            <span v-if="task.manual && task.state !== 'invalid'" class="badge badge-muted">手工任务</span>
            <span v-if="task.sourcePath" class="hint">来源 <code v-if="showSourcePath(task.sourcePath, task.sourceIsCapture)">{{ task.sourcePath }}</code><span v-else>随手记</span></span>
          </div>
        </li>
      </ul>
    </template>
  </section>
</template>
