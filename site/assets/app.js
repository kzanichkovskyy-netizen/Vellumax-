// ClassNode frontend — hash-router SPA. Generic CRUD sections + bespoke views.
'use strict';
const $app = document.getElementById('app');
const esc = (s) => String(s ?? '').replace(/[&<>"']/g, c => ({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[c]));
const fmtDate = (d) => d ? new Date(d).toLocaleDateString('uk-UA') : '—';
const fmtDT = (d) => d ? new Date(d).toLocaleString('uk-UA', {dateStyle:'short', timeStyle:'short'}) : '—';
let ME = null;

// ---------- section config: generic CRUD pages ----------
// fields: k= key, l= label, t= input type (text|number|date|select|json), opts for select, req
const SECTIONS = {
  users: { title:'Користувачі', ep:'/users', roles:['superadmin','admin','teacher'],
    cols:[['email','Email'],['first_name','Імʼя'],['last_name','Прізвище'],['role','Роль'],['is_active','Активний', v=>v?'<span class="badge ok">так</span>':'<span class="badge err">ні</span>']],
    fields:[{k:'email',l:'Email',t:'text',req:1},{k:'password',l:'Пароль (мін. 8)',t:'text',req:1},{k:'role',l:'Роль',t:'select',opts:['admin','teacher','student','parent'],req:1},{k:'first_name',l:'Імʼя',req:1},{k:'last_name',l:'Прізвище',req:1},{k:'phone',l:'Телефон'},{k:'birth_date',l:'Дата народження',t:'date'}],
    canCreate:['superadmin','admin'],
    actions:u=>[['Деактивувати',`/users/${u.id}`,'del',['superadmin','admin']]] },
  classes: { title:'Класи та гуртки', ep:'/classes', roles:['superadmin','admin','teacher','student','parent'],
    cols:[['name','Назва'],['grade_level','Рівень'],['year','Рік'],['type','Тип',v=>v==='club'?'гурток':'клас']],
    fields:[{k:'name',l:'Назва',req:1},{k:'grade_level',l:'Рівень',t:'number',req:1},{k:'year',l:'Навч. рік',t:'number',req:1},{k:'type',l:'Тип',t:'select',opts:['class','club']}],
    canCreate:['superadmin','admin'],
    actions:c=>[['Учні',`#/class-students/${c.id}`,'link','*'],['Підгрупи',`#/class-subgroups/${c.id}`,'link','*'],['Видалити',`/classes/${c.id}`,'del',['superadmin','admin']]] },
  subjects: { title:'Предмети', ep:'/subjects', roles:['superadmin','admin','teacher'],
    cols:[['name','Назва']],
    fields:[{k:'name',l:'Назва',req:1}], canCreate:['superadmin','admin'],
    actions:s=>[['Видалити',`/subjects/${s.id}`,'del',['superadmin','admin']]] },
  homework: { title:'Домашні завдання', ep:'/homework',
    cols:[['title','Назва'],['due_date','До',fmtDate],['created_at','Створено',fmtDate]],
    fields:[{k:'lesson_id',l:'ID уроку',req:1},{k:'title',l:'Назва',req:1},{k:'description',l:'Опис'},{k:'due_date',l:'Термін',t:'date',req:1}],
    canCreate:['teacher','admin','superadmin'],
    actions:a=>[['Видалити',`/homework/${a.id}`,'del',['teacher','admin','superadmin']]] },
  announcements: { title:'Оголошення', ep:'/announcements',
    cols:[['title','Заголовок'],['audience','Аудиторія'],['published_at','Дата',fmtDT]],
    fields:[{k:'title',l:'Заголовок',req:1},{k:'body',l:'Текст',t:'textarea',req:1},{k:'audience',l:'Аудиторія',t:'select',opts:['school','class','role']},{k:'class_id',l:'ID класу (якщо audience=class)'},{k:'role',l:'Роль (якщо audience=role)',t:'select',opts:['','admin','teacher','student','parent']}],
    canCreate:['admin','teacher','superadmin'],
    actions:a=>[['Видалити',`/announcements/${a.id}`,'del',['admin','teacher','superadmin']]] },
  charges: { title:'Нарахування', ep:'/finance/charges', roles:['superadmin','admin','student','parent'],
    cols:[['title','Призначення'],['student_id','Учень'],['amount','Сума'],['status','Статус',v=>`<span class="badge ${v==='paid'?'ok':v==='open'?'warn':''}">${v}</span>`],['due_date','До',fmtDate]],
    fields:[{k:'student_id',l:'ID учня',req:1},{k:'title',l:'Призначення',req:1},{k:'amount',l:'Сума',t:'number',req:1},{k:'due_date',l:'Термін',t:'date'}],
    canCreate:['superadmin','admin'],
    actions:c=>c.status==='open'?[['Скасувати',`/finance/charges/${c.id}/cancel`,'patch',['superadmin','admin']]]:[] },
  payments: { title:'Платежі', ep:'/finance/payments', roles:['superadmin','admin','student','parent'],
    cols:[['student_id','Учень'],['amount','Сума'],['method','Метод'],['paid_at','Дата',fmtDT]],
    fields:[{k:'student_id',l:'ID учня',req:1},{k:'amount',l:'Сума',t:'number',req:1},{k:'method',l:'Метод',t:'select',opts:['cash','card','online']},{k:'charge_id',l:'ID нарахування'},{k:'note',l:'Нотатка'}],
    canCreate:['superadmin','admin'] },
  debts: { title:'Заборгованості', ep:'/finance/debts', roles:['superadmin','admin'],
    cols:[['last_name','Прізвище'],['first_name','Імʼя'],['open_charges','Відкритих'],['debt_total','Борг']] },
  menus: { title:'Меню', ep:'/meals/menus',
    cols:[['date','Дата',fmtDate],['meal_type','Прийом'],['title','Страва'],['price','Ціна']],
    fields:[{k:'date',l:'Дата',t:'date',req:1},{k:'meal_type',l:'Прийом',t:'select',opts:['breakfast','lunch','snack','dinner'],req:1},{k:'title',l:'Страва',req:1},{k:'description',l:'Опис'},{k:'price',l:'Ціна',t:'number'}],
    canCreate:['superadmin','admin'] },
  orders: { title:'Замовлення їжі', ep:'/meals/orders', roles:['superadmin','admin','student','parent'],
    cols:[['student_id','Учень'],['menu_id','Меню'],['status','Статус'],['created_at','Дата',fmtDT]],
    fields:[{k:'menu_id',l:'ID меню',req:1},{k:'student_id',l:'ID учня',req:1}],
    canCreate:['admin','student','parent','superadmin'],
    actions:o=>o.status==='ordered'?[['Скасувати',`/meals/orders/${o.id}/cancel`,'post',['admin','student','parent','superadmin']]]:[] },
  events: { title:'Календар', ep:'/events',
    cols:[['title','Подія'],['start_at','Початок',fmtDT],['end_at','Кінець',fmtDT],['audience','Аудиторія']],
    fields:[{k:'title',l:'Назва',req:1},{k:'description',l:'Опис'},{k:'start_at',l:'Початок',t:'datetime-local',req:1},{k:'end_at',l:'Кінець',t:'datetime-local'},{k:'audience',l:'Аудиторія',t:'select',opts:['school','class','staff']},{k:'class_id',l:'ID класу'},{k:'type',l:'Тип',t:'select',opts:['event','holiday','exam','meeting']}],
    canCreate:['admin','teacher','superadmin'],
    actions:e=>[['Видалити',`/events/${e.id}`,'del',['admin','teacher','superadmin']]] },
  polls: { title:'Опитування', ep:'/polls',
    cols:[['question','Питання'],['audience','Аудиторія'],['ends_at','До',fmtDT]],
    fields:[{k:'question',l:'Питання',req:1},{k:'options',l:'Варіанти (через ;)',t:'textarea',req:1,map:v=>v.split(';').map(s=>s.trim()).filter(Boolean)},{k:'audience',l:'Аудиторія',t:'select',opts:['school','parents','teachers','class']},{k:'class_id',l:'ID класу'}],
    canCreate:['admin','teacher','superadmin'],
    actions:p=>[['Результати',`/polls/${p.id}/results`,'json','*'],['Голосувати',`#/poll-vote/${p.id}`,'link','*']] },
  quizzes: { title:'Тести', ep:'/quizzes', roles:['superadmin','admin','teacher','student','parent'],
    cols:[['title','Назва'],['published','Опубл.',v=>v?'✅':'—']],
    fields:[{k:'title',l:'Назва',req:1},{k:'class_id',l:'ID класу'},{k:'subject_id',l:'ID предмету'},{k:'description',l:'Опис'}],
    canCreate:['admin','teacher','superadmin'],
    actions:q=>[['Питання/відповіді',`/quizzes/${q.id}`,'json','*'],['Опублікувати',`/quizzes/${q.id}/publish`,'post',['admin','teacher','superadmin']],['Спроби',`/quizzes/${q.id}/attempts`,'json',['admin','teacher','superadmin']]] },
  rewardsGive: { title:'Заохочення (видача)', ep:'/rewards', method:'post', roles:['teacher','admin','superadmin'],
    cols:[], fields:[{k:'student_id',l:'ID учня',req:1},{k:'points',l:'Бали (±)',t:'number',req:1},{k:'reason',l:'Причина',req:1}] },
  tasks: { title:'Завдання', ep:'/tasks', roles:['superadmin','admin','teacher'],
    cols:[['title','Назва'],['assignee_id','Виконавець'],['due_date','До',fmtDate],['status','Статус',v=>`<span class="badge ${v==='done'?'ok':'warn'}">${v}</span>`]],
    fields:[{k:'title',l:'Назва',req:1},{k:'description',l:'Опис'},{k:'assignee_id',l:'ID виконавця'},{k:'due_date',l:'Термін',t:'date'}],
    canCreate:['admin','teacher','superadmin'],
    actions:t=>t.status==='open'?[['Виконано',`/tasks/${t.id}`,'patch',['admin','teacher','superadmin','student'],{status:'done'}]]:[] },
  pickups: { title:'Виклики дітей', ep:'/pickups',
    cols:[['student_id','Учень'],['status','Статус',v=>`<span class="badge ${v==='done'?'ok':'warn'}">${v}</span>`],['created_at','Час',fmtDT]],
    fields:[{k:'student_id',l:'ID учня',req:1},{k:'note',l:'Нотатка'}],
    canCreate:['parent','admin','superadmin'],
    actions:p=>p.status==='pending'?[['Готово',`/pickups/${p.id}/done`,'post',['admin','teacher','student','superadmin']]]:[] },
  documents: { title:'Документи', ep:'/documents', roles:['superadmin','admin','teacher','student','parent'],
    cols:[['title','Назва'],['doc_type','Тип'],['filename','Файл'],['created_at','Дата',fmtDate]],
    upload:{fields:[{k:'title',l:'Назва',req:1},{k:'doc_type',l:'Тип',t:'select',opts:['contract','medical','report','other']},{k:'user_id',l:'ID користувача (необовʼязково)'}]},
    canCreate:['admin','teacher','superadmin'],
    actions:d=>[['Завантажити',`/documents/${d.id}/download`,'download','*'],['Видалити',`/documents/${d.id}`,'del',['admin','teacher','superadmin']]] },
  materials: { title:'Матеріали', ep:'/materials', roles:['superadmin','admin','teacher','student','parent'],
    cols:[['filename','Файл'],['size','Розмір',v=>(v/1024).toFixed(0)+' KB'],['created_at','Дата',fmtDate]],
    upload:{fields:[{k:'class_id',l:'ID класу'},{k:'subject_id',l:'ID предмету'}]},
    canCreate:['admin','teacher','superadmin'],
    actions:m=>[['Завантажити',`/materials/${m.id}/download`,'download','*'],['Видалити',`/materials/${m.id}`,'del',['admin','teacher','superadmin']]] },
  locations: { title:'Локації', ep:'/locations', roles:['superadmin','admin'],
    cols:[['name','Назва'],['address','Адреса']],
    fields:[{k:'name',l:'Назва',req:1},{k:'address',l:'Адреса'}],
    canCreate:['superadmin','admin'],
    actions:l=>[['Видалити',`/locations/${l.id}`,'del',['superadmin','admin']]] },
  transport: { title:'Довезення', ep:'/transport', roles:['superadmin','admin'],
    cols:[['name','Маршрут'],['driver_name','Водій'],['driver_phone','Телефон']],
    fields:[{k:'name',l:'Назва',req:1},{k:'driver_name',l:'Водій'},{k:'driver_phone',l:'Телефон'},{k:'stops',l:'Зупинки (через ;)',map:v=>v.split(';').map(s=>s.trim()).filter(Boolean)}],
    canCreate:['superadmin','admin'],
    actions:r=>[['Видалити',`/transport/${r.id}`,'del',['superadmin','admin']]] },
  leads: { title:'Клієнти (CRM)', ep:'/leads', roles:['superadmin','admin'],
    cols:[['parent_name','Батько'],['parent_phone','Телефон'],['child_name','Дитина'],['status','Статус',v=>`<span class="badge">${v}</span>`]],
    fields:[{k:'parent_name',l:'Імʼя батька',req:1},{k:'parent_phone',l:'Телефон'},{k:'parent_email',l:'Email'},{k:'child_name',l:'Імʼя дитини'},{k:'notes',l:'Нотатки'},{k:'status',l:'Статус',t:'select',opts:['new','contacted','trial','enrolled','rejected']}],
    canCreate:['superadmin','admin'],
    actions:l=>[['Видалити',`/leads/${l.id}`,'del',['superadmin','admin']]] },
};

const NAV = [
  ['Основне', [['dashboard','Дашборд'],['users','Користувачі'],['classes','Класи/гуртки'],['subjects','Предмети'],['schedule','Розклад'],['journal','Журнал']]],
  ['Навчання', [['homework','Домашні завдання'],['materials','Матеріали'],['quizzes','Тести'],['rewardsGive','Заохочення']]],
  ['Комунікація', [['announcements','Оголошення'],['events','Календар'],['polls','Опитування'],['tasks','Завдання'],['pickups','Виклики'],['notifications','Сповіщення'],['messages','Повідомлення']]],
  ['Управління', [['charges','Нарахування'],['payments','Платежі'],['debts','Борги'],['menus','Меню'],['orders','Замовлення їжі'],['documents','Документи'],['locations','Локації'],['transport','Довезення'],['leads','CRM'],['analytics','Аналітика'],['settings','Налаштування']]],
];

// ---------- views ----------

function authView(mode) {
  $app.innerHTML = `<div class="auth"><div>
    <div class="logo">🏫 ClassNode</div>
    <div class="card"><h1>${mode==='login'?'Вхід':'Реєстрація школи'}</h1>
    <div class="sub">${mode==='login'?'Увійдіть у свій акаунт':'Створіть заклад і акаунт адміністратора'}</div>
    <div id="amsg"></div>
    <form id="af">
      ${mode==='register'?'<label>Назва школи</label><input name="school_name" required><label>Ваше імʼя</label><input name="first_name" required><label>Прізвище</label><input name="last_name" required>':''}
      <label>Email</label><input name="email" type="email" required>
      <label>Пароль</label><input name="password" type="password" minlength="8" required>
      <button style="width:100%;margin-top:18px">${mode==='login'?'Увійти':'Зареєструвати'}</button>
    </form>
    <div class="sw">${mode==='login'?'Немає акаунта? <a href="#/register">Реєстрація школи</a>':'Маєте акаунт? <a href="#/login">Увійти</a>'}</div>
    </div></div></div>`;
  document.getElementById('af').onsubmit = async (e) => {
    e.preventDefault();
    const f = new FormData(e.target), o = Object.fromEntries(f);
    try {
      if (mode==='login') await API.login(o.email, o.password);
      else await API.register(o);
      location.hash = '#/dashboard';
    } catch (err) {
      document.getElementById('amsg').innerHTML = `<div class="msg err">${esc(err.message)}</div>`;
    }
  };
}

function navLink(id) {
  const map = {dashboard:'Дашборд',journal:'Журнал',schedule:'Розклад',notifications:'Сповіщення',messages:'Повідомлення',analytics:'Аналітика',settings:'Налаштування'};
  return map[id] || (SECTIONS[id] && SECTIONS[id].title) || id;
}

function allowed(id) {
  const s = SECTIONS[id];
  if (!s || !s.roles) return true;
  return ME && s.roles.includes(ME.role);
}

function shell(content, active) {
  const groups = NAV.map(([g, items]) =>
    `<div class="grp">${g}</div>` + items.filter(([id])=>allowed(id)).map(([id]) =>
      `<a href="#/${id}" class="${active===id?'on':''}">${navLink(id)}</a>`).join('')).join('');
  $app.innerHTML = `<div class="shell">
    <nav class="side"><div class="brand">🏫 ClassNode</div>${groups}
      <div class="grp"></div><a href="#" id="logout">Вийти</a></nav>
    <div class="main"><div class="top"><h1>${navLink(active)}</h1>
      <div class="who">${esc(ME.first_name)} ${esc(ME.last_name)} · <span class="badge">${ME.role}</span></div></div>
      <div id="view">${content}</div></div></div>`;
  document.getElementById('logout').onclick = (e) => { e.preventDefault(); API.logout(); };
}

function modal(title, inner) {
  const o = document.createElement('div');
  o.className = 'overlay';
  o.innerHTML = `<div class="modal"><h2>${esc(title)}</h2>${inner}
    <div class="row"><button class="ghost" data-x>Скасувати</button></div></div>`;
  o.onclick = (e) => { if (e.target === o || e.target.dataset.x !== undefined) o.remove(); };
  document.body.appendChild(o);
  return o;
}

async function showJson(url) {
  try { modal('Результат', `<pre style="white-space:pre-wrap;font-size:12px;max-height:60vh;overflow:auto">${esc(JSON.stringify(await API.get(url), null, 2))}</pre>`); }
  catch (e) { modal('Помилка', esc(e.message)); }
}

function createForm(sec, ep, onDone) {
  const fields = (sec.fields||[]).map(f => {
    const req = f.req ? 'required' : '';
    if (f.t === 'select') return `<label>${f.l}</label><select name="${f.k}" ${req}>${f.opts.map(o=>`<option value="${o}">${o||'—'}</option>`).join('')}</select>`;
    if (f.t === 'textarea') return `<label>${f.l}</label><textarea name="${f.k}" rows="3" ${req}></textarea>`;
    return `<label>${f.l}</label><input name="${f.k}" type="${f.t||'text'}" ${req}>`;
  }).join('');
  const o = modal('Додати', `<div id="cmsg"></div><form id="cf">${fields}<button style="margin-top:16px">Зберегти</button></form>`);
  o.querySelector('#cf').onsubmit = async (e) => {
    e.preventDefault();
    const raw = Object.fromEntries(new FormData(e.target));
    const body = {};
    for (const f of sec.fields||[]) {
      let v = raw[f.k];
      if (v === '' || v === undefined) continue;
      if (f.map) v = f.map(v);
      else if (f.t === 'number') v = Number(v);
      else if (f.t === 'datetime-local') v = new Date(v).toISOString();
      body[f.k] = v;
    }
    try { await API.post(ep, body); o.remove(); onDone(); }
    catch (err) { o.querySelector('#cmsg').innerHTML = `<div class="msg err">${esc(err.message)}</div>`; }
  };
}

function uploadForm(sec, onDone) {
  const fields = (sec.upload.fields||[]).map(f =>
    f.t === 'select'
      ? `<label>${f.l}</label><select name="${f.k}">${f.opts.map(o=>`<option>${o}</option>`).join('')}</select>`
      : `<label>${f.l}</label><input name="${f.k}" type="${f.t||'text'}" ${f.req?'required':''}>`).join('');
  const o = modal('Завантажити файл', `<div id="cmsg"></div><form id="cf">${fields}
    <label>Файл</label><input name="file" type="file" required>
    <button style="margin-top:16px">Завантажити</button></form>`);
  o.querySelector('#cf').onsubmit = async (e) => {
    e.preventDefault();
    const fd = new FormData(e.target);
    for (const [k,v] of [...fd]) if (v === '' || (k==='file' && !v.size)) fd.delete(k);
    try { await API.form(sec.ep, fd); o.remove(); onDone(); }
    catch (err) { o.querySelector('#cmsg').innerHTML = `<div class="msg err">${esc(err.message)}</div>`; }
  };
}

async function downloadFile(url) {
  const res = await fetch(API.base + url, { headers: { authorization: `Bearer ${API.access}` } });
  if (!res.ok) return alert('Помилка завантаження');
  const blob = await res.blob();
  const cd = res.headers.get('content-disposition') || '';
  const name = (cd.match(/filename="([^"]+)"/) || [,'file'])[1];
  const a = document.createElement('a');
  a.href = URL.createObjectURL(blob); a.download = name; a.click();
  URL.revokeObjectURL(a.href);
}

async function sectionView(id) {
  const sec = SECTIONS[id];
  shell('<div id="body">Завантаження…</div>', id);
  const v = document.getElementById('view');
  let rows = [];
  try { rows = await API.get(sec.ep); } catch (e) { v.innerHTML = `<div class="msg err">${esc(e.message)}</div>`; return; }

  const canCreate = sec.canCreate && ME && sec.canCreate.includes(ME.role);
  const head = `<div style="margin-bottom:14px;display:flex;gap:8px">
      ${canCreate ? `<button id="add">+ Додати</button>` : ''}
      <button class="ghost" id="reload">Оновити</button></div>`;
  const cols = sec.cols || [];
  const table = `<div class="card" style="padding:0;overflow:auto"><table><thead><tr>
      ${cols.map(c=>`<th>${c[1]}</th>`).join('')}${sec.actions?'<th></th>':''}</tr></thead>
    <tbody>${(rows||[]).map(r => `<tr>
      ${cols.map(([k,,f])=>`<td>${f?f(r[k],r):esc(r[k]??'—')}</td>`).join('')}
      ${sec.actions?`<td style="white-space:nowrap">${sec.actions(r).filter(a=>a[3]==='*'||(a[3]||[]).includes(ME.role)).map((a,i)=>actBtn(r,a,i)).join('')}</td>`:''}
    </tr>`).join('') || `<tr><td colspan="${cols.length+1}" style="color:var(--mut);padding:20px">Порожньо</td></tr>`}</tbody></table></div>`;
  v.innerHTML = head + table;

  const reload = () => sectionView(id);
  document.getElementById('reload').onclick = reload;
  if (canCreate) document.getElementById('add').onclick = () =>
    sec.upload ? uploadForm(sec, reload) : createForm(sec, sec.ep, reload);

  v.querySelectorAll('[data-act]').forEach(b => b.onclick = async () => {
    const [type, url, extra] = [b.dataset.act, b.dataset.url, b.dataset.extra];
    if (type === 'link') { location.hash = url; return; }
    if (type === 'json') return showJson(url);
    if (type === 'download') return downloadFile(url);
    if (type === 'del' && !confirm('Видалити?')) return;
    try {
      if (type === 'del') await API.del(url);
      else if (type === 'post') await API.post(url, extra ? JSON.parse(extra) : {});
      else if (type === 'patch') await API.patch(url, extra ? JSON.parse(extra) : {});
      reload();
    } catch (e) { alert(e.message); }
  });
}

function actBtn(row, [label, url, type, , extra], i) {
  if (type === 'link') return `<button class="sm ghost" data-act="link" data-url="${url}">${label}</button> `;
  return `<button class="sm ${type==='del'?'danger':'ghost'}" data-act="${type}" data-url="${url}" ${extra?`data-extra='${JSON.stringify(extra)}'`:''}>${label}</button> `;
}

async function dashboard() {
  shell('<div id="body">Завантаження…</div>', 'dashboard');
  const v = document.getElementById('view');
  const [unread, notifs] = await Promise.all([
    API.get('/notifications/unread-count').catch(()=>({unread:0})),
    API.get('/notifications').catch(()=>[]),
  ]);
  let stats = '';
  if (['admin','superadmin'].includes(ME.role)) {
    const [users, classes, debts, summary] = await Promise.all([
      API.get('/users').catch(()=>[]), API.get('/classes').catch(()=>[]),
      API.get('/finance/debts').catch(()=>[]), API.get('/finance/summary').catch(()=>({})),
    ]);
    stats = `<div class="grid c3">
      <div class="stat"><div class="n">${users.length}</div><div class="l">Користувачів</div></div>
      <div class="stat"><div class="n">${classes.length}</div><div class="l">Класів/гуртків</div></div>
      <div class="stat"><div class="n">${unread.unread}</div><div class="l">Непрочитаних сповіщень</div></div>
      <div class="stat"><div class="n">${debts.length}</div><div class="l">Боржників</div></div>
      <div class="stat"><div class="n">${Math.round(summary.total_paid||0)} ₴</div><div class="l">Оплачено всього</div></div>
      <div class="stat"><div class="n">${Math.round(summary.total_open||0)} ₴</div><div class="l">Відкритих нарахувань</div></div></div>`;
  }
  const feed = (notifs||[]).slice(0,10).map(n =>
    `<tr><td><span class="badge">${esc(n.type)}</span></td><td>${esc(JSON.stringify(n.payload))}</td><td>${fmtDT(n.created_at)}</td><td>${n.read_at?'':'<b>●</b>'}</td></tr>`).join('');
  v.innerHTML = stats + `<h2 style="margin:20px 0 10px;font-size:18px">Останні сповіщення</h2>
    <div class="card" style="padding:0"><table><tbody>${feed||'<tr><td style="padding:20px;color:var(--mut)">Немає сповіщень</td></tr>'}</tbody></table></div>`;
}

async function journalView() {
  shell('<div id="body">Завантаження…</div>', 'journal');
  const v = document.getElementById('view');
  const classes = await API.get('/classes').catch(()=>[]);
  const subjects = await API.get('/subjects').catch(()=>[]);
  if (!classes.length || !subjects.length) { v.innerHTML = '<div class="msg err">Потрібні класи й предмети</div>'; return; }
  const sel = `<div style="display:flex;gap:10px;margin-bottom:16px;flex-wrap:wrap">
    <select id="jc" style="width:auto">${classes.map(c=>`<option value="${c.id}">${esc(c.name)}</option>`).join('')}</select>
    <select id="js" style="width:auto">${subjects.map(s=>`<option value="${s.id}">${esc(s.name)}</option>`).join('')}</select>
    <button id="jgo">Показати</button>
    <button class="ghost" id="jcsv">Експорт CSV</button></div><div id="jt"></div>`;
  v.innerHTML = sel;
  const load = async () => {
    const cid = document.getElementById('jc').value, sid = document.getElementById('js').value;
    document.getElementById('jcsv').onclick = () => downloadFile(`/analytics/classes/${cid}/journal.csv`);
    const j = await API.get(`/journal/classes/${cid}/journal?subject_id=${sid}`).catch(e => ({err:e.message}));
    if (j.err) { document.getElementById('jt').innerHTML = `<div class="msg err">${esc(j.err)}</div>`; return; }
    const {students, lessons, grades, attendance} = j;
    const gmap = {}, amap = {};
    grades.forEach(g => gmap[g.student_id+':'+g.lesson_id] = g.value);
    attendance.forEach(a => amap[a.student_id+':'+a.lesson_id] = a.status);
    document.getElementById('jt').innerHTML = `<div class="card" style="padding:0;overflow:auto"><table>
      <thead><tr><th>Учень</th>${lessons.map(l=>`<th>${fmtDate(l.date)}<br>№${l.lesson_number}</th>`).join('')}</tr></thead>
      <tbody>${students.map(s => `<tr><td style="white-space:nowrap">${esc(s.last_name)} ${esc(s.first_name)}</td>
        ${lessons.map(l => {
          const g = gmap[s.id+':'+l.id], a = amap[s.id+':'+l.id];
          const att = {present:'✓',absent:'н',late:'з','excused':'п'}[a]||'';
          return `<td style="text-align:center">${g??''}<span style="color:var(--mut)">${att}</span></td>`;
        }).join('')}</tr>`).join('') || '<tr><td style="padding:20px">Немає даних</td></tr>'}</tbody></table></div>`;
  };
  document.getElementById('jgo').onclick = load;
  load();
}

async function scheduleView() {
  shell('<div id="body">Завантаження…</div>', 'schedule');
  const v = document.getElementById('view');
  const classes = await API.get('/classes').catch(()=>[]);
  const tabs = `<div class="tabs"><button class="on" data-t="lessons">Уроки</button><button data-t="timetable">Шаблон тижня</button><button data-t="gen">Генерація тижня</button></div><div id="sb"></div>`;
  v.innerHTML = tabs;
  const sb = document.getElementById('sb');
  const render = async (t) => {
    document.querySelectorAll('.tabs button').forEach(b => b.classList.toggle('on', b.dataset.t===t));
    if (t === 'lessons') {
      const cid = classes[0]?.id;
      const ls = await API.get(`/schedule/lessons?from=${new Date().toISOString().slice(0,10)}`).catch(()=>[]);
      sb.innerHTML = `<div class="card" style="padding:0"><table><thead><tr><th>Дата</th><th>№</th><th>Клас</th><th>Аудиторія</th><th>Тема</th></tr></thead>
        <tbody>${ls.map(l=>`<tr><td>${fmtDate(l.date)}</td><td>${l.lesson_number}</td><td>${esc(classes.find(c=>c.id===l.class_id)?.name||'')}</td><td>${esc(l.room||'—')}</td><td>${esc(l.topic||'')}</td></tr>`).join('')||'<tr><td colspan="5" style="padding:20px">Немає уроків</td></tr>'}</tbody></table></div>`;
    } else if (t === 'timetable') {
      const cid = classes[0]?.id;
      const slots = cid ? await API.get(`/schedule/timetable?class_id=${cid}`).catch(()=>[]) : [];
      const days = ['','Пн','Вт','Ср','Чт','Пт','Сб','Нд'];
      const byDay = {};
      slots.forEach(s => (byDay[s.weekday] ||= []).push(s));
      sb.innerHTML = `<div class="cal">${[1,2,3,4,5,6,7].map(d => `<div class="d"><b>${days[d]}</b>
        ${(byDay[d]||[]).sort((a,b)=>a.lesson_number-b.lesson_number).map(s=>`<div class="ev">№${s.lesson_number} · ${esc(s.room||'')}</div>`).join('')}</div>`).join('')}</div>`;
    } else {
      sb.innerHTML = `<div class="card"><form id="gf"><label>Клас</label>
        <select name="class_id">${classes.map(c=>`<option value="${c.id}">${esc(c.name)}</option>`).join('')}</select>
        <label>Понеділок тижня</label><input name="week_start" type="date" required>
        <button style="margin-top:14px">Згенерувати уроки</button></form><div id="gmsg"></div></div>`;
      document.getElementById('gf').onsubmit = async (e) => {
        e.preventDefault();
        const o = Object.fromEntries(new FormData(e.target));
        try {
          const ls = await API.post('/schedule/lessons/generate', o);
          document.getElementById('gmsg').innerHTML = `<div class="msg ok">Створено уроків: ${ls.length}</div>`;
        } catch (err) { document.getElementById('gmsg').innerHTML = `<div class="msg err">${esc(err.message)}</div>`; }
      };
    }
  };
  document.querySelectorAll('.tabs button').forEach(b => b.onclick = () => render(b.dataset.t));
  render('lessons');
}

async function notificationsView() {
  shell('<div id="body">Завантаження…</div>', 'notifications');
  const rows = await API.get('/notifications').catch(()=>[]);
  document.getElementById('view').innerHTML = `<button id="ra" class="ghost" style="margin-bottom:12px">Позначити всі прочитаними</button>
    <div class="card" style="padding:0"><table><tbody>${rows.map(n => `<tr>
      <td><span class="badge">${esc(n.type)}</span></td><td>${esc(JSON.stringify(n.payload))}</td>
      <td>${fmtDT(n.created_at)}</td><td>${n.read_at?'прочитано':`<button class="sm" data-id="${n.id}">прочитати</button>`}</td></tr>`).join('')||'<tr><td style="padding:20px">Порожньо</td></tr>'}</tbody></table></div>`;
  document.getElementById('ra').onclick = async () => { await API.patch('/notifications/read-all', {}); notificationsView(); };
  document.querySelectorAll('button[data-id]').forEach(b => b.onclick = async () => { await API.patch(`/notifications/${b.dataset.id}/read`, {}); notificationsView(); });
}

async function messagesView() {
  shell('<div id="body">Завантаження…</div>', 'messages');
  const rows = await API.get('/messages/conversations').catch(()=>[]);
  document.getElementById('view').innerHTML = `<div class="card" style="padding:0"><table>
    <thead><tr><th>Співрозмовник</th><th>Останнє</th><th>Час</th><th>Непроч.</th></tr></thead>
    <tbody>${rows.map(c => `<tr><td>${esc(c.last_name)} ${esc(c.first_name)}</td><td>${esc(c.last_message)}</td><td>${fmtDT(c.last_at)}</td><td>${c.unread?`<span class="badge warn">${c.unread}</span>`:''}</td></tr>`).join('')||'<tr><td colspan="4" style="padding:20px">Немає розмов</td></tr>'}</tbody></table></div>
    <div class="card" style="margin-top:16px"><h2 style="font-size:16px;margin-bottom:8px">Нове повідомлення</h2><div id="cmsg"></div>
    <form id="mf"><label>ID отримувача</label><input name="recipient_id" required><label>Текст</label><textarea name="body" rows="3" required></textarea><button style="margin-top:12px">Надіслати</button></form></div>`;
  document.getElementById('mf').onsubmit = async (e) => {
    e.preventDefault();
    try { await API.post('/messages', Object.fromEntries(new FormData(e.target))); messagesView(); }
    catch (err) { document.getElementById('cmsg').innerHTML = `<div class="msg err">${esc(err.message)}</div>`; }
  };
}

async function analyticsView() {
  shell('<div id="body">Завантаження…</div>', 'analytics');
  const [att, load, grades] = await Promise.all([
    API.get('/analytics/attendance').catch(()=>[]),
    API.get('/analytics/teacher-load').catch(()=>[]),
    API.get('/analytics/grades').catch(()=>[]),
  ]);
  document.getElementById('view').innerHTML = `<div class="grid c3">
    <div class="card"><h2 style="font-size:16px;margin-bottom:10px">Відвідуваність</h2><table><tbody>
      ${att.slice(0,15).map(r=>`<tr><td>${esc(r.last_name)} ${esc(r.first_name)}</td><td><b>${Math.round(r.rate*100)}%</b></td></tr>`).join('')||'<tr><td>—</td></tr>'}</tbody></table></div>
    <div class="card"><h2 style="font-size:16px;margin-bottom:10px">Навантаження вчителів (уроків/тиж)</h2><table><tbody>
      ${load.map(r=>`<tr><td>${esc(r.last_name)} ${esc(r.first_name)}</td><td><b>${r.weekly_lessons}</b></td></tr>`).join('')||'<tr><td>—</td></tr>'}</tbody></table></div>
    <div class="card"><h2 style="font-size:16px;margin-bottom:10px">Середні бали по предметах</h2><table><tbody>
      ${grades.map(r=>`<tr><td>${esc(r.subject)}</td><td><b>${r.average}</b> (${r.grades_count})</td></tr>`).join('')||'<tr><td>—</td></tr>'}</tbody></table></div></div>`;
}

async function settingsView() {
  shell('<div id="body">Завантаження…</div>', 'settings');
  const v = document.getElementById('view');
  const schools = await API.get('/schools').catch(()=>[]);
  const s = schools[0];
  if (!s) { v.innerHTML = '<div class="msg err">Школа не знайдена</div>'; return; }
  const g = s.settings?.grading || {};
  v.innerHTML = `<div class="card"><h2 style="font-size:18px;margin-bottom:8px">${esc(s.name)}</h2>
    <div id="cmsg"></div><form id="sf">
    <label>Шкала оцінок — мін</label><input name="min" type="number" value="${g.min??1}">
    <label>Макс</label><input name="max" type="number" value="${g.max??12}">
    <label>Назва школи</label><input name="name" value="${esc(s.name)}">
    <button style="margin-top:16px">Зберегти</button></form></div>`;
  document.getElementById('sf').onsubmit = async (e) => {
    e.preventDefault();
    const o = Object.fromEntries(new FormData(e.target));
    try {
      await API.patch(`/schools/${s.id}`, { name: o.name, settings: { grading: { type:'numeric', min:+o.min, max:+o.max } } });
      document.getElementById('cmsg').innerHTML = '<div class="msg ok">Збережено</div>';
    } catch (err) { document.getElementById('cmsg').innerHTML = `<div class="msg err">${esc(err.message)}</div>`; }
  };
}

async function classStudentsView(cid) {
  shell('<div id="body">Завантаження…</div>', 'classes');
  const v = document.getElementById('view');
  const [students, subs] = await Promise.all([
    API.get(`/classes/${cid}/students`).catch(()=>[]),
    API.get(`/classes/${cid}/subjects`).catch(()=>[]),
  ]);
  const teachers = (await API.get('/users?role=teacher').catch(()=>[]));
  const allStudents = (await API.get('/users?role=student').catch(()=>[]));
  const enrolled = new Set(students.map(s=>s.id));
  const free = allStudents.filter(s=>!enrolled.has(s.id));
  v.innerHTML = `<h2 style="margin-bottom:10px;font-size:18px">Учні класу</h2>
    <div class="card" style="padding:0;margin-bottom:20px"><table><tbody>
      ${students.map(s=>`<tr><td>${esc(s.last_name)} ${esc(s.first_name)}</td><td>${esc(s.email)}</td>
        <td><button class="sm danger" data-un="${s.id}">Вибути</button></td></tr>`).join('')||'<tr><td style="padding:20px">Порожньо</td></tr>'}</tbody></table></div>
    ${ME.role!=='student'?`<div class="card"><form id="ef" style="display:flex;gap:10px;align-items:end">
      <div style="flex:1"><label>Додати учня</label><select name="student_id">${free.map(s=>`<option value="${s.id}">${esc(s.last_name)} ${esc(s.first_name)}</option>`).join('')}</select></div>
      <button>Додати</button></form></div>`:''}
    <h2 style="margin:20px 0 10px;font-size:18px">Предмети та вчителі</h2>
    <div class="card" style="padding:0"><table><tbody>
      ${subs.map(s=>`<tr><td>${esc(s.subject_name)}</td><td>${esc(s.teacher_last_name)} ${esc(s.teacher_first_name)}</td></tr>`).join('')||'<tr><td style="padding:20px">Не призначено</td></tr>'}</tbody></table></div>
    ${['admin','superadmin'].includes(ME.role)?`<div class="card" style="margin-top:16px"><h3>Призначити вчителя на предмет</h3><form id="af" style="display:flex;gap:10px;align-items:end;flex-wrap:wrap">
      <div><label>Предмет</label><select name="subject_id">${(await API.get('/subjects').catch(()=>[])).map(s=>`<option value="${s.id}">${esc(s.name)}</option>`).join('')}</select></div>
      <div><label>Вчитель</label><select name="teacher_id">${teachers.map(t=>`<option value="${t.id}">${esc(t.last_name)} ${esc(t.first_name)}</option>`).join('')}</select></div>
      <button>Призначити</button></form></div>`:''}`;
  document.querySelectorAll('[data-un]').forEach(b => b.onclick = async () => {
    if (confirm('Вилучити учня з класу?')) { await API.del(`/classes/${cid}/students/${b.dataset.un}`); classStudentsView(cid); }
  });
  const ef = document.getElementById('ef');
  if (ef) ef.onsubmit = async (e) => { e.preventDefault();
    await API.post(`/classes/${cid}/students`, Object.fromEntries(new FormData(e.target))); classStudentsView(cid); };
  const af = document.getElementById('af');
  if (af) af.onsubmit = async (e) => { e.preventDefault();
    await API.post(`/classes/${cid}/subjects`, Object.fromEntries(new FormData(e.target))); classStudentsView(cid); };
}

async function classSubgroupsView(cid) {
  shell('<div id="body">Завантаження…</div>', 'classes');
  const v = document.getElementById('view');
  const [sgs, students] = await Promise.all([
    API.get(`/classes/${cid}/subgroups`).catch(()=>[]),
    API.get(`/classes/${cid}/students`).catch(()=>[]),
  ]);
  v.innerHTML = `<h2 style="margin-bottom:10px;font-size:18px">Підгрупи</h2>
    <div class="card" style="padding:0;margin-bottom:16px"><table><tbody>
      ${sgs.map(g=>`<tr><td>${esc(g.name)}</td><td><button class="sm" data-g="${g.id}">Учасники</button> <button class="sm danger" data-dg="${g.id}">Видалити</button></td></tr>`).join('')||'<tr><td style="padding:20px">Немає підгруп</td></tr>'}</tbody></table></div>
    <div class="card"><form id="gf" style="display:flex;gap:10px;align-items:end"><div style="flex:1"><label>Нова підгрупа</label><input name="name" required></div><button>Створити</button></form></div><div id="sgm"></div>`;
  document.getElementById('gf').onsubmit = async (e) => { e.preventDefault();
    await API.post(`/classes/${cid}/subgroups`, Object.fromEntries(new FormData(e.target))); classSubgroupsView(cid); };
  document.querySelectorAll('[data-dg]').forEach(b => b.onclick = async () => {
    if (confirm('Видалити підгрупу?')) { await API.del(`/classes/${cid}/subgroups/${b.dataset.dg}`); classSubgroupsView(cid); } });
  document.querySelectorAll('[data-g]').forEach(b => b.onclick = async () => {
    const gid = b.dataset.g;
    const members = await API.get(`/classes/${cid}/subgroups/${gid}/students`).catch(()=>[]);
    document.getElementById('sgm').innerHTML = `<div class="card" style="margin-top:16px"><h3>Учасники</h3>
      ${members.map(m=>`<div>${esc(m.last_name)} ${esc(m.first_name)} <button class="sm danger" data-rm="${m.id}" data-g="${gid}">×</button></div>`).join('')||'—'}
      <form id="addm" style="display:flex;gap:8px;margin-top:10px"><select name="student_id">${students.map(s=>`<option value="${s.id}">${esc(s.last_name)} ${esc(s.first_name)}</option>`).join('')}</select><button class="sm">Додати</button></form></div>`;
    document.getElementById('addm').onsubmit = async (e) => { e.preventDefault();
      await API.post(`/classes/${cid}/subgroups/${gid}/students`, Object.fromEntries(new FormData(e.target))); classSubgroupsView(cid); };
    document.querySelectorAll('[data-rm]').forEach(x => x.onclick = async () => {
      await API.del(`/classes/${cid}/subgroups/${x.dataset.g}/students/${x.dataset.rm}`); classSubgroupsView(cid); });
  });
}

async function pollVoteView(pid) {
  shell('<div id="body">Завантаження…</div>', 'polls');
  const r = await API.get(`/polls/${pid}/results`).catch(e => ({err:e.message}));
  const v = document.getElementById('view');
  if (r.err) { v.innerHTML = `<div class="msg err">${esc(r.err)}</div>`; return; }
  const opts = r.poll.options;
  v.innerHTML = `<div class="card"><h2 style="font-size:18px;margin-bottom:10px">${esc(r.poll.question)}</h2>
    ${opts.map((o,i)=>`<div style="margin:8px 0"><button class="ghost" data-i="${i}">${esc(o)}</button> — ${r.results[o]||0}</div>`).join('')}
    <div style="color:var(--mut);margin-top:10px">Голосів: ${r.total_votes}${r.my_vote!==null?` · ваш вибір: ${opts[r.my_vote]}`:''}</div></div>`;
  document.querySelectorAll('[data-i]').forEach(b => b.onclick = async () => {
    await API.post(`/polls/${pid}/vote`, { option_index: +b.dataset.i }); pollVoteView(pid); });
}

// ---------- router ----------

const ROUTES = {
  dashboard, journal: journalView, schedule: scheduleView,
  notifications: notificationsView, messages: messagesView,
  analytics: analyticsView, settings: settingsView,
};

async function route() {
  const h = location.hash.slice(2) || 'dashboard';
  if (h === 'login') return authView('login');
  if (h === 'register') return authView('register');
  if (!API.access) { location.hash = '#/login'; return; }
  if (!ME) {
    try { ME = await API.me(); }
    catch { API.logout(); return; }
  }
  if (h.startsWith('class-students/')) return classStudentsView(h.split('/')[1]);
  if (h.startsWith('class-subgroups/')) return classSubgroupsView(h.split('/')[1]);
  if (h.startsWith('poll-vote/')) return pollVoteView(h.split('/')[1]);
  if (ROUTES[h]) return ROUTES[h]();
  if (SECTIONS[h]) {
    if (!allowed(h)) return shell('<div class="msg err">Немає доступу</div>', h);
    return sectionView(h);
  }
  return dashboard();
}

window.addEventListener('hashchange', route);
route();
