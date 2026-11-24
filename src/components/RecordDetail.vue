<script setup lang="ts">
import { computed, defineAsyncComponent, ref, watch } from 'vue'
import { api } from '../api/client'
import { parseAppError, type UiError } from '../api/errors'
import ErrorNotice from './ErrorNotice.vue'
import UiIcon from '../UiIcon.vue'
import { recordTitle, useSourcePresentation, type RecordEntry, type RecordSelection, type SourceView } from '../m2'

const MarkdownPreview = defineAsyncComponent(() => import('../MarkdownPreview.vue'))

const props = defineProps<{
  session?: string
  record?: RecordEntry
  opening: boolean
  searchActive: boolean
  desktop: boolean
  disagreementSlotsFull: boolean
}>()

const emit = defineEmits<{
  organize: [selection: RecordSelection]
  addDisagreementSource: [selection: RecordSelection]
}>()

const { showSourcePath, sourceMarkdown } = useSourcePresentation()
const dateLabel = new Intl.DateTimeFormat('zh-CN', { dateStyle: 'full' })
const timeLabel = new Intl.DateTimeFormat('zh-CN', { timeStyle: 'short', hour12: false })

const rootEl = ref<HTMLElement>()
const detailBodyRef = ref<HTMLElement>()
const detailViewMode = ref<'preview' | 'source'>('preview')
const source = ref<SourceView>()
const copyStatus = ref('')
const openFileError = ref<UiError | null>(null)
let sourceRequest = 0

const detailPreviewLabel = computed(() => props.searchActive ? '索引预览' : '保存时的 Markdown')
const detailBodyText = computed(() => {
  const record = props.record
  if (!record) return ''
  return source.value?.state === 'opened' ? source.value.body : record.text
})

watch([
  () => props.session,
  () => props.searchActive,
  () => props.record?.path,
  () => props.record?.contentHash,
], resetSource, { flush: 'sync' })

function resetSource() {
  sourceRequest += 1
  source.value = undefined
  copyStatus.value = ''
  openFileError.value = null
}

async function openRecordSource() {
  const session = props.session
  const record = props.record
  if (!session || !record || source.value?.state === 'opening') return
  resetSource()
  const request = sourceRequest
  source.value = { state: 'opening' }
  try {
    const opened = await api.openRecordSource(session, record.path, record.contentHash)
    if (request === sourceRequest) {
      source.value = {
        state: 'opened',
        changed: opened.state === 'changed',
        contentHash: opened.contentHash,
        body: opened.body,
        isCapture: opened.isCapture,
      }
    }
  } catch (error) {
    if (request === sourceRequest) source.value = { state: 'failed', error: parseAppError(error) }
  }
}

async function openExternalFile(mode: 'open' | 'reveal') {
  const session = props.session
  const record = props.record
  if (!session || !record) return
  openFileError.value = null
  try {
    await api.openWorkspaceFile(session, record.path, mode)
  } catch (error) {
    openFileError.value = parseAppError(error)
  }
}

async function copyMarkdown(text: string) {
  const request = sourceRequest
  copyStatus.value = ''
  try {
    await navigator.clipboard.writeText(text)
    if (request === sourceRequest) copyStatus.value = '已复制 Markdown'
  } catch {
    if (request === sourceRequest) copyStatus.value = '无法访问剪贴板，请选中正文后手动复制。'
  }
}

function selectedDetailExcerpt(): string | undefined {
  const container = detailBodyRef.value
  const selection = window.getSelection()
  if (!container || !selection?.rangeCount || selection.isCollapsed) return undefined
  if (!selection.getRangeAt(0).intersectsNode(container)) return undefined
  const text = selection.toString().trim()
  return text || undefined
}

function currentSelection(): RecordSelection | undefined {
  const record = props.record
  if (!record) return undefined
  return {
    path: record.path,
    title: recordTitle(record),
    contentHash: source.value?.state === 'opened' ? source.value.contentHash : record.contentHash,
    isCapture: source.value?.state === 'opened' ? source.value.isCapture : record.isCapture,
  }
}

function onOrganize() {
  const selection = currentSelection()
  if (selection) emit('organize', selection)
}

function onAddDisagreementSource() {
  const selection = currentSelection()
  if (!selection) return
  const excerpt = selectedDetailExcerpt()
  emit('addDisagreementSource', excerpt ? { ...selection, excerpt } : selection)
}

defineExpose({
  focus: () => rootEl.value?.focus(),
})
</script>

<template>
  <aside ref="rootEl" class="record-detail" tabindex="-1" aria-labelledby="detail-heading">
    <header class="section-bar"><UiIcon name="file" /><h2 id="detail-heading">记录详情</h2></header>
    <div v-if="record" class="detail-card">
      <h3>{{ recordTitle(record) }}</h3>
      <time v-if="record.capturedAt" :datetime="record.capturedAt">
        {{ dateLabel.format(new Date(record.capturedAt)) }} {{ timeLabel.format(new Date(record.capturedAt)) }}
      </time>
      <p v-else class="hint">记录日期未知</p>
      <div v-if="showSourcePath(record.path, record.isCapture)" class="path-bar"><UiIcon name="folder" /><code>{{ record.path }}</code></div>
      <div class="actions">
        <button
          type="button"
          :disabled="opening"
          :aria-disabled="source?.state === 'opening' || undefined"
          @click="source?.state === 'opened' ? resetSource() : openRecordSource()"
        >
          {{ source?.state === 'opened' ? (searchActive ? '返回索引预览' : '返回保存快照') : source?.state === 'failed' ? '重试打开原文' : '打开原文' }}
        </button>
        <button type="button" :disabled="opening" @click="onOrganize">整理这条记录</button>
        <button
          type="button"
          class="quiet"
          :disabled="opening"
          title="可先在下方正文划选片段后再点击，将自动带入分歧依据"
          @click="onAddDisagreementSource"
        >
          {{ disagreementSlotsFull ? '查看已选分歧依据' : '用作分歧依据' }}
        </button>
        <button v-if="desktop" type="button" class="quiet" :disabled="opening" @click="openExternalFile('open')">
          <UiIcon name="external" />外部打开
        </button>
        <button v-if="desktop" type="button" class="quiet" :disabled="opening" @click="openExternalFile('reveal')">
          <UiIcon name="folder" />定位文件
        </button>
      </div>
      <ErrorNotice :error="openFileError" />
      <p v-if="source?.state === 'opening'" class="notice" role="status">正在读取原文…</p>
      <ErrorNotice v-else-if="source?.state === 'failed'" :error="source.error" prefix="无法打开原文。" />
      <p v-else-if="source?.state === 'opened' && source.changed" class="notice" role="status">
        {{ searchActive ? '原文自索引后已变化。' : '原文自保存后已变化。' }}以下为当前文件内容。
      </p>
      <div class="detail-toolbar actions">
        <span>{{ source?.state === 'opened' ? '当前原文' : detailPreviewLabel }}</span>
        <div class="actions">
          <div class="view-switch" role="group" aria-label="正文显示方式">
            <button type="button" :aria-pressed="detailViewMode === 'preview'" @click="detailViewMode = 'preview'">渲染预览</button>
            <button type="button" :aria-pressed="detailViewMode === 'source'" @click="detailViewMode = 'source'">源码</button>
          </div>
          <button v-if="source?.state === 'opened' || !searchActive" type="button" class="quiet" @click="copyMarkdown(detailBodyText)">复制正文</button>
        </div>
      </div>
      <div ref="detailBodyRef">
        <MarkdownPreview v-if="detailViewMode === 'preview'" :source="sourceMarkdown(detailBodyText)" />
        <pre v-else tabindex="0">{{ sourceMarkdown(detailBodyText) }}</pre>
      </div>
      <p v-if="copyStatus" class="notice" role="status">{{ copyStatus }}</p>
    </div>
    <p v-else class="empty-panel compact">选择记录后可查看详情。</p>
  </aside>
</template>
