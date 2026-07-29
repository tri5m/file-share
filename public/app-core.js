(function () {
  const {
    $,
    copyText,
    escapeHtml,
    formatSize,
    formatSpeed,
    formatTextLength,
    formatTime,
    previewText,
    t
  } = window.FileShareUtils;

  const MAX_PATHLESS_PASTED_ADMIN_FILE_BYTES = 50 * 1024 * 1024;
  const SERVER_BUTTON_ANIMATION_MS = 780;

  const state = {
    role: 'client',
    items: [],
    downloadStats: {},
    apiBase: '',
    events: null,
    downloadEvents: null,
    isTauri: false,
    serverRunning: false,
    shareInfo: null,
    selectedAddressUrl: '',
    statusTimer: null,
    toastId: 0,
    reconnectToastTimer: null,
    adminFileSharing: false,
    adminDragDropBound: false,
    adminDropFeedbackTimer: null,
    serverButtonAnimation: null,
    serverButtonAnimationTarget: null,
    serverVisualStopped: true,
    serverTransitionToken: 0,
    serverDesiredRunning: false,
    serverDesiredPort: 5421,
    serverCommandSync: null
  };

  function isEditableTarget(target) {
    if (!target) return false;
    return Boolean(target.closest?.('input, textarea, select, [contenteditable="true"]'));
  }

  function showToast(message, type = 'info', duration = 3200) {
    if (!message) return;

    let stack = $('globalToast');
    if (!stack) {
      stack = document.createElement('div');
      stack.id = 'globalToast';
      stack.className = 'global-toast-stack';
      stack.setAttribute('aria-live', 'polite');
      document.body.appendChild(stack);
    }

    const toast = document.createElement('div');
    const toastId = `toast-${++state.toastId}`;
    toast.className = `global-toast ${type}`;
    toast.id = toastId;
    toast.innerHTML = `<span class="global-toast-message"></span><button class="global-toast-close" type="button" aria-label="${t('closeTip')}">×</button>`;
    const messageNode = toast.querySelector('.global-toast-message');
    if (messageNode) messageNode.textContent = message;
    toast.querySelector('.global-toast-close')?.addEventListener('click', () => hideToast(toast));
    stack.appendChild(toast);
    requestAnimationFrame(() => toast.classList.add('show'));

    window.setTimeout(() => hideToast(toast), duration);
  }

  function hideToast(toast) {
    if (!toast) return;
    toast.classList.remove('show');
    window.setTimeout(() => {
      toast.remove();
    }, 180);
  }

  function getDownloadUrl(item) {
    const path = `/api/items/${item.id}/download`;
    const selectedUrl = state.role === 'admin' ? currentShareUrl() : window.location.origin;
    if (selectedUrl) {
      return new URL(path, new URL(selectedUrl).origin).toString();
    }
    return new URL(path, window.location.origin).toString();
  }

  function shareAddresses(info = state.shareInfo) {
    if (!info) return [];
    const addresses = Array.isArray(info.addresses) ? info.addresses : [];
    if (addresses.length) return addresses;
    return info.url ? [{ name: '', ip: info.ip || new URL(info.url).hostname, url: info.url, qr: info.qr }] : [];
  }

  function formatAddressLabel(address) {
    const ip = address.ip || new URL(address.url).hostname;
    return address.name ? `${address.name} · ${ip}` : ip;
  }

  function currentShareAddress() {
    const addresses = shareAddresses();
    if (!addresses.length) return null;
    return addresses.find((item) => item.url === state.selectedAddressUrl) || addresses[0];
  }

  function currentShareUrl() {
    return currentShareAddress()?.url || state.shareInfo?.url || '';
  }

  function clientShareAddress() {
    const currentUrl = window.location.origin;
    const match = shareAddresses().find((item) => {
      try {
        return new URL(item.url).origin === currentUrl;
      } catch (_) {
        return false;
      }
    });
    return { url: currentUrl, qr: match?.qr || state.shareInfo?.qr };
  }

  async function request(url, options = {}) {
    const response = await fetch(`${state.apiBase}${url}`, options);
    if (!response.ok) {
      const data = await response.json().catch(() => ({}));
      throw new Error(data.error || t('requestFailed', { status: response.status }));
    }
    return response.json();
  }

  function render() {
    const root = $('items');
    if (!state.items.length) {
      root.innerHTML = `<div class="empty">${t('empty')}</div>`;
      return;
    }

    root.innerHTML = state.items.map((item) => {
      const isText = item.kind === 'text';
      const isMissing = item.kind !== 'text' && item.exists === false;
      const badge = isText ? t('textBadge') : t('fileBadge');
      const title = isText
        ? escapeHtml(previewText(item.content || item.title, 40))
        : escapeHtml(item.title);
      const titleAction = isText
        ? ''
        : `<button class="inline-icon-button" data-action="copy-link" data-id="${item.id}" aria-label="${t('copyLink')}" title="${t('copyLink')}">🔗</button>`;
      const downloadStatus = state.role === 'admin' && !isText
        ? `<span class="download-status" data-download-status="${item.id}" hidden></span>`
        : '';
      const description = isText
        ? `<div class="meta">${formatTextLength(item.content || item.title)} · ${formatTime(item.createdAt)}</div>`
        : `<div class="meta">${formatSize(item.size)} · ${formatTime(item.createdAt)}${downloadStatus}</div>`;
      const primaryAction = isText
        ? `<button class="secondary" data-action="copy" data-id="${item.id}">${t('copy')}</button>`
        : (isMissing
          ? `<button class="secondary" disabled type="button">${t('invalid')}</button>`
          : (state.role === 'admin'
            ? `<button class="secondary" data-action="reveal" data-id="${item.id}">${t('reveal')}</button>`
            : `<button class="secondary" data-action="download" data-id="${item.id}">${t('download')}</button>`));
      const actions = state.role === 'admin'
        ? `${primaryAction}<button class="secondary remove-action" data-action="delete" data-id="${item.id}">${t('remove')}</button>`
        : primaryAction;
      return `
        <article class="item${isMissing ? ' item-missing' : ''}">
          <div class="item-main">
            <div class="item-title">
              <span class="badge">${badge}</span>
              <strong>${title}</strong>
              ${titleAction}
            </div>
            ${description}
          </div>
          <div class="actions">${actions}</div>
        </article>
      `;
    }).join('');
    updateDownloadStatuses();
  }

  function updateDownloadStatuses() {
    document.querySelectorAll('[data-download-status]').forEach((node) => {
      const stat = state.downloadStats[node.dataset.downloadStatus];
      if (stat) {
        node.textContent = `⬇ ${formatSpeed(stat.speedBps)}`;
        node.hidden = false;
      } else {
        node.textContent = '';
        node.hidden = true;
      }
    });
  }

  async function loadItems() {
    state.items = await request('/api/items');
    render();
  }

  async function loadClientShare() {
    const qr = $('clientQr');
    const text = $('clientUrlText');
    const copy = $('copyClientUrl');
    if (!qr || !text || !copy) return;

    const data = await request('/api/share-info');
    applyShareInfo(data);
  }

  async function applyServerInfo(info, syncDesired = !state.serverCommandSync) {
    const wasRunning = state.serverRunning;
    state.apiBase = `http://127.0.0.1:${info.port}`;
    state.serverRunning = true;
    if (syncDesired) {
      state.serverDesiredRunning = true;
      setServerStopped(false);
    }
    applyShareInfo(info);
    setServerControlsRunning(info);

    if (!wasRunning || !state.events || state.shareInfo?.port !== info.port) {
      await loadItems();
      connectEvents();
    }
  }

  function applyShareInfo(info) {
    state.shareInfo = info;
    const qr = $('clientQr');
    const text = $('clientUrlText');
    const copy = $('copyClientUrl');
    const select = $('clientAddressSelect');
    const addressPicker = select?.closest('.address-picker');
    const addresses = shareAddresses(info);
    if (state.role === 'admin' && select) {
      const previousUrl = state.selectedAddressUrl;
      const selected = addresses.find((item) => item.url === previousUrl)
        || addresses[0]
        || null;
      state.selectedAddressUrl = selected?.url || '';
      select.innerHTML = addresses.map((item) => {
        const label = formatAddressLabel(item);
        return `<option value="${escapeHtml(item.url)}"${item.url === state.selectedAddressUrl ? ' selected' : ''}>${escapeHtml(label)}</option>`;
      }).join('');
      if (addressPicker) addressPicker.hidden = addresses.length <= 1;
      select.onchange = () => {
        state.selectedAddressUrl = select.value;
        renderShareAddress();
        render();
      };
    } else {
      state.selectedAddressUrl = '';
    }
    renderShareAddress();
    if (copy) {
      copy.onclick = async () => {
        const copied = await copyText(state.role === 'admin' ? currentShareUrl() : window.location.origin);
        copy.classList.toggle('copied', copied);
        copy.classList.toggle('copy-failed', !copied);
        showToast(copied ? t('linkCopied') : t('copyFailed'), copied ? 'success' : 'error');
        setTimeout(() => copy.classList.remove('copied', 'copy-failed'), 1200);
      };
    }
  }

  function renderShareAddress() {
    const address = state.role === 'admin'
      ? currentShareAddress()
      : clientShareAddress();
    const qr = $('clientQr');
    const text = $('clientUrlText');
    if (!address) return;
    if (qr) {
      qr.innerHTML = address.qr;
    }
    if (text) {
      text.textContent = address.url;
      text.title = address.url;
    }
  }

  function readServerButtonFrame(button) {
    const style = window.getComputedStyle(button);
    const rect = button.getBoundingClientRect();
    const layoutWidth = Number.parseFloat(style.width) || rect.width;
    const layoutHeight = Number.parseFloat(style.height) || rect.height;
    const scaleY = layoutHeight ? rect.height / layoutHeight : 1;
    return {
      centerX: rect.left + rect.width / 2,
      centerY: rect.top + rect.height / 2,
      width: rect.width,
      height: rect.height,
      fontSize: (Number.parseFloat(style.fontSize) || 13) * scaleY
    };
  }

  function setServerStopped(stopped, animate = true) {
    const body = document.body;
    const button = $('startServerButton');
    const target = stopped ? 'stopped' : 'running';
    const alreadyAtTarget = state.serverVisualStopped === stopped;

    if (alreadyAtTarget) {
      return state.serverButtonAnimationTarget === target
        ? state.serverButtonAnimation?.finished?.catch(() => {}) || Promise.resolve()
        : Promise.resolve();
    }

    state.serverVisualStopped = stopped;
    const transitionToken = ++state.serverTransitionToken;

    if (!button || !animate || typeof button.animate !== 'function') {
      state.serverButtonAnimation?.cancel();
      state.serverButtonAnimation = null;
      state.serverButtonAnimationTarget = null;
      button?.classList.remove('share-button-animating');
      button?.classList.toggle('share-button-stopped', stopped);
      body.classList.remove('server-ui-hiding', 'server-ui-revealing');
      body.classList.toggle('server-stopped', stopped);
      return Promise.resolve();
    }

    const from = readServerButtonFrame(button);
    state.serverButtonAnimation?.cancel();
    button.classList.add('share-button-animating');

    if (stopped) {
      body.classList.remove('server-stopped', 'server-ui-revealing');
      body.classList.add('server-ui-hiding');
      button.classList.add('share-button-stopped');
    } else {
      const startsFromStoppedLayout = body.classList.contains('server-stopped');
      if (startsFromStoppedLayout) {
        body.classList.add('server-ui-revealing');
        body.classList.remove('server-stopped');
        body.offsetWidth;
        requestAnimationFrame(() => {
          if (state.serverTransitionToken === transitionToken && !state.serverVisualStopped) {
            body.classList.remove('server-ui-revealing');
          }
        });
      } else {
        body.classList.remove('server-ui-hiding', 'server-ui-revealing');
      }
      button.classList.remove('share-button-stopped');
    }

    const to = readServerButtonFrame(button);
    const scaleX = to.width ? from.width / to.width : 1;
    const scaleY = to.height ? from.height / to.height : 1;
    const translateX = from.centerX - to.centerX;
    const translateY = from.centerY - to.centerY;
    const startFontSize = scaleY ? from.fontSize / scaleY : from.fontSize;
    const baseTransform = 'translate(-50%, -50%)';

    const animation = button.animate([
      {
        transform: `${baseTransform} translate(${translateX}px, ${translateY}px) scale(${scaleX}, ${scaleY})`,
        fontSize: `${startFontSize}px`
      },
      {
        transform: `${baseTransform} translate(0, 0) scale(1, 1)`,
        fontSize: `${to.fontSize}px`
      }
    ], {
      duration: SERVER_BUTTON_ANIMATION_MS,
      easing: 'cubic-bezier(0.4, 0, 0.2, 1)',
      fill: 'both'
    });
    state.serverButtonAnimation = animation;
    state.serverButtonAnimationTarget = target;

    return animation.finished
      .catch(() => {})
      .then(() => {
        if (state.serverTransitionToken !== transitionToken) return;
        body.classList.remove('server-ui-hiding', 'server-ui-revealing');
        body.classList.toggle('server-stopped', stopped);
      })
      .finally(() => {
        if (state.serverButtonAnimation !== animation) return;
        animation.cancel();
        button.classList.remove('share-button-animating');
        state.serverButtonAnimation = null;
        state.serverButtonAnimationTarget = null;
      });
  }

  function resetServerUi(animate = true, syncDesired = !state.serverCommandSync) {
    state.serverRunning = false;
    state.apiBase = '';
    state.items = [];
    state.downloadStats = {};
    if (state.events) {
      state.events.__manuallyClosed = true;
      state.events.close();
      state.events = null;
    }
    if (state.downloadEvents) {
      state.downloadEvents.__manuallyClosed = true;
      state.downloadEvents.close();
      state.downloadEvents = null;
    }
    const root = $('items');
    if (root) root.innerHTML = '';
    if (syncDesired) {
      state.serverDesiredRunning = false;
      setServerStopped(true, animate);
    }
    $('clientQr') && ($('clientQr').innerHTML = '');
    $('clientUrlText') && ($('clientUrlText').textContent = t('serviceNotStarted'));
    const select = $('clientAddressSelect');
    if (select) {
      select.innerHTML = '';
      const addressPicker = select.closest('.address-picker');
      if (addressPicker) addressPicker.hidden = true;
    }
    $('status') && ($('status').textContent = t('notStarted'));
    state.shareInfo = null;
    state.selectedAddressUrl = '';
    setServerControlsStopped();
  }

  async function syncServerStatus() {
    if (!state.isTauri || state.role !== 'admin' || !window.__TAURI__?.core?.invoke || state.serverCommandSync) return;

    try {
      const info = await window.__TAURI__.core.invoke('server_status');
      if (info) {
        await applyServerInfo(info);
      } else if (state.serverRunning) {
        resetServerUi();
      }
    } catch (error) {
      console.warn('Failed to sync server status:', error);
    }
  }

  function startServerStatusSync() {
    if (state.statusTimer) return;
    syncServerStatus();
    // 托盘也能启动/停止服务，管理端窗口需要持续同步后台状态。
    state.statusTimer = window.setInterval(syncServerStatus, 1000);
  }

  function setServerControlsRunning(info) {
    const portInput = $('serverPort');
    const button = $('startServerButton');
    if (portInput) {
      portInput.value = info.port;
      portInput.disabled = true;
    }
    if (button) {
      button.disabled = false;
      button.textContent = t(state.serverDesiredRunning ? 'stopShare' : 'startShare');
    }
  }

  function setServerControlsStopped() {
    const portInput = $('serverPort');
    const button = $('startServerButton');
    if (portInput) {
      portInput.disabled = state.serverDesiredRunning;
    }
    if (button) {
      button.disabled = false;
      button.textContent = t(state.serverDesiredRunning ? 'stopShare' : 'startShare');
    }
  }

  function updateServerControls() {
    if (state.serverRunning && state.shareInfo) {
      setServerControlsRunning(state.shareInfo);
    } else {
      setServerControlsStopped();
    }
  }

  function reconcileServerState() {
    if (state.serverCommandSync) return state.serverCommandSync;

    const commandSync = (async () => {
      try {
        while (state.serverRunning !== state.serverDesiredRunning) {
          if (state.serverDesiredRunning) {
            const info = await window.__TAURI__.core.invoke('start_server', {
              port: state.serverDesiredPort
            });
            await applyServerInfo(info, false);
          } else {
            await window.__TAURI__.core.invoke('stop_server');
            resetServerUi(false, false);
          }
          updateServerControls();
        }
      } catch (error) {
        state.serverDesiredRunning = state.serverRunning;
        setServerStopped(!state.serverRunning);
        updateServerControls();
        showToast(String(error), 'error');
      }
    })();

    state.serverCommandSync = commandSync.finally(() => {
      state.serverCommandSync = null;
      if (state.serverRunning !== state.serverDesiredRunning) {
        reconcileServerState();
      }
    });
    return state.serverCommandSync;
  }

  function bindServerControls() {
    const form = $('serverForm');
    const portInput = $('serverPort');
    const button = $('startServerButton');
    if (!form || !portInput || !button) return;

    resetServerUi(false);
    state.serverDesiredPort = Number(portInput.value) || 5421;
    portInput.addEventListener('input', () => {
      const port = Number(portInput.value);
      if (Number.isInteger(port) && port > 0 && port <= 65535) {
        state.serverDesiredPort = port;
        window.__TAURI__.core.invoke('set_preferred_port', { port }).catch(() => {});
      }
    });

    form.addEventListener('submit', (event) => {
      event.preventDefault();
      const nextRunning = !state.serverDesiredRunning;
      if (nextRunning) {
        const port = Number(portInput.value);
        if (!Number.isInteger(port) || port < 1 || port > 65535) {
          showToast(t('validPort'), 'warning');
          return;
        }
        state.serverDesiredPort = port;
      }

      state.serverDesiredRunning = nextRunning;
      button.textContent = t(nextRunning ? 'stopShare' : 'startShare');
      setServerStopped(!nextRunning);
      updateServerControls();
      reconcileServerState();
    });
  }

  function connectEvents() {
    const status = $('status');
    if (state.events) {
      state.events.__manuallyClosed = true;
      state.events.close();
    }
    // 共享列表由服务端 SSE 推送，管理端和客户端保持同一份实时视图。
    const events = new EventSource(`${state.apiBase}/api/events`);
    state.events = events;
    events.__opened = false;
    events.__manuallyClosed = false;
    events.onopen = () => {
      events.__opened = true;
      status.textContent = t('synced');
    };
    events.onerror = () => {
      if (events.__manuallyClosed || state.events !== events || (state.role === 'admin' && !state.serverRunning)) return;
      status.textContent = t('reconnecting');
      if (events.__opened && !state.reconnectToastTimer) {
        showToast(t('reconnectToast'), 'warning', 2200);
        state.reconnectToastTimer = window.setTimeout(() => {
          state.reconnectToastTimer = null;
        }, 3000);
      }
    };
    events.onmessage = (event) => {
      state.items = JSON.parse(event.data);
      render();
    };
    if (state.role === 'admin') {
      connectDownloadEvents();
    }
  }

  function connectDownloadEvents() {
    if (state.downloadEvents) {
      state.downloadEvents.__manuallyClosed = true;
      state.downloadEvents.close();
    }
    const events = new EventSource(`${state.apiBase}/api/download-events`);
    state.downloadEvents = events;
    events.onerror = () => {
      if (events.__manuallyClosed || state.downloadEvents !== events) return;
      state.downloadStats = {};
      updateDownloadStatuses();
    };
    events.onmessage = (event) => {
      const stats = JSON.parse(event.data);
      state.downloadStats = Object.fromEntries(
        stats
          .filter((item) => item.activeCount > 0)
          .map((item) => [item.itemId, item])
      );
      updateDownloadStatuses();
    };
  }

  async function addAdminLocalFiles(paths) {
    if (!paths?.length || state.adminFileSharing) return;

    const fileHints = document.querySelectorAll('.admin-file-hint');
    const fileForm = $('fileForm');
    const button = $('pickAdminFilesButton');

    state.adminFileSharing = true;
    if (button) {
      button.disabled = true;
      button.textContent = t('sharing');
    }
    fileHints.forEach((hint) => {
      hint.textContent = t('selectedFiles', { count: paths.length });
    });

    try {
      // 这里只提交本机路径，实际文件仍留在原位置，避免复制大文件。
      await request('/api/local-file', {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ paths })
      });
      fileHints.forEach((hint) => {
        hint.textContent = t('shareDropHint');
      });
      fileForm?.closest('dialog')?.close();
    } finally {
      state.adminFileSharing = false;
      if (button) {
        button.disabled = false;
        button.textContent = t('pickAndShare');
      }
    }
  }

  async function shareAdminFiles() {
    if (!window.__TAURI__?.core?.invoke) {
      throw new Error(t('unsupportedPicker'));
    }

    const button = $('pickAdminFilesButton');

    button.disabled = true;
    button.textContent = t('selecting');
    try {
      const paths = await window.__TAURI__.core.invoke('pick_admin_files');
      await addAdminLocalFiles(paths);
    } finally {
      if (!state.adminFileSharing) {
        button.disabled = false;
        button.textContent = t('pickAndShare');
      }
    }
  }

  function setAdminDropActive(active) {
    document.querySelectorAll('.admin-file-drop').forEach((dropZone) => {
      dropZone.classList.toggle('dragging', active);
    });
  }

  function setAdminDropPresentation(mode, count = 0) {
    const dropZones = document.querySelectorAll('.admin-file-drop');
    if (!dropZones.length) return;

    window.clearTimeout(state.adminDropFeedbackTimer);

    dropZones.forEach((dropZone) => {
      const title = dropZone.querySelector('strong');
      const hint = dropZone.querySelector('.admin-file-hint');

      dropZone.classList.remove('dragging', 'shared', 'pending');

      if (mode === 'dragging') {
        dropZone.classList.add('dragging');
        if (title) title.textContent = t('dropRelease');
        if (hint) hint.textContent = t('localPathHint');
        return;
      }

      if (mode === 'pending') {
        dropZone.classList.add('pending');
        if (title) title.textContent = t('sharingFiles');
        if (hint) hint.textContent = t('pleaseWait');
        return;
      }

      if (mode === 'shared') {
        dropZone.classList.add('shared');
        if (title) title.textContent = t('sharedFiles', { count });
        if (hint) hint.textContent = t('fileAdded');
        return;
      }

      if (title) title.textContent = t('shareDropTitle');
      if (hint) hint.textContent = t('shareDropHint');
    });
  }

  function flashAdminDropSuccess(count) {
    setAdminDropPresentation('shared', count);

    state.adminDropFeedbackTimer = window.setTimeout(() => {
      setAdminDropPresentation('idle');
    }, 1600);
  }

  function showStatusMessage(message) {
    showToast(message, 'error');
  }

  function showUpdateHint(message) {
    showToast(message, 'success', 10000);
  }

  async function publishTextContent(content) {
    const text = String(content || '').trim();
    if (!text) return;

    await request('/api/text', {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ content: text, source: state.role })
    });
  }

  async function uploadFiles(files, options = {}) {
    const pickedFiles = Array.from(files || []).filter((file) => file?.size > 0);
    if (!pickedFiles.length) return [];

    options.onStart?.(pickedFiles);
    try {
      const data = new FormData();
      data.append('source', state.role);
      for (const file of pickedFiles) {
        data.append('file', file, file.name || 'file');
      }
      const result = await request('/api/upload', { method: 'POST', body: data });
      options.onSuccess?.(pickedFiles, result);
      return result;
    } catch (error) {
      options.onError?.(error);
      throw error;
    } finally {
      options.onDone?.();
    }
  }

  function fileToBase64(file) {
    return new Promise((resolve, reject) => {
      const reader = new FileReader();
      reader.addEventListener('load', () => {
        const result = String(reader.result || '');
        resolve(result.includes(',') ? result.split(',').pop() : result);
      });
      reader.addEventListener('error', () => reject(reader.error || new Error(t('uploadFailed'))));
      reader.readAsDataURL(file);
    });
  }

  function localPathFromFile(file) {
    const candidates = [
      file?.path,
      file?.filepath,
      file?.mozFullPath,
      file?.webkitRelativePath
    ];
    return candidates.find((value) => (
      typeof value === 'string'
      && (value.startsWith('/') || /^[A-Za-z]:[\\/]/.test(value) || value.startsWith('\\\\'))
    )) || '';
  }

  function hasDirectoryItems(items) {
    return Array.from(items || []).some((item) => {
      const entry = item?.webkitGetAsEntry?.();
      return Boolean(entry?.isDirectory);
    });
  }

  async function sharePastedAdminFiles(files, options = {}) {
    const pickedFiles = Array.from(files || []).filter((file) => file?.size > 0);
    if (!pickedFiles.length) return 0;

    options.onStart?.(pickedFiles);
    try {
      const localPaths = pickedFiles.map(localPathFromFile).filter(Boolean);
      const filesWithoutPath = pickedFiles.filter((file) => !localPathFromFile(file));
      const oversizedFile = filesWithoutPath.find((file) => file.size > MAX_PATHLESS_PASTED_ADMIN_FILE_BYTES);
      if (oversizedFile) {
        throw new Error(t('pasteFileTooLarge', { size: '50MB' }));
      }
      let count = 0;

      if (localPaths.length) {
        await addAdminLocalFiles(localPaths);
        count += localPaths.length;
      }

      const payload = [];
      for (const file of filesWithoutPath) {
        payload.push({
          name: file.name || 'file',
          data: await fileToBase64(file)
        });
      }

      if (payload.length) {
        count += await window.__TAURI__.core.invoke('share_pasted_admin_files', { pathlessFiles: payload });
      }
      options.onSuccess?.(pickedFiles, count);
      return count;
    } catch (error) {
      options.onError?.(error);
      throw error;
    } finally {
      options.onDone?.();
    }
  }

  async function bindAdminFileDrop() {
    if (state.role !== 'admin' || state.adminDragDropBound) return;

    const currentWebview = window.__TAURI__?.webview?.getCurrentWebview?.()
      || window.__TAURI__?.webviewWindow?.getCurrentWebviewWindow?.()
      || window.__TAURI__?.window?.getCurrentWindow?.();
    const listen = window.__TAURI__?.event?.listen;
    if (!currentWebview?.onDragDropEvent && !listen) return;

    state.adminDragDropBound = true;
    const handleDragDrop = (event) => {
      const { payload } = event;

      if (payload.type === 'enter' || payload.type === 'over') {
        setAdminDropPresentation('dragging');
        return;
      }

      if (payload.type === 'drop') {
        setAdminDropPresentation('pending');
        return;
      }

      if (payload.type === 'cancel' || payload.type === 'leave') {
        setAdminDropPresentation('idle');
      }
    };

    if (listen) {
      await listen('admin-file-drop', (event) => handleDragDrop(event));
    }

    if (currentWebview?.onDragDropEvent) {
      await currentWebview.onDragDropEvent(handleDragDrop);
      return;
    }

    await Promise.all([
      listen('tauri://drag-enter', (event) => handleDragDrop({
        ...event,
        payload: { ...event.payload, type: 'enter' }
      })),
      listen('tauri://drag-over', (event) => handleDragDrop({
        ...event,
        payload: { ...event.payload, type: 'over' }
      })),
      listen('tauri://drag-drop', (event) => handleDragDrop({
        ...event,
        payload: { ...event.payload, type: 'drop' }
      })),
      listen('tauri://drag-leave', (event) => handleDragDrop({
        ...event,
        payload: { type: 'leave' }
      }))
    ]);
  }

  function bindQrPreview() {
    const button = $('clientQrButton');
    const dialog = $('qrDialog');
    const preview = $('qrDialogPreview');
    const urlText = $('qrDialogUrlText');
    const copyButton = $('qrDialogCopyButton');
    if (!button || !dialog || !preview || !urlText || !copyButton) return;

    button.addEventListener('click', () => {
      const address = state.role === 'admin'
        ? currentShareAddress()
        : clientShareAddress();
      if (!address?.qr) return;
      preview.innerHTML = address.qr;
      urlText.textContent = address.url;
      urlText.title = address.url;
      dialog.showModal();
    });

    copyButton.addEventListener('click', async () => {
      const url = state.role === 'admin' ? currentShareUrl() : window.location.origin;
      if (!url) return;
      const copied = await copyText(url);
      copyButton.textContent = copied ? t('copied') : t('copyLink');
      showToast(copied ? t('linkCopied') : t('copyFailed'), copied ? 'success' : 'error');
      setTimeout(() => { copyButton.textContent = t('copyLink'); }, 1200);
    });
  }

  function bindPasteSharing() {
    document.addEventListener('paste', (event) => {
      const clipboard = event.clipboardData;
      if (!clipboard) return;

      if (hasDirectoryItems(clipboard.items)) {
        event.preventDefault();
        showToast(t('unsupportedShareType'), 'warning');
        return;
      }

      const files = Array.from(clipboard.files || []).filter((file) => file.size > 0);
      if (files.length) {
        event.preventDefault();
        const adminPaste = state.role === 'admin' && window.__TAURI__?.core?.invoke;
        const shareFiles = adminPaste ? sharePastedAdminFiles : uploadFiles;

        if (adminPaste) setAdminDropPresentation('pending');
        shareFiles(files, {
          onStart: () => showToast(t('pastingFiles', { count: files.length })),
          onSuccess: (_files, count) => {
            const sharedCount = Number(count) || files.length;
            if (adminPaste) flashAdminDropSuccess(sharedCount);
            showToast(t('pasteFileDone', { count: sharedCount }), 'success');
          },
          onError: (error) => {
            if (adminPaste) setAdminDropPresentation('idle');
            showToast(error?.message || String(error) || t('uploadFailed'), 'error');
          }
        }).catch(() => {});
        return;
      }

      if (Array.from(clipboard.items || []).some((item) => item.kind === 'file')) {
        event.preventDefault();
        showToast(t('unsupportedShareType'), 'warning');
        return;
      }

      if (isEditableTarget(event.target)) return;

      const text = clipboard.getData('text/plain');
      if (!text.trim()) return;

      event.preventDefault();
      publishTextContent(text)
        .then(() => showToast(t('pasteTextDone'), 'success'))
        .catch((error) => showToast(error?.message || String(error), 'error'));
    });
  }

  async function bindTauriEvents() {
    const events = window.__TAURI__?.event;
    if (!events?.listen) return;

    try {
      await events.listen('share-started', async (event) => {
        try {
          await applyServerInfo(event.payload);
        } catch (error) {
          showToast(String(error), 'error');
        }
      });
      await events.listen('share-stopped', () => {
        resetServerUi();
      });
      await events.listen('share-error', (event) => {
        showToast(String(event.payload || t('operationFailed')), 'error');
      });
      await events.listen('update-available', () => {
        showUpdateHint(t('updateAvailable'));
      });
      await events.listen('admin-files-added', (event) => {
        flashAdminDropSuccess(Number(event.payload) || 0);
      });
    } catch (error) {
      console.warn('Tauri event bridge unavailable:', error);
    }
  }

  function checkStartupUpdateHint() {
    const invoke = window.__TAURI__?.core?.invoke;
    if (!invoke) return;

    let attempts = 0;
    const timer = window.setInterval(async () => {
      try {
        attempts += 1;
        if (await invoke('has_update_available')) {
          showUpdateHint(t('updateAvailable'));
          window.clearInterval(timer);
        } else if (attempts >= 12) {
          window.clearInterval(timer);
        }
      } catch (error) {
        window.clearInterval(timer);
        console.warn('Update hint check unavailable:', error);
      }
    }, 2500);
  }

  function bindForms() {
    document.querySelectorAll('[data-open]').forEach((button) => {
      button.addEventListener('click', () => {
        const dialog = $(button.dataset.open);
        if (!dialog) return;
        dialog.showModal();
        const selector = dialog.dataset.focus;
        if (selector) {
          const field = dialog.querySelector(selector);
          if (field) {
            requestAnimationFrame(() => field.focus());
          }
        }
      });
    });

    $('textForm').addEventListener('submit', async (event) => {
      event.preventDefault();
      const content = $('textContent').value.trim();
      if (!content) return;
      await publishTextContent(content);
      event.target.reset();
      event.target.closest('dialog')?.close();
    });

    const fileForm = $('fileForm');
    const fileInput = $('fileInput');
    const dropZone = $('dropZone');
    const fileHint = $('fileHint');
    const pickAdminFilesButton = $('pickAdminFilesButton');
    const adminDropZones = Array.from(document.querySelectorAll('.admin-file-drop'));
    if (fileForm && fileInput) {
      let uploadingFiles = false;

      const updateFileHint = (message) => {
        if (!fileHint) return;
        if (message) {
          fileHint.textContent = message;
          return;
        }
        const count = fileInput.files.length;
        fileHint.textContent = count ? t('selectedAndPreparing', { count }) : t('autoUploadHint');
      };

      const uploadSelectedFiles = async () => {
        if (!fileInput.files.length || uploadingFiles) return;
        const files = Array.from(fileInput.files);
        uploadingFiles = true;
        updateFileHint(t('uploadingFiles', { count: files.length }));
        if (dropZone) {
          dropZone.classList.add('uploading');
        }
        try {
          await uploadFiles(files);
          fileForm.reset();
          updateFileHint(t('uploadDone'));
          window.setTimeout(() => updateFileHint(), 900);
          fileForm.closest('dialog')?.close();
        } finally {
          uploadingFiles = false;
          if (dropZone) {
            dropZone.classList.remove('uploading');
          }
        }
      };

      fileInput.addEventListener('change', () => {
        if (!fileInput.files.length) return;
        updateFileHint();
        uploadSelectedFiles().catch((error) => {
          uploadingFiles = false;
          if (dropZone) {
            dropZone.classList.remove('uploading');
          }
          updateFileHint(t('uploadFailed'));
          showToast(error.message, 'error');
        });
      });

      if (dropZone) {
        ['dragenter', 'dragover'].forEach((name) => {
          dropZone.addEventListener(name, (event) => {
            event.preventDefault();
            if (hasDirectoryItems(event.dataTransfer?.items)) {
              dropZone.classList.remove('dragging');
              return;
            }
            dropZone.classList.add('dragging');
          });
        });
        ['dragleave', 'drop'].forEach((name) => {
          dropZone.addEventListener(name, (event) => {
            event.preventDefault();
            dropZone.classList.remove('dragging');
          });
        });
        dropZone.addEventListener('drop', (event) => {
          if (hasDirectoryItems(event.dataTransfer?.items)) {
            showToast(t('unsupportedShareType'), 'warning');
            return;
          }
          if (event.dataTransfer?.files?.length) {
            fileInput.files = event.dataTransfer.files;
            uploadSelectedFiles().catch((error) => showToast(error.message, 'error'));
          }
        });
      }

      fileForm.addEventListener('submit', async (event) => {
        event.preventDefault();
        await uploadSelectedFiles();
      });
    }

    if (state.role === 'admin' && pickAdminFilesButton) {
      const openAdminPicker = (event) => {
        event?.preventDefault();
        shareAdminFiles().catch((error) => showToast(String(error), 'error'));
      };

      pickAdminFilesButton.addEventListener('click', openAdminPicker);
      adminDropZones.forEach((dropZone) => {
        dropZone.addEventListener('click', openAdminPicker);
        dropZone.tabIndex = 0;
        dropZone.setAttribute('role', 'button');
        dropZone.addEventListener('keydown', (event) => {
          if (event.key === 'Enter' || event.key === ' ') {
            openAdminPicker(event);
          }
        });
      });
    }

    $('items').addEventListener('click', async (event) => {
      const button = event.target.closest('button[data-action]');
      if (!button) return;
      if (button.disabled) return;
      const { action, id } = button.dataset;
      if (action === 'reveal' && state.role === 'admin') {
        const item = state.items.find((entry) => entry.id === id);
        if (!item) return;
        if (state.isTauri) {
          try {
            await window.__TAURI__.core.invoke('reveal_admin_file', { id });
          } catch (error) {
            const message = error?.message || String(error) || t('openLocationFailed');
            showToast(t('openFailed', { message }), 'error');
          }
          return;
        }
        window.open(getDownloadUrl(item), '_blank');
      }
      if (action === 'download') {
        const item = state.items.find((entry) => entry.id === id);
        if (!item) return;
        if (item.exists === false) {
          showToast(t('sourceMissing'), 'warning');
          return;
        }
        window.location.href = `${state.apiBase}/api/items/${id}/download`;
      }
      if (action === 'copy') {
        const item = state.items.find((entry) => entry.id === id);
        if (!item) return;
        const copied = await copyText(item.content || '');
        button.textContent = copied ? t('copied') : t('copy');
        showToast(copied ? t('copied') : t('copyFailed'), copied ? 'success' : 'error');
        setTimeout(() => { button.textContent = t('copy'); }, 1200);
      }
      if (action === 'copy-link') {
        const item = state.items.find((entry) => entry.id === id);
        if (!item) return;
        const copied = await copyText(getDownloadUrl(item));
        button.classList.toggle('copied', copied);
        button.classList.toggle('copy-failed', !copied);
        showToast(copied ? t('linkCopied') : t('copyFailed'), copied ? 'success' : 'error');
        setTimeout(() => {
          button.classList.remove('copied', 'copy-failed');
        }, 1200);
      }
      if (action === 'delete' && state.role === 'admin') {
        try {
          await request(`/api/items/${id}`, { method: 'DELETE' });
        } catch (error) {
          showToast(String(error), 'error');
        }
      }
    });
  }

  window.FileShareApp = {
    async init(options) {
      state.role = options.role;
      state.isTauri = Boolean(window.__TAURI_INTERNALS__) || Boolean(window.__TAURI__) || window.location.hostname === 'tauri.localhost';
      state.apiBase = options.apiBase || '';
      bindQrPreview();
      bindForms();
      bindPasteSharing();
      if (state.role === 'admin' && state.isTauri) {
        bindServerControls();
        bindTauriEvents();
        checkStartupUpdateHint();
        bindAdminFileDrop().catch((error) => console.warn('Tauri file drop unavailable:', error));
        startServerStatusSync();
        return;
      }
      await loadClientShare();
      await loadItems();
      connectEvents();
    }
  };

  window.__fileshareAdminDropFeedback = {
    dragging() {
      setAdminDropPresentation('dragging');
    },
    pending() {
      setAdminDropPresentation('pending');
    },
    shared(count) {
      flashAdminDropSuccess(Number(count) || 0);
    },
    reset() {
      setAdminDropPresentation('idle');
    }
  };
})();
