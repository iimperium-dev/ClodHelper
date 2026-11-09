// Clod Scan — проверка аккаунта Claude (закладка).
// Читает только claude.ai: /api/account и /api/organizations/{id}/kyc_status.
// Ничего никуда не отправляет. Код открыт: https://github.com/iimperium-dev/ClodHelper
(async () => {
  // ===== настройки =====
  const CUTOFF = new Date('2026-09-15T00:00:00');
  const REPO = 'https://github.com/iimperium-dev/ClodHelper';
  const ID = 'clod-scan-root';
  const DEMO = window.__CLOD_SCAN_DEMO__; // пример на странице закладки

  // ===== утилиты =====
  const sleep = ms => new Promise(r => setTimeout(r, ms));
  const plural = (n, f) => f[(n % 10 == 1 && n % 100 != 11) ? 0 : (n % 10 >= 2 && n % 10 <= 4 && (n % 100 < 10 || n % 100 >= 20)) ? 1 : 2];
  const fmt = d => new Date(d).toLocaleString('ru-RU', { day: '2-digit', month: '2-digit', year: 'numeric', hour: '2-digit', minute: '2-digit' });
  const esc = s => String(s).replace(/&/g, '&amp;').replace(/</g, '&lt;');
  const cv = document.createElement('canvas').getContext('2d');
  const tw = (txt, font) => { cv.font = font; return Math.ceil(cv.measureText(txt).width); };
  const b64 = s => btoa(unescape(encodeURIComponent(s)));
  const el = (tag, css, attrs = {}) => { const e = document.createElement(tag); if (css) e.style.cssText = css; for (const k in attrs) e.setAttribute(k, attrs[k]); return e; };

  // ===== дизайн-токены Clod Helper =====
  const SANS = 'Inter,"Segoe UI Variable","Segoe UI",-apple-system,system-ui,sans-serif';
  const MONO = '"JetBrains Mono","Cascadia Mono",Menlo,Consolas,monospace';
  const C = {
    bg: '#0b0e0d', surf: '#121614', surf2: '#1a1e1c', border: '#252a28',
    fg: '#f1f4f2', silver: '#ccd1ce', dim: '#8b918e', faint: '#646a67',
    em: '#22c084', emB: '#3ad897', warn: '#e0a33d', loss: '#ef6767'
  };
  const COL = { ok: C.em, warn: C.warn, deny: C.loss };

  // ===== иконки (lucide + simple-icons) =====
  const SHIELD = 'M20 13c0 5-3.5 7.5-7.66 8.95a1 1 0 0 1-.67-.01C7.5 20.5 4 18 4 13V6a1 1 0 0 1 1-1c2 0 4.5-1.2 6.24-2.72a1.17 1.17 0 0 1 1.52 0C14.51 3.81 17 5 19 5a1 1 0 0 1 1 1z';
  const ICONS = {
    ok:   [SHIELD, 'm9 12 2 2 4-4'],
    warn: ['m21.73 18-8-14a2 2 0 0 0-3.48 0l-8 14A2 2 0 0 0 4 21h16a2 2 0 0 0 1.73-3', 'M12 9v4', 'M12 17h.01'],
    deny: [SHIELD, 'm14.5 9.5-5 5', 'm9.5 9.5 5 5']
  };
  const GH = 'M12 .297c-6.63 0-12 5.373-12 12 0 5.303 3.438 9.8 8.205 11.385.6.113.82-.258.82-.577 0-.285-.01-1.04-.015-2.04-3.338.724-4.042-1.61-4.042-1.61C4.422 18.07 3.633 17.7 3.633 17.7c-1.087-.744.084-.729.084-.729 1.205.084 1.838 1.236 1.838 1.236 1.07 1.835 2.809 1.305 3.495.998.108-.776.417-1.305.76-1.605-2.665-.3-5.466-1.332-5.466-5.93 0-1.31.465-2.38 1.235-3.22-.135-.303-.54-1.523.105-3.176 0 0 1.005-.322 3.3 1.23.96-.267 1.98-.399 3-.405 1.02.006 2.04.138 3 .405 2.28-1.552 3.285-1.23 3.285-1.23.645 1.653.24 2.873.12 3.176.765.84 1.23 1.91 1.23 3.22 0 4.61-2.805 5.625-5.475 5.92.42.36.81 1.096.81 2.22 0 1.606-.015 2.896-.015 3.286 0 .315.21.69.825.57C20.565 22.092 24 17.592 24 12.297c0-6.627-5.373-12-12-12';

  // ===== окно поверх страницы =====
  document.getElementById(ID)?.remove();
  const host = el('div', 'all:initial;position:fixed;inset:0;z-index:2147483647;', { id: ID });
  const root = host.attachShadow({ mode: 'closed' });
  const back = el('div', 'position:fixed;inset:0;background:rgba(5,7,6,.62);backdrop-filter:blur(6px);-webkit-backdrop-filter:blur(6px);opacity:0;transition:opacity .3s;');
  const wrap = el('div', 'position:fixed;left:50%;top:50%;transform:translate(-50%,calc(-50% + 10px));opacity:0;transition:opacity .35s,transform .45s cubic-bezier(.22,1,.36,1);filter:drop-shadow(0 24px 60px rgba(0,0,0,.55));');
  root.append(back, wrap);
  (document.body || document.documentElement).append(host);

  const onKey = e => { if (e.key === 'Escape') close(); };
  const close = () => {
    removeEventListener('keydown', onKey, true);
    back.style.opacity = '0'; wrap.style.opacity = '0';
    setTimeout(() => host.remove(), 320);
  };
  addEventListener('keydown', onKey, true);
  back.addEventListener('click', close);
  requestAnimationFrame(() => { back.style.opacity = '1'; });

  // показать готовую SVG-карточку; links — кликабельные зоны поверх неё
  const show = (card, links = []) => {
    const k = Math.min(1.25, (innerHeight - 64) / card.h, (innerWidth - 48) / card.w);
    wrap.style.width = card.w * k + 'px';
    wrap.style.height = card.h * k + 'px';
    const img = el('img', `display:block;width:100%;height:100%;max-width:none;margin:0;border:0;user-select:none;`, {
      src: 'data:image/svg+xml;base64,' + b64(card.svg), alt: 'Clod Scan', draggable: 'false'
    });
    const x = el('button', `all:initial;position:absolute;top:-12px;right:-12px;width:30px;height:30px;border-radius:50%;background:${C.surf2};box-shadow:inset 0 0 0 1px ${C.border};cursor:pointer;display:grid;place-items:center;`, { 'aria-label': 'Закрыть', title: 'Закрыть (Esc)' });
    x.append(el('img', 'display:block;width:14px;height:14px;', { src: 'data:image/svg+xml;base64,' + b64(`<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 24 24' fill='none' stroke='${C.silver}' stroke-width='2.4' stroke-linecap='round'><path d='M18 6 6 18M6 6l12 12'/></svg>`), alt: '' }));
    x.addEventListener('click', close);
    wrap.append(img, x);
    for (const l of links) {
      const a = el('a', `position:absolute;left:${l.x * k}px;top:${l.y * k}px;width:${l.w * k}px;height:${l.h * k}px;border-radius:${10 * k}px;cursor:pointer;visibility:hidden;`, { href: l.href, target: '_blank', rel: 'noopener', 'aria-label': l.label });
      setTimeout(() => { a.style.visibility = 'visible'; }, l.at * 1000);
      wrap.append(a);
    }
    requestAnimationFrame(() => { wrap.style.opacity = '1'; wrap.style.transform = 'translate(-50%,-50%)'; });
  };

  // ===== запросы =====
  const getJSON = async (url, ms = 8000) => {
    const ctrl = new AbortController();
    const tm = setTimeout(() => ctrl.abort(), ms);
    try {
      const r = await fetch(url, { credentials: 'include', signal: ctrl.signal });
      if (!r.ok) return { ok: false, status: r.status, error: `HTTP ${r.status}` };
      if (!(r.headers.get('content-type') || '').includes('json')) return { ok: false, status: 401, error: 'нет данных аккаунта' };
      return { ok: true, data: await r.json() };
    } catch (e) {
      return { ok: false, error: e.name === 'AbortError' ? 'сервер не ответил вовремя' : e.message };
    } finally { clearTimeout(tm); }
  };
  const load = async () => {
    const accR = await getJSON('/api/account');
    if (!accR.ok) throw accR;
    const acc = accR.data;
    const activeId = document.cookie.match(/lastActiveOrg=([^;]+)/)?.[1];
    const orgs = (acc.memberships || []).map(m => m.organization?.uuid).filter(Boolean);
    const order = activeId ? [activeId, ...orgs.filter(o => o !== activeId)] : orgs;
    let kyc = null;
    for (const id of order) {
      const r = await getJSON(`/api/organizations/${id}/kyc_status`);
      if (r.ok) { kyc = r.data; break; }
    }
    return { acc, kyc };
  };

  // ===== SVG-хелперы =====
  const STEPS = [
    ['Защищённое соединение', 620], ['Профиль аккаунта', 940], ['Возраст и история', 700],
    ['Организации', 880], ['Статус KYC', 1060], ['Сверка с политикой', 800]
  ];
  const t = (x, y, str, o = {}) => `<text x='${x}' y='${y}' font-family='${o.mono ? MONO : SANS}' font-size='${o.size || 13}' font-weight='${o.wt || 400}' fill='${o.fill || C.fg}'${o.anchor ? ` text-anchor='${o.anchor}'` : ''}${o.op != null ? ` opacity='${o.op}'` : ''}>${esc(str)}${o.kids || ''}</text>`;
  const setAt = (a, to, at) => `<set attributeName='${a}' to='${to}' begin='${at}s' fill='freeze'/>`;
  const fadeIn = (at, d = .3) => `<animate attributeName='opacity' from='0' to='1' begin='${at}s' dur='${d}s' fill='freeze'/>`;
  const ico = (x, y, s, ps, col, sw = 2) => `<svg x='${x}' y='${y}' width='${s}' height='${s}' viewBox='0 0 24 24' fill='none' stroke='${col}' stroke-width='${sw}' stroke-linecap='round' stroke-linejoin='round'>${ps.map(d => `<path d='${d}'/>`).join('')}</svg>`;
  const logo = (x, y, s) => `<svg x='${x}' y='${y}' width='${s}' height='${s}' viewBox='0 0 32 32'><rect width='32' height='32' rx='7.5' fill='#161c19'/><rect x='.5' y='.5' width='31' height='31' rx='7' fill='none' stroke='#fff' stroke-opacity='.06'/><path d='M 23.09 21.54 A 9 9 0 1 1 23.09 10.46' fill='none' stroke='${C.em}' stroke-width='3.1' stroke-linecap='round'/><circle cx='25.2' cy='16' r='2.3' fill='${C.emB}'/></svg>`;
  const head = P => logo(P, 18, 30) + t(P + 40, 31, 'Clod Helper', { size: 13, wt: 600 }) + t(P + 40, 45, 'Account scan', { size: 10, fill: C.faint });
  const glow = `<radialGradient id='glo' cx='50%' cy='0%' r='75%'><stop offset='0' stop-color='${C.em}' stop-opacity='.10'/><stop offset='1' stop-color='${C.em}' stop-opacity='0'/></radialGradient>`;

  // карточка ошибки: заголовок + до двух строк пояснения
  const fail = (title, lines) => {
    const w = 460, P = 22, H = 120 + lines.length * 20;
    const s = `<rect x='.5' y='.5' width='${w - 1}' height='${H - 1}' rx='10' fill='${C.surf}' stroke='${C.border}'/>` +
      `<rect x='1' y='1' width='${w - 2}' height='120' rx='10' fill='url(#glo)'/>` + head(P) +
      `<line x1='${P}' x2='${w - P}' y1='66' y2='66' stroke='${C.border}'/>` +
      ico(P, 84, 18, ICONS.warn, C.warn, 2) + t(P + 28, 98, title, { size: 15, wt: 600 }) +
      lines.map((l, i) => t(P + 28, 122 + i * 20, l, { fill: C.dim })).join('');
    return { svg: `<svg xmlns='http://www.w3.org/2000/svg' width='${w}' height='${H}'><defs>${glow}</defs>${s}</svg>`, w, h: H };
  };

  // ===== одна анимированная карточка =====
  const build = (dur, res) => {
    const w = 460, P = 22, bw = w - 2 * P, ac = COL[res.state], f = n => n.toFixed(3), c = [0];
    dur.forEach(x => c.push(c[c.length - 1] + x / 1000));
    const T = c[c.length - 1], fT = f(T);
    let s = '', y;

    s += head(P);
    const p1 = 'Сканирование', p2 = 'Проверено';
    const pw = Math.max(tw(p1, `500 12px ${SANS}`), tw(p2, `500 12px ${SANS}`)) + 34, px = w - P - pw;
    s += `<rect x='${px}' y='20' width='${pw}' height='26' rx='13' fill='#0a0a0a' fill-opacity='.55' stroke='#fff' stroke-opacity='.05'/>`;
    s += `<circle cx='${px + 13}' cy='33' r='6' fill='${C.em}' opacity='0'><animate attributeName='r' values='3;8' dur='1.6s' repeatCount='indefinite'/><animate attributeName='opacity' values='.45;0' dur='1.6s' repeatCount='indefinite'/>${setAt('visibility', 'hidden', fT)}</circle>`;
    s += `<circle cx='${px + 13}' cy='33' r='3' fill='${C.em}'>${setAt('fill', ac, fT)}</circle>`;
    s += t(px + 23, 37, p1, { size: 12, wt: 500, fill: C.silver, kids: setAt('opacity', '0', fT) }) +
         t(px + 23, 37, p2, { size: 12, wt: 500, fill: C.silver, op: 0, kids: setAt('opacity', '1', fT) });

    // изумрудная шкала с мягким свечением
    const vals = c.map(x => Math.round(bw * x / T)).join(';');
    const kt = c.map((x, i) => i === c.length - 1 ? '1' : f(x / T)).join(';');
    const anim = `<animate attributeName='width' values='${vals}' keyTimes='${kt}' dur='${fT}s' fill='freeze'/>`;
    s += `<rect x='${P}' y='66' width='${bw}' height='3' rx='1.5' fill='${C.surf2}'/>` +
         `<rect x='${P}' y='65' width='0' height='5' rx='2.5' fill='${C.em}' opacity='.55' filter='url(#gl)'>${anim}${setAt('fill', ac, fT)}</rect>` +
         `<rect x='${P}' y='66' width='0' height='3' rx='1.5' fill='${C.em}'>${anim}${setAt('fill', ac, fT)}</rect>`;

    // шаги
    y = 102;
    STEPS.forEach(([name], i) => {
      const a = f(c[i]), b = f(c[i + 1]);
      s += t(P, y, String(i + 1).padStart(2, '0'), { size: 11, mono: 1, fill: C.faint });
      s += t(P + 28, y, name, { fill: '#3a403d', kids: setAt('fill', C.fg, a) + setAt('fill', C.silver, b) });
      s += `<g opacity='0'>${setAt('opacity', '1', a)}${setAt('opacity', '0', b)}` +
           `<circle cx='${w - P - 6}' cy='${y - 4}' r='3' fill='${C.em}'><animate attributeName='opacity' values='1;.3;1' dur='1s' repeatCount='indefinite'/></circle>` +
           t(w - P - 16, y, 'выполняется', { size: 12, fill: C.dim, anchor: 'end' }) + `</g>`;
      s += `<g opacity='0'>${fadeIn(b)}${ico(w - P - 14, y - 12, 14, ['M20 6 9 17l-5-5'], C.em, 2.6)}` +
           t(w - P - 22, y, dur[i] + ' мс', { size: 11, mono: 1, fill: C.faint, anchor: 'end' }) + `</g>`;
      y += 28;
    });
    y -= 28;
    const H0 = y + 24;
    const ln = yy => `<line x1='${P}' x2='${w - P}' y1='${yy}' y2='${yy}' stroke='${C.border}'/>`;

    // результат
    let r = '';
    y += 22; r += ln(y); y += 30;
    res.rows.forEach(([k, v, l]) => {
      const cc = l ? COL[l] : C.fg;
      r += t(P, y, k, { fill: C.dim });
      if (l) r += `<circle cx='${w - P - tw(v, `500 13px ${SANS}`) - 10}' cy='${y - 4.5}' r='3' fill='${cc}'/>`;
      r += t(w - P, y, v, { wt: 500, fill: cc, anchor: 'end' });
      y += 28;
    });

    // вердикт
    const vy = y - 6;
    r += `<rect x='${P}' y='${vy}' width='${bw}' height='64' rx='10' fill='${ac}' fill-opacity='.07' stroke='${ac}' stroke-opacity='.28'/>` +
         `<rect x='${P + 14}' y='${vy + 14}' width='36' height='36' rx='8' fill='${ac}' fill-opacity='.14'/>` +
         ico(P + 22, vy + 22, 20, ICONS[res.state], ac, 2) +
         t(P + 62, vy + 29, res.big, { size: 15, wt: 600, fill: ac }) +
         t(P + 62, vy + 47, res.title, { size: 12, fill: C.dim });

    // карточка GitHub
    const fy = vy + 64 + 12;
    r += `<rect x='${P}' y='${fy}' width='${bw}' height='54' rx='10' fill='${C.surf2}' stroke='${C.border}'/>` +
         `<rect x='${P + 12}' y='${fy + 12}' width='30' height='30' rx='7' fill='${C.bg}' stroke='#fff' stroke-opacity='.06'/>` +
         `<svg x='${P + 19}' y='${fy + 19}' width='16' height='16' viewBox='0 0 24 24'><path d='${GH}' fill='#C9CFCB'/></svg>` +
         t(P + 54, fy + 24, 'Clod Helper на GitHub', { size: 13, wt: 500 }) +
         t(P + 54, fy + 40, 'iimperium-dev/ClodHelper · открытый код', { size: 11, fill: C.faint }) +
         ico(w - P - 28, fy + 19, 16, ['m9 18 6-6-6-6'], C.em, 2.2);
    const H = fy + 54 + P;

    const defs = `<defs>${glow}<filter id='gl' x='-5%' y='-300%' width='110%' height='700%'><feGaussianBlur stdDeviation='3'/></filter></defs>`;
    const bg = `<rect x='.5' y='.5' width='${w - 1}' height='${H0 - 1}' rx='10' fill='${C.surf}' stroke='${C.border}'>` +
               `<animate attributeName='height' from='${H0 - 1}' to='${H - 1}' begin='${f(T + .15)}s' dur='.5s' calcMode='spline' keyTimes='0;1' keySplines='.22 1 .36 1' fill='freeze'/></rect>` +
               `<rect x='1' y='1' width='${w - 2}' height='170' rx='10' fill='url(#glo)'/>` +
               `<line x1='12' x2='${w - 12}' y1='1.5' y2='1.5' stroke='#fff' stroke-opacity='.04'/>`;
    return {
      svg: `<svg xmlns='http://www.w3.org/2000/svg' width='${w}' height='${H}'>${defs}${bg}${s}<g opacity='0'>${fadeIn(f(T + .35), .45)}${r}</g></svg>`,
      w, h: H, T, gh: { x: P, y: fy, w: bw, h: 54 }
    };
  };

  // ===== запуск =====
  if (!DEMO && !/(^|\.)claude\.ai$/.test(location.hostname))
    return show(fail('Откройте claude.ai', ['Закладка работает только на сайте claude.ai.', 'Войдите в аккаунт и нажмите её там.']));

  let j, kycData;
  try { ({ acc: j, kyc: kycData } = DEMO || await load()); }
  catch (e) {
    return show(e.status === 401 || e.status === 403
      ? fail('Вы не вошли в аккаунт', ['Войдите в Claude на этой странице', 'и нажмите закладку ещё раз.'])
      : fail('Не удалось получить данные', [String(e.error || e.message || e).slice(0, 56),'Обновите страницу и попробуйте ещё раз.']));
  }

  const created = new Date(j.created_at), now = new Date();
  let y = now.getFullYear() - created.getFullYear(), m = now.getMonth() - created.getMonth(), d = now.getDate() - created.getDate();
  if (d < 0) { m--; d += new Date(now.getFullYear(), now.getMonth(), 0).getDate(); }
  if (m < 0) { y--; m += 12; }
  const age = [y && `${y} ${plural(y, ['год', 'года', 'лет'])}`, m && `${m} мес.`, `${d} ${plural(d, ['день', 'дня', 'дней'])}`].filter(Boolean).join(' ');
  // статус KYC от Claude → русская подпись и цвет; «Разрешено» — только если KYC пройден
  const KYC = {
    verified: ['пройден', 'ok'], not_required: ['не пройден', 'warn'],
    pending: ['на проверке', 'warn'], in_progress: ['на проверке', 'warn'], in_review: ['на проверке', 'warn'], submitted: ['на проверке', 'warn'],
    required: ['требуется', 'warn'], not_started: ['требуется', 'warn'],
    failed: ['не пройден', 'deny'], rejected: ['отклонён', 'deny'], declined: ['отклонён', 'deny'], expired: ['истёк', 'deny']
  };
  const ks = String(kycData?.status || '').toLowerCase();
  const kyc = KYC[ks] || [ks ? ks.replace(/_/g, ' ') : 'нет данных', 'warn'];
  const kycOk = kyc[1] === 'ok';
  const earlyReg = created < CUTOFF;
  const state = !earlyReg ? 'deny' : kycOk ? 'ok' : 'warn';
  const yn = (v, yes, no) => v === true ? [yes, 'ok'] : v === false ? [no, 'deny'] : ['нет данных', 'warn'];

  const dur = STEPS.map(([, ms]) => Math.round(ms * (0.85 + Math.random() * 0.3)));
  const card = build(dur, {
    state,
    big:   { ok: 'Разрешено', warn: 'Внимание', deny: 'Запрещено' }[state],
    title: { ok: 'Активации по Org ID доступны', warn: 'Активация возможна, может понадобиться KYC', deny: 'Активации по Org ID недоступны' }[state],
    rows: [
      ['Возраст', age],
      ['Создан', fmt(j.created_at)],
      ['Верификация', ...yn(j.is_verified, 'пройдена', 'не пройдена')],
      ['Возраст 18+', ...yn(j.age_is_verified, 'подтверждён', 'не подтверждён')],
      ['KYC', ...kyc],
      ['Регистрация до ' + CUTOFF.toLocaleDateString('ru-RU'), ...(earlyReg ? ['да', 'ok'] : ['нет', 'deny'])]
    ]
  });
  await sleep(120);
  show(card, [{ ...card.gh, href: REPO, label: 'Clod Helper на GitHub', at: card.T + .5 }]);
})();
