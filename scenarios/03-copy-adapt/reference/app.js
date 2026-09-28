// 参考实现：功能较全的 Todo 列表（用于被复制到目标项目并适配成看板）
// 分区：store(数据) / render(视图) / actions(交互) —— 目标看板应保持同构分区。
(function () {
  'use strict';
  const KEY = 'todo-app-v2';
  // ---------- store：数据层（localStorage，纯函数） ----------
  const store = {
    load() {
      try { return JSON.parse(localStorage.getItem(KEY) || '[]'); }
      catch { return []; }
    },
    save(list) { localStorage.setItem(KEY, JSON.stringify(list)); },
    nextId(list) { return list.reduce((m, t) => Math.max(m, +t.id || 0), 0) + 1; },
  };

  // ---------- state：运行态 ----------
  let state = { filter: 'all' };

  // ---------- render：视图层 ----------
  function itemEl(t) {
    const li = document.createElement('li');
    li.className = t.done ? 'done' : '';
    li.draggable = true;
    li.dataset.id = t.id;
    li.innerHTML =
      `<input type="checkbox" class="chk" ${t.done ? 'checked' : ''} aria-label="完成">` +
      `<span class="txt">${escapeHtml(t.text)}</span>` +
      `<button class="del" aria-label="删除">×</button>`;
    return li;
  }
  function render(list) {
    const el = document.getElementById('list');
    el.innerHTML = '';
    list.filter(byFilter(state.filter)).forEach((t) => el.appendChild(itemEl(t)));
    document.getElementById('count').textContent = list.length + ' 项';
    document.getElementById('left').textContent =
      list.filter((t) => !t.done).length + ' 未完成';
  }
  function byFilter(f) {
    return (t) => f === 'all' ? true : f === 'done' ? t.done : !t.done;
  }
  function escapeHtml(s) {
    return String(s).replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]));
  }

  // ---------- actions：交互层 ----------
  function refresh() { render(store.load()); }
  function add() {
    const v = document.getElementById('input').value.trim();
    if (!v) return;
    const list = store.load();
    list.push({ id: store.nextId(list), text: v, done: false });
    store.save(list);
    document.getElementById('input').value = '';
    refresh();
  }
  function toggle(id) {
    const list = store.load().map((t) => (t.id == id ? { ...t, done: !t.done } : t));
    store.save(list); refresh();
  }
  function remove(id) {
    store.save(store.load().filter((t) => t.id != id)); refresh();
  }
  function toggleAll() {
    const list = store.load();
    const allDone = list.every((t) => t.done);
    store.save(list.map((t) => ({ ...t, done: !allDone }))); refresh();
  }
  function clearDone() {
    store.save(store.load().filter((t) => !t.done)); refresh();
  }
  // 拖拽换序（简化：drop 到另一行前）
  function move(fromId, toBeforeId) {
    const list = store.load();
    const i = list.findIndex((t) => t.id == fromId);
    if (i < 0) return;
    const [it] = list.splice(i, 1);
    const j = toBeforeId == null ? list.length : list.findIndex((t) => t.id == toBeforeId);
    list.splice(j, 0, it);
    store.save(list); refresh();
  }

  // ---------- 事件绑定 ----------
  document.getElementById('addbtn').onclick = add;
  document.getElementById('input').addEventListener('keydown', (e) => { if (e.key === 'Enter') add(); });
  document.getElementById('toggle-all').onclick = toggleAll;
  document.getElementById('clear-done').onclick = clearDone;
  document.getElementById('filter').onchange = (e) => { state.filter = e.target.value; refresh(); };
  document.getElementById('list').addEventListener('click', (e) => {
    const li = e.target.closest('li'); if (!li) return;
    if (e.target.classList.contains('chk')) toggle(li.dataset.id);
    if (e.target.classList.contains('del')) remove(li.dataset.id);
  });
  document.getElementById('list').addEventListener('dragstart', (e) => {
    e.dataTransfer.setData('text/plain', e.target.closest('li').dataset.id);
  });
  document.getElementById('list').addEventListener('dragover', (e) => e.preventDefault());
  document.getElementById('list').addEventListener('drop', (e) => {
    e.preventDefault();
    const fromId = e.dataTransfer.getData('text/plain');
    const target = e.target.closest('li');
    move(fromId, target ? target.dataset.id : null);
  });

  refresh();
})();