<script setup lang="ts">
import { computed, nextTick, ref, watch } from 'vue'
import { api } from '../api/client'
import { parseAppError, type UiError } from '../api/errors'
import ErrorNotice from './ErrorNotice.vue'
import AutoActionsSettings from './AutoActionsSettings.vue'
import UiIcon from '../UiIcon.vue'
import type {
  CredentialReference,
  CredentialSlot,
  CredentialState,
  HotkeyStatus,
  IntegrationsConfig,
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
}>()

const emit = defineEmits<{
  'update:open': [open: boolean]
  workspaceOpened: [opened: OpenedWorkspace]
  hotkeyRequest: [choice?: string]
  integrationsUpdated: [config: IntegrationsConfig]
  desktopAction: [action: 'drag' | 'close']
}>()

const dialogEl = ref<HTMLDialogElement>()
const hotkeySelect = ref<HTMLSelectElement>()
const rootInput = ref('')
const defaultCaptureDir = 'notes'
const captureInput = ref(defaultCaptureDir)
const pickingFolder = ref(false)
const workspaceBusy = ref(false)
const workspaceError = ref<UiError | null>(null)
const settingsWindowError = ref<UiError | null>(null)

const hotkeyChoice = ref('')

const slots = ['model', 'memory'] as const
function emptyForm() {
  return {
    endpoint: '', name: '', enabled: false, secret: '',
    syncResources: true, commitMemories: true,
    saved: null as {
      endpoint: string; name: string; enabled: boolean; syncResources?: boolean; commitMemories?: boolean
    } | null,
    credential: undefined as CredentialReference | undefined,
    pending: '' as '' | 'save' | 'test' | 'link' | 'retry',
    error: null as UiError | null, feedback: '',
  }
}
const forms = ref({ model: emptyForm(), memory: emptyForm() })
const loading = ref(false)
const loadError = ref<UiError | null>(null)
const modelOptions = ref<string[]>([])
const loadingModels = ref(false)

function dirty(slot: CredentialSlot) {
  const f = forms.value[slot]
  return !f.saved || f.endpoint.trim() !== f.saved.endpoint || f.name.trim() !== f.saved.name ||
    f.enabled !== f.saved.enabled || Boolean(f.secret.trim()) ||
    (slot === 'memory' && (f.syncResources !== f.saved.syncResources || f.commitMemories !== f.saved.commitMemories))
}

const isBusy = computed(() => props.opening || workspaceBusy.value)
const activeHotkeyLabel = computed(() => {
  const status = props.hotkey
  if (status?.active) return status.choices.find((choice) => choice.id === status.id)?.label
  return undefined
})
watch([() => props.open, dialogEl], ([open, dialog]) => {
  if (open && dialog && !dialog.open) dialog.showModal()
  if (!open && dialog?.open) dialog.close()
  if (open) {
    rootInput.value = props.workspace?.root ?? ''
    captureInput.value = props.workspace?.captureDir || defaultCaptureDir
    workspaceError.value = props.initialWorkspaceError
    settingsWindowError.value = null
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

watch([() => props.open, () => props.workspace?.session], ([open, session]) => {
  forms.value = { model: emptyForm(), memory: emptyForm() }
  modelOptions.value = []
  loadingModels.value = false
  loading.value = false
  loadError.value = null
  if (open && session) void loadIntegrations(session)
}, { immediate: true })

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
      captureInput.value = defaultCaptureDir
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

function updateForm(config: IntegrationsConfig, slot: CredentialSlot) {
  const form = forms.value[slot]
  const value = slot === 'model' && config.model
    ? { endpoint: config.model.endpoint, name: config.model.model, enabled: config.model.enabled }
    : slot === 'memory' && config.memory
      ? {
          endpoint: config.memory.baseUrl,
          name: config.memory.namespace,
          enabled: config.memory.enabled,
          syncResources: config.memory.syncResources,
          commitMemories: config.memory.commitMemories,
        }
      : null
  if (value) Object.assign(form, value)
  form.saved = value
  form.credential = config.credentials.find(item => item.slot === slot)
}

async function loadIntegrations(session: string) {
  const current = forms.value
  loading.value = true
  loadError.value = null
  try {
    const config = await api.getIntegrationsConfig(session)
    if (current !== forms.value) return
    for (const slot of slots) updateForm(config, slot)
    emit('integrationsUpdated', config)
  } catch (error) {
    if (current === forms.value) loadError.value = parseAppError(error)
  } finally {
    if (current === forms.value) loading.value = false
  }
}

async function fetchModels() {
  const session = props.workspace?.session
  const form = forms.value.model
  if (!session || loadingModels.value || form.pending || isBusy.value || loading.value || loadError.value) return
  if (!form.endpoint.trim()) return
  loadingModels.value = true
  form.error = null
  form.feedback = ''
  try {
    const models = await api.listRemoteModels(session, form.endpoint, form.secret.trim() || undefined)
    if (form !== forms.value.model) return
    modelOptions.value = models
    if (!form.name.trim() && models.length) form.name = models[0]
    form.feedback = `已获取 ${models.length} 个可用模型，可在输入框下拉选择。`
  } catch (error) {
    if (form === forms.value.model) form.error = parseAppError(error)
  } finally {
    loadingModels.value = false
  }
}

async function apply(slot: CredentialSlot, action: 'save' | 'test' | 'link' | 'retry', linked = false) {
  const session = props.workspace?.session
  const form = forms.value[slot]
  if (!session || form.pending || isBusy.value || loading.value || loadError.value) return
  form.pending = action
  form.error = null
  form.feedback = ''
  try {
    if (action === 'test') {
      form.feedback = await api.testConnection(session, slot)
      return
    }
    if (action === 'retry') {
      const count = await api.retryMemorySync(session)
      form.feedback = count ? `已将 ${count} 个同步任务加入队列。` : '没有新增同步任务。'
      return
    }
    if (action === 'link') {
      form.credential = await api.setCredential(session, slot, linked)
      form.feedback = linked ? '已关联系统凭据。' : '已解除关联。'
    } else {
      const secret = form.secret.trim() || undefined
      if (slot === 'model') {
        await api.saveModelConfig(session, form.endpoint, form.name, form.enabled, secret)
      } else {
        await api.saveMemoryConfig(session, {
          baseUrl: form.endpoint,
          namespace: form.name,
          enabled: form.enabled,
          syncResources: form.syncResources,
          commitMemories: form.commitMemories,
        }, secret)
      }
      form.secret = ''
      form.feedback = '已保存配置。'
    }
    const config = await api.getIntegrationsConfig(session)
    if (form !== forms.value[slot]) return
    if (action === 'save') updateForm(config, slot)
    emit('integrationsUpdated', config)
  } catch (error) {
    form.error = parseAppError(error)
  } finally {
    form.pending = ''
  }
}

defineExpose({
  focusModel: () => void nextTick(() => dialogEl.value?.querySelector<HTMLInputElement>('#model-endpoint')?.focus()),
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
      <label for="workspace-root">工作区 / Wiki 根目录（绝对路径）</label>
      <div class="folder-field">
        <input id="workspace-root" v-model="rootInput" required :disabled="isBusy || pickingFolder" aria-describedby="workspace-root-help" spellcheck="false" autofocus>
        <button v-if="desktop" type="button" aria-label="选择工作区根目录" :disabled="isBusy || pickingFolder" @click="pickFolder('root')">选择文件夹</button>
      </div>
      <p id="workspace-root-help" class="hint">搜索和重建索引会读取工作区及其子目录中的可访问 Markdown 文件。</p>
      <label for="capture-dir">笔记存放目录（工作区内子文件夹，留空默认 notes）</label>
      <div class="folder-field">
        <input id="capture-dir" v-model="captureInput" :disabled="isBusy || pickingFolder" aria-describedby="capture-dir-help" placeholder="notes" spellcheck="false">
        <button v-if="desktop" type="button" aria-label="选择笔记存放目录" :disabled="isBusy || pickingFolder || !rootInput.trim()" @click="pickFolder('capture')">选择文件夹</button>
      </div>
      <p id="capture-dir-help" class="hint">笔记会专门存放在该子文件夹内（默认自动创建 notes 文件夹），避免直接散落在 Wiki 根目录下。</p>
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

    <template v-if="workspace">
      <AutoActionsSettings :open="open" />
      <div v-if="loadError" class="settings-form">
        <ErrorNotice :error="loadError" />
        <button type="button" @click="loadIntegrations(workspace.session)">重新读取服务配置</button>
      </div>
      <form v-for="slot in slots" :key="slot" class="settings-form settings-model"
        :aria-labelledby="slot + '-settings-heading'" @submit.prevent="apply(slot, 'save')">
        <h3 :id="slot + '-settings-heading'">{{ slot === 'model' ? '整理与问答模型' : 'OpenViking 记忆服务' }}</h3>
        <fieldset class="settings-form service-fields" :disabled="Boolean(forms[slot].pending || loading || loadError || isBusy)"
          @input="forms[slot].feedback = ''; forms[slot].error = null">
          <label :for="slot + '-endpoint'">{{ slot === 'model' ? '模型地址（支持 Base URL 如 http://127.0.0.1:3425/v1 或完整 URL）' : 'OpenViking 服务地址' }}</label>
          <input :id="slot + '-endpoint'" v-model="forms[slot].endpoint" type="url" required spellcheck="false"
            :placeholder="slot === 'model' ? 'http://127.0.0.1:3425/v1 或 https://…/v1/chat/completions' : 'http://localhost:1933'">
          <label :for="slot + '-name'">{{ slot === 'model' ? '模型名称' : '工作区命名空间' }}</label>
          <div v-if="slot === 'model'" class="folder-field">
            <input :id="slot + '-name'" v-model="forms[slot].name" required spellcheck="false"
              :list="modelOptions.length ? 'remote-model-options' : undefined"
              placeholder="例如：auto/gemini-3.8-flash">
            <button type="button" :disabled="loadingModels || !forms.model.endpoint.trim()" @click="fetchModels">
              {{ loadingModels ? '正在获取…' : '获取模型列表' }}
            </button>
          </div>
          <input v-else :id="slot + '-name'" v-model="forms[slot].name" required spellcheck="false" placeholder="例如：default">
          <datalist v-if="slot === 'model' && modelOptions.length" id="remote-model-options">
            <option v-for="id in modelOptions" :key="id" :value="id" />
          </datalist>
          <label :for="slot + '-api-key'">API Key（保存在 Windows 凭据管理器）</label>
          <input :id="slot + '-api-key'" v-model="forms[slot].secret" type="password" autocomplete="off"
            :placeholder="forms[slot].credential?.state === 'linked' ? '已关联，留空保留当前密钥' : '公网服务必填，本机或局域网按需填写'" spellcheck="false">
          <label class="m2-check"><input v-model="forms[slot].enabled" type="checkbox">
            {{ slot === 'model' ? '启用整理与问答' : '启用 OpenViking' }}
          </label>
          <p v-if="slot === 'model'" class="hint">整理与问答会向此地址发送所选记录或检索命中的片段。测试连接仅发送固定测试消息。</p>
          <template v-else>
            <label class="m2-check"><input v-model="forms.memory.syncResources" type="checkbox">同步每日原文到 resources</label>
            <label class="m2-check"><input v-model="forms.memory.commitMemories" type="checkbox">提交整理要点抽取长期 memories</label>
          </template>
          <div class="actions">
            <button class="primary" type="submit">{{ forms[slot].pending === 'save' ? '正在保存…' : slot === 'model' ? '保存模型配置' : '保存 OpenViking 配置' }}</button>
            <button type="button" :disabled="dirty(slot)" @click="apply(slot, 'test')">{{ forms[slot].pending === 'test' ? '正在连接…' : '测试连接' }}</button>
            <button
              v-if="slot === 'memory' && forms.memory.saved?.enabled"
              type="button"
              :disabled="dirty(slot) || !(forms.memory.syncResources || forms.memory.commitMemories)"
              @click="apply('memory', 'retry')"
            >{{ forms.memory.pending === 'retry' ? '正在补推…' : '补推未同步笔记' }}</button>
          </div>
          <p v-if="dirty(slot)" class="hint">保存当前配置后即可测试连接。</p>
          <details v-if="forms[slot].credential" class="settings-details">
            <summary>系统凭据 · {{ credentialStates[forms[slot].credential!.state] }}</summary>
            <code>{{ forms[slot].credential!.target }}</code>
            <div class="actions">
              <button v-if="forms[slot].credential!.state !== 'linked'" type="button" @click="apply(slot, 'link', true)">关联已有系统凭据</button>
              <button v-if="forms[slot].credential!.state !== 'unlinked'" type="button" @click="apply(slot, 'link', false)">解除关联</button>
            </div>
          </details>
        </fieldset>
        <p v-if="forms[slot].feedback" class="notice success" role="status">{{ forms[slot].feedback }}</p>
        <ErrorNotice :error="forms[slot].error" />
      </form>
    </template>
  </dialog>
</template>

<style scoped>
.service-fields {
  min-width: 0;
  margin: 0;
  padding: 0;
  border: 0;
}
</style>
