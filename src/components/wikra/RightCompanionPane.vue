<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { api } from '../../api/client'
import { parseAppError, type UiError } from '../../api/errors'
import ErrorNotice from '../ErrorNotice.vue'
import MarkdownPreview from '../../MarkdownPreview.vue'
import type {
  ChangesetItem,
  ChatMessage,
  CitationItem,
  ConversationSummary,
  DocCitation,
  DocumentDetail,
  InboxItemDetail,
  RawFileSummary,
  TeachbackTurn,
} from '../../m2'
import { shortDate } from '../../utils/wikra'

const props = defineProps<{
  session: string
  mode: 'library' | 'raw' | 'inbox' | 'organize' | 'review' | 'graph'
  activeTab: 'info' | 'chat'
  activeDoc: DocumentDetail | null
  activeInbox: InboxItemDetail | null
  wikiTitleMap: Record<string, string>
  rawDir: string
  rawFiles: RawFileSummary[]
  changesets: ChangesetItem[]
  chatMode: 'query' | 'review'
  teachbackPagePath: string | null
  teachbackPageTitle: string
  pendingAskPrompt: string | null
}>()

const emit = defineEmits<{
  'update:activeTab': [tab: 'info' | 'chat']
  openEvidence: [citation: DocCitation]
  openWiki: [path: string]
  openRaw: [path: string]
  goOrganize: []
  revertChangeset: [id: string]
  startTeachback: [pagePath: string]
  exitTeachback: []
  understandingAppended: [doc: DocumentDetail]
  answerSaved: [doc: DocumentDetail]
  clearPendingAsk: []
  toast: [message: string]
}>()

const inGraph = computed(() => props.mode === 'graph')
const infoLabel = computed(() => (props.mode === 'inbox' ? '导读与重点' : '文档信息'))

const pageHistory = computed(() => {
  if (!props.activeDoc) return []
  const path = props.activeDoc.path
  return props.changesets.filter((c) => c.pagePaths.includes(path)).slice(0, 3)
})

const inboxHighlights = computed(() =>
  (props.activeInbox?.paragraphs || []).filter((p) => p.highlighted),
)

function wikiTitle(path: string): string {
  return props.wikiTitleMap[path] || path.split('/').pop()?.replace(/\.md$/i, '') || path
}

// ============================================================================
// AI Chat State
// ============================================================================
const conversations = ref<ConversationSummary[]>([])
const activeConversationId = ref<string | undefined>(undefined)
const messages = ref<ChatMessage[]>([])
const chatInput = ref('')
const includeOneHop = ref(false)
const sendingChat = ref(false)
const chatError = ref<UiError | null>(null)
const savedMessageIds = ref<Record<string, boolean>>({})
const savingAnswerId = ref('')

async function loadConversations() {
  if (!props.session) return
  try {
    const list = await api.listConversations(props.session)
    conversations.value = list
    if (list.length > 0 && !activeConversationId.value) {
      await selectConversation(list[0].id)
    }
  } catch {
    // Keep silent until user opens chat.
  }
}

async function selectConversation(id: string) {
  if (!props.session) return
  activeConversationId.value = id
  chatError.value = null
  try {
    const detail = await api.getConversation(props.session, id)
    messages.value = detail.messages
  } catch (err) {
    chatError.value = parseAppError(err)
  }
}

function startNewConversation() {
  activeConversationId.value = undefined
  messages.value = []
  chatError.value = null
}

async function sendMessage(overrideText?: string) {
  const raw = (overrideText ?? chatInput.value).trim()
  if (!raw || !props.session || sendingChat.value) return
  sendingChat.value = true
  chatError.value = null

  let prompt = raw
  if (!overrideText && props.activeDoc) {
    const scopeHint = includeOneHop.value
      ? `（请结合《${props.activeDoc.title}》及其关联知识页回答）`
      : `（当前阅读：《${props.activeDoc.title}》）`
    prompt = `${raw}\n${scopeHint}`
  }

  if (!overrideText) chatInput.value = ''

  try {
    const turn = await api.sendChatMessage(props.session, activeConversationId.value, prompt)
    activeConversationId.value = turn.conversation.id
    messages.value = [...messages.value, turn.userMessage, turn.assistantMessage]
    if (!conversations.value.some((c) => c.id === turn.conversation.id)) {
      conversations.value = [turn.conversation, ...conversations.value]
    }
  } catch (err) {
    chatError.value = parseAppError(err)
  } finally {
    sendingChat.value = false
  }
}

function handleChatKeydown(e: KeyboardEvent) {
  if (e.ctrlKey && e.key === 'Enter') {
    e.preventDefault()
    void sendMessage()
  }
}

async function saveAnswer(msg: ChatMessage) {
  if (!props.session || !activeConversationId.value || savingAnswerId.value) return
  savingAnswerId.value = msg.id
  chatError.value = null
  try {
    const doc = await api.saveAnswerToWiki(props.session, activeConversationId.value, msg.id)
    savedMessageIds.value = { ...savedMessageIds.value, [msg.id]: true }
    emit('answerSaved', doc)
  } catch (err) {
    chatError.value = parseAppError(err)
  } finally {
    savingAnswerId.value = ''
  }
}

async function copyAnswer(content: string) {
  try {
    await navigator.clipboard.writeText(content)
    emit('toast', '已复制回答内容')
  } catch {
    emit('toast', '无法访问剪贴板，请手动复制')
  }
}

function normalizeRelPath(path: string): string {
  return path.trim().replaceAll('\\', '/').replace(/^\.\/+/, '').replace(/^\/+/, '').replace(/\/+$/, '')
}

function isRawCitationPath(path: string): boolean {
  const norm = normalizeRelPath(path).toLowerCase()
  if (!norm) return false
  if (props.rawFiles.some((r) => normalizeRelPath(r.path).toLowerCase() === norm)) {
    return true
  }
  const rawPrefix = normalizeRelPath(props.rawDir || 'raw').toLowerCase()
  return Boolean(rawPrefix && (norm === rawPrefix || norm.startsWith(`${rawPrefix}/`)))
}

function handleChatCitation(citation: CitationItem) {
  if (citation.kind !== 'local') {
    emit('toast', `记忆引用：${citation.title}`)
    return
  }
  const matchedDocCit = props.activeDoc?.citations.find(
    (c) => c.rawPath === citation.path || c.label === citation.title,
  )
  if (matchedDocCit) {
    emit('openEvidence', matchedDocCit)
    return
  }
  if (isRawCitationPath(citation.path)) {
    emit('openEvidence', {
      id: `chat-cit-${citation.index}`,
      refLabel: `s${citation.index}`,
      rawPath: citation.path,
      label: citation.title,
      quote: citation.snippet,
    })
  } else {
    emit('openWiki', citation.path)
  }
}

// ============================================================================
// Active Recall ("让我讲一遍") 3-Step Loop
// ============================================================================
const teachbackTurn = ref<TeachbackTurn | null>(null)
const teachbackAnswer = ref('')
const teachbackInsight = ref('')
const teachbackRating = ref<'ready' | 'again' | 'skip' | null>(null)
const teachbackCompleted = ref(false)
const teachbackBusy = ref(false)
const teachbackError = ref<UiError | null>(null)
const appendingInsight = ref(false)

const teachbackDone = computed(() => teachbackCompleted.value)

async function runTeachbackStep(step: number, userAnswer?: string, markComplete = false) {
  if (!props.session || !props.teachbackPagePath) return
  teachbackBusy.value = true
  teachbackError.value = null
  try {
    const res = await api.teachbackTurn(props.session, props.teachbackPagePath, step, userAnswer)
    teachbackTurn.value = res
    if (res.suggestedInsight) {
      teachbackInsight.value = res.suggestedInsight
    } else if (markComplete && userAnswer && !teachbackInsight.value.trim()) {
      teachbackInsight.value = userAnswer
    }
    if (markComplete) {
      teachbackCompleted.value = true
    }
    teachbackAnswer.value = ''
  } catch (err) {
    teachbackError.value = parseAppError(err)
  } finally {
    teachbackBusy.value = false
  }
}

function advanceTeachback(skipAnswer = false) {
  const curStep = teachbackTurn.value?.step ?? 1
  const totalSteps = teachbackTurn.value?.totalSteps ?? 3
  const answer = skipAnswer ? '' : teachbackAnswer.value.trim()
  if (curStep >= totalSteps) {
    void runTeachbackStep(totalSteps, answer || undefined, true)
    return
  }
  void runTeachbackStep(Math.min(totalSteps, curStep + 1), answer || undefined, false)
}

async function appendUnderstanding() {
  if (!props.session || !props.teachbackPagePath || appendingInsight.value) return
  const text = teachbackInsight.value.trim()
  if (!text) return
  appendingInsight.value = true
  teachbackError.value = null
  try {
    const updated = await api.appendHumanUnderstanding(props.session, props.teachbackPagePath, text)
    emit('understandingAppended', updated)
    emit('exitTeachback')
  } catch (err) {
    teachbackError.value = parseAppError(err)
  } finally {
    appendingInsight.value = false
  }
}

watch(
  () => props.session,
  () => {
    conversations.value = []
    activeConversationId.value = undefined
    messages.value = []
    savedMessageIds.value = {}
    void loadConversations()
  },
  { immediate: true },
)

watch(
  () => [props.chatMode, props.teachbackPagePath] as const,
  ([mode, pagePath]) => {
    if (mode === 'review' && pagePath) {
      teachbackTurn.value = null
      teachbackAnswer.value = ''
      teachbackInsight.value = ''
      teachbackRating.value = null
      teachbackCompleted.value = false
      teachbackError.value = null
      void runTeachbackStep(1)
    }
  },
  { immediate: true },
)

watch(
  () => props.pendingAskPrompt,
  (prompt) => {
    if (!prompt) return
    emit('clearPendingAsk')
    void sendMessage(prompt)
  },
  { immediate: true },
)
</script>

<template>
  <aside class="right-pane" aria-label="右侧文档信息与 AI 问答栏">
    <div class="right-tabs" role="tablist">
      <button
        v-if="!inGraph"
        type="button"
        :class="{ active: activeTab === 'info' }"
        @click="emit('update:activeTab', 'info')"
      >{{ infoLabel }}</button>
      <button
        type="button"
        :class="{ active: activeTab === 'chat' || inGraph }"
        @click="emit('update:activeTab', 'chat')"
      >AI 问答</button>
    </div>

    <!-- Tab 1: Contextual Info (Wiki / Raw / Inbox) -->
    <template v-if="activeTab === 'info' && !inGraph">
      <!-- Inbox Info -->
      <div v-if="mode === 'inbox'" class="right-scroll">
        <template v-if="activeInbox">
          <div class="context-section">
            <div class="kicker">AI 3 句话导读</div>
            <ul
              v-if="activeInbox.summaryPoints.length"
              style="margin:0;padding-left:16px;display:flex;flex-direction:column;gap:6px;font-size:12.5px;color:var(--text-secondary);line-height:1.6;"
            >
              <li v-for="(pt, idx) in activeInbox.summaryPoints" :key="idx">{{ pt }}</li>
            </ul>
            <span v-else class="hint">暂无导读摘要。</span>
          </div>

          <div class="context-section">
            <div class="kicker">你标出的重点（{{ inboxHighlights.length }}）</div>
            <div v-for="h in inboxHighlights" :key="h.id" class="cs-card" style="padding:9px 10px;gap:5px;">
              <div style="display:flex;justify-content:space-between;align-items:center;gap:6px;">
                <strong style="font-size:12px;">{{ h.heading || '重点片段' }}</strong>
                <span class="pill" :class="h.compiled ? 'success' : 'accent'">
                  {{ h.compiled ? '已入库' : '待提炼' }}
                </span>
              </div>
              <div style="font-size:11.5px;color:var(--text-secondary);">
                “{{ h.text.slice(0, 72) }}{{ h.text.length > 72 ? '…' : '' }}”
              </div>
            </div>
            <span v-if="!inboxHighlights.length" class="hint">
              阅读原文时点击任意段落即可标为重点。
            </span>
          </div>

          <div v-if="activeInbox.relatedPagePaths.length" class="context-section">
            <div class="kicker">相关已有知识页</div>
            <button
              v-for="wp in activeInbox.relatedPagePaths"
              :key="wp"
              type="button"
              class="list-item"
              @click="emit('openWiki', wp)"
            >
              <span class="list-item-title">{{ wikiTitle(wp) }}</span>
              <span class="list-item-sub mono">{{ wp }}</span>
            </button>
          </div>
        </template>
        <p v-else class="hint">选择一篇文章后在此查看导读与已标重点。</p>
      </div>

      <!-- Raw File Info -->
      <div v-else-if="mode === 'raw'" class="right-scroll">
        <template v-if="activeDoc">
          <div class="context-section">
            <div class="kicker">由此原文整理出的知识页（{{ activeDoc.linkedWikiPaths.length }}）</div>
            <button
              v-for="wp in activeDoc.linkedWikiPaths"
              :key="wp"
              type="button"
              class="list-item"
              @click="emit('openWiki', wp)"
            >
              <span class="list-item-title">{{ wikiTitle(wp) }}</span>
              <span class="list-item-sub mono">{{ wp }}</span>
            </button>
            <span v-if="!activeDoc.linkedWikiPaths.length" class="hint">尚未整理生成知识页</span>
          </div>

          <div class="context-section">
            <div class="kicker">文件信息</div>
            <div style="font-size:12px;color:var(--text-secondary);display:flex;flex-direction:column;gap:5px;">
              <div>类型：{{ activeDoc.typeLabel || '原始文件' }}</div>
              <div>更新时间：{{ shortDate(activeDoc.updatedAt) }}</div>
              <div class="mono" style="font-size:11px;color:var(--muted);word-break:break-all;">
                {{ activeDoc.path }}
              </div>
            </div>
          </div>
        </template>
        <p v-else class="hint">选择一份原始文件查看关联知识页。</p>
      </div>

      <!-- Wiki Page Info -->
      <div v-else class="right-scroll">
        <template v-if="activeDoc">
          <div class="context-section">
            <div class="kicker">来源原文（{{ activeDoc.citations.length }}）</div>
            <div
              v-for="c in activeDoc.citations"
              :key="c.id"
              class="cs-card"
              style="padding:9px 10px;gap:6px;"
            >
              <div style="display:flex;justify-content:space-between;align-items:center;gap:6px;">
                <strong style="font-size:12px;">{{ c.label }}</strong>
                <button
                  type="button"
                  class="btn quiet xs"
                  style="color:var(--accent);"
                  @click="emit('openEvidence', c)"
                >对照</button>
              </div>
              <div v-if="c.quote" style="font-size:11.5px;color:var(--text-secondary);">
                “{{ c.quote.slice(0, 64) }}{{ c.quote.length > 64 ? '…' : '' }}”
              </div>
            </div>
            <span v-if="!activeDoc.citations.length" class="hint">暂无关联原文</span>
          </div>

          <div class="context-section">
            <div class="kicker">谁引用了这篇（{{ activeDoc.backlinks.length }}）</div>
            <button
              v-for="bl in activeDoc.backlinks"
              :key="bl.pagePath"
              type="button"
              class="list-item"
              @click="emit('openWiki', bl.pagePath)"
            >
              <span class="list-item-title">{{ bl.title }}</span>
              <span class="list-item-sub">{{ bl.excerpt }}</span>
            </button>
            <span v-if="!activeDoc.backlinks.length" class="hint">暂无其他笔记引用</span>
          </div>

          <div class="context-section">
            <div class="kicker">目录大纲</div>
            <div
              v-for="(h, i) in activeDoc.outline"
              :key="i"
              style="font-size:12.5px;color:var(--text-secondary);padding:2px 4px;"
            >· {{ h }}</div>
            <span v-if="!activeDoc.outline.length" class="hint">暂无二级章节标题</span>
          </div>

          <div v-if="pageHistory.length" class="context-section">
            <div style="display:flex;justify-content:space-between;align-items:center;">
              <span class="kicker">本页整理记录</span>
              <button type="button" class="btn quiet xs" @click="emit('goOrganize')">全部</button>
            </div>
            <div
              v-for="cs in pageHistory"
              :key="cs.id"
              style="font-size:12px;color:var(--text-secondary);display:flex;justify-content:space-between;align-items:center;gap:6px;"
            >
              <span style="overflow:hidden;text-overflow:ellipsis;white-space:nowrap;">
                {{ cs.time }} · {{ cs.summary }}
              </span>
              <button
                v-if="cs.status !== 'reverted'"
                type="button"
                class="btn quiet xs"
                @click="emit('revertChangeset', cs.id)"
              >撤销</button>
              <span v-else class="pill">已撤销</span>
            </div>
          </div>
        </template>
        <p v-else class="hint">打开一篇知识页后，可在此查看来源原文、反向引用与大纲。</p>
      </div>
    </template>

    <!-- Tab 2: AI Chat OR Active Recall ("让我讲一遍") -->
    <template v-else>
      <template v-if="chatMode === 'review' && teachbackPagePath">
        <div class="right-scroll">
          <div style="display:flex;justify-content:space-between;align-items:center;">
            <span class="pill accent">让我讲一遍 · 遮蔽复述中</span>
            <button type="button" class="btn quiet xs" @click="emit('exitTeachback')">结束</button>
          </div>

          <ErrorNotice v-if="teachbackError" :error="teachbackError" />

          <div v-if="teachbackBusy && !teachbackTurn" class="hint">正在准备追问…</div>

          <div v-if="teachbackTurn" class="chat-msg">
            <span class="chat-layer-tag">
              {{ teachbackDone ? '复述完成' : `步骤 ${teachbackTurn.step} / ${teachbackTurn.totalSteps}` }} · 《{{ teachbackPageTitle }}》
            </span>
            <p v-if="teachbackTurn.feedback" style="color:var(--text-secondary);">{{ teachbackTurn.feedback }}</p>
            <p v-if="!teachbackDone">{{ teachbackTurn.prompt }}</p>
          </div>

          <div v-if="teachbackDone" class="cs-card">
            <span class="kicker">觉得掌握得怎么样？</span>
            <div style="display:flex;gap:6px;flex-wrap:wrap;">
              <button
                type="button"
                class="btn sm"
                :class="{ primary: teachbackRating === 'ready' }"
                @click="teachbackRating = 'ready'; emit('toast', '已记录：掌握良好')"
              >能用</button>
              <button
                type="button"
                class="btn sm"
                :class="{ primary: teachbackRating === 'again' }"
                @click="teachbackRating = 'again'; emit('toast', '已记下，过几天提醒你再看')"
              >过几天再看</button>
              <button
                type="button"
                class="btn sm"
                :class="{ primary: teachbackRating === 'skip' }"
                @click="teachbackRating = 'skip'; emit('toast', '已记录')"
              >暂不关心</button>
            </div>

            <div style="padding-top:8px;border-top:1px solid var(--line-subtle);display:flex;flex-direction:column;gap:8px;">
              <span style="font-size:12px;color:var(--text-secondary);">将这次复述的新领悟写入本页受保护的「我的理解」：</span>
              <textarea
                v-model="teachbackInsight"
                class="source-editor"
                style="min-height:74px;font-family:var(--font-ui);font-size:12.5px;"
                placeholder="写下或微调你想保留的个人理解…"
              />
              <button
                type="button"
                class="btn primary sm"
                :disabled="appendingInsight || !teachbackInsight.trim()"
                @click="appendUnderstanding"
              >{{ appendingInsight ? '正在写入…' : '写入「我的理解」' }}</button>
            </div>
          </div>
        </div>

        <div v-if="!teachbackDone" class="chat-composer">
          <textarea
            v-model="teachbackAnswer"
            placeholder="用自己的话复述要点，或直接点「还说不清」…"
            :disabled="teachbackBusy"
            @keydown.ctrl.enter.prevent="advanceTeachback(false)"
          />
          <div style="display:flex;justify-content:space-between;">
            <button
              type="button"
              class="btn quiet sm"
              :disabled="teachbackBusy"
              @click="advanceTeachback(true)"
            >还说不清 · 看提示</button>
            <button
              type="button"
              class="btn primary sm"
              :disabled="teachbackBusy"
              @click="advanceTeachback(false)"
            >{{ teachbackBusy ? '思考中…' : '提交作答' }}</button>
          </div>
        </div>
      </template>

      <template v-else>
        <div class="right-scroll">
          <div style="display:flex;justify-content:space-between;align-items:center;gap:6px;">
            <span class="kicker">问答范围</span>
            <div style="display:flex;gap:4px;">
              <button
                v-if="activeDoc && activeDoc.kind === 'wiki'"
                type="button"
                class="btn quiet xs"
                @click="emit('startTeachback', activeDoc.path)"
              >让我讲一遍</button>
              <button type="button" class="btn quiet xs" @click="startNewConversation">新对话</button>
            </div>
          </div>

          <div v-if="activeDoc" class="chat-scope-bar">
            <span class="scope-chip">当前页：{{ activeDoc.title.slice(0, 12) }}</span>
            <button
              type="button"
              class="scope-chip"
              :class="{ add: !includeOneHop }"
              @click="includeOneHop = !includeOneHop"
            >
              {{ includeOneHop ? '已含关联笔记 ×' : '+ 加入关联笔记' }}
            </button>
          </div>

          <ErrorNotice v-if="chatError" :error="chatError" />

          <div v-if="!messages.length && !sendingChat" class="cs-card">
            <strong style="font-size:12.5px;">基于本地知识库的有据问答</strong>
            <p class="hint">回答会自动检索你的知识页、随手笔记与收藏原文，点击引用标记可直接在中间分屏对照出处。</p>
          </div>

          <div
            v-for="m in messages"
            :key="m.id"
            class="chat-msg"
            :class="{ user: m.role === 'user' }"
          >
            <span class="chat-layer-tag">{{ m.role === 'user' ? '你' : '来自你的知识库' }}</span>
            <div v-if="m.role === 'user'" style="white-space:pre-wrap;">{{ m.content }}</div>
            <template v-else>
              <MarkdownPreview
                :source="m.content"
                :session="session"
                compact
                hide-frontmatter
                :citations="m.citations"
                @cite="handleChatCitation"
              />
              <div v-if="m.notice" class="hint">{{ m.notice }}</div>
              <div style="display:flex;gap:6px;padding-top:6px;border-top:1px solid var(--line-subtle);">
                <button
                  type="button"
                  class="btn sm"
                  :disabled="Boolean(savedMessageIds[m.id]) || savingAnswerId === m.id"
                  @click="saveAnswer(m)"
                >
                  {{ savedMessageIds[m.id] ? '已存进知识库' : savingAnswerId === m.id ? '正在保存…' : '存成知识页' }}
                </button>
                <button type="button" class="btn quiet sm" @click="copyAnswer(m.content)">复制</button>
              </div>
            </template>
          </div>

          <div v-if="sendingChat" class="chat-msg">
            <span class="chat-layer-tag">AI 正在检索并组织回答…</span>
          </div>
        </div>

        <div class="chat-composer">
          <textarea
            v-model="chatInput"
            placeholder="围绕当前笔记或整个知识库提问…"
            :disabled="sendingChat"
            @keydown="handleChatKeydown"
          />
          <div style="display:flex;justify-content:space-between;align-items:center;">
            <span style="font-size:11px;color:var(--muted);">Ctrl+Enter 发送</span>
            <button
              type="button"
              class="btn primary sm"
              :disabled="sendingChat || !chatInput.trim()"
              @click="sendMessage()"
            >{{ sendingChat ? '发送中…' : '提问' }}</button>
          </div>
        </div>
      </template>
    </template>
  </aside>
</template>
