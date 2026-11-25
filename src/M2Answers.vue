<script setup lang="ts">
import { isTauri } from '@tauri-apps/api/core'
import { getCurrentWindow } from '@tauri-apps/api/window'
import { computed, defineAsyncComponent, nextTick, onMounted, onUnmounted, ref, watch } from 'vue'
import { api } from './api/client'
import { parseAppError, type UiError } from './api/errors'
import ErrorNotice from './components/ErrorNotice.vue'
import UiIcon from './UiIcon.vue'
import {
  citationKindLabels,
  conflictStatusLabels,
  conflictStatusTone,
  type ChatMessage,
  type CitationItem,
  type ConflictItem,
  type ConversationSummary,
} from './m2'

const MarkdownPreview = defineAsyncComponent(() => import('./MarkdownPreview.vue'))

const props = defineProps<{
  session: string
  modelReady: boolean
  memoryEnabled: boolean
}>()

const emit = defineEmits<{
  openCitation: [citation: CitationItem]
  openConflicts: []
  openSettings: []
  focusSearch: []
}>()

const desktopWindow = isTauri() ? getCurrentWindow() : undefined
const conversations = ref<ConversationSummary[]>([])
const activeId = ref<string>()
const messages = ref<ChatMessage[]>([])
const listLoading = ref(false)
const sending = ref(false)
const streamingText = ref('')
const composing = ref(false)
const prompt = ref('')
const error = ref<UiError | null>(null)
const copyFeedback = ref('')
const scrollContainer = ref<HTMLElement>()
const unlisteners: (() => void)[] = []
let disposed = false

const showHistorySelect = computed(() => conversations.value.length > 0)

function trackUnlisten(fn: () => void) {
  if (disposed) fn()
  else unlisteners.push(fn)
}

function scrollToBottom() {
  void nextTick(() => {
    const el = scrollContainer.value
    if (el) el.scrollTop = el.scrollHeight
  })
}

async function loadConversations() {
  if (!props.session) return
  listLoading.value = true
  try {
    const list = await api.listConversations(props.session)
    conversations.value = list
    if (!activeId.value && list.length) {
      await selectConversation(list[0].id)
    } else if (activeId.value && !list.some((c) => c.id === activeId.value)) {
      activeId.value = list[0]?.id
      if (activeId.value) await selectConversation(activeId.value)
      else messages.value = []
    }
  } catch (err) {
    error.value = parseAppError(err)
  } finally {
    listLoading.value = false
  }
}

async function selectConversation(id: string) {
  if (!id || !props.session) {
    activeId.value = undefined
    messages.value = []
    return
  }
  activeId.value = id
  error.value = null
  try {
    const detail = await api.getConversation(props.session, id)
    if (activeId.value === id) {
      messages.value = detail.messages
      scrollToBottom()
    }
  } catch (err) {
    error.value = parseAppError(err)
  }
}

function startNewConversation() {
  activeId.value = undefined
  messages.value = []
  streamingText.value = ''
  error.value = null
}

async function deleteCurrentConversation() {
  const id = activeId.value
  if (!id || sending.value) return
  try {
    await api.deleteConversation(props.session, id)
    activeId.value = undefined
    messages.value = []
    await loadConversations()
  } catch (err) {
    error.value = parseAppError(err)
  }
}

async function send() {
  const text = prompt.value.trim()
  if (!text || !props.modelReady || sending.value || composing.value) return
  const targetSession = props.session
  const convId = activeId.value
  sending.value = true
  streamingText.value = ''
  error.value = null
  copyFeedback.value = ''

  const optimisticUser: ChatMessage = {
    id: `pending-${Date.now()}`,
    conversationId: convId ?? '',
    role: 'user',
    content: text,
    citations: [],
    relatedConflicts: [],
    createdAt: new Date().toISOString(),
  }
  messages.value = [...messages.value, optimisticUser]
  prompt.value = ''
  scrollToBottom()

  try {
    const turn = await api.sendChatMessage(targetSession, convId, text)
    if (props.session !== targetSession) return
    activeId.value = turn.conversation.id
    messages.value = [
      ...messages.value.filter((m) => m.id !== optimisticUser.id),
      turn.userMessage,
      turn.assistantMessage,
    ]
    streamingText.value = ''
    await loadConversations()
    scrollToBottom()
  } catch (err) {
    messages.value = messages.value.filter((m) => m.id !== optimisticUser.id)
    prompt.value = text
    error.value = parseAppError(err)
  } finally {
    sending.value = false
    streamingText.value = ''
  }
}

function handleKeydown(event: KeyboardEvent) {
  if (composing.value || event.isComposing || event.keyCode === 229) return
  if (event.key === 'Enter' && !event.shiftKey) {
    event.preventDefault()
    void send()
  }
}

async function copyWithCitations(msg: ChatMessage) {
  copyFeedback.value = ''
  const lines = [msg.content.trim()]
  if (msg.citations.length) {
    lines.push('', '参考来源：')
    for (const c of msg.citations) {
      lines.push(`[${c.index}] ${citationKindLabels[c.kind]} ${c.title || c.path} (${c.path})`)
    }
  }
  if (msg.relatedConflicts.length) {
    lines.push('', '关联观点冲突：')
    for (const item of msg.relatedConflicts) {
      lines.push(`- ${item.topic}（${conflictStatusLabels[item.status]}）：${item.currentText} / ${item.incomingText}`)
    }
  }
  try {
    await navigator.clipboard.writeText(lines.join('\n'))
    copyFeedback.value = '已复制回答及引用出处'
  } catch {
    copyFeedback.value = '无法写入剪贴板，请直接划选文本复制'
  }
}

function conflictSummary(c: ConflictItem) {
  return `${c.currentLabel}: ${c.currentText}  ⇄  ${c.incomingLabel}: ${c.incomingText}`
}

watch(() => props.session, () => {
  activeId.value = undefined
  messages.value = []
  void loadConversations()
}, { immediate: true })

onMounted(async () => {
  if (desktopWindow) {
    trackUnlisten(await desktopWindow.listen<{ conversationId: string; delta: string }>('chat-chunk', ({ payload }) => {
      if (!sending.value) return
      if (!activeId.value || payload.conversationId === activeId.value) {
        streamingText.value += payload.delta
        scrollToBottom()
      }
    }))
  }
})

onUnmounted(() => {
  disposed = true
  for (const fn of unlisteners.splice(0)) fn()
})
</script>

<template>
  <aside class="pane pane-right" aria-label="多轮记忆对话">
    <header class="chat-header">
      <div class="actions">
        <UiIcon name="chat" />
        <h2>记忆对话</h2>
      </div>
      <div class="actions">
        <select
          v-if="showHistorySelect"
          aria-label="切换历史对话"
          class="chat-history-select"
          :value="activeId ?? ''"
          :disabled="sending || listLoading"
          @change="selectConversation(($event.target as HTMLSelectElement).value)"
        >
          <option value="" disabled>选择历史会话…</option>
          <option v-for="item in conversations" :key="item.id" :value="item.id">{{ item.title }}</option>
        </select>
        <button type="button" class="quiet compact-btn" :disabled="sending" @click="startNewConversation">
          <UiIcon name="plus" />新对话
        </button>
        <button
          v-if="activeId"
          type="button"
          class="quiet compact-btn"
          title="删除当前对话"
          :disabled="sending"
          @click="deleteCurrentConversation"
        >
          删
        </button>
      </div>
    </header>

    <div ref="scrollContainer" class="pane-scroll chat-messages">
      <div v-if="!modelReady" class="empty-panel compact">
        <p>启用模型后可在这里检索笔记并继续追问。</p>
        <div class="actions">
          <button type="button" class="primary" @click="emit('openSettings')">配置模型</button>
          <button type="button" class="quiet" @click="emit('focusSearch')">用左侧搜索</button>
        </div>
      </div>

      <div v-else-if="!messages.length && !sending" class="empty-panel compact">
        <p class="hint">提问可检索{{ memoryEnabled ? '本地笔记与 OpenViking 记忆和资源' : '本地笔记' }}。点击引用可查看原文。</p>
      </div>

      <article
        v-for="msg in messages"
        :key="msg.id"
        class="chat-bubble"
        :class="msg.role === 'user' ? 'chat-user' : 'chat-assistant'"
      >
        <div v-if="msg.role === 'user'" class="chat-user-text">{{ msg.content }}</div>
        <template v-else>
          <MarkdownPreview
            :source="msg.content"
            :session="session"
            :citations="msg.citations"
            compact
            @cite="emit('openCitation', $event)"
          />
          <div v-if="msg.citations.length" class="citation-pills">
            <button
              v-for="cite in msg.citations"
              :key="cite.index"
              type="button"
              class="cite-chip"
              :title="cite.snippet || cite.title || cite.path"
              @click="emit('openCitation', cite)"
            >
              <span>[{{ cite.index }}] {{ citationKindLabels[cite.kind] }}</span>
              <span class="cite-title">{{ cite.title || cite.path }}</span>
            </button>
          </div>
          <div v-if="msg.relatedConflicts.length" class="chat-conflicts">
            <div v-for="c in msg.relatedConflicts" :key="c.id" class="chat-conflict-card">
              <div class="m2-card-header">
                <span class="badge" :class="conflictStatusTone[c.status]">{{ conflictStatusLabels[c.status] }}</span>
                <strong>{{ c.topic }}</strong>
                <button type="button" class="quiet compact-btn" @click="emit('openConflicts')">查看 Diff</button>
              </div>
              <p class="hint">{{ conflictSummary(c) }}</p>
            </div>
          </div>
          <div class="chat-actions">
            <button type="button" class="quiet compact-btn" @click="copyWithCitations(msg)">
              <UiIcon name="copy" />复制（含引用出处）
            </button>
          </div>
          <p v-if="msg.notice" class="hint" role="status">{{ msg.notice }}</p>
        </template>
      </article>

      <article v-if="sending" class="chat-bubble chat-assistant">
        <MarkdownPreview v-if="streamingText" :source="streamingText" compact />
        <p v-else class="hint" role="status">正在检索并生成回答…</p>
      </article>

      <p v-if="copyFeedback" class="notice success compact-notice" role="status">{{ copyFeedback }}</p>
      <ErrorNotice :error="error" />
    </div>

    <form class="chat-composer" @submit.prevent="send">
      <textarea
        v-model="prompt"
        rows="2"
        :disabled="!modelReady || sending"
        :placeholder="modelReady ? '问点什么…（Enter 发送，Shift+Enter 换行）' : '请先启用模型…'"
        @compositionstart="composing = true"
        @compositionend="composing = false"
        @keydown="handleKeydown"
      />
      <div class="chat-composer-footer">
        <span class="hint">自由划词 `Ctrl+C` 复制已恢复</span>
        <button type="submit" class="primary compact-btn" :disabled="!modelReady || sending || !prompt.trim()">
          {{ sending ? '生成中…' : '发送' }}
        </button>
      </div>
    </form>
  </aside>
</template>
