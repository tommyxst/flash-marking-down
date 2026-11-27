<script setup lang="ts">
import { computed } from 'vue'
import type { InboxItemSummary } from '../../m2'

const props = defineProps<{
  items: InboxItemSummary[]
  loading: boolean
  filter: 'pending' | 'archived' | 'all'
  activeTabId: string
}>()

const emit = defineEmits<{
  'update:filter': [filter: 'pending' | 'archived' | 'all']
  openInbox: [id: string]
}>()

const filtered = computed(() =>
  props.items.filter((item) => {
    if (props.filter === 'pending') return item.state === 'unread' || item.state === 'reading'
    if (props.filter === 'archived') return item.state === 'compiled' || item.state === 'archived'
    return true
  }),
)

function statusPill(item: InboxItemSummary): { label: string; cls: string } {
  if (item.fetchStatus === 'stub') return { label: '待补全', cls: 'warning' }
  if (item.state === 'compiled') return { label: '已提炼', cls: 'success' }
  if (item.state === 'archived') return { label: '已存档', cls: '' }
  if (item.state === 'reading') return { label: '阅读中', cls: 'accent' }
  return { label: '未读', cls: '' }
}
</script>

<template>
  <aside class="left-pane" aria-label="收件箱文章列表">
    <div class="pane-header">
      <div class="segmented" role="tablist">
        <button
          type="button"
          :class="{ active: filter === 'pending' }"
          @click="emit('update:filter', 'pending')"
        >未读</button>
        <button
          type="button"
          :class="{ active: filter === 'archived' }"
          @click="emit('update:filter', 'archived')"
        >已处理</button>
        <button
          type="button"
          :class="{ active: filter === 'all' }"
          @click="emit('update:filter', 'all')"
        >全部</button>
      </div>
    </div>

    <div class="pane-scroll">
      <div v-if="loading && !items.length" class="hint" style="padding: 12px 6px;">
        正在读取收件箱…
      </div>
      <button
        v-for="item in filtered"
        :key="item.id"
        type="button"
        class="list-item"
        :class="{ active: activeTabId === `inbox:${item.id}` }"
        @click="emit('openInbox', item.id)"
      >
        <div class="list-item-row">
          <span class="list-item-title">{{ item.title }}</span>
        </div>
        <div class="list-item-row">
          <span class="list-item-sub">{{ item.domain || item.sourceType }} · {{ item.savedAt }}</span>
          <span class="pill" :class="statusPill(item).cls">{{ statusPill(item).label }}</span>
        </div>
      </button>
      <p v-if="!loading && !filtered.length" class="hint" style="padding: 12px 6px;">
        当前分组暂无文章
      </p>
    </div>
  </aside>
</template>
