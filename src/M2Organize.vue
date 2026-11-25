<script setup lang="ts">
import { computed, ref, shallowRef, watch } from 'vue'
import { api } from './api/client'
import { parseAppError, type UiError } from './api/errors'
import ErrorNotice from './components/ErrorNotice.vue'
import {
  conflictStatusLabels,
  conflictStatusTone,
  wikiStatusLabels,
  wikiStatusTone,
  type DocumentMapping,
  type MemoryConfig,
  type MemorySyncStatus,
  type NoteDetail,
} from './m2'

const props = defineProps<{
  session: string
  detail: NoteDetail
  modelReady: boolean
  organizing: boolean
  memoryConfig: MemoryConfig | null
  wikiTargets: DocumentMapping[]
  taskBusyId: string
}>()

const emit = defineEmits<{
  triggerOrganize: []
  toggleTask: [taskId: string, expectedLine: string, done: boolean]
  openConflicts: []
  openWiki: []
  openSettings: []
  wikiSuggested: []
  createConflict: []
}>()

const selectedWikiTarget = ref('')
const sendingWiki = ref(false)
const wikiFeedback = ref('')
const wikiError = ref<UiError | null>(null)
const syncFeedback = shallowRef<
  { state: 'queuing' } | { state: 'queued'; message: string } | { state: 'failed'; error: UiError }
>()

const wikiPaths = computed(() =>
  props.wikiTargets.filter((m) => m.kind === 'wiki').map((m) => m.path),
)

watch(
  () => [props.detail.id, wikiPaths.value] as const,
  ([, paths]) => {
    wikiFeedback.value = ''
    wikiError.value = null
    if (!paths.includes(selectedWikiTarget.value)) {
      selectedWikiTarget.value = paths[0] ?? ''
    }
  },
  { immediate: true },
)

const statusBadge = computed(() => {
  const s = props.detail.organizeStatus
  if (props.organizing || s === 'running' || s === 'queued') {
    return { label: s === 'queued' ? '排队中…' : '正在整理…', tone: 'badge-warn' }
  }
  if (s === 'done') return { label: '已整理', tone: 'badge-success' }
  if (s === 'failed') return { label: '整理失败', tone: 'badge-error' }
  return { label: '未整理', tone: 'badge-muted' }
})

const syncTones: Record<MemorySyncStatus, string> = {
  pending: 'badge-muted',
  synced: 'badge-success',
  failed: 'badge-error',
}
const resourceSyncLabels: Record<MemorySyncStatus, string> = {
  pending: '待同步',
  synced: '已同步',
  failed: '同步失败',
}
const memorySyncLabels: Record<MemorySyncStatus, string> = {
  pending: '待提交',
  synced: '已提交',
  failed: '提交失败',
}
const syncRoutes = computed(() => props.memoryConfig?.enabled ? [
  {
    name: 'OV 原文',
    enabled: props.memoryConfig.syncResources,
    status: props.detail.ovResourceStatus,
    labels: resourceSyncLabels,
    uri: props.detail.ovResourceUri,
  },
  {
    name: 'OV 记忆',
    enabled: props.memoryConfig.commitMemories,
    status: props.detail.ovMemoryStatus,
    labels: memorySyncLabels,
  },
] : [])
const canSync = computed(() => syncRoutes.value.some((route) => route.enabled && route.status !== 'synced'))
const syncFailed = computed(() => syncRoutes.value.some((route) => route.enabled && route.status === 'failed'))

watch([() => props.session, () => props.detail.locator], () => {
  syncFeedback.value = undefined
})

async function retryMemorySync() {
  if (!canSync.value || syncFeedback.value?.state === 'queuing') return
  const request = { state: 'queuing' } as const
  syncFeedback.value = request
  try {
    const count = await api.retryMemorySync(props.session, props.detail.locator)
    if (syncFeedback.value === request) {
      syncFeedback.value = { state: 'queued', message: count ? `已将 ${count} 个同步任务加入队列。` : '没有新增同步任务。' }
    }
  } catch (error) {
    if (syncFeedback.value === request) syncFeedback.value = { state: 'failed', error: parseAppError(error) }
  }
}

const openConflictsCount = computed(
  () => props.detail.conflicts.filter((c) => c.status === 'open').length,
)

function expectedLineFor(lineNum: number | null, text: string): string {
  if (!lineNum) return ''
  const lines = props.detail.body.replaceAll('\r\n', '\n').split('\n')
  return lines[lineNum - 1] ?? `- [ ] ${text}`
}

async function suggestDecisionToWiki(decision: string) {
  if (!selectedWikiTarget.value || sendingWiki.value) return
  sendingWiki.value = true
  wikiError.value = null
  wikiFeedback.value = ''
  try {
    await api.createWikiSuggestion({
      session: props.session,
      noteId: props.detail.id.startsWith('file:') ? undefined : props.detail.id,
      targetPath: selectedWikiTarget.value,
      content: decision,
      reason: `来自记录「${props.detail.title}」的明确决定`,
    })
    wikiFeedback.value = `已生成到 ${selectedWikiTarget.value} 的变更建议。`
    emit('wikiSuggested')
  } catch (err) {
    wikiError.value = parseAppError(err)
  } finally {
    sendingWiki.value = false
  }
}
</script>

<template>
  <section class="detail-card inline-organize" aria-label="内联后台整理">
    <header class="m2-card-header">
      <div class="actions">
        <h3>AI 内联整理</h3>
        <span class="badge" :class="statusBadge.tone">{{ statusBadge.label }}</span>
        <span
          v-for="route in syncRoutes"
          :key="route.name"
          class="badge"
          :class="route.enabled ? syncTones[route.status] : 'badge-muted'"
          :title="route.enabled ? route.uri ?? undefined : undefined"
        >{{ route.name }} · {{ route.enabled ? route.labels[route.status] : '已关闭' }}</span>
        <button
          v-if="detail.conflicts.length"
          type="button"
          class="badge"
          :class="openConflictsCount ? 'badge-warn' : 'badge-accent'"
          @click="emit('openConflicts')"
        >
          ⚡ 冲突 ({{ openConflictsCount || detail.conflicts.length }})
        </button>
        <button
          v-if="detail.wikiSuggestions.length"
          type="button"
          class="badge badge-accent"
          @click="emit('openWiki')"
        >
          Wiki 建议 ({{ detail.wikiSuggestions.length }})
        </button>
      </div>
      <div class="actions">
        <button
          v-if="canSync"
          type="button"
          class="compact-btn"
          :disabled="syncFeedback?.state === 'queuing'"
          @click="retryMemorySync"
        >{{ syncFeedback?.state === 'queuing' ? '正在入队…' : syncFailed ? '重试 OV 同步' : '同步到 OV' }}</button>
        <button type="button" class="quiet compact-btn" @click="emit('createConflict')">+ 标记冲突</button>
        <button v-if="!modelReady" type="button" class="compact-btn" @click="emit('openSettings')">配置模型</button>
        <button
          v-else
          type="button"
          class="compact-btn"
          :disabled="organizing || detail.organizeStatus === 'running'"
          @click="emit('triggerOrganize')"
        >
          {{ detail.organizeStatus === 'failed' ? '重试整理' : detail.organize ? '重新整理' : '立即整理' }}
        </button>
      </div>
    </header>

    <p v-if="syncFailed && detail.ovError" class="hint">{{ detail.ovError }}</p>
    <p v-if="syncFeedback?.state === 'queued'" class="hint" role="status">{{ syncFeedback.message }}</p>
    <ErrorNotice v-else-if="syncFeedback?.state === 'failed'" :error="syncFeedback.error" />
    <p v-if="detail.organizeError" class="notice error compact-notice" role="alert">
      后台整理未完成：{{ detail.organizeError }}
    </p>
    <p v-if="detail.organize?.sourceChanged" class="notice compact-notice" role="status">
      原文件在整理后有微调，以下保留已有整理结果供核对。
    </p>

    <template v-if="detail.organize">
      <div class="organize-block">
        <h4>摘要</h4>
        <p class="organize-summary">{{ detail.organize.summary }}</p>
      </div>

      <div v-if="detail.organize.decisions.length" class="organize-block">
        <div class="m2-card-header">
          <h4>明确决定</h4>
          <label v-if="wikiPaths.length" class="m2-field m2-inline-field compact-select">
            <span>Wiki 目标</span>
            <select v-model="selectedWikiTarget">
              <option v-for="path in wikiPaths" :key="path" :value="path">{{ path }}</option>
            </select>
          </label>
        </div>
        <ul class="decision-list">
          <li v-for="(decision, idx) in detail.organize.decisions" :key="idx" class="decision-item">
            <span>{{ decision }}</span>
            <button
              v-if="wikiPaths.length"
              type="button"
              class="quiet compact-btn"
              :disabled="sendingWiki"
              @click="suggestDecisionToWiki(decision)"
            >
              推送 Wiki 建议
            </button>
          </li>
        </ul>
        <p v-if="wikiFeedback" class="notice success compact-notice" role="status">{{ wikiFeedback }}</p>
        <ErrorNotice :error="wikiError" />
      </div>

      <div v-if="detail.organize.tasks.length" class="organize-block">
        <h4>行动待办</h4>
        <ul class="m2-list">
          <li v-for="task in detail.organize.tasks" :key="task.id" class="inline-task-item">
            <label class="m2-check">
              <input
                type="checkbox"
                :checked="task.done"
                :disabled="taskBusyId === task.id"
                @change="emit('toggleTask', task.id, expectedLineFor(task.line, task.text), ($event.target as HTMLInputElement).checked)"
              >
              <span :class="{ 'task-done-text': task.done }">{{ task.text }}</span>
            </label>
            <span v-if="task.assignee" class="badge badge-muted">{{ task.assignee }}</span>
            <span v-if="task.due" class="badge badge-muted">{{ task.due }}</span>
          </li>
        </ul>
      </div>
    </template>

    <p v-else-if="detail.organizeStatus === 'idle'" class="hint">
      {{ modelReady ? '尚未整理。随手记保存后会自动触发，也可点击右上角「立即整理」。' : '启用模型后，新记录会在后台自动提取摘要、决定、待办与冲突。' }}
    </p>

    <div v-if="detail.conflicts.length || detail.wikiSuggestions.length" class="organize-block related-drawer-links">
      <div v-for="c in detail.conflicts" :key="c.id" class="inline-chip-row">
        <span class="badge" :class="conflictStatusTone[c.status]">{{ conflictStatusLabels[c.status] }}</span>
        <span class="chip-topic">{{ c.topic }}</span>
        <button type="button" class="quiet compact-btn" @click="emit('openConflicts')">处理 Diff</button>
      </div>
      <div v-for="w in detail.wikiSuggestions" :key="w.id" class="inline-chip-row">
        <span class="badge" :class="wikiStatusTone[w.status]">{{ wikiStatusLabels[w.status] }}</span>
        <code class="chip-topic">{{ w.targetPath }}</code>
        <button type="button" class="quiet compact-btn" @click="emit('openWiki')">审阅变更</button>
      </div>
    </div>
  </section>
</template>
