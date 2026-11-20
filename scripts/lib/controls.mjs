// Управляющие поля подписанного манифеста обновления (latest.json).
// Все необязательные; программа (updater.rs) разбирает их так же:
//   rollout     0-100, по умолчанию 100: доля установок, которым предлагается
//               версия (у каждой установки свой случайный номер 0-99, он
//               никуда не отправляется);
//   paused      true: версию никто не ставит и не получает о ней уведомлений;
//   min_version установки старше этой версии не ставят обновление сами, им
//               показывается ручная установка со ссылкой на страницу релиза;
//   force       true: срочное исправление, ставить, даже если в «Что нового»
//               для этой ОС пусто.
// Значение по умолчанию в манифест не пишется.

const VER = /^\d+\.\d+\.\d+$/;

/** Управляющие поля манифеста (всё, что можно менять без пересборки). */
export const CONTROL_KEYS = ["rollout", "paused", "min_version", "force"];

/** Манифест без управляющих полей: из черновика они не берутся, только из флагов. */
export function stripControls(manifest) {
  const out = { ...manifest };
  for (const k of CONTROL_KEYS) delete out[k];
  return out;
}

/** Подписанное поле `seq`: время подписи в секундах, всегда больше прежнего. */
export function withSeq(manifest, nowMs = Date.now()) {
  const prev = Number.isInteger(manifest.seq) ? manifest.seq : 0;
  return { ...manifest, seq: Math.max(Math.floor(nowMs / 1000), prev + 1) };
}

/** Различаются ли манифесты только управляющими полями и `seq`. */
export function onlyControlsChanged(a, b) {
  const skip = new Set([...CONTROL_KEYS, "seq"]);
  const rest = (m) => JSON.stringify(Object.entries(m).filter(([k]) => !skip.has(k)).sort(([x], [y]) => (x < y ? -1 : 1)));
  return rest(a) === rest(b);
}

/** Применить параметры к манифесту; вернуть новый объект. undefined - не трогать. */
export function applyControls(manifest, o = {}) {
  const out = { ...manifest };
  if (o.rollout !== undefined) {
    const n = Number(o.rollout);
    if (o.rollout === "" || !Number.isInteger(n) || n < 0 || n > 100) {
      throw new Error(`rollout: целое 0-100, а не «${o.rollout}»`);
    }
    if (n === 100) delete out.rollout;
    else out.rollout = n;
  }
  for (const [key, flag] of [
    ["paused", o.paused],
    ["force", o.force],
  ]) {
    if (flag === undefined) continue;
    if (flag) out[key] = true;
    else delete out[key];
  }
  if (o.minVersion !== undefined) {
    if (o.minVersion === "none") delete out.min_version;
    else if (VER.test(o.minVersion)) out.min_version = o.minVersion;
    else throw new Error(`min-version: вид 1.2.3 или none, а не «${o.minVersion}»`);
  }
  return out;
}

/** Разбор аргументов: позиционные (первый - тег) и флаги управления. */
export function parseArgs(argv) {
  const opts = {};
  const positional = [];
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    if (a === "--rollout" || a === "--min-version") {
      const v = argv[++i];
      if (v === undefined || v.startsWith("--")) throw new Error(`у флага ${a} нет значения`);
      if (a === "--rollout") opts.rollout = v;
      else opts.minVersion = v;
    } else if (a === "--paused") opts.paused = true;
    else if (a === "--no-paused") opts.paused = false;
    else if (a === "--controls-only") opts.controlsOnly = true;
    else if (a === "--force") opts.force = true;
    else if (a === "--no-force") opts.force = false;
    else if (a.startsWith("--")) throw new Error(`неизвестный флаг ${a}`);
    else positional.push(a);
  }
  return { positional, tag: positional[0], opts };
}

export const hasControls = (opts) => ["rollout", "paused", "force", "minVersion"].some((k) => opts[k] !== undefined);
