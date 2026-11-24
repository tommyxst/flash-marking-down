<script setup lang="ts">
import { computed, ref } from 'vue'
import { computeLineDiff } from './lineDiff'
import { useSourcePresentation } from './m2'

const props = defineProps<{
  before: string
  after: string
  showFullComparison?: boolean
}>()

const { sourceMarkdown } = useSourcePresentation()
const expandAll = ref(false)

const beforeShown = computed(() => sourceMarkdown(props.before))
const afterShown = computed(() => sourceMarkdown(props.after))
const diff = computed(() => computeLineDiff(beforeShown.value, afterShown.value, 3))
const visibleRows = computed(() => expandAll.value ? diff.value.rows : diff.value.compactRows)
const hasHiddenContext = computed(() => diff.value.compactRows.some((row) => row.kind === 'hunk'))
</script>

<template>
  <div class="m2-stack">
    <div class="m2-card-header">
      <div class="actions">
        <span class="badge badge-success">+{{ diff.added }} 行</span>
        <span class="badge badge-error">-{{ diff.removed }} 行</span>
      </div>
      <button v-if="hasHiddenContext" type="button" class="quiet" @click="expandAll = !expandAll">
        {{ expandAll ? '仅看变更上下文' : '展开全部行' }}
      </button>
    </div>

    <div class="diff-view" role="region" aria-label="逐行文件差异" tabindex="0">
      <template v-if="visibleRows.length">
        <template v-for="(row, index) in visibleRows" :key="index">
          <div v-if="row.kind === 'hunk'" class="diff-hunk">
            … 省略 {{ row.skipped }} 行未变更内容 …
          </div>
          <div v-else class="diff-line" :class="`diff-${row.kind}`">
            <span class="diff-ln" aria-hidden="true">{{ row.oldLine ?? '' }}</span>
            <span class="diff-ln" aria-hidden="true">{{ row.newLine ?? '' }}</span>
            <span class="diff-sign" aria-hidden="true">{{ row.kind === 'add' ? '+' : row.kind === 'del' ? '-' : ' ' }}</span>
            <span class="diff-text">{{ row.text || ' ' }}</span>
          </div>
        </template>
      </template>
      <p v-else class="hint diff-empty">内容无变化</p>
    </div>

    <details v-if="showFullComparison !== false">
      <summary>左右对照完整文件</summary>
      <div class="m2-columns">
        <pre class="m2-pre m2-before" aria-label="修改前完整文件" tabindex="0">{{ beforeShown || '（空文件）' }}</pre>
        <pre class="m2-pre m2-after" aria-label="修改后完整文件" tabindex="0">{{ afterShown || '（空文件）' }}</pre>
      </div>
    </details>
  </div>
</template>
