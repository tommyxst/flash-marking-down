<script setup lang="ts">
import { computed, defineAsyncComponent, ref, watch } from 'vue'
import { api } from '../api/client'
import { parseAppError, type UiError } from '../api/errors'
import ErrorNotice from './ErrorNotice.vue'
import M2Organize from '../M2Organize.vue'
import UiIcon from '../UiIcon.vue'
import { citationKindLabels, useSourcePresentation, type DocumentMapping, type MemoryConfig, type MemoryPreview, type NoteDetail } from '../m2'

const MarkdownPreview = defineAsyncComponent(() => import('../MarkdownPreview.vue'))

const props = defineProps<{
  session?: string
  detail?: NoteDetail
  loading: boolean
  error: UiError | null
  opening: boolean
  desktop: boolean
  modelReady: boolean
  organizing: boolean
  memoryConfig: MemoryConfig | null
  memoryPreview?: MemoryPreview
  wikiTargets: DocumentMapping[]
  taskBusyId: string
}>()

const emit = defineEmits<{
  triggerOrganize: [idOrPath: string]
  toggleTask: [path: string, taskId: string, expectedLine: string, done: boolean]
  openConflicts: []
  openWiki: []
  openSettings: []
  wikiSuggested: []
  createConflictFromNote: [detail: NoteDetail]
  returnToNote: []
  retryMemoryPreview: []
}>()

const { showSourcePath, sourceMarkdown } = useSourcePresentation()
const dateLabel = new Intl.DateTimeFormat('zh-CN', { dateStyle: 'full' })
const timeLabel = new Intl.DateTimeFormat('zh-CN', { timeStyle: 'short', hour12: false })

const rootEl = ref<HTMLElement>()
const detailViewMode = ref<'preview' | 'source'>('preview')
const copyStatus = ref('')
const openFileError = ref<UiError | null>(null)

const displayTitle = computed(() => props.detail?.title || '未命名记录')

watch(() => props.detail?.locator, () => {
  copyStatus.value = ''
  openFileError.value = null
})

async function openExternalFile(mode: 'open' | 'reveal') {
  if (!props.session || !props.detail) return
  openFileError.value = null
  try {
    await api.openWorkspaceFile(props.session, props.detail.path, mode)
  } catch (err) {
    openFileError.value = parseAppError(err)
  }
}

async function copyMarkdown() {
  if (!props.detail) return
  copyStatus.value = ''
  try {
    await navigator.clipboard.writeText(sourceMarkdown(props.detail.body))
    copyStatus.value = '已复制 Markdown'
  } catch {
    copyStatus.value = '无法访问剪贴板，请选中正文后手动复制。'
  }
}

defineExpose({
  focus: () => rootEl.value?.focus(),
})
</script>

<template>
  <section ref="rootEl" class="pane pane-center" tabindex="-1" :aria-label="memoryPreview ? 'OpenViking 引用' : '记录详情与内联整理'">
    <div v-if="memoryPreview" :key="memoryPreview.uri" class="pane-scroll">
      <article class="detail-card">
        <header class="m2-card-header">
          <h2>{{ memoryPreview.title || 'OpenViking 引用' }}</h2>
          <button type="button" class="quiet compact-btn" @click="emit('returnToNote')">返回本地记录</button>
        </header>
        <div class="detail-meta">
          <span class="badge badge-accent">{{ citationKindLabels[memoryPreview.kind] }}</span>
          <code class="inline-path">{{ memoryPreview.uri }}</code>
        </div>
        <p v-if="memoryPreview.state === 'opening'" class="hint" role="status">正在读取 OpenViking 引用…</p>
        <template v-else-if="memoryPreview.state === 'failed'">
          <ErrorNotice :error="memoryPreview.error" prefix="无法打开 OpenViking 引用：" />
          <div class="actions">
            <button type="button" class="compact-btn" @click="emit('retryMemoryPreview')">重试读取</button>
          </div>
        </template>
        <MarkdownPreview v-else :source="memoryPreview.content" />
      </article>
    </div>
    <div v-show="!memoryPreview" class="pane-scroll">
      <p v-if="loading && !detail" class="empty-panel compact" role="status">正在读取原文与整理结果…</p>
      <ErrorNotice v-else-if="error && !detail" :error="error" prefix="无法打开记录：" />

      <div v-else-if="detail" class="center-stack">
        <article class="detail-card">
          <header class="m2-card-header">
            <div class="detail-title-group">
              <h2>{{ displayTitle }}</h2>
              <div class="detail-meta">
                <time v-if="detail.capturedAt" :datetime="detail.capturedAt">
                  {{ dateLabel.format(new Date(detail.capturedAt)) }} {{ timeLabel.format(new Date(detail.capturedAt)) }}
                </time>
                <span v-else class="hint">记录日期未知</span>
                <code v-if="showSourcePath(detail.path, detail.isCapture)" class="inline-path">{{ detail.path }}</code>
              </div>
            </div>
            <div class="actions">
              <div class="view-switch" role="group" aria-label="正文显示方式">
                <button type="button" :aria-pressed="detailViewMode === 'preview'" @click="detailViewMode = 'preview'">预览</button>
                <button type="button" :aria-pressed="detailViewMode === 'source'" @click="detailViewMode = 'source'">源码</button>
              </div>
              <button type="button" class="quiet compact-btn" @click="copyMarkdown">
                <UiIcon name="copy" />复制
              </button>
              <button v-if="desktop" type="button" class="quiet compact-btn" :disabled="opening" @click="openExternalFile('open')">
                <UiIcon name="external" />外部打开
              </button>
              <button v-if="desktop" type="button" class="quiet compact-btn" :disabled="opening" @click="openExternalFile('reveal')">
                <UiIcon name="folder" />定位
              </button>
            </div>
          </header>

          <ErrorNotice :error="openFileError" />
          <p v-if="copyStatus" class="notice success compact-notice" role="status">{{ copyStatus }}</p>

          <MarkdownPreview
            v-if="detailViewMode === 'preview'"
            :source="sourceMarkdown(detail.body)"
            :session="session"
            :base-path="detail.path"
          />
          <pre v-else class="source-pre" tabindex="0">{{ sourceMarkdown(detail.body) }}</pre>
        </article>

        <M2Organize
          v-if="session"
          :session="session"
          :detail="detail"
          :model-ready="modelReady"
          :organizing="organizing"
          :memory-config="memoryConfig"
          :wiki-targets="wikiTargets"
          :task-busy-id="taskBusyId"
          @trigger-organize="emit('triggerOrganize', detail.locator)"
          @toggle-task="(taskId, expectedLine, done) => emit('toggleTask', detail!.locator, taskId, expectedLine, done)"
          @open-conflicts="emit('openConflicts')"
          @open-wiki="emit('openWiki')"
          @open-settings="emit('openSettings')"
          @wiki-suggested="emit('wikiSuggested')"
          @create-conflict="emit('createConflictFromNote', detail)"
        />
      </div>

      <div v-else class="empty-panel">
        <UiIcon name="file" />
        <h2>选择一条记录</h2>
        <p class="hint">在左栏点击任意随手记或搜索命中项，可在此处阅读原文、查看后台整理结果并直接勾选待办。</p>
      </div>
    </div>
  </section>
</template>
