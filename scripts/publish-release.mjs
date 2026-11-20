#!/usr/bin/env node
// Подпись и публикация релиза Clod Helper.
//
// CI собирает установщики и манифест обновления (latest.json) и создаёт
// релиз ЧЕРНОВИКОМ. Этот скрипт на машине разработчика:
//   1. находит успешный запуск release.yml для коммита тега (путь workflow и
//      head_sha проверяются) и берёт из него суммы SHA-256 установщиков,
//      посчитанные в самой сборке (артефакты build-sums-*);
//   2. скачивает манифест и установщики черновика, считает их SHA-256 и
//      сверяет трижды: сборка CI, файл в черновике, манифест. Любое
//      расхождение - остановка: подписывается только то, что собрал CI;
//   3. подписывает манифест закрытым ключом ed25519 (он есть только здесь,
//      на GitHub его нет) -> latest.json.sig;
//   4. публикует релиз (черновик -> последний).
// Программа ставит обновление, только если подпись сошлась с открытым
// ключом, зашитым в неё (src-tauri/src/updater.rs, UPDATE_KEYS). Подмена
// файлов релиза (утёкший токен, взлом аккаунта, зеркало) не пройдёт.
//
//   node scripts/publish-release.mjs v0.8.9      подписать и опубликовать
//   node scripts/publish-release.mjs --keygen    создать ключ (один раз)
//
// Управление выкладкой (попадает в подписанный манифест):
//   --rollout N        показывать версию только N% установок (0-100)
//   --paused           остановить: никто не ставит и не получает уведомлений
//   --no-paused        снять остановку
//   --force            срочное исправление: ставить, даже если в «Что нового»
//                      для ОС пусто (--no-force - снять)
//   --min-version X    установки старше X не ставят сами, им показывается
//                      ручная установка (none - снять)
// Флаги можно применить и к уже опубликованному релизу, например
// `node scripts/publish-release.mjs v0.9.0 --paused`: манифест подписывается
// заново, поля без флага остаются как были.
// Аварийно остановить все установки сразу: удалить у последнего релиза
// ассет latest.json.sig (программа без подписи ничего не ставит).
//
// Ключ: %APPDATA%\ClodHelperRelease\update-signing-key.pem (или путь в
// CLOD_SIGNING_KEY), либо зашифрованный update-signing-key.enc.json рядом
// (scripts/encrypt-signing-key.mjs): тогда пароль берётся из CLOD_SIGN_PASS
// или спрашивается скрытым вводом. ПОТЕРЯ КЛЮЧА = программа не примет ни одно
// новое обновление. Резервный ключ (закрытая часть офлайн у владельца) уже
// прописан в UPDATE_KEYS.

import { createHash, createPublicKey, generateKeyPairSync, sign, verify } from "node:crypto";
import { execFileSync } from "node:child_process";
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { applyControls, hasControls, parseArgs } from "./lib/controls.mjs";
import { KEY_PATH, loadSigningKey, rawPublic } from "./lib/keys.mjs";
import { askHidden } from "./lib/prompt.mjs";
import { assetName, parseSums, pickRun, sumsArtifacts, verifySums } from "./lib/release-checks.mjs";
import { readZip } from "./lib/zip-lite.mjs";

const REPO = "iimperium-dev/ClodHelper";
const API = `https://api.github.com/repos/${REPO}`;
const MANIFEST = "latest.json";

const die = (msg) => {
  console.error(`ОШИБКА: ${msg}`);
  process.exit(1);
};

function keygen() {
  if (existsSync(KEY_PATH)) die(`ключ уже есть: ${KEY_PATH} - не перезаписываю`);
  const { privateKey } = generateKeyPairSync("ed25519");
  mkdirSync(dirname(KEY_PATH), { recursive: true });
  writeFileSync(KEY_PATH, privateKey.export({ type: "pkcs8", format: "pem" }), { mode: 0o600, flag: "wx" });
  console.log(`Закрытый ключ: ${KEY_PATH}`);
  console.log("СДЕЛАЙТЕ РЕЗЕРВНУЮ КОПИЮ этого файла - без него обновления не выпустить.");
  console.log(`Открытый ключ (в UPDATE_KEYS): ${rawPublic(privateKey)}`);
}

/** Токен GitHub - тот же, которым git делает push (Git Credential Manager). */
function token() {
  const out = execFileSync("git", ["credential", "fill"], {
    input: "protocol=https\nhost=github.com\n\n",
    encoding: "utf8",
  });
  const t = out.split("\n").find((l) => l.startsWith("password="))?.slice("password=".length).trim();
  if (!t) die("нет токена GitHub у git");
  return t;
}

async function gh(tok, path, init = {}) {
  const res = await fetch(path.startsWith("http") ? path : `${API}${path}`, {
    ...init,
    headers: {
      Authorization: `Bearer ${tok}`,
      Accept: "application/vnd.github+json",
      "User-Agent": "clod-publish-release",
      ...(init.headers || {}),
    },
  });
  if (!res.ok) die(`${init.method || "GET"} ${path}: ${res.status} ${await res.text()}`);
  return res;
}

/** Файл релиза (в т. ч. черновика) - через API, с токеном. */
async function asset(tok, a) {
  const res = await gh(tok, `/releases/assets/${a.id}`, { headers: { Accept: "application/octet-stream" } });
  return Buffer.from(await res.arrayBuffer());
}

const sha256 = (buf) => createHash("sha256").update(buf).digest("hex");

/** Коммит, на который указывает тег (аннотированный разыменовывается). */
async function tagCommit(tok, tag) {
  const ref = await (await gh(tok, `/git/ref/tags/${tag}`)).json();
  let obj = ref.object;
  for (let i = 0; i < 3 && obj.type === "tag"; i++) obj = (await (await gh(tok, `/git/tags/${obj.sha}`)).json()).object;
  if (obj.type !== "commit") die(`тег ${tag} не указывает на коммит`);
  return obj.sha;
}

/** Суммы, посчитанные самой сборкой CI (Map имя -> sha). */
async function builtSums(tok, tag) {
  const sha = await tagCommit(tok, tag);
  const runs = (await (await gh(tok, `/actions/runs?head_sha=${sha}&per_page=50`)).json()).workflow_runs;
  let run;
  try {
    run = pickRun(runs, { tag, sha, repo: REPO });
  } catch (e) {
    die(e.message);
  }
  console.log(`CI: запуск #${run.id} (${run.path}) на коммите ${sha.slice(0, 10)} - успешен`);
  const list = (await (await gh(tok, `/actions/runs/${run.id}/artifacts?per_page=100`)).json()).artifacts;
  const arts = sumsArtifacts(list);
  if (!arts.has("build-sums-windows")) die(`у запуска #${run.id} нет артефакта build-sums-windows (истёк срок хранения?)`);
  const built = new Map();
  for (const [name, a] of arts) {
    const zip = readZip(Buffer.from(await (await gh(tok, a.archive_download_url)).arrayBuffer()));
    const file = [...zip.keys()].find((k) => /^sums-.*\.txt$/.test(k));
    if (!file) die(`в артефакте ${name} нет файла сумм`);
    for (const [n, s] of parseSums(zip.get(file).toString("utf8"))) built.set(n, s);
  }
  return built;
}

/** Ключ подписи; его открытая часть обязана быть в UPDATE_KEYS программы. */
async function signingKey() {
  const key = await loadSigningKey(askHidden).catch((e) => die(e.message));
  const here = dirname(fileURLToPath(import.meta.url));
  const src = readFileSync(join(here, "..", "src-tauri", "src", "updater.rs"), "utf8");
  const list = /UPDATE_KEYS:\s*&\[&str\]\s*=\s*&\[([\s\S]*?)\];/.exec(src)?.[1] ?? "";
  const keys = [...list.matchAll(/"([A-Za-z0-9+/=]{44})"/g)].map((m) => m[1]);
  if (!keys.includes(rawPublic(key))) die("открытая часть этого ключа не прописана в UPDATE_KEYS (updater.rs): программа подпись не примет");
  return key;
}

async function publish(tag, opts) {
  if (!/^v\d+\.\d+\.\d+$/.test(tag)) die(`тег вида v1.2.3, а не «${tag}»`);
  const key = await signingKey();
  const tok = token();

  const releases = await (await gh(tok, "/releases?per_page=100")).json();
  const rel = releases.find((r) => r.tag_name === tag);
  if (!rel) die(`релиза ${tag} нет (CI ещё собирает?)`);
  const byName = (n) => rel.assets.find((a) => a.name === n);
  const mAsset = byName(MANIFEST);
  if (!mAsset) die(`в релизе нет ${MANIFEST}`);
  if (!rel.draft && !hasControls(opts)) die(`${tag} уже опубликован; переподписать можно только с флагами управления (--rollout, --paused ...)`);

  const body = await asset(tok, mAsset);
  const m = JSON.parse(body.toString("utf8"));
  if (m.tag !== tag || `v${m.version}` !== tag) die(`манифест про ${m.tag}/${m.version}, а релиз ${tag}`);

  // Подписываем только то, что собрал CI: суммы сборки, черновика и манифеста
  // должны совпасть.
  const built = await builtSums(tok, tag);
  const assets = new Map();
  for (const a of rel.assets) {
    if (/\.(exe|dmg)$/i.test(a.name)) assets.set(a.name, sha256(await asset(tok, a)));
  }
  const sumsAsset = byName("SHA256SUMS.txt");
  const sumsTxt = sumsAsset ? (await asset(tok, sumsAsset)).toString("utf8") : undefined;
  try {
    for (const line of verifySums({ built, assets, manifest: m, sumsTxt })) console.log(`✓ ${line}`);
  } catch (e) {
    die(e.message);
  }

  let outManifest = m;
  try {
    outManifest = applyControls(m, opts);
  } catch (e) {
    die(e.message);
  }
  const changed = JSON.stringify(outManifest) !== JSON.stringify(m);
  const out = changed ? Buffer.from(JSON.stringify(outManifest, null, 2) + "\n") : body;

  const sig = sign(null, out, key);
  if (!verify(null, out, createPublicKey(key), sig)) die("подпись не проверилась");
  const sigText = sig.toString("base64");

  const upload = (name, type, data) =>
    gh(tok, rel.upload_url.replace(/\{.*\}$/, "") + `?name=${name}`, { method: "POST", headers: { "Content-Type": type }, body: data });
  if (changed) {
    await gh(tok, `/releases/assets/${mAsset.id}`, { method: "DELETE" });
    await upload(MANIFEST, "application/json", out);
    console.log(`✓ ${MANIFEST} обновлён: rollout=${outManifest.rollout ?? 100} paused=${!!outManifest.paused} force=${!!outManifest.force} min_version=${outManifest.min_version ?? "-"}`);
  }
  const old = byName(`${MANIFEST}.sig`);
  if (old) await gh(tok, `/releases/assets/${old.id}`, { method: "DELETE" });
  await upload(`${MANIFEST}.sig`, "text/plain", sigText);
  console.log(`✓ подпись ${MANIFEST}.sig (ключ ${rawPublic(key).slice(0, 12)}…)`);

  if (rel.draft) {
    await gh(tok, `/releases/${rel.id}`, {
      method: "PATCH",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ draft: false, make_latest: "true" }),
    });
    console.log(`✓ ${tag} опубликован`);
  } else {
    console.log(`✓ ${tag} уже был опубликован: манифест переподписан`);
  }
}

if (process.argv[2] === "--keygen") keygen();
else {
  let parsed;
  try {
    parsed = parseArgs(process.argv.slice(2));
  } catch (e) {
    die(e.message);
  }
  if (!parsed.tag || parsed.positional.length > 1) die("укажите один тег (v1.2.3) или --keygen");
  await publish(parsed.tag, parsed.opts);
}
