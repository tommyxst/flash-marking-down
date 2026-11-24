<script setup lang="ts">
import { computed, nextTick, ref, watch } from 'vue'
import { api } from '../api/client'
import { parseAppError, type UiError } from '../api/errors'
import ErrorNotice from './ErrorNotice.vue'
import UiIcon from '../UiIcon.vue'
import type {
  CredentialReference,
  CredentialSlot,
  CredentialState,
  HotkeyStatus,
  ModelConfig,
  OpenedWorkspace,
} from '../m2'

const credentialStates: Record<CredentialState, string> = {
  unlinked: '未关联',
  linked: '已关联',
  missing: '已关联的凭据在系统中缺失',
  unavailable: '暂时无法查询系统凭据',
  invalid: '引用与当前工作区不匹配',
}

const props = defineProps<{
  open: boolean
  desktop: boolean
  opening: boolean
  workspace?: { session: string; root: string; captureDir: string }
  initialWorkspaceError: UiError | null
  hotkey?: HotkeyStatus
  hotkeyBusy: boolean
  hotkeyError: UiError | null
  modelConfig: ModelConfig | null
}>()

const emit = defineEmits<{
  'update:open': [open: boolean]
  workspaceOpened: [opened: OpenedWorkspace]
  hotkeyRequest: [choice?: string]
  modelUpdated: [config: ModelConfig | null]
  desktopAction: [action: 'drag' | 'close']
}>()

const dialogEl = ref<HTMLDialogElement>()
const hotkeySelect = ref<HTMLSelectElement>()
const modelEndpointInput = ref<HTMLInputElement>()

const rootInput = ref('')
const captureInput = ref('')
const pickingFolder = ref(false)
const workspaceBusy = ref(false)
const workspaceError = ref<UiError | null>(null)
const settingsWindowError = ref<UiError | null>(null)

const hotkeyChoice = ref('')

const modelEndpoint = ref('')
const modelName = ref('')
const modelEnabled = ref(false)
const modelBusy = ref(false)
const modelError = ref<UiError | null>(null)
const modelFeedback = ref('')

const credentials = ref<CredentialReference[]>([])
const credentialBusy = ref(false)
const credentialError = ref<UiError | null>(null)
const credentialSecret = ref('')
const credentialFeedback = ref('')

const isBusy = computed(() => props.opening || workspaceBusy.value)
const activeHotkeyLabel = computed(() => {
  const status = props.hotkey
  if (status?.active) return status.choices.find((choice) => choice.id === status.id)?.label
  return undefined
})
const modelCredential = computed(() => credentials.value.find((item) => item.slot === 'model'))
const modelCredentialState = computed(() => modelCredential.value?.state)
const modelReady = computed(() => Boolean(props.modelConfig?.ready))

watch([() => props.open, dialogEl], ([open, dialog]) => {
  if (open && dialog && !dialog.open) dialog.showModal()
  if (!open && dialog?.open) dialog.close()
  if (open) {
    rootInput.value = props.workspace?.root ?? ''
    captureInput.value = props.workspace?.captureDir ?? ''
    workspaceError.value = props.initialWorkspaceError
    settingsWindowError.value = null
    if (props.workspace) void refreshCredentials(props.workspace.session)
  } else {
    credentialSecret.value = ''
    credentialFeedback.value = ''
  }
}, { flush: 'post' })

watch(() => props.initialWorkspaceError, (err) => {
  if (props.open && !workspaceBusy.value) workspaceError.value = err
})

watch(() => props.hotkey, (status) => {
  if (status && (!hotkeyChoice.value || !status.choices.some((c) => c.id === hotkeyChoice.value))) {
    hotkeyChoice.value = status.id
  } else if (status && status.active) {
    hotkeyChoice.value = status.id
  }
}, { immediate: true })

watch(() => props.modelConfig, (config) => {
  if (config) {
    modelEndpoint.value = config.endpoint
    modelName.value = config.model
    modelEnabled.value = config.enabled
  } else {
    modelEndpoint.value = modelName.value = ''
    modelEnabled.value = false
  }
}, { immediate: true })

watch(() => props.workspace?.session, (session) => {
  credentials.value = []
  credentialSecret.value = ''
  credentialFeedback.value = ''
  credentialError.value = null
  modelFeedback.value = ''
  modelError.value = null
  if (session && props.open) void refreshCredentials(session)
})

function cancelSettings() {
  if (isBusy.value || pickingFolder.value || !props.workspace) return
  workspaceError.value = null
  emit('update:open', false)
}

async function pickFolder(kind: 'root' | 'capture') {
  if (isBusy.value || pickingFolder.value) return
  pickingFolder.value = true
  workspaceError.value = null
  try {
    const selection = await api.pickWorkspaceFolder(kind, rootInput.value, captureInput.value)
    if (selection === null) return
    if (kind === 'capture') {
      captureInput.value = selection
    } else if (selection !== rootInput.value) {
      rootInput.value = selection
      captureInput.value = ''
    }
  } catch (error) {
    workspaceError.value = parseAppError(error)
  } finally {
    pickingFolder.value = false
  }
}

async function openWorkspace() {
  if (isBusy.value || pickingFolder.value) return
  workspaceBusy.value = true
  workspaceError.value = null
  try {
    const opened = await api.openWorkspace(rootInput.value, captureInput.value)
    emit('workspaceOpened', opened)
    emit('update:open', false)
  } catch (error) {
    workspaceError.value = parseAppError(error)
  } finally {
    workspaceBusy.value = false
  }
}

async function refreshModelConfig(session: string) {
  modelBusy.value = true
  modelError.value = null
  modelFeedback.value = ''
  try {
    const config = await api.getModelConfig(session)
    if (props.workspace?.session === session) emit('modelUpdated', config)
  } catch (error) {
    if (props.workspace?.session === session) modelError.value = parseAppError(error)
  } finally {
    if (props.workspace?.session === session) modelBusy.value = false
  }
}

async function saveModelConfig() {
  const session = props.workspace?.session
  if (!session || modelBusy.value || isBusy.value) return
  modelBusy.value = true
  modelError.value = null
  modelFeedback.value = ''
  try {
    const config = await api.setModelConfig(session, modelEndpoint.value.trim(), modelName.value.trim(), modelEnabled.value)
    if (props.workspace?.session === session) {
      emit('modelUpdated', config)
      modelFeedback.value = '已保存模型配置。'
    }
  } catch (error) {
    if (props.workspace?.session === session) modelError.value = parseAppError(error)
  } finally {
    if (props.workspace?.session === session) modelBusy.value = false
  }
}

async function refreshCredentials(session: string) {
  credentialBusy.value = true
  credentialError.value = null
  try {
    const references = await api.listCredentials(session)
    if (props.workspace?.session === session) credentials.value = references
  } catch (error) {
    if (props.workspace?.session === session) credentialError.value = parseAppError(error)
  } finally {
    if (props.workspace?.session === session) credentialBusy.value = false
  }
}

async function syncModelReady(session: string) {
  try {
    const config = await api.getModelConfig(session)
    if (props.workspace?.session === session) emit('modelUpdated', config)
  } catch {
    // Model config refresh failure will surface if the user edits or saves model settings.
  }
}

async function setCredential(slot: CredentialSlot, linked: boolean) {
  const session = props.workspace?.session
  if (!session || credentialBusy.value || isBusy.value) return
  credentialBusy.value = true
  credentialError.value = null
  credentialFeedback.value = ''
  try {
    const reference = await api.setCredential(session, slot, linked)
    if (props.workspace?.session === session) {
      credentials.value = credentials.value.map((entry) => entry.slot === slot ? reference : entry)
      credentialFeedback.value = linked ? '已关联系统凭据。' : '已解除凭据关联。'
      await syncModelReady(session)
    }
  } catch (error) {
    if (props.workspace?.session === session) credentialError.value = parseAppError(error)
  } finally {
    if (props.workspace?.session === session) credentialBusy.value = false
  }
}

async function saveCredential(slot: CredentialSlot) {
  const session = props.workspace?.session
  const secret = credentialSecret.value
  if (!session || !secret.trim() || credentialBusy.value || isBusy.value) return
  credentialBusy.value = true
  credentialError.value = null
  credentialFeedback.value = ''
  try {
    const reference = await api.saveCredential(session, slot, secret)
    if (props.workspace?.session === session) {
      credentialSecret.value = ''
      credentials.value = credentials.value.map((entry) => entry.slot === slot ? reference : entry)
      credentialFeedback.value = '已写入 Windows 凭据管理器并自动关联（密钥未写入工作区）。'
      await syncModelReady(session)
    }
  } catch (error) {
    if (props.workspace?.session === session) credentialError.value = parseAppError(error)
  } finally {
    if (props.workspace?.session === session) credentialBusy.value = false
  }
}

async function copyTargetName(target: string) {
  credentialFeedback.value = ''
  credentialError.value = null
  try {
    await navigator.clipboard.writeText(target)
    credentialFeedback.value = '已复制凭据目标名称。'
  } catch {
    credentialError.value = { safeMessage: '无法访问剪贴板，请手动选中复制。' }
  }
}

defineExpose({
  focusModel: () => void nextTick(() => modelEndpointInput.value?.focus()),
  focusHotkey: () => void nextTick(() => hotkeySelect.value?.focus()),
  setWindowError: (err: UiError | null) => { settingsWindowError.value = err },
})
</script>

<template>
  <dialog ref="dialogEl" class="settings-dialog" aria-labelledby="settings-heading" @cancel.prevent="cancelSettings">
    <header>
      <UiIcon name="settings" />
      <h2 id="settings-heading">{{ workspace ? '工作区设置' : '打开工作区' }}</h2>
      <div v-if="desktop" class="window-drag" aria-hidden="true" @mousedown.left="emit('desktopAction', 'drag')" />
      <div v-if="workspace || desktop" class="window-controls">
        <button
          type="button"
          :aria-label="workspace ? '关闭工作区设置' : '关闭工作台'"
          :title="workspace ? '关闭设置' : '关闭工作台'"
          :disabled="isBusy || pickingFolder"
          @click="workspace ? cancelSettings() : emit('desktopAction', 'close')"
        ><UiIcon name="close" /></button>
      </div>
    </header>

    <form class="settings-form" @submit.prevent="openWorkspace">
      <label for="workspace-root">工作区根目录（绝对路径）</label>
      <div class="folder-field">
        <input id="workspace-root" v-model="rootInput" required :disabled="isBusy || pickingFolder" aria-describedby="workspace-root-help" spellcheck="false" autofocus>
        <button v-if="desktop" type="button" aria-label="选择工作区根目录" :disabled="isBusy || pickingFolder" @click="pickFolder('root')">选择文件夹</button>
      </div>
      <p id="workspace-root-help" class="hint">搜索和重建索引会读取工作区及其子目录中的可访问 Markdown 文件。</p>
      <label for="capture-dir">原文保存目录（相对于工作区）</label>
      <div class="folder-field">
        <input id="capture-dir" v-model="captureInput" :disabled="isBusy || pickingFolder" aria-describedby="capture-dir-help" spellcheck="false">
        <button v-if="desktop" type="button" aria-label="选择原文保存目录" :disabled="isBusy || pickingFolder || !rootInput.trim()" @click="pickFolder('capture')">选择文件夹</button>
      </div>
      <p id="capture-dir-help" class="hint">使用已有目录。留空时保存到工作区根目录。</p>
      <ErrorNotice :error="workspaceError" suffix="。请检查路径和访问权限后重试。" />
      <ErrorNotice :error="settingsWindowError" />
      <div class="actions settings-actions">
        <button v-if="workspace" type="button" :disabled="isBusy || pickingFolder" @click="cancelSettings">取消</button>
        <button class="primary" type="submit" :disabled="isBusy || pickingFolder">{{ isBusy ? '正在打开…' : '打开工作区' }}</button>
      </div>
    </form>

    <form v-if="desktop" class="settings-form settings-hotkey" @submit.prevent="emit('hotkeyRequest', hotkey ? hotkeyChoice : undefined)">
      <label :for="hotkey ? 'global-hotkey' : undefined">全局快捷键</label>
      <div v-if="hotkey" class="actions">
        <select id="global-hotkey" ref="hotkeySelect" v-model="hotkeyChoice" :disabled="hotkeyBusy">
          <option v-for="choice in hotkey.choices" :key="choice.id" :value="choice.id">{{ choice.label }}</option>
        </select>
        <button type="submit" :disabled="hotkeyBusy || (hotkey.active && hotkeyChoice === hotkey.id)">应用快捷键</button>
      </div>
      <button v-else-if="!hotkeyBusy" type="submit">重试读取快捷键</button>
      <p v-if="hotkeyBusy" class="hint" role="status">正在更新快捷键…</p>
      <p v-else-if="activeHotkeyLabel" class="hint" role="status">当前可用 <kbd>{{ activeHotkeyLabel }}</kbd></p>
      <ErrorNotice :error="hotkeyError" />
      <p class="hint">关闭工作台后，仍可从 Windows 托盘新建笔记、打开工作台或退出应用。</p>
    </form>

    <form v-if="workspace" class="settings-form settings-model" aria-labelledby="model-settings-heading" @submit.prevent="saveModelConfig">
      <h3 id="model-settings-heading">整理与问答模型</h3>
      <label for="model-endpoint">模型地址（完整 URL）</label>
      <input id="model-endpoint" ref="modelEndpointInput" v-model="modelEndpoint" type="url" required :disabled="modelBusy || isBusy" spellcheck="false" aria-describedby="model-endpoint-help">
      <p id="model-endpoint-help" class="hint">填写完整的 Chat Completions 地址（支持 query 参数，如 Azure api-version）。公网端点须使用 HTTPS 并配置 API Key；本机（localhost / 127.0.0.1 / [::1]）或局域网私有地址可使用 HTTP，并按需免凭据。</p>
      <label for="model-name">模型名称</label>
      <input id="model-name" v-model="modelName" required :disabled="modelBusy || isBusy" spellcheck="false">
      <label class="m2-check"><input v-model="modelEnabled" type="checkbox" :disabled="modelBusy || isBusy">启用整理与问答</label>
      <p class="hint">启用后，整理与问答将直接发送所选记录或命中片段至上述端点，无需逐次二次确认。</p>
      <p v-if="modelConfig?.enabled && !modelReady && !credentialBusy" class="notice" role="status">模型已启用。请展开模型凭据，录入 API Key 或关联系统凭据。</p>
      <p v-if="modelFeedback" class="notice success" role="status">{{ modelFeedback }}</p>
      <ErrorNotice :error="modelError" />
      <div class="actions">
        <button type="submit" :disabled="modelBusy || isBusy">{{ modelBusy ? '载入配置…' : '保存模型配置' }}</button>
        <button v-if="modelError" type="button" :disabled="modelBusy || isBusy" @click="refreshModelConfig(workspace.session)">重新读取</button>
      </div>
      <details class="settings-details" :key="workspace.session" :open="Boolean(modelConfig?.enabled && !modelReady)">
        <summary>模型凭据（API Key）</summary>
        <template v-if="modelCredential">
          <div class="credential-target-row">
            <span class="badge" :class="modelCredential.state === 'linked' ? 'badge-success' : modelCredential.state === 'missing' ? 'badge-error' : 'badge-muted'">{{ credentialStates[modelCredential.state] }}</span>
            <code>{{ modelCredential.target }}</code>
            <button type="button" class="quiet" @click="copyTargetName(modelCredential.target)"><UiIcon name="copy" />复制目标名</button>
          </div>
          <label for="model-api-key">录入或更新 API Key（直接安全写入 Windows 凭据管理器，不落盘到工作区）</label>
          <div class="folder-field">
            <input id="model-api-key" v-model="credentialSecret" type="password" autocomplete="off" placeholder="粘贴 API Key…" :disabled="credentialBusy || isBusy" spellcheck="false">
            <button type="button" class="primary" :disabled="credentialBusy || isBusy || !credentialSecret.trim()" @click="saveCredential('model')">保存并关联</button>
          </div>
          <p class="hint">提交后输入框立即清空；也可以自行在 Windows 凭据管理器中使用上方目标名称管理。</p>
        </template>
        <div class="actions">
          <button type="button" :disabled="credentialBusy || isBusy" @click="refreshCredentials(workspace.session)">检查凭据状态</button>
          <button v-if="modelCredentialState === 'unlinked' || modelCredentialState === 'missing'" type="button" :disabled="credentialBusy || isBusy" @click="setCredential('model', true)">关联已有系统凭据</button>
          <button v-if="modelCredentialState && modelCredentialState !== 'unlinked'" type="button" :disabled="credentialBusy || isBusy" @click="setCredential('model', false)">解除关联</button>
        </div>
        <p v-if="credentialFeedback" class="notice success" role="status">{{ credentialFeedback }}</p>
        <ErrorNotice :error="credentialError" />
      </details>
    </form>
  </dialog>
</template>
