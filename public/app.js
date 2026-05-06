(function () {
  const state = {
    role: 'client',
    items: [],
    apiBase: '',
    events: null,
    isTauri: false,
    serverRunning: false,
    shareInfo: null,
    statusTimer: null
  };

  function $(id) {
    return document.getElementById(id);
  }

  function formatSize(size) {
    if (!Number.isFinite(size)) return '';
    if (size < 1024) return `${size} B`;
    if (size < 1024 * 1024) return `${(size / 1024).toFixed(1)} KB`;
    return `${(size / 1024 / 1024).toFixed(1)} MB`;
  }

  function formatTime(value) {
    return new Date(value).toLocaleString('zh-CN', { hour12: false });
  }

  function previewText(value, max = 40) {
    const compact = String(value || '').replace(/\s+/g, ' ').trim();
    if (compact.length <= max) return compact;
    return `${compact.slice(0, max)}...`;
  }

  function getDownloadUrl(item) {
    const path = `/api/items/${item.id}/download`;
    if (state.role === 'admin' && state.shareInfo?.url) {
      return state.shareInfo.url.replace(/\/client\.html$/, path);
    }
    return new URL(path, window.location.origin).toString();
  }

  async function request(url, options = {}) {
    const response = await fetch(`${state.apiBase}${url}`, options);
    if (!response.ok) {
      const data = await response.json().catch(() => ({}));
      throw new Error(data.error || `请求失败：${response.status}`);
    }
    return response.json();
  }

  function render() {
    const root = $('items');
    if (!state.items.length) {
      root.innerHTML = '<div class="empty">暂无共享内容</div>';
      return;
    }

    root.innerHTML = state.items.map((item) => {
      const isText = item.kind === 'text';
      const badge = isText ? '文本' : '文件';
      const title = isText
        ? escapeHtml(previewText(item.content || item.title, 40))
        : escapeHtml(item.title);
      const titleAction = isText
        ? ''
        : `<button class="inline-icon-button" data-action="copy-link" data-id="${item.id}" aria-label="复制文件链接" title="复制文件链接">🔗</button>`;
      const description = isText ? '' : `<div class="meta">${formatSize(item.size)} · ${formatTime(item.createdAt)}</div>`;
      const primaryAction = isText
        ? `<button class="secondary" data-action="copy" data-id="${item.id}">复制</button>`
        : `<button class="secondary" data-action="download" data-id="${item.id}">下载</button>`;
      const actions = state.role === 'admin'
        ? `${primaryAction}<button class="danger" data-action="delete" data-id="${item.id}">删除</button>`
        : primaryAction;
      return `
        <article class="item">
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
  }

  function escapeHtml(value) {
    return String(value).replace(/[&<>"']/g, (char) => ({
      '&': '&amp;',
      '<': '&lt;',
      '>': '&gt;',
      '"': '&quot;',
      "'": '&#39;'
    })[char]);
  }

  async function copyText(value) {
    if (navigator.clipboard?.writeText) {
      try {
        await navigator.clipboard.writeText(value);
        return true;
      } catch (_) {
      }
    }

    const input = document.createElement('textarea');
    input.value = value;
    input.setAttribute('readonly', 'readonly');
    input.style.position = 'fixed';
    input.style.opacity = '0';
    input.style.pointerEvents = 'none';
    document.body.appendChild(input);
    input.focus();
    input.select();
    input.setSelectionRange(0, input.value.length);

    let copied = false;
    try {
      copied = document.execCommand('copy');
    } finally {
      document.body.removeChild(input);
    }
    return copied;
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

  async function applyServerInfo(info) {
    const wasRunning = state.serverRunning;
    state.apiBase = `http://127.0.0.1:${info.port}`;
    state.serverRunning = true;
    document.body.classList.remove('server-stopped');
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
    if (qr) {
      qr.innerHTML = info.qr;
    }
    if (text) {
      text.textContent = info.url;
      text.title = info.url;
    }
    if (copy) {
      copy.onclick = async () => {
        const copied = await copyText(info.url);
        copy.textContent = copied ? '已复制' : '复制失败';
        setTimeout(() => { copy.textContent = '复制链接'; }, 1200);
      };
    }
  }

  function resetServerUi() {
    state.serverRunning = false;
    state.apiBase = '';
    state.items = [];
    if (state.events) {
      state.events.close();
      state.events = null;
    }
    const root = $('items');
    if (root) root.innerHTML = '';
    document.body.classList.add('server-stopped');
    $('clientQr') && ($('clientQr').innerHTML = '');
    $('clientUrlText') && ($('clientUrlText').textContent = '服务未启动');
    $('status') && ($('status').textContent = '未启动');
    state.shareInfo = null;
    setServerControlsStopped();
  }

  async function syncServerStatus() {
    if (!state.isTauri || state.role !== 'admin' || !window.__TAURI__?.core?.invoke) return;

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
      button.textContent = '停止分享';
    }
  }

  function setServerControlsStopped() {
    const portInput = $('serverPort');
    const button = $('startServerButton');
    if (portInput) {
      portInput.disabled = false;
    }
    if (button) {
      button.disabled = false;
      button.textContent = '启动服务';
    }
  }

  function bindServerControls() {
    const form = $('serverForm');
    const portInput = $('serverPort');
    const button = $('startServerButton');
    if (!form || !portInput || !button) return;

    resetServerUi();
    portInput.addEventListener('input', () => {
      const port = Number(portInput.value);
      if (Number.isInteger(port) && port > 0 && port <= 65535) {
        window.__TAURI__.core.invoke('set_preferred_port', { port }).catch(() => {});
      }
    });

    form.addEventListener('submit', async (event) => {
      event.preventDefault();
      if (state.serverRunning) {
        button.disabled = true;
        button.textContent = '停止中';
        try {
          await window.__TAURI__.core.invoke('stop_server');
          resetServerUi();
        } catch (error) {
          button.disabled = false;
          button.textContent = '停止分享';
          alert(String(error));
        }
        return;
      }

      const port = Number(portInput.value);
      if (!Number.isInteger(port) || port < 1 || port > 65535) {
        alert('请输入有效端口号');
        return;
      }

      button.disabled = true;
      button.textContent = '启动中';
      try {
        const info = await window.__TAURI__.core.invoke('start_server', { port });
        await applyServerInfo(info);
      } catch (error) {
        button.disabled = false;
        button.textContent = '启动服务';
        alert(String(error));
      }
    });
  }

  function connectEvents() {
    const status = $('status');
    if (state.events) state.events.close();
    const events = new EventSource(`${state.apiBase}/api/events`);
    state.events = events;
    events.onopen = () => { status.textContent = '实时同步'; };
    events.onerror = () => { status.textContent = '重连中'; };
    events.onmessage = (event) => {
      state.items = JSON.parse(event.data);
      render();
    };
  }

  async function shareAdminFiles() {
    if (!window.__TAURI__?.core?.invoke) {
      throw new Error('当前环境不支持系统文件选择器');
    }

    const fileHint = $('fileHint');
    const fileForm = $('fileForm');
    const button = $('pickAdminFilesButton');

    button.disabled = true;
    button.textContent = '选择中';
    try {
      const paths = await window.__TAURI__.core.invoke('pick_admin_files');
      if (!paths.length) return;
      if (fileHint) {
        fileHint.textContent = `已选择 ${paths.length} 个文件`;
      }
      await request('/api/local-file', {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ paths })
      });
      if (fileHint) {
        fileHint.textContent = '选择后会直接共享本机文件路径，不会复制到应用目录';
      }
      fileForm?.closest('dialog')?.close();
    } finally {
      button.disabled = false;
      button.textContent = '选择并共享文件';
    }
  }

  function bindQrPreview() {
    const button = $('clientQrButton');
    const dialog = $('qrDialog');
    const preview = $('qrDialogPreview');
    const urlText = $('qrDialogUrlText');
    const copyButton = $('qrDialogCopyButton');
    if (!button || !dialog || !preview || !urlText || !copyButton) return;

    button.addEventListener('click', () => {
      if (!state.shareInfo?.qr) return;
      preview.innerHTML = state.shareInfo.qr;
      urlText.textContent = state.shareInfo.url;
      urlText.title = state.shareInfo.url;
      dialog.showModal();
    });

    copyButton.addEventListener('click', async () => {
      if (!state.shareInfo?.url) return;
      const copied = await copyText(state.shareInfo.url);
      copyButton.textContent = copied ? '已复制' : '复制失败';
      setTimeout(() => { copyButton.textContent = '复制链接'; }, 1200);
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
          alert(String(error));
        }
      });
      await events.listen('share-stopped', () => {
        resetServerUi();
      });
      await events.listen('share-error', (event) => {
        alert(String(event.payload || '操作失败'));
      });
    } catch (error) {
      console.warn('Tauri event bridge unavailable:', error);
    }
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
      await request('/api/text', {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ content, source: state.role })
      });
      event.target.reset();
      event.target.closest('dialog')?.close();
    });

    const fileForm = $('fileForm');
    const fileInput = $('fileInput');
    const dropZone = $('dropZone');
    const fileHint = $('fileHint');
    const pickAdminFilesButton = $('pickAdminFilesButton');
    if (fileForm && fileInput) {
      const updateFileHint = () => {
        const count = fileInput.files.length;
        fileHint.textContent = count ? `已选择 ${count} 个文件` : '支持多文件上传';
      };

      const uploadSelectedFiles = async () => {
        if (!fileInput.files.length) return;
        updateFileHint();
        const data = new FormData();
        data.append('source', state.role);
        for (const file of fileInput.files) {
          data.append('file', file);
        }
        await request('/api/upload', { method: 'POST', body: data });
        fileForm.reset();
        updateFileHint();
        fileForm.closest('dialog')?.close();
      };

      fileInput.addEventListener('change', () => {
        uploadSelectedFiles().catch((error) => alert(error.message));
      });

      if (dropZone) {
        ['dragenter', 'dragover'].forEach((name) => {
          dropZone.addEventListener(name, (event) => {
            event.preventDefault();
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
          if (event.dataTransfer?.files?.length) {
            fileInput.files = event.dataTransfer.files;
            uploadSelectedFiles().catch((error) => alert(error.message));
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
        shareAdminFiles().catch((error) => alert(String(error)));
      };

      pickAdminFilesButton.addEventListener('click', openAdminPicker);
      if (dropZone) {
        dropZone.addEventListener('click', openAdminPicker);
        dropZone.tabIndex = 0;
        dropZone.setAttribute('role', 'button');
        dropZone.addEventListener('keydown', (event) => {
          if (event.key === 'Enter' || event.key === ' ') {
            openAdminPicker(event);
          }
        });
      }
    }

    $('items').addEventListener('click', async (event) => {
      const button = event.target.closest('button[data-action]');
      if (!button) return;
      const { action, id } = button.dataset;
      if (action === 'download') {
        if (state.role === 'admin' && state.isTauri) {
          await window.__TAURI__.core.invoke('download_admin_file', { id });
          return;
        }
        window.location.href = `${state.apiBase}/api/items/${id}/download`;
      }
      if (action === 'copy') {
        const item = state.items.find((entry) => entry.id === id);
        if (!item) return;
        const copied = await copyText(item.content || '');
        button.textContent = copied ? '已复制' : '复制失败';
        setTimeout(() => { button.textContent = '复制'; }, 1200);
      }
      if (action === 'copy-link') {
        const item = state.items.find((entry) => entry.id === id);
        if (!item) return;
        const copied = await copyText(getDownloadUrl(item));
        button.classList.toggle('copied', copied);
        button.classList.toggle('copy-failed', !copied);
        setTimeout(() => {
          button.classList.remove('copied', 'copy-failed');
        }, 1200);
      }
      if (action === 'delete' && state.role === 'admin') {
        await request(`/api/items/${id}`, { method: 'DELETE' });
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
      if (state.role === 'admin' && state.isTauri) {
        bindServerControls();
        bindTauriEvents();
        startServerStatusSync();
        return;
      }
      await loadClientShare();
      await loadItems();
      connectEvents();
    }
  };
})();
