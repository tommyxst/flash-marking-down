<script setup lang="ts">
import { onMounted, onUnmounted, ref, watch } from 'vue'
import { api } from './api/client'
import { parseAppError, type UiError } from './api/errors'
import ErrorNotice from './components/ErrorNotice.vue'
import M2Documents from './M2Documents.vue'
import M2Relation from './M2Relation.vue'
import UiIcon from './UiIcon.vue'
import type {
  ConflictItem,
  DocumentMapping,
  DrawerCounts,
  DrawerTab,
  WikiSuggestion,
} from './m2'

const props = defineProps<{
  open: boolean
  tab: DrawerTab
  session: string
  counts: DrawerCounts
  wikiTargets: DocumentMapping[]
  workspaceFiles: string[]
  desktop: boolean
  wikiReviewId?: string
  prefillConflict?: { topic: string; incomingPath: string; incomingText: string } | null
}>()

const emit = defineEmits<{
  'update:open': [open: boolean]
  'update:tab': [tab: DrawerTab]
  openNote: [path: string]
  changed: []
  targetsChanged: []
  clearPrefill: []
}>()

const conflicts = ref<ConflictItem[]>([])
const suggestions = ref<WikiSuggestion[]>([])
const loading = ref(false)
const error = ref<UiError | null>(null)

async function refresh() {
  if (!props.session || !props.open) return
  loading.value = true
  error.value = null
  try {
    if (props.tab === 'conflicts') {
      conflicts.value = await api.listConflicts(props.session)
    } else {
      suggestions.value = await api.listWikiSuggestions(props.session)
    }
  } catch (err) {
    error.value = parseAppError(err)
  } finally {
    loading.value = false
  }
}

watch(() => [props.open, props.tab, props.session] as const, () => {
  if (props.open) void refresh()
}, { immediate: true })

function handleKey(event: KeyboardEvent) {
  if (props.open && event.key === 'Escape') {
    event.preventDefault()
    emit('update:open', false)
  }
}

onMounted(() => window.addEventListener('keydown', handleKey))
onUnmounted(() => window.removeEventListener('keydown', handleKey))

function onUpdated() {
  void refresh()
  emit('changed')
}

function openNote(path: string) {
  emit('openNote', path)
  emit('update:open', false)
}
</script>

<template>
  <div v-if="open" class="drawer-backdrop" @click.self="emit('update:open', false)">
    <aside class="aux-drawer" aria-label="冲突与 Wiki 变更抽屉">
      <header class="drawer-header">
        <div class="drawer-tabs" role="tablist">
          <button
            type="button"
            role="tab"
            :aria-selected="tab === 'conflicts'"
            :class="{ active: tab === 'conflicts' }"
            @click="emit('update:tab', 'conflicts')"
          >
            <UiIcon name="conflict" />
            <span>观点冲突 ({{ counts.openConflicts }})</span>
          </button>
          <button
            type="button"
            role="tab"
            :aria-selected="tab === 'wiki'"
            :class="{ active: tab === 'wiki' }"
            @click="emit('update:tab', 'wiki')"
          >
            <UiIcon name="file" />
            <span>Wiki 变更 ({{ counts.pendingWiki }})</span>
          </button>
        </div>
        <button type="button" class="quiet compact-btn" aria-label="关闭抽屉" @click="emit('update:open', false)">
          <UiIcon name="close" />
        </button>
      </header>

      <div class="pane-scroll drawer-body">
        <ErrorNotice :error="error" />
        <p v-if="loading" class="hint" role="status">正在刷新…</p>
        <M2Relation
          v-if="tab === 'conflicts'"
          :session="session"
          :conflicts="conflicts"
          :wiki-targets="wikiTargets"
          :prefill="prefillConflict"
          @updated="onUpdated"
          @open-note="openNote"
          @clear-prefill="emit('clearPrefill')"
        />
        <M2Documents
          v-else
          :session="session"
          :suggestions="suggestions"
          :wiki-targets="wikiTargets"
          :workspace-files="workspaceFiles"
          :desktop="desktop"
          :wiki-review-id="wikiReviewId"
          @updated="onUpdated"
          @targets-changed="emit('targetsChanged')"
          @open-note="openNote"
        />
      </div>
    </aside>
  </div>
</template>
