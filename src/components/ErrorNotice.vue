<script setup lang="ts">
import { computed } from 'vue'
import { parseAppError, type UiError } from '../api/errors'

const props = defineProps<{
  error?: UiError | string | null
  prefix?: string
  suffix?: string
}>()

const parsed = computed(() => {
  if (!props.error) return null
  return typeof props.error === 'string' ? { safeMessage: props.error } : parseAppError(props.error)
})

const hasTechDetails = computed(() => Boolean(
  parsed.value && (parsed.value.code || parsed.value.stage || parsed.value.operationId || parsed.value.technicalDetail),
))
</script>

<template>
  <div v-if="parsed" class="notice error error-notice" role="alert">
    <p>{{ prefix }}{{ parsed.safeMessage }}{{ suffix }}</p>
    <details v-if="hasTechDetails" class="error-tech">
      <summary>技术详情</summary>
      <code>
        <template v-if="parsed.code">code={{ parsed.code }}</template>
        <template v-if="parsed.stage"> · stage={{ parsed.stage }}</template>
        <template v-if="parsed.operationId"> · op={{ parsed.operationId }}</template>
        <template v-if="parsed.technicalDetail">{{ '\n' + parsed.technicalDetail }}</template>
      </code>
    </details>
  </div>
</template>
