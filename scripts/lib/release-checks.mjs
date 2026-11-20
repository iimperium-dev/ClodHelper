// Сверка файлов черновика релиза с тем, что собрал CI (чистые функции:
// проверяются на `node --test scripts/`). Сеть и подпись - в publish-release.mjs.

export const WORKFLOW_PATH = ".github/workflows/release.yml";

/** GitHub заменяет пробелы в именах ассетов точками. */
export const assetName = (n) => n.replaceAll(" ", ".");

const isInstaller = (n) => /\.(exe|dmg)$/i.test(n);
const SHA = /^[0-9a-f]{64}$/;

/** Текст в формате sha256sum ("<sha>  <имя>") -> Map имя(как у ассета) -> sha. */
export function parseSums(text) {
  const m = new Map();
  for (const line of String(text).split(/\r?\n/)) {
    if (!line.trim()) continue;
    const mt = /^([0-9a-fA-F]{64}) [ *](.+)$/.exec(line.trim());
    if (!mt) throw new Error(`строка сумм не разобрана: «${line.slice(0, 80)}»`);
    m.set(assetName(mt[2].trim().replace(/^.*[\\/]/, "")), mt[1].toLowerCase());
  }
  return m;
}

/** Один и тот же коммит (полные sha, без учёта регистра). */
export const sameCommit = (a, b) => /^[0-9a-f]{40}$/i.test(String(a)) && String(a).toLowerCase() === String(b).toLowerCase();

/** Нужный запуск release.yml для коммита тега: успешный, тот же коммит и путь. */
export function pickRun(runs, { tag, sha, repo }) {
  const mine = (runs ?? []).filter(
    (r) =>
      r &&
      r.head_sha === sha &&
      typeof r.path === "string" &&
      (r.path === WORKFLOW_PATH || r.path.startsWith(`${WORKFLOW_PATH}@`)) &&
      r.event === "push" &&
      (!r.head_branch || r.head_branch === tag) &&
      (!r.head_repository || !repo || r.head_repository.full_name === repo),
  );
  if (!mine.length) throw new Error(`нет запуска ${WORKFLOW_PATH} для коммита ${String(sha).slice(0, 10)} (тег ${tag})`);
  const ok = mine.filter((r) => r.status === "completed" && r.conclusion === "success");
  if (!ok.length) {
    const r = mine[0];
    throw new Error(`запуск CI #${r.id} не успешен: статус ${r.status}, итог ${r.conclusion ?? "ещё идёт"}`);
  }
  return ok.sort((a, b) => String(b.created_at).localeCompare(String(a.created_at)))[0];
}

/**
 * built    - Map имя -> sha из артефактов CI (то, что собрано);
 * assets   - Map имя -> sha установщиков, лежащих в черновике (посчитаны здесь);
 * manifest - latest.json черновика;
 * sumsTxt  - необязательно: текст SHA256SUMS.txt из черновика (для старых версий).
 * Возвращает строки отчёта; при любом расхождении - исключение.
 */
export function verifySums({ built, assets, manifest, sumsTxt }) {
  const report = [];
  const platforms = Object.entries(manifest?.platforms ?? {});
  if (!platforms.length) throw new Error("в манифесте нет ни одной платформы");
  for (const [p, f] of platforms) {
    const name = f.file;
    const want = String(f.sha256 ?? "").toLowerCase();
    if (!SHA.test(want)) throw new Error(`${p}: в манифесте не сумма SHA-256`);
    // Платформа и тип установщика должны сходиться: windows - .exe, darwin - .dmg.
    const ext = p.startsWith("windows-") ? /\.exe$/i : p.startsWith("darwin-") ? /\.dmg$/i : null;
    if (!ext) throw new Error(`${p}: неизвестная платформа в манифесте`);
    if (!ext.test(String(name))) throw new Error(`${p}: файл ${name} не того типа (нужен ${p.startsWith("windows-") ? ".exe" : ".dmg"})`);
    const b = built.get(name);
    if (!b) throw new Error(`${p}: файла ${name} нет среди собранных CI (суммы сборки)`);
    const a = assets.get(name);
    if (!a) throw new Error(`${p}: файла ${name} нет в черновике`);
    if (b !== a) throw new Error(`${p}: ${name} в черновике НЕ тот, что собрал CI (сборка ${b.slice(0, 12)}, черновик ${a.slice(0, 12)})`);
    if (b !== want) throw new Error(`${p}: SHA-256 в манифесте не совпал с собранным CI (${name})`);
    report.push(`${p}: ${name} совпал с CI (${b.slice(0, 12)})`);
  }
  // Лишних установщиков в черновике быть не должно.
  const listed = new Set(platforms.map(([, f]) => f.file));
  for (const name of assets.keys()) {
    if (isInstaller(name) && !listed.has(name)) throw new Error(`в черновике лишний установщик ${name}, его нет в манифесте`);
  }
  if (sumsTxt !== undefined) {
    for (const [name, sum] of parseSums(sumsTxt)) {
      if (!isInstaller(name)) continue;
      const b = built.get(name);
      if (!b || b !== sum) throw new Error(`SHA256SUMS.txt: сумма ${name} не совпала с собранным CI`);
    }
    report.push("SHA256SUMS.txt совпал с CI");
  }
  return report;
}

/** Из списка артефактов запуска - нужные файлы сумм (имя -> артефакт). */
export function sumsArtifacts(list) {
  const out = new Map();
  for (const a of list ?? []) {
    if (/^build-sums-(windows|macos)$/.test(a.name) && !a.expired) out.set(a.name, a);
  }
  return out;
}
