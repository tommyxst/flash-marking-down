<script setup lang="ts">
import { history, historyKeymap, insertNewline, standardKeymap } from '@codemirror/commands'
import { markdownLanguage } from '@codemirror/lang-markdown'
import { Compartment, EditorState, Text } from '@codemirror/state'
import { EditorView, keymap, placeholder } from '@codemirror/view'
import { onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { captureMarkdown } from './captureMarkdown'

const props = withDefaults(defineProps<{
  readonly?: boolean
  placeholderText?: string
  autofocus?: boolean
  editorId?: string
}>(), {
  readonly: false,
  placeholderText: '记下现在想到的事…',
  autofocus: false,
  editorId: 'note',
})
const model = defineModel<string>({ required: true })
const emit = defineEmits<{ ready: []; blur: []; 'composition-change': [composing: boolean] }>()
const host = ref<HTMLElement>()
const editable = new Compartment()
const placeholderConfig = new Compartment()
let view: EditorView | undefined
let composing = false
let compositionTimer: number | undefined
let compositionSelection: EditorState | undefined

function editMode() {
  return [EditorState.readOnly.of(props.readonly), EditorView.editable.of(!props.readonly)]
}

function startComposition() {
  window.clearTimeout(compositionTimer)
  const state = view?.state
  compositionSelection = state && !state.selection.main.empty ? state : undefined
  composing = true
  emit('composition-change', true)
}

function finishComposition(event: CompositionEvent) {
  // CodeMirror collects the final composition text from the DOM after compositionend.
  compositionTimer = window.setTimeout(() => {
    if (!view || !composing) return
    if (event.data === '' && compositionSelection) {
      const { from, to } = compositionSelection.selection.main
      // CodeMirror can remove a selected range before the IME commits any replacement.
      if (view.state.doc.eq(compositionSelection.doc.replace(from, to, Text.empty))) {
        view.setState(compositionSelection)
      }
    }
    compositionSelection = undefined
    const { state } = view
    if (!state.toText(model.value).eq(state.doc)) model.value = state.doc.toString()
    composing = false
    emit('composition-change', false)
  })
}

function newEditorState(doc: Text) {
  const enter = (editor: EditorView) => !editor.state.readOnly && insertNewline(editor)
  const nonce = document.querySelector<HTMLStyleElement>('#capture-editor-style')?.nonce
  return EditorState.create({
    doc,
    selection: { anchor: doc.length },
    extensions: [
      nonce ? EditorView.cspNonce.of(nonce) : [],
      EditorView.contentAttributes.of({ id: props.editorId, 'aria-label': 'Markdown 正文', tabindex: '0' }),
      EditorView.lineWrapping,
      placeholderConfig.of(placeholder(props.placeholderText)),
      editable.of(editMode()),
      markdownLanguage,
      captureMarkdown,
      history(),
      keymap.of([
        { key: 'Enter', run: enter, shift: enter },
        ...standardKeymap,
        ...historyKeymap,
      ]),
      EditorView.domEventHandlers({
        blur: () => { emit('blur') },
      }),
      EditorView.updateListener.of((update) => {
        if (update.docChanged && !composing) model.value = update.state.doc.toString()
      }),
    ],
  })
}

onMounted(() => {
  const state = newEditorState(Text.of(model.value.split(/\r\n?|\n/)))
  view = new EditorView({ state, parent: host.value, scrollTo: EditorView.scrollIntoView(state.selection.main) })
  if (props.autofocus) view.focus()
  emit('ready')
})

onBeforeUnmount(() => {
  window.clearTimeout(compositionTimer)
  view?.destroy()
  view = undefined
})

watch(() => props.readonly, () => view?.dispatch({ effects: editable.reconfigure(editMode()) }))
watch(() => props.placeholderText, (value) => {
  view?.dispatch({ effects: placeholderConfig.reconfigure(placeholder(value)) })
})
watch(model, (value) => {
  if (!view) return
  const doc = view.state.toText(value)
  if (!doc.eq(view.state.doc)) view.setState(newEditorState(doc))
})

defineExpose({ focus: () => view?.focus() })
</script>

<template>
  <div ref="host" class="capture-editor" @compositionstart.capture="startComposition" @compositionend="finishComposition" />
</template>

<style scoped>
.capture-editor {
  flex: 1 0 5.5rem;
  min-height: 5.5rem;
  min-width: 0;
  background: var(--inset);
  color: var(--text);
  font: 1rem/1.65 var(--font-mono);
}

.capture-editor:focus-within { outline: 2px solid var(--accent); outline-offset: -2px; }
.capture-editor :deep(.cm-editor) { height: 100%; }
.capture-editor :deep(.cm-editor.cm-focused) { outline: none; }
.capture-editor :deep(.cm-scroller) { font: inherit; }
.capture-editor :deep(.cm-content) { padding: 16px 20px; caret-color: var(--accent); }
.capture-editor :deep(.cm-line) { padding: 0; }
.capture-editor :deep(.cm-placeholder) { color: var(--muted); }
.capture-editor :deep(.cm-line.cm-md-h1) {
  font-size: 1.45em;
  font-weight: 700;
  line-height: 1.38;
  padding-top: 4px;
  padding-bottom: 2px;
  color: var(--text);
}

.capture-editor :deep(.cm-line.cm-md-h2) {
  font-size: 1.25em;
  font-weight: 660;
  line-height: 1.42;
  padding-top: 3px;
  padding-bottom: 1px;
  color: var(--text);
}

.capture-editor :deep(.cm-line.cm-md-h3) {
  font-size: 1.12em;
  font-weight: 640;
  line-height: 1.48;
  color: var(--text);
}

.capture-editor :deep(.cm-line.cm-md-h4) {
  font-size: 1.04em;
  font-weight: 620;
  color: var(--text);
}

.capture-editor :deep(.cm-line.cm-md-quote-line) {
  padding-left: 10px;
  border-left: 3px solid var(--accent);
  color: var(--text-secondary);
}

.capture-editor :deep(.cm-md-strong) { font-weight: 700; color: var(--text); }
.capture-editor :deep(.cm-md-emphasis) { font-style: italic; }
.capture-editor :deep(.cm-md-strikethrough) { text-decoration: line-through; }
.capture-editor :deep(.cm-md-code) { background: var(--surface); color: var(--text-secondary); }
.capture-editor :deep(.cm-md-link) { color: var(--accent); text-decoration: underline; }
.capture-editor :deep(.cm-md-marker) { color: var(--accent); }
.capture-editor :deep(.cm-md-syntax) { color: var(--accent); opacity: 0.78; }
</style>
