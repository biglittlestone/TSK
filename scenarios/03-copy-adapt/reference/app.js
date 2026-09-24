// 参考实现：极简 Todo 列表（用于被复制到目标项目并适配）
const store = {
  load: () => JSON.parse(localStorage.getItem('app') || '[]'),
  save: (d) => localStorage.setItem('app', JSON.stringify(d)),
};
function render(items) {
  const ul = document.getElementById('list');
  ul.innerHTML = items.map((t, i) => `<li><input type="checkbox" ${t.done ? 'checked' : ''}><span>${t.text}</span><button data-i="${i}">×</button></li>`).join('');
}
function add(text) { const d = store.load(); d.push({ text, done: false }); store.save(d); render(d); }
function toggle(i) { const d = store.load(); d[i].done = !d[i].done; store.save(d); render(d); }
function remove(i) { const d = store.load(); d.splice(i, 1); store.save(d); render(d); }
document.getElementById('addbtn').onclick = () => add(document.getElementById('input').value);
document.addEventListener('click', (e) => {
  if (e.target.tagName === 'BUTTON') remove(+e.target.dataset.i);
  if (e.target.type === 'checkbox') toggle(+e.target.dataset.i); // 简化：checkbox 未直接用，见注释
});
render(store.load());
