<script setup lang="ts">
import { computed, ref } from 'vue'
import { api } from './api/client'
import { parseAppError, type UiError } from './api/errors'
import ErrorNotice from './components/ErrorNotice.vue'
import DiffViewer from './DiffViewer.vue'
import UiIcon from './UiIcon.vue'
import {
  wikiStatusLabels,
  wikiStatusTone,
  type DocumentMapping,
  type WikiSuggestion,
} from './m2'

const props = defineProps<{
  session: string
  suggestions: WikiSuggestion[]
  wikiTargets: DocumentMapping[]
  workspaceFiles: string[]
  desktop: boolean
}>()

const emit = defineEmits<{
  updated: []
  targetsChanged: []
}>()

const mappingPath = ref('')
const mappingMode = ref<'existing' | 'new'>('existing')
const mappingBusy = ref(false)
const pickingFile = ref(false)
const busyId = ref('')
const error = ref<UiError | null>(null)
const feedback = ref('')

const mappedWikis = computed(() =>
  props.wikiTargets.filter((item) => item.kind === 'wiki'),
)

async function pickMarkdownFile() {
  if (pickingFile.value || mappingBusy.value) return
  pickingFile.value = true
  error.value = null
  try {
    const selected = await api.pickWorkspaceMarkdown(props.session, 'wiki')
    if (selected) mappingPath.value = selected
  } catch (err) {
    error.value = parseAppError(err)
  } finally {
    pickingFile.value = false
  }
}

async function addMapping() {
  const path = mappingPath.value.trim()
  if (!path || mappingBusy.value) return
  mappingBusy.value = true
  error.value = null
  feedback.value = ''
  try {
    if (mappingMode.value === 'new') await api.createWikiTarget(props.session, path)
    else await api.mapWikiTarget(props.session, 'wiki', path)
    mappingPath.value = ''
    feedback.value = mappingMode.value === 'new' ? `已新建并映射 ${path}` : `已映射 ${path}`
    emit('targetsChanged')
  } catch (err) {
    error.value = parseAppError(err)
  } finally {
    mappingBusy.value = false
  }
}

async function unmapTarget(path: string) {
  if (mappingBusy.value) return
  mappingBusy.value = true
  error.value = null
  feedback.value = ''
  try {
    await api.unmapWikiTarget(props.session, path)
    feedback.value = `已解除 Wiki 目标映射：${path}`
    emit('targetsChanged')
  } catch (err) {
    error.value = parseAppError(err)
  } finally {
    mappingBusy.value = false
  }
}

async function runAction(
  id: string,
  action: 'apply' | 'recompare' | 'reject' | 'undo',
) {
  if (busyId.value) return
  busyId.value = id
  error.value = null
  feedback.value = ''
  try {
    if (action === 'apply') {
      await api.applyWikiSuggestion(props.session, id)
      feedback.value = '已采纳并写入 Wiki 文件。'
    } else if (action === 'recompare') {
      await api.recompareWikiSuggestion(props.session, id)
      feedback.value = '已基于最新磁盘内容重新生成 Diff，请核对后采纳。'
    } else if (action === 'reject') {
      await api.rejectWikiSuggestion(props.session, id)
      feedback.value = '已拒绝该变更建议。'
    } else {
      await api.undoWikiSuggestion(props.session, id)
      feedback.value = '已撤销此次写入（保留随后的手工编辑）。'
    }
    emit('updated')
  } catch (err) {
    error.value = parseAppError(err)
    emit('updated')
  } finally {
    busyId.value = ''
  }
}

async function openExternal(path: string) {
  try {
    await api.openWorkspaceFile(props.session, path, 'open')
  } catch (err) {
    error.value = parseAppError(err)
  }
}
</script>

<template>
  <div class="m2-stack">
    <details class="detail-card" :open="!mappedWikis.length">
      <summary>Wiki 目标页管理（{{ mappedWikis.length }}）</summary>
      <ul v-if="mappedWikis.length" class="m2-list wiki-target-list">
        <li v-for="item in mappedWikis" :key="item.path" class="wiki-target-row">
          <code>{{ item.path }}</code>
          <div class="actions">
            <button v-if="desktop" type="button" class="quiet compact-btn" @click="openExternal(item.path)">
              <UiIcon name="external" />打开
            </button>
            <button
              type="button"
              class="quiet compact-btn"
              :disabled="mappingBusy"
              @click="unmapTarget(item.path)"
            >
              解除映射
            </button>
          </div>
        </li>
      </ul>
      <form class="m2-form" @submit.prevent="addMapping">
        <div class="actions">
          <select v-model="mappingMode" :disabled="mappingBusy || pickingFile">
            <option value="existing">映射已有 Wiki 页</option>
            <option value="new">新建空白 Wiki 页</option>
          </select>
        </div>
        <div class="folder-field">
          <input
            v-model="mappingPath"
            required
            :list="mappingMode === 'existing' ? 'drawer-wiki-files' : undefined"
            :placeholder="mappingMode === 'new' ? '例如：wiki/主题.md' : '选择或输入工作区 .md 路径…'"
            :disabled="mappingBusy || pickingFile"
            spellcheck="false"
          >
          <button
            v-if="desktop && mappingMode === 'existing'"
            type="button"
            class="compact-btn"
            :disabled="mappingBusy || pickingFile"
            @click="pickMarkdownFile"
          >
            浏览
          </button>
        </div>
        <datalist id="drawer-wiki-files">
          <option v-for="file in workspaceFiles" :key="file" :value="file" />
        </datalist>
        <button type="submit" class="compact-btn" :disabled="mappingBusy || pickingFile || !mappingPath.trim()">
          {{ mappingMode === 'new' ? '创建并映射' : '添加映射' }}
        </button>
      </form>
    </details>

    <ErrorNotice :error="error" />
    <p v-if="feedback" class="notice success compact-notice" role="status">{{ feedback }}</p>

    <div v-if="!suggestions.length" class="empty-panel compact">
      <p>暂无待审阅的 Wiki 文档变更建议。后台整理或中栏「推送 Wiki 建议」后会在此列出。</p>
    </div>

    <article v-for="item in suggestions" :key="item.id" class="detail-card">
      <header class="m2-card-header">
        <div class="actions">
          <code>{{ item.targetPath }}</code>
          <span class="badge" :class="wikiStatusTone[item.status]">{{ wikiStatusLabels[item.status] }}</span>
        </div>
        <button v-if="desktop" type="button" class="quiet compact-btn" @click="openExternal(item.targetPath)">
          <UiIcon name="external" />外部打开
        </button>
      </header>

      <p v-if="item.reason" class="hint">{{ item.reason }}</p>
      <DiffViewer :before="item.beforeText" :after="item.afterText" :show-full-comparison="false" />

      <div class="actions">
        <template v-if="item.status === 'pending'">
          <button type="button" class="primary compact-btn" :disabled="busyId === item.id" @click="runAction(item.id, 'apply')">
            采纳写入
          </button>
          <button type="button" class="compact-btn" :disabled="busyId === item.id" @click="runAction(item.id, 'reject')">
            拒绝
          </button>
        </template>
        <template v-else-if="item.status === 'conflict'">
          <button type="button" class="primary compact-btn" :disabled="busyId === item.id" @click="runAction(item.id, 'recompare')">
            重新比较当前文件
          </button>
          <button type="button" class="compact-btn" :disabled="busyId === item.id" @click="runAction(item.id, 'reject')">
            拒绝
          </button>
        </template>
        <template v-else-if="item.status === 'applied'">
          <button type="button" class="compact-btn" :disabled="busyId === item.id" @click="runAction(item.id, 'undo')">
            撤销此变更
          </button>
        </template>
      </div>
    </article>
  </div>
</template>
