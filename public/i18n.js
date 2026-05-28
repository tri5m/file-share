(function () {
  const messages = {
    'zh-CN': {
      qrPreview: '放大查看二维码',
      usageTitle: '使用说明',
      usage1: '1. 启动服务后，扫描二维码或复制链接',
      usage2: '2. 在局域网内的其他设备打开链接',
      usage3: '3. 发布文本或共享文件即可分享',
      usage4: '4. 客户端可下载或复制共享内容',
      usageNote: '指定客户端访问端口，启动后局域网设备可通过二维码或链接访问客户端页面。',
      chooseAddress: '选择访问地址',
      loadingClientLink: '加载客户端链接',
      loadingAccessLink: '加载访问链接',
      copyLink: '复制链接',
      shareDropTitle: '拖动文件到此处或点击此处分享文件',
      shareDropHint: '文件会以本机路径方式共享，不会复制到应用目录',
      startShare: '启动分享',
      port: '端口号',
      sharedList: '共享列表',
      connecting: '连接中',
      publishText: '📝 发布文本',
      shareFile: '📁 共享文件',
      uploadTextAction: '📝 上传文本',
      uploadFileAction: '📁 上传文件',
      publishTextTitle: '发布文本',
      shareFileTitle: '共享文件',
      uploadTextTitle: '上传文本',
      uploadFileTitle: '上传文件',
      close: '关闭',
      textPlaceholder: '输入要共享的文本',
      publishTextSubmit: '发布文本',
      uploadTextSubmit: '上传文本',
      pickAndShare: '选择并共享文件',
      qrCode: '二维码',
      clientPickFile: '点击选择文件，选完自动上传',
      autoUploadHint: '选择后自动上传',
      qqGroup: 'QQ群：1079340875',
      emailLabel: 'Email：',
      empty: '暂无共享内容',
      textBadge: '文本',
      fileBadge: '文件',
      copy: '复制',
      invalid: '已失效',
      reveal: '查看',
      download: '下载',
      remove: '移除',
      chars: '{count} 字',
      requestFailed: '请求失败：{status}',
      closeTip: '关闭提示',
      copied: '已复制',
      copyFailed: '复制失败',
      linkCopied: '链接已复制',
      serviceNotStarted: '服务未启动',
      notStarted: '未启动',
      stopShare: '停止分享',
      validPort: '请输入有效端口号',
      stopping: '停止中',
      starting: '启动中',
      synced: '实时同步',
      reconnecting: '重连中',
      reconnectToast: '连接中断，正在重试',
      sharing: '共享中',
      selectedFiles: '已选择 {count} 个文件',
      unsupportedPicker: '当前环境不支持系统文件选择器',
      selecting: '选择中',
      dropRelease: '松开即可共享文件',
      localPathHint: '将直接登记本机文件路径，不会复制到应用目录',
      sharingFiles: '正在共享文件...',
      pleaseWait: '请稍候',
      sharedFiles: '已共享 {count} 个文件',
      fileAdded: '文件已加入共享列表',
      operationFailed: '操作失败',
      updateAvailable: '发现新版本，可在托盘检查更新',
      selectedAndPreparing: '已选择 {count} 个文件，正在准备上传',
      uploadingFiles: '正在上传 {count} 个文件，请保持页面打开',
      uploadDone: '上传完成',
      uploadFailed: '上传失败，请重新选择',
      pastingFiles: '正在分享粘贴的 {count} 个文件',
      pasteFileDone: '已分享 {count} 个粘贴文件',
      pasteTextDone: '已分享粘贴文本',
      pasteFileTooLarge: '无本机路径的粘贴文件最大支持 {size}，请改用拖拽或选择文件',
      unsupportedShareType: '暂不支持分享目录或该类型内容',
      openLocationFailed: '无法打开文件位置',
      openFailed: '打开失败：{message}',
      sourceMissing: '源文件已不存在'
    },
    'en-US': {
      qrPreview: 'Show QR',
      usageTitle: 'Guide',
      usage1: '1. Start sharing',
      usage2: '2. Scan the QR code or open the link',
      usage3: '3. Share text or files',
      usage4: '4. Download or copy on other devices',
      usageNote: 'Set a port, then use the QR code or link on the same LAN.',
      chooseAddress: 'Address',
      loadingClientLink: 'Loading link',
      loadingAccessLink: 'Loading link',
      copyLink: 'Copy link',
      shareDropTitle: 'Drop or click to share files',
      shareDropHint: 'Uses local paths. Files are not copied.',
      startShare: 'Start',
      port: 'Port',
      sharedList: 'Shared',
      connecting: 'Connecting',
      publishText: '📝 Publish text',
      shareFile: '📁 Share files',
      uploadTextAction: '📝 Upload text',
      uploadFileAction: '📁 Upload files',
      publishTextTitle: 'Publish text',
      shareFileTitle: 'Share files',
      uploadTextTitle: 'Upload text',
      uploadFileTitle: 'Upload files',
      close: 'Close',
      textPlaceholder: 'Text to share',
      publishTextSubmit: 'Publish text',
      uploadTextSubmit: 'Upload text',
      pickAndShare: 'Share files',
      qrCode: 'QR code',
      clientPickFile: 'Choose files to upload',
      autoUploadHint: 'Auto upload after selection',
      qqGroup: 'QQ Group: 1079340875',
      emailLabel: 'Email: ',
      empty: 'Nothing shared',
      textBadge: 'Text',
      fileBadge: 'File',
      copy: 'Copy',
      invalid: 'Invalid',
      reveal: 'Show',
      download: 'Download',
      remove: 'Remove',
      chars: '{count} chars',
      requestFailed: 'Request failed: {status}',
      closeTip: 'Close',
      copied: 'Copied',
      copyFailed: 'Copy failed',
      linkCopied: 'Link copied',
      serviceNotStarted: 'Not started',
      notStarted: 'Not started',
      stopShare: 'STOP',
      validPort: 'Invalid port',
      stopping: 'Stopping',
      starting: 'Starting',
      synced: 'Online',
      reconnecting: 'Reconnecting',
      reconnectToast: 'Reconnecting...',
      sharing: 'Sharing',
      selectedFiles: '{count} selected',
      unsupportedPicker: 'File picker unavailable',
      selecting: 'Selecting',
      dropRelease: 'Release to share',
      localPathHint: 'Uses local paths. Files are not copied.',
      sharingFiles: 'Sharing files...',
      pleaseWait: 'Please wait',
      sharedFiles: '{count} shared',
      fileAdded: 'Added',
      operationFailed: 'Failed',
      updateAvailable: 'Update available',
      selectedAndPreparing: '{count} selected. Preparing...',
      uploadingFiles: 'Uploading {count}...',
      uploadDone: 'Uploaded',
      uploadFailed: 'Upload failed',
      pastingFiles: 'Sharing {count} pasted file(s)',
      pasteFileDone: 'Shared {count} pasted file(s)',
      pasteTextDone: 'Pasted text shared',
      pasteFileTooLarge: 'Pasted files without a local path are limited to {size}. Please drag or choose the file instead.',
      unsupportedShareType: 'Folders or this content type are not supported',
      openLocationFailed: 'Cannot open location',
      openFailed: 'Open failed: {message}',
      sourceMissing: 'File missing'
    }
  };

  const browserLanguage =
    navigator.language
    || '';
  const language = browserLanguage.toLowerCase().startsWith('zh') ? 'zh-CN' : 'en-US';

  function t(key, values = {}) {
    const template = messages[language][key] || messages['zh-CN'][key] || key;
    return template.replace(/\{(\w+)\}/g, (_, name) => values[name] ?? '');
  }

  function text(selector, key) {
    document.querySelectorAll(selector).forEach((element) => {
      element.textContent = t(key);
    });
  }

  function attr(selector, name, key) {
    document.querySelectorAll(selector).forEach((element) => {
      element.setAttribute(name, t(key));
    });
  }

  function apply() {
    document.documentElement.lang = language;
    document.querySelectorAll('[data-i18n]').forEach((element) => {
      element.textContent = t(element.dataset.i18n);
    });
    document.querySelectorAll('[data-i18n-placeholder]').forEach((element) => {
      element.setAttribute('placeholder', t(element.dataset.i18nPlaceholder));
    });
    document.querySelectorAll('[data-i18n-aria-label]').forEach((element) => {
      element.setAttribute('aria-label', t(element.dataset.i18nAriaLabel));
    });
    attr('#clientQrButton', 'aria-label', 'qrPreview');
    attr('#clientAddressSelect', 'aria-label', 'chooseAddress');
  }

  window.FileShareI18n = { language, t, apply };
  document.addEventListener('DOMContentLoaded', apply);
})();
