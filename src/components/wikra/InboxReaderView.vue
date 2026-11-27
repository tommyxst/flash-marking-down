<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import type { UiError } from '../../api/errors'
import ErrorNotice from '../ErrorNotice.vue'
import UiIcon from '../../UiIcon.vue'
import type { DocWidthMode, InboxItemDetail } from '../../m2'

const props = defineProps<{
  item: InboxItemDetail | null
  loading: boolean
  error: UiError | null
  readMode: 'full' | 'highlights'
  docWidth: DocWidthMode
  ingesting: boolean
  compiling: boolean
  completingStub: boolean
  togglingParaId: string
}>()

const emit = defineEmits<{
  'update:readMode': [mode: 'full' | 'highlights']
  toggleWidth: []
  ingest: [input: string]
  openExternalUrl: [url: string]
  toggleParagraph: [paragraphId: string]
  compile: [mode: 'excerpt' | 'full' | 'archive']
  completeStub: [pastedBody: string]
}>()

const captureInput = ref('')
const stubPaste = ref('')

watch(
  () => props.item?.id,
  () => {
    stubPaste.value = ''
  },
)

const isFullWidth = computed(() => props.docWidth === 'full')
const highlightedPending = computed(() =>
  (props.item?.paragraphs || []).filter((p) => p.highlighted && !p.compiled),
)
const allHighlighted = computed(() =>
  (props.item?.paragraphs || []).filter((p) => p.highlighted),
)
const visibleParas = computed(() =>
  props.readMode === 'highlights' ? allHighlighted.value : (props.item?.paragraphs || []),
)

function submitIngest() {
  const val = captureInput.value.trim()
  if (!val || props.ingesting) return
  emit('ingest', val)
  captureInput.value = ''
}

function submitStub() {
  const val = stubPaste.value.trim()
  if (!val || props.completingStub) return
  emit('completeStub', val)
}
</script>

<template>
  <div class="stage-body flush">
    <form class="inbox-capture-bar" @submit.prevent="submitIngest">
      <input
        v-model="captureInput"
        type="text"
        placeholder="粘贴网页链接（http/https）或长文内容收入收件箱…"
        aria-label="收集网页链接或文本"
        :disabled="ingesting"
      >
      <button type="submit" class="btn primary sm" :disabled="ingesting || !captureInput.trim()">
        {{ ingesting ? '正在收集…' : '收集' }}
      </button>
    </form>

    <div v-if="loading && !item" class="empty-stage">
      <p>正在加载文章全文…</p>
    </div>

    <div v-else-if="error && !item" class="empty-stage">
      <ErrorNotice :error="error" />
    </div>

    <div v-else-if="!item" class="empty-stage">
      <UiIcon name="inbox" size="lg" />
      <p>收件箱还没有文章。在上方粘贴网页链接或文本，即可收入这里舒心阅读并标重点。</p>
    </div>

    <template v-else>
      <div class="doc-toolbar">
        <div style="display:flex;align-items:center;gap:8px;min-width:0;">
          <span class="pill">{{ item.sourceType }}</span>
          <span class="breadcrumb" :title="item.url || item.rawPath">
            {{ item.url || item.rawPath || '本地粘贴文档' }}
          </span>
        </div>
        <div style="display:flex;align-items:center;gap:6px;flex:none;">
          <button
            v-if="item.url"
            type="button"
            class="btn sm"
            @click="emit('openExternalUrl', item.url)"
          >
            <UiIcon name="external" size="sm" /> 打开原网页 ↗
          </button>
          <template v-if="item.fetchStatus === 'full'">
            <button
              type="button"
              class="btn sm"
              :class="{ primary: readMode === 'full' }"
              @click="emit('update:readMode', 'full')"
            >完整原文</button>
            <button
              type="button"
              class="btn sm"
              :class="{ primary: readMode === 'highlights' }"
              @click="emit('update:readMode', 'highlights')"
            >已标重点 ({{ allHighlighted.length }})</button>
          </template>
          <button
            type="button"
            class="btn quiet sm"
            :class="{ 'active-toggle': isFullWidth }"
            title="切换适读宽度或铺满整栏"
            @click="emit('toggleWidth')"
          >
            <UiIcon name="width" size="sm" />
            <span>{{ isFullWidth ? '适读宽' : '铺满宽' }}</span>
          </button>
        </div>
      </div>

      <div class="doc-scroll">
        <article class="doc-paper">
          <ErrorNotice v-if="error" :error="error" style="margin-bottom:12px;" />

          <header class="reader-head">
            <h1 style="font-size:23px;font-weight:700;letter-spacing:-0.02em;">{{ item.title }}</h1>
            <div class="reader-meta">
              <span>{{ item.domain || item.sourceType }}</span>
              <span>·</span>
              <span>收藏于 {{ item.savedAt }}</span>
              <span>·</span>
              <span>{{ item.readTime }}</span>
            </div>

            <div v-if="item.fetchStatus === 'full'" class="reader-actions">
              <div style="display:flex;flex-wrap:wrap;gap:8px;align-items:center;">
                <template v-if="highlightedPending.length > 0">
                  <button
                    type="button"
                    class="btn primary sm"
                    :disabled="compiling"
                    @click="emit('compile', 'excerpt')"
                  >{{ compiling ? '正在提炼…' : `提炼重点入库（${highlightedPending.length} 段）` }}</button>
                  <button
                    type="button"
                    class="btn sm"
                    :disabled="compiling"
                    @click="emit('compile', 'full')"
                  >整篇提炼（约 {{ item.wordCount || 300 }} 字）</button>
                </template>
                <button
                  v-else
                  type="button"
                  class="btn primary sm"
                  :disabled="compiling"
                  @click="emit('compile', 'full')"
                >{{ compiling ? '正在提炼…' : `整篇提炼入库（约 ${item.wordCount || 300} 字）` }}</button>

                <button
                  type="button"
                  class="btn quiet sm"
                  :disabled="compiling"
                  @click="emit('compile', 'archive')"
                >仅存档（保留原文）</button>
              </div>
              <span style="font-size:11.5px;color:var(--muted);">点击下方任意段落即可标为重点</span>
            </div>
          </header>

          <section v-if="item.fetchStatus === 'stub'" class="setting-card">
            <strong>该网页需要登录后查看，已为你保存链接</strong>
            <p style="font-size:12.5px;color:var(--text-secondary);">
              为了隐私安全，应用不会读取你的浏览器登录态。点上方「打开原网页 ↗」阅读后，把想保留的内容粘贴到下方即可存为本地原文：
            </p>
            <textarea
              v-model="stubPaste"
              class="source-editor"
              style="min-height:160px;"
              placeholder="粘贴文章正文或关键段落…"
              :disabled="completingStub"
            />
            <div>
              <button
                type="button"
                class="btn primary sm"
                :disabled="completingStub || !stubPaste.trim()"
                @click="submitStub"
              >{{ completingStub ? '正在保存…' : '保存为本地原文' }}</button>
            </div>
          </section>

          <div v-else class="prose">
            <div
              v-for="para in visibleParas"
              :key="para.id"
              class="reader-para"
              :class="{ highlighted: para.highlighted, compiled: para.compiled }"
              role="button"
              tabindex="0"
              :aria-pressed="para.highlighted"
              @click="!para.compiled && togglingParaId !== para.id && emit('toggleParagraph', para.id)"
              @keydown.enter.prevent="!para.compiled && togglingParaId !== para.id && emit('toggleParagraph', para.id)"
              @keydown.space.prevent="!para.compiled && togglingParaId !== para.id && emit('toggleParagraph', para.id)"
            >
              <div class="reader-para-head">
                <h3 v-if="para.heading">{{ para.heading }}</h3>
                <span v-else />
                <div style="display:flex;align-items:center;gap:6px;">
                  <span v-if="para.compiled" class="pill success">已提炼入库</span>
                  <span v-else-if="para.highlighted" class="pill accent">已标重点</span>
                  <span v-else class="reader-para-hint">点击标为重点</span>
                </div>
              </div>
              <p>{{ para.text }}</p>
            </div>

            <p v-if="!visibleParas.length" class="hint">
              还没有标出重点段落。切换回「完整原文」，点击你认可的段落即可高亮标记。
            </p>
          </div>
        </article>
      </div>
    </template>
  </div>
</template>
