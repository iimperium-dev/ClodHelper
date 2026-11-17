// Видеоплеер портала: оживляет каждый <div class="player">. Стили — assets/player.css.
(() => {
  const $ = (s, r = document) => r.querySelector(s);
  const fmt = s => { s = Math.max(0, Math.floor(s || 0)); return Math.floor(s / 60) + ':' + String(s % 60).padStart(2, '0'); };
  const SYM = {
    play: '<path d="M7 4.6v14.8a1 1 0 0 0 1.52.85l12.1-7.4a1 1 0 0 0 0-1.7L8.52 3.75A1 1 0 0 0 7 4.6z"/>',
    pause: '<rect x="5.5" y="4" width="4.5" height="16" rx="1.2"/><rect x="14" y="4" width="4.5" height="16" rx="1.2"/>'
  };
  const ico = id => `<svg class="f" viewBox="0 0 24 24">${SYM[id]}</svg>`;
  const I = {
    vol: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M11 4.7a.7.7 0 0 0-1.2-.5L6.4 7.6A1.4 1.4 0 0 1 5.4 8H3a1 1 0 0 0-1 1v6a1 1 0 0 0 1 1h2.4a1.4 1.4 0 0 1 1 .4l3.4 3.4a.7.7 0 0 0 1.2-.5z"/><path d="M16 9a5 5 0 0 1 0 6"/><path d="M19.4 18.4a9 9 0 0 0 0-12.8"/></svg>',
    low: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M11 4.7a.7.7 0 0 0-1.2-.5L6.4 7.6A1.4 1.4 0 0 1 5.4 8H3a1 1 0 0 0-1 1v6a1 1 0 0 0 1 1h2.4a1.4 1.4 0 0 1 1 .4l3.4 3.4a.7.7 0 0 0 1.2-.5z"/><path d="M16 9a5 5 0 0 1 0 6"/></svg>',
    mute: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M11 4.7a.7.7 0 0 0-1.2-.5L6.4 7.6A1.4 1.4 0 0 1 5.4 8H3a1 1 0 0 0-1 1v6a1 1 0 0 0 1 1h2.4a1.4 1.4 0 0 1 1 .4l3.4 3.4a.7.7 0 0 0 1.2-.5z"/><path d="m22 9-6 6"/><path d="m16 9 6 6"/></svg>',
    full: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M8 3H5a2 2 0 0 0-2 2v3"/><path d="M21 8V5a2 2 0 0 0-2-2h-3"/><path d="M3 16v3a2 2 0 0 0 2 2h3"/><path d="M16 21h3a2 2 0 0 0 2-2v-3"/></svg>',
    exit: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M8 3v3a2 2 0 0 1-2 2H3"/><path d="M21 8h-3a2 2 0 0 1-2-2V3"/><path d="M3 16h3a2 2 0 0 1 2 2v3"/><path d="M16 21v-3a2 2 0 0 1 2-2h3"/></svg>'
  };
  const players = [];
  // громкость общая для всех роликов сайта и запоминается; по умолчанию 70%
  const store = { get(k) { try { return localStorage.getItem(k); } catch { return null; } }, set(k, v) { try { localStorage.setItem(k, v); } catch {} } };
  let volume = parseFloat(store.get('clodscan.volume'));
  if (!(volume >= 0 && volume <= 1)) volume = 0.7;
  const volSync = [];

  document.querySelectorAll('.player').forEach(p => {
    const d = p.dataset;
    p.tabIndex = 0;
    p.setAttribute('role', 'region');
    p.setAttribute('aria-label', 'Видео: ' + d.title);
    p.innerHTML =
      `<video preload="metadata" playsinline webkit-playsinline poster="${d.poster}" src="${d.src}"></video>` +
      `<div class="flash"></div>` +
      `<div class="cover" style="background-image:url('${d.poster}')"><span class="big">${ico('play')}</span>` +
        `<div class="meta"><div><b>${d.title}</b><span>${d.note}</span></div><span class="dur">${d.dur}</span></div></div>` +
      `<div class="bar">` +
        `<button class="btn pp" aria-label="Пауза">${ico('pause')}</button>` +
        `<span class="time">0:00 / ${d.dur}</span>` +
        `<div class="track" role="slider" aria-label="Перемотка" tabindex="-1"><div class="rail"><div class="buf"></div><div class="fill"></div><div class="knob"></div></div><span class="pv">0:00</span></div>` +
        `<div class="volwrap vol"><button class="btn vb" aria-label="Выключить звук">${I.vol}</button>` +
          `<div class="vs" role="slider" tabindex="0" aria-label="Громкость" aria-valuemin="0" aria-valuemax="100"><div class="rail"><div class="fill"></div><div class="knob"></div></div></div></div>` +
        `<button class="btn fs" aria-label="На весь экран">${I.full}</button>` +
      `</div>`;
    const v = $('video', p), pp = $('.pp', p), time = $('.time', p), track = $('.track', p), fill = $('.fill', p), buf = $('.buf', p),
          knob = $('.track .knob', p), pv = $('.pv', p), vol = $('.vb', p), fs = $('.fs', p), flash = $('.flash', p),
          vw = $('.volwrap', p), vs = $('.vs', p), vfill = $('.vs .fill', p), vknob = $('.vs .knob', p);
    players.push(v);
    v.volume = volume;

    let idle;
    const wake = () => { p.classList.add('ui'); clearTimeout(idle); idle = setTimeout(() => { if (!v.paused && !track.classList.contains('drag')) p.classList.remove('ui'); }, 2200); };
    const flashIcon = id => { flash.innerHTML = ico(id); flash.classList.remove('go'); void flash.offsetWidth; flash.classList.add('go'); };
    const toggle = () => { if (v.paused || v.ended) { players.forEach(o => o !== v && o.pause()); v.play(); flashIcon('play'); } else { v.pause(); flashIcon('pause'); } };
    const paint = () => {
      const k = v.duration ? v.currentTime / v.duration : 0;
      fill.style.width = knob.style.left = k * 100 + '%';
      time.textContent = fmt(v.currentTime) + ' / ' + (v.duration ? fmt(v.duration) : d.dur);
      track.setAttribute('aria-valuenow', Math.round(v.currentTime));
    };

    $('.cover', p).addEventListener('click', () => { p.classList.remove('ended'); p.classList.add('started'); players.forEach(o => o !== v && o.pause()); v.currentTime = v.ended ? 0 : v.currentTime; v.play(); wake(); });
    v.addEventListener('click', () => { if (matchMedia('(hover:none)').matches && !p.classList.contains('ui')) return wake(); toggle(); wake(); });
    v.addEventListener('dblclick', () => fs.click());
    v.addEventListener('play', () => { p.classList.remove('paused', 'ended'); pp.innerHTML = ico('pause'); pp.setAttribute('aria-label', 'Пауза'); wake(); });
    v.addEventListener('pause', () => { p.classList.add('paused'); pp.innerHTML = ico('play'); pp.setAttribute('aria-label', 'Смотреть'); });
    v.addEventListener('ended', () => { p.classList.add('ended'); $('.meta b', p).textContent = 'Посмотреть ещё раз'; });
    v.addEventListener('timeupdate', paint);
    v.addEventListener('loadedmetadata', () => { track.setAttribute('aria-valuemax', Math.round(v.duration)); paint(); });
    v.addEventListener('progress', () => { if (v.duration && v.buffered.length) buf.style.width = v.buffered.end(v.buffered.length - 1) / v.duration * 100 + '%'; });
    pp.addEventListener('click', toggle);
    p.addEventListener('pointermove', e => { if (e.pointerType === 'mouse') wake(); });
    p.addEventListener('touchstart', wake, { passive: true });

    // перемотка мышью и пальцем
    const at = e => { const r = track.getBoundingClientRect(); return Math.min(1, Math.max(0, (e.clientX - r.left) / r.width)); };
    const preview = e => { const k = at(e); pv.style.left = k * 100 + '%'; pv.textContent = fmt(k * (v.duration || 0)); };
    track.addEventListener('pointermove', e => { preview(e); if (track.classList.contains('drag') && v.duration) { v.currentTime = at(e) * v.duration; paint(); } });
    track.addEventListener('pointerdown', e => { track.setPointerCapture(e.pointerId); track.classList.add('drag'); preview(e); if (v.duration) { v.currentTime = at(e) * v.duration; paint(); } });
    const up = () => { track.classList.remove('drag'); wake(); };
    track.addEventListener('pointerup', up); track.addEventListener('pointercancel', up);

    // звук: кнопка — выключить/включить, ползунок — громкость
    const vpaint = () => {
      const k = v.muted ? 0 : v.volume;
      vfill.style.width = vknob.style.left = k * 100 + '%';
      vol.innerHTML = k === 0 ? I.mute : k < 0.5 ? I.low : I.vol;
      vol.setAttribute('aria-label', k === 0 ? 'Включить звук' : 'Выключить звук');
      vs.setAttribute('aria-valuenow', Math.round(k * 100));
    };
    volSync.push(() => { v.volume = volume; vpaint(); });
    const setVol = k => {
      volume = Math.min(1, Math.max(0, k));
      v.muted = volume === 0;
      store.set('clodscan.volume', volume);
      volSync.forEach(f => f());
    };
    vol.addEventListener('click', () => {
      if (v.muted || v.volume === 0) { v.muted = false; if (v.volume === 0) setVol(0.5); }
      else v.muted = true;
      vpaint();
    });
    const vat = e => { const r = $('.rail', vs).getBoundingClientRect(); return (e.clientX - r.left) / r.width; };
    vs.addEventListener('pointerdown', e => { vs.setPointerCapture(e.pointerId); vw.classList.add('drag'); setVol(vat(e)); wake(); });
    vs.addEventListener('pointermove', e => { if (vw.classList.contains('drag')) { setVol(vat(e)); wake(); } });
    const vup = () => vw.classList.remove('drag');
    vs.addEventListener('pointerup', vup); vs.addEventListener('pointercancel', vup);
    vs.addEventListener('keydown', e => {
      const d = { ArrowRight: .1, ArrowUp: .1, ArrowLeft: -.1, ArrowDown: -.1 }[e.key];
      if (d) { e.preventDefault(); e.stopPropagation(); setVol((v.muted ? 0 : volume) + d); wake(); }
    });
    vs.addEventListener('wheel', e => { e.preventDefault(); setVol((v.muted ? 0 : volume) - Math.sign(e.deltaY) * .05); wake(); }, { passive: false });
    vpaint();

    // весь экран (iPhone умеет только своим плеером)
    const isFs = () => (document.fullscreenElement || document.webkitFullscreenElement) === p;
    fs.addEventListener('click', () => {
      if (isFs()) (document.exitFullscreen || document.webkitExitFullscreen).call(document);
      else if (p.requestFullscreen) p.requestFullscreen();
      else if (p.webkitRequestFullscreen) p.webkitRequestFullscreen();
      else if (v.webkitEnterFullscreen) v.webkitEnterFullscreen();
    });
    const fsc = () => { fs.innerHTML = isFs() ? I.exit : I.full; fs.setAttribute('aria-label', isFs() ? 'Выйти из полноэкранного режима' : 'На весь экран'); };
    document.addEventListener('fullscreenchange', fsc); document.addEventListener('webkitfullscreenchange', fsc);

    // клавиатура
    p.addEventListener('keydown', e => {
      if (e.target.closest('button') && (e.key === ' ' || e.key === 'Enter')) return;
      const k = e.key.toLowerCase();
      if (k === ' ' || k === 'k') { e.preventDefault(); if (!p.classList.contains('started')) $('.cover', p).click(); else toggle(); }
      else if (k === 'arrowright' || k === 'arrowleft') { e.preventDefault(); v.currentTime += k === 'arrowright' ? 5 : -5; wake(); }
      else if (k === 'arrowup' || k === 'arrowdown') { e.preventDefault(); setVol((v.muted ? 0 : volume) + (k === 'arrowup' ? .1 : -.1)); wake(); }
      else if (k === 'm') vol.click();
      else if (k === 'f') fs.click();
    });
  });
})();
