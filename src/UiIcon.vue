<script setup lang="ts">
const paths = {
  alert: 'M12 9v4m0 4h.01M10.3 3.9 2.6 17.5A1.5 1.5 0 0 0 3.9 19.7h16.2a1.5 1.5 0 0 0 1.3-2.2L13.7 3.9a1.5 1.5 0 0 0-3.4 0Z',
  bolt: 'M13 2 4 14h7l-1 8 10-12h-7l1-8Z',
  book: 'M12 6a6 6 0 0 0-4.5-2H3v15h4.5a6 6 0 0 1 4.5 2 6 6 0 0 1 4.5-2H21V4h-4.5A6 6 0 0 0 12 6Zm0 0v15',
  chat: 'M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2Z',
  check: 'M5 13l4 4L19 7',
  chevronDown: 'M6 9l6 6 6-6',
  chevronRight: 'M9 6l6 6-6 6',
  clock: 'M12 7v5l3 2m6-2a9 9 0 1 1-18 0 9 9 0 0 1 18 0Z',
  close: 'M6 6l12 12M18 6 6 18',
  collapseAll: 'M7 4l5 5 5-5M7 20l5-5 5 5',
  conflict: 'M6 3v12M18 9a3 3 0 1 0 0-6 3 3 0 0 0 0 6Zm0 0c0 6-6 3-12 6m0 6a3 3 0 1 0 0-6 3 3 0 0 0 0 6Z',
  copy: 'M9 9h10v10H9zM5 15H4a1 1 0 0 1-1-1V4a1 1 0 0 1 1-1h10a1 1 0 0 1 1 1v1',
  expandAll: 'M7 9l5-5 5 5M7 15l5 5 5-5',
  external: 'M14 5h5v5m0-5-9 9M10 7H6a1 1 0 0 0-1 1v10a1 1 0 0 0 1 1h10a1 1 0 0 0 1-1v-4',
  file: 'M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8Zm0 0v6h6M8 13h8m-8 4h6',
  folder: 'M3 5h6l2 3h10v11H3Z',
  folderOpen: 'M3 6h6l2 2h10v2H3Zm0 4h18l-1.5 9h-15Z',
  graph: 'M6 6a2.5 2.5 0 1 0 0 .01M18 6a2.5 2.5 0 1 0 0 .01M12 18a2.5 2.5 0 1 0 0 .01M8.2 7.2 15.8 7.2M7.3 8.2 10.8 15.8M16.7 8.2 13.2 15.8',
  history: 'M3 11a9 9 0 1 1 2.5 7M3 5v6h6m3-3v5l3 2',
  inbox: 'M4 4h16l2 10v6H2v-6l2-10Zm-2 10h6l2 3h4l2-3h6',
  link: 'M10 13a5 5 0 0 0 7.1 0l2.8-2.8a5 5 0 0 0-7.1-7.1L11.2 4.7M14 11a5 5 0 0 0-7.1 0L4.1 13.8a5 5 0 1 0 7.1 7.1l1.6-1.6',
  maximize: 'M5 5h14v14H5z',
  minimize: 'M5 12h14',
  moon: 'M20 14.5A8 8 0 0 1 9.5 4a7 7 0 1 0 10.5 10.5Z',
  organize: 'M4 5h16M4 12h12M4 19h8m1-7 4 4 4-4',
  plus: 'M12 5v14M5 12h14',
  question: 'M4 4h16v12h-9l-5 4v-4H4Zm6 5a2 2 0 1 1 3 2l-1 1m0 2h.01',
  refresh: 'M20 11a8 8 0 0 0-14.5-4.7L4 8m0-4v4h4M4 13a8 8 0 0 0 14.5 4.7L20 16m0 4v-4h-4',
  restore: 'M8 8h11v11H8zM5 16H4V4h12v1',
  search: 'M21 21l-4.4-4.4m2.4-5.6a8 8 0 1 1-16 0 8 8 0 0 1 16 0Z',
  settings: 'M4 6h16M4 12h16M4 18h16M8 4v4m8 2v4m-6 2v4',
  shield: 'M12 3 5 6v6c0 4.5 3 7.5 7 9 4-1.5 7-4.5 7-9V6l-7-3Z',
  sidebar: 'M3 5h18v14H3zM15 5v14',
  sortAsc: 'M4 6h11M4 12h8M4 18h5m11-11v11m0 0-3-3m3 3 3-3',
  sparkles: 'M12 3v4m0 10v4M3 12h4m10 0h4m-3.2-6.8-2.8 2.8m-6 6-2.8 2.8m0-11.6 2.8 2.8m6 6 2.8 2.8',
  split: 'M3 5h18v14H3zM12 5v14',
  sun: 'M12 4v2m0 12v2M4 12h2m12 0h2m-2.3-5.7-1.4 1.4M7.7 16.3l-1.4 1.4m0-11.4 1.4 1.4m8.6 8.6 1.4 1.4M12 8.5a3.5 3.5 0 1 0 0 7 3.5 3.5 0 0 0 0-7Z',
  tasks: 'M4 6l2 2 3-4m2 2h9M4 13l2 2 3-4m2 2h9M4 20l2 2 3-4m2 2h9',
  terminal: 'M3 5h18v14H3zm4 4 3 3-3 3m6 0h4',
  undo: 'M9 14 4 9l5-5M4 9h10.5a5.5 5.5 0 0 1 0 11H11',
  width: 'M4 12h16M7 9l-3 3 3 3M17 9l3 3-3 3',
} as const

export type IconName = keyof typeof paths

withDefaults(defineProps<{
  name: IconName
  size?: 'sm' | 'md' | 'lg'
}>(), {
  size: 'md',
})
</script>

<template>
  <svg
    class="ui-icon"
    :class="size !== 'md' ? size : undefined"
    aria-hidden="true"
    viewBox="0 0 24 24"
    fill="none"
    stroke="currentColor"
    stroke-width="1.75"
    stroke-linecap="round"
    stroke-linejoin="round"
  >
    <path :d="paths[name]" />
  </svg>
</template>
