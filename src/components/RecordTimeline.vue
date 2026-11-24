<script setup lang="ts">
import { computed } from 'vue'
import type { UiError } from '../api/errors'
import ErrorNotice from './ErrorNotice.vue'
import UiIcon from '../UiIcon.vue'
import { recordTitle, useSourcePresentation, type IndexFeedback, type RecordEntry } from '../m2'

const props = defineProps<{
  hasWorkspace: boolean
  opening: boolean
  modelReady: boolean
  searchActive: boolean
  invalidDateRange: boolean
  fromDate: string
  toDate: string
  recentRecords: RecordEntry[]
  recordsLoading: boolean
  recordsError: UiError | null
  searchPage?: { records: RecordEntry[]; hasMore: boolean; skipped: number }
  searchLoading: boolean
  searchError?: { error: UiError; append: boolean }
  indexFeedback?: IndexFeedback
  selectedPath?: string
}>()

const emit = defineEmits<{
  'update:fromDate': [value: string]
  'update:toDate': [value: string]
  clearSearch: []
  rebuildIndex: []
  openSettings: []
  openCapture: []
  retryRecords: []
  retrySearch: [append: boolean]
  loadMore: []
  select: [path: string, focusDetail: boolean]
}>()

const { showSourcePath, sourceMarkdown } = useSourcePresentation()
const dateLabel = new Intl.DateTimeFormat('zh-CN', { dateStyle: 'full' })
const timeLabel = new Intl.DateTimeFormat('zh-CN', { timeStyle: 'short', hour12: false })

const displayedRecords = computed(() =>
  props.searchActive ? props.searchPage?.records ?? [] : props.recentRecords,
)

const groupedRecords = computed(() => {
  const days = new Map<string, RecordEntry[]>()
  for (const record of displayedRecords.value) {
    const date = record.capturedAt ? dateLabel.format(new Date(record.capturedAt)) : '记录日期未知'
    const day = days.get(date) ?? []
    day.push(record)
    days.set(date, day)
  }
  return days
})

function recordExcerpt(text: string) {
  if (props.searchActive) return text
  const line = text.trimStart()
  const start = line.indexOf('\n')
  return start < 0 ? '' : line.slice(start + 1).trim()
}
</script>

<template>
  <section id="timeline" class="timeline" :class="{ 'search-results': searchActive }" aria-labelledby="timeline-heading">
    <header class="section-bar">
      <h1 id="timeline-heading">{{ searchActive ? '搜索结果' : '最近记录' }}</h1>
      <span v-if="searchActive && searchPage && !searchLoading" class="count" role="status">
        {{ searchPage.hasMore ? '已显示 ' : '' }}{{ displayedRecords.length }} 条
      </span>
      <span v-else-if="hasWorkspace && !searchActive && !recordsLoading && !recordsError" class="count">
        {{ recentRecords.length }} 条
      </span>
    </header>
    <div v-if="hasWorkspace" class="record-controls">
      <div class="actions">
        <div class="actions date-filters" role="group" aria-label="记录日期筛选">
          <span>记录日期</span>
          <label>
            <span class="visually-hidden">开始日期</span>
            <input
              :value="fromDate"
              type="date"
              :disabled="opening"
              :aria-invalid="invalidDateRange || undefined"
              @input="emit('update:fromDate', ($event.target as HTMLInputElement).value)"
            >
          </label>
          <span aria-hidden="true">至</span>
          <label>
            <span class="visually-hidden">结束日期</span>
            <input
              :value="toDate"
              type="date"
              :disabled="opening"
              :aria-invalid="invalidDateRange || undefined"
              @input="emit('update:toDate', ($event.target as HTMLInputElement).value)"
            >
          </label>
        </div>
        <button v-if="searchActive" type="button" class="quiet" @click="emit('clearSearch')">清除搜索与筛选</button>
        <button
          type="button"
          class="quiet"
          :disabled="opening"
          :aria-disabled="indexFeedback?.state === 'building' || undefined"
          @click="emit('rebuildIndex')"
        >
          {{ indexFeedback?.state === 'failed' ? '重试重建索引' : '重建索引' }}
        </button>
      </div>
      <p v-if="searchActive" class="hint">
        本地 Markdown 搜索。<template v-if="!modelReady">问答未启用。</template><template v-if="fromDate || toDate">日期筛选只包含记录日期已知的文件。</template>
      </p>
      <p v-if="indexFeedback?.state === 'building'" class="notice" role="status">正在重建索引…</p>
      <p v-else-if="indexFeedback?.state === 'ready'" class="notice success" role="status">
        索引已重建，收录 {{ indexFeedback.indexed }} 份 Markdown。<template v-if="indexFeedback.skipped">跳过 {{ indexFeedback.skipped }} 项。</template>
      </p>
      <ErrorNotice v-else-if="indexFeedback?.state === 'failed'" :error="indexFeedback.error" prefix="索引重建失败。" />
      <p v-if="searchActive && searchPage?.skipped && indexFeedback?.state !== 'ready'" class="notice" role="status">
        建立索引时跳过了 {{ searchPage.skipped }} 项。可检查工作区文件后重建索引。
      </p>
    </div>

    <p v-if="opening && !hasWorkspace" class="empty-panel" role="status">正在打开工作区…</p>
    <div v-else-if="!hasWorkspace" class="empty-panel">
      <UiIcon name="folder" />
      <h2>打开本地工作区</h2>
      <p>选择 Markdown 笔记的保存位置。</p>
      <button type="button" class="primary" @click="emit('openSettings')">选择工作区</button>
    </div>
    <p v-else-if="invalidDateRange" class="notice error" role="alert">开始日期不能晚于结束日期。</p>
    <template v-else-if="searchActive">
      <div v-if="searchError" class="empty-panel">
        <ErrorNotice :error="searchError.error" prefix="搜索失败。" />
        <button type="button" @click="emit('retrySearch', searchError.append)">重试搜索</button>
      </div>
      <p v-else-if="searchLoading && !searchPage" class="empty-panel" role="status">正在搜索工作区…</p>
      <div v-else-if="searchPage && !displayedRecords.length" class="empty-panel">
        <UiIcon name="search" />
        <h2>没有匹配的记录</h2>
        <p>调整关键词或记录日期后再试。</p>
      </div>
    </template>
    <div v-else-if="recordsError" class="empty-panel">
      <ErrorNotice :error="recordsError" />
      <button type="button" @click="emit('retryRecords')">重试读取</button>
    </div>
    <p v-else-if="recordsLoading && !recentRecords.length" class="empty-panel" role="status">正在读取最近记录…</p>
    <div v-else-if="!recentRecords.length" class="empty-panel">
      <UiIcon name="history" />
      <h2>还没有随手记</h2>
      <button type="button" class="primary" @click="emit('openCapture')">写第一条笔记</button>
    </div>

    <section v-for="[date, day] in groupedRecords" :key="date" class="timeline-day" :aria-label="date">
      <h2><span class="timeline-dot" />{{ date }}</h2>
      <ul>
        <li v-for="record in day" :key="record.path">
          <button
            type="button"
            class="record-card"
            :aria-pressed="selectedPath === record.path"
            @click="emit('select', record.path, $event.detail === 0)"
          >
            <span class="record-heading">
              <UiIcon name="file" /><strong>{{ recordTitle(record) }}</strong>
              <time v-if="record.capturedAt" :datetime="record.capturedAt">{{ timeLabel.format(new Date(record.capturedAt)) }}</time>
            </span>
            <span v-if="recordExcerpt(record.text)" class="record-excerpt">{{ sourceMarkdown(recordExcerpt(record.text)) }}</span>
            <code v-if="showSourcePath(record.path, record.isCapture)" class="record-path">{{ record.path }}</code>
          </button>
        </li>
      </ul>
    </section>

    <button
      v-if="searchActive && searchPage?.hasMore && !searchError"
      class="load-more"
      type="button"
      :aria-disabled="searchLoading || undefined"
      @click="emit('loadMore')"
    >
      {{ searchLoading ? '正在加载…' : '加载更多' }}
    </button>
  </section>
</template>
