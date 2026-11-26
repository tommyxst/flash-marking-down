<script setup lang="ts">
import { isTauri } from '@tauri-apps/api/core'
import { getCurrentWindow } from '@tauri-apps/api/window'
import { defineAsyncComponent } from 'vue'
import './ui.css'

const isCapture = isTauri() && getCurrentWindow().label === 'capture'
const CaptureWindow = defineAsyncComponent(() => import('./windows/CaptureWindow.vue'))
const WorkbenchShell = defineAsyncComponent(() => import('./windows/WorkbenchShell.vue'))
const ActiveWindow = isCapture ? CaptureWindow : WorkbenchShell
</script>

<template>
  <ActiveWindow />
</template>
