<script setup lang="ts">
import { isTauri } from '@tauri-apps/api/core'
import { computed, nextTick, ref, watch } from 'vue'
import { api } from './api/client'
import { parseAppError, type UiError } from './api/errors'
import ErrorNotice from './components/ErrorNotice.vue'
import M2Answers from './M2Answers.vue'
import M2Documents from './M2Documents.vue'
import M2Organize from './M2Organize.vue'
import UiIcon from './UiIcon.vue'
import type { DocumentChange, DocumentMapping, ModelConfig, RecordSelection, WorkspacePage } from './m2'

const props = defineProps<{
  session: string
  page: WorkspacePage
  record?: RecordSelection
  sources: [RecordSelection | null, RecordSelection | null]
  model: ModelConfig | null
  modelReady: boolean
}>()
const emit = defineEmits<{
  navigate: [page: WorkspacePage]
  showRecords: []
  showSettings: []
  removeSource: [index: number]
  clearSources: []
  selectDisagreementSource: [source: RecordSelection]
}>()

const desktop = isTauri()
const pages = {
  organize: { title: '整理建议', icon: 'organize' },
  wiki: { title: 'Wiki 变更', icon: 'file' },
  tasks: { title: '任务清单', icon: 'tasks' },
  answers: { title: '问答与分歧', icon: 'question' },
} as const
const active = computed(() => props.page === 'records' ? undefined : pages[props.page])
const documentMode = computed(() => props.page === 'wiki' || props.page === 'tasks' ? props.page : null)
const heading = ref<HTMLElement>()
const mappings = ref<DocumentMapping[]>([])
const workspaceFiles = ref<string[]>([])
const preview = ref<DocumentChange>()
const mappingPath = ref('')
const mappingMode = ref<'existing' | 'new'>('existing')
const mappingBusy = ref(false)
const pickingFile = ref(false)
const mappingError = ref<UiError | null>(null)
const mappingFeedback = ref('')
const visibleMappings = computed(() => mappings.value.filter((item) => item.kind === documentMode.value))
let mappingsRequest = 0
let filesRequest = 0

watch(() => props.page, (page) => {
  mappingPath.value = mappingFeedback.value = ''
  mappingError.value = null
  mappingMode.value = 'existing'
  if (page !== 'records') void nextTick(() => heading.value?.focus())
  if (page === 'organize' || page === 'wiki' || page === 'tasks') void loadMappings()
  if (page === 'wiki' || page === 'tasks') void loadWorkspaceFiles()
})

async function loadWorkspaceFiles() {
  const request = ++filesRequest
  try {
    const files = await api.listWorkspaceMarkdownFiles(props.session)
    if (request === filesRequest) workspaceFiles.value = files
  } catch {
    // Autocomplete list is optional; manual entry and file picker remain available.
  }
}

async function pickMarkdownFile() {
  const kind = documentMode.value
  if (!kind || pickingFile.value || mappingBusy.value) return
  pickingFile.value = true
  mappingError.value = null
  try {
    const selected = await api.pickWorkspaceMarkdown(props.session, kind)
    if (selected) mappingPath.value = selected
  } catch (failure) {
    mappingError.value = parseAppError(failure)
  } finally {
    pickingFile.value = false
  }
}

async function loadMappings() {
  const request = ++mappingsRequest
  try {
    const results = await api.wikiTargets(props.session)
    if (request === mappingsRequest) mappings.value = results
  } catch (failure) {
    if (request === mappingsRequest) mappingError.value = parseAppError(failure)
  }
}

async function saveMapping() {
  const kind = documentMode.value
  const path = mappingPath.value.trim()
  const mode = mappingMode.value
  if (!kind || !path || mappingBusy.value) return
  mappingBusy.value = true
  mappingError.value = null
  mappingFeedback.value = ''
  try {
    if (kind === 'wiki' && mode === 'new') await api.createWikiTarget(props.session, path)
    else await api.mapWikiTarget(props.session, kind, path)
    if (documentMode.value === kind) {
      mappingPath.value = ''
      mappingFeedback.value = mode === 'new' ? '已新建并映射 Wiki 文件。' : '已更新写入目标。'
    }
    await Promise.all([loadMappings(), loadWorkspaceFiles()])
  } catch (failure) {
    if (documentMode.value === kind) mappingError.value = parseAppError(failure)
  } finally {
    mappingBusy.value = false
  }
}

function review(change: DocumentChange) {
  preview.value = change
  emit('navigate', 'wiki')
}
</script>

<template>
  <div class="m2-workbench">
    <header v-if="active" class="section-bar m2-heading">
      <UiIcon :name="active.icon" />
      <h1 ref="heading" tabindex="-1">{{ active.title }}</h1>
    </header>

    <M2Organize
      v-show="page === 'organize'"
      :session="session" :active="page === 'organize'" :record="record" :model="model" :model-ready="modelReady" :mappings="mappings"
      @show-records="emit('showRecords')" @show-settings="emit('showSettings')" @navigate="emit('navigate', $event)" @review="review"
      @select-disagreement-source="emit('selectDisagreementSource', $event)"
    />

    <template v-if="documentMode">
      <details :key="documentMode" class="detail-card m2-mapping" :open="visibleMappings.length === 0">
        <summary>{{ documentMode === 'wiki' ? 'Wiki 写入目标' : '任务清单文件' }}</summary>
        <ul v-if="visibleMappings.length" class="m2-list">
          <li v-for="item in visibleMappings" :key="item.path"><code>{{ item.path }}</code></li>
        </ul>
        <form class="m2-form" aria-label="设置 Markdown 写入目标" @submit.prevent="saveMapping">
          <label v-if="documentMode === 'wiki'" class="m2-field">目标类型
            <select v-model="mappingMode" :disabled="mappingBusy || pickingFile"><option value="existing">映射已有 Wiki 页</option><option value="new">新建空白 Wiki 页</option></select>
          </label>
          <label class="m2-field">工作区相对路径
            <div class="folder-field">
              <input
                v-model="mappingPath"
                required
                :list="mappingMode === 'existing' ? 'workspace-md-files' : undefined"
                :placeholder="mappingMode === 'new' ? '例如：wiki/主题名.md' : '选择或输入工作区内的 .md 路径…'"
                :disabled="mappingBusy || pickingFile"
                spellcheck="false"
              >
              <button v-if="desktop && mappingMode === 'existing'" type="button" :disabled="mappingBusy || pickingFile" @click="pickMarkdownFile">选择文件</button>
            </div>
          </label>
          <datalist id="workspace-md-files">
            <option v-for="file in workspaceFiles" :key="file" :value="file" />
          </datalist>
          <p class="hint">
            {{ mappingMode === 'new' ? '将在已有文件夹中创建一个新的 Markdown 文件。' : '可直接点击「选择文件」或从下拉提示中选择工作区内已有 Markdown 文件。' }}
          </p>
          <div class="actions">
            <button type="submit" :disabled="mappingBusy || pickingFile || !mappingPath.trim()">
              {{ mappingMode === 'new' ? '创建并映射 Wiki 页' : '映射此文件' }}
            </button>
          </div>
        </form>
      </details>
      <p v-if="mappingFeedback" class="notice success m2-mapping" role="status">{{ mappingFeedback }}</p>
    </template>
    <div v-if="mappingError && (documentMode || page === 'organize')" class="actions">
      <ErrorNotice :error="mappingError" />
      <button type="button" class="quiet" @click="mappingError = null; loadMappings()">刷新写入目标</button>
    </div>
    <M2Documents
      :session="session" :mode="documentMode" :mappings="mappings" :preview="preview"
      @show-organize="emit('navigate', 'organize')"
      @select-disagreement-source="emit('selectDisagreementSource', $event)"
    />

    <M2Answers
      v-show="page === 'answers'"
      :session="session" :active="page === 'answers'" :sources="sources" :model="model" :model-ready="modelReady"
      @show-records="emit('showRecords')" @show-settings="emit('showSettings')"
      @remove-source="emit('removeSource', $event)" @clear-sources="emit('clearSources')"
      @select-disagreement-source="emit('selectDisagreementSource', $event)"
    />
  </div>
</template>
