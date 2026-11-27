<script setup lang="ts">
import { computed } from 'vue'
import type { UiError } from '../../api/errors'
import ErrorNotice from '../ErrorNotice.vue'
import MarkdownPreview from '../../MarkdownPreview.vue'
import UiIcon from '../../UiIcon.vue'
import type { DocumentDetail, DocWidthMode } from '../../m2'
import { shortDate } from '../../utils/wikra'

const props = defineProps<{
  session: string
  doc: DocumentDetail | null
  loading: boolean
  error: UiError | null
  docWidth: DocWidthMode
  wikiTitleMap: Record<string, string>
  organizing: boolean
}>()

const emit = defineEmits<{
  toggleWidth: []
  openExternalUrl: [url: string]
  openWiki: [path: string]
  reorganize: [path: string]
}>()

const isFullWidth = computed(() => props.docWidth === 'full')

function wikiTitle(path: string): string {
  return props.wikiTitleMap[path] || path.split('/').pop()?.replace(/\.md$/i, '') || path
}
</script>

<template>
  <div class="stage-body flush">
    <div v-if="loading && !doc" class="empty-stage">
      <p>正在读取原始文件…</p>
    </div>

    <div v-else-if="error && !doc" class="empty-stage">
      <ErrorNotice :error="error" />
    </div>

    <div v-else-if="!doc" class="empty-stage">
      <UiIcon name="file" size="lg" />
      <p>请选择一份原始文件进行阅读</p>
    </div>

    <template v-else>
      <div class="doc-toolbar">
        <div class="breadcrumb" :title="doc.path">{{ doc.path }}</div>
        <div style="display:flex;align-items:center;gap:6px;flex:none;">
          <button
            v-if="doc.sourceUrl"
            type="button"
            class="btn sm"
            @click="emit('openExternalUrl', doc.sourceUrl)"
          >
            <UiIcon name="external" size="sm" /> 打开原网页 ↗
          </button>
          <button
            type="button"
            class="btn quiet sm"
            :class="{ 'active-toggle': isFullWidth }"
            title="切换适读宽度或铺满整栏"
            @click="emit('toggleWidth')"
          >
            <UiIcon name="width" size="sm" />
            <span>{{ isFullWidth ? '适读宽' : '铺满宽' }}</span>
          </button>
          <button
            type="button"
            class="btn sm"
            :disabled="organizing"
            @click="emit('reorganize', doc.path)"
          >{{ organizing ? '正在加入整理…' : '重新整理' }}</button>
        </div>
      </div>

      <div class="doc-scroll">
        <article class="doc-paper">
          <div class="doc-title-row">
            <h1>{{ doc.title }}</h1>
            <span class="mono" style="font-size:11.5px;color:var(--muted);">
              {{ shortDate(doc.updatedAt) }}
            </span>
          </div>

          <div class="raw-origin-banner">
            <div style="display:flex;flex-wrap:wrap;align-items:center;gap:8px;">
              <span class="pill accent">{{ doc.typeLabel || '原始原件' }} · 只读原件</span>
              <span v-if="doc.linkedWikiPaths.length > 0" style="color:var(--text-secondary);">
                已整理进知识页：
                <template v-for="(wp, i) in doc.linkedWikiPaths" :key="wp">
                  <span v-if="i > 0"> · </span>
                  <button type="button" class="wikilink" @click="emit('openWiki', wp)">
                    [[{{ wikiTitle(wp) }}]]
                  </button>
                </template>
              </span>
              <span v-else style="color:var(--muted);">尚未整理进知识页</span>
            </div>
            <span style="font-size:11.5px;color:var(--muted);">原件只读保存，AI 整理绝不会改动原文</span>
          </div>

          <div class="prose">
            <MarkdownPreview
              :source="doc.body"
              :session="session"
              :base-path="doc.path"
              hide-frontmatter
              @wikilink="emit('openWiki', $event)"
            />
          </div>
        </article>
      </div>
    </template>
  </div>
</template>
