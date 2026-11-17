// Страницы-инструкции: подсветка раздела в оглавлении, отметки «Сделано»,
// поиск по вопросам, «скопировать ссылку» и раскрытие вопроса по #якорю.
(() => {
  const $ = (s, r = document) => r.querySelector(s);
  const $$ = (s, r = document) => Array.from(r.querySelectorAll(s));
  const store = {
    get(k, d) { try { return JSON.parse(localStorage.getItem(k)) ?? d; } catch { return d; } },
    set(k, v) { try { localStorage.setItem(k, JSON.stringify(v)); } catch {} },
  };

  // ---- оглавление: какой раздел сейчас на экране ----
  const links = $$(".toc a[href^='#']");
  const targets = links.map((a) => document.getElementById(a.hash.slice(1))).filter(Boolean);
  if (targets.length) {
    const mark = () => {
      const y = innerHeight * 0.3;
      let cur = targets[0];
      for (const t of targets) if (t.getBoundingClientRect().top <= y) cur = t;
      links.forEach((a) => a.classList.toggle("on", a.hash === "#" + cur.id));
    };
    addEventListener("scroll", mark, { passive: true });
    addEventListener("resize", mark);
    mark();
    // на телефоне оглавление свёрнуто
    const det = $(".toc details");
    if (det && matchMedia("(max-width:1000px)").matches) det.open = false;
    links.forEach((a) => a.addEventListener("click", () => { if (det && matchMedia("(max-width:1000px)").matches) det.open = false; }));
  }

  // ---- отметки «Сделано» ----
  // свой прогресс у каждого пути гайда
  const KEY = "clodhelper.guide.v2:" + location.pathname.replace(/index.html$/, "");
  const btns = $$(".mark[data-step]");
  if (btns.length) {
    const done = new Set(store.get(KEY, []));
    const paint = () => {
      btns.forEach((b) => {
        const on = done.has(b.dataset.step);
        b.setAttribute("aria-pressed", String(on));
        b.lastChild.textContent = on ? "Сделано ✓" : "Сделано";
      });
      $$(".toc a[data-step]").forEach((a) => a.classList.toggle("done", done.has(a.dataset.step)));
      const n = btns.filter((b) => done.has(b.dataset.step)).length;
      const pt = $(".prog .pt"), bar = $(".prog .bar i");
      if (pt) pt.textContent = n === btns.length ? "Все шаги пройдены — готово!" : `Пройдено ${n} из ${btns.length} шагов`;
      if (bar) bar.style.width = (n / btns.length) * 100 + "%";
    };
    btns.forEach((b) => b.addEventListener("click", () => {
      const s = b.dataset.step;
      done.has(s) ? done.delete(s) : done.add(s);
      store.set(KEY, [...done]);
      paint();
      if (done.has(s)) {
        const i = btns.indexOf(b);
        const next = btns[i + 1] && btns[i + 1].closest("section");
        if (next) setTimeout(() => next.scrollIntoView({ behavior: "smooth", block: "start" }), 250);
      }
    }));
    paint();
  }

  // ---- вопросы: открыть по якорю ----
  const openHash = () => {
    const el = location.hash && document.getElementById(decodeURIComponent(location.hash.slice(1)));
    if (el && el.tagName === "DETAILS") { el.open = true; setTimeout(() => el.scrollIntoView({ block: "start" }), 0); }
  };
  addEventListener("hashchange", openHash);
  openHash();

  // ---- «ссылка на ответ» ----
  $$(".qa[id]").forEach((d) => {
    const a = $(".a", d);
    if (!a || $(".lnk", a)) return;
    const b = document.createElement("button");
    b.type = "button";
    b.className = "lnk";
    b.innerHTML = '<svg class="i" viewBox="0 0 24 24"><path d="M10 13a5 5 0 0 0 7.54.54l3-3a5 5 0 0 0-7.07-7.07l-1.72 1.71"/><path d="M14 11a5 5 0 0 0-7.54-.54l-3 3a5 5 0 0 0 7.07 7.07l1.71-1.71"/></svg><span>Скопировать ссылку на ответ</span>';
    b.addEventListener("click", async () => {
      const url = location.href.split("#")[0] + "#" + d.id;
      try { await navigator.clipboard.writeText(url); b.lastChild.textContent = "Ссылка скопирована"; }
      catch { location.hash = d.id; b.lastChild.textContent = "Ссылка в адресной строке"; }
      setTimeout(() => (b.lastChild.textContent = "Скопировать ссылку на ответ"), 2200);
    });
    a.appendChild(b);
  });

  // ---- поиск по вопросам ----
  // Ищем по вопросу, ответу и скрытым меткам data-tags — без текста кнопки
  // «Скопировать ссылку на ответ». Кнопка с data-tag фильтрует по метке
  // (нажата — подсвечена), кнопка с data-q подставляет слово в поиск.
  const q = $("#faq-q");
  if (q) {
    const items = $$(".qa");
    const groups = $$(".fgroup");
    const empty = $("#faq-empty");
    const tagChips = $$(".chip[data-tag]");
    const norm = (s) => s.toLowerCase().replace(/ё/g, "е");
    items.forEach((d) => {
      const parts = [$("summary", d)?.textContent || "", ...$$(".a > :not(.lnk)", d).map((e) => e.textContent), d.dataset.tags || ""];
      d.dataset.text = norm(parts.join(" "));
    });
    let tag = "";
    const run = () => {
      const words = norm(q.value).split(/\s+/).filter(Boolean);
      let shown = 0;
      items.forEach((d) => {
        const okTag = !tag || (d.dataset.tags || "").split(/\s+/).includes(tag);
        const ok = okTag && words.every((w) => d.dataset.text.includes(w));
        d.hidden = !ok;
        if (ok) shown++;
        if ((words.length || tag) && ok && shown <= 3) d.open = true;
        if (!words.length && !tag) d.open = false;
      });
      groups.forEach((g) => (g.hidden = !$$(".qa", g).some((d) => !d.hidden)));
      if (empty) empty.hidden = shown > 0;
      tagChips.forEach((c) => c.setAttribute("aria-pressed", String(c.dataset.tag === tag)));
    };
    q.addEventListener("input", () => { tag = ""; run(); });
    $$(".chip").forEach((c) => c.addEventListener("click", () => {
      if (c.dataset.tag) { tag = tag === c.dataset.tag ? "" : c.dataset.tag; q.value = ""; }
      else { tag = ""; q.value = c.dataset.q || ""; q.focus(); }
      run();
    }));
  }
})();
