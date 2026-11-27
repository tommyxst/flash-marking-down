<script setup lang="ts">
import { computed, nextTick, ref, watch } from 'vue'
import type { UiError } from '../../api/errors'
import ErrorNotice from '../ErrorNotice.vue'
import UiIcon from '../../UiIcon.vue'
import type { LibraryTree } from '../../m2'
import { shortDate } from '../../utils/wikra'

type SortMode = 'name' | 'updated'
type SectionKind = 'wiki' | 'raw'

interface TreeFileNode {
  kind: 'file'
  section: SectionKind
  tabId: string
  path: string
  displayRelPath: string
  title: string
  timestamp: number
  updatedAt: string | null
  conflictCount: number
  rawKind?: 'note' | 'clip'
  searchText: string
  tooltip: string
}

interface TreeFolderNode {
  kind: 'folder'
  section: SectionKind
  key: string
  name: string
  relDir: string
  fullPath: string
  depth: number
  folders: Map<string, TreeFolderNode>
  files: TreeFileNode[]
}

interface FilteredFolder {
  key: string
  section: SectionKind
  name: string
  relDir: string
  fullPath: string
  depth: number
  folders: FilteredFolder[]
  files: TreeFileNode[]
  fileCount: number
  conflictCount: number
  latestTimestamp: number
}

type FlatTreeRow =
  | {
      kind: 'folder'
      key: string
      section: SectionKind
      name: string
      fullPath: string
      depth: number
      expanded: boolean
      fileCount: number
      conflictCount: number
    }
  | {
      kind: 'file'
      key: string
      section: SectionKind
      path: string
      title: string
      depth: number
      conflictCount: number
      rawKind?: 'note' | 'clip'
      tooltip: string
    }

interface PersistedTreeState {
  version: 1
  sectionOpen: { wiki: boolean; raw: boolean }
  expandedFolders: string[]
  sortBy: SortMode
}

const TREE_STATE_PREFIX = 'wikra-tree-state-v2:'
const SMALL_SECTION_THRESHOLD = 15
const zhCollator = new Intl.Collator('zh-CN', { numeric: true, sensitivity: 'base' })

const props = defineProps<{
  tree: LibraryTree
  vaultRoot: string
  rawDir: string
  loading: boolean
  filter: 'all' | 'wiki' | 'raw'
  activeTabId: string
  creating: boolean
  error: UiError | null
}>()

const emit = defineEmits<{
  'update:filter': [filter: 'all' | 'wiki' | 'raw']
  openWiki: [path: string]
  openRaw: [path: string]
  createPage: [nameOrPath: string]
}>()

const scrollEl = ref<HTMLElement>()
const filterInputEl = ref<HTMLInputElement>()
const createOpen = ref(false)
const createInput = ref('')
const createField = ref<HTMLInputElement>()

const searchQuery = ref('')
const sortBy = ref<SortMode>('name')
const sectionOpen = ref<{ wiki: boolean; raw: boolean }>({ wiki: true, raw: true })
const expandedFolders = ref<Set<string>>(new Set())
const filterCollapsed = ref<Set<string>>(new Set())

const vaultInitialized = ref('')
const hasPersistedState = ref(false)

function normalizeRelPath(input: string): string {
  return input
    .trim()
    .replaceAll('\\', '/')
    .replace(/^\.\/+/, '')
    .replace(/^\/+/, '')
    .replace(/\/{2,}/g, '/')
    .replace(/\/+$/, '')
}

const effectiveRawDir = computed(() => normalizeRelPath(props.tree.rawDir || props.rawDir || 'raw') || 'raw')

function stripSectionPrefix(section: SectionKind, rawPath: string, rawDirName: string): string {
  const norm = normalizeRelPath(rawPath)
  if (!norm) return ''
  if (section === 'wiki') {
    if (/^wiki\//i.test(norm)) return norm.slice(5)
    return norm
  }
  const prefix = `${rawDirName}/`
  if (rawDirName && norm.toLowerCase().startsWith(prefix.toLowerCase())) {
    return norm.slice(prefix.length)
  }
  return norm
}

function ancestorFolderKeys(section: SectionKind, filePath: string, rawDirName: string): string[] {
  const stripped = stripSectionPrefix(section, filePath, rawDirName)
  if (!stripped) return []
  const segments = stripped.split('/').filter(Boolean)
  if (segments.length <= 1) return []
  const keys: string[] = []
  let acc = ''
  for (let i = 0; i < segments.length - 1; i++) {
    acc = acc ? `${acc}/${segments[i]}` : segments[i]
    keys.push(`${section}:${acc}`)
  }
  return keys
}

function parseTimestamp(value: string | null | undefined): number {
  if (!value) return 0
  const parsed = Date.parse(value)
  return Number.isNaN(parsed) ? 0 : parsed
}

function createRootFolder(section: SectionKind): TreeFolderNode {
  return {
    kind: 'folder',
    section,
    key: `${section}:__root__`,
    name: '',
    relDir: '',
    fullPath: '',
    depth: -1,
    folders: new Map(),
    files: [],
  }
}

function insertFileIntoTree(root: TreeFolderNode, file: TreeFileNode, strippedPrefix = '') {
  const segments = file.displayRelPath.split('/').filter(Boolean)
  if (segments.length <= 1) {
    root.files.push(file)
    return
  }
  let cur = root
  let acc = ''
  for (let i = 0; i < segments.length - 1; i++) {
    const seg = segments[i]
    acc = acc ? `${acc}/${seg}` : seg
    let child = cur.folders.get(seg)
    if (!child) {
      child = {
        kind: 'folder',
        section: root.section,
        key: `${root.section}:${acc}`,
        name: seg,
        relDir: acc,
        fullPath: strippedPrefix ? `${strippedPrefix}/${acc}` : acc,
        depth: i,
        folders: new Map(),
        files: [],
      }
      cur.folders.set(seg, child)
    }
    cur = child
  }
  cur.files.push(file)
}

const wikiRootTree = computed<TreeFolderNode>(() => {
  const root = createRootFolder('wiki')
  for (const p of props.tree.wikiPages) {
    const normPath = normalizeRelPath(p.path)
    const stripped = stripSectionPrefix('wiki', normPath, effectiveRawDir.value)
    const strippedPrefix = stripped !== normPath ? 'wiki' : ''
    const fallbackName = stripped.split('/').pop()?.replace(/\.md$/i, '') || normPath
    const title = p.title?.trim() || fallbackName
    const dateLabel = shortDate(p.updatedAt)
    insertFileIntoTree(root, {
      kind: 'file',
      section: 'wiki',
      tabId: `page:${p.path}`,
      path: p.path,
      displayRelPath: stripped || normPath,
      title,
      timestamp: parseTimestamp(p.updatedAt),
      updatedAt: p.updatedAt,
      conflictCount: p.pendingConflictCount || 0,
      searchText: `${title}\n${stripped}\n${normPath}\n${(p.tags || []).join(' ')}`.toLowerCase(),
      tooltip: `${normPath}${p.updatedAt ? ` · 更新于 ${dateLabel}` : ''}`,
    }, strippedPrefix)
  }
  return root
})

const rawRootTree = computed<TreeFolderNode>(() => {
  const root = createRootFolder('raw')
  const rawDirName = effectiveRawDir.value
  for (const r of props.tree.rawFiles) {
    const normPath = normalizeRelPath(r.path)
    const stripped = stripSectionPrefix('raw', normPath, rawDirName)
    const strippedPrefix = stripped !== normPath ? rawDirName : ''
    const fallbackName = stripped.split('/').pop()?.replace(/\.md$/i, '') || normPath
    const title = r.title?.trim() || fallbackName
    const dateLabel = shortDate(r.createdAt)
    insertFileIntoTree(root, {
      kind: 'file',
      section: 'raw',
      tabId: `raw:${r.path}`,
      path: r.path,
      displayRelPath: stripped || normPath,
      title,
      timestamp: parseTimestamp(r.createdAt),
      updatedAt: r.createdAt,
      conflictCount: 0,
      rawKind: r.kind,
      searchText: `${title}\n${stripped}\n${normPath}\n${r.kindLabel || ''}`.toLowerCase(),
      tooltip: `${normPath}${r.createdAt ? ` · 记录于 ${dateLabel}` : ''}`,
    }, strippedPrefix)
  }
  return root
})

const filterTokens = computed(() =>
  searchQuery.value
    .trim()
    .toLowerCase()
    .split(/\s+/)
    .filter(Boolean),
)
const isFiltering = computed(() => filterTokens.value.length > 0)

function matchesTokens(haystack: string, tokens: string[]): boolean {
  for (const t of tokens) {
    if (!haystack.includes(t)) return false
  }
  return true
}

function buildFilteredFolder(
  folder: TreeFolderNode,
  tokens: string[],
  parentMatched: boolean,
  mode: SortMode,
): FilteredFolder {
  const folderSelfMatched =
    tokens.length === 0
    || parentMatched
    || (folder.relDir.length > 0 && matchesTokens(folder.relDir.toLowerCase(), tokens))

  const keptFolders: FilteredFolder[] = []
  for (const child of folder.folders.values()) {
    const built = buildFilteredFolder(child, tokens, folderSelfMatched, mode)
    if (built.fileCount > 0) {
      keptFolders.push(built)
    }
  }

  const keptFiles =
    tokens.length === 0 || folderSelfMatched
      ? [...folder.files]
      : folder.files.filter((f) => matchesTokens(f.searchText, tokens))

  keptFolders.sort((a, b) => {
    if (mode === 'updated') {
      const diff = b.latestTimestamp - a.latestTimestamp
      if (diff !== 0) return diff
    }
    return zhCollator.compare(a.name, b.name)
  })

  keptFiles.sort((a, b) => {
    if (mode === 'updated') {
      const diff = b.timestamp - a.timestamp
      if (diff !== 0) return diff
    }
    return zhCollator.compare(a.title, b.title)
  })

  let fileCount = keptFiles.length
  let conflictCount = 0
  let latestTimestamp = 0

  for (const f of keptFiles) {
    conflictCount += f.conflictCount
    if (f.timestamp > latestTimestamp) latestTimestamp = f.timestamp
  }
  for (const sub of keptFolders) {
    fileCount += sub.fileCount
    conflictCount += sub.conflictCount
    if (sub.latestTimestamp > latestTimestamp) latestTimestamp = sub.latestTimestamp
  }

  return {
    key: folder.key,
    section: folder.section,
    name: folder.name,
    relDir: folder.relDir,
    fullPath: folder.fullPath,
    depth: folder.depth,
    folders: keptFolders,
    files: keptFiles,
    fileCount,
    conflictCount,
    latestTimestamp,
  }
}

const filteredWikiTree = computed(() =>
  buildFilteredFolder(wikiRootTree.value, filterTokens.value, false, sortBy.value),
)

const filteredRawTree = computed(() =>
  buildFilteredFolder(rawRootTree.value, filterTokens.value, false, sortBy.value),
)

const showWikiSection = computed(() => props.filter === 'all' || props.filter === 'wiki')
const showRawSection = computed(() => props.filter === 'all' || props.filter === 'raw')

const totalFilteredMatches = computed(() => {
  let count = 0
  if (showWikiSection.value) count += filteredWikiTree.value.fileCount
  if (showRawSection.value) count += filteredRawTree.value.fileCount
  return count
})

function collectFolderKeys(node: FilteredFolder, out: string[]) {
  for (const sub of node.folders) {
    out.push(sub.key)
    collectFolderKeys(sub, out)
  }
}

const visibleFolderKeys = computed(() => {
  const keys: string[] = []
  if (showWikiSection.value) collectFolderKeys(filteredWikiTree.value, keys)
  if (showRawSection.value) collectFolderKeys(filteredRawTree.value, keys)
  return keys
})

const isAllCollapsed = computed(() => {
  const keys = visibleFolderKeys.value
  if (keys.length > 0) {
    if (isFiltering.value) {
      return keys.every((k) => filterCollapsed.value.has(k))
    }
    return keys.every((k) => !expandedFolders.value.has(k))
  }
  const sections: SectionKind[] = []
  if (showWikiSection.value) sections.push('wiki')
  if (showRawSection.value) sections.push('raw')
  return sections.length > 0 && sections.every((s) => !sectionOpen.value[s])
})

function flattenFilteredTree(node: FilteredFolder, depthOffset: number, out: FlatTreeRow[]) {
  for (const sub of node.folders) {
    const expanded = isFiltering.value
      ? !filterCollapsed.value.has(sub.key)
      : expandedFolders.value.has(sub.key)
    out.push({
      kind: 'folder',
      key: sub.key,
      section: sub.section,
      name: sub.name,
      fullPath: sub.fullPath || sub.relDir,
      depth: depthOffset,
      expanded,
      fileCount: sub.fileCount,
      conflictCount: sub.conflictCount,
    })
    if (expanded) {
      flattenFilteredTree(sub, depthOffset + 1, out)
    }
  }
  for (const f of node.files) {
    out.push({
      kind: 'file',
      key: f.tabId,
      section: f.section,
      path: f.path,
      title: f.title,
      depth: depthOffset,
      conflictCount: f.conflictCount,
      rawKind: f.rawKind,
      tooltip: f.tooltip,
    })
  }
}

const wikiRows = computed<FlatTreeRow[]>(() => {
  if (!showWikiSection.value) return []
  if (!isFiltering.value && !sectionOpen.value.wiki) return []
  const rows: FlatTreeRow[] = []
  flattenFilteredTree(filteredWikiTree.value, 0, rows)
  return rows
})

const rawRows = computed<FlatTreeRow[]>(() => {
  if (!showRawSection.value) return []
  if (!isFiltering.value && !sectionOpen.value.raw) return []
  const rows: FlatTreeRow[] = []
  flattenFilteredTree(filteredRawTree.value, 0, rows)
  return rows
})

// ============================================================================
// Persistence, Smart Default Collapse & Active Document Auto-Reveal
// ============================================================================
function storageKeyForVault(vaultRoot: string): string {
  return `${TREE_STATE_PREFIX}${vaultRoot}`
}

function saveStateToStorage() {
  if (!props.vaultRoot || isFiltering.value) return
  if (vaultInitialized.value !== props.vaultRoot) return
  try {
    const payload: PersistedTreeState = {
      version: 1,
      sectionOpen: { ...sectionOpen.value },
      expandedFolders: Array.from(expandedFolders.value),
      sortBy: sortBy.value,
    }
    window.localStorage.setItem(storageKeyForVault(props.vaultRoot), JSON.stringify(payload))
  } catch {
    // Ignore storage quota or sandbox restrictions.
  }
}

function loadVaultState(vaultRoot: string) {
  filterCollapsed.value = new Set()
  searchQuery.value = ''
  if (!vaultRoot) {
    hasPersistedState.value = false
    vaultInitialized.value = ''
    sectionOpen.value = { wiki: true, raw: true }
    expandedFolders.value = new Set()
    sortBy.value = 'name'
    return
  }
  try {
    const raw = window.localStorage.getItem(storageKeyForVault(vaultRoot))
    if (raw) {
      const parsed = JSON.parse(raw) as Partial<PersistedTreeState>
      if (parsed && parsed.version === 1) {
        sectionOpen.value = {
          wiki: parsed.sectionOpen?.wiki ?? true,
          raw: parsed.sectionOpen?.raw ?? true,
        }
        expandedFolders.value = new Set(Array.isArray(parsed.expandedFolders) ? parsed.expandedFolders : [])
        sortBy.value = parsed.sortBy === 'updated' ? 'updated' : 'name'
        hasPersistedState.value = true
        vaultInitialized.value = vaultRoot
        return
      }
    }
  } catch {
    // Fallback to smart defaults.
  }
  hasPersistedState.value = false
  vaultInitialized.value = ''
  sectionOpen.value = { wiki: true, raw: true }
  expandedFolders.value = new Set()
  sortBy.value = 'name'
}

function maybeApplySmartDefaults() {
  if (!props.vaultRoot) return
  if (hasPersistedState.value || vaultInitialized.value === props.vaultRoot) return
  if (props.tree.wikiPages.length === 0 && props.tree.rawFiles.length === 0) return

  const nextExpanded = new Set<string>()
  if (props.tree.wikiPages.length <= SMALL_SECTION_THRESHOLD) {
    for (const topFolder of wikiRootTree.value.folders.values()) {
      nextExpanded.add(topFolder.key)
    }
  }
  if (props.tree.rawFiles.length <= SMALL_SECTION_THRESHOLD) {
    for (const topFolder of rawRootTree.value.folders.values()) {
      nextExpanded.add(topFolder.key)
    }
  }

  sectionOpen.value = { wiki: true, raw: true }
  expandedFolders.value = nextExpanded
  vaultInitialized.value = props.vaultRoot
  revealActiveTab(props.activeTabId, false)
  saveStateToStorage()
}

function revealActiveTab(tabId: string, scrollIntoView: boolean) {
  if (!tabId) return
  let section: SectionKind | null = null
  let docPath = ''
  if (tabId.startsWith('page:')) {
    section = 'wiki'
    docPath = tabId.slice(5)
  } else if (tabId.startsWith('raw:')) {
    section = 'raw'
    docPath = tabId.slice(4)
  } else {
    return
  }

  const ancestors = ancestorFolderKeys(section, docPath, effectiveRawDir.value)
  let changed = false
  if (!sectionOpen.value[section]) {
    sectionOpen.value = { ...sectionOpen.value, [section]: true }
    changed = true
  }
  if (ancestors.length > 0) {
    const next = new Set(expandedFolders.value)
    for (const k of ancestors) {
      if (!next.has(k)) {
        next.add(k)
        changed = true
      }
    }
    if (changed) {
      expandedFolders.value = next
    }
  }
  if (changed && !isFiltering.value && vaultInitialized.value === props.vaultRoot) {
    saveStateToStorage()
  }

  if (scrollIntoView) {
    void nextTick(() => {
      const el = scrollEl.value?.querySelector<HTMLElement>('.tree-row.active')
      el?.scrollIntoView({ block: 'nearest' })
    })
  }
}

watch(
  () => props.vaultRoot,
  (root) => {
    loadVaultState(root)
    maybeApplySmartDefaults()
    revealActiveTab(props.activeTabId, false)
  },
  { immediate: true },
)

watch(
  () => [props.tree, props.loading] as const,
  () => {
    maybeApplySmartDefaults()
    revealActiveTab(props.activeTabId, false)
  },
)

watch(
  () => props.activeTabId,
  (tabId) => {
    revealActiveTab(tabId, true)
  },
)

watch(isFiltering, (filtering) => {
  if (!filtering) {
    filterCollapsed.value = new Set()
  }
})

// ============================================================================
// User Interactions
// ============================================================================
function toggleSection(section: SectionKind) {
  if (isFiltering.value) return
  sectionOpen.value = {
    ...sectionOpen.value,
    [section]: !sectionOpen.value[section],
  }
  saveStateToStorage()
}

function toggleFolder(key: string) {
  if (isFiltering.value) {
    const next = new Set(filterCollapsed.value)
    if (next.has(key)) next.delete(key)
    else next.add(key)
    filterCollapsed.value = next
    return
  }
  const next = new Set(expandedFolders.value)
  if (next.has(key)) next.delete(key)
  else next.add(key)
  expandedFolders.value = next
  saveStateToStorage()
}

function toggleCollapseAll() {
  const keys = visibleFolderKeys.value
  if (isFiltering.value) {
    if (isAllCollapsed.value) {
      filterCollapsed.value = new Set()
    } else {
      filterCollapsed.value = new Set(keys)
    }
    return
  }

  if (isAllCollapsed.value) {
    sectionOpen.value = {
      wiki: showWikiSection.value ? true : sectionOpen.value.wiki,
      raw: showRawSection.value ? true : sectionOpen.value.raw,
    }
    const next = new Set(expandedFolders.value)
    for (const k of keys) next.add(k)
    expandedFolders.value = next
  } else if (keys.length > 0) {
    const next = new Set(expandedFolders.value)
    for (const k of keys) next.delete(k)
    expandedFolders.value = next
  } else {
    sectionOpen.value = {
      wiki: showWikiSection.value ? false : sectionOpen.value.wiki,
      raw: showRawSection.value ? false : sectionOpen.value.raw,
    }
  }
  saveStateToStorage()
}

function toggleSortMode() {
  sortBy.value = sortBy.value === 'name' ? 'updated' : 'name'
  saveStateToStorage()
}

function clearSearch() {
  searchQuery.value = ''
  filterCollapsed.value = new Set()
  filterInputEl.value?.blur()
}

function openFileRow(row: Extract<FlatTreeRow, { kind: 'file' }>) {
  if (row.section === 'wiki') emit('openWiki', row.path)
  else emit('openRaw', row.path)
}

function toggleCreate() {
  createOpen.value = !createOpen.value
  if (createOpen.value) {
    createInput.value = ''
    void nextTick(() => createField.value?.focus())
  }
}

function submitCreate() {
  const val = createInput.value.trim()
  if (!val || props.creating) return
  emit('createPage', val)
  createInput.value = ''
  createOpen.value = false
}
</script>

<template>
  <aside class="left-pane" aria-label="知识库文件目录">
    <div class="pane-header">
      <div class="segmented" role="tablist">
        <button
          type="button"
          :class="{ active: filter === 'all' }"
          @click="emit('update:filter', 'all')"
        >全部</button>
        <button
          type="button"
          :class="{ active: filter === 'wiki' }"
          @click="emit('update:filter', 'wiki')"
        >知识页</button>
        <button
          type="button"
          :class="{ active: filter === 'raw' }"
          @click="emit('update:filter', 'raw')"
        >原文</button>
      </div>
      <div class="pane-header-actions">
        <button
          type="button"
          class="icon-btn sm"
          :title="isAllCollapsed ? '全部展开目录' : '全部折叠目录'"
          :aria-label="isAllCollapsed ? '全部展开目录' : '全部折叠目录'"
          @click="toggleCollapseAll"
        >
          <UiIcon :name="isAllCollapsed ? 'expandAll' : 'collapseAll'" size="sm" />
        </button>
        <button
          type="button"
          class="icon-btn sm"
          title="新建知识页（支持子目录，如 concepts/新概念）"
          aria-label="新建知识页"
          @click="toggleCreate"
        >
          <UiIcon name="plus" size="sm" />
        </button>
      </div>
    </div>

    <div class="tree-toolbar">
      <div class="tree-filter-box">
        <UiIcon name="search" size="sm" class="tree-filter-icon" />
        <input
          ref="filterInputEl"
          v-model="searchQuery"
          type="text"
          placeholder="筛选文件名或目录…"
          aria-label="筛选文件名或目录"
          spellcheck="false"
          @keydown.esc.stop="clearSearch"
        >
        <button
          v-if="searchQuery"
          type="button"
          class="tree-filter-clear"
          title="清空筛选"
          aria-label="清空筛选"
          @click="clearSearch"
        >
          <UiIcon name="close" size="sm" />
        </button>
      </div>
      <button
        type="button"
        class="tree-sort-btn"
        :class="{ active: sortBy === 'updated' }"
        :title="sortBy === 'name' ? '当前：按名称排序（点击切换按最近更新）' : '当前：按最近更新排序（点击切换按名称）'"
        :aria-label="sortBy === 'name' ? '切换为按最近更新排序' : '切换为按名称排序'"
        @click="toggleSortMode"
      >
        <UiIcon :name="sortBy === 'name' ? 'sortAsc' : 'clock'" size="sm" />
        <span>{{ sortBy === 'name' ? '名称' : '更新' }}</span>
      </button>
    </div>

    <div v-if="isFiltering" class="tree-filter-status" role="status">
      <span>
        {{ totalFilteredMatches > 0 ? `命中 ${totalFilteredMatches} 篇文档` : '无匹配的文件或目录' }}
      </span>
      <button type="button" class="btn quiet xs" @click="clearSearch">清空</button>
    </div>

    <form v-if="createOpen" class="create-page-inline" @submit.prevent="submitCreate">
      <input
        ref="createField"
        v-model="createInput"
        type="text"
        placeholder="输入标题或路径（如 concepts/新概念）"
        :disabled="creating"
        @keydown.esc.stop="createOpen = false"
      >
      <div style="display:flex;justify-content:flex-end;gap:6px;">
        <button type="button" class="btn quiet xs" @click="createOpen = false">取消</button>
        <button type="submit" class="btn primary xs" :disabled="creating || !createInput.trim()">
          {{ creating ? '创建中…' : '创建' }}
        </button>
      </div>
    </form>

    <div ref="scrollEl" class="pane-scroll tree-pane-scroll" role="tree" aria-label="知识库目录树">
      <ErrorNotice v-if="error" :error="error" style="margin-bottom: 8px;" />

      <div v-if="loading && !tree.wikiPages.length && !tree.rawFiles.length" class="hint" style="padding: 12px 6px;">
        正在读取知识库目录…
      </div>

      <!-- Section 1: Organized Wiki Pages -->
      <section v-if="showWikiSection" class="tree-section">
        <button
          type="button"
          class="tree-section-head"
          :aria-expanded="isFiltering || sectionOpen.wiki"
          @click="toggleSection('wiki')"
        >
          <UiIcon :name="(isFiltering || sectionOpen.wiki) ? 'chevronDown' : 'chevronRight'" size="sm" class="tree-chevron" />
          <span class="tree-section-title">整理后的知识</span>
          <span class="tree-section-count">
            {{ isFiltering ? `${filteredWikiTree.fileCount}/${tree.wikiPages.length}` : tree.wikiPages.length }}
          </span>
        </button>

        <div v-if="isFiltering || sectionOpen.wiki" class="tree-list" role="group">
          <template v-for="row in wikiRows" :key="row.key">
            <button
              v-if="row.kind === 'folder'"
              type="button"
              class="tree-row tree-folder"
              role="treeitem"
              :aria-expanded="row.expanded"
              :aria-level="row.depth + 1"
              :style="{ paddingLeft: `${6 + row.depth * 14}px` }"
              :title="row.fullPath"
              @click="toggleFolder(row.key)"
            >
              <span
                v-for="lvl in row.depth"
                :key="lvl"
                class="tree-guide"
                aria-hidden="true"
                :style="{ left: `${(lvl - 1) * 14 + 13}px` }"
              />
              <UiIcon :name="row.expanded ? 'chevronDown' : 'chevronRight'" size="sm" class="tree-chevron" />
              <UiIcon :name="row.expanded ? 'folderOpen' : 'folder'" size="sm" class="tree-node-icon folder" />
              <span class="tree-label">{{ row.name }}</span>
              <span v-if="row.conflictCount > 0" class="tree-conflict-dot" :title="`包含 ${row.conflictCount} 处待确认冲突`" />
              <span class="tree-count">{{ row.fileCount }}</span>
            </button>

            <button
              v-else
              type="button"
              class="tree-row tree-file"
              :class="{ active: activeTabId === row.key }"
              role="treeitem"
              :aria-selected="activeTabId === row.key"
              :aria-level="row.depth + 1"
              :style="{ paddingLeft: `${22 + row.depth * 14}px` }"
              :title="row.tooltip"
              @click="openFileRow(row)"
            >
              <span
                v-for="lvl in row.depth"
                :key="lvl"
                class="tree-guide"
                aria-hidden="true"
                :style="{ left: `${(lvl - 1) * 14 + 13}px` }"
              />
              <UiIcon name="file" size="sm" class="tree-node-icon" />
              <span class="tree-label">{{ row.title }}</span>
              <span v-if="row.conflictCount > 0" class="tree-badge-warn" title="存在待确认冲突">冲突</span>
            </button>
          </template>

          <p v-if="!loading && !wikiRows.length" class="hint tree-empty-hint">
            {{ isFiltering ? '知识页中无匹配项' : '还没有知识页，点右上角 + 可新建。' }}
          </p>
        </div>
      </section>

      <!-- Section 2: Raw Files -->
      <section v-if="showRawSection" class="tree-section">
        <button
          type="button"
          class="tree-section-head"
          :aria-expanded="isFiltering || sectionOpen.raw"
          :title="`原始文件目录：${effectiveRawDir}/`"
          @click="toggleSection('raw')"
        >
          <UiIcon :name="(isFiltering || sectionOpen.raw) ? 'chevronDown' : 'chevronRight'" size="sm" class="tree-chevron" />
          <span class="tree-section-title">原始文件（{{ effectiveRawDir }}/）</span>
          <span class="tree-section-count">
            {{ isFiltering ? `${filteredRawTree.fileCount}/${tree.rawFiles.length}` : tree.rawFiles.length }}
          </span>
        </button>

        <div v-if="isFiltering || sectionOpen.raw" class="tree-list" role="group">
          <template v-for="row in rawRows" :key="row.key">
            <button
              v-if="row.kind === 'folder'"
              type="button"
              class="tree-row tree-folder"
              role="treeitem"
              :aria-expanded="row.expanded"
              :aria-level="row.depth + 1"
              :style="{ paddingLeft: `${6 + row.depth * 14}px` }"
              :title="row.fullPath"
              @click="toggleFolder(row.key)"
            >
              <span
                v-for="lvl in row.depth"
                :key="lvl"
                class="tree-guide"
                aria-hidden="true"
                :style="{ left: `${(lvl - 1) * 14 + 13}px` }"
              />
              <UiIcon :name="row.expanded ? 'chevronDown' : 'chevronRight'" size="sm" class="tree-chevron" />
              <UiIcon :name="row.expanded ? 'folderOpen' : 'folder'" size="sm" class="tree-node-icon folder" />
              <span class="tree-label">{{ row.name }}</span>
              <span class="tree-count">{{ row.fileCount }}</span>
            </button>

            <button
              v-else
              type="button"
              class="tree-row tree-file"
              :class="{ active: activeTabId === row.key }"
              role="treeitem"
              :aria-selected="activeTabId === row.key"
              :aria-level="row.depth + 1"
              :style="{ paddingLeft: `${22 + row.depth * 14}px` }"
              :title="row.tooltip"
              @click="openFileRow(row)"
            >
              <span
                v-for="lvl in row.depth"
                :key="lvl"
                class="tree-guide"
                aria-hidden="true"
                :style="{ left: `${(lvl - 1) * 14 + 13}px` }"
              />
              <UiIcon name="file" size="sm" class="tree-node-icon" />
              <span class="tree-label">{{ row.title }}</span>
              <span v-if="row.rawKind === 'clip'" class="tree-tag-clip" title="收藏原文">藏</span>
            </button>
          </template>

          <p v-if="!loading && !rawRows.length" class="hint tree-empty-hint">
            {{ isFiltering ? '原始文件中无匹配项' : '还没有原始笔记或收藏原文。' }}
          </p>
        </div>
      </section>
    </div>
  </aside>
</template>
