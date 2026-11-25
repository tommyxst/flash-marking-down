<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { api } from './api/client'
import { parseAppError, type UiError } from './api/errors'
import ErrorNotice from './components/ErrorNotice.vue'
import DiffViewer from './DiffViewer.vue'
import {
  conflictStatusLabels,
  conflictStatusTone,
  type ConflictItem,
  type ConflictStatus,
  type DocumentMapping,
} from './m2'

const props = defineProps<{
  session: string
  conflicts: ConflictItem[]
  wikiTargets: DocumentMapping[]
  prefill?: { topic: string; incomingPath: string; incomingText: string } | null
}>()

const emit = defineEmits<{
  updated: []
  openNote: [path: string]
  clearPrefill: []
}>()

const creating = ref(false)
const topic = ref('')
const currentPath = ref('')
const currentText = ref('')
const incomingPath = ref('')
const incomingText = ref('')

const notes = ref<Record<string, string>>({})
const targetWiki = ref<Record<string, string>>({})
const busyId = ref('')
const savingNew = ref(false)
const error = ref<UiError | null>(null)
const feedback = ref('')

const wikiPaths = computed(() =>
  props.wikiTargets.filter((item) => item.kind === 'wiki').map((item) => item.path),
)

watch(
  () => props.prefill,
  (prefill) => {
    if (!prefill) return
    creating.value = true
    topic.value = prefill.topic
    incomingPath.value = prefill.incomingPath
    incomingText.value = prefill.incomingText
  },
  { immediate: true },
)

watch(
  () => props.conflicts,
  (list) => {
    for (const item of list) {
      if (notes.value[item.id] === undefined && item.resolutionNote) {
        notes.value[item.id] = item.resolutionNote
      }
    }
  },
  { immediate: true },
)

async function submitConflict() {
  if (!topic.value.trim() || !currentText.value.trim() || !incomingText.value.trim() || savingNew.value) return
  savingNew.value = true
  error.value = null
  feedback.value = ''
  try {
    await api.createConflict({
      session: props.session,
      topic: topic.value.trim(),
      currentPath: currentPath.value.trim() || undefined,
      currentText: currentText.value.trim(),
      incomingPath: incomingPath.value.trim() || undefined,
      incomingText: incomingText.value.trim(),
    })
    creating.value = false
    topic.value = currentPath.value = currentText.value = incomingPath.value = incomingText.value = ''
    emit('clearPrefill')
    feedback.value = '已记录观点冲突。'
    emit('updated')
  } catch (err) {
    error.value = parseAppError(err)
  } finally {
    savingNew.value = false
  }
}

async function resolve(item: ConflictItem, status: ConflictStatus) {
  if (busyId.value) return
  busyId.value = item.id
  error.value = null
  feedback.value = ''
  try {
    const chosenWiki = status === 'accept_incoming' ? (targetWiki.value[item.id] || undefined) : undefined
    await api.resolveConflict({
      session: props.session,
      id: item.id,
      status,
      resolutionNote: notes.value[item.id]?.trim() || undefined,
      wikiTarget: chosenWiki,
    })
    feedback.value = status === 'accept_incoming' && chosenWiki
      ? `已采纳新记录，并生成 ${chosenWiki} 的 Wiki 更新建议。`
      : `已更新冲突决策：${conflictStatusLabels[status]}。`
    emit('updated')
  } catch (err) {
    error.value = parseAppError(err)
  } finally {
    busyId.value = ''
  }
}
</script>

<template>
  <div class="m2-stack">
    <div class="m2-card-header">
      <span class="hint">按 Git Merge Conflict 对比双边观点并一键三选一决策。</span>
      <button type="button" class="quiet compact-btn" @click="creating = !creating">
        {{ creating ? '收起录入' : '+ 标记冲突' }}
      </button>
    </div>

    <form v-if="creating" class="detail-card m2-form" @submit.prevent="submitConflict">
      <label class="m2-field">冲突主题
        <input v-model="topic" required placeholder="例如：离线缓存策略选型" :disabled="savingNew">
      </label>
      <div class="m2-columns">
        <label class="m2-field">Current（现有观点）
          <textarea v-model="currentText" required rows="3" placeholder="原有记录主张…" :disabled="savingNew" />
        </label>
        <label class="m2-field">Incoming（新记录主张）
          <textarea v-model="incomingText" required rows="3" placeholder="新记录观点…" :disabled="savingNew" />
        </label>
      </div>
      <div class="m2-columns">
        <label class="m2-field">Current 来源路径（可选）
          <input v-model="currentPath" placeholder="如 notes/a.md" :disabled="savingNew">
        </label>
        <label class="m2-field">Incoming 来源路径（可选）
          <input v-model="incomingPath" placeholder="如 2026-11-25.md" :disabled="savingNew">
        </label>
      </div>
      <div class="actions">
        <button type="button" class="quiet" :disabled="savingNew" @click="creating = false; emit('clearPrefill')">取消</button>
        <button type="submit" class="primary" :disabled="savingNew">保存冲突</button>
      </div>
    </form>

    <ErrorNotice :error="error" />
    <p v-if="feedback" class="notice success compact-notice" role="status">{{ feedback }}</p>

    <div v-if="!conflicts.length" class="empty-panel compact">
      <p>当前没有待处理的观点冲突。</p>
    </div>

    <article v-for="item in conflicts" :key="item.id" class="detail-card conflict-card">
      <header class="m2-card-header">
        <h3>{{ item.topic }}</h3>
        <span class="badge" :class="conflictStatusTone[item.status]">{{ conflictStatusLabels[item.status] }}</span>
      </header>

      <div class="git-conflict-box">
        <div class="git-marker git-marker-current">
          <span>&lt;&lt;&lt;&lt;&lt;&lt;&lt; {{ item.currentLabel }}</span>
          <button
            v-if="item.currentPath"
            type="button"
            class="quiet link-btn"
            @click="emit('openNote', item.currentPath)"
          >
            {{ item.currentPath }}
          </button>
        </div>
        <div class="git-marker git-marker-incoming">
          <span>&gt;&gt;&gt;&gt;&gt;&gt;&gt; {{ item.incomingLabel }}</span>
          <button
            v-if="item.incomingPath"
            type="button"
            class="quiet link-btn"
            @click="emit('openNote', item.incomingPath)"
          >
            {{ item.incomingPath }}
          </button>
        </div>
        <DiffViewer :before="item.currentText" :after="item.incomingText" :show-full-comparison="false" />
      </div>

      <label class="m2-field">补充条件或适用备注（可选）
        <input v-model="notes[item.id]" placeholder="例如：弱网走方案 A，内网走方案 B" :disabled="busyId === item.id">
      </label>

      <label v-if="wikiPaths.length" class="m2-field m2-inline-field">
        <span>采纳新记录时联动更新 Wiki</span>
        <select v-model="targetWiki[item.id]" :disabled="busyId === item.id">
          <option value="">不生成 Wiki 建议</option>
          <option v-for="path in wikiPaths" :key="path" :value="path">{{ path }}</option>
        </select>
      </label>

      <div class="actions">
        <button
          type="button"
          :class="{ primary: item.status === 'keep_current' }"
          :disabled="busyId === item.id"
          @click="resolve(item, 'keep_current')"
        >
          Keep Current
        </button>
        <button
          type="button"
          :class="{ primary: item.status === 'accept_incoming' }"
          :disabled="busyId === item.id"
          @click="resolve(item, 'accept_incoming')"
        >
          Accept Incoming
        </button>
        <button
          type="button"
          :class="{ primary: item.status === 'keep_both' }"
          :disabled="busyId === item.id"
          @click="resolve(item, 'keep_both')"
        >
          Keep Both
        </button>
      </div>
    </article>
  </div>
</template>
