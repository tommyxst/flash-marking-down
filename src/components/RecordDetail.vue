<script setup lang="ts">
import { computed, defineAsyncComponent, nextTick, ref, watch } from 'vue'
import { api } from '../api/client'
import { parseAppError, type UiError } from '../api/errors'
import CaptureEditor from '../CaptureEditor.vue'
import ErrorNotice from './ErrorNotice.vue'
import M2Organize from '../M2Organize.vue'
import UiIcon from '../UiIcon.vue'
import {
  citationKindLabels,
  useSourcePresentation,
  wikiStatusLabels,
  wikiStatusTone,
  type DocumentMapping,
  type MemoryConfig,
  type MemoryPreview,
  type NoteDetail,
} from '../m2'

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
  wikiTargetBusy: boolean
  wikiTargetError: UiError | null
  taskBusyId: string
}>()

const emit = defineEmits<{
  triggerOrganize: [idOrPath: string]
  toggleTask: [path: string, taskId: string, expectedLine: string, done: boolean]
  openConflicts: []
  openWiki: [suggestionId?: string]
  openSettings: []
  wikiSuggested: []
  toggleWikiTarget: [path: string, mapped: boolean]
  createConflictFromNote: [detail: NoteDetail]
  returnToNote: []
  retryMemoryPreview: []
  noteSaved: [detail: NoteDetail]
  reloadDetail: []
}>()

const { showSourcePath, sourceMarkdown } = useSourcePresentation()
const dateLabel = new Intl.DateTimeFormat('zh-CN', { dateStyle: 'full' })
const timeLabel = new Intl.DateTimeFormat('zh-CN', { timeStyle: 'short', hour12: false })

const rootEl = ref<HTMLElement>()
const previewButton = ref<HTMLButtonElement>()
const detailViewMode = ref<'preview' | 'edit' | 'source'>('preview')
const copyStatus = ref('')
const openFileError = ref<UiError | null>(null)
const draftBody = ref('')
const editBaseHash = ref('')
const savingNote = ref(false)
const saveError = ref<UiError | null>(null)
const editComposing = ref(false)
const saveFeedback = ref('')
const reloadDraft = ref<string>()
const reloadError = ref<UiError | null>(null)
let detailVersion = 0

const displayTitle = computed(() => {
  const detail = props.detail
  if (detail?.title) return detail.title
  if (!detail || detail.isCapture) return '未命名记录'
  return detail.path.split(/[\\/]/).pop()?.replace(/\.md$/i, '') || '未命名文档'
})
const documentKind = computed(() =>
  props.wikiTargets.find((target) => target.path === props.detail?.path)?.kind,
)
const wikiMapped = computed(() => documentKind.value === 'wiki')
const externalChanged = computed(() =>
  detailViewMode.value === 'edit' && !savingNote.value && Boolean(props.detail && props.detail.contentHash !== editBaseHash.value),
)
const reloadingNote = computed(() => reloadDraft.value !== undefined)
const editBusy = computed(() => savingNote.value || editComposing.value || reloadingNote.value)
const saveLabel = computed(() => {
  if (savingNote.value) return '保存中…'
  if (!props.detail?.isCapture) return '保存文档'
  return props.modelReady ? '保存并整理' : '保存修改'
})

function syncDraft(detail?: NoteDetail) {
  draftBody.value = detail?.body ?? ''
  editBaseHash.value = detail?.contentHash ?? ''
}

function resetEditor(detail?: NoteDetail) {
  syncDraft(detail)
  saveError.value = null
  reloadError.value = null
  reloadDraft.value = undefined
  editComposing.value = false
  copyStatus.value = ''
}

watch([() => props.session, () => props.detail?.locator], () => {
  detailVersion++
  detailViewMode.value = 'preview'
  savingNote.value = false
  saveFeedback.value = ''
  openFileError.value = null
  resetEditor(props.detail)
}, { immediate: true })

watch(() => props.detail, (detail) => {
  if (detailViewMode.value !== 'edit') syncDraft(detail)
})

watch([() => props.loading, () => props.detail, () => props.error], ([loading, detail, error]) => {
  const originalDraft = reloadDraft.value
  if (originalDraft === undefined || loading) return
  reloadDraft.value = undefined
  if (error) {
    reloadError.value = error
  } else if (detail && !editComposing.value && draftBody.value === originalDraft) {
    resetEditor(detail)
  }
})

function editNote() {
  if (!props.session || !props.detail || props.memoryPreview || detailViewMode.value === 'edit') return
  syncDraft(props.detail)
  saveFeedback.value = ''
  copyStatus.value = ''
  detailViewMode.value = 'edit'
}

function showPreview() {
  detailViewMode.value = 'preview'
  void nextTick(() => {
    const focused = document.activeElement
    if (!props.memoryPreview && (focused === document.body || rootEl.value?.contains(focused))) {
      previewButton.value?.focus()
    }
  })
}

function cancelEdit() {
  if (savingNote.value || editComposing.value) return
  resetEditor(props.detail)
  showPreview()
}

function reloadLatest() {
  if (editBusy.value) return
  reloadError.value = null
  reloadDraft.value = draftBody.value
  emit('reloadDetail')
}

async function saveNote() {
  const { session, detail } = props
  if (!session || !detail || detailViewMode.value !== 'edit' || editBusy.value) return

  const version = detailVersion
  const expectedHash = editBaseHash.value || detail.contentHash
  savingNote.value = true
  saveError.value = null
  reloadError.value = null
  try {
    const updated = await api.updateNote({
      session,
      idOrPath: detail.locator,
      expectedHash,
      body: draftBody.value,
      reorganize: detail.isCapture,
    })
    if (version !== detailVersion) return
    const current = props.detail
    // A background read may already have returned a later revision.
    const newer = current && current.contentHash !== expectedHash && current.contentHash !== updated.contentHash
    resetEditor(newer ? current : updated)
    showPreview()
    saveFeedback.value = detail.isCapture ? '修改已保存' : 'Wiki 文档已保存'
    if (newer) emit('reloadDetail')
    else emit('noteSaved', updated)
  } catch (error) {
    if (version === detailVersion) saveError.value = parseAppError(error)
  } finally {
    if (version === detailVersion) savingNote.value = false
  }
}

function handleEditorKey(event: KeyboardEvent) {
  if (!(event.ctrlKey || event.metaKey) || event.altKey || event.shiftKey ||
      editComposing.value || event.isComposing || event.keyCode === 229) return
  if (event.key !== 'Enter' && event.key.toLowerCase() !== 's') return
  event.preventDefault()
  // CodeMirror binds Mod-Enter to insert a blank line.
  event.stopPropagation()
  if (!event.repeat) void saveNote()
}

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
  const version = detailVersion
  const text = detailViewMode.value === 'edit' ? draftBody.value : sourceMarkdown(draftBody.value)
  copyStatus.value = ''
  try {
    await navigator.clipboard.writeText(text)
    if (version === detailVersion) copyStatus.value = '已复制 Markdown'
  } catch {
    if (version === detailVersion) copyStatus.value = '无法访问剪贴板，请选中正文后手动复制。'
  }
}

defineExpose({
  focus: () => rootEl.value?.focus(),
  startEditing: editNote,
})
</script>

<template>
  <section
    ref="rootEl"
    class="pane pane-center"
    tabindex="-1"
    :aria-label="memoryPreview ? 'OpenViking 引用' : detail && !detail.isCapture ? '文档详情' : '记录详情与内联整理'"
  >
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
      <p v-if="loading && !detail" class="empty-panel compact" role="status">正在读取原文…</p>
      <ErrorNotice v-else-if="error && !detail" :error="error" prefix="无法打开原文。" />

      <div v-else-if="detail" class="center-stack">
        <article class="detail-card">
          <header class="m2-card-header">
            <div class="detail-title-group">
              <h2>{{ displayTitle }}</h2>
              <div class="detail-meta">
                <span v-if="!detail.isCapture" class="badge" :class="wikiMapped ? 'badge-accent' : 'badge-muted'" role="status">
                  {{ wikiMapped ? '已映射 Wiki' : '工作区文档' }}
                </span>
                <time v-if="detail.capturedAt" :datetime="detail.capturedAt">
                  {{ dateLabel.format(new Date(detail.capturedAt)) }} {{ timeLabel.format(new Date(detail.capturedAt)) }}
                </time>
                <span v-else-if="detail.isCapture" class="hint">记录日期未知</span>
                <code v-if="showSourcePath(detail.path, detail.isCapture)" class="inline-path">{{ detail.path }}</code>
              </div>
            </div>
            <div class="actions">
              <div class="view-switch" role="group" aria-label="正文模式">
                <button ref="previewButton" type="button" :aria-pressed="detailViewMode === 'preview'" :disabled="detailViewMode === 'edit'" @click="detailViewMode = 'preview'">预览</button>
                <button type="button" :aria-pressed="detailViewMode === 'edit'" :disabled="!session" @click="editNote">编辑</button>
                <button type="button" :aria-pressed="detailViewMode === 'source'" :disabled="detailViewMode === 'edit'" @click="detailViewMode = 'source'">源码</button>
              </div>
              <button v-if="detailViewMode !== 'edit'" type="button" class="quiet compact-btn" @click="copyMarkdown">
                <UiIcon name="copy" />复制
              </button>
              <button v-if="desktop" type="button" class="quiet compact-btn" :disabled="opening" @click="openExternalFile('open')">
                <UiIcon name="external" />外部打开
              </button>
              <button v-if="desktop" type="button" class="quiet compact-btn" :disabled="opening" @click="openExternalFile('reveal')">
                <UiIcon name="folder" />定位
              </button>
              <button
                v-if="session && !detail.isCapture && documentKind !== 'tasks'"
                type="button"
                class="compact-btn"
                :disabled="wikiTargetBusy"
                @click="emit('toggleWikiTarget', detail.path, wikiMapped)"
              >{{ wikiTargetBusy ? '正在更新…' : wikiMapped ? '解除映射' : '映射为 Wiki 目标' }}</button>
            </div>
          </header>

          <ErrorNotice :error="openFileError" />
          <ErrorNotice :error="wikiTargetError" prefix="无法更新 Wiki 映射。" />
          <ErrorNotice :error="saveError" prefix="保存失败：" />
          <ErrorNotice :error="reloadError" prefix="重新加载失败。" />
          <p v-if="externalChanged" class="notice detail-external-change" role="status">原文在编辑期间已更新，当前草稿已保留。</p>
          <p v-if="copyStatus" class="notice success compact-notice" role="status">{{ copyStatus }}</p>
          <p v-if="saveFeedback" class="notice success" role="status">{{ saveFeedback }}</p>

          <MarkdownPreview
            v-if="detailViewMode === 'preview'"
            :source="sourceMarkdown(draftBody)"
            :session="session"
            :base-path="detail.path"
          />
          <div v-else-if="detailViewMode === 'edit'" class="detail-editor-panel">
            <div class="detail-editor-wrap" @keydown.capture="handleEditorKey">
              <CaptureEditor
                v-model="draftBody"
                editor-id="detail-note-editor"
                :readonly="savingNote"
                :placeholder-text="detail.isCapture ? '使用 Markdown 编辑笔记内容…' : '使用 Markdown 编写 Wiki 文档…'"
                autofocus
                @composition-change="editComposing = $event"
              />
            </div>
            <p v-if="saveError || externalChanged || reloadError" class="hint">重新加载会替换当前草稿。可先复制正文。</p>
            <div class="detail-editor-actions actions">
              <span class="hint"><kbd>Ctrl+S</kbd> 或 <kbd>Ctrl+Enter</kbd> 保存</span>
              <button
                v-if="saveError || externalChanged || reloadError || reloadingNote"
                type="button"
                class="compact-btn"
                :disabled="editBusy"
                @click="reloadLatest"
              >{{ reloadingNote ? '正在加载…' : '重新加载最新内容' }}</button>
              <button
                v-if="saveError || externalChanged || reloadError"
                type="button"
                class="quiet compact-btn"
                :disabled="editComposing"
                @click="copyMarkdown"
              >复制当前编辑内容</button>
              <button type="button" class="quiet compact-btn" :disabled="savingNote || editComposing" @click="cancelEdit">取消</button>
              <button type="button" class="primary compact-btn" :disabled="editBusy" @click="saveNote">{{ saveLabel }}</button>
            </div>
          </div>
          <pre v-else class="source-pre" tabindex="0">{{ sourceMarkdown(draftBody) }}</pre>
        </article>

        <M2Organize
          v-if="session && detail.isCapture"
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
        <section v-else-if="session" class="detail-card" aria-label="Wiki 页面关联变更">
          <h3>Wiki 页面关联变更</h3>
          <p v-if="documentKind === 'tasks'" class="hint">此文档用作待办清单。</p>
          <p v-else-if="!wikiMapped" class="hint">映射为 Wiki 目标后，可接收随手记的整理建议。</p>
          <ul v-if="detail.wikiSuggestions.length" class="m2-list">
            <li v-for="suggestion in detail.wikiSuggestions" :key="suggestion.id" class="m2-card-header">
              <div class="actions">
                <span class="badge" :class="wikiStatusTone[suggestion.status]">{{ wikiStatusLabels[suggestion.status] }}</span>
                <p v-if="suggestion.reason" class="hint">{{ suggestion.reason }}</p>
              </div>
              <button type="button" class="quiet compact-btn" @click="emit('openWiki', suggestion.id)">审阅变更 Diff</button>
            </li>
          </ul>
          <p v-else-if="wikiMapped" class="hint">暂无关联的 Wiki 变更。</p>
        </section>
      </div>

      <div v-else class="empty-panel">
        <UiIcon name="file" />
        <h2>选择记录或 Wiki 文档</h2>
        <p class="hint">在左栏选择一项，即可阅读和编辑 Markdown 正文。</p>
      </div>
    </div>
  </section>
</template>
