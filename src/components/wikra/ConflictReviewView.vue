<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import type { UiError } from '../../api/errors'
import ErrorNotice from '../ErrorNotice.vue'
import { conflictStatusLabels, type ConflictItem, type ConflictStatus, type LibraryDocSummary } from '../../m2'

const props = defineProps<{
  conflict: ConflictItem | null
  wikiPages: LibraryDocSummary[]
  resolving: boolean
  error: UiError | null
}>()

const emit = defineEmits<{
  openWiki: [path: string]
  resolve: [payload: { id: string; status: ConflictStatus; resolutionNote?: string; wikiTarget?: string }]
}>()

const conditionText = ref('')
const conditionInvalid = ref(false)
const conditionInput = ref<HTMLInputElement>()
const pageOverrides = ref<Record<string, 'follow' | 'incoming' | 'skip'>>({})

interface AffectedPage {
  path: string
  title: string
  opLabel: string
}

const affectedPages = computed<AffectedPage[]>(() => {
  const c = props.conflict
  if (!c) return []
  const seen = new Set<string>()
  const list: AffectedPage[] = []
  const push = (path: string | null | undefined, opLabel: string) => {
    if (!path || seen.has(path)) return
    seen.add(path)
    const found = props.wikiPages.find((p) => p.path === path)
    list.push({
      path,
      title: found?.title || path.split('/').pop()?.replace(/\.md$/i, '') || path,
      opLabel,
    })
  }
  push(c.currentPath, '主要关联知识页（当前结论来源）')
  push(c.incomingPath, '关联新来源文档')
  for (const p of props.wikiPages) {
    if (p.pendingConflictCount > 0 && !seen.has(p.path)) {
      push(p.path, '存在关联待确认冲突')
    }
  }
  return list
})

watch(
  () => props.conflict?.id,
  () => {
    conditionText.value = props.conflict?.resolutionNote || ''
    conditionInvalid.value = false
    const next: Record<string, 'follow' | 'incoming' | 'skip'> = {}
    for (const ap of affectedPages.value) {
      next[ap.path] = 'follow'
    }
    pageOverrides.value = next
  },
  { immediate: true },
)

function handleResolve(status: Exclude<ConflictStatus, 'open'>) {
  const c = props.conflict
  if (!c || props.resolving) return
  const trimmed = conditionText.value.trim()
  if (status === 'keep_both' && !trimmed) {
    conditionInvalid.value = true
    conditionInput.value?.focus()
    return
  }
  conditionInvalid.value = false

  const customNotes = affectedPages.value
    .filter((ap) => pageOverrides.value[ap.path] && pageOverrides.value[ap.path] !== 'follow')
    .map((ap) => `${ap.title}:${pageOverrides.value[ap.path] === 'skip' ? '先不改这篇' : '直接采用新说法'}`)
  const noteParts = [trimmed, customNotes.length ? `单页策略：${customNotes.join('；')}` : ''].filter(Boolean)

  const wikiPathSet = new Set(props.wikiPages.map((p) => p.path))
  const targetPage = affectedPages.value.find(
    (ap) => pageOverrides.value[ap.path] !== 'skip' && wikiPathSet.has(ap.path),
  )?.path
  emit('resolve', {
    id: c.id,
    status,
    resolutionNote: noteParts.join(' · ') || undefined,
    wikiTarget: targetPage || undefined,
  })
}
</script>

<template>
  <div class="stage-body">
    <div v-if="!conflict" class="empty-stage">
      <p>未找到该条冲突记录，可能已经处理完成。</p>
    </div>

    <div v-else class="review-wrap">
      <div style="display:flex;flex-wrap:wrap;align-items:flex-start;justify-content:space-between;gap:12px;">
        <div>
          <div style="display:flex;align-items:center;gap:8px;margin-bottom:6px;">
            <span class="pill" :class="conflict.status === 'open' ? 'warning' : 'success'">
              {{ conflict.status === 'open' ? '新旧观点对比 · 待确认' : `已完成确认 · ${conflictStatusLabels[conflict.status]}` }}
            </span>
          </div>
          <h1 style="font-size:21px;font-weight:700;">{{ conflict.topic }}</h1>
          <p style="font-size:13px;color:var(--text-secondary);margin-top:4px;">
            请对比原有记录与新来源的说法，决定知识库采用哪一种结论。
          </p>
        </div>
        <button
          v-if="conflict.currentPath"
          type="button"
          class="btn sm"
          @click="emit('openWiki', conflict.currentPath)"
        >返回对应知识页</button>
      </div>

      <ErrorNotice v-if="error" :error="error" />

      <div class="diff-split">
        <section class="diff-col current">
          <div class="diff-col-head">
            <span class="kicker" style="color:var(--error);">旧笔记怎么说</span>
            <span class="mono" style="font-size:11px;color:var(--muted);">
              {{ conflict.currentLabel || conflict.currentPath || '现有知识库' }}
            </span>
          </div>
          <div class="diff-quote">{{ conflict.currentText }}</div>
        </section>

        <section class="diff-col incoming">
          <div class="diff-col-head">
            <span class="kicker" style="color:var(--success);">新来源怎么说</span>
            <span class="mono" style="font-size:11px;color:var(--muted);">
              {{ conflict.incomingLabel || conflict.incomingPath || '新收集记录' }}
            </span>
          </div>
          <div class="diff-quote">{{ conflict.incomingText }}</div>
        </section>
      </div>

      <section v-if="affectedPages.length > 0">
        <div class="kicker" style="margin-bottom:8px;">
          涉及的 {{ affectedPages.length }} 篇文档（可单独跳过某篇，无需一刀切）
        </div>
        <div class="affected-pages">
          <div v-for="ap in affectedPages" :key="ap.path" class="affected-card">
            <button
              type="button"
              class="wikilink"
              style="text-align:left;font-weight:600;"
              @click="emit('openWiki', ap.path)"
            >{{ ap.title }}</button>
            <span style="font-size:12px;color:var(--muted);">{{ ap.opLabel }}</span>
            <select
              v-model="pageOverrides[ap.path]"
              :disabled="conflict.status !== 'open' || resolving"
              :aria-label="`${ap.title} 处理方式`"
            >
              <option value="follow">按下方选择同步更新</option>
              <option value="incoming">这篇直接采用新说法</option>
              <option value="skip">先不改这篇</option>
            </select>
          </div>
        </div>
      </section>

      <section class="decision-bar">
        <div class="kicker">确认采用哪一种结论（随时可在「整理」页撤销）</div>
        <template v-if="conflict.status === 'open'">
          <input
            ref="conditionInput"
            v-model="conditionText"
            type="text"
            class="condition-input"
            :class="{ invalid: conditionInvalid }"
            placeholder="选择「两个都保留」时，请写明各自适用情况（必填）…"
            aria-label="适用条件说明"
            @input="conditionInvalid = false"
          >
          <p v-if="conditionInvalid" style="font-size:12px;color:var(--error);">
            选择「两个都保留」时，必须写明两种结论各自的适用范围或边界条件。
          </p>
          <div style="display:flex;flex-wrap:wrap;gap:8px;">
            <button
              type="button"
              class="btn primary"
              :disabled="resolving"
              @click="handleResolve('keep_both')"
            >两个都保留并写明适用范围（推荐）</button>
            <button
              type="button"
              class="btn"
              :disabled="resolving"
              @click="handleResolve('accept_incoming')"
            >改用新结论（旧结论归档保留）</button>
            <button
              type="button"
              class="btn"
              :disabled="resolving"
              @click="handleResolve('keep_current')"
            >维持旧结论</button>
          </div>
        </template>
        <p v-else style="font-size:13px;color:var(--success);">
          已确认：{{ conflictStatusLabels[conflict.status] }}
          <template v-if="conflict.resolutionNote">（{{ conflict.resolutionNote }}）</template>。
        </p>
      </section>
    </div>
  </div>
</template>
