<script setup lang="ts">
import { onMounted, onUnmounted, ref } from 'vue'
import { api } from '../../api/client'
import { parseAppError, type UiError } from '../../api/errors'
import ErrorNotice from '../ErrorNotice.vue'
import type { OpenedWorkspace } from '../../m2'

const props = defineProps<{
  open: boolean
  inline?: boolean
  desktop: boolean
  canDismiss: boolean
  currentRoot?: string
  currentCaptureDir?: string
}>()

const emit = defineEmits<{
  close: []
  workspaceOpened: [opened: OpenedWorkspace]
}>()

const customRoot = ref(props.currentRoot || '')
const captureDir = ref(props.currentCaptureDir || 'raw/notes')
const busy = ref(false)
const error = ref<UiError | null>(null)

async function pickAndInitStandard() {
  if (busy.value) return
  error.value = null
  busy.value = true
  try {
    let targetRoot = customRoot.value.trim()
    if (props.desktop) {
      const picked = await api.pickWorkspaceFolder('root', targetRoot, 'raw/notes')
      if (!picked) return
      targetRoot = picked
      customRoot.value = picked
    }
    if (!targetRoot) {
      error.value = { safeMessage: '请先选择或输入要创建标准知识库的本地目录路径。' }
      return
    }
    const opened = await api.initStandardVault(targetRoot)
    emit('workspaceOpened', opened)
    emit('close')
  } catch (err) {
    error.value = parseAppError(err)
  } finally {
    busy.value = false
  }
}

async function pickAndAttachExisting() {
  if (busy.value) return
  error.value = null
  busy.value = true
  try {
    let targetRoot = customRoot.value.trim()
    if (props.desktop) {
      const picked = await api.pickWorkspaceFolder('root', targetRoot, captureDir.value || 'notes')
      if (!picked) return
      targetRoot = picked
      customRoot.value = picked
    }
    if (!targetRoot) {
      error.value = { safeMessage: '请先选择或输入已有的 Obsidian / Markdown 目录路径。' }
      return
    }
    const opened = await api.openWorkspace(targetRoot, captureDir.value.trim() || 'notes')
    emit('workspaceOpened', opened)
    emit('close')
  } catch (err) {
    error.value = parseAppError(err)
  } finally {
    busy.value = false
  }
}

async function openByTypedPath(mode: 'standard' | 'attach') {
  if (busy.value) return
  const targetRoot = customRoot.value.trim()
  if (!targetRoot) {
    error.value = { safeMessage: '请输入本地绝对目录路径。' }
    return
  }
  busy.value = true
  error.value = null
  try {
    const opened = mode === 'standard'
      ? await api.initStandardVault(targetRoot)
      : await api.openWorkspace(targetRoot, captureDir.value.trim() || 'notes')
    emit('workspaceOpened', opened)
    emit('close')
  } catch (err) {
    error.value = parseAppError(err)
  } finally {
    busy.value = false
  }
}

function onKeydown(e: KeyboardEvent) {
  if (!props.inline && props.open && props.canDismiss && e.key === 'Escape') {
    e.preventDefault()
    emit('close')
  }
}

onMounted(() => window.addEventListener('keydown', onKeydown))
onUnmounted(() => window.removeEventListener('keydown', onKeydown))
</script>

<template>
  <div v-if="inline" class="onboarding-welcome">
    <div class="modal-window medium" style="max-height:none;">
      <div class="hud-head">
        <strong style="font-size:14.5px;">欢迎使用 Wikra · 选择或新建本地知识库</strong>
        <span class="pill accent">100% 兼容 Obsidian</span>
      </div>
      <div class="modal-body">
        <p style="font-size:13px;color:var(--text-secondary);">
          Wikra 是独立通用的本地个人知识工作台，所有笔记都以标准 Markdown 保存在你指定的文件夹中。你可以一键新建标准库，或零搬迁打开已有 Obsidian 目录。
        </p>

        <div class="onboarding-card-grid">
          <div class="setting-card">
            <span class="pill accent" style="align-self:flex-start;">推荐 · 开箱即用</span>
            <strong>新建标准知识库</strong>
            <p style="font-size:12px;color:var(--text-secondary);">
              自动创建 <span class="mono">wiki/</span>（整理后的知识）与 <span class="mono">raw/</span>（原文归档），立刻开始记录与整理。
            </p>
            <button type="button" class="btn primary sm" :disabled="busy" @click="pickAndInitStandard">
              {{ busy ? '正在初始化…' : desktop ? '选择目录并新建标准库' : '新建标准库' }}
            </button>
          </div>

          <div class="setting-card">
            <span class="pill" style="align-self:flex-start;">零搬迁兼容</span>
            <strong>打开已有 Obsidian 目录</strong>
            <p style="font-size:12px;color:var(--text-secondary);">
              不移动、不改写你原有笔记；工作台状态独立存于 <span class="mono">.wikra/</span>。
            </p>
            <button type="button" class="btn sm" :disabled="busy" @click="pickAndAttachExisting">
              {{ busy ? '正在打开…' : desktop ? '选择已有目录打开' : '打开已有目录' }}
            </button>
          </div>
        </div>

        <details style="font-size:12px;color:var(--text-secondary);">
          <summary style="cursor:pointer;">手动输入目录路径与随手记子目录</summary>
          <div class="form-grid" style="margin-top:10px;">
            <div class="field full">
              <label for="ob-inline-root">知识库绝对路径</label>
              <input id="ob-inline-root" v-model="customRoot" type="text" placeholder="例如：D:\Notes\my-wikra-vault">
            </div>
            <div class="field full">
              <label for="ob-inline-cap">随手记存放子目录（仅「打开已有目录」时使用）</label>
              <input id="ob-inline-cap" v-model="captureDir" type="text" placeholder="raw/notes 或 notes">
            </div>
          </div>
          <div style="display:flex;gap:8px;margin-top:10px;">
            <button type="button" class="btn primary sm" :disabled="busy" @click="openByTypedPath('standard')">按路径新建标准库</button>
            <button type="button" class="btn sm" :disabled="busy" @click="openByTypedPath('attach')">按路径打开已有库</button>
          </div>
        </details>

        <ErrorNotice :error="error" />
      </div>
    </div>
  </div>

  <div
    v-else-if="open"
    class="overlay-layer"
    role="dialog"
    aria-modal="true"
    aria-label="选择或新建本地知识库"
    @click.self="canDismiss && emit('close')"
  >
    <div class="modal-window medium">
      <div class="hud-head">
        <strong style="font-size:14px;">选择或新建本地知识库</strong>
        <button v-if="canDismiss" type="button" class="icon-btn" aria-label="关闭" @click="emit('close')">&times;</button>
      </div>
      <div class="modal-body">
        <p style="font-size:13px;color:var(--text-secondary);">
          独立通用的本地 Markdown 知识工作台，100% 兼容 Obsidian 文件格式。你可以新建空白知识库，或零搬迁直接打开已有 Obsidian 目录。
        </p>

        <div class="onboarding-card-grid">
          <div class="setting-card">
            <span class="pill accent" style="align-self:flex-start;">推荐 · 开箱即用</span>
            <strong>新建标准知识库</strong>
            <p style="font-size:12px;color:var(--text-secondary);">
              自动创建 <span class="mono">wiki/</span>（整理后的知识）与 <span class="mono">raw/</span>（原文归档），立刻开始记录。
            </p>
            <button type="button" class="btn primary sm" :disabled="busy" @click="pickAndInitStandard">
              {{ busy ? '正在初始化…' : desktop ? '选择目录并新建标准库' : '新建标准库' }}
            </button>
          </div>

          <div class="setting-card">
            <span class="pill" style="align-self:flex-start;">零搬迁兼容</span>
            <strong>打开已有 Obsidian 目录</strong>
            <p style="font-size:12px;color:var(--text-secondary);">
              不移动、不修改你原有笔记；配置独立存于 <span class="mono">.wikra/</span>。
            </p>
            <button type="button" class="btn sm" :disabled="busy" @click="pickAndAttachExisting">
              {{ busy ? '正在打开…' : desktop ? '选择已有目录打开' : '打开已有目录' }}
            </button>
          </div>
        </div>

        <details style="font-size:12px;color:var(--text-secondary);">
          <summary style="cursor:pointer;">手动输入目录路径与随手记子目录</summary>
          <div class="form-grid" style="margin-top:10px;">
            <div class="field full">
              <label for="ob-modal-root">知识库绝对路径</label>
              <input id="ob-modal-root" v-model="customRoot" type="text" placeholder="例如：D:\Notes\my-wikra-vault">
            </div>
            <div class="field full">
              <label for="ob-modal-cap">随手记存放子目录（打开已有目录时使用）</label>
              <input id="ob-modal-cap" v-model="captureDir" type="text" placeholder="raw/notes 或 notes">
            </div>
          </div>
          <div style="display:flex;gap:8px;margin-top:10px;">
            <button type="button" class="btn primary sm" :disabled="busy" @click="openByTypedPath('standard')">按路径新建标准库</button>
            <button type="button" class="btn sm" :disabled="busy" @click="openByTypedPath('attach')">按路径打开已有库</button>
          </div>
        </details>

        <ErrorNotice :error="error" />
      </div>
    </div>
  </div>
</template>
