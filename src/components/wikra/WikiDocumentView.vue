<script setup lang="ts">
import { computed, nextTick, onMounted, onUnmounted, ref, watch } from 'vue'
import { api } from '../../api/client'
import { parseAppError, type UiError } from '../../api/errors'
import CaptureEditor from '../../CaptureEditor.vue'
import ErrorNotice from '../ErrorNotice.vue'
import MarkdownPreview from '../../MarkdownPreview.vue'
import UiIcon from '../../UiIcon.vue'
import type {
  CitationContext,
  DocCitation,
  DocTaskItem,
  DocumentDetail,
  DocWidthMode,
  LintFinding,
  WikilinkCandidate,
  WikilinkMatch,
} from '../../m2'
import { parseWikiBodyForView, shortDate } from '../../utils/wikra'

const props = defineProps<{
  session: string
  doc: DocumentDetail | null
  loading: boolean
  error: UiError | null
  editorMode: 'preview' | 'source'
  docWidth: DocWidthMode
  recallCurtainPagePath: string | null
  evidenceCitation: DocCitation | null
  evidenceRawDoc: DocumentDetail | null
  evidenceLoading: boolean
  externalChanged: boolean
  wikilinkCandidates?: WikilinkCandidate[]
  pageLintFindings?: LintFinding[]
}>()

const emit = defineEmits<{
  'update:editorMode': [mode: 'preview' | 'source']
  toggleWidth: []
  toggleSplitOrigin: []
  openEvidence: [citation: DocCitation]
  closeEvidence: []
  openRaw: [path: string]
  openWikilink: [target: string]
  openWikiPath: [path: string]
  openPageLint: [path: string]
  openConflict: [conflictId: string]
  startTeachback: [pagePath: string]
  exitCurtain: []
  openObsidian: [path: string]
  openExternalUrl: [url: string]
  documentSaved: [doc: DocumentDetail]
  reloadDoc: [path: string]
  dismissExternal: []
  toggleTask: [task: DocTaskItem]
  toast: [message: string]
}>()

const propsExpanded = ref(false)
const peekSections = ref<Record<string, boolean>>({})
const sourceDraft = ref('')
const dirty = ref(false)
const saving = ref(false)
const saveError = ref<UiError | null>(null)
const versionConflict = ref(false)

const curtainOn = computed(() => Boolean(props.doc && props.recallCurtainPagePath === props.doc.path))
const parsedBody = computed(() => (props.doc ? parseWikiBodyForView(props.doc.body) : null))
const isFullWidth = computed(() => props.docWidth === 'full')

// ============================================================================
// P0 Scene 4.1: Lightweight L1 Lint Badge (only rendered for error / warn)
// ============================================================================
const openPageLint = computed(() =>
  (props.pageLintFindings || []).filter(
    (f) => !f.dismissed && (f.severity === 'error' || f.severity === 'warn'),
  ),
)
const lintErrorCount = computed(() => openPageLint.value.filter((f) => f.severity === 'error').length)
const lintWarnCount = computed(() => openPageLint.value.filter((f) => f.severity === 'warn').length)
const showLintBadge = computed(() => lintErrorCount.value + lintWarnCount.value > 0)

// ============================================================================
// P0 Scene 4.2: Wikilink Resolution & Horizontally Clamped Disambiguation Popover
// ============================================================================
interface DisambigState {
  target: string
  matches: WikilinkMatch[]
  left: number
  top: number | null
  bottom: number | null
  activeIdx: number
}

const disambig = ref<DisambigState | null>(null)
const disambigPopoverEl = ref<HTMLElement>()

function closeDisambig() {
  disambig.value = null
}

async function handleWikilinkClick(target: string, event?: MouseEvent) {
  closeCiteHoverImmediately()
  if (!props.session || !props.doc) {
    emit('openWikilink', target)
    return
  }
  const anchorEl = event?.currentTarget instanceof HTMLElement ? event.currentTarget : null
  const anchorRect = anchorEl?.getBoundingClientRect()

  try {
    const resolution = await api.resolveWikilink(props.session, props.doc.path, target)
    if (resolution.status === 'unique' && resolution.matches[0]) {
      closeDisambig()
      emit('openWikiPath', resolution.matches[0].path)
      return
    }
    if (resolution.status === 'missing' || !resolution.matches.length) {
      closeDisambig()
      emit('toast', `未找到《${target}》，可在整理或新建时创建`)
      return
    }

    // Ambiguous: compute viewport-clamped popover coordinates (min 12px horizontal margin)
    const popWidth = Math.min(316, window.innerWidth - 24)
    const rawLeft = anchorRect ? anchorRect.left : (event?.clientX ?? 24)
    const left = Math.max(12, Math.min(rawLeft, window.innerWidth - popWidth - 12))
    const estimatedHeight = Math.min(280, 44 + resolution.matches.length * 56)
    const spaceBelow = anchorRect ? window.innerHeight - anchorRect.bottom : window.innerHeight - (event?.clientY ?? 120)
    const above = spaceBelow < estimatedHeight + 16 && (anchorRect?.top ?? 0) > estimatedHeight + 16

    disambig.value = {
      target,
      matches: resolution.matches,
      left,
      top: above ? null : Math.min(window.innerHeight - 60, (anchorRect?.bottom ?? event?.clientY ?? 120) + 6),
      bottom: above ? Math.max(12, window.innerHeight - (anchorRect?.top ?? 120) + 6) : null,
      activeIdx: 0,
    }
  } catch {
    // Graceful fallback to local title lookup if resolve_wikilink errors unexpectedly
    emit('openWikilink', target)
  }
}

function selectDisambigMatch(match: WikilinkMatch) {
  closeDisambig()
  emit('openWikiPath', match.path)
}

// ============================================================================
// P0 Scene 4.3: Footnote Hover / Focus Citation Preview Card + Evidence Locate
// ============================================================================
interface CiteHoverState {
  citation: DocCitation
  left: number
  top: number | null
  bottom: number | null
  placement: 'above' | 'below'
  context: CitationContext | null
  loading: boolean
}

const citeHover = ref<CiteHoverState | null>(null)
const citationCtxCache = ref<Record<string, CitationContext>>({})
let citeShowTimer: number | undefined
let citeHideTimer: number | undefined

function citeCacheKey(pagePath: string, refLabel: string): string {
  return `${pagePath}::${refLabel}`
}

async function fetchCitationContext(pagePath: string, refLabel: string): Promise<CitationContext | null> {
  if (!props.session || !pagePath || !refLabel) return null
  const key = citeCacheKey(pagePath, refLabel)
  if (citationCtxCache.value[key]) return citationCtxCache.value[key]
  try {
    const ctx = await api.readCitationContext(props.session, pagePath, refLabel)
    citationCtxCache.value = { ...citationCtxCache.value, [key]: ctx }
    return ctx
  } catch {
    return null
  }
}

function showCiteCardAt(citation: DocCitation, el: HTMLElement) {
  const docPath = props.doc?.path
  if (!docPath) return
  const rect = el.getBoundingClientRect()
  const cardWidth = Math.min(364, window.innerWidth - 24)
  const centeredLeft = rect.left + rect.width / 2 - cardWidth / 2
  const left = Math.max(12, Math.min(centeredLeft, window.innerWidth - cardWidth - 12))
  const estimatedHeight = 180
  const showBelow = rect.top < estimatedHeight + 24 && window.innerHeight - rect.bottom > estimatedHeight + 16
  const placement: 'above' | 'below' = showBelow ? 'below' : 'above'

  const cached = citationCtxCache.value[citeCacheKey(docPath, citation.refLabel)] ?? null
  citeHover.value = {
    citation,
    left,
    top: showBelow ? rect.bottom + 8 : null,
    bottom: showBelow ? null : Math.max(12, window.innerHeight - rect.top + 8),
    placement,
    context: cached,
    loading: !cached,
  }

  if (!cached) {
    void fetchCitationContext(docPath, citation.refLabel).then((ctx) => {
      if (
        citeHover.value
        && citeHover.value.citation.id === citation.id
        && props.doc?.path === docPath
      ) {
        citeHover.value = { ...citeHover.value, context: ctx, loading: false }
      }
    })
  }
}

function handleDocCiteHover(citation: DocCitation, el: HTMLElement, immediate: boolean) {
  window.clearTimeout(citeHideTimer)
  window.clearTimeout(citeShowTimer)
  if (immediate) {
    showCiteCardAt(citation, el)
  } else {
    citeShowTimer = window.setTimeout(() => {
      showCiteCardAt(citation, el)
    }, 180)
  }
}

function scheduleCiteHide() {
  window.clearTimeout(citeShowTimer)
  window.clearTimeout(citeHideTimer)
  citeHideTimer = window.setTimeout(() => {
    citeHover.value = null
  }, 150)
}

function cancelCiteHide() {
  window.clearTimeout(citeHideTimer)
}

function closeCiteHoverImmediately() {
  window.clearTimeout(citeShowTimer)
  window.clearTimeout(citeHideTimer)
  citeHover.value = null
}

function openEvidenceFromHover() {
  if (!citeHover.value) return
  const target = citeHover.value.citation
  closeCiteHoverImmediately()
  emit('openEvidence', target)
}

// ============================================================================
// Evidence Split Paragraph Highlight & Auto-Scroll
// ============================================================================
const evidenceContext = ref<CitationContext | null>(null)
const evidenceBodyEl = ref<HTMLElement>()

function stripFrontmatterText(raw: string): string {
  const norm = raw.replace(/^\uFEFF/, '').replaceAll('\r\n', '\n')
  if (!norm.startsWith('---\n')) return norm
  const end = norm.indexOf('\n---', 4)
  if (end < 0) return norm
  const afterClose = end + 4
  if (afterClose < norm.length && norm[afterClose] !== '\n') return norm
  return norm.slice(afterClose).replace(/^\n/, '')
}

function splitRawParagraphs(raw: string): string[] {
  const clean = stripFrontmatterText(raw).trim()
  if (!clean) return []
  const lines = clean.split('\n')
  const blocks: string[] = []
  let buf: string[] = []
  let inFence = false

  for (const line of lines) {
    if (/^\s*(?:```|~~~)/.test(line)) inFence = !inFence
    if (!inFence && line.trim() === '') {
      if (buf.length) {
        blocks.push(buf.join('\n').trim())
        buf = []
      }
    } else {
      buf.push(line)
    }
  }
  if (buf.length) blocks.push(buf.join('\n').trim())
  return blocks.filter(Boolean)
}

function normalizeForMatch(s: string): string {
  return s.replace(/\s+/g, ' ').trim().toLowerCase()
}

const evidenceParagraphs = computed(() =>
  props.evidenceRawDoc ? splitRawParagraphs(props.evidenceRawDoc.body) : [],
)

const locatedEvidenceIndex = computed(() => {
  const blocks = evidenceParagraphs.value
  if (!blocks.length || !props.evidenceCitation) return -1
  const needles = [
    evidenceContext.value?.paragraph || '',
    evidenceContext.value?.quote || '',
    props.evidenceCitation.quote || '',
  ]
    .map(normalizeForMatch)
    .filter((n) => n.length >= 6)

  for (const needle of needles) {
    const idx = blocks.findIndex((b) => {
      const normBlock = normalizeForMatch(b)
      return normBlock.includes(needle) || (needle.length > 24 && normBlock.includes(needle.slice(0, 24)))
    })
    if (idx >= 0) return idx
  }
  return -1
})

watch(
  () => [props.doc?.path, props.evidenceCitation?.refLabel] as const,
  async ([docPath, refLabel]) => {
    evidenceContext.value = null
    if (!docPath || !refLabel) return
    const ctx = await fetchCitationContext(docPath, refLabel)
    if (props.doc?.path === docPath && props.evidenceCitation?.refLabel === refLabel) {
      evidenceContext.value = ctx
    }
  },
  { immediate: true },
)

watch(
  () => [props.evidenceRawDoc?.path, locatedEvidenceIndex.value] as const,
  ([rawPath, idx]) => {
    if (!rawPath || idx < 0) return
    void nextTick(() => {
      const target = evidenceBodyEl.value?.querySelector<HTMLElement>(`[data-raw-para="${idx}"]`)
      target?.scrollIntoView({ behavior: 'smooth', block: 'center' })
    })
  },
)

// ============================================================================
// Document Lifecycle & Source Editing
// ============================================================================
watch(
  () => props.doc?.path,
  () => {
    peekSections.value = {}
    propsExpanded.value = false
    saveError.value = null
    versionConflict.value = false
    sourceDraft.value = props.doc?.body ?? ''
    dirty.value = false
    citationCtxCache.value = {}
    closeDisambig()
    closeCiteHoverImmediately()
  },
  { immediate: true },
)

watch(
  () => props.doc?.contentHash,
  () => {
    if (!props.doc) return
    citationCtxCache.value = {}
    if (!dirty.value || props.editorMode !== 'source') {
      sourceDraft.value = props.doc.body
      dirty.value = false
      versionConflict.value = false
    }
  },
)

watch(
  () => props.recallCurtainPagePath,
  () => {
    peekSections.value = {}
  },
)

function onSourceUpdate(val: string) {
  sourceDraft.value = val
  dirty.value = props.doc ? val !== props.doc.body : false
  if (saveError.value) saveError.value = null
}

async function saveSource() {
  const current = props.doc
  if (!current || saving.value || !dirty.value) return
  saving.value = true
  saveError.value = null
  versionConflict.value = false
  try {
    const updated = await api.saveMarkdownDocument(
      props.session,
      current.path,
      current.contentHash,
      sourceDraft.value,
    )
    dirty.value = false
    sourceDraft.value = updated.body
    emit('documentSaved', updated)
  } catch (err) {
    const parsed = parseAppError(err)
    if (parsed.code === 'VERSION_CONFLICT') {
      versionConflict.value = true
    } else {
      saveError.value = parsed
    }
  } finally {
    saving.value = false
  }
}

function discardSource() {
  if (!props.doc) return
  sourceDraft.value = props.doc.body
  dirty.value = false
  saveError.value = null
  versionConflict.value = false
}

function reloadFromDisk() {
  if (!props.doc) return
  dirty.value = false
  versionConflict.value = false
  saveError.value = null
  emit('reloadDoc', props.doc.path)
}

function handleSourceKeydown(e: KeyboardEvent) {
  if (e.isComposing || e.keyCode === 229) return
  if (e.ctrlKey && e.key.toLowerCase() === 's') {
    e.preventDefault()
    void saveSource()
  }
}

function peekSection(key: string) {
  peekSections.value = { ...peekSections.value, [key]: true }
}

function handleGlobalKey(e: KeyboardEvent) {
  if (e.isComposing || e.keyCode === 229) return
  if (disambig.value) {
    if (e.key === 'Escape') {
      e.preventDefault()
      closeDisambig()
      return
    }
    if (e.key === 'ArrowDown') {
      e.preventDefault()
      disambig.value = {
        ...disambig.value,
        activeIdx: (disambig.value.activeIdx + 1) % disambig.value.matches.length,
      }
      return
    }
    if (e.key === 'ArrowUp') {
      e.preventDefault()
      disambig.value = {
        ...disambig.value,
        activeIdx: (disambig.value.activeIdx - 1 + disambig.value.matches.length) % disambig.value.matches.length,
      }
      return
    }
    if (e.key === 'Enter') {
      e.preventDefault()
      const chosen = disambig.value.matches[disambig.value.activeIdx] || disambig.value.matches[0]
      if (chosen) selectDisambigMatch(chosen)
      return
    }
  }
  if (citeHover.value && e.key === 'Escape') {
    e.preventDefault()
    closeCiteHoverImmediately()
  }
}

function handleGlobalMousedown(e: MouseEvent) {
  if (!disambig.value) return
  const target = e.target as HTMLElement | null
  if (target && disambigPopoverEl.value?.contains(target)) return
  closeDisambig()
}

onMounted(() => {
  window.addEventListener('keydown', handleGlobalKey)
  window.addEventListener('mousedown', handleGlobalMousedown)
})

onUnmounted(() => {
  window.removeEventListener('keydown', handleGlobalKey)
  window.removeEventListener('mousedown', handleGlobalMousedown)
  window.clearTimeout(citeShowTimer)
  window.clearTimeout(citeHideTimer)
})
</script>

<template>
  <div class="stage-body flush">
    <div v-if="loading && !doc" class="empty-stage">
      <p>正在加载知识页…</p>
    </div>

    <div v-else-if="error && !doc" class="empty-stage">
      <ErrorNotice :error="error" />
    </div>

    <div v-else-if="!doc" class="empty-stage">
      <UiIcon name="book" size="lg" />
      <p>从左侧选择一篇知识页或原始文件开始阅读</p>
    </div>

    <div v-else class="editor-shell" :class="{ 'with-evidence': Boolean(evidenceCitation) }">
      <div class="editor-main">
        <div class="doc-toolbar">
          <div class="breadcrumb" :title="doc.path">{{ doc.path }}</div>
          <div class="doc-toolbar-actions">
            <button
              type="button"
              class="btn sm"
              :class="{ primary: editorMode === 'preview' }"
              title="阅读模式 (Ctrl+E)"
              @click="emit('update:editorMode', 'preview')"
            >阅读</button>
            <button
              type="button"
              class="btn sm"
              :class="{ primary: editorMode === 'source' }"
              title="编辑原始 Markdown 源码 (Ctrl+E)"
              @click="emit('update:editorMode', 'source')"
            >源码</button>
            <button
              v-if="doc.citations.length > 0"
              type="button"
              class="btn sm"
              :class="{ 'active-toggle': Boolean(evidenceCitation) }"
              title="右侧并排打开来源原文"
              @click="emit('toggleSplitOrigin')"
            >
              <UiIcon name="split" size="sm" />
              <span>对照原文</span>
            </button>
            <button
              type="button"
              class="btn quiet sm"
              :class="{ 'active-toggle': isFullWidth }"
              title="切换适读宽度（920px）或铺满整栏"
              @click="emit('toggleWidth')"
            >
              <UiIcon name="width" size="sm" />
              <span>{{ isFullWidth ? '适读宽' : '铺满宽' }}</span>
            </button>
            <button
              type="button"
              class="btn quiet sm"
              title="遮住正文用自己的话复述一遍"
              @click="emit('startTeachback', doc.path)"
            >让我讲一遍</button>
            <button
              v-if="showLintBadge"
              type="button"
              class="lint-toolbar-badge"
              :class="lintErrorCount > 0 ? 'error' : 'warn'"
              title="点击前往整理中心查看并修复本页体检项"
              @click="emit('openPageLint', doc.path)"
            >
              <UiIcon name="alert" size="sm" />
              <span>{{ lintErrorCount > 0 ? `${lintErrorCount + lintWarnCount} 项体检异常` : `${lintWarnCount} 项体检提醒` }}</span>
            </button>
            <button
              type="button"
              class="btn quiet sm"
              title="用系统默认关联程序（如 Obsidian）打开同一本地文件"
              @click="emit('openObsidian', doc.path)"
            >
              <UiIcon name="external" size="sm" /> Obsidian
            </button>
          </div>
        </div>

        <div class="doc-scroll" @scroll.passive="closeDisambig(); closeCiteHoverImmediately()">
          <article class="doc-paper">
            <div class="doc-title-row">
              <h1>{{ doc.title }}</h1>
              <span class="mono" style="font-size:11.5px;color:var(--muted);">
                更新于 {{ shortDate(doc.updatedAt) }}
              </span>
            </div>

            <div v-if="versionConflict || externalChanged" class="external-reload-bar" role="status">
              <span>
                {{ versionConflict
                  ? '本地文件已被外部（如 Obsidian）或后台整理修改，当前保存未覆盖磁盘文件。是否重新加载最新内容？'
                  : '检测到本地文件已在外部更新，是否重新加载最新内容？' }}
              </span>
              <div style="display:flex;gap:6px;flex:none;">
                <button type="button" class="btn sm primary" @click="reloadFromDisk">重新加载</button>
                <button
                  type="button"
                  class="btn quiet sm"
                  @click="versionConflict = false; emit('dismissExternal')"
                >保留当前</button>
              </div>
            </div>

            <ErrorNotice v-if="saveError" :error="saveError" style="margin-bottom:12px;" />

            <template v-if="editorMode === 'source'">
              <div class="source-editor-box" @keydown="handleSourceKeydown">
                <div class="source-editor-bar">
                  <span class="hint">
                    {{ dirty ? '有未保存的改动（Ctrl+S 保存）' : '已与本地磁盘文件同步' }}
                  </span>
                  <div style="display:flex;gap:6px;">
                    <button
                      type="button"
                      class="btn quiet sm"
                      :disabled="!dirty || saving"
                      @click="discardSource"
                    >放弃修改</button>
                    <button
                      type="button"
                      class="btn primary sm"
                      :disabled="!dirty || saving"
                      @click="saveSource"
                    >{{ saving ? '保存中…' : '保存改动' }}</button>
                  </div>
                </div>
                <CaptureEditor
                  :model-value="sourceDraft"
                  editor-id="wiki-source-editor"
                  :wikilink-candidates="wikilinkCandidates"
                  placeholder-text="在此编辑 Markdown 源码，输入 [[ 或 【【 可关联知识页…"
                  @update:model-value="onSourceUpdate"
                />
              </div>
              <p class="hint" style="margin-top:8px;">
                100% 兼容 Obsidian 本地 Markdown 格式，保留所有自定义标签、换行符与语法块。
              </p>
            </template>

            <template v-else>
              <div class="props-summary-bar">
                <div style="display:flex;flex-wrap:wrap;align-items:center;gap:6px;">
                  <span class="pill" :class="{ accent: doc.wikraCore }">{{ doc.typeLabel }}</span>
                  <span v-for="t in doc.tags" :key="t" class="pill">#{{ t }}</span>
                  <span style="color:var(--muted);">参考了 {{ doc.citations.length }} 处原文</span>
                </div>
                <button
                  type="button"
                  class="btn quiet xs"
                  @click="propsExpanded = !propsExpanded"
                >{{ propsExpanded ? '收起信息 ▴' : '属性 ▾' }}</button>
              </div>

              <section v-if="propsExpanded" class="props-expanded" aria-label="文档属性详情">
                <div class="fm-grid">
                  <span class="fm-key">页面分类</span>
                  <div class="fm-val">{{ doc.typeLabel }}</div>
                  <span class="fm-key">别名</span>
                  <div class="fm-val">{{ doc.aliases.join('、') || '无' }}</div>
                  <template v-if="doc.status">
                    <span class="fm-key">页面状态</span>
                    <div class="fm-val">{{ doc.status }}</div>
                  </template>
                  <span class="fm-key">AI 更新规则</span>
                  <div class="fm-val">
                    {{ doc.wikraCore ? '核心页：涉及结论变化时先请你确认' : '常规页：后台自动补充，随时可撤销' }}
                  </div>
                </div>
              </section>

              <div v-if="doc.pendingConflictIds.length > 0" class="single-conflict-banner" role="note">
                <div>
                  <strong style="color:var(--warning);">有 {{ doc.pendingConflictIds.length }} 处观点冲突：</strong>
                  <span>新收集的内容与之前的结论不一致，确认一下以哪条为准？</span>
                </div>
                <button
                  type="button"
                  class="btn sm"
                  style="flex:none;"
                  @click="emit('openConflict', doc.pendingConflictIds[0])"
                >去对比确认</button>
              </div>

              <div v-if="curtainOn" class="recall-curtain-banner">
                <span>
                  <strong>主动回忆中：</strong>正文和原文已遮住，先在右侧用自己的话讲一遍，再展开核对。
                </span>
                <button type="button" class="btn sm" @click="emit('exitCurtain')">展开全部核对</button>
              </div>

              <div v-if="parsedBody" class="prose">
                <template v-if="parsedBody.leadMarkdown">
                  <div v-if="curtainOn && !peekSections.lead" class="masked-body">
                    <span>导语与摘要已遮蔽 —— 先在右侧试着用自己的话复述要点</span>
                    <button type="button" class="btn sm" @click="peekSection('lead')">展开核对</button>
                  </div>
                  <MarkdownPreview
                    v-else
                    :source="parsedBody.leadMarkdown"
                    :session="session"
                    :base-path="doc.path"
                    hide-frontmatter
                    :doc-citations="doc.citations"
                    :doc-tasks="doc.tasks"
                    @doc-cite="closeCiteHoverImmediately(); emit('openEvidence', $event)"
                    @doc-cite-hover="handleDocCiteHover"
                    @doc-cite-leave="scheduleCiteHide"
                    @wikilink="handleWikilinkClick"
                    @toggle-task="emit('toggleTask', $event)"
                  />
                </template>

                <template v-for="(sec, idx) in parsedBody.sections" :key="`${idx}-${sec.heading}`">
                  <h2>{{ sec.heading }}</h2>
                  <div v-if="curtainOn && !peekSections[`sec-${idx}`]" class="masked-body">
                    <span>正文结论已遮蔽 —— 先在右侧试着复述这一节的核心要点</span>
                    <button type="button" class="btn sm" @click="peekSection(`sec-${idx}`)">展开核对</button>
                  </div>
                  <MarkdownPreview
                    v-else
                    :source="sec.body"
                    :session="session"
                    :base-path="doc.path"
                    hide-frontmatter
                    :doc-citations="doc.citations"
                    :doc-tasks="doc.tasks"
                    @doc-cite="closeCiteHoverImmediately(); emit('openEvidence', $event)"
                    @doc-cite-hover="handleDocCiteHover"
                    @doc-cite-leave="scheduleCiteHide"
                    @wikilink="handleWikilinkClick"
                    @toggle-task="emit('toggleTask', $event)"
                  />
                </template>

                <section v-if="doc.hasProtectedSection || parsedBody.protectedText" class="human-section">
                  <div style="display:flex;align-items:center;justify-content:space-between;margin-bottom:6px;">
                    <strong style="font-size:14.5px;">我的理解</strong>
                    <span class="pill success" title="此段落由你亲自维护，AI 整理不会改动它">
                      <UiIcon name="shield" size="sm" /> 手动保护
                    </span>
                  </div>
                  <div v-if="curtainOn && !peekSections.human" class="masked-body" style="margin-bottom:0;">
                    <span>「我的理解」已同步遮蔽 —— 复述后再展开核对或追加新想法</span>
                    <button type="button" class="btn sm" @click="peekSection('human')">展开核对</button>
                  </div>
                  <MarkdownPreview
                    v-else
                    :source="parsedBody.protectedText || '（尚未写入个人理解）'"
                    :session="session"
                    :base-path="doc.path"
                    hide-frontmatter
                    :doc-citations="doc.citations"
                    @doc-cite="closeCiteHoverImmediately(); emit('openEvidence', $event)"
                    @doc-cite-hover="handleDocCiteHover"
                    @doc-cite-leave="scheduleCiteHide"
                    @wikilink="handleWikilinkClick"
                  />
                </section>

                <section v-if="doc.citations.length > 0" class="footnotes-block" aria-label="参考原文出处">
                  <span class="kicker">参考原文出处（点击可分屏对照或直接打开完整原文）</span>
                  <div v-if="curtainOn && !peekSections.footnotes" class="masked-body" style="margin-bottom:0;">
                    <span>主动回忆期间已折起 {{ doc.citations.length }} 处原文引用，避免提前看到答案</span>
                    <button type="button" class="btn sm" @click="peekSection('footnotes')">展开引文</button>
                  </div>
                  <template v-else>
                    <div v-for="c in doc.citations" :key="c.id" class="footnote-item">
                      <div style="min-width:0;">
                        <span class="mono" style="color:var(--accent);">[^{{ c.refLabel }}]</span>
                        <strong style="margin-left:6px;">{{ c.label }}</strong>
                        <div v-if="c.quote" class="footnote-quote">“{{ c.quote }}”</div>
                      </div>
                      <div style="display:flex;gap:6px;flex:none;">
                        <button type="button" class="btn sm" @click="emit('openEvidence', c)">分屏对照</button>
                        <button
                          v-if="c.rawPath"
                          type="button"
                          class="btn quiet sm"
                          @click="emit('openRaw', c.rawPath)"
                        >完整原文</button>
                      </div>
                    </div>
                  </template>
                </section>
              </div>
            </template>
          </article>
        </div>
      </div>

      <aside v-if="evidenceCitation" class="evidence-pane" aria-label="原文对照分屏">
        <div class="pane-header">
          <div style="min-width:0;">
            <div style="font-size:12.5px;font-weight:650;">
              对照原文 · {{ evidenceRawDoc?.typeLabel || '原件' }}
            </div>
            <div
              class="mono"
              style="font-size:11px;color:var(--muted);overflow:hidden;text-overflow:ellipsis;white-space:nowrap;"
            >
              {{ evidenceRawDoc?.path || evidenceCitation.rawPath || evidenceCitation.label }}
            </div>
          </div>
          <div style="display:flex;align-items:center;gap:4px;">
            <button
              v-if="evidenceCitation.rawPath"
              type="button"
              class="btn quiet sm"
              @click="emit('openRaw', evidenceCitation.rawPath)"
            >全屏打开</button>
            <button
              type="button"
              class="icon-btn"
              aria-label="关闭原文分屏"
              @click="emit('closeEvidence')"
            >&times;</button>
          </div>
        </div>

        <div ref="evidenceBodyEl" class="evidence-body">
          <div style="display:flex;align-items:center;justify-content:space-between;gap:8px;margin-bottom:10px;flex-wrap:wrap;">
            <div class="row-flex">
              <span class="pill">只读原件</span>
              <span v-if="evidenceRawDoc?.updatedAt" class="pill">{{ shortDate(evidenceRawDoc.updatedAt) }}</span>
              <span
                v-if="evidenceContext"
                class="pill"
                :class="evidenceContext.verified ? 'success' : 'warning'"
              >{{ evidenceContext.verified ? '已核对原件' : '原文有变动' }}</span>
            </div>
            <button
              v-if="evidenceRawDoc?.sourceUrl"
              type="button"
              class="btn sm"
              @click="emit('openExternalUrl', evidenceRawDoc.sourceUrl)"
            >
              <UiIcon name="external" size="sm" /> 打开原网页
            </button>
          </div>

          <div class="kicker">引用片段</div>
          <div class="quote-highlight">“{{ evidenceContext?.quote || evidenceCitation.quote || evidenceCitation.label }}”</div>

          <div class="kicker" style="margin-top:14px;">原文完整内容</div>
          <p v-if="evidenceLoading" class="hint" style="margin-top:8px;">正在读取原件全文…</p>
          <div v-else-if="evidenceRawDoc && evidenceParagraphs.length > 0" style="margin-top:8px;">
            <div
              v-for="(block, idx) in evidenceParagraphs"
              :key="idx"
              :data-raw-para="idx"
              class="raw-para-block"
              :class="{ located: idx === locatedEvidenceIndex }"
            >
              <div v-if="idx === locatedEvidenceIndex" style="margin-bottom:4px;">
                <span class="pill accent">定位原段</span>
              </div>
              <MarkdownPreview
                :source="block"
                :session="session"
                :base-path="evidenceRawDoc.path"
                hide-frontmatter
                compact
              />
            </div>
          </div>
          <div v-else-if="evidenceRawDoc" style="margin-top:8px;">
            <MarkdownPreview
              :source="evidenceRawDoc.body"
              :session="session"
              :base-path="evidenceRawDoc.path"
              hide-frontmatter
              compact
            />
          </div>
          <p v-else class="hint" style="margin-top:8px;">
            未找到对应原件文件，上方展示已记录的原文引文片段。
          </p>
        </div>
      </aside>
    </div>

    <!-- Disambiguation Popover (Viewport-Clamped Fixed Overlay) -->
    <div
      v-if="disambig"
      ref="disambigPopoverEl"
      class="disambig-popover"
      role="listbox"
      :aria-label="`「${disambig.target}」同名知识页选择`"
      :style="{
        left: `${disambig.left}px`,
        top: disambig.top !== null ? `${disambig.top}px` : undefined,
        bottom: disambig.bottom !== null ? `${disambig.bottom}px` : undefined,
      }"
    >
      <div class="disambig-head">
        <span>「{{ disambig.target }}」对应 {{ disambig.matches.length }} 篇同名知识页</span>
        <button type="button" class="icon-btn sm" aria-label="关闭消歧列表" @click="closeDisambig">&times;</button>
      </div>
      <div class="disambig-list">
        <button
          v-for="(m, idx) in disambig.matches"
          :key="m.path"
          type="button"
          class="disambig-item"
          role="option"
          :aria-selected="idx === disambig.activeIdx"
          :class="{ active: idx === disambig.activeIdx }"
          @mouseenter="disambig.activeIdx = idx"
          @click="selectDisambigMatch(m)"
        >
          <div class="row-between" style="width:100%;">
            <strong style="font-size:12.5px;">{{ m.title }}</strong>
            <span class="pill">{{ m.typeLabel || '知识页' }}</span>
          </div>
          <div class="row-between" style="width:100%;font-size:11px;color:var(--muted);">
            <span class="mono" style="word-break:break-all;">{{ m.path }}</span>
            <span style="flex:none;">{{ shortDate(m.updatedAt) }}</span>
          </div>
        </button>
      </div>
    </div>

    <!-- Footnote Citation Hover Preview Card (Viewport-Clamped with Anti-Flicker Bridge) -->
    <div
      v-if="citeHover"
      class="cite-hover-card"
      :class="citeHover.placement === 'below' ? 'below' : 'above'"
      role="tooltip"
      :style="{
        left: `${citeHover.left}px`,
        top: citeHover.top !== null ? `${citeHover.top}px` : undefined,
        bottom: citeHover.bottom !== null ? `${citeHover.bottom}px` : undefined,
      }"
      @mouseenter="cancelCiteHide"
      @mouseleave="scheduleCiteHide"
    >
      <div class="row-between">
        <strong style="font-size:12.5px;min-width:0;overflow:hidden;text-overflow:ellipsis;white-space:nowrap;">
          {{ citeHover.citation.label }}
        </strong>
        <span
          v-if="citeHover.context"
          class="pill"
          :class="citeHover.context.verified ? 'success' : 'warning'"
        >{{ citeHover.context.verified ? '已核对原件' : '原文有变动' }}</span>
        <span v-else-if="citeHover.loading" class="pill">核对中…</span>
      </div>

      <div
        v-if="citeHover.context?.rawPath || citeHover.citation.rawPath"
        class="mono hint"
        style="font-size:11px;word-break:break-all;"
      >
        {{ citeHover.context?.rawPath || citeHover.citation.rawPath }}
      </div>

      <div class="cite-hover-quote">
        “{{ citeHover.context?.quote || citeHover.citation.quote || citeHover.citation.label }}”
      </div>

      <div class="row-between">
        <span v-if="citeHover.context?.reason" class="hint" style="color:var(--warning);">
          {{ citeHover.context.reason }}
        </span>
        <span v-else class="hint">点击脚注可直接对照原文</span>
        <button
          type="button"
          class="btn quiet xs"
          style="color:var(--accent);"
          @click="openEvidenceFromHover"
        >分屏对照并定位原段 →</button>
      </div>
    </div>
  </div>
</template>
