<script setup lang="ts">
import { isTauri } from '@tauri-apps/api/core'
import { getCurrentWindow } from '@tauri-apps/api/window'
import { defineAsyncComponent } from 'vue'
import './ui.css'

const isCapture = isTauri() && getCurrentWindow().label === 'capture'
const ActiveWindow = defineAsyncComponent(() =>
  isCapture ? import('./windows/CaptureWindow.vue') : import('./windows/WorkbenchShell.vue'),
)
</script>

<template>
  <ActiveWindow />
</template>
