/* NovelReader/Tomato bridge: uses the existing application API and settings. */
'use strict';
(() => {
  const $t = id => document.getElementById(id);
  const form = $t('tomato-form');
  if (!form) return;

  const idField = $t('tomato-book-id');
  const startButton = $t('tomato-start');
  const cancelButton = $t('tomato-cancel');
  const statusLine = $t('tomato-status');
  const stage = $t('tomato-stage');
  const count = $t('tomato-count');
  const progress = $t('tomato-progress');
  const progressCard = form.querySelector('.tomato-card-body');
  const idHelp = $t('tomato-id-help');
  // Read the full clipboard before maxlength can truncate a pasted URL.
  idField.addEventListener('paste', event => {
    const text = event.clipboardData?.getData('text').trim();
    if (!text) return;
    let url;
    try {
      url = new URL(text.includes('://') ? text : 'https://' + text);
    } catch {
      return;
    }
    if (url.hostname !== 'fanqienovel.com' && !url.hostname.endsWith('.fanqienovel.com')) return;
    const pathId = url.pathname.match(/^\/page\/([0-9]{8,25})\/?$/)?.[1];
    const queryId = url.searchParams.get('book_id');
    const bookId = pathId || (/^[0-9]{8,25}$/.test(queryId || '') ? queryId : null);
    if (bookId) {
      event.preventDefault();
      idField.value = bookId;
      idField.dispatchEvent(new Event('input', { bubbles: true }));
    }
  });
  const labels = {
    idle: 'Chưa bắt đầu',
    starting: 'Khởi tạo',
    downloading: 'Tải chương',
    importing: 'Nạp chương',
    done: 'Đã hoàn tất',
    error: 'Lỗi',
    canceled: 'Đã hủy'
  };
  let lastStage = null;
  let polling = false;

  // Tab switching for Source Panels
  document.querySelectorAll('.source-tab').forEach(btn => {
    btn.addEventListener('click', () => {
      document.querySelectorAll('.source-tab').forEach(b => {
        b.classList.remove('active');
        b.setAttribute('aria-selected', 'false');
      });
      document.querySelectorAll('.source-panel').forEach(p => p.classList.remove('active'));
      btn.classList.add('active');
      btn.setAttribute('aria-selected', 'true');
      const target = document.getElementById(btn.dataset.target);
      if (target) target.classList.add('active');
    });
  });

  async function refreshTomato() {
    if (polling) return;
    polling = true;
    try {
      const info = await api('/api/tomato/status');
      progressCard.hidden = info.stage === 'idle';
      idHelp.hidden = !!info.active;
      startButton.disabled = !!info.active;
      cancelButton.disabled = !info.active || !['starting', 'downloading'].includes(info.stage);
      stage.textContent = labels[info.stage] || info.stage;
      statusLine.textContent = info.message || '';
      statusLine.classList.toggle('tomato-error', info.stage === 'error');
      count.textContent = Number(info.saved || 0) + ' / ' + Number(info.total || 0) + ' chương';
      progress.value = info.total ? Math.min(100, Math.round(100 * Number(info.saved || 0) / Number(info.total))) : 0;
      if (info.active) {
        const tomatoTab = document.querySelector('.source-tab[data-target="panel-tomato"]');
        if (tomatoTab && !tomatoTab.classList.contains('active')) {
          tomatoTab.click();
        }
      }
      if (info.stage !== lastStage) {
        if (info.stage === 'done' && lastStage) {
          await refreshAll();
          if (chapters.length) await selectChapter(chapters[0].id);
          notice('Đã lấy và nạp truyện thành công vào danh sách chương.');
        }
        if (info.stage === 'error' && lastStage && lastStage !== 'error') notice(info.message || 'Tải truyện lỗi.', true);
      }
      lastStage = info.stage;
    } catch (error) {
      statusLine.textContent = 'Không cập nhật được tiến độ: ' + error.message;
    } finally {
      polling = false;
    }
  }

  form.addEventListener('submit', event => {
    event.preventDefault();
    safe(async () => {
      const bookId = idField.value.trim();
      if (!/^[0-9]{8,25}$/.test(bookId)) throw new Error('ID truyện phải có từ 8 đến 25 chữ số.');
      const replace = !!state.book;
      if (state.progress?.active) throw new Error('Hãy dừng tác vụ dịch hiện tại trước.');
      if (replace && !confirm('Thay thế truyện đang mở bằng truyện có ID ' + bookId + '? Dữ liệu hiện tại trong cơ sở dữ liệu sẽ được thay thế.')) return;
      const data = await api('/api/tomato/start', { book_id: bookId, replace_existing: replace });
      progressCard.hidden = false;
      idHelp.hidden = true;
      lastStage = null;
      statusLine.textContent = data.message || 'Đã bắt đầu tải.';
      startButton.disabled = true;
      cancelButton.disabled = false;
      await refreshTomato();
    });
  });

  cancelButton.addEventListener('click', () => safe(async () => {
    if (!confirm('Hủy quá trình tải truyện từ Tomato?')) return;
    await api('/api/tomato/cancel', {});
    await refreshTomato();
  }));

  refreshTomato();
  setInterval(refreshTomato, 3000);
})();
