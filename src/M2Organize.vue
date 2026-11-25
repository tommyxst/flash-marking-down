<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { api } from './api/client'
import { parseAppError, type UiError } from './api/errors'
import ErrorNotice from './components/ErrorNotice.vue'
import DiffViewer from './DiffViewer.vue'
import M2Evidence from './M2Evidence.vue'
import type { Analysis, AnalysisItem, ChangePhase, DocumentChange, DocumentMapping, ModelConfig, RecordSelection, WorkspacePage } from './m2'
import { phaseBadgeTone, useSourcePresentation } from './m2'

const props = defineProps<{
  session: string
  active: boolean
  record?: RecordSelection
  model: ModelConfig | null
  modelReady: boolean
  mappings: DocumentMapping[]
}>()
const emit = defineEmits<{
  showRecords: []
  showSettings: []
  navigate: [page: WorkspacePage]
  review: [change: DocumentChange]
  selectDisagreementSource: [source: RecordSelection]
}>()
const { showSourcePath, sourceMarkdown } = useSourcePresentation()

const phases = { running: '整理中', succeeded: '整理完成', failed: '整理失败', stale: '来源已变化' } as const
const phaseTone: Record<Analysis['status'], string> = {
  running: 'badge-warn',
  succeeded: 'badge-success',
  failed: 'badge-error',
  stale: 'badge-warn',
}
const changePhaseLabels: Record<ChangePhase, string> = {
  pending: '待审阅', deferred: '稍后处理', rejected: '已拒绝', prepared: '待核对写入结果',
  applied: '已采用', conflict: '版本冲突', reverted: '已撤销', superseded: '已由新版预览替代',
}
const sections = {
  discussion: '讨论', decision: '明确决定', action: '候选任务', open: '未决问题', wiki: 'Wiki 建议',
} satisfies Record<AnalysisItem['kind'], string>
const entries = Object.entries(sections) as [AnalysisItem['kind'], string][]
const analyses = ref<Analysis[]>([])
const selectedAnalysis = ref('')
const busy = ref(false)
const error = ref<UiError | null>(null)
const preparing = ref('')
const previewError = ref<UiError | null>(null)
const targets = ref<Record<string, string>>({})
const inlinePreview = ref<{ itemId: string; change: DocumentChange } | null>(null)
const inlineBusy = ref(false)
const inlineFeedback = ref('')
const inlineError = ref<UiError | null>(null)
let readRequest = 0

const current = computed(() => analyses.value.find((entry) => entry.id === selectedAnalysis.value) ?? analyses.value[0])
const taskTarget = computed(() => props.mappings.find((m) => m.kind === 'tasks')?.path)
const wikiTargets = computed(() => props.mappings.filter((m) => m.kind === 'wiki').map((m) => m.path))
const groups = computed(() => entries.map(([kind, label]) => ({
  kind, label, items: current.value?.items.filter((item) => item.kind === kind) ?? [],
})))

watch(() => props.record, () => {
  readRequest += 1
  analyses.value = []
  selectedAnalysis.value = preparing.value = ''
  previewError.value = error.value = null
  inlinePreview.value = null
  inlineFeedback.value = ''
  inlineError.value = null
  targets.value = {}
  busy.value = false
})
watch(() => [props.active, props.record] as const, ([active]) => {
  if (active) void loadAnalyses()
}, { immediate: true })

async function loadAnalyses() {
  if (!props.record || busy.value) return
  const request = ++readRequest
  busy.value = true
  error.value = null
  try {
    const results = await api.listAnalyses(props.session, props.record.path)
    if (request === readRequest) {
      analyses.value = results
      if (!results.some(({ id }) => id === selectedAnalysis.value)) selectedAnalysis.value = results[0]?.id ?? ''
    }
  } catch (failure) {
    if (request === readRequest) error.value = parseAppError(failure)
  } finally {
    if (request === readRequest) busy.value = false
  }
}

async function analyze() {
  const record = props.record
  const endpoint = props.model?.endpoint
  if (!record || !endpoint || !props.modelReady || busy.value) return
  const request = ++readRequest
  busy.value = true
  error.value = null
  inlinePreview.value = null
  try {
    const result = await api.analyzeSource(props.session, record.path, record.contentHash, endpoint)
    if (request === readRequest) {
      analyses.value = [result, ...analyses.value.filter(({ id }) => id !== result.id)]
      selectedAnalysis.value = result.id
    }
  } catch (failure) {
    if (request === readRequest) error.value = parseAppError(failure)
  } finally {
    if (request === readRequest) busy.value = false
  }
}

async function prepareChange(item: AnalysisItem) {
  const target = item.kind === 'action' ? taskTarget.value : targets.value[item.id]
  if (!target || preparing.value || inlineBusy.value) return
  const record = props.record
  preparing.value = item.id
  previewError.value = inlineError.value = null
  inlineFeedback.value = ''
  try {
    const result = await api.prepareAnalysisChange(props.session, item.id, target)
    if (props.record === record && props.active) {
      inlinePreview.value = { itemId: item.id, change: result }
    }
  } catch (failure) {
    if (props.record === record) previewError.value = parseAppError(failure)
  } finally {
    if (props.record === record) preparing.value = ''
  }
}

function inlineReviewable(change: DocumentChange) {
  return change.phase === 'pending' || change.phase === 'deferred'
}

async function applyInlineChange() {
  const active = inlinePreview.value
  if (!active || inlineBusy.value) return
  inlineBusy.value = true
  inlineError.value = null
  inlineFeedback.value = ''
  try {
    const [result] = await api.applyWikiChanges(props.session, [active.change.id])
    if (result?.outcome === 'applied') {
      inlinePreview.value = {
        ...active,
        change: { ...active.change, phase: 'applied' },
      }
      inlineFeedback.value = `已写入 ${active.change.target}。`
    } else {
      inlineError.value = { safeMessage: result?.message || '变更未应用，请到 Wiki 变更页核对。' }
    }
  } catch (failure) {
    inlineError.value = parseAppError(failure)
  } finally {
    inlineBusy.value = false
  }
}

async function decideInlineChange(decision: 'rejected' | 'deferred') {
  const active = inlinePreview.value
  if (!active || inlineBusy.value) return
  inlineBusy.value = true
  inlineError.value = null
  inlineFeedback.value = ''
  try {
    await api.reviewWikiChange(props.session, active.change.id, decision)
    inlinePreview.value = {
      ...active,
      change: { ...active.change, phase: decision },
    }
    inlineFeedback.value = decision === 'rejected' ? '已拒绝此变更。' : '已标记为稍后处理。'
  } catch (failure) {
    inlineError.value = parseAppError(failure)
  } finally {
    inlineBusy.value = false
  }
}
</script>

<template>
  <div class="m2-stack">
    <section v-if="!record" class="empty-panel">
      <h2>选择要整理的记录</h2>
      <p>在时间线或搜索结果的记录详情中选择「整理这条记录」。</p>
      <button class="primary" type="button" @click="emit('showRecords')">前往记录</button>
    </section>
    <template v-else>
      <section class="detail-card" aria-labelledby="organize-source">
        <div class="m2-card-header">
          <h2 id="organize-source">{{ record.title }}</h2>
          <button type="button" class="quiet" @click="emit('showRecords')">更换记录</button>
        </div>
        <code v-if="showSourcePath(record.path, record.isCapture)" class="m2-path">{{ record.path }}</code>
        <div v-if="!modelReady" class="actions">
          <p class="hint">启用模型并配置凭据后可整理。</p>
          <button type="button" @click="emit('showSettings')">设置模型</button>
        </div>
        <div v-else class="actions">
          <button type="button" class="primary" :disabled="busy" @click="analyze">
            {{ current?.status === 'failed' ? '重试整理' : current && record.contentHash !== current.revision ? '重新整理当前版本' : '整理记录' }}
          </button>
          <span v-if="model" class="hint">模型 <code>{{ model.model }}</code></span>
        </div>
        <p v-if="busy" class="hint" role="status">正在读取整理状态或请求模型整理…</p>
        <ErrorNotice :error="error" />
        <div v-if="error || current?.status === 'running'" class="actions">
          <button type="button" :disabled="busy" @click="loadAnalyses">刷新整理状态</button>
        </div>
      </section>

      <section v-if="current" class="m2-stack" aria-label="整理结果">
        <div class="m2-card-header">
          <div class="actions">
            <h2>整理结果</h2>
            <span class="badge" :class="phaseTone[current.status]" role="status">{{ phases[current.status] }}</span>
          </div>
          <label v-if="analyses.length > 1" class="m2-field m2-inline-field">整理历史
            <select v-model="selectedAnalysis">
              <option v-for="(entry, index) in analyses" :key="entry.id" :value="entry.id">{{ index === 0 ? '最近整理' : `较早整理 ${index}` }} · {{ phases[entry.status] }}</option>
            </select>
          </label>
        </div>
        <p v-if="record.contentHash !== current.revision" class="notice">此结果基于先前版本。点击上方「重新整理当前版本」即可刷新。</p>
        <p v-if="current.error" :class="current.status === 'stale' ? 'notice' : 'notice error'" :role="current.status === 'stale' ? 'status' : 'alert'">{{ current.error }}</p>
        <p v-if="(current.status === 'succeeded' || current.status === 'stale') && !current.items.length" class="empty-panel compact">这次整理没有生成新的纪要或变更建议。</p>
        <ErrorNotice :error="previewError" />
        <template v-if="current.status === 'succeeded' || current.status === 'stale'">
          <section v-for="group in groups" :key="group.kind" class="m2-stack">
            <template v-if="group.items.length || group.kind === 'decision' && current.items.length">
              <h3>{{ group.label }}</h3>
              <p v-if="!group.items.length" class="hint">原文没有形成明确决定。</p>
              <article v-for="item in group.items" :key="item.id" class="detail-card">
                <p>{{ sourceMarkdown(item.text) }}</p>
                <div v-if="item.kind === 'action'" class="actions">
                  <span class="badge badge-muted">负责人：{{ item.assignee || '未知' }}</span>
                  <span class="badge badge-muted">截止日期：{{ item.due || '未知' }}</span>
                </div>
                <M2Evidence
                  :session="session"
                  :evidence="item.source"
                  @select-disagreement-source="emit('selectDisagreementSource', $event)"
                />
                <div v-if="item.kind === 'action' && record.contentHash === current.revision" class="actions">
                  <template v-if="taskTarget">
                    <span class="hint">目标 <code>{{ taskTarget }}</code></span>
                    <button type="button" :disabled="Boolean(preparing) || inlineBusy" @click="prepareChange(item)">预览加入任务清单</button>
                  </template>
                  <button v-else type="button" class="quiet" @click="emit('navigate', 'tasks')">配置任务清单</button>
                </div>
                <div v-else-if="(item.kind === 'wiki' || item.kind === 'decision') && record.contentHash === current.revision" class="actions">
                  <template v-if="wikiTargets.length">
                    <label class="m2-field m2-inline-field">Wiki 目标
                      <select v-model="targets[item.id]">
                        <option value="" :selected="!targets[item.id]" disabled>选择目标</option>
                        <option v-for="path in wikiTargets" :key="path" :value="path">{{ path }}</option>
                      </select>
                    </label>
                    <button type="button" :disabled="!targets[item.id] || Boolean(preparing) || inlineBusy" @click="prepareChange(item)">同页预览修改</button>
                  </template>
                  <button v-else type="button" class="quiet" @click="emit('navigate', 'wiki')">配置 Wiki 目标</button>
                </div>
                <p v-if="preparing === item.id" class="hint" role="status">正在读取目标版本并比较…</p>

                <div v-if="inlinePreview?.itemId === item.id" class="inline-review-panel m2-stack">
                  <div class="m2-card-header">
                    <div class="actions">
                      <strong>变更预览</strong>
                      <code>{{ inlinePreview.change.target }}</code>
                      <span class="badge" :class="phaseBadgeTone[inlinePreview.change.phase]">{{ changePhaseLabels[inlinePreview.change.phase] }}</span>
                    </div>
                    <button type="button" class="quiet" :disabled="inlineBusy" @click="inlinePreview = null">收起预览</button>
                  </div>
                  <DiffViewer :before="inlinePreview.change.before" :after="inlinePreview.change.after" :show-full-comparison="false" />
                  <p v-if="inlineFeedback" class="notice success" role="status">{{ inlineFeedback }}</p>
                  <ErrorNotice :error="inlineError" />
                  <div class="actions">
                    <template v-if="inlineReviewable(inlinePreview.change)">
                      <button type="button" class="primary" :disabled="inlineBusy" @click="applyInlineChange">{{ inlineBusy ? '正在写入…' : '采用此项' }}</button>
                      <button type="button" :disabled="inlineBusy" @click="decideInlineChange('rejected')">拒绝</button>
                      <button v-if="inlinePreview.change.phase === 'pending'" type="button" class="quiet" :disabled="inlineBusy" @click="decideInlineChange('deferred')">稍后处理</button>
                    </template>
                    <button type="button" class="quiet" :disabled="inlineBusy" @click="emit('review', inlinePreview.change)">去 Wiki 变更页查看 / 撤销</button>
                  </div>
                </div>
              </article>
            </template>
          </section>
        </template>
      </section>
    </template>
  </div>
</template>
