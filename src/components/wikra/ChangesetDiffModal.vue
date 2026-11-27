<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watch } from 'vue'
import { api } from '../../api/client'
import { parseAppError, type UiError } from '../../api/errors'
import { computeLineDiff, type DiffRow } from '../../lineDiff'
import type { ChangesetItem, ChangesetPageDiff } from '../../m2'
import UiIcon from '../../UiIcon.vue'
import ErrorNotice from '../ErrorNotice.vue'
import { pendingReasonLabel } from '../../utils/wikra'

const props = defineProps<{
  open: boolean
  session: string
  changesetId: string
  pagePath: string
  changesetSummary?: string
}>()

const emit = defineEmits<{
  close: []
  openWiki: [path: string]
  externalStatus: [payload: { changesetId: string; path: string; externallyModified: boolean }]
  decided: [payload: { decision: 'apply' | 'discard'; changeset: ChangesetItem; pageTitle: string }]
}>()

const loading = ref(false)
const rechecking = ref(false)
const deciding = ref<'apply' | 'discard' | null>(null)
const error = ref<UiError | null>(null)
const page = ref<ChangesetPageDiff | null>(null)
const versionConflict = ref(false)
const showFullLines = ref(true)

const isDirtyBlocked = computed(() => Boolean(page.value?.externallyModified || versionConflict.value))

const leftSource = computed(() => {
  if (!page.value) return ''
  return page.value.current ?? page.value.before ?? ''
})
const rightSource = computed(() => page.value?.after ?? '')

const diff = computed(() => computeLineDiff(leftSource.value, rightSource.value, 4))
const hasHiddenContext = computed(() => diff.value.compactRows.some((r) => r.kind === 'hunk'))

const activeRows = computed<DiffRow[]>(() =>
  showFullLines.value || !hasHiddenContext.value ? diff.value.rows : diff.value.compactRows,
)

function buildProtectLineSet(text: string): Set<number> {
  const protectedLines = new Set<number>()
  if (!text) return protectedLines
  const lines = text.replaceAll('\r\n', '\n').split('\n')
  let inProtect = false
  for (let i = 0; i < lines.length; i += 1) {
    const line = lines[i]
    if (/<!--\s*(?:wikra|fmd)-protect:start\s*-->/i.test(line)) {
      inProtect = true
      protectedLines.add(i + 1)
      continue
    }
    if (inProtect) {
      protectedLines.add(i + 1)
      if (/<!--\s*(?:wikra|fmd)-protect:end\s*-->/i.test(line)) {
        inProtect = false
      }
    }
  }
  return protectedLines
}

const leftProtectLines = computed(() => buildProtectLineSet(leftSource.value))
const rightProtectLines = computed(() => buildProtectLineSet(rightSource.value))

const leftRows = computed(() =>
  activeRows.value.filter((row) => row.kind !== 'add'),
)
const rightRows = computed(() =>
  activeRows.value.filter((row) => row.kind !== 'del'),
)

const displayReasons = computed(() => {
  const raw = page.value?.pendingReasons || []
  const seen = new Set<string>()
  const out: string[] = []
  for (const r of raw) {
    const label = pendingReasonLabel(r)
    if (!seen.has(label)) {
      seen.add(label)
      out.push(label)
    }
  }
  return out
})

async function loadDetail(isRecheck = false) {
  if (!props.session || !props.changesetId) return
  if (isRecheck) rechecking.value = true
  else loading.value = true
  error.value = null
  try {
    const detail = await api.getChangesetDetail(props.session, props.changesetId)
    const matched =
      detail.pages.find((p) => p.path === props.pagePath)
      || detail.pages.find((p) => p.hasPending)
      || detail.pages[0]
      || null
    page.value = matched
    if (matched) {
      versionConflict.value = matched.externallyModified
      emit('externalStatus', {
        changesetId: props.changesetId,
        path: matched.path,
        externallyModified: matched.externallyModified,
      })
      if (!isRecheck) {
        const lineCount = Math.max(
          (matched.current || '').split('\n').length,
          (matched.after || '').split('\n').length,
        )
        showFullLines.value = lineCount <= 120
      }
    }
  } catch (err) {
    error.value = parseAppError(err)
  } finally {
    loading.value = false
    rechecking.value = false
  }
}

async function decide(decision: 'apply' | 'discard') {
  const current = page.value
  if (!props.session || !props.changesetId || !current || deciding.value) return
  if (decision === 'apply' && isDirtyBlocked.value) return
  deciding.value = decision
  error.value = null
  try {
    const updated = await api.decideReview(props.session, props.changesetId, current.path, decision)
    emit('externalStatus', {
      changesetId: props.changesetId,
      path: current.path,
      externallyModified: false,
    })
    emit('decided', { decision, changeset: updated, pageTitle: current.title })
    emit('close')
  } catch (err) {
    const parsed = parseAppError(err)
    if (parsed.code === 'VERSION_CONFLICT') {
      versionConflict.value = true
      emit('externalStatus', {
        changesetId: props.changesetId,
        path: current.path,
        externallyModified: true,
      })
      // Refresh disk snapshot so the left pane reflects the latest external edits.
      await loadDetail(true)
      versionConflict.value = true
    } else {
      error.value = parsed
    }
  } finally {
    deciding.value = null
  }
}

function handleOpenManualMerge() {
  const path = page.value?.path || props.pagePath
  if (!path) return
  emit('openWiki', path)
  emit('close')
}

watch(
  () => [props.open, props.changesetId, props.pagePath] as const,
  ([isOpen]) => {
    if (isOpen) {
      page.value = null
      versionConflict.value = false
      error.value = null
      void loadDetail(false)
    }
  },
  { immediate: true },
)

function onKeydown(e: KeyboardEvent) {
  if (props.open && e.key === 'Escape') {
    e.preventDefault()
    emit('close')
  }
}

onMounted(() => window.addEventListener('keydown', onKeydown))
onUnmounted(() => window.removeEventListener('keydown', onKeydown))
</script>

<template>
  <div
    v-if="open"
    class="overlay-layer"
    role="dialog"
    aria-modal="true"
    aria-label="待审改动差异确认"
    @click.self="emit('close')"
  >
    <div class="diff-modal">
      <div class="hud-head">
        <div class="row-flex" style="min-width:0;">
          <strong style="font-size:14px;">
            待审改动确认 · {{ page?.title || pagePath }}
          </strong>
          <span v-if="changesetSummary" class="hint" style="overflow:hidden;text-overflow:ellipsis;white-space:nowrap;">
            {{ changesetSummary }}
          </span>
        </div>
        <button type="button" class="icon-btn" aria-label="关闭差异确认面板" @click="emit('close')">&times;</button>
      </div>

      <div class="diff-modal-body">
        <div v-if="loading && !page" class="hint" style="padding:16px 4px;">
          正在读取当前文件现状并计算差异…
        </div>

        <ErrorNotice v-if="error" :error="error" />

        <template v-if="page">
          <div class="row-between">
            <div class="row-flex" style="min-width:0;">
              <span class="mono hint" style="word-break:break-all;">{{ page.path }}</span>
              <span v-for="r in displayReasons" :key="r" class="pill warning">{{ r }}</span>
            </div>
            <div class="row-flex">
              <span class="pill success">+{{ diff.added }} 行</span>
              <span class="pill error">-{{ diff.removed }} 行</span>
              <button
                v-if="hasHiddenContext"
                type="button"
                class="btn quiet xs"
                @click="showFullLines = !showFullLines"
              >{{ showFullLines ? '仅看变更上下文' : '展开全部行' }}</button>
            </div>
          </div>

          <div v-if="page.hasProtectedSection" class="cs-gentle-note">
            <UiIcon name="shield" size="sm" />
            <span>你的「我的理解」手动保护区不受本次改动影响</span>
          </div>

          <div v-if="isDirtyBlocked" class="version-conflict-bar" role="alert">
            <div style="flex:1 1 260px;min-width:0;display:flex;flex-direction:column;gap:4px;">
              <strong style="color:var(--error);">
                页面刚刚有新变动，为防覆盖你的编辑，本次未写入
              </strong>
              <span style="font-size:12px;color:var(--text-secondary);line-height:1.55;">
                该文件已被外部程序（如 Obsidian）或其他整理更新。请点击「重新检查文件状态」核对磁盘现状；仅当外部修改已还原回原基线时才会重新解锁「采纳写入」，否则请「打开页面手动合并」或「丢弃此页改动」。
              </span>
            </div>
            <div class="row-flex" style="flex:none;">
              <button
                type="button"
                class="btn sm"
                :disabled="rechecking || Boolean(deciding)"
                @click="loadDetail(true)"
              >
                <UiIcon name="refresh" size="sm" />
                {{ rechecking ? '正在检查…' : '重新检查文件状态' }}
              </button>
              <button
                type="button"
                class="btn sm"
                :disabled="Boolean(deciding)"
                @click="handleOpenManualMerge"
              >打开页面手动合并</button>
              <button
                type="button"
                class="btn danger-outline sm"
                :disabled="Boolean(deciding)"
                @click="decide('discard')"
              >{{ deciding === 'discard' ? '正在丢弃…' : '丢弃此页改动' }}</button>
            </div>
          </div>

          <div class="diff-pane-grid">
            <section class="diff-doc-col" aria-label="当前页面内容（磁盘现状）">
              <div class="diff-doc-head">
                <span>当前页面内容（磁盘现状）</span>
                <span v-if="isDirtyBlocked" class="pill error">磁盘已有外部修改</span>
                <span v-else-if="!leftSource" class="pill">尚未创建</span>
              </div>
              <div v-if="!leftSource" class="hint" style="padding:14px 12px;">
                （磁盘上尚未创建该文件，采纳后将新建此页）
              </div>
              <div v-else class="diff-code">
                <template v-for="(row, idx) in leftRows" :key="`l-${idx}`">
                  <div v-if="row.kind === 'hunk'" class="diff-line diff-hunk-row">
                    <span class="diff-ln" aria-hidden="true">…</span>
                    <span>省略 {{ row.skipped }} 行未变更内容</span>
                  </div>
                  <div
                    v-else
                    class="diff-line"
                    :class="{
                      del: row.kind === 'del',
                      protect: row.kind === 'context' && row.oldLine !== null && leftProtectLines.has(row.oldLine),
                    }"
                  >
                    <span class="diff-ln" aria-hidden="true">{{ row.oldLine ?? '' }}</span>
                    <span>{{ row.text || ' ' }}</span>
                  </div>
                </template>
              </div>
            </section>

            <section class="diff-doc-col" aria-label="采纳后的页面效果">
              <div class="diff-doc-head">
                <span>采纳后的页面效果</span>
                <span class="pill accent">待确认方案</span>
              </div>
              <div class="diff-code">
                <template v-for="(row, idx) in rightRows" :key="`r-${idx}`">
                  <div v-if="row.kind === 'hunk'" class="diff-line diff-hunk-row">
                    <span class="diff-ln" aria-hidden="true">…</span>
                    <span>省略 {{ row.skipped }} 行未变更内容</span>
                  </div>
                  <div
                    v-else
                    class="diff-line"
                    :class="{
                      add: row.kind === 'add',
                      protect: row.kind === 'context' && row.newLine !== null && rightProtectLines.has(row.newLine),
                    }"
                  >
                    <span class="diff-ln" aria-hidden="true">{{ row.newLine ?? '' }}</span>
                    <span>{{ row.text || ' ' }}</span>
                  </div>
                </template>
              </div>
            </section>
          </div>
        </template>
      </div>

      <div class="hud-foot">
        <span class="hint">采纳后会生成关联子整理记录，随时可一键撤销。</span>
        <div class="row-flex">
          <button
            type="button"
            class="btn sm"
            :disabled="loading || Boolean(deciding) || !page"
            @click="decide('discard')"
          >{{ deciding === 'discard' ? '正在丢弃…' : '丢弃此页改动' }}</button>
          <button
            v-if="isDirtyBlocked"
            type="button"
            class="btn sm"
            :disabled="Boolean(deciding)"
            @click="handleOpenManualMerge"
          >打开页面手动合并</button>
          <button
            type="button"
            class="btn primary sm"
            :disabled="loading || rechecking || Boolean(deciding) || !page || isDirtyBlocked"
            @click="decide('apply')"
          >{{ deciding === 'apply' ? '正在写入…' : '采纳写入' }}</button>
        </div>
      </div>
    </div>
  </div>
</template>
