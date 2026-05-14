(function () {
  function $(id) {
    return document.getElementById(id);
  }

  function t(key, values = {}) {
    return window.FileShareI18n?.t?.(key, values) || key;
  }

  function formatSize(size) {
    if (!Number.isFinite(size)) return '';
    if (size < 1024) return `${size} B`;
    if (size < 1024 * 1024) return `${(size / 1024).toFixed(1)} KB`;
    return `${(size / 1024 / 1024).toFixed(1)} MB`;
  }

  function formatSpeed(size) {
    if (!Number.isFinite(size) || size <= 0) return '0 B/s';
    return `${formatSize(size)}/s`;
  }

  function formatTime(value) {
    return new Date(value).toLocaleString(window.FileShareI18n?.language || 'zh-CN', {
      hour12: false
    });
  }

  function formatTextLength(value) {
    return t('chars', { count: Array.from(String(value || '').trim()).length });
  }

  function previewText(value, max = 40) {
    const compact = String(value || '').replace(/\s+/g, ' ').trim();
    if (compact.length <= max) return compact;
    return `${compact.slice(0, max)}...`;
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

  window.FileShareUtils = {
    $,
    t,
    formatSize,
    formatSpeed,
    formatTime,
    formatTextLength,
    previewText,
    escapeHtml,
    copyText
  };
})();
