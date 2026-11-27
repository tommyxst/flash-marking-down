<script setup lang="ts">
import { isTauri } from '@tauri-apps/api/core'
import { getCurrentWindow } from '@tauri-apps/api/window'
import { computed, nextTick, onMounted, onUnmounted, ref } from 'vue'
import { api } from '../api/client'
import { parseAppError, type UiError } from '../api/errors'
import CaptureEditor from '../CaptureEditor.vue'
import ErrorNotice from '../components/ErrorNotice.vue'
import type { CaptureCommitMode, OpenedWorkspace, RecoveryErrorDto, WikilinkCandidate } from '../m2'
import {
  WIKRA_PREFS_STORAGE_KEY,
  applyBodyPreferences,
  readCachedPreferences,
} from '../utils/wikra'

type PendingCapture = { requestId: string; revision: number; body: string; mode: CaptureCommitMode }

const desktopWindow = isTauri() ? getCurrentWindow() : undefined
const workspace = ref<{ session: string; root: string; captureDir: string }>()
const opening = ref(true)
const workspaceError = ref<UiError | null>(null)
const recoveredPaths = ref<string[]>([])
const recoveryErrors = ref<RecoveryErrorDto[]>([])
const editor = ref<{ focus: () => void; isCompleting: () => boolean }>()

const body = ref('')
const revision = ref(0)
const persistedRevision = ref(0)
const draftError = ref<UiError | null>(null)
const saving = ref(false)
const pendingCapture = ref<PendingCapture | null>(null)
const savedMessage = ref('')
const saveError = ref<UiError | null>(null)
const closing = ref(false)
const exitRequested = ref(false)
const composing = ref(false)
const windowError = ref<UiError | null>(null)
const copyStatus = ref('')

// Session-scoped wikilink candidates loaded directly via list_wikilink_candidates
const wikilinkCandidates = ref<WikilinkCandidate[]>([])

let draftWrite: Promise<boolean> | undefined
let draftTimer: number | undefined
const draftDebounceMs = 300
const unlisteners: (() => void)[] = []
let disposed = false

const captureFolderLabel = computed(() => workspace.value?.captureDir || 'raw/notes')
const canSave = computed(
  () =>
    !opening.value
    && !saving.value
    && !closing.value
    && !composing.value
    && Boolean(pendingCapture.value || body.value.trim()),
)
const hasWriteError = computed(() => Boolean(draftError.value || saveError.value))
const showEscape = computed(() => Boolean(draftError.value || saveError.value || windowError.value || recoveryErrors.value.length))

const draftStatus = computed(() => {
  if (composing.value) return '中文组词期：Enter 与 Esc 优先交给输入法'
  if (saving.value) return '正在保存原文…'
  if (savedMessage.value) return savedMessage.value
  if (revision.value !== persistedRevision.value) {
    return draftError.value ? '最新编辑暂存内存（写盘失败）' : '正在保留草稿…'
  }
  return body.value ? 'Esc 仅收起并保留草稿，绝不误提交' : '支持 Markdown 与输入 [[ 或 【【 关联已有知识页'
})

function focusCaptureEditor() {
  editor.value?.focus()
}

function syncThemeFromCache() {
  const cached = readCachedPreferences()
  document.body.dataset.theme = cached.theme
  document.body.dataset.width = cached.docWidth
  if (!document.body.dataset.density) document.body.dataset.density = 'standard'
}

async function syncThemeFromSettings() {
  syncThemeFromCache()
  try {
    const s = await api.getAppSettings()
    applyBodyPreferences(s.theme === 'paper' ? 'paper' : 'slate', s.docWidth === 'full' ? 'full' : 'readable')
  } catch {
    // Main-window-only settings command falls back cleanly to shared localStorage theme cache.
  }
}

async function loadWikilinkCandidates(session: string) {
  wikilinkCandidates.value = []
  try {
    const candidates = await api.listWikilinkCandidates(session)
    if (workspace.value?.session === session) {
      wikilinkCandidates.value = candidates
    }
  } catch {
    // Keep empty candidate list cleanly if vault has no wiki pages yet.
  }
}

function handleStorageSync(event: StorageEvent) {
  if (event.key === WIKRA_PREFS_STORAGE_KEY) syncThemeFromCache()
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
    pendingCapture.value = null
    savedMessage.value = ''
    saveError.value = null
    body.value = value
    revision.value += 1
    scheduleDraftPersist()
  },
})

function applyWorkspace(opened: OpenedWorkspace) {
  window.clearTimeout(draftTimer)
  draftWrite = undefined
  closing.value = composing.value = false
  windowError.value = null
  workspace.value = { session: opened.session, root: opened.root, captureDir: opened.captureDir }
  revision.value = persistedRevision.value = opened.draft.revision
  body.value = opened.draft.body ?? ''
  recoveredPaths.value = opened.recovered
  recoveryErrors.value = opened.recoveryErrors
  pendingCapture.value = opened.pendingRequestId
    ? { requestId: opened.pendingRequestId, revision: opened.draft.revision, body: body.value, mode: 'queue' }
    : null
  savedMessage.value = ''
  saveError.value = draftError.value = null
  void loadWikilinkCandidates(opened.session)
  void nextTick(focusCaptureEditor)
}

function trackUnlisten(unlisten: () => void) {
  if (disposed) unlisten()
  else unlisteners.push(unlisten)
}

onMounted(async () => {
  window.addEventListener('keydown', handleWindowKey, true)
  window.addEventListener('storage', handleStorageSync)
  await syncThemeFromSettings()
  try {
    if (desktopWindow) {
      trackUnlisten(await desktopWindow.onCloseRequested(async (event) => {
        event.preventDefault()
        await closeCapture()
      }))
      trackUnlisten(await desktopWindow.listen<OpenedWorkspace>('capture-opened', ({ payload }) => {
        opening.value = false
        exitRequested.value = false
        workspaceError.value = null
        copyStatus.value = ''
        void syncThemeFromSettings()
        applyWorkspace(payload)
      }))
      trackUnlisten(await desktopWindow.listen('capture-focus-editor', focusCaptureEditor))
      trackUnlisten(await desktopWindow.listen('capture-close-requested', () => { void closeCapture() }))
      trackUnlisten(await desktopWindow.listen('exit-requested', () => {
        exitRequested.value = true
        void closeCapture()
      }))
    }
    applyWorkspace(await api.loadCaptureWorkspace())
  } catch (error) {
    workspaceError.value = parseAppError(error)
  } finally {
    opening.value = false
    if (exitRequested.value) void closeCapture()
  }
})

onUnmounted(() => {
  disposed = true
  window.removeEventListener('keydown', handleWindowKey, true)
  window.removeEventListener('storage', handleStorageSync)
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
  const current = workspace.value
  if (!current || opening.value) return Promise.resolve(false)
  draftWrite ??= flushDraft(current).finally(() => {
    if (workspace.value === current) draftWrite = undefined
  })
  return draftWrite
}

async function flushDraft(current: NonNullable<typeof workspace.value>) {
  try {
    while (revision.value !== persistedRevision.value) {
      const written = await api.persistDraft(current.session, revision.value, body.value)
      if (workspace.value !== current || opening.value) return false
      persistedRevision.value = written
    }
    draftError.value = null
    return true
  } catch (error) {
    if (workspace.value === current && !opening.value) draftError.value = parseAppError(error)
    return false
  }
}

async function commitWithMode(mode: CaptureCommitMode) {
  const current = workspace.value
  if (!current || !canSave.value) return
  saving.value = true
  saveError.value = null
  savedMessage.value = ''
  let succeeded = false
  try {
    if (!(await flushDraftNow())) return
    const capture: PendingCapture = pendingCapture.value && pendingCapture.value.body === body.value
      ? { ...pendingCapture.value, mode }
      : {
          requestId: `${Date.now()}-${crypto.randomUUID()}`,
          revision: revision.value,
          body: body.value,
          mode,
        }
    pendingCapture.value = capture
    const res = await api.commitCaptureV2(current.session, capture.requestId, capture.revision, capture.body, mode)
    await api.confirmCapture(current.session, capture.requestId).catch(() => false)
    recoveryErrors.value = recoveryErrors.value.filter(({ requestId }) => requestId !== capture.requestId)
    pendingCapture.value = null
    if (revision.value === capture.revision) body.value = ''
    savedMessage.value = mode === 'only'
      ? '已保存为原文笔记'
      : mode === 'now'
        ? '已保存并立即整理'
        : `已保存原文（${res.bufferSecondsLeft} 秒后自动整理）`
    succeeded = true
  } catch (error) {
    saveError.value = parseAppError(error)
  } finally {
    saving.value = false
  }
  if (succeeded) await closeCapture(true)
}

async function closeCapture(ignoreDraft = false) {
  if (!desktopWindow || closing.value) return
  if (opening.value || saving.value) {
    if (exitRequested.value) windowError.value = { safeMessage: '正在完成当前操作。' }
    return
  }
  if (composing.value && !ignoreDraft) {
    windowError.value = { safeMessage: '请先完成组词，或忽略当前输入强制关闭。' }
    return
  }
  closing.value = true
  windowError.value = null
  try {
    window.clearTimeout(draftTimer)
    if (!ignoreDraft && workspace.value && body.value.trim() && !(await flushDraftNow())) return
    opening.value = true
    await nextTick()
    await api.closeCaptureWindow(exitRequested.value)
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
    copyStatus.value = '已复制全文到剪贴板'
  } catch {
    copyStatus.value = '无法访问剪贴板，请选中正文后手动复制。'
  }
}

function handleWindowKey(event: KeyboardEvent) {
  // Level 1: IME composing priority — never intercept any key during IME composition!
  if (composing.value || event.isComposing || event.keyCode === 229) return

  // Pass-through special case: Ctrl+Enter / Ctrl+Shift+Enter submit even when completion popup is open.
  if (event.ctrlKey && !event.altKey && !event.metaKey && event.key === 'Enter') {
    event.preventDefault()
    void commitWithMode(event.shiftKey ? 'now' : 'queue')
    return
  }

  // Level 2: When CM6 completion is open, bare Esc / Enter / Tab / ArrowUp / ArrowDown return early to CM6.
  const noModifier = !event.ctrlKey && !event.altKey && !event.metaKey && !event.shiftKey
  if (
    editor.value?.isCompleting()
    && noModifier
    && (
      event.key === 'Escape'
      || event.key === 'Enter'
      || event.key === 'Tab'
      || event.key === 'ArrowUp'
      || event.key === 'ArrowDown'
    )
  ) {
    return
  }

  // Level 3 (empty -> close immediately) & Level 4 (non-empty -> persist draft and hide, never submit)
  if (event.key === 'Escape') {
    event.preventDefault()
    void closeCapture(!body.value.trim())
  }
}
</script>

<template>
  <main class="capture-shell">
    <header class="capture-header">
      <div class="capture-identity">
        <span class="pill accent">随手记</span>
        <span style="font-size:11.5px;color:var(--muted);">
          存为本地 Markdown 原文（<span class="mono">{{ captureFolderLabel }}</span>）
        </span>
      </div>
      <div class="capture-drag" aria-hidden="true" @mousedown.left="dragWindow" />
      <button
        type="button"
        class="icon-btn"
        title="收起浮窗并保留草稿 (Esc)"
        aria-label="收起浮窗"
        :disabled="saving || closing"
        @click="closeCapture(!body.trim())"
      >&times;</button>
    </header>

    <form v-if="workspace && !opening" class="capture-form" @submit.prevent="commitWithMode('queue')">
      <div class="capture-content">
        <CaptureEditor
          ref="editor"
          v-model="editorBody"
          :readonly="saving || closing"
          :wikilink-candidates="wikilinkCandidates"
          compact-tooltip
          placeholder-text="记下想法，支持 Markdown 语法与输入 [[ 或 【【 关联已有知识页…"
          @composition-change="composing = $event"
          @blur="!draftError && !closing && !composing && flushDraftNow()"
          @ready="nextTick(focusCaptureEditor)"
        />

        <div v-if="hasWriteError" class="hud-error-bar" role="alert">
          <div style="display:flex;flex-direction:column;gap:2px;">
            <strong>写入磁盘失败：内容仅暂存内存（退出程序前有效），请先复制全文备份。</strong>
            <span style="font-size:11.5px;color:var(--text-secondary);">
              {{ (saveError || draftError)?.safeMessage }}
            </span>
          </div>
          <div style="display:flex;gap:6px;flex:none;">
            <button type="button" class="btn primary sm" :disabled="!body" @click="copyMarkdown(body)">
              复制全文
            </button>
            <button v-if="draftError" type="button" class="btn sm" :disabled="saving || closing" @click="flushDraftNow">
              重试落盘
            </button>
            <button type="button" class="btn sm" :disabled="saving || closing" @click="closeCapture(true)">
              {{ exitRequested ? '忽略草稿并退出' : '暂存内存并收起' }}
            </button>
          </div>
        </div>

        <div
          v-if="windowError || recoveredPaths.length || recoveryErrors.length || copyStatus"
          style="padding:8px 14px;display:flex;flex-direction:column;gap:6px;background:var(--surface);border-top:1px solid var(--line-subtle);"
        >
          <ErrorNotice :error="windowError" />
          <p v-if="recoveredPaths.length" class="notice success" role="status">
            已恢复 {{ recoveredPaths.length }} 条随手记
          </p>
          <p v-for="item in recoveryErrors" :key="item.requestId" class="notice error" role="alert">
            恢复遇到问题：{{ item.message }}
          </p>
          <p v-if="copyStatus" class="notice success" role="status">{{ copyStatus }}</p>
          <div v-if="showEscape && !hasWriteError" style="display:flex;gap:6px;">
            <button type="button" class="btn primary sm" :disabled="!body" @click="copyMarkdown(body)">复制全文</button>
            <button type="button" class="btn sm" :disabled="saving || closing" @click="closeCapture(true)">
              {{ exitRequested ? '忽略草稿并退出应用' : '忽略草稿强制关闭' }}
            </button>
          </div>
        </div>
      </div>

      <footer class="capture-footer">
        <span style="font-size:11.5px;color:var(--muted);" role="status">
          <span
            class="status-dot"
            :class="{ off: !body || Boolean(draftError) || revision !== persistedRevision }"
          />
          {{ draftStatus }}
        </span>

        <div style="display:flex;align-items:center;gap:6px;flex-wrap:wrap;">
          <button
            type="button"
            class="btn quiet sm"
            :disabled="!canSave"
            @click="commitWithMode('only')"
          >仅保存原文</button>
          <button
            type="button"
            class="btn sm"
            :disabled="!canSave"
            @click="commitWithMode('now')"
          >立即整理 <kbd>Ctrl+Shift+Enter</kbd></button>
          <button
            type="submit"
            class="btn primary sm"
            :disabled="!canSave"
          >
            {{ pendingCapture ? '重试保存' : '保存并整理' }} <kbd class="hud-kbd-inline">Ctrl+Enter</kbd>
          </button>
        </div>
      </footer>
    </form>

    <div v-else class="capture-placeholder">
      <p v-if="opening" role="status" class="hint">正在读取草稿…</p>
      <ErrorNotice v-else :error="workspaceError" />
      <button v-if="!opening" type="button" class="btn sm" @click="closeCapture(true)">关闭</button>
    </div>
  </main>
</template>
