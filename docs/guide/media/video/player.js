// Плеер видеоинструкции гайда (общий для new/, upgrade/, fresh/; взят со страницы docs/proxy/): главы на полосе перемотки, превью кадров, свои субтитры, панель глав справа.
(() => {
  "use strict";
  const root = document.getElementById("vp");
  if (!root) return;
  const q = (s) => root.querySelector(s);
  const video = q("video"), stage = q(".vp-stage");
  const seek = q(".vp-seek"), segsEl = q(".vp-segs"), knob = q(".vp-knob");
  const tip = q(".vp-tip"), tipTh = q(".vp-th"), tipT = q(".vp-tl b"), tipC = q(".vp-tl span");
  const capEl = q(".vp-cap span"), flash = q(".vp-flash"), flashSvg = flash.querySelector("svg");
  const cur = q(".vp-cur"), durEl = q(".vp-dur"), chapBtn = q(".vp-chap"), chapLbl = chapBtn.querySelector("span");
  const list = q(".vp-list"), side = q(".vp-side"), chBtn = q(".chbtn"), ccBtn = q(".cc");
  const sBtn = q(".sbtn"), menu = q(".vp-menu"), vol = q(".vp-vol input"), resumeBtn = q(".vp-resume");
  let DATA = { duration: 0, chapters: [] };
  try { DATA = JSON.parse(document.getElementById("vp-data").textContent); } catch (e) {}
  const CH = (DATA.chapters || []).map((c, i) => ({ t: +c.t || 0, title: c.title, a: c.a || null, i }));
  const SPEEDS = [0.75, 1, 1.25, 1.5, 1.75, 2];
  const KEY = "clodguide.video.v1";
  const BASE = root.dataset.base || "media/video/";
  const store = (() => { try { return JSON.parse(localStorage.getItem(KEY)) || {}; } catch (e) { return {}; } })();
  const persist = () => { try { localStorage.setItem(KEY, JSON.stringify(store)); } catch (e) {} };

  const fmt = (t) => {
    t = Math.max(0, Math.floor(t || 0));
    const h = Math.floor(t / 3600), m = Math.floor((t % 3600) / 60), s = t % 60;
    return (h ? h + ":" + String(m).padStart(2, "0") : m) + ":" + String(s).padStart(2, "0");
  };
  const total = () => (isFinite(video.duration) && video.duration > 0 ? video.duration : DATA.duration || 0);
  const chAt = (t) => { let k = 0; CH.forEach((c, i) => { if (t + 0.25 >= c.t) k = i; }); return k; };
  const chEnd = (i) => (i + 1 < CH.length ? CH[i + 1].t : total());
  const plural = (n, a, b, c) => n % 10 === 1 && n % 100 !== 11 ? a : n % 10 >= 2 && n % 10 <= 4 && (n % 100 < 10 || n % 100 >= 20) ? b : c;
  const play = () => { const p = video.play(); if (p && p.catch) p.catch(() => {}); };
  const toggle = () => { if (video.paused || video.ended) play(); else video.pause(); };
  const go = (t, andPlay) => {
    video.currentTime = Math.max(0, Math.min(total() - 0.05, t));
    root.classList.remove("ended");
    paint(video.currentTime);
    if (andPlay) play();
  };

  // ---- превью кадров (спрайт из thumbs.vtt) ----
  let thumbs = [];
  const thumbAt = (t) => thumbs.find((c) => t >= c.s && t < c.e) || thumbs[thumbs.length - 1];
  const setThumb = (el, t, w) => {
    const c = thumbAt(t); if (!c) return false;
    const k = w / c.w;
    el.style.backgroundImage = "url(" + c.url + ")";
    el.style.backgroundPosition = -c.x * k + "px " + -c.y * k + "px";
    el.style.backgroundSize = c.sw * k + "px auto";
    return true;
  };
  const toSec = (s) => s.trim().split(":").reduce((a, b) => a * 60 + parseFloat(b), 0);
  fetch(BASE + "thumbs.vtt").then((r) => (r.ok ? r.text() : "")).then((txt) => {
    const base = new URL(BASE, location.href);
    txt.split(/\r?\n\r?\n/).forEach((b) => {
      const m = b.match(/([\d:.]+)\s+-->\s+([\d:.]+)\s*\n\s*(\S+)#xywh=(\d+),(\d+),(\d+),(\d+)/);
      if (m) thumbs.push({ s: toSec(m[1]), e: toSec(m[2]), url: new URL(m[3], base).href, x: +m[4], y: +m[5], w: +m[6], h: +m[7] });
    });
    if (!thumbs.length) return;
    const img = new Image();
    img.onload = () => {
      thumbs.forEach((c) => { c.sw = img.naturalWidth; });
      list.querySelectorAll(".vp-ith").forEach((el, i) => { if (setThumb(el, CH[i].t + 5, 104)) el.classList.add("has"); });
    };
    img.src = thumbs[0].url;
  }).catch(() => {});

  // ---- главы: полоса, список, подпись ----
  const segs = [];
  const buildSegs = () => {
    segsEl.innerHTML = ""; segs.length = 0;
    const d = total(); if (!d) return;
    const parts = CH.length ? CH : [{ t: 0 }];
    parts.forEach((c, i) => {
      const s = document.createElement("div"); s.className = "vp-seg";
      s.innerHTML = '<i class="b"></i><i class="f"></i>';
      const end = i + 1 < parts.length ? parts[i + 1].t : d;
      s.style.flex = Math.max(0.001, end - c.t) + " 1 0";
      segsEl.appendChild(s);
      segs.push({ el: s, b: s.firstChild, f: s.lastChild, s: c.t, e: end });
    });
  };
  const items = CH.map((c, i) => {
    const li = document.createElement("li");
    li.innerHTML = '<button class="vp-it" type="button"><span class="vp-ith"><u></u><em></em><i></i></span><span><b></b><small><span class="tm"></span></small></span></button>';
    const btn = li.firstChild;
    btn.querySelector("u").textContent = i + 1;
    btn.querySelector("b").textContent = c.title;
    btn.querySelector(".tm").textContent = fmt(c.t);
    if (c.a) {
      const a = document.createElement("a"); a.className = "vp-txt"; a.href = c.a; a.textContent = "текстом →"; a.title = "Открыть этот шаг на странице";
      a.addEventListener("click", () => { if (matchMedia("(max-width:860px)").matches) setSide(false); });
      li.appendChild(a);
    }
    btn.addEventListener("click", () => {
      go(c.t, true);
      if (matchMedia("(max-width:860px)").matches) setSide(false);
    });
    list.appendChild(li);
    return { btn, bar: btn.querySelector("i"), dur: btn.querySelector("em") };
  });
  q(".vp-sc").textContent = CH.length + " " + plural(CH.length, "глава", "главы", "глав");
  q(".vp-nch").textContent = CH.length + " " + plural(CH.length, "глава", "главы", "глав");

  let lastCh = -1;
  const paint = (t) => {
    const d = total(), r = d ? Math.min(1, t / d) : 0;
    segs.forEach((g) => { g.f.style.transform = "scaleX(" + Math.min(1, Math.max(0, (t - g.s) / (g.e - g.s))) + ")"; });
    knob.style.left = r * 100 + "%";
    cur.textContent = fmt(t);
    seek.setAttribute("aria-valuenow", Math.round(r * 100));
    seek.setAttribute("aria-valuetext", fmt(t) + " из " + fmt(d));
    if (!CH.length) return;
    const k = chAt(t);
    items.forEach((it, i) => {
      it.bar.style.width = i < k ? "100%" : i > k ? "0" : Math.min(100, ((t - CH[i].t) / Math.max(0.1, chEnd(i) - CH[i].t)) * 100) + "%";
    });
    if (k !== lastCh) {
      lastCh = k;
      chapLbl.textContent = (k + 1) + ". " + CH[k].title;
      items.forEach((it, i) => it.btn.classList.toggle("on", i === k));
      if (root.classList.contains("side-on")) items[k].btn.scrollIntoView({ block: "nearest", behavior: "smooth" });
    }
  };
  const paintBuf = () => {
    const b = video.buffered; let end = 0;
    for (let i = 0; i < b.length; i++) if (b.start(i) <= video.currentTime + 0.5) end = Math.max(end, b.end(i));
    segs.forEach((g) => { g.b.style.transform = "scaleX(" + Math.min(1, Math.max(0, (end - g.s) / (g.e - g.s))) + ")"; });
  };
  const paintDur = () => {
    const d = total();
    durEl.textContent = fmt(d); q(".vp-dur0").textContent = d ? fmt(d) : "—";
    items.forEach((it, i) => { it.dur.textContent = fmt(chEnd(i) - CH[i].t); });
    buildSegs(); paint(video.currentTime || 0); paintBuf();
  };

  // ---- перемотка: тянуть, подсказка с кадром, клавиатура ----
  let dragging = false, hovSeg = null;
  const ratio = (x) => { const r = seek.getBoundingClientRect(); return Math.min(1, Math.max(0, (x - r.left) / r.width)); };
  const showTip = (r) => {
    const t = r * total(), k = chAt(t);
    tipT.textContent = fmt(t);
    tipC.textContent = CH.length ? CH[k].title : "";
    tip.classList.toggle("nothumb", !setThumb(tipTh, t, tipTh.offsetWidth || 192));
    const w = seek.getBoundingClientRect().width, half = (tip.offsetWidth || 200) / 2;
    tip.style.left = Math.min(w - half, Math.max(half, r * w)) + "px";
    seek.classList.add("tipping"); root.classList.add("tipping");
    const g = segs.find((s) => t >= s.s && t < s.e) || null;
    if (g !== hovSeg) { if (hovSeg) hovSeg.el.classList.remove("hov"); hovSeg = g; if (g) g.el.classList.add("hov"); }
  };
  const hideTip = () => { seek.classList.remove("tipping"); root.classList.remove("tipping"); if (hovSeg) hovSeg.el.classList.remove("hov"); hovSeg = null; };
  seek.addEventListener("pointerdown", (e) => {
    if (!total()) return;
    dragging = true; seek.setPointerCapture(e.pointerId); seek.classList.add("drag");
    const r = ratio(e.clientX); paint(r * total()); showTip(r);
  });
  seek.addEventListener("pointermove", (e) => {
    if (!total()) return;
    const r = ratio(e.clientX);
    if (dragging) paint(r * total());
    if (dragging || e.pointerType === "mouse") showTip(r);
  });
  seek.addEventListener("pointerup", (e) => {
    if (!dragging) return;
    dragging = false; seek.classList.remove("drag"); if (e.pointerType !== "mouse") hideTip();
    go(ratio(e.clientX) * total(), !root.classList.contains("started") || !video.paused);
  });
  seek.addEventListener("pointercancel", () => { dragging = false; seek.classList.remove("drag"); hideTip(); });
  seek.addEventListener("pointerleave", () => { if (!dragging) hideTip(); });
  seek.addEventListener("keydown", (e) => {
    const st = { ArrowLeft: -5, ArrowRight: 5, PageDown: -30, PageUp: 30 }[e.key];
    if (st) { e.preventDefault(); e.stopPropagation(); go(video.currentTime + st); poke(); }
    else if (e.key === "Home") { e.preventDefault(); go(0); }
    else if (e.key === "End") { e.preventDefault(); go(total()); }
  });

  // ---- показ/скрытие панели управления ----
  let idleTimer = 0;
  const poke = () => {
    root.classList.remove("idle"); clearTimeout(idleTimer);
    idleTimer = setTimeout(() => {
      if (!video.paused && !dragging && !menu.classList.contains("on") && !q(".vp-ui").matches(":hover")) root.classList.add("idle");
    }, 2600);
  };
  stage.addEventListener("pointermove", poke);
  stage.addEventListener("pointerdown", poke);
  root.addEventListener("focusin", poke);
  stage.addEventListener("pointerleave", () => { if (!video.paused) { clearTimeout(idleTimer); idleTimer = setTimeout(() => root.classList.add("idle"), 800); } });

  // ---- кнопки ----
  const ICON_PLAY = '<path d="M7 4.5v15a1 1 0 0 0 1.52.85l12.2-7.5a1 1 0 0 0 0-1.7L8.52 3.65A1 1 0 0 0 7 4.5"/>';
  const ICON_PAUSE = '<rect x="6" y="4.5" width="4" height="15" rx="1.2"/><rect x="14" y="4.5" width="4" height="15" rx="1.2"/>';
  const doFlash = (html) => { flashSvg.innerHTML = html; flash.classList.remove("go"); void flash.offsetWidth; flash.classList.add("go"); };
  const skipFx = (d) => { const el = q(d < 0 ? ".vp-skip.l" : ".vp-skip.r"); el.classList.remove("go"); void el.offsetWidth; el.classList.add("go"); };
  const skip = (d) => { go(video.currentTime + d); skipFx(d); poke(); };

  q(".vp-cover").addEventListener("click", () => { root.classList.add("started"); play(); });
  resumeBtn.addEventListener("click", () => { root.classList.add("started"); go(store.t || 0, true); });
  q(".vp-again").addEventListener("click", () => go(0, true));
  q(".vp-pp").addEventListener("click", toggle);
  q(".vp-bk").addEventListener("click", () => skip(-10));
  q(".vp-fw").addEventListener("click", () => skip(10));

  let lastTap = 0, tapTimer = 0;
  video.addEventListener("click", (e) => {
    if (e.pointerType === "touch" || e.pointerType === "pen") {
      // касание: первое показывает панель, двойное по краям — ±10 с
      const now = Date.now(), x = e.offsetX / video.clientWidth;
      if (now - lastTap < 320 && (x < 0.33 || x > 0.67)) { clearTimeout(tapTimer); skip(x < 0.33 ? -10 : 10); lastTap = 0; return; }
      lastTap = now;
      if (root.classList.contains("idle")) { poke(); return; }
      clearTimeout(tapTimer); tapTimer = setTimeout(() => { toggle(); doFlash(video.paused ? ICON_PAUSE : ICON_PLAY); }, 330);
      return;
    }
    toggle(); doFlash(video.paused ? ICON_PAUSE : ICON_PLAY);
  });
  video.addEventListener("dblclick", (e) => { if (e.pointerType !== "touch") fs(); });

  // ---- кликабельные места в кадре ----
  // s/e — секунды ролика, x/y/w/h — доли кадра 1920×1080 (сняты с готового mp4).
  // Ссылка открывается в новой вкладке, ролик встаёт на паузу.
  const SPOTS = [
    { s: 15.7, e: 21.8, x: 0.224, y: 0.435, w: 0.548, h: 0.272, href: "../../proxy/#video", label: "Открыть видео «Свой прокси» в новой вкладке" },
  ];
  const spots = SPOTS.map((sp) => {
    const a = document.createElement("a");
    a.className = "vp-spot"; a.href = sp.href; a.target = "_blank"; a.rel = "noopener";
    a.setAttribute("aria-label", sp.label); a.title = sp.label; a.tabIndex = -1;
    a.innerHTML = '<span>Открыть<svg viewBox="0 0 24 24"><path d="M15 3h6v6"/><path d="M10 14 21 3"/><path d="M18 13v6a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h6"/></svg></span>';
    a.addEventListener("click", () => video.pause());
    video.after(a);
    return { sp, a };
  });
  const placeSpots = () => {
    const W = stage.clientWidth, H = stage.clientHeight;
    const vw = Math.min(W, H * 16 / 9), vh = vw * 9 / 16, ox = (W - vw) / 2, oy = (H - vh) / 2;
    spots.forEach(({ sp, a }) => Object.assign(a.style, { left: ox + sp.x * vw + "px", top: oy + sp.y * vh + "px", width: sp.w * vw + "px", height: sp.h * vh + "px" }));
  };
  const showSpots = () => {
    const t = video.currentTime;
    spots.forEach(({ sp, a }) => { const on = root.classList.contains("started") && t >= sp.s && t < sp.e; a.classList.toggle("on", on); a.tabIndex = on ? 0 : -1; });
  };
  if (spots.length) {
    placeSpots();
    if (window.ResizeObserver) new ResizeObserver(placeSpots).observe(stage); else addEventListener("resize", placeSpots);
    ["timeupdate", "seeked", "play", "pause"].forEach((ev) => video.addEventListener(ev, showSpots));
  }

  // громкость
  const applyVol = () => {
    vol.value = video.muted ? 0 : video.volume;
    root.classList.toggle("muted", video.muted || video.volume === 0);
    vol.style.background = "";
  };
  if (typeof store.vol === "number") video.volume = store.vol;
  q(".vp-mute").addEventListener("click", () => {
    if (video.muted || video.volume === 0) { video.muted = false; if (video.volume === 0) video.volume = 0.6; } else video.muted = true;
  });
  vol.addEventListener("input", () => { video.volume = +vol.value; video.muted = +vol.value === 0; });
  video.addEventListener("volumechange", () => { applyVol(); store.vol = video.volume; persist(); });
  applyVol();

  // субтитры: дорожка скрыта, текст рисуем сами — так его видно над панелью и в полноэкранном режиме
  let ccOn = store.cc !== false;
  const track = video.textTracks && video.textTracks[0];
  const showCue = () => {
    if (!track) return;
    const cues = track.activeCues ? Array.from(track.activeCues) : [];
    capEl.textContent = cues.map((c) => c.text.replace(/<[^>]+>/g, "")).join("\n");
  };
  if (track) { track.mode = "hidden"; track.addEventListener("cuechange", showCue); }
  const setCC = (on) => {
    ccOn = on; root.classList.toggle("cc-off", !on); ccBtn.classList.toggle("on", on);
    ccBtn.setAttribute("aria-pressed", on); store.cc = on; persist();
  };
  ccBtn.addEventListener("click", () => setCC(!ccOn));
  setCC(ccOn);

  // скорость
  SPEEDS.forEach((s) => {
    const b = document.createElement("button"); b.type = "button"; b.setAttribute("role", "menuitemradio");
    b.textContent = (s === 1 ? "Обычная" : String(s).replace(".", ",") + "×"); b.dataset.s = s;
    b.addEventListener("click", () => { video.playbackRate = s; closeMenu(); });
    menu.appendChild(b);
  });
  const paintSpeed = () => {
    const r = video.playbackRate;
    sBtn.textContent = String(r).replace(".", ",") + "×";
    sBtn.classList.toggle("on", r !== 1);
    menu.querySelectorAll("button").forEach((b) => { b.classList.toggle("on", +b.dataset.s === r); b.setAttribute("aria-checked", +b.dataset.s === r); });
  };
  const closeMenu = () => { menu.classList.remove("on"); sBtn.setAttribute("aria-expanded", "false"); };
  sBtn.addEventListener("click", (e) => { e.stopPropagation(); const on = !menu.classList.contains("on"); menu.classList.toggle("on", on); sBtn.setAttribute("aria-expanded", on); poke(); });
  document.addEventListener("click", (e) => { if (!menu.contains(e.target)) closeMenu(); });
  video.addEventListener("ratechange", paintSpeed);
  paintSpeed();

  // картинка в картинке
  const pip = q(".vp-pip");
  if (document.pictureInPictureEnabled && !video.disablePictureInPicture) {
    pip.hidden = false;
    pip.addEventListener("click", () => {
      if (document.pictureInPictureElement) document.exitPictureInPicture().catch(() => {});
      else video.requestPictureInPicture().catch(() => {});
    });
  }

  // панель глав
  const setSide = (on) => {
    root.classList.toggle("side-on", on);
    chBtn.classList.toggle("on", on); chBtn.setAttribute("aria-expanded", on);
    store.side = on; persist();
    if (on && lastCh >= 0) setTimeout(() => items[lastCh].btn.scrollIntoView({ block: "nearest" }), 60);
  };
  chBtn.addEventListener("click", () => setSide(!root.classList.contains("side-on")));
  chapBtn.addEventListener("click", () => setSide(true));
  q(".vp-close").addEventListener("click", () => setSide(false));
  if (store.side && matchMedia("(min-width:861px)").matches) setSide(true);

  // во весь экран — вместе с панелью глав
  const fsEl = () => document.fullscreenElement || document.webkitFullscreenElement;
  const fs = () => {
    if (fsEl()) return (document.exitFullscreen || document.webkitExitFullscreen).call(document);
    if (root.requestFullscreen) root.requestFullscreen().catch(() => {});
    else if (root.webkitRequestFullscreen) root.webkitRequestFullscreen();
    else if (video.webkitEnterFullscreen) video.webkitEnterFullscreen(); // iPhone
  };
  q(".vp-fs").addEventListener("click", fs);
  const onFs = () => { root.classList.toggle("fs", fsEl() === root); poke(); };
  document.addEventListener("fullscreenchange", onFs);
  document.addEventListener("webkitfullscreenchange", onFs);

  // ---- горячие клавиши (когда плеер в фокусе или на экране) ----
  let visible = false;
  if ("IntersectionObserver" in window) new IntersectionObserver((es) => { visible = es[0].intersectionRatio > 0.4; }, { threshold: [0, 0.4, 1] }).observe(root);
  document.addEventListener("keydown", (e) => {
    const tag = (e.target.tagName || "").toLowerCase();
    if (e.ctrlKey || e.metaKey || e.altKey || tag === "input" || tag === "textarea" || tag === "select") return;
    if (!root.contains(e.target) && !(visible && e.target === document.body) && !fsEl()) return;
    const k = e.key.toLowerCase(), code = e.code;
    const is = (c, ...ks) => code === c || ks.includes(k);
    if ((k === " " || k === "enter") && (tag === "button" || tag === "a")) return;
    let hit = true;
    if (is("KeyK", " ", "k", "л")) { root.classList.add("started"); toggle(); doFlash(video.paused ? ICON_PAUSE : ICON_PLAY); }
    else if (is("KeyJ", "j", "о")) skip(-10);
    else if (is("KeyL", "l", "д")) skip(10);
    else if (k === "arrowleft") skip(-5);
    else if (k === "arrowright") skip(5);
    else if (k === "arrowup" && root.contains(e.target)) { video.muted = false; video.volume = Math.min(1, video.volume + 0.1); }
    else if (k === "arrowdown" && root.contains(e.target)) video.volume = Math.max(0, video.volume - 0.1);
    else if (is("KeyM", "m", "ь")) q(".vp-mute").click();
    else if (is("KeyF", "f", "а")) fs();
    else if (is("KeyC", "c", "с")) setCC(!ccOn);
    else if (is("KeyG", "g", "п")) setSide(!root.classList.contains("side-on"));
    else if (k === "escape" && root.classList.contains("side-on") && !fsEl()) setSide(false);
    else if (/^digit[0-9]$/.test(code.toLowerCase())) go(total() * (+code.slice(5)) / 10);
    else if ((k === ">" || k === "ю") && e.shiftKey) video.playbackRate = SPEEDS[Math.min(SPEEDS.length - 1, SPEEDS.indexOf(video.playbackRate) + 1)] || 1;
    else if ((k === "<" || k === "б") && e.shiftKey) video.playbackRate = SPEEDS[Math.max(0, SPEEDS.indexOf(video.playbackRate) - 1)] || 1;
    else hit = false;
    if (hit) { e.preventDefault(); poke(); }
  });

  // ---- состояние видео ----
  const sync = () => {
    const playing = !video.paused && !video.ended;
    root.classList.toggle("playing", playing);
    q(".vp-pp").setAttribute("aria-label", playing ? "Пауза (K)" : "Смотреть (K)");
    if (playing) { root.classList.add("started"); root.classList.remove("ended"); poke(); }
    else { clearTimeout(idleTimer); root.classList.remove("idle"); }
  };
  ["play", "pause", "playing"].forEach((ev) => video.addEventListener(ev, sync));
  video.addEventListener("ended", () => { sync(); root.classList.add("ended"); store.t = 0; persist(); });
  let saveAt = 0;
  video.addEventListener("timeupdate", () => {
    if (!dragging) paint(video.currentTime);
    const now = Date.now();
    if (now - saveAt > 4000) { saveAt = now; store.t = video.currentTime > 5 && video.currentTime < total() - 15 ? Math.floor(video.currentTime) : 0; persist(); }
  });
  video.addEventListener("progress", paintBuf);
  video.addEventListener("waiting", () => root.classList.add("loading"));
  ["playing", "canplay", "seeked", "pause"].forEach((ev) => video.addEventListener(ev, () => root.classList.remove("loading")));
  video.addEventListener("loadedmetadata", paintDur);
  addEventListener("resize", () => { if (hovSeg) hideTip(); });
  paintDur();

  // продолжить с места, где остановились
  if (store.t > 5) { resumeBtn.hidden = false; resumeBtn.querySelector("b").textContent = fmt(store.t); }

  // ссылка на момент: #t=95, #t=1:35 или #t=1m35s
  const fromHash = () => {
    const m = location.hash.match(/^#t=([\dhms:.]+)$/i);
    if (!m) return;
    const s = m[1];
    const t = s.includes(":") ? toSec(s) : /[hms]/i.test(s)
      ? ((+(s.match(/(\d+)h/i) || [0, 0])[1]) * 3600 + (+(s.match(/(\d+)m/i) || [0, 0])[1]) * 60 + (+(s.match(/(\d+)s/i) || [0, 0])[1]))
      : +s;
    root.scrollIntoView({ block: "center" });
    root.classList.add("started"); go(t, false);
  };
  addEventListener("hashchange", fromHash); fromHash();

  // плашки «▶ В видео» у шагов страницы: по полю "a" глав (#id секции), первая глава на секцию
  const seen = new Set();
  CH.forEach((c) => {
    if (!c.a || seen.has(c.a)) return; seen.add(c.a);
    const sec = document.querySelector(c.a); if (!sec) return;
    const st = sec.querySelector(".stp-h .t") || sec.querySelector("h2"); if (!st) return;
    const a = document.createElement("a"); a.className = "vjump"; a.href = "#video";
    a.innerHTML = '<svg viewBox="0 0 24 24"><path d="M7 4.5v15a1 1 0 0 0 1.52.85l12.2-7.5a1 1 0 0 0 0-1.7L8.52 3.65A1 1 0 0 0 7 4.5"/></svg>В видео <b></b>';
    a.querySelector("b").textContent = fmt(c.t);
    a.addEventListener("click", (e) => {
      e.preventDefault();
      root.scrollIntoView({ behavior: matchMedia("(prefers-reduced-motion:reduce)").matches ? "auto" : "smooth", block: "center" });
      root.classList.add("started"); go(c.t, true);
    });
    if (st.tagName === "H2") st.appendChild(a); else st.after(a);
  });

  // карточка в шторке телефона и на экране блокировки
  if ("mediaSession" in navigator) {
    navigator.mediaSession.metadata = new MediaMetadata({ title: "Clod Helper: настройка по шагам", artist: "Clod Helper · гайд", artwork: [{ src: new URL(BASE + "poster.jpg", location.href).href, sizes: "1920x1080", type: "image/jpeg" }] });
    const h = (a, f) => { try { navigator.mediaSession.setActionHandler(a, f); } catch (e) {} };
    h("seekbackward", () => skip(-10)); h("seekforward", () => skip(10));
    h("previoustrack", () => go(CH[Math.max(0, chAt(video.currentTime - 2))]?.t || 0, true));
    h("nexttrack", () => { const n = CH[chAt(video.currentTime) + 1]; if (n) go(n.t, true); });
  }
})();
