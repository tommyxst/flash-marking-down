<script setup lang="ts">
import { isTauri } from '@tauri-apps/api/core'
import { getCurrentWindow } from '@tauri-apps/api/window'
import { computed, nextTick, onMounted, onUnmounted, ref } from 'vue'
import { api } from '../api/client'
import { parseAppError, type UiError } from '../api/errors'
import CaptureEditor from '../CaptureEditor.vue'
import ErrorNotice from '../components/ErrorNotice.vue'
import UiIcon from '../UiIcon.vue'
import type { OpenedWorkspace, RecoveryErrorDto } from '../m2'

type PendingCapture = { requestId: string; revision: number; body: string }

const desktopWindow = isTauri() ? getCurrentWindow() : undefined
const workspace = ref<{ session: string; root: string; captureDir: string }>()
const opening = ref(true)
const workspaceError = ref<UiError | null>(null)
const recoveredPaths = ref<string[]>([])
const recoveryErrors = ref<RecoveryErrorDto[]>([])
const editor = ref<{ focus: () => void }>()

const body = ref('')
const revision = ref(0)
const persistedRevision = ref(0)
const draftError = ref<UiError | null>(null)
const saving = ref(false)
const pendingCapture = ref<PendingCapture | null>(null)
const savedPath = ref('')
const saveError = ref<UiError | null>(null)
const closing = ref(false)
const composing = ref(false)
const windowError = ref<UiError | null>(null)
const copyStatus = ref('')

let draftWrite: Promise<boolean> | undefined
let draftTimer: number | undefined
const draftDebounceMs = 300
const unlisteners: (() => void)[] = []
let disposed = false

const captureFolderLabel = computed(() => workspace.value?.captureDir || '工作区根目录')
const canSave = computed(() => !opening.value && !saving.value && !closing.value && !composing.value
  && Boolean(pendingCapture.value || body.value.trim()))

const draftStatus = computed(() => {
  if (saving.value) return '正在保存原文…'
  if (revision.value !== persistedRevision.value) {
    return draftError.value ? '最新编辑尚未落盘' : '正在保留草稿…'
  }
  return body.value ? '草稿已保留' : '等待输入'
})

function focusCaptureEditor() {
  editor.value?.focus()
}

function scheduleDraftPersist() {
  window.clearTimeout(draftTimer)
  draftTimer = window.setTimeout(() => {
    draftTimer = undefined
    void persistDraft()
  }, draftDebounceMs)
}

function flushDraftNow() {
  window.clearTimeout(draftTimer)
  draftTimer = undefined
  return persistDraft()
}

const editorBody = computed({
  get: () => body.value,
  set: (value: string) => {
    if (value === body.value) return
    body.value = value
    revision.value += 1
    scheduleDraftPersist()
  },
})

function applyWorkspace(opened: OpenedWorkspace) {
  workspace.value = { session: opened.session, root: opened.root, captureDir: opened.captureDir }
  revision.value = persistedRevision.value = opened.draft.revision
  body.value = opened.draft.body ?? ''
  recoveredPaths.value = opened.recovered
  recoveryErrors.value = opened.recoveryErrors
  pendingCapture.value = opened.pendingRequestId
    ? { requestId: opened.pendingRequestId, revision: opened.draft.revision, body: body.value }
    : null
  savedPath.value = ''
  saveError.value = draftError.value = null
  void nextTick(focusCaptureEditor)
}

function trackUnlisten(unlisten: () => void) {
  if (disposed) unlisten()
  else unlisteners.push(unlisten)
}

onMounted(async () => {
  window.addEventListener('keydown', handleWindowKey)
  try {
    if (desktopWindow) {
      trackUnlisten(await desktopWindow.onCloseRequested(async (event) => {
        event.preventDefault()
        await closeCapture()
      }))
      trackUnlisten(await desktopWindow.listen<OpenedWorkspace>('capture-opened', ({ payload }) => {
        opening.value = false
        workspaceError.value = null
        copyStatus.value = ''
        applyWorkspace(payload)
      }))
      trackUnlisten(await desktopWindow.listen('capture-focus-editor', focusCaptureEditor))
      trackUnlisten(await desktopWindow.listen('exit-requested', () => { void closeCapture(true) }))
    }
    applyWorkspace(await api.loadCaptureWorkspace())
  } catch (error) {
    workspaceError.value = parseAppError(error)
  } finally {
    opening.value = false
  }
})

onUnmounted(() => {
  disposed = true
  window.removeEventListener('keydown', handleWindowKey)
  window.clearTimeout(draftTimer)
  for (const unlisten of unlisteners.splice(0)) unlisten()
})

async function dragWindow() {
  if (!desktopWindow) return
  windowError.value = null
  try {
    await api.desktopWindowAction('drag')
  } catch (error) {
    windowError.value = parseAppError(error)
  }
}

function persistDraft() {
  const session = workspace.value?.session
  if (!session) return Promise.resolve(false)
  draftWrite ??= flushDraft(session).finally(() => { draftWrite = undefined })
  return draftWrite
}

async function flushDraft(session: string) {
  try {
    while (revision.value !== persistedRevision.value) {
      persistedRevision.value = await api.persistDraft(session, revision.value, body.value)
    }
    draftError.value = null
    return true
  } catch (error) {
    draftError.value = parseAppError(error)
    return false
  }
}

async function save() {
  const current = workspace.value
  if (!current || !canSave.value) return
  saving.value = true
  saveError.value = null
  savedPath.value = ''
  try {
    if (!(await flushDraftNow())) return
    const capture = pendingCapture.value ?? {
      requestId: crypto.randomUUID(),
      revision: revision.value,
      body: body.value,
    }
    pendingCapture.value = capture
    const path = await api.commitCapture(current.session, capture.requestId, capture.revision, capture.body)
    await api.confirmCapture(current.session, capture.requestId)
    recoveryErrors.value = recoveryErrors.value.filter(({ requestId }) => requestId !== capture.requestId)
    pendingCapture.value = null
    if (revision.value === capture.revision) body.value = ''
    savedPath.value = path
  } catch (error) {
    saveError.value = parseAppError(error)
  } finally {
    saving.value = false
  }
  if (savedPath.value) await closeCapture()
}

async function closeCapture(exit = false) {
  if (!desktopWindow || closing.value) return
  if (opening.value || saving.value) {
    if (exit) windowError.value = { safeMessage: '请等待当前操作完成，再退出应用。' }
    return
  }
  if (composing.value) {
    windowError.value = { safeMessage: exit ? '请先完成当前输入，再退出应用。' : '请先完成当前输入，再关闭窗口。' }
    return
  }
  closing.value = true
  windowError.value = null
  try {
    if (workspace.value && !(await flushDraftNow())) return
    opening.value = true
    await nextTick()
    await api.closeCaptureWindow(exit)
  } catch (error) {
    opening.value = false
    windowError.value = parseAppError(error)
  } finally {
    closing.value = false
  }
}

async function copyMarkdown(text: string) {
  copyStatus.value = ''
  try {
    await navigator.clipboard.writeText(text)
    copyStatus.value = '已复制 Markdown'
  } catch {
    copyStatus.value = '无法访问剪贴板，请选中正文后手动复制。'
  }
}

function handleWindowKey(event: KeyboardEvent) {
  if (event.isComposing || event.keyCode === 229) return
  if (event.key === 'Escape') {
    event.preventDefault()
    void closeCapture()
  } else if (event.ctrlKey && event.key === 'Enter') {
    event.preventDefault()
    void save()
  }
}
</script>

<template>
  <main class="capture-shell">
    <header class="capture-header">
      <div class="capture-identity">
        <UiIcon name="terminal" />
        <h1>随手记</h1>
      </div>
      <code v-if="workspace" class="destination" :title="workspace.captureDir || workspace.root">
        <UiIcon name="folder" /> {{ captureFolderLabel }}
      </code>
      <div class="window-drag" aria-hidden="true" @mousedown.left="dragWindow" />
    </header>

    <form v-if="workspace && !opening" class="capture-form" @submit.prevent="save">
      <div class="capture-content">
        <CaptureEditor
          ref="editor"
          v-model="editorBody"
          :readonly="opening || saving || closing || pendingCapture !== null"
          @composition-change="composing = $event"
          @blur="flushDraftNow"
          @ready="nextTick(focusCaptureEditor)"
        />
        <div v-if="draftError || saveError || windowError || recoveredPaths.length || recoveryErrors.length || copyStatus" class="capture-notices">
          <ErrorNotice :error="draftError" suffix="。请重试保留草稿，或复制正文。" />
          <ErrorNotice :error="saveError" suffix="。正文已保留，可以重试保存或复制正文。" />
          <ErrorNotice :error="windowError" />
          <p v-if="recoveredPaths.length" class="notice success" role="status">已恢复 {{ recoveredPaths.length }} 条随手记</p>
          <p v-for="item in recoveryErrors" :key="item.requestId" class="notice error" role="alert">恢复遇到问题。{{ item.message }}</p>
          <p v-if="copyStatus" class="notice" role="status">{{ copyStatus }}</p>
          <div v-if="draftError || saveError || (pendingCapture && recoveryErrors.length)" class="actions">
            <button v-if="draftError" type="button" :disabled="saving || closing" @click="flushDraftNow">重试保留草稿</button>
            <button type="button" :disabled="!body" @click="copyMarkdown(body)">复制正文</button>
          </div>
        </div>
      </div>
      <footer class="capture-footer">
        <p class="draft-state" role="status">
          <span class="status-dot" :class="{ good: savedPath || (body && !draftError && revision === persistedRevision) }" />
          {{ savedPath ? '已保存原文' : draftStatus }}
        </p>
        <div class="actions">
          <button type="button" class="quiet" :disabled="saving || closing" @click="closeCapture()">关闭 <kbd>Esc</kbd></button>
          <button type="submit" class="primary" :disabled="!canSave">
            {{ pendingCapture ? '重试保存' : '保存并返回' }} <kbd>Ctrl+Enter</kbd>
          </button>
        </div>
      </footer>
    </form>
    <div v-else class="capture-placeholder">
      <p v-if="opening" role="status">正在读取草稿…</p>
      <ErrorNotice v-else :error="workspaceError" />
      <button v-if="!opening" type="button" @click="closeCapture()">关闭</button>
    </div>
  </main>
</template>
