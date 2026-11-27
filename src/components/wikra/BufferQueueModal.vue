<script setup lang="ts">
import { onMounted, onUnmounted } from 'vue'
import type { BufferQueueItem } from '../../m2'
import { formatCountdown } from '../../utils/wikra'

const props = defineProps<{
  open: boolean
  items: BufferQueueItem[]
  secondsLeft: number
  busy: boolean
}>()

const emit = defineEmits<{
  close: []
  cancelItem: [itemId: string]
  flushNow: []
}>()

function onKeydown(e: KeyboardEvent) {
  if (props.open && e.key === 'Escape') {
    e.preventDefault()
    emit('close')
  }
}

onMounted(() => window.addEventListener('keydown', onKeydown))
onUnmounted(() => window.removeEventListener('keydown', onKeydown))
</script>

<template>
  <div v-if="open" class="overlay-layer" role="dialog" aria-modal="true" aria-label="待整理笔记队列" @click.self="emit('close')">
    <div class="modal-window compact">
      <div class="hud-head">
        <strong style="font-size:14px;">
          待整理笔记队列
          <template v-if="secondsLeft > 0"> · 剩余 {{ formatCountdown(secondsLeft) }}</template>
        </strong>
        <button type="button" class="icon-btn" aria-label="关闭" @click="emit('close')">&times;</button>
      </div>

      <div class="modal-body">
        <p style="font-size:12.5px;color:var(--text-secondary);">
          以下笔记已安全保存为本地原文。写错了可在倒计时结束前移出本次整理（本地原文会保留）。
        </p>

        <div v-for="n in items" :key="n.id" class="ops-row">
          <div class="ops-row-main">
            <span class="ops-row-title">{{ n.title }}</span>
            <span class="ops-row-desc">{{ n.preview }}</span>
          </div>
          <button
            type="button"
            class="btn sm"
            :disabled="busy"
            @click="emit('cancelItem', n.id)"
          >移出本批</button>
        </div>

        <p v-if="!items.length" class="hint">当前待整理队列已清空。</p>
      </div>

      <div class="hud-foot">
        <button type="button" class="btn quiet sm" @click="emit('close')">关闭</button>
        <button
          v-if="items.length > 0"
          type="button"
          class="btn primary sm"
          :disabled="busy"
          @click="emit('flushNow')"
        >{{ busy ? '正在触发…' : '立即整理' }}</button>
      </div>
    </div>
  </div>
</template>
