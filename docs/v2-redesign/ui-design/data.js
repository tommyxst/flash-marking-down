// Wikra v2.0 — Synthetic Dataset (Rev.3: Low Cognitive Load + Unified Raw & Wiki Viewer)
window.FMD_ICONS = {
  book: 'M12 6a6 6 0 0 0-4.5-2H3v15h4.5a6 6 0 0 1 4.5 2 6 6 0 0 1 4.5-2H21V4h-4.5A6 6 0 0 0 12 6Zm0 0v15',
  inbox: 'M4 4h16l2 10v6H2v-6l2-10Zm-2 10h6l2 3h4l2-3h6',
  sparkles: 'M12 3v4m0 10v4M3 12h4m10 0h4m-3.2-6.8-2.8 2.8m-6 6-2.8 2.8m0-11.6 2.8 2.8m6 6 2.8 2.8',
  graph: 'M6 6a2.5 2.5 0 1 0 0 .01M18 6a2.5 2.5 0 1 0 0 .01M12 18a2.5 2.5 0 1 0 0 .01M8.2 7.2 15.8 7.2M7.3 8.2 10.8 15.8M16.7 8.2 13.2 15.8',
  bolt: 'M13 2 4 14h7l-1 8 10-12h-7l1-8Z',
  chat: 'M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2Z',
  conflict: 'M6 3v12M18 9a3 3 0 1 0 0-6 3 3 0 0 0 0 6Zm0 0c0 6-6 3-12 6m0 6a3 3 0 1 0 0-6 3 3 0 0 0 0 6Z',
  search: 'M21 21l-4.4-4.4m2.4-5.6a8 8 0 1 1-16 0 8 8 0 0 1 16 0Z',
  settings: 'M4 6h16M4 12h16M4 18h16M8 4v4m8 2v4m-6 2v4',
  plus: 'M12 5v14M5 12h14',
  history: 'M3 11a9 9 0 1 1 2.5 7M3 5v6h6m3-3v5l3 2',
  sidebar: 'M3 5h18v14H3zM15 5v14',
  split: 'M3 5h18v14H3zM12 5v14',
  width: 'M4 12h16M7 9l-3 3 3 3M17 9l3 3-3 3',
  check: 'M5 13l4 4L19 7',
  close: 'M6 6l12 12M18 6 6 18',
  file: 'M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8Zm0 0v6h6M8 13h8m-8 4h6',
  minimize: 'M5 12h14',
  maximize: 'M5 5h14v14H5z',
  external: 'M14 5h5v5m0-5-9 9M10 7H6a1 1 0 0 0-1 1v10a1 1 0 0 0 1 1h10a1 1 0 0 0 1-1v-4',
  shield: 'M12 3 5 6v6c0 4.5 3 7.5 7 9 4-1.5 7-4.5 7-9V6l-7-3Z',
  undo: 'M9 14 4 9l5-5M4 9h10.5a5.5 5.5 0 0 1 0 11H11',
  sun: 'M12 4v2m0 12v2M4 12h2m12 0h2m-2.3-5.7-1.4 1.4M7.7 16.3l-1.4 1.4m0-11.4 1.4 1.4m8.6 8.6 1.4 1.4M12 8.5a3.5 3.5 0 1 0 0 7 3.5 3.5 0 0 0 0-7Z',
  moon: 'M20 14.5A8 8 0 0 1 9.5 4a7 7 0 1 0 10.5 10.5Z'
};

window.FMD_INITIAL_STATE = {
  vaultName: 'personal-llm-wiki',
  // Default landing = 'docs' (Unified Knowledge Base Document Viewer, Obsidian-first!)
  activeWorkspace: 'docs', // docs | organize | graph
  docLeftMode: 'library', // library | inbox
  libraryFilter: 'all', // all | wiki | raw
  inboxFilter: 'pending', // pending | archived | all
  inboxReadMode: 'full', // full | highlights
  activeDocTab: 'page:decision-reconnect', // page:<id> | raw:<id> | inbox:<id> | review:<id>
  openDocTabs: [
    { id: 'page:decision-reconnect', label: '流式断线恢复决策', kind: 'page' },
    { id: 'raw:raw-note-1126', label: '原文：SSE 续传现场速记', kind: 'raw' }
  ],
  activePageId: 'decision-reconnect',
  activeRawId: 'raw-note-1126',
  selectedInboxId: 'raw-proto-v2',
  editorMode: 'preview', // preview | source
  propsExpanded: false,
  externalChangedBanner: false,
  // Per-page active recall blind curtain (isolated by pageId per Carbon R2 guardrail!)
  recallCurtainPageId: null,
  recallPeekSections: {},
  // Per-page split-screen original source viewer (click citation or toolbar "对照原文")
  evidenceByPage: {},
  // Per-view right panel memory (default OPEN on library & inbox for balanced 3-pane wide-screen!)
  rightOpenByView: { library: true, raw: true, inbox: true, organize: false, review: false, graph: false },
  rightTab: 'info', // info | chat
  bufferSecondsLeft: 0,
  bufferNotes: [],
  chatIncludeOneHop: false,
  chatMode: 'query', // query | review
  reviewFlowStep: 0,
  reviewSelfRating: null,
  graphSelectedNode: 'decision-reconnect',
  graphSelectedEdgeId: 'e1',
  graphTypeFilter: 'all',
  graphShowMetaPages: false,
  searchQuery: '',
  searchOpen: false,

  // Original raw files (User's quick notes + saved web clips — browsable directly in Knowledge Base!)
  rawFiles: {
    'raw-note-1126': {
      id: 'raw-note-1126',
      path: 'raw/notes/2026/11/20261126-1015-sse-reconnect.md',
      title: '协议评审现场速记：SSE 续传与事件编号',
      created: '2026-11-26 10:15',
      kind: 'note',
      kindLabel: '随手笔记',
      url: '',
      linkedPageIds: ['decision-reconnect', 'concept-dedup'],
      content: '上午网关评审提到，SSE v2 响应帧已经强制带上单调递增的 event_id。\n\n客户端只要拿到服务端确认过的最后一个 event_id，断线后不需要重跑整段推理，直接携带 Last-Event-ID 从检查点续传即可。老版本网关没有 event_id，依然要整段重算。\n\n还需要注意：如果前端已经渲染了未确认的本地缓存草稿，续传前必须核对 offset，避免界面上出现重复句子。'
    },
    'raw-proto-v2': {
      id: 'raw-proto-v2',
      path: 'raw/clips/2026/11/stream-gateway-v2-spec.md',
      title: '端侧流式网关协议 v2.1 规范摘录',
      created: '2026-11-26 09:40',
      kind: 'clip',
      kindLabel: '收藏原文',
      url: 'https://specs.example.dev/stream-gateway/v2.1#resume',
      linkedPageIds: ['decision-reconnect', 'concept-event-id', 'synthesis-resilience'],
      content: '## 3.2 重连条件与检查点续传\n\n当客户端与网关连接在回答尚未结束时中断，若当前会话使用协议 v2.1 且客户端已收到服务端确认的 event_id，则允许在 120 秒窗口内通过 Last-Event-ID 请求检查点续传。\n\n## 3.3 幂等与去重窗口\n\n服务端在 120 秒内保留已确认帧的顺序快照；若客户端未携带 event_id 或窗口已过期，网关将返回 410 Gone，客户端必须回退到整段重新生成。\n\n## 3.4 移动端弱网心跳\n\n在弱网切换（Wi-Fi 转 5G）场景下，建议客户端保持 15 秒心跳探测，超时 2 次即触发受控重连。'
    },
    'raw-old-adr': {
      id: 'raw-old-adr',
      path: 'raw/notes/2026/09/20260912-1620-v1-fallback.md',
      title: '早期架构记录：v1 断线统一重算方案',
      created: '2026-09-12 16:20',
      kind: 'note',
      kindLabel: '随手笔记',
      url: '',
      linkedPageIds: ['decision-reconnect'],
      content: '当前 v1 网关流式接口不返回消息序号，服务端也没有中间态缓存。\n\n因此客户端在断线后一律清空半截气泡并重新发起完整请求，先保证不会出现拼接乱序。'
    }
  },

  // Organized LLM Wiki pages
  pages: {
    'decision-reconnect': {
      id: 'decision-reconnect',
      path: 'wiki/decisions/stream-reconnect-strategy.md',
      title: '流式断线恢复决策',
      type: 'decision',
      typeLabel: '核心决策',
      folder: 'wiki/decisions',
      aliases: ['SSE 断线恢复', '流式续传策略'],
      tags: ['流式架构', '客户端'],
      updated: '2026-11-26',
      fmdManaged: true,
      fmdCore: true,
      pendingConflicts: ['rev-12'],
      outline: ['当前决策结论', '版本演进与适用边界', '待办清单', '我的理解'],
      tasks: [
        { id: 't1', text: '与网关侧确认 v2.1 410 Gone 过期窗口精确时长', done: true },
        { id: 't2', text: '在弱网模拟环境验证 Last-Event-ID 续传与界面去重逻辑', done: false }
      ],
      humanUnderstanding: '核心边界不在“能不能续传”，而在有没有服务端确认过的事件编号。没有编号就退回整段重算，不能靠客户端猜字符偏移量来拼接。',
      sections: [
        {
          heading: '当前决策结论',
          body: '早期决策要求客户端在流式回答断线后一律清空半截内容并整段重新生成',
          citationIds: ['c1'],
          links: ['concept-event-id', 'concept-dedup']
        },
        {
          heading: '版本演进与适用边界',
          body: '按协议版本演进自动区分范围：v1 网关无事件编号，继续沿用整段重算；v2.1 网关已引入服务端确认编号，但默认客户端策略存在新旧分歧，点上方提示可左右对比确认。',
          citationIds: ['c2', 'c3'],
          links: ['entity-stream-gateway', 'synthesis-resilience']
        }
      ],
      citations: [
        { id: 'c1', ref: 's1', rawId: 'raw-old-adr', anchor: 'raw/notes/2026/09/20260912-1620-v1-fallback.md', label: '早期架构记录（09-12）', quote: '客户端在断线后一律清空半截气泡并重新发起完整请求，先保证不会出现拼接乱序。' },
        { id: 'c2', ref: 's2', rawId: 'raw-proto-v2', anchor: 'raw/clips/2026/11/stream-gateway-v2-spec.md#3.2', label: '协议 v2.1 规范 #3.2', quote: '若当前会话使用协议 v2.1 且客户端已收到服务端确认的 event_id，则允许在 120 秒窗口内通过 Last-Event-ID 请求检查点续传。' },
        { id: 'c3', ref: 's3', rawId: 'raw-note-1126', anchor: 'raw/notes/2026/11/20261126-1015-sse-reconnect.md', label: '评审现场速记（11-26）', quote: '客户端只要拿到服务端确认过的最后一个 event_id，断线后不需要重跑整段推理，直接携带 Last-Event-ID 从检查点续传即可。' }
      ],
      backlinks: [
        { pageId: 'concept-dedup', title: '流式消息去重机制', excerpt: '续传恢复与 [[流式断线恢复决策]] 共享 event_id 游标校验。' },
        { pageId: 'synthesis-resilience', title: '端侧长连接韧性综论', excerpt: '现行恢复边界以 [[流式断线恢复决策]] 为基准。' }
      ]
    },
    'concept-event-id': {
      id: 'concept-event-id',
      path: 'wiki/concepts/server-confirmed-event-id.md',
      title: '服务端确认事件编号（event_id）',
      type: 'concept',
      typeLabel: '概念',
      folder: 'wiki/concepts',
      aliases: ['Last-Event-ID', '事件游标'],
      tags: ['流式架构', '协议'],
      updated: '2026-11-26',
      fmdManaged: true,
      fmdCore: false,
      pendingConflicts: [],
      outline: ['定义与约束', '过期与回退行为', '我的理解'],
      tasks: [],
      humanUnderstanding: 'event_id 必须由服务端单调递增签发，客户端本地生成的临时序号不能用于断点续传。',
      sections: [
        { heading: '定义与约束', body: '单调递增的帧级确认标识，仅在服务端完成持久化快照后下发，用于界定客户端与网关共同认可的已交付边界。', citationIds: ['c2'], links: ['decision-reconnect', 'concept-dedup'] },
        { heading: '过期与回退行为', body: '快照窗口默认为 120 秒；超时后网关返回 410 Gone，调用方必须放弃续传并回退至整段重新生成。', citationIds: ['c4'], links: ['entity-stream-gateway'] }
      ],
      citations: [
        { id: 'c2', ref: 's1', rawId: 'raw-proto-v2', anchor: 'raw/clips/2026/11/stream-gateway-v2-spec.md#3.2', label: '协议 v2.1 规范 #3.2', quote: '若当前会话使用协议 v2.1 且客户端已收到服务端确认的 event_id，则允许在 120 秒窗口内通过 Last-Event-ID 请求检查点续传。' },
        { id: 'c4', ref: 's2', rawId: 'raw-proto-v2', anchor: 'raw/clips/2026/11/stream-gateway-v2-spec.md#3.3', label: '协议 v2.1 规范 #3.3', quote: '若客户端未携带 event_id 或窗口已过期，网关将返回 410 Gone，客户端必须回退到整段重新生成。' }
      ],
      backlinks: [
        { pageId: 'decision-reconnect', title: '流式断线恢复决策', excerpt: '续传前提依赖 [[服务端确认事件编号（event_id）]]。' },
        { pageId: 'concept-dedup', title: '流式消息去重机制', excerpt: '去重键以 [[服务端确认事件编号（event_id）]] 结合 offset 组成。' }
      ]
    },
    'concept-dedup': {
      id: 'concept-dedup',
      path: 'wiki/concepts/stream-chunk-deduplication.md',
      title: '流式消息去重机制',
      type: 'concept',
      typeLabel: '概念',
      folder: 'wiki/concepts',
      aliases: ['片段去重'],
      tags: ['客户端', '稳定性'],
      updated: '2026-11-26',
      fmdManaged: true,
      fmdCore: false,
      pendingConflicts: ['rev-12'],
      outline: ['去重边界'],
      tasks: [],
      humanUnderstanding: '',
      sections: [
        { heading: '去重边界', body: '客户端渲染流式增量片段时，需要同时记录已确认的 event_id 与本地已渲染字节偏移，避免重连回放导致界面重复吐字。', citationIds: ['c5'], links: ['decision-reconnect', 'concept-event-id'] }
      ],
      citations: [
        { id: 'c5', ref: 's1', rawId: 'raw-note-1126', anchor: 'raw/notes/2026/11/20261126-1015-sse-reconnect.md', label: '评审现场速记（11-26）', quote: '如果前端已经渲染了未确认的本地缓存草稿，续传前必须核对 offset，避免界面上出现重复句子。' }
      ],
      backlinks: [{ pageId: 'decision-reconnect', title: '流式断线恢复决策', excerpt: '关联 [[流式消息去重机制]]。' }]
    },
    'synthesis-resilience': {
      id: 'synthesis-resilience',
      path: 'wiki/syntheses/client-streaming-resilience.md',
      title: '端侧长连接韧性综论',
      type: 'synthesis',
      typeLabel: '综论',
      folder: 'wiki/syntheses',
      aliases: ['流式韧性总结'],
      tags: ['流式架构'],
      updated: '2026-11-25',
      fmdManaged: true,
      fmdCore: false,
      pendingConflicts: ['rev-12'],
      outline: ['总览'],
      tasks: [],
      humanUnderstanding: '',
      sections: [
        { heading: '总览', body: '端侧流式会话的稳定性由三个环节组成：心跳探测、检查点续传和界面片段去重。', citationIds: ['c6'], links: ['decision-reconnect', 'entity-stream-gateway'] }
      ],
      citations: [
        { id: 'c6', ref: 's1', rawId: 'raw-proto-v2', anchor: 'raw/clips/2026/11/stream-gateway-v2-spec.md#3.4', label: '协议 v2.1 规范 #3.4', quote: '在弱网切换（Wi-Fi 转 5G）场景下，建议客户端保持 15 秒心跳探测，超时 2 次即触发受控重连。' }
      ],
      backlinks: []
    },
    'entity-stream-gateway': {
      id: 'entity-stream-gateway',
      path: 'wiki/entities/stream-gateway-v2.md',
      title: 'Stream Gateway v2.1',
      type: 'entity',
      typeLabel: '组件',
      folder: 'wiki/entities',
      aliases: ['流式网关'],
      tags: ['基础设施'],
      updated: '2026-11-26',
      fmdManaged: true,
      fmdCore: false,
      pendingConflicts: [],
      outline: ['能力摘要'],
      tasks: [],
      humanUnderstanding: '',
      sections: [
        { heading: '能力摘要', body: '负责端侧对话流式转发、单调递增 event_id 签发与 120 秒检查点快照缓存。', citationIds: ['c2'], links: ['concept-event-id'] }
      ],
      citations: [
        { id: 'c2', ref: 's1', rawId: 'raw-proto-v2', anchor: 'raw/clips/2026/11/stream-gateway-v2-spec.md#3.2', label: '协议 v2.1 规范 #3.2', quote: '若当前会话使用协议 v2.1 且客户端已收到服务端确认的 event_id，则允许在 120 秒窗口内通过 Last-Event-ID 请求检查点续传。' }
      ],
      backlinks: [
        { pageId: 'decision-reconnect', title: '流式断线恢复决策', excerpt: '依赖 [[Stream Gateway v2.1]] 提供的检查点能力。' },
        { pageId: 'synthesis-resilience', title: '端侧长连接韧性综论', excerpt: '网关侧由 [[Stream Gateway v2.1]] 承载。' }
      ]
    },
    'orphan-cache': {
      id: 'orphan-cache',
      path: 'wiki/concepts/local-buffer-eviction.md',
      title: '端侧本地缓存淘汰策略',
      type: 'concept',
      typeLabel: '概念',
      folder: 'wiki/concepts',
      aliases: [],
      tags: ['客户端'],
      updated: '2026-11-18',
      fmdManaged: true,
      fmdCore: false,
      pendingConflicts: [],
      outline: ['LRU 上限说明'],
      tasks: [],
      humanUnderstanding: '',
      sections: [
        { heading: 'LRU 上限说明', body: '端侧未提交会话缓存上限设为 20 MiB，超出后按最近最少使用顺序淘汰。', citationIds: [], links: [] }
      ],
      citations: [],
      backlinks: []
    }
  },

  reviews: [
    {
      id: 'rev-12',
      status: 'pending',
      resolution: null,
      title: '断线后是整段重算，还是按 event_id 断点续传？',
      plainHint: '你在 9 月写的旧决策要求“一律整段重算”，而今天新收集的协议 v2.1 规范与评审速记提出“120 秒内可断点续传”。请确认最终结论。',
      current: {
        claim: '客户端在流式回答断线后一律清空半截内容并整段重新生成。',
        sourceLabel: '早期架构记录（09-12）',
        quote: '客户端在断线后一律清空半截气泡并重新发起完整请求，先保证不会出现拼接乱序。'
      },
      incoming: {
        claim: '收到服务端确认 event_id 且在 120 秒窗口内时，携带 Last-Event-ID 从检查点续传；无编号或 410 Gone 再回退整段重算。',
        sourceLabel: '协议 v2.1 规范 & 评审速记（11-26）',
        quote: '若当前会话使用协议 v2.1 且客户端已收到服务端确认的 event_id，则允许在 120 秒窗口内通过 Last-Event-ID 请求检查点续传。'
      },
      affectedPages: [
        { pageId: 'decision-reconnect', title: '流式断线恢复决策', opLabel: '更新「当前决策结论」', override: 'follow' },
        { pageId: 'concept-dedup', title: '流式消息去重机制', opLabel: '补充续传 offset 核对规则', override: 'follow' },
        { pageId: 'synthesis-resilience', title: '端侧长连接韧性综论', opLabel: '更新恢复边界总览', override: 'follow' }
      ],
      suggestedCondition: 'v2.1 协议且有服务端确认 event_id（120 秒内）→ 断点续传；老版本 v1 或超时 410 Gone → 整段重算'
    }
  ],

  // Inbox: Readwise Reader-style full article reading + click paragraph to highlight + Open Original URL!
  inbox: [
    {
      id: 'raw-proto-v2',
      title: '端侧流式网关协议 v2.1 规范摘录',
      domain: 'specs.example.dev',
      sourceType: '网页文章',
      url: 'https://specs.example.dev/stream-gateway/v2.1#resume',
      savedAt: '今天 09:40',
      readTime: '约 2 分钟',
      fetchStatus: 'full',
      state: 'reading', // unread | reading | compiled | archived
      wordCount: 410,
      summaryPoints: [
        '协议 v2.1 引入单调递增的 event_id，允许断线后在 120 秒内按 Last-Event-ID 续传。',
        '若未带编号或超过 120 秒，网关返回 410 Gone，客户端需回退整段重新生成。',
        '移动端 Wi-Fi 切 5G 等弱网场景建议保持 15 秒心跳，连续 2 次超时触发重连。'
      ],
      relatedPageIds: ['decision-reconnect', 'concept-event-id'],
      paragraphs: [
        { id: 'p0', kind: 'intro', heading: '', text: '本文档定义端侧客户端与流式推理网关 v2.1 之间的长连接恢复协议，旨在减少弱网断线导致的整段重算开销，同时保证客户端渲染顺序严格一致。', highlighted: false, compiled: false },
        { id: 'p1', kind: 'section', heading: '3.2 重连条件与检查点续传', text: '当客户端与网关连接在回答尚未结束时中断，若当前会话使用协议 v2.1 且客户端已收到服务端确认的 event_id，则允许在 120 秒窗口内通过 Last-Event-ID 请求检查点续传。', highlighted: true, compiled: true },
        { id: 'p2', kind: 'section', heading: '3.3 幂等与去重窗口', text: '服务端在 120 秒内保留已确认帧的顺序快照；若客户端未携带 event_id 或窗口已过期，网关将返回 410 Gone，客户端必须回退到整段重新生成。', highlighted: true, compiled: false },
        { id: 'p3', kind: 'section', heading: '3.4 移动端弱网心跳', text: '在弱网切换（Wi-Fi 转 5G）场景下，建议客户端保持 15 秒心跳探测，超时 2 次即触发受控重连。客户端应先比对本地已渲染偏移量，再追加续传帧。', highlighted: false, compiled: false }
      ]
    },
    {
      id: 'inbox-paywall',
      title: 'Hindsight Memory Architecture: Mental Models in Production',
      domain: 'members.example.org',
      sourceType: '需登录网页',
      url: 'https://members.example.org/hindsight-mental-models-deep-dive',
      savedAt: '2 天前',
      readTime: '待补全原文',
      fetchStatus: 'stub',
      state: 'unread',
      wordCount: 0,
      summaryPoints: [
        '该页面受登录限制，已安全保存链接与标题，不会读取浏览器 Cookie。',
        '点击上方「打开原网页」阅读后，可将关键段落粘贴至下方保存为本地原文。'
      ],
      relatedPageIds: [],
      paragraphs: [],
      pastedBody: ''
    },
    {
      id: 'inbox-longread',
      title: 'Designing Local-First Knowledge Compilers with Structured Ops',
      domain: 'notes.local',
      sourceType: '粘贴长文',
      url: '',
      savedAt: '11 天前',
      readTime: '约 1 分钟',
      fetchStatus: 'full',
      state: 'unread',
      wordCount: 280,
      summaryPoints: [
        '采用结构化增量更新优于整页重写，可避免破坏手动笔记与第三方插件语法。',
        '高风险的观点覆盖自动交给人确认，普通补充后台静默完成并支持撤销。'
      ],
      relatedPageIds: ['decision-reconnect'],
      paragraphs: [
        { id: 'p4', kind: 'section', heading: '为什么增量更新优于整页重写', text: '整页重写会在无关段落制造大量差异噪声，并破坏用户在 Obsidian 中手写的格式与插件语法块；将 AI 变更约束为局部增量后，人工保护与一键撤销变得非常可靠。', highlighted: false, compiled: false },
        { id: 'p5', kind: 'section', heading: '让用户保持在阅读流中', text: '优秀整理的标志是“安静”——平时不必打断用户的记录与阅读，只在真正出现观点矛盾时邀请用户做一次清晰对比。', highlighted: false, compiled: false }
      ]
    }
  ],

  changesets: [
    {
      id: 'cs-0142',
      time: '今天 10:19',
      trigger: '根据随手笔记《协议评审现场速记》整理',
      modelAlias: 'GPT-6.2 Sol',
      status: 'partial',
      summary: '更新 2 篇知识页 · 发现 1 处观点冲突',
      pageIds: ['concept-event-id', 'decision-reconnect'],
      ops: [
        { label: '补充内容', target: '服务端确认事件编号（event_id）', outcome: '已更新' },
        { label: '发现冲突', target: '流式断线恢复决策等 3 篇', outcome: '待你确认' }
      ]
    },
    {
      id: 'cs-0141',
      time: '昨天 21:08',
      trigger: '根据收藏文章《协议 v2.1 规范摘录》整理',
      modelAlias: 'GPT-6.2 Sol',
      status: 'applied',
      summary: '更新 2 篇知识页 · 原文出处已核对',
      pageIds: ['synthesis-resilience', 'entity-stream-gateway'],
      ops: [
        { label: '补充章节', target: '端侧长连接韧性综论', outcome: '已更新' },
        { label: '补充关联', target: 'Stream Gateway v2.1', outcome: '已更新' }
      ]
    }
  ],

  // Gentle optional suggestions (only visible inside "整理" view, never cluttering the main reading screen!)
  suggestions: [
    { id: 's1', kind: '复习建议', title: '回想一下《流式断线恢复决策》', detail: '用自己的话复述核心边界（系统会先遮住正文），巩固记忆。', actionLabel: '让我讲一遍', actionType: 'teachback', pageId: 'decision-reconnect', dismissed: false },
    { id: 's2', kind: '孤立笔记', title: '《端侧本地缓存淘汰策略》尚未关联其他笔记', detail: '可以打开看看是否要和流式恢复笔记建立关联，或者直接忽略。', actionLabel: '打开查看', actionType: 'open', pageId: 'orphan-cache', dismissed: false }
  ],

  graph: {
    nodes: [
      { id: 'decision-reconnect', label: '流式断线恢复决策', type: 'decision', x: 320, y: 185, hub: true },
      { id: 'concept-event-id', label: '服务端确认 event_id', type: 'concept', x: 540, y: 120, hub: true },
      { id: 'concept-dedup', label: '流式消息去重机制', type: 'concept', x: 530, y: 270, hub: false },
      { id: 'synthesis-resilience', label: '端侧长连接韧性综论', type: 'synthesis', x: 145, y: 275, hub: false },
      { id: 'entity-stream-gateway', label: 'Stream Gateway v2.1', type: 'entity', x: 170, y: 95, hub: false },
      { id: 'orphan-cache', label: '本地缓存淘汰策略', type: 'concept', x: 640, y: 345, orphan: true }
    ],
    edges: [
      { id: 'e1', from: 'decision-reconnect', to: 'concept-event-id', kind: 'link', label: '显式关联', snippet: '“客户端只要拿到服务端确认过的最后一个 event_id，断线后不需要重跑整段推理。”' },
      { id: 'e2', from: 'decision-reconnect', to: 'concept-dedup', kind: 'backlink', label: '反向引用', snippet: '“续传恢复与 [[流式断线恢复决策]] 共享 event_id 游标校验，避免界面重复吐字。”' },
      { id: 'e4', from: 'concept-event-id', to: 'concept-dedup', kind: 'link', label: '显式关联', snippet: '“去重键以 event_id 结合本地已渲染 offset 组成。”' },
      { id: 'e5', from: 'synthesis-resilience', to: 'decision-reconnect', kind: 'backlink', label: '反向引用', snippet: '“现行恢复边界以 [[流式断线恢复决策]] 为基准。”' },
      { id: 'e6', from: 'entity-stream-gateway', to: 'concept-event-id', kind: 'link', label: '显式关联', snippet: '“Stream Gateway v2.1 负责单调递增 event_id 签发与 120 秒快照缓存。”' },
      { id: 'e7', from: 'synthesis-resilience', to: 'entity-stream-gateway', kind: 'link', label: '显式关联', snippet: '“网关侧心跳与快照由 Stream Gateway v2.1 承载。”' }
    ],
    gap: {
      title: '弱网重试 × 本地缓存淘汰',
      reason: '《端侧长连接韧性综论》与《端侧本地缓存淘汰策略》都涉及端侧会话缓存，但尚未建立关联。',
      question: '当 120 秒续传窗口内触发本地 20 MiB 缓存淘汰时，客户端应如何回退整段重算？',
      dismissed: false
    }
  },

  chatMessages: [
    { role: 'user', text: '什么时候必须放弃断点续传、回退到整段重新生成？' },
    {
      role: 'assistant',
      layers: [
        { tag: '来自你的知识库（点击可对照原文）', text: '满足任一条件即须回退至整段重新生成：① 会话走老版本 v1 网关（无服务端确认 event_id）；② 未携带 event_id 或超过 120 秒窗口，网关返回 410 Gone。', citationIds: ['c1', 'c4'] },
        { tag: '冲突提醒', text: '《流式断线恢复决策》目前还有 1 处新旧结论冲突待确认，实施前建议先拍板。', citationIds: [] },
        { tag: '关联记忆（来自 OpenViking）', text: '你上周评审时重点关注过弱网切换避免重复吐字，建议同步核对去重 offset 逻辑。', citationIds: [] }
      ],
      savedToWiki: false
    }
  ],

  settings: {
    modelEndpoint: 'http://127.0.0.1:3425/v1',
    modelPrimary: 'gpt-6.2-sol',
    modelAlias: 'GPT-6.2 Sol',
    modelLight: 'deepseek-v4.1-flash',
    allowNoteFulltextToModel: true,
    memoryProvider: 'openviking',
    ovEndpoint: 'http://127.0.0.1:1933',
    mcpEnabled: true,
    hotkey: 'Win+Alt+N',
    bufferSeconds: 60,
    quietThreshold: true
  }
};
