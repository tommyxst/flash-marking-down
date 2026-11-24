<script setup lang="ts">
import { computed, nextTick, ref, watch } from 'vue'
import { api } from './api/client'
import { parseAppError, type UiError } from './api/errors'
import ErrorNotice from './components/ErrorNotice.vue'
import { useSourcePresentation, type Evidence, type RecordSelection, type RecordSource } from './m2'

const props = defineProps<{ session: string; evidence: Evidence }>()
const emit = defineEmits<{ selectDisagreementSource: [source: RecordSelection] }>()
const { showSourcePath, sourceMarkdown } = useSourcePresentation()
const opened = ref<RecordSource>()
const openedPre = ref<HTMLElement>()
const loading = ref(false)
const error = ref<UiError | null>(null)
const marker = ref<HTMLElement>()
let reading = 0

function pickForDisagreement() {
  const container = openedPre.value
  const selection = window.getSelection()
  let excerpt = props.evidence.excerpt
  if (container && selection?.rangeCount && !selection.isCollapsed && selection.getRangeAt(0).intersectsNode(container)) {
    const selected = selection.toString().trim()
    if (selected) excerpt = selected
  }
  const revision = opened.value ? opened.value.contentHash : props.evidence.revision
  const isCapture = opened.value ? opened.value.isCapture : props.evidence.isCapture
  const title = showSourcePath(props.evidence.path, isCapture) ? props.evidence.path : '随手记'
  emit('selectDisagreementSource', { path: props.evidence.path, title, contentHash: revision, isCapture, excerpt })
}

watch(() => props.evidence, () => {
  reading += 1
  opened.value = undefined
  loading.value = false
  error.value = null
})

const locator = computed(() => {
  const { startLine, endLine } = props.evidence
  return startLine === endLine ? `第 ${startLine} 行` : `第 ${startLine}–${endLine} 行`
})

const located = computed(() => {
  const source = opened.value
  if (source?.state !== 'current' || props.evidence.changed || !props.evidence.excerpt) return
  const body = sourceMarkdown(source.body).replaceAll('\r\n', '\n')
  const { startLine, endLine } = props.evidence
  const excerpt = sourceMarkdown(props.evidence.excerpt)
  const lines = body.split('\n')
  const before = lines.slice(0, startLine - 1).join('\n')
  const offset = startLine > 1 ? before.length + 1 : 0
  const marked = lines.slice(startLine - 1, endLine).join('\n')
  if (marked.includes(excerpt)) return [body.slice(0, offset), marked, body.slice(offset + marked.length)]
  const unique = body.indexOf(excerpt)
  if (unique >= 0 && body.indexOf(excerpt, unique + 1) < 0) {
    return [body.slice(0, unique), excerpt, body.slice(unique + excerpt.length)]
  }
})

async function toggleSource() {
  if (loading.value) return
  if (opened.value) {
    opened.value = undefined
    return
  }
  const request = ++reading
  const evidence = props.evidence
  loading.value = true
  error.value = null
  try {
    const result = await api.openRecordSource(props.session, evidence.path, evidence.revision)
    if (request === reading) {
      opened.value = result
      void nextTick(() => marker.value?.scrollIntoView({ block: 'nearest' }))
    }
  } catch (failure) {
    if (request === reading) error.value = parseAppError(failure)
  } finally {
    if (request === reading) loading.value = false
  }
}
</script>

<template>
  <div class="m2-evidence">
    <div class="actions">
      <code v-if="showSourcePath(evidence.path, evidence.isCapture) && (!evidence.movedFrom || showSourcePath(evidence.movedFrom))">{{ evidence.path }}</code>
      <span v-else>随手记</span>
      <span v-if="evidence.available" class="hint">{{ locator }}</span>
      <span v-if="evidence.date" class="hint">记录于 <time :datetime="evidence.date">{{ evidence.date }}</time></span>
      <span v-if="evidence.eventDate" class="hint">事件或发表日期 <time :datetime="evidence.eventDate">{{ evidence.eventDate }}</time></span>
      <span v-if="!evidence.date && !evidence.eventDate" class="hint">资料日期未知</span>
    </div>
    <p v-if="evidence.movedFrom" class="hint">
      <template v-if="showSourcePath(evidence.movedFrom) && showSourcePath(evidence.path, evidence.isCapture)">原路径 <code>{{ evidence.movedFrom }}</code>，已定位到上方的新路径。</template>
      <template v-else>原文已移动，已找到当前位置。</template>
    </p>
    <p v-if="evidence.outsideRange" class="badge badge-warn">此依据在所选时间范围外，与已记录的分歧相关</p>
    <p v-if="!evidence.available" class="notice error">原文缺失或不可读取。</p>
    <template v-else>
      <p v-if="evidence.changed" class="badge badge-warn">原文已有新版本，以下为引用时的片段</p>
      <blockquote v-if="evidence.excerpt">{{ sourceMarkdown(evidence.excerpt) }}</blockquote>
      <div class="actions">
        <button type="button" class="quiet" :aria-disabled="loading || undefined" @click="toggleSource">
          {{ opened ? '收起原文' : loading ? '正在读取原文…' : error ? '重试读取原文' : '打开引用原文' }}
        </button>
        <button v-if="opened" type="button" class="quiet" title="可先在下方原文中划选片段再点击" @click="pickForDisagreement">设为分歧依据</button>
      </div>
      <ErrorNotice :error="error" />
      <p v-if="opened && (opened.state === 'changed' || evidence.changed)" class="notice" role="status">原文已变化，当前版本无法沿用旧片段的位置。</p>
      <p v-else-if="opened && !located" class="notice" role="status">无法定位引用片段，已显示原文。</p>
      <pre v-if="opened && located" ref="openedPre" class="m2-pre" tabindex="0">{{ located[0] }}<mark ref="marker">{{ located[1] }}</mark>{{ located[2] }}</pre>
      <pre v-else-if="opened" ref="openedPre" class="m2-pre" tabindex="0">{{ sourceMarkdown(opened.body) }}</pre>
    </template>
  </div>
</template>
