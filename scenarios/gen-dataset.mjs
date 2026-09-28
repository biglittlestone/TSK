#!/usr/bin/env node
// Generate a realistic book catalog for scenario 02 — a genuinely large reference
// file (~1,100 entries) so TSK's Read-skeleton + search have real material to work on.
// The model needs only the FORMAT + a sampling; it must NOT read all ~250KB.
import fs from 'node:fs';

const CATS = ['科幻', '文学', '历史', '计算机', '经济', '哲学', '科普', '小说', '心理学', '艺术', '传记', '数学'];
const AUTHORS = ['刘慈欣', '莫言', '余华', '王小波', '村上春树', '加西亚·马尔克斯', '乔治·奥威尔', 'J.R.R.托尔金',
  '尤瓦尔·赫拉利', '塔勒布', '东野圭吾', '阿西莫夫', '理查德·费曼', '史蒂芬·霍金', '亚当·斯密', '冯友兰',
  '刘震云', '严歌苓', '麦家', '金宇澄', '沈从文', '汪曾祺', '卡尔·马克思', '罗曼·罗兰', '毛姆', '杜拉斯'];
const ADJ = ['暗夜', '时间', '寂静', '边界', '潮汐', '旧城', '浮生', '荒野', '回声', '微光', '深渊', '迁徙',
  '远山', '长夜', '初雪', '枯木', '逆旅', '白昼', '原野', '群岛'];
const NOUN = ['之书', '纪事', '漫记', '手记', '备忘录', '编年史', '札记', '随想录', '春秋', '博物志', '地图册', '小史', '影像集'];

let idc = 0;
const books = [];
const years = []; // for later ISBN realistic ranges
for (let i = 0; i < 1100; i++) {
  const author = AUTHORS[i % AUTHORS.length] + (i >= AUTHORS.length ? `${String.fromCharCode(97 + (i % 26))}${i % 5}` : '');
  const title = `${ADJ[i % ADJ.length]}${NOUN[(i * 7) % NOUN.length]} · 第${(i % 9) + 1}卷`;
  const cat = CATS[(i * 13 + 5) % CATS.length];
  const year = 1950 + ((i * 17) % 75);
  const status = i % 4 === 0 ? 'borrowed' : 'in';
  const addedAt = `202${(i % 6)}-${String((i % 12) + 1).padStart(2, '0')}-${String((i % 27) + 1).padStart(2, '0')}`;
  const isbn = `978-${i % 2 === 0 ? '7' : '8'}${String((i * 101) % 1000000000).padStart(9, '0')}-${i % 10}`;
  const b = { id: ++idc, title, author, isbn, category: cat, publishedYear: year, status };
  if (status === 'borrowed') { b.borrower = `读者${i % 60}`; b.dueDate = `202-${String((i % 12) + 1).padStart(2, '0')}-${String((i % 27) + 1).padStart(2, '0')}`; }
  b.addedAt = addedAt;
  books.push(b);
}
const out = JSON.stringify({ generated: '参考图书目录', total: books.length, categories: CATS, entries: books }, null, 2);
const target = 'E:/00work/TSK/scenarios/02-library-crud/reference/books.json';
fs.writeFileSync(target, out);
console.log(`wrote ${books.length} books → ${target} (${(out.length / 1024).toFixed(0)} KB)`);