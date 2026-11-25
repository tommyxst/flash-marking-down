<script setup lang="ts">
import { ref, watch } from 'vue'
import { api, type AppSettings } from '../api/client'
import { parseAppError, type UiError } from '../api/errors'
import ErrorNotice from './ErrorNotice.vue'

const props = defineProps<{ open: boolean }>()

const preferences = ref<AppSettings>()
const busy = ref(false)
const error = ref<UiError | null>(null)

async function load() {
  busy.value = true
  error.value = null
  try {
    preferences.value = await api.getAppSettings()
  } catch (cause) {
    error.value = parseAppError(cause)
  } finally {
    busy.value = false
  }
}

async function change(key: 'autoOrganize' | 'autoSyncMemory', event: Event) {
  const input = event.target as HTMLInputElement
  busy.value = true
  error.value = null
  try {
    preferences.value = await api.saveAppSettings({ [key]: input.checked })
  } catch (cause) {
    input.checked = preferences.value![key]
    error.value = parseAppError(cause)
  } finally {
    busy.value = false
  }
}

watch(() => props.open, (open) => {
  if (open) void load()
}, { immediate: true })
</script>

<template>
  <section class="settings-form settings-model" aria-labelledby="auto-actions-heading">
    <h3 id="auto-actions-heading">自动处理偏好</h3>
    <template v-if="preferences">
      <label class="m2-check"><input type="checkbox" :checked="preferences.autoOrganize" :disabled="busy" @change="change('autoOrganize', $event)" />保存后自动整理笔记</label>
      <label class="m2-check"><input type="checkbox" :checked="preferences.autoSyncMemory" :disabled="busy" @change="change('autoSyncMemory', $event)" />自动同步到 OpenViking</label>
      <p class="hint">当前版本会保存你的选择。后台整理与同步将在对应功能上线后启用。</p>
    </template>
    <p v-else-if="busy" class="hint" role="status">正在读取偏好…</p>
    <ErrorNotice :error="error" />
    <button v-if="error && !preferences" type="button" :disabled="busy" @click="load">重试读取</button>
  </section>
</template>
