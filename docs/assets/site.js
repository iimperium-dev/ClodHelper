// Общая шапка портала: меню на телефоне.
(() => {
  const sh = document.querySelector(".sh");
  if (!sh) return;
  const btn = sh.querySelector(".sh-burger");
  const set = (on) => {
    sh.classList.toggle("open", on);
    btn.setAttribute("aria-expanded", String(on));
  };
  btn.addEventListener("click", () => set(!sh.classList.contains("open")));
  sh.querySelectorAll(".sh-a").forEach((a) => a.addEventListener("click", () => set(false)));
  document.addEventListener("keydown", (e) => { if (e.key === "Escape") set(false); });
  document.addEventListener("click", (e) => { if (!sh.contains(e.target)) set(false); });
  matchMedia("(min-width:921px)").addEventListener("change", (m) => { if (m.matches) set(false); });
})();
