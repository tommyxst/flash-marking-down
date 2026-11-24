<script setup lang="ts">
const paths = {
  bolt: 'M13 2 4 14h7l-1 8 10-12h-7l1-8Z',
  check: 'M5 13l4 4L19 7',
  close: 'M6 6l12 12M18 6 6 18',
  copy: 'M9 9h10v10H9zM5 15H4a1 1 0 0 1-1-1V4a1 1 0 0 1 1-1h10a1 1 0 0 1 1 1v1',
  external: 'M14 5h5v5m0-5-9 9M10 7H6a1 1 0 0 0-1 1v10a1 1 0 0 0 1 1h10a1 1 0 0 0 1-1v-4',
  file: 'M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8Zm0 0v6h6M8 13h8m-8 4h6',
  folder: 'M3 5h6l2 3h10v11H3Z',
  history: 'M3 11a9 9 0 1 1 2.5 7M3 5v6h6m3-3v5l3 2',
  maximize: 'M5 5h14v14H5z',
  minimize: 'M5 12h14',
  organize: 'M4 5h16M4 12h12M4 19h8m1-7 4 4 4-4',
  question: 'M4 4h16v12h-9l-5 4v-4H4Zm6 5a2 2 0 1 1 3 2l-1 1m0 2h.01',
  restore: 'M8 8h11v11H8zM5 16H4V4h12v1',
  search: 'M21 21l-4.4-4.4m2.4-5.6a8 8 0 1 1-16 0 8 8 0 0 1 16 0Z',
  settings: 'M4 6h16M4 12h16M4 18h16M8 4v4m8 2v4m-6 2v4',
  terminal: 'M3 5h18v14H3zm4 4 3 3-3 3m6 0h4',
  tasks: 'M4 6l2 2 3-4m2 2h9M4 13l2 2 3-4m2 2h9M4 20l2 2 3-4m2 2h9',
}

defineProps<{ name: keyof typeof paths }>()
</script>

<template>
  <svg class="ui-icon" aria-hidden="true" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round">
    <path :d="paths[name]" />
  </svg>
</template>
