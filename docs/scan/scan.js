// Clod Scan — проверка аккаунта Claude (закладка).
// Читает только claude.ai: /api/account и /api/organizations/{id}/kyc_status.
// Ничего никуда не отправляет. Код открыт: https://github.com/iimperium-dev/ClodHelper
(async () => {
  // ===== настройки =====
  const CUTOFF = new Date('2026-09-15T00:00:00');
  const REPO = 'https://github.com/iimperium-dev/ClodHelper';
  const ID = 'clod-scan-root';
  // пример на странице закладки: читаем один раз и сразу стираем,
  // чтобы следующее нажатие закладки не показало пример вместо ошибки
  const DEMO = window.__CLOD_SCAN_DEMO__;
  try { delete window.__CLOD_SCAN_DEMO__; } catch (e) { window.__CLOD_SCAN_DEMO__ = undefined; }

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

  // стили кнопок поверх карточки (живут в закрытом shadow DOM — сайт их не видит и не ломает)
  const css = el('style');
  css.textContent = `
    .z{position:absolute;visibility:hidden;cursor:pointer;border-radius:8px;transition:background .15s}
    .z.on{visibility:visible}
    .cp:hover,.cp:focus-visible{background:rgba(255,255,255,.045);outline:none}
    .tag{position:absolute;top:50%;display:flex;align-items:center;gap:5px;transform:translateY(-50%);padding:2px 8px 2px 6px;border-radius:6px;background:${C.surf2};box-shadow:inset 0 0 0 1px #303633;color:${C.dim};font:500 11px ${SANS};white-space:nowrap;opacity:0;transition:opacity .15s,color .15s;pointer-events:none}
    .cp:hover .tag,.cp:focus-visible .tag,.tag.ok{opacity:1}
    .tag.ok{color:${C.em}}
    .tag img{width:12px;height:12px;display:block}
    .snap{all:unset;box-sizing:border-box;position:absolute;display:flex;align-items:center;justify-content:center;transform:translateX(-50%);border-radius:999px;background:transparent;box-shadow:inset 0 0 0 1px rgba(255,255,255,.08);color:${C.dim};font-family:${SANS};font-weight:500;white-space:nowrap;cursor:pointer;visibility:hidden;opacity:0;transition:opacity .4s,background .15s,box-shadow .15s,color .15s}
    .snap.on{visibility:visible;opacity:1}
    .snap:hover,.snap:focus-visible{background:rgba(34,192,132,.08);box-shadow:inset 0 0 0 1px rgba(34,192,132,.35);color:${C.fg}}
    .snap:active{transform:translateX(-50%) scale(.96)}
    .snap img{display:block;opacity:.85}
    .flash{position:absolute;inset:0;border-radius:inherit;background:#fff;opacity:0;pointer-events:none}
    .flash.go{animation:fl .5s ease-out}
    @keyframes fl{0%{opacity:.9}100%{opacity:0}}
    .shot{animation:sh .45s cubic-bezier(.22,1,.36,1)}
    @keyframes sh{0%{scale:1}30%{scale:.975}100%{scale:1}}
    .toast{position:absolute;left:50%;bottom:18px;transform:translate(-50%,6px);display:flex;align-items:center;gap:8px;padding:9px 14px;border-radius:10px;background:${C.surf2};box-shadow:inset 0 0 0 1px #303633,0 12px 30px rgba(0,0,0,.4);color:${C.fg};font:500 13px ${SANS};white-space:nowrap;opacity:0;transition:opacity .2s,transform .3s;pointer-events:none}
    .toast.on{opacity:1;transform:translate(-50%,0)}
    .toast img{width:15px;height:15px;display:block}`;
  root.append(css);
  const svgUrl = s => 'data:image/svg+xml;base64,' + b64(s);
  const icon = (paths, col, sw = 2) => svgUrl(`<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 24 24' fill='none' stroke='${col}' stroke-width='${sw}' stroke-linecap='round' stroke-linejoin='round'>${paths.map(d => `<path d='${d}'/>`).join('')}</svg>`);
  const I_COPY = ['M8 8h12a2 2 0 0 1 2 2v10a2 2 0 0 1-2 2H10a2 2 0 0 1-2-2z', 'M4 16a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h10a2 2 0 0 1 2 2'];
  const I_OK = ['M20 6 9 17l-5-5'];
  const I_CAM = ['M14.5 4h-5L7 7H4a2 2 0 0 0-2 2v9a2 2 0 0 0 2 2h16a2 2 0 0 0 2-2V9a2 2 0 0 0-2-2h-3z', 'M12 16.5a3.5 3.5 0 1 0 0-7 3.5 3.5 0 0 0 0 7z'];
  const isMac = /Mac|iPhone|iPad/.test(navigator.platform || navigator.userAgent);

  // звук затвора: синтезируется на месте (без файлов) — два коротких щелчка шума
  let ac;
  const shutter = () => {
    try {
      ac = ac || new (window.AudioContext || window.webkitAudioContext)();
      if (ac.state === 'suspended') ac.resume();
      const len = Math.floor(ac.sampleRate * .05), buf = ac.createBuffer(1, len, ac.sampleRate), d = buf.getChannelData(0);
      for (let i = 0; i < len; i++) d[i] = (Math.random() * 2 - 1) * Math.pow(1 - i / len, 3);
      const click = (at, gain, freq) => {
        const src = ac.createBufferSource(), bp = ac.createBiquadFilter(), g = ac.createGain();
        src.buffer = buf; bp.type = 'bandpass'; bp.frequency.value = freq; bp.Q.value = .9;
        g.gain.setValueAtTime(gain, ac.currentTime + at);
        src.connect(bp).connect(g).connect(ac.destination);
        src.start(ac.currentTime + at);
      };
      click(0, .55, 2600);   // открытие шторки
      click(.075, .4, 1700); // закрытие
    } catch {}
  };

  const copyText = async text => {
    try { await navigator.clipboard.writeText(text); return true; } catch {}
    const ta = el('textarea', 'position:fixed;opacity:0;'); ta.value = text; root.append(ta); ta.select();
    let ok = false; try { ok = document.execCommand('copy'); } catch {} ta.remove(); return ok;
  };

  // снимок: та же карточка в конечном состоянии → PNG. При рисовании в canvas браузер
  // плавные анимации не проигрывает, а мгновенные <set> применяет — поэтому каждую
  // анимацию меняем на <set> с её конечным значением, начавшийся «давно».
  const stillSvg = s => s
    .replace(/<animate\b([^>]*?)\/>/g, (m, at) => {
      const name = /attributeName='([^']*)'/.exec(at)[1];
      const to = /\bto='([^']*)'/.exec(at)?.[1] ?? (/values='([^']*)'/.exec(at)?.[1] || '').split(';').pop();
      return `<set attributeName='${name}' to='${to}' fill='freeze'/>`;
    })
    .replace(/<set\b([^>]*?)\sbegin='[^']*'/g, '<set$1')
    .replace(/<set\b/g, "<set begin='-100s'");
  const toPng = (svg, w, h) => new Promise((ok, bad) => {
    const im = new Image();
    im.onload = () => {
      setTimeout(() => {
        try {
          const s = 2, cv2 = document.createElement('canvas');
          cv2.width = w * s; cv2.height = h * s;
          const g = cv2.getContext('2d');
          g.fillStyle = C.bg; g.fillRect(0, 0, cv2.width, cv2.height);
          g.drawImage(im, 0, 0, cv2.width, cv2.height);
          cv2.toBlob(b => b ? ok(b) : bad(new Error('пустой снимок')), 'image/png');
        } catch (e) { bad(e); }
      }, 60);
    };
    im.onerror = () => bad(new Error('не удалось нарисовать снимок'));
    im.src = svgUrl(stillSvg(svg));
  });

  // показать готовую SVG-карточку; поверх — ссылки, зоны копирования и кнопка снимка
  const show = (card, o = {}) => {
    const k = Math.min(1.25, (innerHeight - 64) / card.h, (innerWidth - 48) / card.w);
    wrap.style.width = card.w * k + 'px';
    wrap.style.height = card.h * k + 'px';
    wrap.style.borderRadius = 10 * k + 'px';
    const img = el('img', `display:block;width:100%;height:100%;max-width:none;margin:0;border:0;user-select:none;`, {
      src: svgUrl(card.svg), alt: 'Clod Scan', draggable: 'false'
    });
    const x = el('button', `all:initial;position:absolute;top:-12px;right:-12px;width:30px;height:30px;border-radius:50%;background:${C.surf2};box-shadow:inset 0 0 0 1px ${C.border};cursor:pointer;display:grid;place-items:center;`, { 'aria-label': 'Закрыть', title: 'Закрыть (Esc)' });
    x.append(el('img', 'display:block;width:14px;height:14px;', { src: icon(['M18 6 6 18', 'M6 6l12 12'], C.silver, 2.4), alt: '' }));
    x.addEventListener('click', close);
    const flash = el('div'); flash.className = 'flash';
    const toast = el('div'); toast.className = 'toast';
    wrap.append(img, flash, x, toast);
    const later = (node, at) => setTimeout(() => node.classList.add('on'), at * 1000);
    const pos = (z, pad = 0) => `left:${(z.x - pad) * k}px;top:${z.y * k}px;width:${(z.w + 2 * pad) * k}px;height:${z.h * k}px;`;
    let tt;
    const say = (text, ico) => {
      toast.replaceChildren(el('img', '', { src: icon(ico, C.em, 2.4), alt: '' }), document.createTextNode(text));
      toast.classList.add('on'); clearTimeout(tt); tt = setTimeout(() => toast.classList.remove('on'), 2600);
    };

    for (const l of o.links || []) {
      const a = el('a', pos(l) + `border-radius:${10 * k}px;`, { href: l.href, target: '_blank', rel: 'noopener', 'aria-label': l.label });
      a.className = 'z'; later(a, l.at); wrap.append(a);
    }

    // строка с Org ID: наведение — «Копировать», нажатие — копирует
    for (const c of o.copy || []) {
      const z = el('div', pos(c, 8), { role: 'button', tabindex: '0', 'aria-label': 'Скопировать ' + c.label, title: c.text });
      z.className = 'z cp';
      // подсказка: справа от подписи строки, а у центрированного блока — над ним
      const tag = el('span', c.tagX != null ? `left:${(c.tagX + 16) * k}px;font-size:${11 * k}px;`
        : `left:50%;top:auto;bottom:calc(100% + ${4 * k}px);transform:translateX(-50%);font-size:${11 * k}px;`);
      tag.className = 'tag';
      const ti = el('img', '', { src: icon(I_COPY, C.dim), alt: '' }), tl = document.createTextNode('Копировать');
      tag.append(ti, tl); z.append(tag);
      const go = async () => {
        if (!(await copyText(c.text))) return say('Не получилось скопировать', I_COPY);
        tag.classList.add('ok'); ti.src = icon(I_OK, C.em, 2.6); tl.data = 'Скопировано';
        setTimeout(() => { tag.classList.remove('ok'); ti.src = icon(I_COPY, C.dim); tl.data = 'Копировать'; }, 1600);
      };
      z.addEventListener('click', go);
      z.addEventListener('keydown', e => { if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); go(); } });
      later(z, c.at); wrap.append(z);
    }

    // кнопка снимка в центре шапки
    if (o.snap) {
      const b = el('button', `left:${card.w / 2 * k}px;top:${20 * k}px;height:${26 * k}px;padding:0 ${11 * k}px 0 ${9 * k}px;gap:${6 * k}px;font-size:${12 * k}px;`, { 'aria-label': 'Снимок результата', title: 'Снимок результата — в буфер обмена' });
      b.className = 'snap';
      b.append(el('img', `width:${14 * k}px;height:${14 * k}px;`, { src: icon(I_CAM, C.silver, 1.9), alt: '' }), document.createTextNode('Снимок'));
      let busy = false;
      b.addEventListener('click', async () => {
        if (busy) return; busy = true;
        shutter();
        flash.classList.remove('go'); wrap.classList.remove('shot'); void wrap.offsetWidth;
        flash.classList.add('go'); wrap.classList.add('shot');
        const blob = toPng(card.svg, card.w, card.h);
        try {
          await navigator.clipboard.write([new ClipboardItem({ 'image/png': blob })]);
          say(`Снимок в буфере — вставьте ${isMac ? '⌘V' : 'Ctrl+V'}`, I_OK);
        } catch (e) {
          console.warn('Clod Scan: буфер обмена не принял снимок —', e?.name, e?.message);
          try {
            const a = el('a', '', { href: URL.createObjectURL(await blob), download: 'clod-scan.png' });
            root.append(a); a.click(); a.remove();
            say('Снимок сохранён в загрузки', I_OK);
          } catch { say('Не удалось сделать снимок', I_CAM); }
        }
        setTimeout(() => { busy = false; }, 600);
      });
      later(b, o.snap.at); wrap.append(b);
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
  // Личная организация аккаунта. У человека в рабочей подписке (Team / Enterprise) организаций
  // несколько: такие у Claude помечены возможностью «raven» (или полем raven_type / тарифом *raven*).
  // Личная — с чатом и без raven; при равенстве — та, в названии которой есть почта.
  const pickOrg = acc => {
    const orgs = (acc.memberships || []).map(m => m.organization).filter(o => o?.uuid);
    const mail = String(acc.email_address || acc.email || '').toLowerCase();
    const team = o => (o.capabilities || []).includes('raven') || !!o.raven_type || /raven/i.test(o.rate_limit_tier || '');
    const chat = o => !o.capabilities || o.capabilities.includes('chat');
    const named = o => !!mail && String(o.name || '').toLowerCase().includes(mail);
    const score = o => (team(o) ? 0 : 4) + (chat(o) ? 2 : 0) + (named(o) ? 1 : 0);
    return orgs.slice().sort((a, b) => score(b) - score(a))[0] || null;
  };
  const load = async () => {
    const accR = await getJSON('/api/account');
    if (!accR.ok) throw accR;
    const acc = accR.data;
    const own = pickOrg(acc);
    const orgs = (acc.memberships || []).map(m => m.organization?.uuid).filter(Boolean);
    // KYC — сначала у личной организации, а не у той, что последней открыта на сайте
    const order = own ? [own.uuid, ...orgs.filter(o => o !== own.uuid)] : orgs;
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
    const p1 = 'Сканирование', p2 = res.demo ? 'Пример' : 'Проверено';
    // пилюля статуса: ширина ровно по тексту, после проверки плавно сжимается под «Проверено»
    const pw1 = tw(p1, `500 12px ${SANS}`) + 36, pw2 = tw(p2, `500 12px ${SANS}`) + 36;
    const px = w - P - pw1, px2 = w - P - pw2;
    const glide = (a, to) => `<animate attributeName='${a}' to='${to}' begin='${fT}s' dur='.4s' calcMode='spline' keyTimes='0;1' keySplines='.22 1 .36 1' fill='freeze'/>`;
    s += `<rect x='${px}' y='20' width='${pw1}' height='26' rx='13' fill='#0a0a0a' fill-opacity='.55' stroke='#fff' stroke-opacity='.05'>${glide('x', px2)}${glide('width', pw2)}</rect>`;
    s += `<circle cx='${px + 13}' cy='33' r='6' fill='${C.em}' opacity='0'><animate attributeName='r' values='3;8' dur='1.6s' repeatCount='indefinite'/><animate attributeName='opacity' values='.45;0' dur='1.6s' repeatCount='indefinite'/>${setAt('visibility', 'hidden', fT)}</circle>`;
    s += `<circle cx='${px + 13}' cy='33' r='3' fill='${C.em}'>${setAt('fill', ac, fT)}${glide('cx', px2 + 13)}</circle>`;
    s += t(px + 24, 37, p1, { size: 12, wt: 500, fill: C.silver, kids: setAt('opacity', '0', fT) }) +
         t(px2 + 24, 37, p2, { size: 12, wt: 500, fill: C.silver, op: 0, kids: fadeIn(f(T + .15), .3) });

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
    y += 22; r += ln(y); const y0r = y; y += 34;
    const copy = [];
    const fit = (v, font, room) => { // длинное значение: середину — в «…»
      if (tw(v, font) <= room) return v;
      let a = Math.ceil(v.length / 2), b = a;
      while (a > 3 && tw(v.slice(0, a) + '…' + v.slice(b), font) > room) { a--; b++; }
      return v.slice(0, a) + '…' + v.slice(b);
    };

    // кто это: Org ID и почта — блоком по центру
    {
      const lab = 'Org ID', lf = `400 11px ${SANS}`, vf = `500 13px ${MONO}`;
      const val = res.org || 'нет данных', vw = tw(val, res.org ? vf : `500 13px ${SANS}`), lw = tw(lab, lf);
      const x0 = Math.round((w - lw - 10 - vw) / 2);
      r += t(x0, y, lab, { size: 11, fill: C.faint }) +
           t(x0 + lw + 10, y, val, { size: 13, wt: 500, mono: !!res.org, fill: res.org ? C.fg : C.warn });
      if (res.org && !res.demo) copy.push({ x: x0 - 10, y: y - 18, w: lw + vw + 30, h: 26, text: res.org, label: lab, at: T + .9 });
      y += 22;
      r += t(w / 2, y, fit(res.mail || 'почта: нет данных', `400 12px ${SANS}`, bw), { size: 12, fill: C.dim, anchor: 'middle' });
      y += 22; r += ln(y); y += 30;
    }
    res.rows.forEach(([k, v, l, o = {}]) => {
      const cc = l ? COL[l] : C.fg;
      const font = o.mono ? `500 12px ${MONO}` : `500 13px ${SANS}`;
      v = fit(v, font, bw - tw(k, `400 13px ${SANS}`) - (l ? 40 : 24));
      r += t(P, y, k, { fill: C.dim });
      if (l) r += `<circle cx='${w - P - tw(v, font) - 10}' cy='${y - 4.5}' r='3' fill='${cc}'/>`;
      r += t(w - P, y, v, { wt: 500, fill: cc, anchor: 'end', mono: o.mono, size: o.mono ? 12 : 13 });
      if (o.copy) copy.push({ x: P, y: y - 19, w: bw, h: 27, tagX: tw(k, `400 13px ${SANS}`) + 6, text: o.copy, label: k, at: T + .9 });
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
    // когда сделана проверка — неброско, по центру внизу (видно и на снимке)
    r += res.demo
      ? t(w / 2, fy + 54 + 24, 'ПРИМЕР · не настоящие данные', { size: 11, wt: 600, fill: C.warn, anchor: 'middle' })
      : t(w / 2, fy + 54 + 24, res.when, { size: 11, mono: 1, fill: C.faint, anchor: 'middle' });
    const H = fy + 54 + 38;
    // пример: крупный косой штамп поверх результата — попадает и в снимок
    if (res.demo) {
      const cy = Math.round((y0r + fy + 54) / 2);
      const stamp = (yy, sz) => `<text x='${w / 2}' y='${yy}' text-anchor='middle' font-family='${SANS}' font-size='${sz}' font-weight='800' letter-spacing='6' fill='${C.warn}' fill-opacity='.13' stroke='${C.warn}' stroke-opacity='.45' stroke-width='1.5' transform='rotate(-24 ${w / 2} ${yy})'>ПРИМЕР</text>`;
      r += stamp(cy - 140, 60) + stamp(cy + 20, 84) + stamp(cy + 170, 60);
    }

    const defs = `<defs>${glow}<filter id='gl' x='-5%' y='-300%' width='110%' height='700%'><feGaussianBlur stdDeviation='3'/></filter></defs>`;
    const bg = `<rect x='.5' y='.5' width='${w - 1}' height='${H0 - 1}' rx='10' fill='${C.surf}' stroke='${C.border}'>` +
               `<animate attributeName='height' from='${H0 - 1}' to='${H - 1}' begin='${f(T + .15)}s' dur='.5s' calcMode='spline' keyTimes='0;1' keySplines='.22 1 .36 1' fill='freeze'/></rect>` +
               `<rect x='1' y='1' width='${w - 2}' height='170' rx='10' fill='url(#glo)'/>` +
               `<line x1='12' x2='${w - 12}' y1='1.5' y2='1.5' stroke='#fff' stroke-opacity='.04'/>`;
    return {
      svg: `<svg xmlns='http://www.w3.org/2000/svg' width='${w}' height='${H}'>${defs}${bg}${s}<g opacity='0'>${fadeIn(f(T + .35), .45)}${r}</g></svg>`,
      w, h: H, T, gh: { x: P, y: fy, w: bw, h: 54 }, copy
    };
  };

  // ===== запуск =====
  if (!DEMO && !/(^|\.)claude\.ai$/.test(location.hostname))
    return show(/github\.io$/.test(location.hostname)
      ? fail('Это страница инструкции', ['Закладку нажимают на claude.ai, а не здесь.', 'Откройте claude.ai, войдите и нажмите её там.'])
      : fail('Откройте claude.ai', ['Закладка работает только на сайте claude.ai.', 'Войдите в аккаунт и нажмите её там.']));

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
  const org = pickOrg(j);
  const mail = j.email_address || j.email || '';
  const earlyReg = created < CUTOFF;
  const state = !earlyReg ? 'deny' : kycOk ? 'ok' : 'warn';
  const yn = (v, yes, no) => v === true ? [yes, 'ok'] : v === false ? [no, 'deny'] : ['нет данных', 'warn'];

  const dur = STEPS.map(([, ms]) => Math.round(ms * (0.85 + Math.random() * 0.3)));
  const card = build(dur, {
    state, demo: !!DEMO,
    org: org?.uuid, mail,
    when: now.toLocaleDateString('ru-RU') + ' · ' + now.toLocaleTimeString('ru-RU', { hour: '2-digit', minute: '2-digit' }),
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
  show(card, {
    links: [{ ...card.gh, href: REPO, label: 'Clod Helper на GitHub', at: card.T + .5 }],
    copy: card.copy,
    snap: { at: card.T + .9 }
  });
})();
