<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watch } from 'vue'
import { api } from './api/client'
import { parseAppError, type UiError } from './api/errors'
import ErrorNotice from './components/ErrorNotice.vue'
import M2Evidence from './M2Evidence.vue'
import M2Relation from './M2Relation.vue'
import {
  disagreementStatuses, relationLabels, useSourcePresentation,
  type Answer, type AnswerPreparation, type Disagreement, type DisagreementStatus, type ModelConfig, type RecordSelection, type Relation,
  type OutputKind, type PossibleDisagreement,
} from './m2'

const props = defineProps<{
  session: string
  active: boolean
  sources: [RecordSelection | null, RecordSelection | null]
  model: ModelConfig | null
  modelReady: boolean
}>()
const emit = defineEmits<{
  showRecords: []
  showSettings: []
  removeSource: [index: number]
  clearSources: []
  selectDisagreementSource: [source: RecordSelection]
}>()
const { sourceMarkdown } = useSourcePresentation()

const pane = ref<'answer' | 'relations'>('answer')
const outputLabels: Record<OutputKind, string> = { answer: '回答', summary: '纪要', plan: '方案' }
const outputKind = ref<OutputKind>('answer')
const query = ref('')
const keyword = ref('')
const anchorClaimId = ref<string>()
const dateBasis = ref<'recorded' | 'event'>('recorded')
const fromDate = ref('')
const toDate = ref('')
const preparation = ref<AnswerPreparation>()
const answer = ref<Answer>()
const answerBody = ref<HTMLElement>()
const answerBusy = ref<'prepare' | 'send'>()
const answerError = ref<UiError | null>(null)
const copyBusy = ref(false)
const copyFeedback = ref('')
const copyError = ref<UiError | null>(null)
const confirmedDisagreements = ref<Record<number, Disagreement>>({})
const confirmingDisagreement = ref<number>()
const confirmationError = ref<{ index: number; error: UiError }>()
const invalidRange = computed(() => Boolean(fromDate.value && toDate.value && fromDate.value > toDate.value))

const relations = ref<Disagreement[]>([])
const relationsBusy = ref(false)
const saveBusy = ref(false)
const relationsError = ref<UiError | null>(null)
const relationFeedback = ref('')
const creating = ref(false)
const relationQuestion = ref('')
const relationKind = ref<Relation>('contradiction')
const excerpts = ref(['', ''])
const sideLabels = ['一方依据', '另一方依据']
const editing = ref('')
const reviewStatus = ref<DisagreementStatus>('open')
const reviewJudgment = ref('')
const reviewReason = ref('')
const requiresReason = computed(() => reviewStatus.value === 'resolved' || reviewStatus.value === 'superseded')

watch(() => props.active, (active) => { if (active) void loadDisagreements() }, { immediate: true })
watch(() => [...props.sources], (sources, previous) => {
  sources.forEach((item, index) => {
    if (item !== previous?.[index]) {
      excerpts.value[index] = item?.excerpt ?? ''
    }
    if (item && item !== previous?.[index]) {
      pane.value = 'relations'
      creating.value = true
    }
  })
}, { immediate: true })
watch([outputKind, query, keyword, dateBasis, fromDate, toDate, anchorClaimId], clearOutput)
watch(() => [props.modelReady, props.model?.endpoint, props.model?.model], () => { preparation.value = undefined })

function clearOutput() {
  preparation.value = answer.value = undefined
  answerError.value = copyError.value = null
  copyFeedback.value = ''
  confirmedDisagreements.value = {}
  confirmationError.value = undefined
}

onMounted(() => document.addEventListener('copy', copyFromSelection))
onUnmounted(() => document.removeEventListener('copy', copyFromSelection))

function localDate(date: Date) {
  const month = String(date.getMonth() + 1).padStart(2, '0')
  const day = String(date.getDate()).padStart(2, '0')
  return `${date.getFullYear()}-${month}-${day}`
}

function chooseHalfYear() {
  const today = new Date()
  const start = new Date(today.getFullYear(), today.getMonth() - 6, 1)
  const monthEnd = new Date(start.getFullYear(), start.getMonth() + 1, 0).getDate()
  start.setDate(Math.min(today.getDate(), monthEnd))
  fromDate.value = localDate(start)
  toDate.value = localDate(today)
}

async function prepareAnswer() {
  if (!query.value.trim() || !props.modelReady || invalidRange.value || answerBusy.value) return
  const model = props.model
  answerBusy.value = 'prepare'
  clearOutput()
  try {
    const result = await api.prepareAnswer({
      session: props.session, query: query.value.trim(), keyword: keyword.value.trim(), dateBasis: dateBasis.value,
      fromDate: fromDate.value || undefined, toDate: toDate.value || undefined, outputKind: outputKind.value,
      anchorClaimId: anchorClaimId.value,
    })
    if (props.model === model && props.modelReady) preparation.value = result
  } catch (failure) {
    answerError.value = parseAppError(failure)
  } finally {
    answerBusy.value = undefined
  }
}

async function generateDirect() {
  if (!query.value.trim() || !props.modelReady || invalidRange.value || answerBusy.value) return
  const model = props.model
  answerBusy.value = 'prepare'
  clearOutput()
  try {
    const scope = await api.prepareAnswer({
      session: props.session, query: query.value.trim(), keyword: keyword.value.trim(), dateBasis: dateBasis.value,
      fromDate: fromDate.value || undefined, toDate: toDate.value || undefined, outputKind: outputKind.value,
      anchorClaimId: anchorClaimId.value,
    })
    if (props.model !== model || !props.modelReady) return
    if (!scope.sources.length) {
      preparation.value = scope
      return
    }
    answerBusy.value = 'send'
    answer.value = await api.answerQuery(props.session, scope.id, scope.endpoint)
  } catch (failure) {
    answerError.value = parseAppError(failure)
  } finally {
    answerBusy.value = undefined
  }
}

function askAboutClaim(item: Disagreement, claimId: string) {
  query.value = item.question
  keyword.value = ''
  anchorClaimId.value = claimId
  pane.value = 'answer'
}

async function sendQuestion() {
  const scope = preparation.value
  if (!scope || !scope.sources.length || !props.modelReady || answerBusy.value) return
  answerBusy.value = 'send'
  answerError.value = null
  preparation.value = undefined
  try {
    answer.value = await api.answerQuery(props.session, scope.id, scope.endpoint)
  } catch (failure) {
    answerError.value = parseAppError(failure)
  } finally {
    answerBusy.value = undefined
  }
}

async function cancelPreparation() {
  const current = preparation.value
  if (!current) return
  preparation.value = undefined
  try {
    await api.cancelAnswerPreparation(props.session, current.id)
  } catch (failure) {
    answerError.value = parseAppError(failure)
  }
}

function copyFromSelection(event: ClipboardEvent) {
  const region = answerBody.value
  const selection = window.getSelection()
  if (!props.active || pane.value !== 'answer' || !region || !selection?.rangeCount || selection.isCollapsed
    || !selection.getRangeAt(0).intersectsNode(region)) return
  const text = selection.toString().trim()
  // Allow short single-line term copies; enforce full provenance bundle on claim/multi-line copies.
  const isSubstantiveClaim = text.includes('\n') || /\[\d+\]/.test(text) || text.length >= 40
  if (!isSubstantiveClaim) return
  event.preventDefault()
  void copyAnswer()
}

async function copyAnswer() {
  const current = answer.value
  if (!current || copyBusy.value) return
  copyBusy.value = true
  copyFeedback.value = ''
  copyError.value = null
  try {
    const text = await api.copyAnswer(props.session, current.id)
    if (answer.value === current) {
      await navigator.clipboard.writeText(text)
      copyFeedback.value = `已复制带来源和相关分歧的${outputLabels[current.outputKind]}。`
    }
  } catch (failure) {
    if (answer.value === current) copyError.value = parseAppError(failure)
  } finally {
    copyBusy.value = false
  }
}

async function confirmDisagreement(item: PossibleDisagreement, index: number) {
  const current = answer.value
  if (!current || saveBusy.value || confirmedDisagreements.value[index]) return
  saveBusy.value = true
  confirmingDisagreement.value = index
  confirmationError.value = undefined
  try {
    const saved = await api.createDisagreement(
      props.session,
      item.question,
      item.relation,
      item.members.map(({ source: { path, revision, excerpt } }) => ({ path, revision, excerpt })),
    )
    if (answer.value === current) confirmedDisagreements.value[index] = saved
    await loadDisagreements()
  } catch (failure) {
    if (answer.value === current) confirmationError.value = { index, error: parseAppError(failure) }
  } finally {
    confirmingDisagreement.value = undefined
    saveBusy.value = false
  }
}

async function loadDisagreements() {
  if (relationsBusy.value) return
  relationsBusy.value = true
  relationsError.value = null
  try {
    relations.value = await api.listDisagreements(props.session)
  } catch (failure) {
    relationsError.value = parseAppError(failure)
  } finally {
    relationsBusy.value = false
  }
}

async function createDisagreement() {
  const [left, right] = props.sources
  if (!left || !right || saveBusy.value || relationsBusy.value) return
  saveBusy.value = true
  relationsError.value = null
  relationFeedback.value = ''
  try {
    await api.createDisagreement(
      props.session,
      relationQuestion.value.trim(),
      relationKind.value,
      [left, right].map((source, index) => ({ path: source.path, revision: source.contentHash, excerpt: excerpts.value[index].trim() })),
    )
    creating.value = false
    relationQuestion.value = ''
    excerpts.value = ['', '']
    emit('clearSources')
    relationFeedback.value = '已保存分歧记录。'
    await loadDisagreements()
  } catch (failure) {
    relationsError.value = parseAppError(failure)
  } finally {
    saveBusy.value = false
  }
}

function editDisagreement(item: Disagreement) {
  editing.value = item.id
  reviewStatus.value = item.status
  reviewJudgment.value = item.judgment ?? ''
  reviewReason.value = item.reason ?? ''
}

async function reviewDisagreement() {
  if (!editing.value || saveBusy.value || relationsBusy.value) return
  saveBusy.value = true
  relationsError.value = null
  relationFeedback.value = ''
  try {
    await api.reviewDisagreement(
      props.session,
      editing.value,
      reviewStatus.value,
      reviewJudgment.value.trim(),
      reviewReason.value.trim(),
    )
    editing.value = ''
    relationFeedback.value = '已保存当前判断。'
    await loadDisagreements()
  } catch (failure) {
    relationsError.value = parseAppError(failure)
  } finally {
    saveBusy.value = false
  }
}
</script>

<template>
  <div class="m2-stack">
    <div class="actions m2-switch" role="group" aria-label="问答与分歧视图">
      <button type="button" :aria-pressed="pane === 'answer'" @click="pane = 'answer'">回查问答</button>
      <button type="button" :aria-pressed="pane === 'relations'" @click="pane = 'relations'">分歧记录</button>
    </div>

    <div v-if="pane === 'answer'" class="m2-stack">
      <form class="detail-card m2-form" aria-label="回查问答" @submit.prevent="generateDirect">
        <div class="actions">
          <label class="m2-field">输出类型
            <select v-model="outputKind" :disabled="Boolean(answerBusy)">
              <option v-for="(label, kind) in outputLabels" :key="kind" :value="kind">{{ label }}</option>
            </select>
          </label>
        </div>
        <label class="m2-field">{{ outputKind === 'answer' ? '问题' : '主题或要求' }}<textarea v-model="query" required rows="3" :disabled="Boolean(answerBusy)" /></label>
        <label class="m2-field">检索关键词（可选）<input v-model="keyword" :disabled="Boolean(answerBusy)"></label>
        <div v-if="anchorClaimId" class="actions">
          <p class="hint">本次回查包含所选主张及其关联依据。</p>
          <button type="button" class="quiet" :disabled="Boolean(answerBusy)" @click="anchorClaimId = undefined">取消主张定位</button>
        </div>
        <div class="actions">
          <label class="m2-field">资料日期依据
            <select v-model="dateBasis" :disabled="Boolean(answerBusy)">
              <option value="recorded">记录日期</option>
              <option value="event">事件或发表日期</option>
            </select>
          </label>
          <label class="m2-field">资料开始日期<input v-model="fromDate" type="date" :disabled="Boolean(answerBusy)"></label>
          <label class="m2-field">资料结束日期<input v-model="toDate" type="date" :disabled="Boolean(answerBusy)"></label>
          <button type="button" class="quiet m2-bottom-action" :disabled="Boolean(answerBusy)" @click="chooseHalfYear">近半年</button>
        </div>
        <p v-if="invalidRange" class="notice error" role="alert">资料开始日期不能晚于结束日期。</p>
        <p v-else-if="fromDate || toDate" class="hint">{{ dateBasis === 'recorded' ? '记录日期' : '事件或发表日期' }} {{ fromDate || '不限' }} 至 {{ toDate || '不限' }}。日期未知的材料不会作为初始命中。</p>
        <div v-if="!modelReady" class="actions">
          <p class="hint">整理与问答模型尚未可用。</p>
          <button type="button" @click="emit('showSettings')">设置模型</button>
          <button type="button" class="quiet" @click="emit('showRecords')">打开本地搜索</button>
        </div>
        <div v-else class="actions">
          <button type="submit" class="primary" :disabled="!query.trim() || invalidRange || Boolean(answerBusy)">生成{{ outputLabels[outputKind] }}</button>
          <button type="button" class="quiet" :disabled="!query.trim() || invalidRange || Boolean(answerBusy)" @click="prepareAnswer">仅预览命中范围</button>
          <span v-if="model" class="hint">模型 <code>{{ model.model }}</code></span>
        </div>
        <p v-if="answerBusy" class="hint" role="status">{{ answerBusy === 'prepare' ? '正在查找本地资料…' : `正在请求模型生成${outputLabels[outputKind]}…` }}</p>
        <ErrorNotice :error="answerError" />
      </form>

      <section v-if="preparation" class="m2-stack m2-authorization" aria-labelledby="answer-scope">
        <h2 id="answer-scope">本次发送范围</h2>
        <code>{{ preparation.endpoint }}</code>
        <p>{{ preparation.outputKind === 'answer' ? '问题' : outputLabels[preparation.outputKind] }}　{{ preparation.query }}</p>
        <p v-if="!preparation.sources.length">没有找到可发送的原文片段。请调整检索关键词或资料日期。</p>
        <M2Evidence
          v-for="item in preparation.sources"
          :key="item.id"
          :session="session"
          :evidence="item"
          @select-disagreement-source="emit('selectDisagreementSource', $event)"
        />
        <template v-if="preparation.disagreements.length">
          <h3>相关分歧</h3>
          <M2Relation
            v-for="item in preparation.disagreements"
            :key="item.id"
            :session="session"
            :disagreement="item"
            @select-disagreement-source="emit('selectDisagreementSource', $event)"
          />
        </template>
        <div class="actions">
          <button type="button" class="quiet" @click="cancelPreparation">取消</button>
          <button type="button" class="primary" :disabled="!preparation.sources.length" @click="sendQuestion">
            {{ preparation.outputKind === 'answer' ? '确认发送并回答' : `确认发送并生成${outputLabels[preparation.outputKind]}` }}
          </button>
        </div>
      </section>

      <section v-if="answer" class="detail-card m2-stack" aria-labelledby="answer-result">
        <header class="m2-card-header">
          <h2 id="answer-result">{{ outputLabels[answer.outputKind] }}</h2>
          <button type="button" :aria-disabled="copyBusy || undefined" @click="copyAnswer">复制{{ outputLabels[answer.outputKind] }}及依据</button>
        </header>
        <p class="hint">{{ answer.outputKind === 'answer' ? '问题' : '主题或要求' }}　{{ answer.query }}</p>
        <p v-if="answer.fromDate || answer.toDate" class="hint">{{ answer.dateBasis === 'recorded' ? '记录日期' : '事件或发表日期' }} {{ answer.fromDate || '不限' }} 至 {{ answer.toDate || '不限' }}</p>
        <p v-if="answer.skipped" class="notice" role="status">检索时跳过了 {{ answer.skipped }} 项无法读取或索引的资料。</p>
        <p v-if="answer.sourceNotice" class="notice" role="status">{{ answer.sourceNotice }}</p>
        <pre ref="answerBody" class="m2-pre" tabindex="0">{{ sourceMarkdown(answer.text) }}</pre>
        <p v-if="copyFeedback" class="notice success" role="status">{{ copyFeedback }}</p>
        <ErrorNotice :error="copyError" prefix="复制失败。" />
        <template v-if="answer.disagreements.length">
          <h3>相关分歧</h3>
          <M2Relation
            v-for="item in answer.disagreements"
            :key="item.id"
            :session="session"
            :disagreement="item"
            @select-disagreement-source="emit('selectDisagreementSource', $event)"
          />
        </template>
        <template v-if="answer.possibleDisagreements?.length">
          <h3>模型发现的可能分歧</h3>
          <M2Relation
            v-for="(item, index) in answer.possibleDisagreements"
            :key="index"
            :session="session"
            :disagreement="confirmedDisagreements[index] ?? item"
            @select-disagreement-source="emit('selectDisagreementSource', $event)"
          >
            <p v-if="confirmedDisagreements[index]" class="notice success" role="status">已保存为分歧记录。</p>
            <div v-else class="actions">
              <button type="button" :aria-disabled="saveBusy || undefined" @click="confirmDisagreement(item, index)">
                {{ confirmingDisagreement === index ? '正在保存分歧…' : '确认并保存分歧' }}
              </button>
            </div>
            <ErrorNotice v-if="confirmationError?.index === index" :error="confirmationError.error" prefix="保存分歧失败。" />
          </M2Relation>
        </template>
        <h3>使用的来源</h3>
        <ol class="m2-stack m2-source-list">
          <li v-for="item in answer.sources" :key="item.id">
            <M2Evidence :session="session" :evidence="item" @select-disagreement-source="emit('selectDisagreementSource', $event)" />
          </li>
        </ol>
      </section>
    </div>

    <section v-else class="m2-stack" aria-label="分歧记录">
      <div class="m2-card-header">
        <h2>分歧记录</h2>
        <div class="actions">
          <button type="button" class="quiet" :disabled="relationsBusy || saveBusy" @click="loadDisagreements">刷新分歧</button>
          <button type="button" :aria-expanded="creating" @click="creating = !creating">{{ creating ? '收起录入' : '记录分歧' }}</button>
        </div>
      </div>
      <form v-if="creating" class="detail-card m2-form" aria-label="录入分歧" @submit.prevent="createDisagreement">
        <label class="m2-field">讨论的问题<input v-model="relationQuestion" required :disabled="saveBusy"></label>
        <label class="m2-field">分歧类型
          <select v-model="relationKind" :disabled="saveBusy"><option v-for="(label, id) in relationLabels" :key="id" :value="id">{{ label }}</option></select>
        </label>
        <div class="m2-columns">
          <section v-for="(label, index) in sideLabels" :key="label" class="m2-stack">
            <h3>{{ label }}</h3>
            <template v-if="sources[index]">
              <p>{{ sources[index]?.title }}</p>
              <p class="hint">可从记录详情或展开的原文中划词后一键带入，也可在此直接编辑原文片段。</p>
              <label class="m2-field">{{ label }}的原文片段<textarea v-model="excerpts[index]" required rows="3" :disabled="saveBusy" /></label>
              <div class="actions"><button type="button" class="quiet" :disabled="saveBusy" @click="emit('removeSource', index)">移除{{ label }}</button></div>
            </template>
            <div v-else class="actions"><button type="button" :disabled="saveBusy" @click="emit('showRecords')">前往记录选择依据</button></div>
          </section>
        </div>
        <div class="actions"><button type="submit" class="primary" :disabled="!sources.every(Boolean) || saveBusy || relationsBusy">保存分歧</button></div>
      </form>
      <ErrorNotice :error="relationsError" />
      <p v-if="relationFeedback" class="notice success" role="status">{{ relationFeedback }}</p>
      <p v-if="relationsBusy || saveBusy" class="hint" role="status">{{ saveBusy ? '正在保存分歧…' : '正在读取分歧…' }}</p>
      <p v-else-if="!relations.length && !relationsError" class="empty-panel compact">还没有记录分歧。</p>
      <M2Relation v-for="item in relations" :key="item.id" :session="session" :disagreement="item">
        <template #member="{ member }">
          <div v-if="member.claimId" class="actions">
            <button type="button" class="quiet" :disabled="Boolean(answerBusy)" @click="askAboutClaim(item, member.claimId)">以此主张回查</button>
          </div>
        </template>
        <form v-if="editing === item.id" class="m2-form" aria-label="更新当前判断" @submit.prevent="reviewDisagreement">
          <label class="m2-field">分歧状态
            <select v-model="reviewStatus" :disabled="saveBusy"><option v-for="(label, id) in disagreementStatuses" :key="id" :value="id">{{ label }}</option></select>
          </label>
          <label class="m2-field">当前判断<textarea v-model="reviewJudgment" rows="2" :required="reviewStatus === 'decided'" :disabled="saveBusy" /></label>
          <label class="m2-field">依据与适用范围<textarea v-model="reviewReason" rows="2" :required="requiresReason" :disabled="saveBusy" /></label>
          <div class="actions">
            <button type="button" class="quiet" :disabled="saveBusy" @click="editing = ''">取消</button>
            <button type="submit" :disabled="saveBusy || relationsBusy">保存当前判断</button>
          </div>
        </form>
        <div v-else class="actions"><button type="button" class="quiet" :disabled="saveBusy" @click="editDisagreement(item)">更新当前判断</button></div>
      </M2Relation>
    </section>
  </div>
</template>
