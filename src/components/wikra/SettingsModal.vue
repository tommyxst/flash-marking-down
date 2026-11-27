<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watch } from 'vue'
import { api, type AppSettings } from '../../api/client'
import { parseAppError, type UiError } from '../../api/errors'
import ErrorNotice from '../ErrorNotice.vue'
import type {
  CredentialReference,
  CredentialSlot,
  CredentialState,
  DocWidthMode,
  HotkeyStatus,
  IndexRebuildResult,
  IntegrationsConfig,
  OpenedWorkspace,
  ThemeName,
} from '../../m2'
import { isLoopbackUrl } from '../../utils/wikra'

const credentialLabels: Record<CredentialState, string> = {
  unlinked: '未关联',
  linked: '已关联',
  missing: '系统凭据缺失',
  unavailable: '暂无法查询系统凭据',
  invalid: '与当前知识库不匹配',
}

const props = defineProps<{
  open: boolean
  desktop: boolean
  session: string
  vaultRoot: string
  rawDir: string
  captureDir: string
  settings: AppSettings
  hotkey?: HotkeyStatus
  hotkeyBusy: boolean
  hotkeyError: UiError | null
}>()

const emit = defineEmits<{
  close: []
  settingsSaved: [settings: AppSettings]
  integrationsUpdated: [config: IntegrationsConfig]
  workspaceDirsUpdated: [workspace: OpenedWorkspace]
  hotkeyRequest: [choice?: string]
  openOnboarding: []
  indexRebuilt: [result: IndexRebuildResult]
}>()

// Section 1 & 4: AppSettings local draft
const theme = ref<ThemeName>('slate')
const docWidth = ref<DocWidthMode>('readable')
const allowNoteFulltextToModel = ref(true)
const quietOrganize = ref(true)
const autoOrganize = ref(true)
const autoSyncMemory = ref(false)
const bufferSeconds = ref(60)
const savingGeneral = ref(false)
const generalError = ref<UiError | null>(null)
const generalFeedback = ref('')

// Section 2 & 3: Model & OpenViking integrations
const modelEndpoint = ref('')
const modelName = ref('')
const modelEnabled = ref(false)
const modelSecret = ref('')
const modelCred = ref<CredentialReference | undefined>(undefined)
const modelOptions = ref<string[]>([])
const loadingModels = ref(false)
const modelBusy = ref(false)
const modelFeedback = ref('')
const modelError = ref<UiError | null>(null)

const ovEnabled = ref(false)
const ovEndpoint = ref('http://127.0.0.1:1933')
const ovNamespace = ref('default')
const ovSyncResources = ref(true)
const ovCommitMemories = ref(true)
const ovSecret = ref('')
const ovCred = ref<CredentialReference | undefined>(undefined)
const ovBusy = ref(false)
const ovFeedback = ref('')
const ovError = ref<UiError | null>(null)

const loadingIntegrations = ref(false)
const integrationsError = ref<UiError | null>(null)

// Section 5: Hotkey, Workspace Dirs & Index
const hotkeyChoice = ref('')
const rawDirDraft = ref('raw')
const captureDirDraft = ref('raw/notes')
const lastSyncedRawDir = ref('raw')
let syncingDirs = false
const pickingDir = ref<null | 'raw' | 'capture'>(null)
const savingDirs = ref(false)
const dirsFeedback = ref('')
const dirsError = ref<UiError | null>(null)
const rebuilding = ref(false)
const rebuildMessage = ref('')
const rebuildError = ref<UiError | null>(null)

function normalizeDirPath(val: string): string {
  return val
    .trim()
    .replaceAll('\\', '/')
    .replace(/^\.\/+/, '')
    .replace(/^\/+/, '')
    .replace(/\/{2,}/g, '/')
    .replace(/\/+$/, '')
}

const effectiveRawDirPreview = computed(() => normalizeDirPath(rawDirDraft.value) || 'raw')

const modelIsLocal = computed(() => !modelEndpoint.value.trim() || isLoopbackUrl(modelEndpoint.value))
const ovIsLocal = computed(() => !ovEndpoint.value.trim() || isLoopbackUrl(ovEndpoint.value))

function syncDirsFromProps() {
  syncingDirs = true
  const rawVal = props.rawDir || 'raw'
  rawDirDraft.value = rawVal
  captureDirDraft.value = props.captureDir || `${normalizeDirPath(rawVal) || 'raw'}/notes`
  lastSyncedRawDir.value = normalizeDirPath(rawVal) || 'raw'
  syncingDirs = false
}

function syncFromProps() {
  theme.value = props.settings.theme === 'paper' ? 'paper' : 'slate'
  docWidth.value = props.settings.docWidth === 'full' ? 'full' : 'readable'
  allowNoteFulltextToModel.value = props.settings.allowNoteFulltextToModel
  quietOrganize.value = props.settings.quietOrganize
  autoOrganize.value = props.settings.autoOrganize
  autoSyncMemory.value = props.settings.autoSyncMemory
  bufferSeconds.value = props.settings.bufferSeconds
  syncDirsFromProps()
  generalError.value = null
  generalFeedback.value = ''
  modelFeedback.value = ''
  modelError.value = null
  ovFeedback.value = ''
  ovError.value = null
  dirsFeedback.value = ''
  dirsError.value = null
  rebuildMessage.value = ''
  rebuildError.value = null
}

async function loadIntegrations() {
  if (!props.session) return
  loadingIntegrations.value = true
  integrationsError.value = null
  try {
    const config = await api.getIntegrationsConfig(props.session)
    if (config.model) {
      modelEndpoint.value = config.model.endpoint
      modelName.value = config.model.model
      modelEnabled.value = config.model.enabled
    } else {
      modelEndpoint.value = ''
      modelName.value = ''
      modelEnabled.value = false
    }
    if (config.memory) {
      ovEndpoint.value = config.memory.baseUrl
      ovNamespace.value = config.memory.namespace
      ovEnabled.value = config.memory.enabled
      ovSyncResources.value = config.memory.syncResources
      ovCommitMemories.value = config.memory.commitMemories
    } else {
      ovEndpoint.value = 'http://127.0.0.1:1933'
      ovNamespace.value = 'default'
      ovEnabled.value = false
      ovSyncResources.value = true
      ovCommitMemories.value = true
    }
    modelSecret.value = ''
    ovSecret.value = ''
    modelCred.value = config.credentials.find((c) => c.slot === 'model')
    ovCred.value = config.credentials.find((c) => c.slot === 'memory')
    emit('integrationsUpdated', config)
  } catch (err) {
    integrationsError.value = parseAppError(err)
  } finally {
    loadingIntegrations.value = false
  }
}

watch(
  () => props.open,
  (isOpen) => {
    if (isOpen) {
      syncFromProps()
      if (props.hotkey) hotkeyChoice.value = props.hotkey.id
      if (props.session) void loadIntegrations()
    }
  },
  { immediate: true },
)

watch(
  () => props.hotkey,
  (h) => {
    if (h) hotkeyChoice.value = h.id
  },
  { immediate: true },
)

watch(rawDirDraft, (nextRaw) => {
  if (syncingDirs) return
  const prevClean = lastSyncedRawDir.value
  const nextClean = normalizeDirPath(nextRaw)
  const curCaptureClean = normalizeDirPath(captureDirDraft.value)
  if (prevClean && nextClean && curCaptureClean === `${prevClean}/notes`) {
    captureDirDraft.value = `${nextClean}/notes`
  }
  if (nextClean) {
    lastSyncedRawDir.value = nextClean
  }
})

async function pickWorkspaceDir(kind: 'raw' | 'capture') {
  if (!props.desktop || !props.vaultRoot || pickingDir.value || savingDirs.value) return
  pickingDir.value = kind
  dirsError.value = null
  dirsFeedback.value = ''
  try {
    const current = kind === 'raw' ? rawDirDraft.value : captureDirDraft.value
    const selected = await api.pickWorkspaceFolder(kind, props.vaultRoot, current)
    if (selected !== null) {
      if (kind === 'raw') {
        rawDirDraft.value = selected
      } else {
        captureDirDraft.value = selected
      }
    }
  } catch (err) {
    dirsError.value = parseAppError(err)
  } finally {
    pickingDir.value = null
  }
}

async function saveWorkspaceDirs() {
  if (!props.session || savingDirs.value) return
  const nextRaw = rawDirDraft.value.trim()
  const nextCapture = captureDirDraft.value.trim()
  if (!nextRaw) return
  savingDirs.value = true
  dirsError.value = null
  dirsFeedback.value = ''
  try {
    const updated = await api.updateWorkspaceDirs(
      props.session,
      nextRaw,
      nextCapture || undefined,
    )
    syncingDirs = true
    rawDirDraft.value = updated.rawDir || nextRaw
    captureDirDraft.value = updated.captureDir || nextCapture
    lastSyncedRawDir.value = normalizeDirPath(rawDirDraft.value) || 'raw'
    syncingDirs = false
    emit('workspaceDirsUpdated', updated)
    dirsFeedback.value = `目录设置已保存：原始文件位于 ${rawDirDraft.value}/，随手记保存至 ${captureDirDraft.value}/。`
  } catch (err) {
    dirsError.value = parseAppError(err)
  } finally {
    savingDirs.value = false
  }
}

async function saveGeneralSettings() {
  if (savingGeneral.value) return
  savingGeneral.value = true
  generalError.value = null
  generalFeedback.value = ''
  try {
    const clampedBuf = Math.max(0, Math.min(300, Math.round(Number(bufferSeconds.value) || 0)))
    bufferSeconds.value = clampedBuf
    const saved = await api.saveAppSettings({
      theme: theme.value,
      docWidth: docWidth.value,
      allowNoteFulltextToModel: allowNoteFulltextToModel.value,
      quietOrganize: quietOrganize.value,
      autoOrganize: autoOrganize.value,
      autoSyncMemory: autoSyncMemory.value,
      bufferSeconds: clampedBuf,
    })
    emit('settingsSaved', saved)
    generalFeedback.value = '外观与整理偏好已保存。'
  } catch (err) {
    generalError.value = parseAppError(err)
  } finally {
    savingGeneral.value = false
  }
}

async function fetchRemoteModels() {
  if (!props.session || !modelEndpoint.value.trim() || loadingModels.value) return
  loadingModels.value = true
  modelError.value = null
  modelFeedback.value = ''
  try {
    const list = await api.listRemoteModels(
      props.session,
      modelEndpoint.value.trim(),
      modelSecret.value.trim() || undefined,
    )
    modelOptions.value = list
    if (!modelName.value.trim() && list.length > 0) {
      modelName.value = list[0]
    }
    modelFeedback.value = `已拉取 ${list.length} 个可用模型，可在模型 ID 输入框下拉选择。`
  } catch (err) {
    modelError.value = parseAppError(err)
  } finally {
    loadingModels.value = false
  }
}

async function saveModel() {
  if (!props.session || modelBusy.value) return
  modelBusy.value = true
  modelError.value = null
  modelFeedback.value = ''
  try {
    await api.saveModelConfig(
      props.session,
      modelEndpoint.value.trim(),
      modelName.value.trim(),
      modelEnabled.value,
      modelSecret.value.trim() || undefined,
    )
    modelSecret.value = ''
    await loadIntegrations()
    modelFeedback.value = '模型配置已保存。'
  } catch (err) {
    modelError.value = parseAppError(err)
  } finally {
    modelBusy.value = false
  }
}

async function testSlot(slot: CredentialSlot) {
  if (!props.session) return
  if (slot === 'model') {
    if (modelBusy.value) return
    modelBusy.value = true
    modelError.value = null
    modelFeedback.value = ''
    try {
      modelFeedback.value = await api.testConnection(props.session, 'model')
    } catch (err) {
      modelError.value = parseAppError(err)
    } finally {
      modelBusy.value = false
    }
  } else {
    if (ovBusy.value) return
    ovBusy.value = true
    ovError.value = null
    ovFeedback.value = ''
    try {
      ovFeedback.value = await api.testConnection(props.session, 'memory')
    } catch (err) {
      ovError.value = parseAppError(err)
    } finally {
      ovBusy.value = false
    }
  }
}

async function saveMemory() {
  if (!props.session || ovBusy.value) return
  ovBusy.value = true
  ovError.value = null
  ovFeedback.value = ''
  try {
    await api.saveMemoryConfig(
      props.session,
      {
        baseUrl: ovEndpoint.value.trim(),
        namespace: ovNamespace.value.trim() || 'default',
        enabled: ovEnabled.value,
        syncResources: ovSyncResources.value,
        commitMemories: ovCommitMemories.value,
      },
      ovSecret.value.trim() || undefined,
    )
    ovSecret.value = ''
    await loadIntegrations()
    ovFeedback.value = 'OpenViking 配置已保存。'
  } catch (err) {
    ovError.value = parseAppError(err)
  } finally {
    ovBusy.value = false
  }
}

async function retrySyncMemory() {
  if (!props.session || ovBusy.value) return
  ovBusy.value = true
  ovError.value = null
  ovFeedback.value = ''
  try {
    const count = await api.retryMemorySync(props.session)
    ovFeedback.value = count > 0 ? `已将 ${count} 条未同步记录重新加入同步队列。` : '当前没有待补推的记录。'
  } catch (err) {
    ovError.value = parseAppError(err)
  } finally {
    ovBusy.value = false
  }
}

async function toggleCredentialLink(slot: CredentialSlot, linked: boolean) {
  if (!props.session) return
  try {
    await api.setCredential(props.session, slot, linked)
    await loadIntegrations()
    if (slot === 'model') modelFeedback.value = linked ? '已关联系统凭据。' : '已解除凭据关联。'
    else ovFeedback.value = linked ? '已关联系统凭据。' : '已解除凭据关联。'
  } catch (err) {
    if (slot === 'model') modelError.value = parseAppError(err)
    else ovError.value = parseAppError(err)
  }
}

async function handleRebuildIndex() {
  if (!props.session || rebuilding.value) return
  rebuilding.value = true
  rebuildError.value = null
  rebuildMessage.value = ''
  try {
    const res = await api.rebuildSearchIndex(props.session)
    rebuildMessage.value = `索引已更新：共索引 ${res.indexed} 篇文档${res.skipped ? `（跳过 ${res.skipped} 篇）` : ''}。`
    emit('indexRebuilt', res)
  } catch (err) {
    rebuildError.value = parseAppError(err)
  } finally {
    rebuilding.value = false
  }
}

function onKeydown(e: KeyboardEvent) {
  if (props.open && e.key === 'Escape') {
    e.preventDefault()
    emit('close')
  }
}

onMounted(() => window.addEventListener('keydown', onKeydown))
onUnmounted(() => window.removeEventListener('keydown', onKeydown))
</script>

<template>
  <div v-if="open" class="overlay-layer" role="dialog" aria-modal="true" aria-label="设置" @click.self="emit('close')">
    <div class="modal-window">
      <div class="hud-head">
        <strong style="font-size:14px;">设置 · 外观、AI 模型与数据范围</strong>
        <button type="button" class="icon-btn" aria-label="关闭设置" @click="emit('close')">&times;</button>
      </div>

      <div class="modal-body">
        <!-- 1. Appearance & Reading Width -->
        <section class="setting-card">
          <strong>1. 外观与阅读宽度</strong>
          <div class="form-grid">
            <div class="field">
              <label for="set-theme">界面主题</label>
              <select id="set-theme" v-model="theme">
                <option value="slate">深色 · Cognitive Slate（默认）</option>
                <option value="paper">浅色 · Paper Daylight</option>
              </select>
            </div>
            <div class="field">
              <label for="set-width">默认文档宽度</label>
              <select id="set-width" v-model="docWidth">
                <option value="readable">适读宽度（920px）</option>
                <option value="full">铺满整栏（宽屏满宽）</option>
              </select>
            </div>
            <div class="field">
              <label for="set-buffer-sec">随手记合并缓冲秒数（0–300 秒）</label>
              <input id="set-buffer-sec" v-model.number="bufferSeconds" type="number" min="0" max="300">
            </div>
            <div class="field" style="justify-content:flex-end;">
              <label style="display:flex;align-items:center;gap:6px;cursor:pointer;">
                <input v-model="autoOrganize" type="checkbox"> 随手记保存后自动加入整理队列
              </label>
              <label style="display:flex;align-items:center;gap:6px;cursor:pointer;">
                <input v-model="autoSyncMemory" type="checkbox"> 整理完成后自动同步至记忆服务
              </label>
            </div>
          </div>
          <div style="display:flex;align-items:center;justify-content:space-between;gap:8px;">
            <span class="hint">主题与文档宽度保存后会立即同步并记住。</span>
            <button type="button" class="btn primary sm" :disabled="savingGeneral" @click="saveGeneralSettings">
              {{ savingGeneral ? '保存中…' : '保存外观与偏好' }}
            </button>
          </div>
          <p v-if="generalFeedback" class="notice success" role="status">{{ generalFeedback }}</p>
          <ErrorNotice :error="generalError" />
        </section>

        <!-- 2. OpenAI-Compatible Model -->
        <section v-if="session" class="setting-card">
          <div style="display:flex;justify-content:space-between;align-items:center;gap:8px;">
            <strong>2. OpenAI 兼容模型与自定义网关</strong>
            <span class="pill success">API Key 仅存 Windows 凭据管理器</span>
          </div>

          <ErrorNotice v-if="integrationsError" :error="integrationsError" />

          <div class="form-grid">
            <div class="field full">
              <label for="set-model-url">端点地址 Base URL（支持任意 OpenAI 兼容服务或本机网关）</label>
              <input
                id="set-model-url"
                v-model="modelEndpoint"
                type="url"
                spellcheck="false"
                placeholder="http://127.0.0.1:3425/v1 或 https://api.example.com/v1"
              >
            </div>
            <div class="field">
              <label for="set-model-name">主模型 ID</label>
              <div class="field-row">
                <input
                  id="set-model-name"
                  v-model="modelName"
                  type="text"
                  spellcheck="false"
                  :list="modelOptions.length ? 'wikra-model-options' : undefined"
                  placeholder="例如：gpt-4.1-mini"
                >
                <button
                  type="button"
                  class="btn sm"
                  :disabled="loadingModels || !modelEndpoint.trim()"
                  @click="fetchRemoteModels"
                >{{ loadingModels ? '拉取中…' : '获取列表' }}</button>
              </div>
              <datalist v-if="modelOptions.length" id="wikra-model-options">
                <option v-for="m in modelOptions" :key="m" :value="m" />
              </datalist>
            </div>
            <div class="field">
              <label for="set-model-key">API Key（留空保留已有密钥）</label>
              <input
                id="set-model-key"
                v-model="modelSecret"
                type="password"
                autocomplete="off"
                :placeholder="modelCred?.state === 'linked' ? '••••••••（已安全保存）' : '公网服务必填，本机按需填写'"
              >
            </div>
          </div>

          <label style="display:flex;align-items:center;gap:6px;font-size:12.5px;cursor:pointer;">
            <input v-model="modelEnabled" type="checkbox"> 启用 AI 整理与知识问答
          </label>

          <div style="display:flex;flex-wrap:wrap;justify-content:space-between;align-items:center;gap:8px;font-size:12px;">
            <span :style="{ color: modelIsLocal ? 'var(--text-secondary)' : 'var(--warning)' }">
              {{ modelIsLocal
                ? '地址为本机回环；若经代理转发至第三方服务，请在下方核对发送范围。'
                : '注意：当前为非本机远端地址，授权正文将发送至外部服务。' }}
            </span>
            <div style="display:flex;gap:6px;">
              <button type="button" class="btn sm" :disabled="modelBusy || loadingIntegrations" @click="testSlot('model')">
                测试连接
              </button>
              <button type="button" class="btn primary sm" :disabled="modelBusy || loadingIntegrations" @click="saveModel">
                {{ modelBusy ? '处理中…' : '保存模型配置' }}
              </button>
            </div>
          </div>

          <div v-if="modelCred" style="font-size:11.5px;color:var(--muted);display:flex;align-items:center;justify-content:space-between;gap:8px;">
            <span>系统凭据：{{ credentialLabels[modelCred.state] }}（<span class="mono">{{ modelCred.target }}</span>）</span>
            <button
              v-if="modelCred.state !== 'linked'"
              type="button"
              class="btn quiet xs"
              @click="toggleCredentialLink('model', true)"
            >关联已有凭据</button>
            <button
              v-else
              type="button"
              class="btn quiet xs"
              @click="toggleCredentialLink('model', false)"
            >解除关联</button>
          </div>
          <p v-if="modelFeedback" class="notice success" role="status">{{ modelFeedback }}</p>
          <ErrorNotice :error="modelError" />
        </section>

        <!-- 3. Local OpenViking Memory -->
        <section v-if="session" class="setting-card">
          <div style="display:flex;justify-content:space-between;align-items:center;gap:8px;">
            <strong>3. 记忆体系对接（本地 OpenViking）</strong>
            <span class="pill">{{ ovEnabled ? '已启用' : '纯本地模式' }}</span>
          </div>

          <div class="form-grid">
            <div class="field">
              <label for="set-ov-url">OpenViking 服务地址</label>
              <input id="set-ov-url" v-model="ovEndpoint" type="url" spellcheck="false" placeholder="http://127.0.0.1:1933">
            </div>
            <div class="field">
              <label for="set-ov-ns">工作区命名空间</label>
              <input id="set-ov-ns" v-model="ovNamespace" type="text" spellcheck="false" placeholder="default">
            </div>
            <div class="field full">
              <label for="set-ov-key">OpenViking API Key（仅存 Windows 凭据管理器，留空保留现有密钥）</label>
              <input
                id="set-ov-key"
                v-model="ovSecret"
                type="password"
                autocomplete="off"
                :placeholder="ovCred?.state === 'linked' ? '••••••••（已安全保存）' : '按服务端要求填写'"
              >
            </div>
          </div>

          <div style="display:flex;flex-wrap:wrap;gap:14px;font-size:12.5px;">
            <label style="display:flex;align-items:center;gap:6px;cursor:pointer;">
              <input v-model="ovEnabled" type="checkbox"> 启用 OpenViking 记忆服务
            </label>
            <label style="display:flex;align-items:center;gap:6px;cursor:pointer;">
              <input v-model="ovSyncResources" type="checkbox"> 同步原文摘要到资源区
            </label>
            <label style="display:flex;align-items:center;gap:6px;cursor:pointer;">
              <input v-model="ovCommitMemories" type="checkbox"> 提交整理要点抽取长期记忆
            </label>
          </div>

          <p v-if="!ovIsLocal" style="font-size:12px;color:var(--warning);">
            注意：非本机地址可能是共享服务，请确认数据可见范围。
          </p>

          <div style="display:flex;justify-content:flex-end;gap:6px;">
            <button v-if="ovEnabled" type="button" class="btn sm" :disabled="ovBusy" @click="retrySyncMemory">
              补推未同步笔记
            </button>
            <button type="button" class="btn sm" :disabled="ovBusy || loadingIntegrations" @click="testSlot('memory')">
              测试连接
            </button>
            <button type="button" class="btn primary sm" :disabled="ovBusy || loadingIntegrations" @click="saveMemory">
              {{ ovBusy ? '处理中…' : '保存 OpenViking 配置' }}
            </button>
          </div>
          <p v-if="ovFeedback" class="notice success" role="status">{{ ovFeedback }}</p>
          <ErrorNotice :error="ovError" />
        </section>

        <!-- 4. Unified Data Scope Matrix -->
        <section class="setting-card">
          <strong>4. 数据发送范围与安静整理偏好</strong>
          <table class="privacy-table">
            <thead>
              <tr>
                <th>数据类型</th>
                <th>AI 模型端点</th>
                <th>记忆服务</th>
                <th>本地只读共享</th>
              </tr>
            </thead>
            <tbody>
              <tr>
                <td>随手笔记原文</td>
                <td>
                  <label style="display:inline-flex;align-items:center;gap:5px;cursor:pointer;">
                    <input v-model="allowNoteFulltextToModel" type="checkbox"> 允许发送整理
                  </label>
                </td>
                <td><span class="pill">仅同步事件摘要</span></td>
                <td><span class="pill">默认不暴露</span></td>
              </tr>
              <tr>
                <td>收件箱收藏文章</td>
                <td>仅发标出重点（或整篇确认）</td>
                <td>仅同步摘要</td>
                <td>默认不暴露</td>
              </tr>
              <tr>
                <td>整理后的知识页</td>
                <td>按问答/整理范围发送</td>
                <td>按目录授权同步</td>
                <td><span class="pill">本地 Markdown 兼容</span></td>
              </tr>
            </tbody>
          </table>
          <div style="display:flex;flex-wrap:wrap;justify-content:space-between;align-items:center;gap:8px;">
            <label style="font-size:12px;color:var(--text-secondary);display:flex;align-items:center;gap:6px;cursor:pointer;">
              <input v-model="quietOrganize" type="checkbox">
              减少打扰模式：仅在明确观点冲突时提醒确认，常规更新静默完成
            </label>
            <button type="button" class="btn sm" :disabled="savingGeneral" @click="saveGeneralSettings">
              保存隐私与整理偏好
            </button>
          </div>
        </section>

        <!-- 5. Global Hotkey & Vault Management -->
        <section class="setting-card">
          <strong>5. 全局快捷键与当前知识库</strong>
          <div class="form-grid">
            <div v-if="desktop" class="field">
              <label for="set-hotkey">随手记全局热键</label>
              <div class="field-row">
                <select id="set-hotkey" v-model="hotkeyChoice" :disabled="hotkeyBusy || !hotkey">
                  <option v-for="c in hotkey?.choices || []" :key="c.id" :value="c.id">{{ c.label }}</option>
                </select>
                <button
                  type="button"
                  class="btn sm"
                  :disabled="hotkeyBusy || !hotkey || (hotkey.active && hotkeyChoice === hotkey.id)"
                  @click="emit('hotkeyRequest', hotkeyChoice)"
                >{{ hotkeyBusy ? '应用中…' : '应用热键' }}</button>
              </div>
              <ErrorNotice v-if="hotkeyError" :error="hotkeyError" />
            </div>

            <div class="field">
              <label>当前知识库目录</label>
              <div class="field-row">
                <input type="text" :value="vaultRoot || '尚未打开目录'" readonly>
                <button type="button" class="btn sm" @click="emit('openOnboarding')">切换目录</button>
              </div>
            </div>
          </div>

          <div
            v-if="session"
            style="padding-top:12px;border-top:1px solid var(--line-subtle);display:flex;flex-direction:column;gap:10px;"
          >
            <div class="form-grid">
              <div class="field">
                <label for="set-raw-dir">原始文件目录（raw）</label>
                <div class="field-row">
                  <input
                    id="set-raw-dir"
                    v-model="rawDirDraft"
                    type="text"
                    spellcheck="false"
                    :disabled="savingDirs"
                    placeholder="例如：raw 或 00-Inbox"
                  >
                  <button
                    v-if="desktop"
                    type="button"
                    class="btn sm"
                    :disabled="pickingDir !== null || savingDirs || !vaultRoot"
                    @click="pickWorkspaceDir('raw')"
                  >{{ pickingDir === 'raw' ? '选择中…' : '选择文件夹' }}</button>
                </div>
              </div>

              <div class="field">
                <label for="set-capture-dir">随手记保存目录</label>
                <div class="field-row">
                  <input
                    id="set-capture-dir"
                    v-model="captureDirDraft"
                    type="text"
                    spellcheck="false"
                    :disabled="savingDirs"
                    placeholder="例如：raw/notes 或 notes"
                  >
                  <button
                    v-if="desktop"
                    type="button"
                    class="btn sm"
                    :disabled="pickingDir !== null || savingDirs || !vaultRoot"
                    @click="pickWorkspaceDir('capture')"
                  >{{ pickingDir === 'capture' ? '选择中…' : '选择文件夹' }}</button>
                </div>
              </div>
            </div>

            <div style="display:flex;flex-wrap:wrap;align-items:center;justify-content:space-between;gap:8px;">
              <span class="hint" style="flex:1;min-width:240px;">
                该目录及子目录下的 Markdown 归为「原始文件」只读展示，收件箱新收藏的文章将保存至
                <code class="mono">{{ effectiveRawDirPreview }}/clips/</code>；修改目录不会移动或删除你磁盘上的已有文件。
              </span>
              <button
                type="button"
                class="btn primary sm"
                :disabled="savingDirs || !rawDirDraft.trim()"
                @click="saveWorkspaceDirs"
              >{{ savingDirs ? '保存中…' : '保存目录设置' }}</button>
            </div>
            <p v-if="dirsFeedback" class="notice success" role="status">{{ dirsFeedback }}</p>
            <ErrorNotice :error="dirsError" />
          </div>

          <div
            v-if="session"
            style="padding-top:10px;border-top:1px solid var(--line-subtle);display:flex;align-items:center;justify-content:space-between;gap:8px;"
          >
            <span class="hint">如外部批量增删了本地 Markdown 文件，可手动重建全文检索索引。</span>
            <button type="button" class="btn sm" :disabled="rebuilding" @click="handleRebuildIndex">
              {{ rebuilding ? '正在重建…' : '重建全文索引' }}
            </button>
          </div>
          <p v-if="rebuildMessage" class="notice success" role="status">{{ rebuildMessage }}</p>
          <ErrorNotice :error="rebuildError" />
        </section>
      </div>

      <div class="hud-foot">
        <span style="font-size:12px;color:var(--muted);">目录配置保存在 .wikra/ 目录，绝不破坏你的笔记原件</span>
        <button type="button" class="btn primary sm" @click="emit('close')">完成</button>
      </div>
    </div>
  </div>
</template>
