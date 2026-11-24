<script setup lang="ts">
import { computed } from 'vue'
import M2Evidence from './M2Evidence.vue'
import {
  disagreementStatuses, relationBadgeTone, relationLabels, statusBadgeTone, useSourcePresentation,
  type Disagreement, type PossibleDisagreement, type RecordSelection,
} from './m2'

const props = defineProps<{ session: string; disagreement: Disagreement | PossibleDisagreement }>()
const emit = defineEmits<{ selectDisagreementSource: [source: RecordSelection] }>()
const recorded = computed(() => 'status' in props.disagreement ? props.disagreement : undefined)
const { sourceMarkdown } = useSourcePresentation()
</script>

<template>
  <article class="detail-card">
    <header class="m2-card-header">
      <h3>{{ disagreement.question }}</h3>
      <div class="actions">
        <span class="badge" :class="relationBadgeTone[disagreement.relation]">{{ relationLabels[disagreement.relation] }}</span>
        <span class="badge" :class="recorded ? statusBadgeTone[recorded.status] : 'badge-warn'">{{ recorded ? disagreementStatuses[recorded.status] : '待确认' }}</span>
      </div>
    </header>
    <p v-if="recorded?.sourcesChanged" class="notice" role="status">部分引用原文已有新版本，以下保留先前判断与引用片段，建议复核。</p>
    <p v-if="recorded?.judgment"><strong>当前判断</strong>　{{ recorded.judgment }}</p>
    <p v-if="recorded?.reason" class="hint">依据与范围　{{ recorded.reason }}</p>
    <p v-if="recorded?.judgmentNeedsSources" class="hint">部分原文缺失或不可读取，暂不展示先前判断和理由。</p>
    <div class="m2-columns">
      <section v-for="member in disagreement.members" :key="member.source.id" class="m2-stack">
        <p v-if="member.source.available && member.text && member.text !== member.source.excerpt">{{ sourceMarkdown(member.text) }}</p>
        <M2Evidence
          :session="session"
          :evidence="member.source"
          @select-disagreement-source="emit('selectDisagreementSource', $event)"
        />
        <slot name="member" :member="member" />
      </section>
    </div>
    <slot />
  </article>
</template>
