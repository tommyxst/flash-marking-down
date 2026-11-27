<script setup lang="ts">
import { computed, nextTick, onUnmounted, ref, watch } from 'vue'
import { api } from '../../api/client'
import { parseAppError, type UiError } from '../../api/errors'
import ErrorNotice from '../ErrorNotice.vue'
import ChangesetDiffModal from './ChangesetDiffModal.vue'
import UiIcon from '../../UiIcon.vue'
import {
  conflictStatusLabels,
  type ChangesetItem,
  type ChangesetOp,
  type ConflictItem,
  type LibraryDocSummary,
  type LintFinding,
  type LintReport,
  type OrganizeOverview,
  type SemanticLintPreview,
  type SemanticLintScope,
} from '../../m2'
import {
  changesetStatusLabel,
  changesetStatusTone,
  formatCountdown,
  lintRuleLabel,
  opKindLabel,
  pendingReasonLabel,
  shortChangesetId,
} from '../../utils/wikra'

const props = defineProps<{
  session: string
  overview: OrganizeOverview | null
  loading: boolean
  error: UiError | null
  bufferSecondsLeft: number
  revertingId: string
  lintReport: LintReport | null
  lintLoading: boolean
  lintError: UiError | null
  filterPagePath: string | null
  activeWikiPath: string
  wikiPages: LibraryDocSummary[]
  rollbackFeedbackByCs: Record<string, string>
  revertBlockedByCs: Record<string, { message: string; childIds: string[] }>
}>()

const emit = defineEmits<{
  goLibrary: []
  openCapture: []
  openConflict: [id: string]
  openWiki: [path: string]
  openBufferQueue: []
  flushBuffer: []
  revertChangeset: [id: string]
  refreshLint: []
  clearLintFilter: []
  lintUpdated: [report: LintReport]
  reviewDecided: [payload: { decision: 'apply' | 'discard'; changeset: ChangesetItem; pageTitle: string }]
  escalatedConflict: [conflict: ConflictItem]
  toast: [message: string]
}>()

// ============================================================================
// Card 1: Open Conflicts + Pending Review Pages (Aggregated by changesetId + pagePath)
// ============================================================================
interface PendingPageReview {
  key: string
  changesetId: string
  changesetSummary: string
  path: string
  title: string
  ops: ChangesetOp[]
  reasons: string[]
  summaryText: string
}

const openConflicts = computed(() =>
  (props.overview?.conflicts || []).filter((c) => c.status === 'open'),
)
const lastResolvedConflict = computed(() =>
  (props.overview?.conflicts || []).find((c) => c.status !== 'open') || null,
)
const changesets = computed(() => props.overview?.changesets || [])
const bufferCount = computed(() => props.overview?.bufferQueue.length || 0)

// Strictly never assumed on overview; only lit up after opening diff detail or hitting VERSION_CONFLICT.
const externalModifiedMap = ref<Record<string, boolean>>({})

const pendingReviews = computed<PendingPageReview[]>(() => {
  const result: PendingPageReview[] = []
  for (const cs of changesets.value) {
    if (cs.status === 'reverted') continue
    const byPath = new Map<string, ChangesetOp[]>()
    for (const op of cs.ops || []) {
      if (op.outcome !== 'pending_review') continue
      const list = byPath.get(op.targetPath) || []
      list.push(op)
      byPath.set(op.targetPath, list)
    }
    for (const [path, ops] of byPath.entries()) {
      const title = ops[0]?.targetTitle || path.split('/').pop()?.replace(/\.md$/i, '') || path
      const reasonSet = new Set<string>()
      for (const op of ops) {
        if (op.reason?.trim()) reasonSet.add(pendingReasonLabel(op.reason))
      }
      const labels = ops.map((o) => o.label || opKindLabel(o.op)).filter(Boolean)
      const uniqueLabels = Array.from(new Set(labels))
      result.push({
        key: `${cs.id}::${path}`,
        changesetId: cs.id,
        changesetSummary: cs.summary,
        path,
        title,
        ops,
        reasons: Array.from(reasonSet),
        summaryText: uniqueLabels.join(' · ') || '包含待确认改动',
      })
    }
  }
  return result
})

const totalPriorityCount = computed(() => openConflicts.value.length + pendingReviews.value.length)
const priorityPillText = computed(() => {
  const c = openConflicts.value.length
  const p = pendingReviews.value.length
  if (c > 0 && p > 0) return `${c} 处冲突 · ${p} 篇待审`
  if (c > 0) return `${c} 处冲突`
  if (p > 0) return `${p} 篇待审`
  return '已全部确认'
})

// Diff modal state
const diffModal = ref<{ open: boolean; changesetId: string; pagePath: string; summary: string }>({
  open: false,
  changesetId: '',
  pagePath: '',
  summary: '',
})

function openDiffModal(changesetId: string, pagePath: string, summary = '') {
  diffModal.value = { open: true, changesetId, pagePath, summary }
}

function handleExternalStatus(payload: { changesetId: string; path: string; externallyModified: boolean }) {
  const key = `${payload.changesetId}::${payload.path}`
  externalModifiedMap.value = {
    ...externalModifiedMap.value,
    [key]: payload.externallyModified,
  }
}

// ============================================================================
// Card 2: Knowledge Base Health Check (L1 Structure Lint + L2 Semantic Lint)
// ============================================================================
const lintCardEl = ref<HTMLElement>()
const lintTab = ref<'attention' | 'all' | 'dismissed'>('attention')
const fixingIds = ref<Record<string, boolean>>({})
const batchFixing = ref(false)
const lintActionError = ref<UiError | null>(null)
const localDismissedOverrides = ref<Record<string, boolean>>({})

// L2 semantic findings returned from run_semantic_lint in this session
const sessionSemanticFindings = ref<LintFinding[]>([])
const semanticRanOnce = ref(false)
const semanticScopeKind = ref<'page' | 'recent' | 'folder'>(props.activeWikiPath ? 'page' : 'recent')
const selectedFolder = ref('')
const semanticPreview = ref<SemanticLintPreview | null>(null)
const semanticPreviewLoading = ref(false)
const semanticRunning = ref(false)
const semanticError = ref<UiError | null>(null)
const escalatingId = ref('')

const wikiFolders = computed(() => {
  const set = new Set<string>()
  for (const p of props.wikiPages) {
    const folder = (p.folder || '').replace(/^\/+|\/+$/g, '')
    if (folder) set.add(folder)
  }
  const sorted = Array.from(set).sort((a, b) => a.localeCompare(b))
  return sorted.length ? sorted : ['wiki']
})

watch(
  wikiFolders,
  (folders) => {
    if (!selectedFolder.value || !folders.includes(selectedFolder.value)) {
      selectedFolder.value = folders[0] || 'wiki'
    }
  },
  { immediate: true },
)

watch(
  () => props.activeWikiPath,
  (path) => {
    if (!path && semanticScopeKind.value === 'page') {
      semanticScopeKind.value = 'recent'
    }
  },
  { immediate: true },
)

watch(
  () => props.filterPagePath,
  (path) => {
    if (path) {
      lintTab.value = 'attention'
      void nextTick(() => {
        lintCardEl.value?.scrollIntoView({ behavior: 'smooth', block: 'start' })
      })
    }
  },
  { immediate: true },
)

watch(
  () => props.lintReport,
  () => {
    localDismissedOverrides.value = {}
  },
)

const mergedFindings = computed<LintFinding[]>(() => {
  const byId = new Map<string, LintFinding>()
  for (const f of props.lintReport?.findings || []) {
    const override = localDismissedOverrides.value[f.id]
    byId.set(f.id, override === undefined ? f : { ...f, dismissed: override })
  }
  for (const f of sessionSemanticFindings.value) {
    const existing = byId.get(f.id)
    const override = localDismissedOverrides.value[f.id]
    const dismissed = override !== undefined ? override : (existing?.dismissed ?? f.dismissed)
    byId.set(f.id, { ...f, dismissed })
  }
  return Array.from(byId.values())
})

const scopedFindings = computed(() => {
  const filter = props.filterPagePath
  if (!filter) return mergedFindings.value
  return mergedFindings.value.filter(
    (f) => f.pagePath === filter || f.relatedPaths.includes(filter),
  )
})

const l1ScopedFindings = computed(() =>
  scopedFindings.value.filter((f) => f.layer !== 'L2'),
)

const l2OpenFindings = computed(() =>
  mergedFindings.value.filter((f) => f.layer === 'L2' && !f.dismissed),
)

const attentionFindings = computed(() =>
  l1ScopedFindings.value.filter((f) => !f.dismissed && (f.severity === 'error' || f.severity === 'warn')),
)
const openAllFindings = computed(() =>
  l1ScopedFindings.value.filter((f) => !f.dismissed),
)
const dismissedFindings = computed(() =>
  scopedFindings.value.filter((f) => f.dismissed),
)
const foldedInfoCount = computed(() =>
  l1ScopedFindings.value.filter((f) => !f.dismissed && f.severity === 'info').length,
)

const visibleLintFindings = computed(() => {
  if (lintTab.value === 'attention') return attentionFindings.value
  if (lintTab.value === 'all') return openAllFindings.value
  return dismissedFindings.value
})

const lintErrorCount = computed(() =>
  l1ScopedFindings.value.filter((f) => !f.dismissed && f.severity === 'error').length,
)
const lintWarnCount = computed(() =>
  l1ScopedFindings.value.filter((f) => !f.dismissed && f.severity === 'warn').length,
)

// Safe batch fix ONLY collects non-dismissed findings where fix.kind === 'safe'
// (broken_link has fix.kind === 'one_click' and is strictly excluded from batch fix).
const safeFixFindings = computed(() =>
  l1ScopedFindings.value.filter((f) => !f.dismissed && f.fix?.kind === 'safe'),
)

const filterPageTitle = computed(() => {
  if (!props.filterPagePath) return ''
  const matched = props.wikiPages.find((p) => p.path === props.filterPagePath)
  return matched?.title || props.filterPagePath.split('/').pop()?.replace(/\.md$/i, '') || props.filterPagePath
})

function openPageButtonLabel(rule: string): string {
  if (rule === 'missing_source' || rule === 'quote_mismatch') return '打开页面核对'
  if (rule === 'structure_blocked') return '打开页面修复'
  if (rule === 'index_drift') return '打开目录页'
  return '打开页面'
}

function severityTone(sev: LintFinding['severity']): string {
  if (sev === 'error') return 'error'
  if (sev === 'warn') return 'warning'
  return 'info'
}

async function applySafeBatchFix() {
  if (!props.session || batchFixing.value || !safeFixFindings.value.length) return
  const ids = safeFixFindings.value.map((f) => f.id)
  batchFixing.value = true
  lintActionError.value = null
  try {
    const res = await api.applyLintFix(props.session, ids)
    emit('lintUpdated', res.report)
    const parts: string[] = []
    if (res.changeset) parts.push(`已生成可撤销整理记录`)
    if (res.indexRebuilt) parts.push(`已同步知识库目录页`)
    emit('toast', parts.length ? `安全修复完成：${parts.join('，')}` : '已完成安全项修复')
  } catch (err) {
    lintActionError.value = parseAppError(err)
  } finally {
    batchFixing.value = false
  }
}

async function applySingleLintFix(finding: LintFinding) {
  if (!props.session || fixingIds.value[finding.id] || batchFixing.value) return
  fixingIds.value = { ...fixingIds.value, [finding.id]: true }
  lintActionError.value = null
  try {
    const res = await api.applyLintFix(props.session, [finding.id])
    emit('lintUpdated', res.report)
    emit(
      'toast',
      res.indexRebuilt && !res.changeset
        ? '已重新生成目录页托管区块'
        : `已修复「${lintRuleLabel(finding.rule)}」，支持在右侧一键撤销`,
    )
  } catch (err) {
    lintActionError.value = parseAppError(err)
  } finally {
    const next = { ...fixingIds.value }
    delete next[finding.id]
    fixingIds.value = next
  }
}

async function dismissLint(finding: LintFinding) {
  if (!props.session) return
  lintActionError.value = null
  try {
    await api.dismissSuggestion(props.session, finding.id)
    localDismissedOverrides.value = { ...localDismissedOverrides.value, [finding.id]: true }
    emit('refreshLint')
  } catch (err) {
    lintActionError.value = parseAppError(err)
  }
}

async function undismissLint(finding: LintFinding) {
  if (!props.session) return
  lintActionError.value = null
  try {
    await api.undismissSuggestion(props.session, finding.id)
    localDismissedOverrides.value = { ...localDismissedOverrides.value, [finding.id]: false }
    emit('refreshLint')
    emit('toast', '已恢复该条体检提示')
  } catch (err) {
    lintActionError.value = parseAppError(err)
  }
}

function buildSemanticScope(): SemanticLintScope {
  if (semanticScopeKind.value === 'page' && props.activeWikiPath) {
    return { kind: 'page', path: props.activeWikiPath }
  }
  if (semanticScopeKind.value === 'folder') {
    return { kind: 'folder', path: selectedFolder.value || 'wiki' }
  }
  return { kind: 'recent', days: 7 }
}

async function loadSemanticPreview() {
  if (!props.session) return
  semanticPreviewLoading.value = true
  semanticError.value = null
  try {
    semanticPreview.value = await api.previewSemanticLint(props.session, buildSemanticScope())
  } catch (err) {
    semanticPreview.value = null
    semanticError.value = parseAppError(err)
  } finally {
    semanticPreviewLoading.value = false
  }
}

async function startSemanticLint() {
  if (!props.session || semanticRunning.value || !semanticPreview.value || semanticPreview.value.pairCount === 0) {
    return
  }
  semanticRunning.value = true
  semanticError.value = null
  try {
    const findings = await api.runSemanticLint(props.session, buildSemanticScope())
    sessionSemanticFindings.value = findings
    semanticRanOnce.value = true
    emit('refreshLint')
    emit(
      'toast',
      findings.length > 0
        ? `定向语义体检完成，发现 ${findings.length} 处疑似重复或分歧`
        : '定向语义体检完成，未发现明显重复或观点冲突',
    )
  } catch (err) {
    semanticError.value = parseAppError(err)
  } finally {
    semanticRunning.value = false
  }
}

async function escalateToConflict(finding: LintFinding) {
  if (!props.session || escalatingId.value) return
  escalatingId.value = finding.id
  semanticError.value = null
  try {
    const conflict = await api.escalateLintToConflict(props.session, finding.id)
    sessionSemanticFindings.value = sessionSemanticFindings.value.filter((f) => f.id !== finding.id)
    emit('escalatedConflict', conflict)
    emit('refreshLint')
  } catch (err) {
    semanticError.value = parseAppError(err)
  } finally {
    escalatingId.value = ''
  }
}

watch(
  () => [props.session, semanticScopeKind.value, selectedFolder.value, props.activeWikiPath] as const,
  ([session]) => {
    if (session) void loadSemanticPreview()
  },
  { immediate: true },
)

// ============================================================================
// Right Column: Multi-page Changesets Helpers & Highlight Locate
// ============================================================================
const highlightedCsId = ref('')
let highlightTimer: number | undefined

function locateChangeset(changesetId: string) {
  highlightedCsId.value = changesetId
  window.clearTimeout(highlightTimer)
  void nextTick(() => {
    const el = document.getElementById(`cs-card-${changesetId}`)
    el?.scrollIntoView({ behavior: 'smooth', block: 'center' })
  })
  highlightTimer = window.setTimeout(() => {
    if (highlightedCsId.value === changesetId) highlightedCsId.value = ''
  }, 2400)
}

onUnmounted(() => {
  window.clearTimeout(highlightTimer)
})

function opTagClass(op: string): string {
  if (op === 'create_page') return 'create'
  if (op === 'replace_section') return 'replace'
  return ''
}

function pageStateForOp(cs: ChangesetItem, op: ChangesetOp): string | null {
  const matched = (cs.pages || []).find((p) => p.path === op.targetPath)
  return matched?.state ?? null
}

function opOutcomeBadge(cs: ChangesetItem, op: ChangesetOp): { text: string; tone: string; clickable: boolean } {
  if (op.outcome === 'pending_review') {
    return { text: '等待你确认', tone: 'warning', clickable: true }
  }
  const pageState = pageStateForOp(cs, op)
  if (op.outcome === 'dropped') {
    return { text: '已跳过', tone: '', clickable: false }
  }
  if (op.outcome === 'discarded') {
    return { text: '已丢弃', tone: '', clickable: false }
  }
  if (pageState === 'kept_modified') {
    return { text: '已保留你的修改', tone: 'info', clickable: false }
  }
  if (pageState === 'reverted' || op.outcome === 'reverted') {
    return { text: '已还原', tone: '', clickable: false }
  }
  if (op.outcome === 'applied') {
    return { text: '已写入', tone: 'success', clickable: false }
  }
  return { text: '已记录', tone: '', clickable: false }
}

function canRevertChangeset(cs: ChangesetItem): boolean {
  if (cs.status === 'reverted') return false
  const hasAppliedPage = (cs.pages || []).some((p) => p.state === 'applied')
  const hasPendingOp = (cs.ops || []).some((o) => o.outcome === 'pending_review')
  if (cs.pages && cs.pages.length > 0) {
    return hasAppliedPage || hasPendingOp
  }
  return true
}
</script>

<template>
  <div class="stage-body">
    <div class="organize-wrap">
      <section class="organize-header">
        <div>
          <h1>整理与同步</h1>
          <p>平时后台会自动把你的随手笔记和收藏文章整理进知识库；只有出现观点矛盾或需确认的关键改动时，才会在这里请你确认。</p>
        </div>
        <div class="row-flex">
          <button type="button" class="btn sm" @click="emit('goLibrary')">
            <UiIcon name="book" size="sm" /> 回到知识库
          </button>
          <button type="button" class="btn primary sm" @click="emit('openCapture')">
            <UiIcon name="plus" size="sm" /> 记一条随手笔记
          </button>
        </div>
      </section>

      <ErrorNotice v-if="error" :error="error" />

      <div v-if="loading && !overview" class="hint">正在读取整理状态…</div>

      <div v-else class="organize-cols">
        <!-- Left Column: Card 1 (Conflicts + Pending Reviews) + Card 2 (Health Check) -->
        <div class="org-col">
          <!-- Card 1 -->
          <section class="ops-card" :class="{ priority: totalPriorityCount > 0 }">
            <div class="ops-card-head">
              <div class="ops-card-title">
                <UiIcon name="conflict" />
                <span>需要你确认的冲突与待审改动</span>
                <span class="pill" :class="totalPriorityCount > 0 ? 'warning' : 'success'">
                  {{ priorityPillText }}
                </span>
              </div>
            </div>

            <div v-if="totalPriorityCount > 0" class="ops-list">
              <!-- Open Conflicts -->
              <div v-for="c in openConflicts" :key="`conflict-${c.id}`" class="ops-row">
                <div class="ops-row-main">
                  <div class="row-flex">
                    <span class="pill warning">观点冲突</span>
                    <span class="ops-row-title">{{ c.topic }}</span>
                  </div>
                  <span class="ops-row-desc">
                    {{ c.currentLabel || '原有结论' }} 与 {{ c.incomingLabel || '新来源' }} 存在分歧，确认以哪条为准。
                  </span>
                </div>
                <div class="ops-row-actions">
                  <button
                    type="button"
                    class="btn primary sm"
                    @click="emit('openConflict', c.id)"
                  >左右对比确认</button>
                </div>
              </div>

              <!-- Aggregated Per-Page Pending Reviews -->
              <div v-for="item in pendingReviews" :key="item.key" class="ops-row">
                <div class="ops-row-main">
                  <div class="row-flex">
                    <span class="pill accent">待审改动</span>
                    <span class="ops-row-title">{{ item.title }}</span>
                    <span v-for="reason in item.reasons" :key="reason" class="pill warning">
                      {{ reason }}
                    </span>
                    <span v-if="externalModifiedMap[item.key]" class="pill error">
                      文件已有外部变动
                    </span>
                  </div>
                  <span class="ops-row-desc">{{ item.summaryText }}</span>
                </div>
                <div class="ops-row-actions">
                  <button
                    type="button"
                    class="btn primary sm"
                    @click="openDiffModal(item.changesetId, item.path, item.changesetSummary)"
                  >查看差异并确认</button>
                </div>
              </div>
            </div>

            <div v-else class="ops-row">
              <div class="ops-row-main">
                <span class="ops-row-title">当前没有观点冲突或待审改动，知识库保持一致</span>
                <span class="ops-row-desc">
                  <template v-if="lastResolvedConflict">
                    上次确认：《{{ lastResolvedConflict.topic }}》· {{ conflictStatusLabels[lastResolvedConflict.status] }}
                    <template v-if="lastResolvedConflict.resolutionNote">（{{ lastResolvedConflict.resolutionNote }}）</template>。
                  </template>
                  <template v-else>
                    后台整理仅在发现明显观点分歧、核心页变动或替换章节时请你确认。
                  </template>
                </span>
              </div>
              <div v-if="lastResolvedConflict?.currentPath" class="ops-row-actions">
                <button
                  type="button"
                  class="btn sm"
                  @click="emit('openWiki', lastResolvedConflict.currentPath!)"
                >查看笔记</button>
              </div>
            </div>
          </section>

          <!-- Card 2: Knowledge Base Health Check (L1 + L2) -->
          <section ref="lintCardEl" class="ops-card">
            <div class="ops-card-head">
              <div class="ops-card-title">
                <UiIcon name="shield" />
                <span>知识库体检</span>
                <span
                  v-if="lintReport"
                  class="pill"
                  :class="lintErrorCount > 0 ? 'error' : lintWarnCount > 0 ? 'warning' : 'success'"
                >
                  {{ lintErrorCount === 0 && lintWarnCount === 0
                    ? '未发现异常'
                    : `${lintErrorCount} 项错误 · ${lintWarnCount} 项提醒` }}
                </span>
              </div>
              <div class="row-flex">
                <button
                  type="button"
                  class="btn quiet sm"
                  :disabled="lintLoading || batchFixing"
                  @click="emit('refreshLint')"
                >
                  <UiIcon name="refresh" size="sm" />
                  {{ lintLoading ? '检查中…' : '重新检查' }}
                </button>
                <button
                  v-if="safeFixFindings.length > 0"
                  type="button"
                  class="btn primary sm"
                  :disabled="batchFixing || lintLoading"
                  @click="applySafeBatchFix"
                >{{ batchFixing ? '正在修复…' : `一键修复安全项 (${safeFixFindings.length})` }}</button>
              </div>
            </div>

            <!-- Single-page filter bar when jumped from WikiDocumentView -->
            <div v-if="filterPagePath" class="lint-page-filter-bar">
              <span>正在查看《{{ filterPageTitle }}》的体检项</span>
              <button type="button" class="btn sm" @click="emit('clearLintFilter')">查看全库</button>
            </div>

            <!-- 3-state filter capsules -->
            <div class="lint-filter-row">
              <div class="lint-filter-pills" role="tablist" aria-label="体检项筛选">
                <button
                  type="button"
                  role="tab"
                  :aria-selected="lintTab === 'attention'"
                  :class="{ active: lintTab === 'attention' }"
                  @click="lintTab = 'attention'"
                >需关注 ({{ attentionFindings.length }})</button>
                <button
                  type="button"
                  role="tab"
                  :aria-selected="lintTab === 'all'"
                  :class="{ active: lintTab === 'all' }"
                  @click="lintTab = 'all'"
                >全部 ({{ openAllFindings.length }})</button>
                <button
                  type="button"
                  role="tab"
                  :aria-selected="lintTab === 'dismissed'"
                  :class="{ active: lintTab === 'dismissed' }"
                  @click="lintTab = 'dismissed'"
                >已忽略 ({{ dismissedFindings.length }})</button>
              </div>
              <span v-if="lintReport" class="hint">
                共检查 {{ lintReport.pageCount }} 篇知识页
              </span>
            </div>

            <ErrorNotice v-if="lintError" :error="lintError" />
            <ErrorNotice v-if="lintActionError" :error="lintActionError" />

            <div v-if="lintLoading && !lintReport" class="hint">正在扫描知识库链接与引文状态…</div>

            <div v-else class="ops-list">
              <div
                v-for="f in visibleLintFindings"
                :key="f.id"
                class="lint-row"
                :class="`sev-${f.severity}`"
              >
                <div class="lint-row-main">
                  <div class="row-flex">
                    <span class="pill" :class="severityTone(f.severity)">{{ lintRuleLabel(f.rule) }}</span>
                    <span class="ops-row-title">{{ f.pageTitle || f.pagePath }}</span>
                    <span v-if="f.line" class="hint mono">第 {{ f.line }} 行</span>
                  </div>
                  <span class="ops-row-desc">{{ f.message }}</span>
                  <span v-if="f.pagePath" class="lint-row-meta">{{ f.pagePath }}</span>

                  <div v-if="f.evidence" class="l2-quotes-grid" style="margin-top:4px;">
                    <div class="l2-quote-box">
                      <button type="button" class="wikilink" style="text-align:left;font-weight:600;" @click="emit('openWiki', f.evidence.pathA)">
                        {{ f.evidence.titleA || f.evidence.pathA }}
                      </button>
                      <span>“{{ f.evidence.quoteA }}”</span>
                    </div>
                    <div class="l2-quote-box">
                      <button type="button" class="wikilink" style="text-align:left;font-weight:600;" @click="emit('openWiki', f.evidence.pathB)">
                        {{ f.evidence.titleB || f.evidence.pathB }}
                      </button>
                      <span>“{{ f.evidence.quoteB }}”</span>
                    </div>
                  </div>
                </div>

                <div class="ops-row-actions">
                  <template v-if="!f.dismissed">
                    <button
                      v-if="f.fix"
                      type="button"
                      class="btn primary sm"
                      :disabled="Boolean(fixingIds[f.id]) || batchFixing"
                      @click="applySingleLintFix(f)"
                    >{{ fixingIds[f.id] ? '处理中…' : f.fix.label }}</button>
                    <button
                      v-if="f.pagePath"
                      type="button"
                      class="btn sm"
                      @click="emit('openWiki', f.pagePath)"
                    >{{ openPageButtonLabel(f.rule) }}</button>
                    <button
                      type="button"
                      class="btn quiet sm"
                      @click="dismissLint(f)"
                    >忽略</button>
                  </template>
                  <template v-else>
                    <button
                      v-if="f.pagePath"
                      type="button"
                      class="btn sm"
                      @click="emit('openWiki', f.pagePath)"
                    >打开页面</button>
                    <button
                      type="button"
                      class="btn sm"
                      @click="undismissLint(f)"
                    >恢复提示</button>
                  </template>
                </div>
              </div>

              <p v-if="!visibleLintFindings.length" class="hint" style="padding:4px 2px;">
                <template v-if="lintTab === 'attention'">
                  {{ filterPagePath ? '这篇知识页当前没有需要处理的错误或提醒。' : '当前没有需要关注的错误或提醒，知识库结构健康。' }}
                </template>
                <template v-else-if="lintTab === 'all'">
                  当前范围未检出任何体检项。
                </template>
                <template v-else>
                  当前没有已忽略的体检提示。
                </template>
              </p>

              <div v-if="lintTab === 'attention' && foldedInfoCount > 0" class="lint-collapsed-note">
                <span>已折叠 {{ foldedInfoCount }} 项低优先级建议（如单薄页、未关联页），切至「全部」可查看</span>
                <button type="button" class="btn quiet xs" @click="lintTab = 'all'">查看全部</button>
              </div>
            </div>

            <!-- L2 Targeted Semantic Health Check Footer -->
            <div class="l2-zone">
              <div class="row-between">
                <div class="row-flex">
                  <strong style="font-size:13px;">定向语义体检</strong>
                  <span class="hint">按需深度排查重复页面与潜在观点分歧</span>
                </div>
                <button
                  type="button"
                  class="btn quiet xs"
                  :disabled="semanticPreviewLoading || semanticRunning"
                  @click="loadSemanticPreview"
                >刷新预估</button>
              </div>

              <div class="row-flex">
                <div class="lint-filter-pills" role="radiogroup" aria-label="语义体检范围">
                  <button
                    type="button"
                    :disabled="!activeWikiPath"
                    :title="activeWikiPath ? '扫描当前知识页及直接关联的相邻页面' : '请先打开一篇知识页'"
                    :class="{ active: semanticScopeKind === 'page' }"
                    @click="activeWikiPath && (semanticScopeKind = 'page')"
                  >当前页邻域</button>
                  <button
                    type="button"
                    :class="{ active: semanticScopeKind === 'recent' }"
                    @click="semanticScopeKind = 'recent'"
                  >近 7 天变动</button>
                  <button
                    type="button"
                    :class="{ active: semanticScopeKind === 'folder' }"
                    @click="semanticScopeKind = 'folder'"
                  >指定文件夹</button>
                </div>

                <select
                  v-if="semanticScopeKind === 'folder'"
                  v-model="selectedFolder"
                  class="compact-select"
                  aria-label="选择知识库文件夹"
                >
                  <option v-for="folder in wikiFolders" :key="folder" :value="folder">
                    {{ folder }}/
                  </option>
                </select>
              </div>

              <div class="l2-budget-strip">
                <div style="flex:1 1 240px;min-width:0;display:flex;flex-direction:column;gap:2px;">
                  <template v-if="semanticPreviewLoading">
                    <span class="hint">正在本地估算扫描页数与候选对…</span>
                  </template>
                  <template v-else-if="semanticPreview">
                    <span>
                      预计扫描 <strong>{{ semanticPreview.pageCount }}</strong> 篇页面 ·
                      筛出 <strong>{{ semanticPreview.pairCount }}</strong> 组候选对 ·
                      约 <strong>{{ semanticPreview.estimatedChars }}</strong> 字 ·
                      模型：{{ semanticPreview.modelAlias || '当前配置模型' }}
                    </span>
                    <span class="hint">
                      {{ semanticPreview.pairCount === 0
                        ? '当前范围内未筛出需要深度比对的相似页面'
                        : '仅发送候选摘录，确认后才调用模型' }}
                    </span>
                  </template>
                  <template v-else>
                    <span class="hint">选择范围后可查看预估扫描页数与字数</span>
                  </template>
                </div>

                <button
                  type="button"
                  class="btn primary sm"
                  :disabled="semanticRunning || semanticPreviewLoading || !semanticPreview || semanticPreview.pairCount === 0"
                  @click="startSemanticLint"
                >{{ semanticRunning ? '正在体检…' : '确认开始语义体检' }}</button>
              </div>

              <ErrorNotice v-if="semanticError" :error="semanticError" />

              <div v-if="l2OpenFindings.length > 0" class="ops-list">
                <div v-for="item in l2OpenFindings" :key="item.id" class="l2-result-card">
                  <div class="row-between">
                    <div class="row-flex">
                      <span class="pill warning">{{ lintRuleLabel(item.rule) }}</span>
                      <strong style="font-size:13px;">{{ item.message }}</strong>
                    </div>
                    <div class="row-flex">
                      <button
                        type="button"
                        class="btn primary sm"
                        :disabled="escalatingId === item.id"
                        @click="escalateToConflict(item)"
                      >{{ escalatingId === item.id ? '处理中…' : '转为冲突' }}</button>
                      <button
                        type="button"
                        class="btn quiet sm"
                        @click="dismissLint(item)"
                      >忽略</button>
                    </div>
                  </div>

                  <div v-if="item.evidence" class="l2-quotes-grid">
                    <div class="l2-quote-box">
                      <button
                        type="button"
                        class="wikilink"
                        style="text-align:left;font-weight:600;"
                        @click="emit('openWiki', item.evidence.pathA)"
                      >{{ item.evidence.titleA || item.evidence.pathA }}</button>
                      <span>“{{ item.evidence.quoteA }}”</span>
                    </div>
                    <div class="l2-quote-box">
                      <button
                        type="button"
                        class="wikilink"
                        style="text-align:left;font-weight:600;"
                        @click="emit('openWiki', item.evidence.pathB)"
                      >{{ item.evidence.titleB || item.evidence.pathB }}</button>
                      <span>“{{ item.evidence.quoteB }}”</span>
                    </div>
                  </div>
                </div>
              </div>

              <p v-else-if="semanticRanOnce && !semanticRunning" class="hint">
                本次定向语义体检已完成，未发现明显重复内容或观点冲突。
              </p>
            </div>
          </section>
        </div>

        <!-- Right Column: Buffer Banner + Multi-page Changesets -->
        <div class="org-col">
          <div v-if="bufferSecondsLeft > 0 || bufferCount > 0" class="buffer-banner">
            <div>
              <strong>有 {{ bufferCount || 1 }} 条新笔记正在等待合并整理</strong>
              <div style="font-size:12px;color:var(--text-secondary);margin-top:2px;">
                <template v-if="bufferSecondsLeft > 0">
                  将在 <strong class="mono">{{ formatCountdown(bufferSecondsLeft) }}</strong> 后自动开始，写错了可先移出。
                </template>
                <template v-else>
                  正在准备合并整理，写错了可先移出本次队列。
                </template>
              </div>
            </div>
            <div class="row-flex" style="flex:none;">
              <button type="button" class="btn sm" @click="emit('openBufferQueue')">查看 / 撤回</button>
              <button type="button" class="btn primary sm" @click="emit('flushBuffer')">立即整理</button>
            </div>
          </div>

          <section class="ops-card">
            <div class="ops-card-head">
              <div class="ops-card-title">
                <UiIcon name="history" />
                <span>AI 最近帮你整理了什么</span>
              </div>
              <span class="hint">每一批都支持按页安全撤销</span>
            </div>

            <div class="ops-list">
              <div
                v-for="cs in changesets"
                :id="`cs-card-${cs.id}`"
                :key="cs.id"
                class="cs-card"
                :class="{ highlighted: highlightedCsId === cs.id }"
              >
                <div class="row-between">
                  <strong style="font-size:13px;word-break:break-word;">{{ cs.summary }}</strong>
                  <span class="pill" :class="changesetStatusTone(cs.status)">
                    {{ changesetStatusLabel(cs.status) }}
                  </span>
                </div>

                <div class="row-flex" style="font-size:11.5px;color:var(--muted);">
                  <span>
                    {{ cs.time }} · {{ cs.trigger }}
                    <template v-if="cs.modelAlias"> · {{ cs.modelAlias }}</template>
                  </span>
                  <button
                    v-if="cs.parentId"
                    type="button"
                    class="pill info cs-parent-link"
                    title="定位并高亮来源整理批次"
                    @click="locateChangeset(cs.parentId)"
                  >由待审确认生成 · 关联 #{{ shortChangesetId(cs.parentId) }}</button>
                </div>

                <div v-if="cs.note" class="hint">{{ cs.note }}</div>

                <div v-if="cs.droppedClaims > 0" class="cs-gentle-note">
                  已跳过 {{ cs.droppedClaims }} 条不符原文的结论（未写入知识库）
                </div>

                <div v-if="cs.ops.length" class="editop-list">
                  <div v-for="(op, idx) in cs.ops" :key="`${cs.id}-${op.seq ?? idx}`" class="cs-op-row">
                    <div class="cs-op-left">
                      <span class="cs-op-tag" :class="opTagClass(op.op)">
                        {{ opKindLabel(op.op) }}
                      </span>
                      <button
                        v-if="op.targetPath"
                        type="button"
                        class="wikilink"
                        style="text-align:left;word-break:break-all;"
                        @click="emit('openWiki', op.targetPath)"
                      >{{ op.targetTitle || op.targetPath }}</button>
                      <span v-else>{{ op.targetTitle }}</span>
                      <span v-if="op.label && op.label !== opKindLabel(op.op)" class="hint">
                        {{ op.label }}
                      </span>
                      <span v-if="op.outcome === 'dropped' && op.reason" class="hint">
                        （{{ op.reason }}）
                      </span>
                    </div>

                    <div class="row-flex" style="flex:none;">
                      <button
                        v-if="opOutcomeBadge(cs, op).clickable"
                        type="button"
                        class="pill warning cs-parent-link"
                        @click="openDiffModal(cs.id, op.targetPath, cs.summary)"
                      >{{ opOutcomeBadge(cs, op).text }}</button>
                      <span v-else class="pill" :class="opOutcomeBadge(cs, op).tone">
                        {{ opOutcomeBadge(cs, op).text }}
                      </span>
                    </div>
                  </div>
                </div>

                <!-- Persistent inline error when child changeset must be reverted first -->
                <div v-if="revertBlockedByCs[cs.id]" class="cs-revert-blocked" role="alert">
                  <span style="flex:1 1 200px;">{{ revertBlockedByCs[cs.id].message }}</span>
                  <div v-if="revertBlockedByCs[cs.id].childIds.length" class="row-flex">
                    <button
                      v-for="cid in revertBlockedByCs[cs.id].childIds"
                      :key="cid"
                      type="button"
                      class="btn sm"
                      @click="locateChangeset(cid)"
                    >定位子批次 #{{ shortChangesetId(cid) }}</button>
                  </div>
                </div>

                <!-- Partial / full rollback feedback banner -->
                <div v-if="rollbackFeedbackByCs[cs.id]" class="cs-rollback-feedback" role="status">
                  {{ rollbackFeedbackByCs[cs.id] }}
                </div>

                <div class="row-flex">
                  <button
                    v-if="canRevertChangeset(cs)"
                    type="button"
                    class="btn sm"
                    :disabled="revertingId === cs.id"
                    @click="emit('revertChangeset', cs.id)"
                  >
                    <UiIcon name="undo" size="sm" />
                    {{ revertingId === cs.id ? '正在撤销…' : '撤销此次整理' }}
                  </button>
                  <button
                    v-if="cs.status === 'partial' && openConflicts.length > 0"
                    type="button"
                    class="btn sm"
                    @click="emit('openConflict', openConflicts[0].id)"
                  >去确认冲突</button>
                </div>
              </div>

              <p v-if="!changesets.length" style="font-size:12.5px;color:var(--muted);">
                还没有 AI 整理记录。写下随手笔记或在收件箱提炼文章后，每批改动都会列在这里，随时可一键撤销。
              </p>
            </div>
          </section>
        </div>
      </div>
    </div>

    <ChangesetDiffModal
      :open="diffModal.open"
      :session="session"
      :changeset-id="diffModal.changesetId"
      :page-path="diffModal.pagePath"
      :changeset-summary="diffModal.summary"
      @close="diffModal.open = false"
      @open-wiki="emit('openWiki', $event)"
      @external-status="handleExternalStatus"
      @decided="emit('reviewDecided', $event)"
    />
  </div>
</template>
