// Wikra v2.0 — Interactive Prototype Controller (Rev.3: Low Cognitive Load & Unified Viewer)
(function () {
  const S = structuredClone(window.FMD_INITIAL_STATE);
  const ICONS = window.FMD_ICONS;

  const $ = (sel, root = document) => root.querySelector(sel);
  const esc = (str) => String(str ?? '')
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;');

  function icon(name, cls = '') {
    const d = ICONS[name] || ICONS.file;
    return `<svg class="ui-icon ${cls}" aria-hidden="true" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round"><path d="${d}"/></svg>`;
  }

  function showToast(msg, actions = []) {
    const region = $('#toast-region');
    const el = document.createElement('div');
    el.className = 'toast';
    el.innerHTML = `<span>${esc(msg)}</span>` + actions.map((a, i) => `<button type="button" class="btn sm quiet" data-toast-act="${i}" style="min-height:22px;padding:1px 6px;color:var(--accent);">${esc(a.label)}</button>`).join('');
    actions.forEach((a, i) => {
      el.querySelector(`[data-toast-act="${i}"]`)?.addEventListener('click', () => {
        el.remove();
        a.onClick();
      });
    });
    region.appendChild(el);
    setTimeout(() => el.remove(), actions.length > 0 ? 4500 : 2800);
  }

  const firstPendingReview = () => S.reviews.find((r) => r.status === 'pending');
  const pendingReviewCount = () => S.reviews.filter((r) => r.status === 'pending').length;
  const inboxPendingCount = () => S.inbox.filter((i) => i.state === 'unread' || i.state === 'reading').length;
  const getPage = (id) => S.pages[id] || S.pages['decision-reconnect'];
  const getRaw = (id) => S.rawFiles[id] || Object.values(S.rawFiles)[0];
  const getInbox = (id) => S.inbox.find((i) => i.id === id) || S.inbox[0];

  function findCitation(cid) {
    for (const p of Object.values(S.pages)) {
      const hit = (p.citations || []).find((c) => c.id === cid);
      if (hit) return hit;
    }
    return null;
  }

  function currentViewMode() {
    if (S.activeWorkspace === 'organize') return 'organize';
    if (S.activeWorkspace === 'graph') return 'graph';
    if (S.activeDocTab.startsWith('review:')) return 'review';
    if (S.activeDocTab.startsWith('inbox:')) return 'inbox';
    if (S.activeDocTab.startsWith('raw:')) return 'raw';
    return 'library';
  }

  function isRightOpen() {
    return Boolean(S.rightOpenByView[currentViewMode()]);
  }

  function setRightOpen(val) {
    const mode = currentViewMode();
    S.rightOpenByView[mode] = val;
    if (val && window.innerWidth < 1240 && mode === 'library') {
      S.evidenceByPage[S.activePageId] = null;
    }
  }

  function switchWorkspace(ws) {
    S.activeWorkspace = ws;
    if (ws !== 'docs') S.recallCurtainPageId = null;
    render();
  }

  function openDocTab(tab) {
    S.activeWorkspace = 'docs';
    if (!S.openDocTabs.some((t) => t.id === tab.id)) {
      S.openDocTabs.push(tab);
    }
    S.activeDocTab = tab.id;
    if (tab.kind === 'page') {
      const pid = tab.id.replace('page:', '');
      // Carbon R2 guardrail: isolate recall curtain per pageId so switching notes never leaks mask!
      if (S.recallCurtainPageId && S.recallCurtainPageId !== pid) {
        S.recallCurtainPageId = null;
        S.chatMode = 'query';
      }
      S.docLeftMode = 'library';
      S.activePageId = pid;
    } else if (tab.kind === 'raw') {
      S.recallCurtainPageId = null;
      S.docLeftMode = 'library';
      S.activeRawId = tab.id.replace('raw:', '');
    } else if (tab.kind === 'inbox') {
      S.recallCurtainPageId = null;
      S.docLeftMode = 'inbox';
      S.selectedInboxId = tab.id.replace('inbox:', '');
    } else if (tab.kind === 'review') {
      S.recallCurtainPageId = null;
      S.rightOpenByView.review = false;
    }
    render();
  }

  function closeDocTab(tabId) {
    const idx = S.openDocTabs.findIndex((t) => t.id === tabId);
    if (idx === -1) return;
    S.openDocTabs.splice(idx, 1);
    if (S.openDocTabs.length === 0) {
      openPage('decision-reconnect');
      return;
    }
    if (S.activeDocTab === tabId) {
      openDocTab(S.openDocTabs[Math.max(0, idx - 1)]);
    } else {
      render();
    }
  }

  function openPage(pageId) {
    const p = getPage(pageId);
    openDocTab({ id: `page:${p.id}`, label: p.title, kind: 'page' });
  }

  function openRaw(rawId) {
    const r = getRaw(rawId);
    openDocTab({ id: `raw:${r.id}`, label: `原文：${r.title.slice(0, 14)}`, kind: 'raw' });
  }

  function openInboxItem(itemId) {
    const item = getInbox(itemId);
    openDocTab({ id: `inbox:${item.id}`, label: item.title, kind: 'inbox' });
  }

  function openReview(reviewId) {
    const r = S.reviews.find((x) => x.id === reviewId) || firstPendingReview() || S.reviews[0];
    openDocTab({ id: `review:${r.id}`, label: `对比确认 · ${r.title.slice(0, 12)}`, kind: 'review' });
  }

  // Non-sliding 60s countdown buffer (Carbon P0-2 + R2 contenteditable guardrail)
  let bufferInterval = null;
  function startBufferCountdownIfIdle(noteText) {
    const id = `buf-${Date.now()}`;
    S.bufferNotes.push({ id, title: noteText.slice(0, 22), text: noteText });
    // Also save immediately into rawFiles so user sees their own note in Knowledge Base right away!
    const rawId = `raw-note-${Date.now()}`;
    S.rawFiles[rawId] = {
      id: rawId,
      path: `raw/notes/2026/11/20261126-new-note.md`,
      title: noteText.slice(0, 22),
      created: '刚刚',
      kind: 'note',
      kindLabel: '随手笔记',
      url: '',
      linkedPageIds: [],
      content: noteText
    };
    if (S.bufferSecondsLeft <= 0) {
      S.bufferSecondsLeft = S.settings.bufferSeconds;
      if (bufferInterval) clearInterval(bufferInterval);
      bufferInterval = setInterval(() => {
        if (S.bufferSecondsLeft > 1) {
          S.bufferSecondsLeft -= 1;
          updateTimerLabels();
        } else {
          clearInterval(bufferInterval);
          bufferInterval = null;
          flushBufferCompile(true);
        }
      }, 1000);
    }
  }

  function updateTimerLabels() {
    document.querySelectorAll('[data-buffer-timer]').forEach((el) => {
      el.textContent = formatSec(S.bufferSecondsLeft);
    });
  }

  function formatSec(sec) {
    return `${Math.floor(sec / 60)}:${String(sec % 60).padStart(2, '0')}`;
  }

  function flushBufferCompile(fromBackgroundTimer = false) {
    if (bufferInterval) {
      clearInterval(bufferInterval);
      bufferInterval = null;
    }
    const count = S.bufferNotes.length || 1;
    const label = S.bufferNotes.map((n) => n.title).join('、') || '新随手笔记';
    S.bufferSecondsLeft = 0;
    S.bufferNotes = [];
    const newId = `cs-0${143 + S.changesets.length}`;
    S.changesets.unshift({
      id: newId,
      time: '刚刚',
      trigger: `整理了 ${count} 条随手笔记（${label}）`,
      modelAlias: S.settings.modelAlias,
      status: 'applied',
      summary: '更新 2 篇知识页 · 原文出处已核对',
      pageIds: ['concept-event-id', 'decision-reconnect'],
      ops: [
        { label: '补充内容', target: '服务端确认事件编号（event_id）', outcome: '已更新' },
        { label: '补充关联', target: '流式断线恢复决策', outcome: '已更新' }
      ]
    });

    showToast('AI 已整理更新 2 篇知识页', [
      { label: '查看改动', onClick: () => switchWorkspace('organize') },
      { label: '撤销', onClick: () => revertChangeset(newId) }
    ]);

    // Guardrail: never wipe active INPUT / TEXTAREA / contenteditable focus on background timer tick!
    const activeEl = document.activeElement;
    const isTyping = activeEl && (activeEl.tagName === 'INPUT' || activeEl.tagName === 'TEXTAREA' || activeEl.isContentEditable);
    if (fromBackgroundTimer && isTyping) {
      const slot = $('#status-buffer-slot');
      if (slot) slot.innerHTML = '';
      const banner = $('#ops-buffer-banner');
      if (banner) banner.remove();
      return;
    }
    render();
  }

  function revertChangeset(csId) {
    const cs = S.changesets.find((c) => c.id === csId);
    if (!cs || cs.status === 'reverted') return;
    cs.status = 'reverted';
    showToast('已撤销该次整理（你手动写的内容保持不动）');
    render();
  }

  // Real cross-view search across Wiki pages, Original Raw files, AND verbatim quotes!
  function runGlobalSearch(q) {
    const needle = q.trim().toLowerCase();
    if (!needle) return [];
    const hits = [];
    for (const p of Object.values(S.pages)) {
      if (p.title.toLowerCase().includes(needle) || p.tags.some((t) => t.toLowerCase().includes(needle))) {
        hits.push({ kind: '知识页', title: p.title, sub: p.path, targetKind: 'page', targetId: p.id });
      }
      for (const c of p.citations || []) {
        if (c.quote.toLowerCase().includes(needle)) {
          hits.push({ kind: '原文引文', title: `${p.title} · ${c.label}`, sub: `“${c.quote}”`, targetKind: 'page', targetId: p.id, citationId: c.id });
        }
      }
    }
    for (const r of Object.values(S.rawFiles)) {
      if (r.title.toLowerCase().includes(needle) || r.content.toLowerCase().includes(needle)) {
        hits.push({ kind: r.kindLabel, title: r.title, sub: r.path, targetKind: 'raw', targetId: r.id });
      }
    }
    return hits.slice(0, 7);
  }

  function isLoopbackUrl(url) {
    return /^https?:\/\/(127\.0\.0\.1|localhost|\[::1\])(?::\d+)?(\/|$)/i.test(url.trim());
  }

  // ==========================================================================
  // Main Render
  // ==========================================================================
  function render() {
    const activeId = document.activeElement?.id;
    const activeSel = activeId === 'global-search-input' ? document.activeElement.selectionStart : null;

    const app = $('#app');
    const mode = currentViewMode();
    const rightOpen = isRightOpen();
    const isNarrow = window.innerWidth < 1240;
    const isDark = (document.body.dataset.theme || 'slate') === 'slate';

    const showLeft = S.activeWorkspace === 'docs' && mode !== 'review' && !(isNarrow && rightOpen);
    const showRight = rightOpen;

    const pReviews = pendingReviewCount();
    const pInbox = inboxPendingCount();
    const nextPending = firstPendingReview();
    const searchHits = S.searchOpen && S.searchQuery.trim() ? runGlobalSearch(S.searchQuery) : [];
    const wikiCount = Object.keys(S.pages).length;
    const rawCount = Object.keys(S.rawFiles).length;

    app.innerHTML = `
      <header class="topbar">
        <div class="brand-mark">
          <div class="brand-glyph">${icon('bolt', 'sm')}</div>
          <span class="brand-title">Wikra</span>
          <button type="button" class="vault-chip" data-action="open-onboard" title="切换知识库或打开已有 Obsidian 目录">${esc(S.vaultName)}</button>
        </div>

        <div class="drag-spacer" title="按住此处拖动桌面窗口"></div>

        <div class="global-search-wrap">
          <div class="global-search" role="search">
            ${icon('search', 'sm')}
            <input type="text" id="global-search-input" value="${esc(S.searchQuery)}" placeholder="搜索知识页、原文或直接提问…" aria-label="全局搜索">
            <kbd>Ctrl+K</kbd>
          </div>
          ${S.searchOpen && S.searchQuery.trim() ? `
            <div class="search-popover" role="listbox" aria-label="全局搜索结果">
              ${searchHits.map((h) => `
                <button type="button" class="search-hit" data-search-kind="${esc(h.targetKind)}" data-search-id="${esc(h.targetId)}" data-search-cit="${esc(h.citationId || '')}">
                  <div style="display:flex;align-items:center;gap:6px;">
                    <span class="pill ${h.kind === '原文引文' ? 'accent' : ''}">${esc(h.kind)}</span>
                    <strong style="font-size:12.5px;">${esc(h.title)}</strong>
                  </div>
                  <span style="font-size:11.5px;color:var(--muted);overflow:hidden;text-overflow:ellipsis;white-space:nowrap;">${esc(h.sub)}</span>
                </button>
              `).join('') || '<div style="padding:6px 8px;font-size:12px;color:var(--muted);">未找到本地匹配内容</div>'}
              <button type="button" class="search-hit" data-action="ask-from-search" style="border-top:1px solid var(--line-subtle);margin-top:2px;padding-top:8px;">
                <strong style="font-size:12.5px;color:var(--accent);">直接问 AI：「${esc(S.searchQuery.trim())}」 →</strong>
              </button>
            </div>
          ` : ''}
        </div>

        <div class="drag-spacer" title="按住此处拖动桌面窗口"></div>

        <div class="topbar-actions">
          <button type="button" class="btn primary sm" data-action="open-hud" title="随时新建笔记 (Win+Alt+N)">
            ${icon('plus', 'sm')} <span>随手记</span> <kbd class="hud-kbd-inline">Win+Alt+N</kbd>
          </button>
          ${pReviews > 0 && nextPending ? `
            <button type="button" class="btn sm" data-action="open-review" data-id="${esc(nextPending.id)}" style="border-color:var(--warning);color:var(--warning);" title="有观点冲突等待你确认">
              ${icon('conflict', 'sm')} <span>有冲突 (${pReviews})</span>
            </button>
          ` : ''}
          <button type="button" class="btn sm ${showRight ? 'active-toggle' : 'quiet'}" data-action="toggle-right" aria-pressed="${showRight}" title="显示/隐藏右侧栏 (Ctrl+J)">
            ${icon('sidebar', 'sm')} <span>侧栏</span>
          </button>
          <div class="win-controls" aria-label="窗口控制">
            <button type="button" class="win-btn" title="最小化到任务栏" data-action="win-minimize">${icon('minimize', 'sm')}</button>
            <button type="button" class="win-btn" title="最大化 / 还原" data-action="toast" data-msg="切换窗口最大化状态">${icon('maximize', 'sm')}</button>
            <button type="button" class="win-btn close" title="隐藏到系统托盘" data-action="toast" data-msg="已隐藏至系统托盘，按 Win+Alt+N 可随时记录">${icon('close', 'sm')}</button>
          </div>
        </div>
      </header>

      <div class="workspace-body">
        <nav class="nav-rail" aria-label="主导航">
          <div class="nav-group">
            <button type="button" class="nav-item ${S.activeWorkspace === 'docs' && S.docLeftMode === 'library' ? 'active' : ''}" data-nav="library" title="知识库（浏览知识页与全部原文）">
              ${icon('book', 'lg')}
              <span class="nav-label">知识库</span>
            </button>
            <button type="button" class="nav-item ${S.activeWorkspace === 'docs' && S.docLeftMode === 'inbox' ? 'active' : ''}" data-nav="inbox" title="收件箱（阅读收藏的文章网页并提炼重点）">
              ${icon('inbox', 'lg')}
              <span class="nav-label">收件箱</span>
              ${pInbox > 0 ? `<span class="nav-badge">${pInbox}</span>` : ''}
            </button>
            <button type="button" class="nav-item ${S.activeWorkspace === 'organize' ? 'active' : ''}" data-nav="organize" title="整理（查看 AI 整理记录与需要你确认的冲突）">
              ${icon('sparkles', 'lg')}
              <span class="nav-label">整理</span>
              ${pReviews > 0 ? `<span class="nav-badge warn">${pReviews}</span>` : ''}
            </button>
            <button type="button" class="nav-item ${S.activeWorkspace === 'graph' ? 'active' : ''}" data-nav="graph" title="知识图谱 (Ctrl+G)">
              ${icon('graph', 'lg')}
              <span class="nav-label">图谱</span>
            </button>
          </div>
          <div class="nav-group">
            <button type="button" class="nav-item" data-action="toggle-theme" title="${isDark ? '切换为浅色主题（Paper Daylight）' : '切换为深色主题（Cognitive Slate）'}">
              ${icon(isDark ? 'sun' : 'moon', 'lg')}
              <span class="nav-label">${isDark ? '浅色' : '深色'}</span>
            </button>
            <button type="button" class="nav-item" data-action="open-settings" title="设置（模型端点、OpenViking、外观与目录）">
              ${icon('settings', 'lg')}
              <span class="nav-label">设置</span>
            </button>
          </div>
        </nav>

        ${showLeft ? renderLeftPane() : ''}

        <main class="center-stage">
          ${S.activeWorkspace === 'docs' ? `
            <div class="tab-strip" role="tablist" aria-label="已打开文档标签">
              ${S.openDocTabs.map((t) => `
                <div class="doc-tab ${S.activeDocTab === t.id ? 'active' : ''}" role="tab" aria-selected="${S.activeDocTab === t.id}" data-tab-id="${esc(t.id)}" title="${esc(t.label)}">
                  <span class="tab-label">${esc(t.label)}</span>
                  <button type="button" class="tab-close" data-close-tab="${esc(t.id)}" aria-label="关闭标签">&times;</button>
                </div>
              `).join('')}
            </div>
          ` : ''}
          ${renderCenterContent()}
        </main>

        ${showRight ? renderRightPane() : ''}
      </div>

      <footer class="statusbar">
        <div class="status-group">
          <span>本地库：${esc(S.vaultName)}（${wikiCount} 篇知识 · ${rawCount} 篇原文）</span>
        </div>
        <div class="status-group">
          <span id="status-buffer-slot" class="status-slot-buffer">
            ${S.bufferSecondsLeft > 0 ? `
              <span>待整理（${S.bufferNotes.length} 条）· <strong data-buffer-timer>${formatSec(S.bufferSecondsLeft)}</strong></span>
              <button type="button" class="btn sm quiet" data-action="open-buffer-queue" style="min-height:20px;padding:1px 6px;">查看/撤回</button>
              <button type="button" class="btn sm quiet" data-action="flush-buffer" style="min-height:20px;padding:1px 6px;color:var(--accent);">立即整理</button>
            ` : ''}
          </span>
          <span><span class="status-dot"></span>AI：${esc(S.settings.modelAlias)}</span>
          <span><span class="status-dot ${S.settings.memoryProvider === 'none' ? 'off' : ''}"></span>记忆：${S.settings.memoryProvider === 'openviking' ? 'OpenViking 本地' : '已停用'}</span>
        </div>
      </footer>
    `;

    bindEvents();

    if (activeId === 'global-search-input') {
      const inp = $('#global-search-input');
      if (inp) {
        inp.focus();
        if (activeSel !== null) inp.setSelectionRange(activeSel, activeSel);
      }
    }
  }

  // ==========================================================================
  // Left Sidebar: Unified Library (Wiki + Original Raw Files!) OR Inbox List
  // ==========================================================================
  function renderLeftPane() {
    if (S.docLeftMode === 'inbox') {
      const filtered = S.inbox.filter((item) => {
        if (S.inboxFilter === 'pending') return item.state === 'unread' || item.state === 'reading';
        if (S.inboxFilter === 'archived') return item.state === 'compiled' || item.state === 'archived';
        return true;
      });
      const singlePill = (item) => {
        if (item.fetchStatus === 'stub') return '<span class="pill warning">待补全</span>';
        if (item.state === 'compiled') return '<span class="pill success">已提炼</span>';
        if (item.state === 'archived') return '<span class="pill">已存档</span>';
        if (item.state === 'reading') return '<span class="pill accent">阅读中</span>';
        return '<span class="pill">未读</span>';
      };

      return `
        <aside class="left-pane" aria-label="收件箱文章列表">
          <div class="pane-header">
            <div class="segmented" role="tablist">
              <button type="button" class="${S.inboxFilter === 'pending' ? 'active' : ''}" data-inbox-filter="pending">未读</button>
              <button type="button" class="${S.inboxFilter === 'archived' ? 'active' : ''}" data-inbox-filter="archived">已处理</button>
              <button type="button" class="${S.inboxFilter === 'all' ? 'active' : ''}" data-inbox-filter="all">全部</button>
            </div>
          </div>
          <div class="pane-scroll">
            ${filtered.map((item) => `
              <button type="button" class="list-item ${S.activeDocTab === `inbox:${item.id}` ? 'active' : ''}" data-open-inbox="${item.id}">
                <div class="list-item-row">
                  <span class="list-item-title">${esc(item.title)}</span>
                </div>
                <div class="list-item-row">
                  <span class="list-item-sub">${esc(item.domain || item.sourceType)} · ${esc(item.savedAt)}</span>
                  ${singlePill(item)}
                </div>
              </button>
            `).join('') || '<p style="padding:12px 6px;font-size:12px;color:var(--muted);">当前分组暂无文章</p>'}
          </div>
        </aside>
      `;
    }

    const pages = Object.values(S.pages);
    const raws = Object.values(S.rawFiles);
    const showWiki = S.libraryFilter === 'all' || S.libraryFilter === 'wiki';
    const showRaw = S.libraryFilter === 'all' || S.libraryFilter === 'raw';

    return `
      <aside class="left-pane" aria-label="知识库文件目录">
        <div class="pane-header">
          <div class="segmented" role="tablist">
            <button type="button" class="${S.libraryFilter === 'all' ? 'active' : ''}" data-lib-filter="all">全部</button>
            <button type="button" class="${S.libraryFilter === 'wiki' ? 'active' : ''}" data-lib-filter="wiki">知识页</button>
            <button type="button" class="${S.libraryFilter === 'raw' ? 'active' : ''}" data-lib-filter="raw">原文</button>
          </div>
          <button type="button" class="icon-btn" title="新建空白笔记" data-action="toast" data-msg="已新建空白笔记">${icon('plus', 'sm')}</button>
        </div>
        <div class="pane-scroll">
          ${showWiki ? `
            <div class="tree-group">
              <div class="tree-group-head">
                <span>整理后的知识（wiki/）</span>
                <span>${pages.length}</span>
              </div>
              ${pages.map((p) => `
                <button type="button" class="list-item ${S.activeDocTab === `page:${p.id}` ? 'active' : ''}" data-open-page="${p.id}">
                  <div class="list-item-row">
                    <span class="list-item-title">${esc(p.title)}</span>
                    ${p.pendingConflicts.length > 0 ? '<span class="pill warning">有冲突</span>' : ''}
                  </div>
                  <div class="list-item-row">
                    <span class="list-item-sub">${esc(p.typeLabel)} · ${esc(p.updated.slice(5))}</span>
                    <span class="list-item-sub">${p.citations.length} 处原文</span>
                  </div>
                </button>
              `).join('')}
            </div>
          ` : ''}

          ${showRaw ? `
            <div class="tree-group">
              <div class="tree-group-head">
                <span>原始文件（raw/）</span>
                <span>${raws.length}</span>
              </div>
              ${raws.map((r) => `
                <button type="button" class="list-item ${S.activeDocTab === `raw:${r.id}` ? 'active' : ''}" data-open-raw="${r.id}">
                  <div class="list-item-row">
                    <span class="list-item-title">${esc(r.title)}</span>
                    <span class="pill">${esc(r.kindLabel)}</span>
                  </div>
                  <div class="list-item-row">
                    <span class="list-item-sub">${esc(r.created)}</span>
                    <span class="list-item-sub">${r.linkedPageIds.length > 0 ? `已整理 ${r.linkedPageIds.length} 页` : '未整理'}</span>
                  </div>
                </button>
              `).join('')}
            </div>
          ` : ''}
        </div>
      </aside>
    `;
  }

  function renderCenterContent() {
    if (S.activeWorkspace === 'organize') return renderOrganizeView();
    if (S.activeWorkspace === 'graph') return renderGraphView();
    if (S.activeDocTab.startsWith('review:')) return renderConflictReview(S.activeDocTab.replace('review:', ''));
    if (S.activeDocTab.startsWith('inbox:')) return renderInboxView(S.activeDocTab.replace('inbox:', ''));
    if (S.activeDocTab.startsWith('raw:')) return renderRawViewer(S.activeDocTab.replace('raw:', ''));
    return renderWikiEditor(S.activeDocTab.replace('page:', ''));
  }

  // ==========================================================================
  // View 1 (Default): Wiki Document Viewer / Editor + 1-Click Split Original!
  // ==========================================================================
  function renderWikiEditor(pageId) {
    const p = getPage(pageId);
    const evCid = S.evidenceByPage[p.id] || null;
    const evCit = evCid ? findCitation(evCid) : null;
    const evRaw = evCit ? S.rawFiles[evCit.rawId] : null;
    const hasPending = p.pendingConflicts.length > 0;
    const curtainOn = S.recallCurtainPageId === p.id;
    const isFullWidth = (document.body.dataset.width || 'readable') === 'full';

    const rawMarkdown = [
      '---',
      `type: ${p.type}`,
      `aliases: [${p.aliases.join(', ')}]`,
      `tags: [${p.tags.join(', ')}]`,
      `updated: ${p.updated}`,
      `wikra-core: ${p.fmdCore}`,
      ...(hasPending ? ['wikra_pending_conflicts:', '  - "新旧断线恢复策略存在分歧（待确认）"'] : []),
      '---',
      '',
      `# ${p.title}`,
      '',
      ...p.sections.flatMap((s) => [`## ${s.heading}`, '', `${s.body}${s.citationIds.map((id) => { const c = p.citations.find((x) => x.id === id); return c ? `[^${c.ref}]` : ''; }).join('')}`, '']),
      ...(p.humanUnderstanding ? ['<!-- wikra-protect:start -->', '## 我的理解', '', p.humanUnderstanding, '<!-- wikra-protect:end -->', ''] : []),
      ...p.citations.map((c) => `[^${c.ref}]: [[${c.anchor}|${c.label}]] —— “${c.quote}”`)
    ].join('\n');

    return `
      <div class="stage-body flush">
        <div class="editor-shell ${evRaw ? 'with-evidence' : ''}">
          <div class="editor-main">
            <div class="doc-toolbar">
              <div class="breadcrumb" title="${esc(p.path)}">${esc(p.path)}</div>
              <div style="display:flex;align-items:center;gap:5px;flex:none;">
                <button type="button" class="btn sm ${S.editorMode === 'preview' ? 'primary' : ''}" data-editor-mode="preview" title="切换实时预览与源码 (Ctrl+E)">阅读</button>
                <button type="button" class="btn sm ${S.editorMode === 'source' ? 'primary' : ''}" data-editor-mode="source" title="编辑原始 Markdown 源码">源码</button>
                ${p.citations.length > 0 ? `
                  <button type="button" class="btn sm ${evRaw ? 'active-toggle' : ''}" data-action="toggle-split-origin" title="右侧并排打开来源原文">
                    ${icon('split', 'sm')} <span>对照原文</span>
                  </button>
                ` : ''}
                <button type="button" class="btn quiet sm ${isFullWidth ? 'active-toggle' : ''}" data-action="toggle-width" title="切换适读宽度（920px）或铺满整栏">
                  ${icon('width', 'sm')} <span>${isFullWidth ? '适读宽' : '铺满宽'}</span>
                </button>
                <button type="button" class="btn quiet sm" data-action="start-teachback" title="遮住正文用自己的话复述一遍">让我讲一遍</button>
                <button type="button" class="btn quiet sm" data-action="sim-obsidian" title="用 Obsidian 打开同一本地文件">${icon('external', 'sm')} Obsidian</button>
              </div>
            </div>

            <div class="doc-scroll">
              <article class="doc-paper">
                <div class="doc-title-row">
                  <h1>${esc(p.title)}</h1>
                  <span class="mono" style="font-size:11.5px;color:var(--muted);">更新于 ${esc(p.updated.slice(5))}</span>
                </div>

                ${S.externalChangedBanner ? `
                  <div class="external-reload-bar">
                    <span>检测到 Obsidian 已修改本地文件，是否重新加载最新内容？</span>
                    <div style="display:flex;gap:6px;">
                      <button type="button" class="btn sm" data-action="dismiss-external">重新加载</button>
                      <button type="button" class="btn quiet sm" data-action="dismiss-external">保留当前</button>
                    </div>
                  </div>
                ` : ''}

                ${S.editorMode === 'source' ? `
                  <textarea class="source-editor" aria-label="Markdown 源码编辑器">${esc(rawMarkdown)}</textarea>
                  <p style="margin-top:8px;font-size:12px;color:var(--muted);">100% 兼容 Obsidian 本地 Markdown 格式，保留所有自定义标签与插件语法块。</p>
                ` : `
                  <div class="props-summary-bar">
                    <div style="display:flex;flex-wrap:wrap;align-items:center;gap:6px;">
                      <span class="pill ${p.fmdCore ? 'accent' : ''}">${esc(p.typeLabel)}</span>
                      ${p.tags.map((t) => `<span class="pill">#${esc(t)}</span>`).join('')}
                      <span style="color:var(--muted);">参考了 ${p.citations.length} 处原文</span>
                    </div>
                    <button type="button" class="btn quiet sm" data-action="toggle-props" style="min-height:22px;padding:1px 6px;">${S.propsExpanded ? '收起信息 ▴' : '属性 ▾'}</button>
                  </div>

                  ${S.propsExpanded ? `
                    <section class="props-expanded" aria-label="文档属性详情">
                      <div class="fm-grid">
                        <span class="fm-key">页面分类</span><div class="fm-val">${esc(p.typeLabel)}</div>
                        <span class="fm-key">别名</span><div class="fm-val">${p.aliases.join('、') || '无'}</div>
                        <span class="fm-key">AI 更新规则</span><div class="fm-val">${p.fmdCore ? '核心页：涉及结论变化时先请你确认' : '常规页：后台自动增量补充，随时可撤销'}</div>
                      </div>
                    </section>
                  ` : ''}

                  ${hasPending ? `
                    <div class="single-conflict-banner" role="note">
                      <div>
                        <strong style="color:var(--warning);">有 1 处观点冲突：</strong>
                        <span>新收集的协议规范与你之前的旧结论不一致，确认一下以哪条为准？</span>
                      </div>
                      <button type="button" class="btn sm" data-action="open-review" data-id="${esc(p.pendingConflicts[0])}" style="flex:none;">去对比确认</button>
                    </div>
                  ` : ''}

                  ${curtainOn ? `
                    <div class="recall-curtain-banner">
                      <span><strong>主动回忆中：</strong>正文和原文已遮住，先在右侧用自己的话讲一遍，再展开核对。</span>
                      <button type="button" class="btn sm" data-action="exit-curtain">展开全部核对</button>
                    </div>
                  ` : ''}

                  <div class="prose">
                    ${p.sections.map((sec, sIdx) => {
                      const masked = curtainOn && !S.recallPeekSections[sIdx];
                      return `
                        <h2>${esc(sec.heading)}</h2>
                        ${masked ? `
                          <div class="masked-body">
                            <span>正文结论已遮蔽 —— 先在右侧试着复述这一节的核心要点</span>
                            <button type="button" class="btn sm" data-peek-section="${sIdx}">展开核对</button>
                          </div>
                        ` : `
                          <p>
                            ${esc(sec.body)}
                            ${sec.citationIds.map((cid) => {
                              const c = p.citations.find((x) => x.id === cid);
                              return c ? `<button type="button" class="footnote-ref" data-open-evidence="${c.id}" title="点击右侧分屏对照原文">[^${c.ref}]</button>` : '';
                            }).join('')}
                          </p>
                          ${sec.links.length > 0 ? `
                            <p style="font-size:13px;color:var(--text-secondary);">相关笔记：${sec.links.map((lid) => {
                              const target = S.pages[lid];
                              return target ? `<button type="button" class="wikilink" data-open-page="${lid}">[[${esc(target.title)}]]</button>` : '';
                            }).filter(Boolean).join(' · ')}</p>
                          ` : ''}
                        `}
                      `;
                    }).join('')}

                    ${p.tasks.length > 0 ? `
                      <h2>待办清单</h2>
                      <ul style="padding-left:0;">
                        ${p.tasks.map((t) => `
                          <li class="task-row ${t.done ? 'done' : ''}">
                            <input type="checkbox" ${t.done ? 'checked' : ''} data-toggle-task="${p.id}:${t.id}" id="task-${t.id}">
                            <label for="task-${t.id}" style="cursor:pointer;"><span>${esc(t.text)}</span></label>
                          </li>
                        `).join('')}
                      </ul>
                    ` : ''}

                    ${p.humanUnderstanding ? `
                      <section class="human-section">
                        <div style="display:flex;align-items:center;justify-content:space-between;margin-bottom:6px;">
                          <strong style="font-size:14.5px;">我的理解</strong>
                          <span class="pill success" title="此段落由你亲自维护，AI 后台整理不会改动它">${icon('shield', 'sm')} 手动保护</span>
                        </div>
                        ${curtainOn && !S.recallPeekSections.human ? `
                          <div class="masked-body" style="margin-bottom:0;">
                            <span>「我的理解」已同步遮蔽 —— 复述后再展开核对或追加新想法</span>
                            <button type="button" class="btn sm" data-peek-section="human">展开核对</button>
                          </div>
                        ` : `<p style="margin:0;">${esc(p.humanUnderstanding)}</p>`}
                      </section>
                    ` : ''}

                    ${p.citations.length > 0 ? `
                      <section class="footnotes-block" aria-label="参考原文出处">
                        <span class="kicker">参考原文出处（点击可分屏对照或直接打开完整原文）</span>
                        ${curtainOn && !S.recallPeekSections.footnotes ? `
                          <div class="masked-body" style="margin-bottom:0;">
                            <span>主动回忆期间已折起 ${p.citations.length} 处原文引用，避免提前看到答案</span>
                            <button type="button" class="btn sm" data-peek-section="footnotes">展开引文</button>
                          </div>
                        ` : p.citations.map((c) => `
                          <div class="footnote-item">
                            <div style="min-width:0;">
                              <span class="mono" style="color:var(--accent);">[^${c.ref}]</span>
                              <strong>${esc(c.label)}</strong>
                              <div class="footnote-quote">“${esc(c.quote)}”</div>
                            </div>
                            <div style="display:flex;gap:6px;flex:none;">
                              <button type="button" class="btn sm" data-open-evidence="${c.id}">分屏对照</button>
                              <button type="button" class="btn quiet sm" data-open-raw="${c.rawId}">完整原文</button>
                            </div>
                          </div>
                        `).join('')}
                      </section>
                    ` : ''}
                  </div>
                `}
              </article>
            </div>
          </div>

          ${evRaw && evCit ? `
            <aside class="evidence-pane" aria-label="原文对照分屏">
              <div class="pane-header">
                <div style="min-width:0;">
                  <div style="font-size:12.5px;font-weight:650;">对照原文 · ${esc(evRaw.kindLabel)}</div>
                  <div class="mono" style="font-size:11px;color:var(--muted);overflow:hidden;text-overflow:ellipsis;white-space:nowrap;">${esc(evRaw.path)}</div>
                </div>
                <div style="display:flex;align-items:center;gap:4px;">
                  <button type="button" class="btn quiet sm" data-open-raw="${evRaw.id}">全屏打开</button>
                  <button type="button" class="icon-btn" data-action="close-evidence" aria-label="关闭原文分屏">&times;</button>
                </div>
              </div>
              <div class="evidence-body">
                <div style="display:flex;align-items:center;justify-content:space-between;gap:8px;margin-bottom:10px;">
                  <div style="display:flex;gap:6px;">
                    <span class="pill">只读原件</span>
                    <span class="pill">${esc(evRaw.created)}</span>
                  </div>
                  ${evRaw.url ? `<button type="button" class="btn sm" data-open-url="${esc(evRaw.url)}">${icon('external', 'sm')} 打开原网页</button>` : ''}
                </div>
                <div class="kicker">引用片段</div>
                <div class="quote-highlight">“${esc(evCit.quote)}”</div>
                <div class="kicker" style="margin-top:14px;">原文完整内容</div>
                <pre style="white-space:pre-wrap;font-size:13px;line-height:1.72;margin-top:8px;color:var(--text-secondary);font-family:inherit;">${esc(evRaw.content)}</pre>
              </div>
            </aside>
          ` : ''}
        </div>
      </div>
    `;
  }

  // ==========================================================================
  // View 1B: Direct Original Raw File Viewer inside Knowledge Base!
  // ==========================================================================
  function renderRawViewer(rawId) {
    const r = getRaw(rawId);
    const isFullWidth = (document.body.dataset.width || 'readable') === 'full';
    const blocks = r.content.split(/\n\n+/).filter(Boolean);

    return `
      <div class="stage-body flush">
        <div class="doc-toolbar">
          <div class="breadcrumb" title="${esc(r.path)}">${esc(r.path)}</div>
          <div style="display:flex;align-items:center;gap:6px;flex:none;">
            ${r.url ? `<button type="button" class="btn sm" data-open-url="${esc(r.url)}">${icon('external', 'sm')} 打开原网页</button>` : ''}
            <button type="button" class="btn quiet sm ${isFullWidth ? 'active-toggle' : ''}" data-action="toggle-width" title="切换适读宽度或铺满整栏">
              ${icon('width', 'sm')} <span>${isFullWidth ? '适读宽' : '铺满宽'}</span>
            </button>
            <button type="button" class="btn sm" data-action="toast" data-msg="已加入队列，AI 将按最新内容重新整理关联知识页">重新整理</button>
          </div>
        </div>
        <div class="doc-scroll">
          <article class="doc-paper">
            <div class="doc-title-row">
              <h1>${esc(r.title)}</h1>
              <span class="mono" style="font-size:11.5px;color:var(--muted);">${esc(r.created)}</span>
            </div>

            <div class="raw-origin-banner">
              <div style="display:flex;flex-wrap:wrap;align-items:center;gap:8px;">
                <span class="pill accent">${esc(r.kindLabel)} · 原始原件</span>
                ${r.linkedPageIds.length > 0 ? `
                  <span style="color:var(--text-secondary);">已整理进知识页：${r.linkedPageIds.map((pid) => {
                    const pg = S.pages[pid];
                    return pg ? `<button type="button" class="wikilink" data-open-page="${pid}">[[${esc(pg.title)}]]</button>` : '';
                  }).filter(Boolean).join(' · ')}</span>
                ` : '<span style="color:var(--muted);">尚未整理进知识页</span>'}
              </div>
              <span style="font-size:11.5px;color:var(--muted);">原件只读保存，AI 整理绝不会改动原文</span>
            </div>

            <div class="prose">
              ${blocks.map((b) => b.startsWith('## ')
                ? `<h2>${esc(b.replace(/^##\s+/, ''))}</h2>`
                : `<p>${esc(b)}</p>`
              ).join('')}
            </div>
          </article>
        </div>
      </div>
    `;
  }

  // ==========================================================================
  // View 2: Readwise Reader-Style Inbox (Full Original Article + Highlight!)
  // ==========================================================================
  function renderInboxView(itemId) {
    const item = getInbox(itemId);
    const isFullWidth = (document.body.dataset.width || 'readable') === 'full';
    const highlightedPending = (item.paragraphs || []).filter((p) => p.highlighted && !p.compiled);
    const allHighlighted = (item.paragraphs || []).filter((p) => p.highlighted);
    const visibleParas = S.inboxReadMode === 'highlights' ? allHighlighted : (item.paragraphs || []);

    return `
      <div class="stage-body flush">
        <div class="inbox-capture-bar">
          <input type="text" id="inbox-url-input" placeholder="粘贴网页链接、长文，或拖入 .md / .txt 文件收入收件箱…" aria-label="收集网页链接或文本">
          <button type="button" class="btn primary sm" data-action="ingest-url">收集</button>
        </div>

        <div class="doc-toolbar">
          <div style="display:flex;align-items:center;gap:8px;min-width:0;">
            <span class="pill">${esc(item.sourceType)}</span>
            <span class="breadcrumb">${esc(item.url || '本地粘贴文档')}</span>
          </div>
          <div style="display:flex;align-items:center;gap:6px;flex:none;">
            ${item.url ? `<button type="button" class="btn sm" data-open-url="${esc(item.url)}">${icon('external', 'sm')} 打开原网页</button>` : ''}
            ${item.fetchStatus === 'full' ? `
              <button type="button" class="btn sm ${S.inboxReadMode === 'full' ? 'primary' : ''}" data-inbox-readmode="full">完整原文</button>
              <button type="button" class="btn sm ${S.inboxReadMode === 'highlights' ? 'primary' : ''}" data-inbox-readmode="highlights">已标重点 (${allHighlighted.length})</button>
            ` : ''}
            <button type="button" class="btn quiet sm ${isFullWidth ? 'active-toggle' : ''}" data-action="toggle-width" title="切换适读宽度或铺满整栏">
              ${icon('width', 'sm')} <span>${isFullWidth ? '适读宽' : '铺满宽'}</span>
            </button>
          </div>
        </div>

        <div class="doc-scroll">
          <article class="doc-paper">
            <header class="reader-head">
              <h1 style="font-size:23px;font-weight:700;letter-spacing:-0.02em;">${esc(item.title)}</h1>
              <div class="reader-meta">
                <span>${esc(item.domain || item.sourceType)}</span>
                <span>·</span>
                <span>收藏于 ${esc(item.savedAt)}</span>
                <span>·</span>
                <span>${esc(item.readTime)}</span>
              </div>

              ${item.fetchStatus === 'full' ? `
                <div class="reader-actions">
                  <div style="display:flex;flex-wrap:wrap;gap:8px;align-items:center;">
                    ${highlightedPending.length > 0 ? `
                      <button type="button" class="btn primary sm" data-triage="excerpt">提炼重点入库（${highlightedPending.length} 段）</button>
                      <button type="button" class="btn sm" data-triage="full">整篇提炼（约 ${item.wordCount || 300} 字）</button>
                    ` : `
                      <button type="button" class="btn primary sm" data-triage="full">整篇提炼入库（约 ${item.wordCount || 300} 字）</button>
                    `}
                    <button type="button" class="btn quiet sm" data-triage="archive">仅存档（保留原文）</button>
                  </div>
                  <span style="font-size:11.5px;color:var(--muted);">点击下方任意段落即可标为重点</span>
                </div>
              ` : ''}
            </header>

            ${item.fetchStatus === 'stub' ? `
              <section class="setting-card">
                <strong>该网页需要登录后查看，已为你保存链接</strong>
                <p style="font-size:12.5px;color:var(--text-secondary);">为了隐私安全，应用不会读取你的浏览器登录态。点上方「打开原网页」阅读后，把想保留的内容粘贴到下方即可存为本地原文：</p>
                <textarea id="stub-paste-area" class="source-editor" style="min-height:140px;" placeholder="粘贴文章正文或关键段落…">${esc(item.pastedBody || '')}</textarea>
                <div><button type="button" class="btn primary sm" data-complete-stub="${item.id}">保存为本地原文</button></div>
              </section>
            ` : `
              <div class="prose">
                ${visibleParas.map((para) => `
                  <div class="reader-para ${para.highlighted ? 'highlighted' : ''} ${para.compiled ? 'compiled' : ''}" data-toggle-para="${item.id}:${para.id}" role="button" tabindex="0">
                    <div class="reader-para-head">
                      ${para.heading ? `<h3>${esc(para.heading)}</h3>` : '<span></span>'}
                      <div style="display:flex;align-items:center;gap:6px;">
                        ${para.compiled ? '<span class="pill success">已提炼入库</span>' : para.highlighted ? '<span class="pill accent">已标重点</span>' : '<span class="reader-para-hint">点击标为重点</span>'}
                      </div>
                    </div>
                    <p>${esc(para.text)}</p>
                  </div>
                `).join('') || '<p style="color:var(--muted);font-size:13px;">还没有标出重点段落。切换回「完整原文」，点击你认可的段落即可高亮标记。</p>'}
              </div>
            `}
          </article>
        </div>
      </div>
    `;
  }

  // ==========================================================================
  // View 3: Simplified Plain-Chinese "整理" View (Fluid Wide-Screen 2-Column!)
  // ==========================================================================
  function renderOrganizeView() {
    const rev = S.reviews[0];
    const activeSuggs = S.suggestions.filter((s) => !s.dismissed);

    return `
      <div class="stage-body">
        <div class="organize-wrap">
          <section class="organize-header">
            <div>
              <h1>整理与同步</h1>
              <p>平时后台会自动把你的随手笔记和收藏文章整理进知识库；只有出现观点矛盾时，才会在这里请你确认。</p>
            </div>
            <div style="display:flex;gap:8px;">
              <button type="button" class="btn sm" data-nav="library">${icon('book', 'sm')} 回到知识库</button>
              <button type="button" class="btn primary sm" data-action="open-hud">${icon('plus', 'sm')} 记一条随手笔记</button>
            </div>
          </section>

          <div class="organize-cols">
            <!-- Left Column: What needs your decision + gentle optional suggestions -->
            <div class="org-col">
              <section class="ops-card ${rev.status === 'pending' ? 'priority' : ''}">
                <div class="ops-card-head">
                  <div class="ops-card-title">
                    ${icon('conflict')}
                    <span>需要你确认的冲突</span>
                    <span class="pill ${rev.status === 'pending' ? 'warning' : 'success'}">${rev.status === 'pending' ? '1 处待确认' : '已全部确认'}</span>
                  </div>
                </div>
                ${rev.status === 'pending' ? `
                  <div class="ops-row">
                    <div class="ops-row-main">
                      <span class="ops-row-title">${esc(rev.title)}</span>
                      <span class="ops-row-desc">${esc(rev.plainHint)}</span>
                    </div>
                    <div class="ops-row-actions">
                      <button type="button" class="btn primary sm" data-action="open-review" data-id="${esc(rev.id)}">左右对比确认</button>
                    </div>
                  </div>
                ` : `
                  <div class="ops-row">
                    <div class="ops-row-main">
                      <span class="ops-row-title">当前没有观点冲突，知识库保持一致</span>
                      <span class="ops-row-desc">上次确认：${esc(rev.resolution)}。</span>
                    </div>
                    <div class="ops-row-actions">
                      <button type="button" class="btn sm" data-open-page="decision-reconnect">查看笔记</button>
                    </div>
                  </div>
                `}
              </section>

              <section class="ops-card">
                <div class="ops-card-head">
                  <div class="ops-card-title">
                    ${icon('sparkles')}
                    <span>可选的复习与关联建议</span>
                  </div>
                  <span style="font-size:11.5px;color:var(--muted);">有空再看，不影响正常记录</span>
                </div>
                <div class="ops-list">
                  ${activeSuggs.map((s) => `
                    <div class="ops-row">
                      <div class="ops-row-main">
                        <div style="display:flex;align-items:center;gap:6px;">
                          <span class="pill ${s.actionType === 'teachback' ? 'accent' : ''}">${esc(s.kind)}</span>
                          <span class="ops-row-title">${esc(s.title)}</span>
                        </div>
                        <span class="ops-row-desc">${esc(s.detail)}</span>
                      </div>
                      <div class="ops-row-actions">
                        ${s.actionType === 'teachback'
                          ? `<button type="button" class="btn primary sm" data-action="start-teachback" data-page="${s.pageId}">${esc(s.actionLabel)}</button>`
                          : `<button type="button" class="btn sm" data-open-page="${s.pageId}">${esc(s.actionLabel)}</button>`}
                        <button type="button" class="btn quiet sm" data-dismiss-sugg="${s.id}">忽略</button>
                      </div>
                    </div>
                  `).join('') || '<p style="font-size:12.5px;color:var(--muted);">当前没有新的建议。</p>'}
                </div>
              </section>
            </div>

            <!-- Right Column: Pending 60s buffer + AI Recent Activity Timeline -->
            <div class="org-col">
              ${S.bufferSecondsLeft > 0 ? `
                <div id="ops-buffer-banner" class="buffer-banner">
                  <div>
                    <strong>有 ${S.bufferNotes.length} 条新笔记正在等待合并整理</strong>
                    <div style="font-size:12px;color:var(--text-secondary);margin-top:2px;">
                      将在 <strong class="mono" data-buffer-timer>${formatSec(S.bufferSecondsLeft)}</strong> 后自动开始，写错了可先移出。
                    </div>
                  </div>
                  <div style="display:flex;gap:6px;">
                    <button type="button" class="btn sm" data-action="open-buffer-queue">查看 / 撤回</button>
                    <button type="button" class="btn primary sm" data-action="flush-buffer">立即整理</button>
                  </div>
                </div>
              ` : ''}

              <section class="ops-card">
                <div class="ops-card-head">
                  <div class="ops-card-title">
                    ${icon('history')}
                    <span>AI 最近帮你整理了什么</span>
                  </div>
                  <span style="font-size:11.5px;color:var(--muted);">每一批都支持一键撤销</span>
                </div>
                <div class="ops-list">
                  ${S.changesets.map((cs) => `
                    <div class="cs-card">
                      <div style="display:flex;justify-content:space-between;align-items:center;gap:8px;">
                        <strong style="font-size:13px;">${esc(cs.summary)}</strong>
                        <span class="pill ${cs.status === 'applied' ? 'success' : cs.status === 'partial' ? 'warning' : ''}">
                          ${cs.status === 'applied' ? '已完成' : cs.status === 'partial' ? '含待确认项' : '已撤销'}
                        </span>
                      </div>
                      <div style="font-size:11.5px;color:var(--muted);">${esc(cs.time)} · ${esc(cs.trigger)}</div>
                      <div class="editop-list">
                        ${cs.ops.map((op) => `<div>${esc(op.label)} → ${esc(op.target)}（${esc(op.outcome)}）</div>`).join('')}
                      </div>
                      <div style="display:flex;gap:6px;">
                        ${cs.status !== 'reverted' ? `<button type="button" class="btn sm" data-revert-cs="${cs.id}">${icon('undo', 'sm')} 撤销此次整理</button>` : ''}
                        ${cs.status === 'partial' && firstPendingReview() ? `<button type="button" class="btn sm" data-action="open-review" data-id="${esc(firstPendingReview().id)}">去确认冲突</button>` : ''}
                      </div>
                    </div>
                  `).join('')}
                </div>
              </section>
            </div>
          </div>
        </div>
      </div>
    `;
  }

  // ==========================================================================
  // View 4: Plain-Chinese Side-by-Side Conflict Comparison (Fluid 1400px)
  // ==========================================================================
  function renderConflictReview(reviewId) {
    const rev = S.reviews.find((r) => r.id === reviewId) || S.reviews[0];
    return `
      <div class="stage-body">
        <div class="review-wrap">
          <div style="display:flex;flex-wrap:wrap;align-items:flex-start;justify-content:space-between;gap:12px;">
            <div>
              <div style="display:flex;align-items:center;gap:8px;margin-bottom:6px;">
                <span class="pill ${rev.status === 'pending' ? 'warning' : 'success'}">${rev.status === 'pending' ? '新旧观点对比 · 待确认' : '已完成确认'}</span>
              </div>
              <h1 style="font-size:21px;font-weight:700;">${esc(rev.title)}</h1>
              <p style="font-size:13px;color:var(--text-secondary);margin-top:4px;">${esc(rev.plainHint)}</p>
            </div>
            <button type="button" class="btn sm" data-open-page="decision-reconnect">返回对应知识页</button>
          </div>

          <div class="diff-split">
            <section class="diff-col current">
              <div class="diff-col-head">
                <span class="kicker" style="color:var(--error);">旧笔记怎么说</span>
                <span class="mono" style="font-size:11px;color:var(--muted);">${esc(rev.current.sourceLabel)}</span>
              </div>
              <p style="font-size:14.5px;font-weight:600;">${esc(rev.current.claim)}</p>
              <div class="diff-quote">原文：“${esc(rev.current.quote)}”</div>
            </section>

            <section class="diff-col incoming">
              <div class="diff-col-head">
                <span class="kicker" style="color:var(--success);">新来源怎么说</span>
                <span class="mono" style="font-size:11px;color:var(--muted);">${esc(rev.incoming.sourceLabel)}</span>
              </div>
              <p style="font-size:14.5px;font-weight:600;">${esc(rev.incoming.claim)}</p>
              <div class="diff-quote">原文：“${esc(rev.incoming.quote)}”</div>
            </section>
          </div>

          <section>
            <div class="kicker" style="margin-bottom:8px;">涉及的 3 篇知识页（可单独跳过某篇，无需一刀切）</div>
            <div class="affected-pages">
              ${rev.affectedPages.map((ap, i) => `
                <div class="affected-card">
                  <button type="button" class="wikilink" style="text-align:left;font-weight:600;" data-open-page="${ap.pageId}">${esc(ap.title)}</button>
                  <span style="font-size:12px;color:var(--muted);">${esc(ap.opLabel)}</span>
                  <select data-page-override="${i}" ${rev.status !== 'pending' ? 'disabled' : ''} aria-label="${esc(ap.title)} 处理方式">
                    <option value="follow" ${ap.override === 'follow' ? 'selected' : ''}>按下方选择同步更新</option>
                    <option value="incoming" ${ap.override === 'incoming' ? 'selected' : ''}>这篇直接采用新说法</option>
                    <option value="skip" ${ap.override === 'skip' ? 'selected' : ''}>先不改这篇</option>
                  </select>
                </div>
              `).join('')}
            </div>
          </section>

          <section class="decision-bar">
            <div class="kicker">确认采用哪一种结论（随时可在「整理」页撤销）</div>
            ${rev.status === 'pending' ? `
              <input type="text" id="conflict-condition-input" class="condition-input" value="${esc(rev.suggestedCondition)}" placeholder="选择「两个都保留」时，请写明各自适用情况（必填）…" aria-label="适用条件说明">
              <div style="display:flex;flex-wrap:wrap;gap:8px;">
                <button type="button" class="btn primary" data-resolve-conflict="coexist">两个都保留并写明适用范围（推荐）</button>
                <button type="button" class="btn" data-resolve-conflict="incoming">改用新结论（旧结论归档保留）</button>
                <button type="button" class="btn" data-resolve-conflict="current">维持旧结论</button>
              </div>
            ` : `
              <p style="font-size:13px;color:var(--success);">已确认：${esc(rev.resolution)}。相关知识页已同步更新。</p>
            `}
          </section>
        </div>
      </div>
    `;
  }

  // ==========================================================================
  // View 5: Interactive Knowledge Graph
  // ==========================================================================
  function renderGraphView() {
    const nodes = S.graph.nodes.filter((n) => S.graphTypeFilter === 'all' || n.type === S.graphTypeFilter);
    const nodeIds = new Set(nodes.map((n) => n.id));
    const edges = S.graph.edges.filter((e) => nodeIds.has(e.from) && nodeIds.has(e.to));
    const activeEdge = edges.find((e) => e.id === S.graphSelectedEdgeId) || edges[0];
    const activeNode = nodes.find((n) => n.id === S.graphSelectedNode) || nodes[0];
    const edgeColor = (kind) => kind === 'backlink' ? 'var(--info)' : 'var(--accent)';

    return `
      <div class="stage-body flush">
        <div class="graph-layout">
          <div class="graph-canvas-wrap">
            <div class="graph-toolbar">
              <div style="display:flex;align-items:center;gap:6px;">
                <span class="kicker">筛选</span>
                ${['all', 'decision', 'concept', 'synthesis', 'entity'].map((t) => `
                  <button type="button" class="btn sm ${S.graphTypeFilter === t ? 'primary' : 'quiet'}" data-graph-filter="${t}">
                    ${{ all: '全部', decision: '决策', concept: '概念', synthesis: '综论', entity: '组件' }[t]}
                  </button>
                `).join('')}
              </div>
              <div style="display:flex;align-items:center;gap:8px;font-size:11.5px;color:var(--muted);">
                <span>${S.graphShowMetaPages ? '已显示目录与日志页' : '已隐藏目录与日志页'}</span>
                <button type="button" class="btn sm quiet" data-action="toggle-graph-meta" style="min-height:22px;padding:1px 6px;">${S.graphShowMetaPages ? '隐藏' : '显示'}</button>
              </div>
            </div>

            <div class="graph-svg-stage">
              <svg class="graph-svg" viewBox="0 0 740 410" role="img" aria-label="交互式知识图谱">
                ${edges.map((e) => {
                  const a = S.graph.nodes.find((n) => n.id === e.from);
                  const b = S.graph.nodes.find((n) => n.id === e.to);
                  if (!a || !b) return '';
                  const isSel = activeEdge && activeEdge.id === e.id;
                  const dash = e.kind === 'backlink' ? '3 3' : '';
                  return `
                    <g data-select-edge="${e.id}" style="cursor:pointer;">
                      <line x1="${a.x}" y1="${a.y}" x2="${b.x}" y2="${b.y}" stroke="transparent" stroke-width="14"/>
                      <line x1="${a.x}" y1="${a.y}" x2="${b.x}" y2="${b.y}" stroke="${edgeColor(e.kind)}" stroke-width="${isSel ? 2.6 : 1.5}" stroke-opacity="${isSel ? 1 : 0.55}" ${dash ? `stroke-dasharray="${dash}"` : ''}/>
                    </g>
                  `;
                }).join('')}
                ${nodes.map((n) => {
                  const isSel = activeNode && activeNode.id === n.id;
                  const r = n.hub ? 11 : n.orphan ? 7.5 : 8.5;
                  const fill = isSel ? 'var(--accent)' : n.orphan ? 'var(--warning)' : 'var(--surface-raised)';
                  const stroke = isSel ? 'var(--accent)' : n.hub ? 'var(--accent)' : 'var(--line-strong)';
                  return `
                    <g data-select-node="${n.id}" style="cursor:pointer;">
                      <circle cx="${n.x}" cy="${n.y}" r="${r + (isSel ? 4 : 0)}" fill="${isSel ? 'var(--accent-soft)' : 'transparent'}"/>
                      <circle cx="${n.x}" cy="${n.y}" r="${r}" fill="${fill}" stroke="${stroke}" stroke-width="1.8"/>
                      <text x="${n.x}" y="${n.y + r + 15}" text-anchor="middle" fill="var(--text)" font-size="11.5" font-weight="${isSel ? 650 : 500}">${esc(n.label)}</text>
                    </g>
                  `;
                }).join('')}
              </svg>

              ${activeEdge ? `
                <div class="edge-popover">
                  <div style="display:flex;align-items:center;justify-content:space-between;gap:8px;">
                    <div style="display:flex;align-items:center;gap:6px;">
                      <span class="pill accent">${esc(activeEdge.label)}</span>
                      <strong style="font-size:12.5px;">${esc(getPage(activeEdge.from).title)} → ${esc(getPage(activeEdge.to).title)}</strong>
                    </div>
                    <button type="button" class="btn sm" data-open-page="${activeEdge.from}">打开笔记</button>
                  </div>
                  <div style="font-size:12.5px;color:var(--text-secondary);">关联依据：${esc(activeEdge.snippet)}</div>
                </div>
              ` : ''}
            </div>
          </div>

          <aside class="graph-side" aria-label="关联列表与待补充连接">
            <div>
              <div class="kicker" style="margin-bottom:6px;">关联列表（支持键盘上下切换）</div>
              <div style="font-size:13.5px;font-weight:650;margin-bottom:8px;">${activeNode ? esc(activeNode.label) : '未选中'}</div>
              <div style="display:flex;flex-direction:column;gap:5px;">
                ${edges.filter((e) => !activeNode || e.from === activeNode.id || e.to === activeNode.id).map((e) => {
                  const otherId = activeNode && e.from === activeNode.id ? e.to : e.from;
                  return `
                    <button type="button" class="list-item ${activeEdge && activeEdge.id === e.id ? 'active' : ''}" data-select-edge="${e.id}" data-focus-edge="${e.id}">
                      <div class="list-item-row">
                        <span class="list-item-title">${esc(getPage(otherId).title)}</span>
                        <span class="pill">${esc(e.label)}</span>
                      </div>
                      <span class="list-item-sub">${esc(e.snippet)}</span>
                    </button>
                  `;
                }).join('') || '<p style="font-size:12px;color:var(--muted);">此笔记暂无连接。</p>'}
              </div>
            </div>

            <div style="padding-top:12px;border-top:1px solid var(--line-subtle);">
              <div class="kicker" style="margin-bottom:8px;">可能遗漏的关联</div>
              ${!S.graph.gap.dismissed ? `
                <div class="cs-card">
                  <div style="display:flex;justify-content:space-between;align-items:center;">
                    <span class="pill warning">待补充关联</span>
                    <button type="button" class="btn quiet sm" data-action="dismiss-gap">忽略</button>
                  </div>
                  <strong style="font-size:13px;">${esc(S.graph.gap.title)}</strong>
                  <p style="font-size:12px;color:var(--text-secondary);">${esc(S.graph.gap.reason)}</p>
                  <button type="button" class="btn primary sm" data-action="ask-gap">让 AI 帮忙查证</button>
                </div>
              ` : '<p style="font-size:12px;color:var(--muted);">已忽略该建议。</p>'}
            </div>
          </aside>
        </div>
      </div>
    `;
  }

  // ==========================================================================
  // Right Companion Panel (Super Clean 2 Tabs: 文档信息 / 导读 | AI 问答)
  // ==========================================================================
  function renderRightPane() {
    const mode = currentViewMode();
    const inGraph = S.activeWorkspace === 'graph';
    const infoLabel = mode === 'inbox' ? '导读与重点' : '文档信息';

    return `
      <aside class="right-pane" aria-label="右侧文档信息与 AI 问答栏">
        <div class="right-tabs" role="tablist">
          ${!inGraph ? `<button type="button" class="${S.rightTab === 'info' ? 'active' : ''}" data-right-tab="info">${infoLabel}</button>` : ''}
          <button type="button" class="${S.rightTab === 'chat' || inGraph ? 'active' : ''}" data-right-tab="chat">AI 问答</button>
        </div>

        ${S.rightTab === 'info' && !inGraph
          ? (mode === 'inbox' ? renderRightInboxInfo() : mode === 'raw' ? renderRightRawInfo() : renderRightWikiInfo())
          : renderRightChat()}
      </aside>
    `;
  }

  function renderRightWikiInfo() {
    const p = getPage(S.activePageId);
    const pageHistory = S.changesets.filter((c) => (c.pageIds || []).includes(p.id)).slice(0, 2);

    return `
      <div class="right-scroll">
        <div class="context-section">
          <div class="kicker">来源原文（${p.citations.length}）</div>
          ${p.citations.map((c) => `
            <div class="cs-card" style="padding:9px 10px;gap:6px;">
              <div style="display:flex;justify-content:space-between;align-items:center;gap:6px;">
                <strong style="font-size:12px;">${esc(c.label)}</strong>
                <button type="button" class="btn quiet sm" data-open-evidence="${c.id}" style="min-height:22px;padding:1px 6px;color:var(--accent);">对照</button>
              </div>
              <div style="font-size:11.5px;color:var(--text-secondary);">“${esc(c.quote.slice(0, 56))}…”</div>
            </div>
          `).join('') || '<span style="font-size:12px;color:var(--muted);">暂无关联原文</span>'}
        </div>

        <div class="context-section">
          <div class="kicker">谁引用了这篇（${p.backlinks.length}）</div>
          ${p.backlinks.map((bl) => `
            <button type="button" class="list-item" data-open-page="${bl.pageId}">
              <span class="list-item-title">${esc(bl.title)}</span>
              <span class="list-item-sub">${esc(bl.excerpt)}</span>
            </button>
          `).join('') || '<span style="font-size:12px;color:var(--muted);">暂无其他笔记引用</span>'}
        </div>

        <div class="context-section">
          <div class="kicker">目录大纲</div>
          ${p.outline.map((h) => `<div style="font-size:12.5px;color:var(--text-secondary);padding:2px 4px;">· ${esc(h)}</div>`).join('')}
        </div>

        ${pageHistory.length > 0 ? `
          <div class="context-section">
            <div style="display:flex;justify-content:space-between;align-items:center;">
              <span class="kicker">本页整理记录</span>
              <button type="button" class="btn quiet sm" data-nav="organize" style="min-height:22px;padding:1px 6px;">全部</button>
            </div>
            ${pageHistory.map((cs) => `
              <div style="font-size:12px;color:var(--text-secondary);display:flex;justify-content:space-between;align-items:center;gap:6px;">
                <span style="overflow:hidden;text-overflow:ellipsis;white-space:nowrap;">${esc(cs.time)} · ${esc(cs.summary)}</span>
                ${cs.status !== 'reverted' ? `<button type="button" class="btn quiet sm" data-revert-cs="${cs.id}" style="min-height:22px;padding:1px 6px;">撤销</button>` : '<span class="pill">已撤销</span>'}
              </div>
            `).join('')}
          </div>
        ` : ''}
      </div>
    `;
  }

  function renderRightRawInfo() {
    const r = getRaw(S.activeRawId);
    return `
      <div class="right-scroll">
        <div class="context-section">
          <div class="kicker">由此原文整理出的知识页（${r.linkedPageIds.length}）</div>
          ${r.linkedPageIds.map((pid) => {
            const pg = S.pages[pid];
            return pg ? `
              <button type="button" class="list-item" data-open-page="${pg.id}">
                <span class="list-item-title">${esc(pg.title)}</span>
                <span class="list-item-sub">${esc(pg.typeLabel)} · 更新于 ${esc(pg.updated.slice(5))}</span>
              </button>
            ` : '';
          }).join('') || '<span style="font-size:12px;color:var(--muted);">尚未整理生成知识页</span>'}
        </div>

        <div class="context-section">
          <div class="kicker">文件信息</div>
          <div style="font-size:12px;color:var(--text-secondary);display:flex;flex-direction:column;gap:5px;">
            <div>类型：${esc(r.kindLabel)}</div>
            <div>保存时间：${esc(r.created)}</div>
            <div class="mono" style="font-size:11px;color:var(--muted);word-break:break-all;">${esc(r.path)}</div>
          </div>
        </div>
      </div>
    `;
  }

  function renderRightInboxInfo() {
    const item = getInbox(S.selectedInboxId);
    const highlights = (item.paragraphs || []).filter((p) => p.highlighted);
    return `
      <div class="right-scroll">
        <div class="context-section">
          <div class="kicker">AI 3 句话导读</div>
          <ul style="margin:0;padding-left:16px;display:flex;flex-direction:column;gap:6px;font-size:12.5px;color:var(--text-secondary);line-height:1.6;">
            ${(item.summaryPoints || []).map((pt) => `<li>${esc(pt)}</li>`).join('')}
          </ul>
        </div>

        <div class="context-section">
          <div class="kicker">你标出的重点（${highlights.length}）</div>
          ${highlights.map((h) => `
            <div class="cs-card" style="padding:9px 10px;gap:5px;">
              <div style="display:flex;justify-content:space-between;align-items:center;">
                <strong style="font-size:12px;">${esc(h.heading || '重点片段')}</strong>
                ${h.compiled ? '<span class="pill success">已入库</span>' : '<span class="pill accent">待提炼</span>'}
              </div>
              <div style="font-size:11.5px;color:var(--text-secondary);">“${esc(h.text.slice(0, 62))}…”</div>
            </div>
          `).join('') || '<span style="font-size:12px;color:var(--muted);">阅读原文时点击任意段落即可标为重点。</span>'}
        </div>

        ${(item.relatedPageIds || []).length > 0 ? `
          <div class="context-section">
            <div class="kicker">相关已有知识页</div>
            ${item.relatedPageIds.map((pid) => {
              const pg = S.pages[pid];
              return pg ? `<button type="button" class="list-item" data-open-page="${pg.id}"><span class="list-item-title">${esc(pg.title)}</span><span class="list-item-sub">${esc(pg.typeLabel)}</span></button>` : '';
            }).join('')}
          </div>
        ` : ''}
      </div>
    `;
  }

  function renderRightChat() {
    const p = getPage(S.activePageId);
    if (S.chatMode === 'review') {
      const steps = [
        { q: `正文已遮住：如果要向同事解释「${p.title}」，你会怎么概括它的核心边界与前提条件？`, placeholder: '用自己的话复述，或直接点「还说不清」…' },
        { q: '核对一下边界条件：如果超过 120 秒窗口或缺少服务端确认编号，原方案还成立吗？', placeholder: '说明失效后的回退方式…' },
        { q: '换个场景想想：如果移动端切网时界面已渲染半句本地缓存，而服务端只确认到上一帧，你会先做什么？', placeholder: '写下你的判断与理由…' }
      ];
      const curStep = steps[Math.min(S.reviewFlowStep, steps.length - 1)];

      return `
        <div class="right-scroll">
          <div style="display:flex;justify-content:space-between;align-items:center;">
            <span class="pill accent">让我讲一遍 · 遮蔽复述中</span>
            <button type="button" class="btn quiet sm" data-action="exit-curtain">结束</button>
          </div>
          <div class="chat-msg">
            <span class="chat-layer-tag">步骤 ${Math.min(S.reviewFlowStep + 1, 3)} / 3 · 《${esc(p.title)}》</span>
            <p>${esc(curStep.q)}</p>
          </div>
          ${S.reviewFlowStep >= 3 ? `
            <div class="cs-card">
              <span class="kicker">觉得掌握得怎么样？</span>
              <div style="display:flex;gap:6px;flex-wrap:wrap;">
                <button type="button" class="btn sm ${S.reviewSelfRating === 'ready' ? 'primary' : ''}" data-self-rate="ready">能用</button>
                <button type="button" class="btn sm ${S.reviewSelfRating === 'again' ? 'primary' : ''}" data-self-rate="again">过几天再看</button>
                <button type="button" class="btn sm ${S.reviewSelfRating === 'skip' ? 'primary' : ''}" data-self-rate="skip">暂不关心</button>
              </div>
              <div style="padding-top:8px;border-top:1px solid var(--line-subtle);">
                <p style="font-size:12px;color:var(--text-secondary);margin-bottom:6px;">刚才你补充了一条好想法：「先按 offset 截断未确认缓存，再拼续传帧」。要记入本页「我的理解」吗？</p>
                <button type="button" class="btn primary sm" data-action="write-understanding">写入「我的理解」</button>
              </div>
            </div>
          ` : ''}
        </div>
        ${S.reviewFlowStep < 3 ? `
          <div class="chat-composer">
            <textarea id="teachback-input" placeholder="${esc(curStep.placeholder)}"></textarea>
            <div style="display:flex;justify-content:space-between;">
              <button type="button" class="btn quiet sm" data-action="advance-teachback">还说不清 · 看提示</button>
              <button type="button" class="btn primary sm" data-action="advance-teachback">提交作答</button>
            </div>
          </div>
        ` : ''}
      `;
    }

    return `
      <div class="right-scroll">
        <div style="display:flex;justify-content:space-between;align-items:center;">
          <span class="kicker">问答范围</span>
          <button type="button" class="btn quiet sm" data-action="start-teachback">让我讲一遍</button>
        </div>
        <div class="chat-scope-bar">
          <span class="scope-chip">当前页：${esc(p.title.slice(0, 10))}</span>
          <button type="button" class="scope-chip ${S.chatIncludeOneHop ? '' : 'add'}" data-action="toggle-onehop">
            ${S.chatIncludeOneHop ? '已含关联笔记 ×' : '+ 加入关联笔记（3 篇）'}
          </button>
        </div>

        ${S.chatMessages.map((m, idx) => m.role === 'user' ? `
          <div class="chat-msg user">
            <span class="chat-layer-tag">你</span>
            <div>${esc(m.text)}</div>
          </div>
        ` : `
          <div class="chat-msg">
            ${m.layers.map((l) => `
              <div>
                <div class="chat-layer-tag">${esc(l.tag)}</div>
                <div>
                  ${esc(l.text)}
                  ${(l.citationIds || []).map((cid, ci) => {
                    const cit = findCitation(cid);
                    return `<button type="button" class="footnote-ref" data-open-evidence="${cid}" title="${esc(cit ? cit.label : '对照原文')}">[^${esc(cit ? cit.ref : `s${ci + 1}`)}]</button>`;
                  }).join('')}
                </div>
              </div>
            `).join('')}
            <div style="display:flex;gap:6px;padding-top:6px;border-top:1px solid var(--line-subtle);">
              <button type="button" class="btn sm" data-query-to-wiki="${idx}" ${m.savedToWiki ? 'disabled' : ''}>${m.savedToWiki ? '已存进知识库' : '存成知识页'}</button>
              <button type="button" class="btn quiet sm" data-action="toast" data-msg="已复制回答正文与出处">复制</button>
            </div>
          </div>
        `).join('')}
      </div>
      <div class="chat-composer">
        <textarea id="chat-input" placeholder="围绕当前笔记或整个知识库提问…"></textarea>
        <div style="display:flex;justify-content:space-between;align-items:center;">
          <span style="font-size:11px;color:var(--muted);">Ctrl+Enter 发送</span>
          <button type="button" class="btn primary sm" data-action="send-chat">提问</button>
        </div>
      </div>
    `;
  }

  // ==========================================================================
  // Quick Capture HUD
  // ==========================================================================
  const hudState = { text: '', imeComposing: false, diskError: false, showAc: false };

  function openHud() {
    $('#hud-layer').classList.remove('hidden');
    renderHud();
    setTimeout(() => { $('#hud-textarea')?.focus(); }, 20);
  }

  function closeHud(msg) {
    $('#hud-layer').classList.add('hidden');
    if (msg) showToast(msg);
  }

  function renderHud() {
    const layer = $('#hud-layer');
    layer.innerHTML = `
      <div class="hud-window">
        <div class="hud-head">
          <div style="display:flex;align-items:center;gap:8px;">
            <span class="pill accent">随手记</span>
            <span style="font-size:11.5px;color:var(--muted);">存为本地 Markdown 原文 · 后台自动整理进知识库</span>
          </div>
          <div style="display:flex;align-items:center;gap:8px;font-size:11.5px;">
            <label style="display:inline-flex;align-items:center;gap:4px;cursor:pointer;color:var(--muted);">
              <input type="checkbox" id="hud-sim-ime" ${hudState.imeComposing ? 'checked' : ''}> 模拟组词期
            </label>
            <label style="display:inline-flex;align-items:center;gap:4px;cursor:pointer;color:var(--muted);">
              <input type="checkbox" id="hud-sim-err" ${hudState.diskError ? 'checked' : ''}> 模拟写盘失败
            </label>
            <button type="button" class="icon-btn" id="hud-close" aria-label="收起浮窗">&times;</button>
          </div>
        </div>

        <div class="hud-editor-wrap">
          <textarea id="hud-textarea" class="hud-textarea" placeholder="记下想法，支持 Markdown 语法与输入 [[ 关联已有笔记…">${esc(hudState.text)}</textarea>
          ${hudState.showAc ? `
            <div class="hud-autocomplete" role="listbox" aria-label="双链补全候选">
              ${Object.values(S.pages).slice(0, 4).map((p) => `
                <button type="button" class="hud-ac-item" data-hud-insert="${esc(p.title)}">
                  <span>[[${esc(p.title)}]]</span>
                  <span style="color:var(--muted);">${esc(p.typeLabel)}</span>
                </button>
              `).join('')}
            </div>
          ` : ''}
        </div>

        ${hudState.diskError ? `
          <div class="hud-error-bar">
            <span><strong>写入磁盘失败：</strong>内容仅暂存内存（退出程序前有效），请先复制全文备份。</span>
            <div style="display:flex;gap:6px;">
              <button type="button" class="btn primary sm" id="hud-copy-rescue">复制全文</button>
              <button type="button" class="btn sm" id="hud-hide-rescue">暂存内存并收起</button>
            </div>
          </div>
        ` : ''}

        <div class="hud-foot">
          <span style="font-size:11.5px;color:var(--muted);">
            ${hudState.imeComposing ? '中文组词期：Enter 与 Esc 优先交给输入法' : 'Esc 仅收起并保留草稿，绝不误提交'}
          </span>
          <div style="display:flex;align-items:center;gap:6px;">
            <button type="button" class="btn quiet sm" id="hud-save-only">仅保存原文</button>
            <button type="button" class="btn sm" id="hud-save-now">立即整理 <kbd>Ctrl+Shift+Enter</kbd></button>
            <button type="button" class="btn primary sm" id="hud-save-queue">保存并整理 <kbd class="hud-kbd-inline">Ctrl+Enter</kbd></button>
          </div>
        </div>
      </div>
    `;

    const ta = $('#hud-textarea');
    ta.addEventListener('input', (e) => {
      hudState.text = e.target.value;
      const shouldShow = /\[\[[^\]]*$/.test(hudState.text);
      if (shouldShow !== hudState.showAc) {
        hudState.showAc = shouldShow;
        renderHud();
        const nextTa = $('#hud-textarea');
        nextTa.focus();
        nextTa.selectionStart = nextTa.selectionEnd = hudState.text.length;
      }
    });

    ta.addEventListener('keydown', (e) => {
      if (e.key === 'Escape') {
        e.preventDefault();
        if (hudState.imeComposing) {
          hudState.imeComposing = false;
          showToast('Esc 已优先取消中文组词，未关窗');
          renderHud();
        } else if (hudState.showAc) {
          hudState.showAc = false;
          renderHud();
        } else if (!hudState.text.trim()) {
          closeHud();
        } else {
          closeHud('已收起并保留草稿，下次打开自动恢复');
        }
      } else if (e.key === 'Enter' && e.ctrlKey && e.shiftKey) {
        e.preventDefault();
        submitHud('now');
      } else if (e.key === 'Enter' && e.ctrlKey) {
        e.preventDefault();
        submitHud('queue');
      }
    });

    $('#hud-sim-ime').addEventListener('change', (e) => { hudState.imeComposing = e.target.checked; renderHud(); });
    $('#hud-sim-err').addEventListener('change', (e) => { hudState.diskError = e.target.checked; renderHud(); });
    $('#hud-close').addEventListener('click', () => closeHud(hudState.text.trim() ? '已收起，草稿已保留' : ''));
    $('#hud-save-only').addEventListener('click', () => submitHud('only'));
    $('#hud-save-now').addEventListener('click', () => submitHud('now'));
    $('#hud-save-queue').addEventListener('click', () => submitHud('queue'));
    $('#hud-copy-rescue')?.addEventListener('click', () => showToast('已复制全文到剪贴板'));
    $('#hud-hide-rescue')?.addEventListener('click', () => closeHud('已暂存内存并收起，退出程序前请先备份'));

    layer.querySelectorAll('[data-hud-insert]').forEach((btn) => {
      btn.addEventListener('click', () => {
        hudState.text = hudState.text.replace(/\[\[[^\]]*$/, `[[${btn.dataset.hudInsert}]] `);
        hudState.showAc = false;
        renderHud();
        const nextTa = $('#hud-textarea');
        nextTa.focus();
        nextTa.selectionStart = nextTa.selectionEnd = hudState.text.length;
      });
    });
  }

  function submitHud(mode) {
    if (hudState.imeComposing) { showToast('中文组词期保护：请先完成选词'); return; }
    if (hudState.diskError) { showToast('磁盘写入失败：请先复制全文备份'); return; }
    const text = hudState.text.trim() || '弱网切换 5G 时若收到 410 Gone，应立即回退整段重算并清除旧游标。';
    hudState.text = '';
    hudState.showAc = false;
    closeHud();
    if (mode === 'only') {
      const rawId = `raw-note-${Date.now()}`;
      S.rawFiles[rawId] = { id: rawId, path: 'raw/notes/2026/11/20261126-quick.md', title: text.slice(0, 22), created: '刚刚', kind: 'note', kindLabel: '随手笔记', url: '', linkedPageIds: [], content: text };
      showToast('已保存为原文笔记（可在知识库「原文」直接查看）');
      render();
      return;
    }
    if (mode === 'now') {
      S.bufferNotes.push({ id: `buf-${Date.now()}`, title: text.slice(0, 20), text });
      flushBufferCompile(false);
      return;
    }
    startBufferCountdownIfIdle(text);
    showToast(`已保存原文，${S.bufferSecondsLeft} 秒后自动整理进知识库`, [
      { label: '查看 / 撤回', onClick: () => openBufferQueueModal() }
    ]);
    render();
  }

  function openBufferQueueModal() {
    const layer = $('#modal-layer');
    layer.classList.remove('hidden');
    layer.innerHTML = `
      <div class="modal-window" style="width:min(540px,100%);">
        <div class="hud-head">
          <strong style="font-size:14px;">待整理笔记队列 · 剩余 ${formatSec(S.bufferSecondsLeft)}</strong>
          <button type="button" class="icon-btn" id="modal-close">&times;</button>
        </div>
        <div class="modal-body">
          <p style="font-size:12.5px;color:var(--text-secondary);">以下笔记已安全保存为本地原文。写错了可在倒计时结束前移出本次整理（本地原文会保留）。</p>
          ${S.bufferNotes.map((n) => `
            <div class="ops-row">
              <div class="ops-row-main">
                <span class="ops-row-title">${esc(n.title)}</span>
                <span class="ops-row-desc">${esc(n.text)}</span>
              </div>
              <button type="button" class="btn sm" data-cancel-buf="${n.id}">移出本批</button>
            </div>
          `).join('') || '<p style="font-size:12.5px;color:var(--muted);">当前队列已清空。</p>'}
        </div>
        <div class="hud-foot">
          <button type="button" class="btn quiet" id="modal-cancel">关闭</button>
          ${S.bufferNotes.length > 0 ? '<button type="button" class="btn primary sm" id="modal-flush-now">立即整理</button>' : ''}
        </div>
      </div>
    `;
    $('#modal-close').addEventListener('click', () => layer.classList.add('hidden'));
    $('#modal-cancel').addEventListener('click', () => layer.classList.add('hidden'));
    $('#modal-flush-now')?.addEventListener('click', () => { layer.classList.add('hidden'); flushBufferCompile(false); });
    layer.querySelectorAll('[data-cancel-buf]').forEach((btn) => {
      btn.addEventListener('click', () => {
        S.bufferNotes = S.bufferNotes.filter((n) => n.id !== btn.dataset.cancelBuf);
        if (S.bufferNotes.length === 0) {
          S.bufferSecondsLeft = 0;
          if (bufferInterval) { clearInterval(bufferInterval); bufferInterval = null; }
        }
        showToast('已移出本次整理队列（本地原文保留）');
        openBufferQueueModal();
        render();
      });
    });
  }

  function openSettingsModal() {
    const layer = $('#modal-layer');
    layer.classList.remove('hidden');
    const modelIsLocal = isLoopbackUrl(S.settings.modelEndpoint);
    const ovIsLocal = isLoopbackUrl(S.settings.ovEndpoint);
    const curTheme = document.body.dataset.theme || 'slate';
    const curWidth = document.body.dataset.width || 'readable';

    layer.innerHTML = `
      <div class="modal-window">
        <div class="hud-head">
          <strong style="font-size:14px;">设置 · 外观、AI 模型与数据范围</strong>
          <button type="button" class="icon-btn" id="modal-close">&times;</button>
        </div>
        <div class="modal-body">
          <section class="setting-card">
            <strong>1. 外观与阅读宽度</strong>
            <div class="form-grid">
              <div class="field">
                <label>界面主题</label>
                <select id="set-theme">
                  <option value="slate" ${curTheme === 'slate' ? 'selected' : ''}>深色 · Cognitive Slate（默认）</option>
                  <option value="paper" ${curTheme === 'paper' ? 'selected' : ''}>浅色 · Paper Daylight</option>
                </select>
              </div>
              <div class="field">
                <label>默认文档宽度</label>
                <select id="set-width">
                  <option value="readable" ${curWidth === 'readable' ? 'selected' : ''}>适读宽度（920px）</option>
                  <option value="full" ${curWidth === 'full' ? 'selected' : ''}>铺满整栏（宽屏满宽）</option>
                </select>
              </div>
            </div>
          </section>

          <section class="setting-card">
            <div style="display:flex;justify-content:space-between;align-items:center;">
              <strong>2. OpenAI 兼容模型与自定义网关</strong>
              <span class="pill success">API Key 仅存 Windows 凭据管理器</span>
            </div>
            <div class="form-grid">
              <div class="field full">
                <label>端点地址 Base URL（支持任意 OpenAI 兼容服务或网关）</label>
                <input type="text" id="set-model-url" value="${esc(S.settings.modelEndpoint)}">
              </div>
              <div class="field">
                <label>主整理模型 ID</label>
                <input type="text" id="set-model-primary" value="${esc(S.settings.modelPrimary)}">
              </div>
              <div class="field">
                <label>显示名称</label>
                <input type="text" id="set-model-alias" value="${esc(S.settings.modelAlias)}">
              </div>
              <div class="field">
                <label>可选轻模型（文章导读用）</label>
                <input type="text" id="set-model-light" value="${esc(S.settings.modelLight)}">
              </div>
              <div class="field">
                <label>API Key（留空保留已有密钥）</label>
                <input type="password" placeholder="••••••••（已安全保存）">
              </div>
            </div>
            <div style="display:flex;justify-content:space-between;align-items:center;font-size:12px;">
              <span id="set-model-risk-note" style="color:${modelIsLocal ? 'var(--text-secondary)' : 'var(--warning)'};">
                ${modelIsLocal ? '地址为本机回环；若经代理转发至第三方服务，请在下方核对发送范围。' : '注意：当前为非本机远端地址，授权正文将发送至外部服务。'}
              </span>
              <button type="button" class="btn sm" id="btn-test-model">测试连接</button>
            </div>
          </section>

          <section class="setting-card">
            <div style="display:flex;justify-content:space-between;align-items:center;">
              <strong>3. 记忆体系对接（先本地 OpenViking，可插拔扩展）</strong>
              <span class="pill">可替换架构</span>
            </div>
            <div class="form-grid">
              <div class="field">
                <label>记忆后端</label>
                <select id="set-mem-provider">
                  <option value="openviking" ${S.settings.memoryProvider === 'openviking' ? 'selected' : ''}>OpenViking 本地（当前默认）</option>
                  <option value="none" ${S.settings.memoryProvider === 'none' ? 'selected' : ''}>停用（纯本地模式）</option>
                  <option value="hindsight" disabled>Hindsight 记忆库（后续扩展）</option>
                </select>
              </div>
              <div class="field">
                <label>OpenViking 服务地址</label>
                <input type="text" id="set-ov-url" value="${esc(S.settings.ovEndpoint)}">
              </div>
            </div>
            <p id="set-ov-risk-note" style="font-size:12px;color:var(--warning);display:${ovIsLocal ? 'none' : 'block'};">注意：非本机地址可能是共享服务，请确认数据可见范围。</p>
          </section>

          <section class="setting-card">
            <strong>4. 数据发送范围与安静整理偏好</strong>
            <table class="privacy-table">
              <thead>
                <tr><th>数据类型</th><th>AI 模型端点</th><th>记忆后端</th><th>本地只读 MCP</th></tr>
              </thead>
              <tbody>
                <tr>
                  <td>随手笔记原文</td>
                  <td><label><input type="checkbox" id="set-allow-note" ${S.settings.allowNoteFulltextToModel ? 'checked' : ''}> 允许发送整理</label></td>
                  <td><span class="pill">仅同步事件摘要</span></td>
                  <td><span class="pill">默认不暴露</span></td>
                </tr>
                <tr>
                  <td>收件箱收藏文章</td>
                  <td>仅发标出重点（或整篇确认）</td>
                  <td>仅同步摘要</td>
                  <td>默认不暴露</td>
                </tr>
                <tr>
                  <td>整理后的知识页</td>
                  <td>按问答/整理范围发送</td>
                  <td>按目录授权同步</td>
                  <td><label><input type="checkbox" id="set-mcp-on" ${S.settings.mcpEnabled ? 'checked' : ''}> 只读暴露</label></td>
                </tr>
              </tbody>
            </table>
            <label style="font-size:12px;color:var(--text-secondary);display:flex;align-items:center;gap:6px;">
              <input type="checkbox" id="set-quiet-mode" ${S.settings.quietThreshold ? 'checked' : ''}> 减少打扰模式：仅在明确观点冲突时提醒确认，常规增量静默完成
            </label>
          </section>
        </div>
        <div class="hud-foot">
          <span style="font-size:12px;color:var(--muted);">目录配置保存在 .wikra/config.json，不改动你的笔记原件</span>
          <div style="display:flex;gap:8px;">
            <button type="button" class="btn quiet" id="modal-cancel">取消</button>
            <button type="button" class="btn primary" id="modal-save">保存设置</button>
          </div>
        </div>
      </div>
    `;

    $('#modal-close').addEventListener('click', () => layer.classList.add('hidden'));
    $('#modal-cancel').addEventListener('click', () => layer.classList.add('hidden'));
    $('#set-model-url').addEventListener('input', (e) => {
      const local = isLoopbackUrl(e.target.value);
      const note = $('#set-model-risk-note');
      if (note) {
        note.style.color = local ? 'var(--text-secondary)' : 'var(--warning)';
        note.textContent = local ? '地址为本机回环；若经代理转发至第三方服务，请在下方核对发送范围。' : '注意：当前为非本机远端地址，授权正文将发送至外部服务。';
      }
    });
    $('#set-ov-url').addEventListener('input', (e) => {
      const note = $('#set-ov-risk-note');
      if (note) note.style.display = isLoopbackUrl(e.target.value) ? 'none' : 'block';
    });
    $('#btn-test-model').addEventListener('click', () => showToast(`连接成功：${$('#set-model-alias').value} 已就绪`));
    $('#modal-save').addEventListener('click', () => {
      updateTweak('theme', $('#set-theme').value);
      updateTweak('width', $('#set-width').value);
      S.settings.modelEndpoint = $('#set-model-url').value.trim();
      S.settings.modelPrimary = $('#set-model-primary').value.trim();
      S.settings.modelAlias = $('#set-model-alias').value.trim() || S.settings.modelPrimary;
      S.settings.modelLight = $('#set-model-light').value.trim();
      S.settings.allowNoteFulltextToModel = $('#set-allow-note').checked;
      S.settings.memoryProvider = $('#set-mem-provider').value;
      S.settings.ovEndpoint = $('#set-ov-url').value.trim();
      S.settings.mcpEnabled = $('#set-mcp-on').checked;
      S.settings.quietThreshold = $('#set-quiet-mode').checked;
      layer.classList.add('hidden');
      showToast('设置已保存');
      render();
    });
  }

  function openOnboardingModal() {
    const layer = $('#modal-layer');
    layer.classList.remove('hidden');
    layer.innerHTML = `
      <div class="modal-window" style="width:min(640px,100%);">
        <div class="hud-head">
          <strong style="font-size:14px;">选择或新建本地知识库</strong>
          <button type="button" class="icon-btn" id="modal-close">&times;</button>
        </div>
        <div class="modal-body">
          <p style="font-size:13px;color:var(--text-secondary);">独立通用的本地 Markdown 知识工作台，100% 兼容 Obsidian 文件格式。你可以新建空白知识库，或零搬迁直接打开已有 Obsidian 目录。</p>
          <div class="diff-split" style="grid-template-columns:1fr 1fr;">
            <div class="setting-card">
              <span class="pill accent" style="align-self:flex-start;">推荐 · 开箱即用</span>
              <strong>新建标准知识库</strong>
              <p style="font-size:12px;color:var(--text-secondary);">自动创建 <span class="mono">wiki/</span>（整理后的知识）与 <span class="mono">raw/</span>（原文归档），立刻开始记录。</p>
              <button type="button" class="btn primary sm" id="ob-new">新建标准库</button>
            </div>
            <div class="setting-card">
              <span class="pill" style="align-self:flex-start;">零搬迁兼容</span>
              <strong>打开已有 Obsidian 目录</strong>
              <p style="font-size:12px;color:var(--text-secondary);">不移动、不修改你原有笔记；配置独立存于 <span class="mono">.wikra/config.json</span>。</p>
              <button type="button" class="btn sm" id="ob-attach">打开已有目录</button>
            </div>
          </div>
        </div>
      </div>
    `;
    $('#modal-close').addEventListener('click', () => layer.classList.add('hidden'));
    $('#ob-new').addEventListener('click', () => { S.vaultName = 'personal-llm-wiki'; layer.classList.add('hidden'); showToast('已就绪：标准本地知识库'); render(); });
    $('#ob-attach').addEventListener('click', () => { S.vaultName = 'existing-vault'; layer.classList.add('hidden'); showToast('已打开已有目录，原有笔记零改动'); render(); });
  }

  // ==========================================================================
  // Events
  // ==========================================================================
  function bindEvents() {
    document.querySelectorAll('[data-nav]').forEach((btn) => {
      btn.addEventListener('click', () => {
        const nav = btn.dataset.nav;
        if (nav === 'organize') switchWorkspace('organize');
        else if (nav === 'graph') switchWorkspace('graph');
        else if (nav === 'library') { S.docLeftMode = 'library'; openPage(S.activePageId); }
        else if (nav === 'inbox') { S.docLeftMode = 'inbox'; openInboxItem(S.selectedInboxId); }
      });
    });

    document.querySelectorAll('[data-tab-id]').forEach((el) => {
      el.addEventListener('click', (e) => {
        if (e.target.closest('[data-close-tab]')) return;
        const tab = S.openDocTabs.find((t) => t.id === el.dataset.tabId);
        if (tab) openDocTab(tab);
      });
    });

    document.querySelectorAll('[data-close-tab]').forEach((btn) => {
      btn.addEventListener('click', (e) => { e.stopPropagation(); closeDocTab(btn.dataset.closeTab); });
    });

    document.querySelectorAll('[data-open-page]').forEach((el) => el.addEventListener('click', () => openPage(el.dataset.openPage)));
    document.querySelectorAll('[data-open-raw]').forEach((el) => el.addEventListener('click', () => openRaw(el.dataset.openRaw)));
    document.querySelectorAll('[data-open-inbox]').forEach((el) => el.addEventListener('click', () => openInboxItem(el.dataset.openInbox)));
    document.querySelectorAll('[data-lib-filter]').forEach((btn) => btn.addEventListener('click', () => { S.libraryFilter = btn.dataset.libFilter; render(); }));
    document.querySelectorAll('[data-inbox-filter]').forEach((btn) => btn.addEventListener('click', () => { S.inboxFilter = btn.dataset.inboxFilter; render(); }));
    document.querySelectorAll('[data-inbox-readmode]').forEach((btn) => btn.addEventListener('click', () => { S.inboxReadMode = btn.dataset.inboxReadmode; render(); }));
    document.querySelectorAll('[data-editor-mode]').forEach((btn) => btn.addEventListener('click', () => { S.editorMode = btn.dataset.editorMode; render(); }));

    document.querySelectorAll('[data-open-url]').forEach((btn) => {
      btn.addEventListener('click', () => showToast(`正在默认浏览器打开原网页：${btn.dataset.openUrl}`));
    });

    document.querySelectorAll('[data-open-evidence]').forEach((btn) => {
      btn.addEventListener('click', () => {
        const cid = btn.dataset.openEvidence;
        S.evidenceByPage[S.activePageId] = cid;
        // On <1680px screens, auto-collapse right panel so Wiki + Original sit 50/50 side-by-side cleanly!
        if (window.innerWidth < 1680) S.rightOpenByView.library = false;
        if (S.activeWorkspace !== 'docs' || !S.activeDocTab.startsWith('page:')) openPage(S.activePageId);
        else render();
      });
    });

    document.querySelectorAll('[data-peek-section]').forEach((btn) => {
      btn.addEventListener('click', () => { S.recallPeekSections[btn.dataset.peekSection] = true; render(); });
    });

    document.querySelectorAll('[data-toggle-task]').forEach((cb) => {
      cb.addEventListener('change', () => {
        const [pid, tid] = cb.dataset.toggleTask.split(':');
        const t = getPage(pid).tasks.find((x) => x.id === tid);
        if (t) { t.done = cb.checked; showToast('已更新待办状态'); render(); }
      });
    });

    document.querySelectorAll('[data-toggle-para]').forEach((el) => {
      const toggle = () => {
        const [iid, pid] = el.dataset.togglePara.split(':');
        const item = getInbox(iid);
        const para = item?.paragraphs.find((x) => x.id === pid);
        if (para && !para.compiled) {
          para.highlighted = !para.highlighted;
          render();
        }
      };
      el.addEventListener('click', toggle);
      el.addEventListener('keydown', (e) => { if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); toggle(); } });
    });

    document.querySelectorAll('[data-triage]').forEach((btn) => {
      btn.addEventListener('click', () => {
        const item = getInbox(S.selectedInboxId);
        if (!item) return;
        const act = btn.dataset.triage;
        if (act === 'archive') {
          item.state = 'archived';
          showToast('已存档，原文仍保留在知识库「原文」中可随时阅读');
        } else {
          const picked = (item.paragraphs || []).filter((p) => (act === 'full' || p.highlighted) && !p.compiled);
          picked.forEach((p) => { p.compiled = true; p.highlighted = true; });
          item.state = 'compiled';
          S.changesets.unshift({
            id: `cs-0${143 + S.changesets.length}`,
            time: '刚刚',
            trigger: `${act === 'excerpt' ? '提炼文章重点' : '提炼整篇文章'} · ${item.title}`,
            modelAlias: S.settings.modelAlias,
            status: 'applied',
            summary: `更新 1 篇知识页 · 提炼了 ${picked.length} 段重点`,
            pageIds: ['concept-event-id'],
            ops: [{ label: '补充内容', target: '服务端确认事件编号（event_id）', outcome: '已更新' }]
          });
          showToast(`已提炼 ${picked.length} 段内容更新知识库`);
        }
        render();
      });
    });

    document.querySelectorAll('[data-complete-stub]').forEach((btn) => {
      btn.addEventListener('click', () => {
        const item = getInbox(btn.dataset.completeStub);
        const txt = $('#stub-paste-area')?.value?.trim();
        if (!item || !txt) { showToast('请先粘贴文章正文再保存'); return; }
        item.fetchStatus = 'full';
        item.sourceType = '网页文章（已补全）';
        item.state = 'reading';
        item.readTime = '约 1 分钟';
        item.wordCount = txt.length;
        item.paragraphs = [{ id: 'p-new', kind: 'section', heading: '补全正文', text: txt, highlighted: true, compiled: false }];
        showToast('已存为本地原文，现在可阅读并标重点提炼');
        render();
      });
    });

    document.querySelectorAll('[data-page-override]').forEach((sel) => {
      sel.addEventListener('change', () => {
        const revId = S.activeDocTab.startsWith('review:') ? S.activeDocTab.replace('review:', '') : S.reviews[0].id;
        const rev = S.reviews.find((r) => r.id === revId) || S.reviews[0];
        const idx = Number(sel.dataset.pageOverride);
        if (rev.affectedPages[idx]) rev.affectedPages[idx].override = sel.value;
      });
    });

    document.querySelectorAll('[data-resolve-conflict]').forEach((btn) => {
      btn.addEventListener('click', () => {
        const revId = S.activeDocTab.startsWith('review:') ? S.activeDocTab.replace('review:', '') : S.reviews[0].id;
        const rev = S.reviews.find((r) => r.id === revId) || S.reviews[0];
        const mode = btn.dataset.resolveConflict;
        const condInput = $('#conflict-condition-input');
        const cond = condInput?.value?.trim() || '';
        if (mode === 'coexist' && !cond) {
          condInput?.classList.add('invalid');
          condInput?.focus();
          showToast('选择「两个都保留」时，请先写明各自适用范围');
          return;
        }
        rev.status = 'resolved';
        const appliedPages = rev.affectedPages.filter((ap) => ap.override !== 'skip');
        rev.resolution = mode === 'coexist' ? `两个都保留（${cond}）` : mode === 'incoming' ? '改用新结论' : '维持旧结论';
        appliedPages.forEach((ap) => {
          const pg = S.pages[ap.pageId];
          if (pg) pg.pendingConflicts = pg.pendingConflicts.filter((id) => id !== rev.id);
        });
        S.changesets.unshift({
          id: `cs-0${143 + S.changesets.length}`,
          time: '刚刚',
          trigger: `确认冲突结论 · ${rev.resolution}`,
          modelAlias: '手动确认',
          status: 'applied',
          summary: `已同步更新 ${appliedPages.length} 篇知识页`,
          pageIds: appliedPages.map((a) => a.pageId),
          ops: appliedPages.map((ap) => ({ label: ap.override === 'incoming' ? '改用新说法' : '同步更新', target: ap.title, outcome: '已更新' }))
        });
        showToast(`已确认结论，同步更新 ${appliedPages.length} 篇知识页`);
        render();
      });
    });

    document.querySelectorAll('[data-graph-filter]').forEach((btn) => btn.addEventListener('click', () => { S.graphTypeFilter = btn.dataset.graphFilter; render(); }));
    document.querySelectorAll('[data-select-node]').forEach((el) => el.addEventListener('click', () => { S.graphSelectedNode = el.dataset.selectNode; render(); }));
    document.querySelectorAll('[data-select-edge]').forEach((el) => el.addEventListener('click', () => { S.graphSelectedEdgeId = el.dataset.selectEdge; render(); }));
    document.querySelectorAll('[data-focus-edge]').forEach((el) => el.addEventListener('focus', () => {
      if (S.graphSelectedEdgeId !== el.dataset.focusEdge) {
        S.graphSelectedEdgeId = el.dataset.focusEdge;
        const edge = S.graph.edges.find((x) => x.id === S.graphSelectedEdgeId);
        const pop = $('.edge-popover');
        if (edge && pop) pop.querySelector('div:last-child').textContent = `关联依据：${edge.snippet}`;
      }
    }));

    document.querySelectorAll('[data-right-tab]').forEach((btn) => btn.addEventListener('click', () => { S.rightTab = btn.dataset.rightTab; render(); }));
    document.querySelectorAll('[data-self-rate]').forEach((btn) => btn.addEventListener('click', () => { S.reviewSelfRating = btn.dataset.selfRate; showToast(S.reviewSelfRating === 'again' ? '已记下，过几天提醒你再看' : '已记录'); render(); }));
    document.querySelectorAll('[data-revert-cs]').forEach((btn) => btn.addEventListener('click', () => revertChangeset(btn.dataset.revertCs)));
    document.querySelectorAll('[data-dismiss-sugg]').forEach((btn) => btn.addEventListener('click', () => { const s = S.suggestions.find((x) => x.id === btn.dataset.dismissSugg); if (s) { s.dismissed = true; render(); } }));
    document.querySelectorAll('[data-query-to-wiki]').forEach((btn) => {
      btn.addEventListener('click', () => {
        const m = S.chatMessages[Number(btn.dataset.queryToWiki)];
        if (m) { m.savedToWiki = true; showToast('回答已附原文出处保存至知识库'); render(); }
      });
    });

    document.querySelectorAll('[data-search-kind]').forEach((btn) => {
      btn.addEventListener('click', () => {
        S.searchOpen = false;
        const k = btn.dataset.searchKind;
        const id = btn.dataset.searchId;
        const cid = btn.dataset.searchCit;
        if (k === 'raw') openRaw(id);
        else {
          if (cid) S.evidenceByPage[id] = cid;
          openPage(id);
        }
      });
    });

    document.querySelectorAll('[data-action]').forEach((btn) => {
      btn.addEventListener('click', () => {
        const act = btn.dataset.action;
        if (act === 'open-hud') openHud();
        else if (act === 'open-settings') openSettingsModal();
        else if (act === 'open-onboard') openOnboardingModal();
        else if (act === 'open-buffer-queue') openBufferQueueModal();
        else if (act === 'open-review') openReview(btn.dataset.id);
        else if (act === 'toggle-right') { setRightOpen(!isRightOpen()); render(); }
        else if (act === 'toggle-theme') {
          const next = (document.body.dataset.theme || 'slate') === 'slate' ? 'paper' : 'slate';
          updateTweak('theme', next);
        }
        else if (act === 'toggle-width') {
          const next = (document.body.dataset.width || 'readable') === 'readable' ? 'full' : 'readable';
          updateTweak('width', next);
        }
        else if (act === 'toggle-split-origin') {
          const p = getPage(S.activePageId);
          if (S.evidenceByPage[p.id]) {
            S.evidenceByPage[p.id] = null;
          } else if (p.citations.length > 0) {
            S.evidenceByPage[p.id] = p.citations[0].id;
            if (window.innerWidth < 1680) S.rightOpenByView.library = false;
          }
          render();
        }
        else if (act === 'toggle-props') { S.propsExpanded = !S.propsExpanded; render(); }
        else if (act === 'sim-obsidian') { S.externalChangedBanner = true; showToast('已在 Obsidian 打开；现模拟外部保存同步提示'); render(); }
        else if (act === 'dismiss-external') { S.externalChangedBanner = false; showToast('已同步本地最新文件'); render(); }
        else if (act === 'start-teachback') {
          const targetPage = btn.dataset.page || S.activePageId;
          S.recallCurtainPageId = targetPage;
          S.recallPeekSections = {};
          S.chatMode = 'review';
          S.reviewFlowStep = 0;
          S.rightTab = 'chat';
          S.rightOpenByView.library = true;
          openPage(targetPage);
        }
        else if (act === 'exit-curtain') { S.recallCurtainPageId = null; S.chatMode = 'query'; render(); }
        else if (act === 'advance-teachback') {
          S.reviewFlowStep = Math.min(3, S.reviewFlowStep + 1);
          if (S.reviewFlowStep >= 3) S.recallCurtainPageId = null;
          render();
        }
        else if (act === 'write-understanding') {
          const p = getPage(S.activePageId);
          p.humanUnderstanding = (p.humanUnderstanding ? p.humanUnderstanding + ' ' : '') + '新洞察：续传前先按 offset 截断未确认缓存，再拼后续帧。';
          S.recallCurtainPageId = null;
          S.chatMode = 'query';
          showToast('已写入受手动保护的「我的理解」');
          openPage(p.id);
        }
        else if (act === 'toggle-onehop') { S.chatIncludeOneHop = !S.chatIncludeOneHop; render(); }
        else if (act === 'send-chat') {
          const val = $('#chat-input')?.value?.trim();
          if (!val) return;
          S.chatMessages.push({ role: 'user', text: val });
          S.chatMessages.push({
            role: 'assistant',
            layers: [
              { tag: '来自你的知识库', text: '依据《流式断线恢复决策》与协议 v2.1 #3.3，若缺少有效 event_id 或返回 410 Gone，应回退整段重算。', citationIds: ['c2'] },
              { tag: 'AI 补充建议（未入库）', text: '可在重连前对比本地最后确认字节数作为二级校验。', citationIds: [] }
            ],
            savedToWiki: false
          });
          render();
        }
        else if (act === 'ask-from-search') {
          const q = S.searchQuery.trim();
          S.searchOpen = false;
          setRightOpen(true);
          S.rightTab = 'chat';
          S.chatMode = 'query';
          S.chatMessages.push({ role: 'user', text: q });
          S.chatMessages.push({
            role: 'assistant',
            layers: [{ tag: '来自你的知识库', text: `围绕「${q}」命中 2 处原文记录，点击脚注可右侧分屏对照。`, citationIds: ['c2'] }],
            savedToWiki: false
          });
          render();
        }
        else if (act === 'close-evidence') { S.evidenceByPage[S.activePageId] = null; render(); }
        else if (act === 'flush-buffer') flushBufferCompile(false);
        else if (act === 'toggle-graph-meta') { S.graphShowMetaPages = !S.graphShowMetaPages; render(); }
        else if (act === 'ingest-url') {
          const val = $('#inbox-url-input')?.value?.trim();
          if (!val) { showToast('请先粘贴网页链接或文字'); return; }
          const isUrl = /^https?:\/\//i.test(val);
          const newId = `inbox-${Date.now()}`;
          S.inbox.unshift({
            id: newId,
            title: isUrl ? `收集的文章：${val.replace(/^https?:\/\//, '').slice(0, 32)}` : val.slice(0, 28),
            domain: isUrl ? val.replace(/^https?:\/\//, '').split('/')[0] : 'notes.local',
            sourceType: isUrl ? '网页文章' : '粘贴长文',
            url: isUrl ? val : '',
            savedAt: '刚刚',
            readTime: '约 1 分钟',
            fetchStatus: 'full',
            state: 'unread',
            wordCount: val.length,
            summaryPoints: ['已保存为本地完整原文，你可以先阅读，点击任意段落标为重点后再提炼进知识库。'],
            relatedPageIds: [],
            paragraphs: [{ id: 'p-auto', kind: 'section', heading: '收集的正文', text: val, highlighted: false, compiled: false }]
          });
          openInboxItem(newId);
          showToast('已收入收件箱，可直接阅读原文');
        }
        else if (act === 'dismiss-gap') { S.graph.gap.dismissed = true; showToast('已忽略该关联建议'); render(); }
        else if (act === 'ask-gap') {
          setRightOpen(true);
          S.rightTab = 'chat';
          S.chatMode = 'query';
          S.chatMessages.push({ role: 'user', text: S.graph.gap.question });
          S.chatMessages.push({
            role: 'assistant',
            layers: [{ tag: '知识库缺口提示', text: '《端侧本地缓存淘汰策略》尚未说明淘汰时是否同步失效 event_id 游标，建议补充记录。', citationIds: [] }],
            savedToWiki: false
          });
          render();
        }
        else if (act === 'win-minimize') { /* Windows taskbar minimize */ }
        else if (act === 'toast') showToast(btn.dataset.msg || '操作完成');
      });
    });

    const searchInput = $('#global-search-input');
    if (searchInput) {
      searchInput.addEventListener('focus', () => { if (S.searchQuery.trim() && !S.searchOpen) { S.searchOpen = true; render(); } });
      searchInput.addEventListener('input', (e) => {
        S.searchQuery = e.target.value;
        S.searchOpen = Boolean(S.searchQuery.trim());
        render();
      });
      searchInput.addEventListener('keydown', (e) => {
        if (e.key === 'Escape') { S.searchOpen = false; searchInput.blur(); render(); }
        else if (e.key === 'Enter' && S.searchQuery.trim()) {
          e.preventDefault();
          document.querySelector('[data-action="ask-from-search"]')?.click();
        }
      });
    }
  }

  $('#modal-layer').addEventListener('click', (e) => {
    if (e.target === $('#modal-layer')) $('#modal-layer').classList.add('hidden');
  });

  window.addEventListener('resize', () => render());
  window.addEventListener('keydown', (e) => {
    if (e.key === 'Escape' && !$('#modal-layer').classList.contains('hidden')) {
      e.preventDefault();
      $('#modal-layer').classList.add('hidden');
      return;
    }
    if (e.ctrlKey && e.key.toLowerCase() === 'k') { e.preventDefault(); $('#global-search-input')?.focus(); }
    else if (e.ctrlKey && e.key.toLowerCase() === 'e' && S.activeWorkspace === 'docs' && S.activeDocTab.startsWith('page:')) {
      e.preventDefault(); S.editorMode = S.editorMode === 'preview' ? 'source' : 'preview'; render();
    } else if (e.ctrlKey && e.key.toLowerCase() === 'j') {
      e.preventDefault(); setRightOpen(!isRightOpen()); render();
    } else if (e.ctrlKey && e.key.toLowerCase() === 'g') {
      e.preventDefault(); switchWorkspace('graph');
    }
  });

  // Prototype helper protocol (supports A Dark / C Light + Readable/Full width + Density)
  const TWEAK_KEY = 'fmd-v2-prototype-tweaks-r3';
  const tweakState = { ...window.FMD_TWEAK_DEFAULTS, ...(JSON.parse(localStorage.getItem(TWEAK_KEY) || 'null') || {}) };
  if (tweakState.theme !== 'slate' && tweakState.theme !== 'paper') tweakState.theme = 'slate';

  function applyTweaks() {
    document.body.dataset.theme = tweakState.theme;
    document.body.dataset.density = tweakState.density;
    document.body.dataset.width = tweakState.width || 'readable';
    document.querySelectorAll('[data-tweak-theme]').forEach((btn) => btn.classList.toggle('active', btn.dataset.tweakTheme === tweakState.theme));
    const dSel = $('#tweak-density'); if (dSel) dSel.value = tweakState.density;
    const wSel = $('#tweak-width'); if (wSel) wSel.value = tweakState.width || 'readable';
  }

  function updateTweak(key, value) {
    tweakState[key] = value;
    localStorage.setItem(TWEAK_KEY, JSON.stringify(tweakState));
    applyTweaks();
    render();
    try { window.parent.postMessage({ type: '__edit_mode_set_keys', edits: { [key]: value } }, '*'); } catch {}
  }

  window.addEventListener('message', (e) => {
    if (e.data?.type === '__activate_edit_mode') $('#tweaks-panel').classList.add('open');
    if (e.data?.type === '__deactivate_edit_mode') $('#tweaks-panel').classList.remove('open');
  });
  try { window.parent.postMessage({ type: '__edit_mode_available' }, '*'); } catch {}

  $('#tweaks-toggle').addEventListener('click', () => $('#tweaks-panel').classList.toggle('open'));
  $('#tweaks-close').addEventListener('click', () => $('#tweaks-panel').classList.remove('open'));
  document.querySelectorAll('[data-tweak-theme]').forEach((btn) => btn.addEventListener('click', () => updateTweak('theme', btn.dataset.tweakTheme)));
  $('#tweak-density').addEventListener('change', (e) => updateTweak('density', e.target.value));
  $('#tweak-width').addEventListener('change', (e) => updateTweak('width', e.target.value));
  $('#tweak-open-hud').addEventListener('click', () => { $('#tweaks-panel').classList.remove('open'); openHud(); });
  $('#tweak-open-onboard').addEventListener('click', () => { $('#tweaks-panel').classList.remove('open'); openOnboardingModal(); });

  applyTweaks();
  render();
})();
