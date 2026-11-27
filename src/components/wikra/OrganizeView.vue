<script setup lang="ts">
import { computed } from 'vue'
import type { UiError } from '../../api/errors'
import ErrorNotice from '../ErrorNotice.vue'
import UiIcon from '../../UiIcon.vue'
import { conflictStatusLabels, type OrganizeOverview } from '../../m2'
import { formatCountdown } from '../../utils/wikra'

const props = defineProps<{
  overview: OrganizeOverview | null
  loading: boolean
  error: UiError | null
  bufferSecondsLeft: number
  revertingId: string
}>()

const emit = defineEmits<{
  goLibrary: []
  openCapture: []
  openConflict: [id: string]
  openWiki: [path: string]
  startTeachback: [pagePath: string]
  dismissSuggestion: [id: string]
  openBufferQueue: []
  flushBuffer: []
  revertChangeset: [id: string]
}>()

const openConflicts = computed(() =>
  (props.overview?.conflicts || []).filter((c) => c.status === 'open'),
)
const lastResolvedConflict = computed(() =>
  (props.overview?.conflicts || []).find((c) => c.status !== 'open') || null,
)
const suggestions = computed(() => props.overview?.suggestions || [])
const changesets = computed(() => props.overview?.changesets || [])
const bufferCount = computed(() => props.overview?.bufferQueue.length || 0)

function outcomeLabel(outcome: string): string {
  if (outcome === 'applied') return '已更新'
  if (outcome === 'pending_review') return '待确认'
  if (outcome === 'reverted') return '已撤销'
  return outcome
}
</script>

<template>
  <div class="stage-body">
    <div class="organize-wrap">
      <section class="organize-header">
        <div>
          <h1>整理与同步</h1>
          <p>平时后台会自动把你的随手笔记和收藏文章整理进知识库；只有出现观点矛盾时，才会在这里请你确认。</p>
        </div>
        <div style="display:flex;gap:8px;">
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
        <div class="org-col">
          <section class="ops-card" :class="{ priority: openConflicts.length > 0 }">
            <div class="ops-card-head">
              <div class="ops-card-title">
                <UiIcon name="conflict" />
                <span>需要你确认的冲突</span>
                <span class="pill" :class="openConflicts.length > 0 ? 'warning' : 'success'">
                  {{ openConflicts.length > 0 ? `${openConflicts.length} 处待确认` : '已全部确认' }}
                </span>
              </div>
            </div>

            <div v-if="openConflicts.length > 0" class="ops-list">
              <div v-for="c in openConflicts" :key="c.id" class="ops-row">
                <div class="ops-row-main">
                  <span class="ops-row-title">{{ c.topic }}</span>
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
            </div>

            <div v-else class="ops-row">
              <div class="ops-row-main">
                <span class="ops-row-title">当前没有观点冲突，知识库保持一致</span>
                <span class="ops-row-desc">
                  <template v-if="lastResolvedConflict">
                    上次确认：《{{ lastResolvedConflict.topic }}》· {{ conflictStatusLabels[lastResolvedConflict.status] }}
                    <template v-if="lastResolvedConflict.resolutionNote">（{{ lastResolvedConflict.resolutionNote }}）</template>。
                  </template>
                  <template v-else>
                    后台整理仅在发现明显观点分歧时提醒你处理。
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

          <section class="ops-card">
            <div class="ops-card-head">
              <div class="ops-card-title">
                <UiIcon name="sparkles" />
                <span>可选的复习与关联建议</span>
              </div>
              <span style="font-size:11.5px;color:var(--muted);">有空再看，不影响正常记录</span>
            </div>
            <div class="ops-list">
              <div v-for="s in suggestions" :key="s.id" class="ops-row">
                <div class="ops-row-main">
                  <div style="display:flex;align-items:center;gap:6px;">
                    <span class="pill" :class="{ accent: s.kind === 'review' }">{{ s.kindLabel }}</span>
                    <span class="ops-row-title">{{ s.title }}</span>
                  </div>
                  <span class="ops-row-desc">{{ s.detail }}</span>
                </div>
                <div class="ops-row-actions">
                  <button
                    v-if="s.actionLabel.includes('讲一遍')"
                    type="button"
                    class="btn primary sm"
                    @click="emit('startTeachback', s.pagePath)"
                  >{{ s.actionLabel }}</button>
                  <button
                    v-else
                    type="button"
                    class="btn sm"
                    @click="emit('openWiki', s.pagePath)"
                  >{{ s.actionLabel || '打开查看' }}</button>
                  <button
                    type="button"
                    class="btn quiet sm"
                    @click="emit('dismissSuggestion', s.id)"
                  >忽略</button>
                </div>
              </div>
              <p v-if="!suggestions.length" style="font-size:12.5px;color:var(--muted);">
                当前没有新的建议。
              </p>
            </div>
          </section>
        </div>

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
            <div style="display:flex;gap:6px;flex:none;">
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
              <span style="font-size:11.5px;color:var(--muted);">每一批都支持一键撤销</span>
            </div>

            <div class="ops-list">
              <div v-for="cs in changesets" :key="cs.id" class="cs-card">
                <div style="display:flex;justify-content:space-between;align-items:center;gap:8px;">
                  <strong style="font-size:13px;">{{ cs.summary }}</strong>
                  <span
                    class="pill"
                    :class="cs.status === 'applied' ? 'success' : cs.status === 'partial' ? 'warning' : ''"
                  >
                    {{ cs.status === 'applied' ? '已完成' : cs.status === 'partial' ? '含待确认项' : '已撤销' }}
                  </span>
                </div>
                <div style="font-size:11.5px;color:var(--muted);">
                  {{ cs.time }} · {{ cs.trigger }}
                  <template v-if="cs.modelAlias"> · {{ cs.modelAlias }}</template>
                </div>
                <div v-if="cs.ops.length" class="editop-list">
                  <div v-for="(op, idx) in cs.ops" :key="`${cs.id}-${idx}`">
                    {{ op.label }} →
                    <button
                      v-if="op.targetPath"
                      type="button"
                      class="wikilink"
                      @click="emit('openWiki', op.targetPath)"
                    >{{ op.targetTitle || op.targetPath }}</button>
                    <span v-else>{{ op.targetTitle }}</span>
                    （{{ outcomeLabel(op.outcome) }}）
                  </div>
                </div>
                <div style="display:flex;gap:6px;">
                  <button
                    v-if="cs.status !== 'reverted'"
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
  </div>
</template>
