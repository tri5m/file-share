(function () {
  const role = document.body.dataset.appRole || 'client';

  window.FileShareApp?.init({ role }).catch((error) => {
    console.error('FileShare failed to start:', error);
  });
})();
