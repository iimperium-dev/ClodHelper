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
// Эти четыре поля берутся ТОЛЬКО из флагов командной строки: что лежит в
// черновике, игнорируется. Плюс подписанное поле seq (время подписи):
// программа хранит последнее принятое и отвергает меньшее (откат к старому
// манифесту невозможен).
//
// Сменить управляющие поля уже опубликованного релиза без сверки с CI:
//   node scripts/publish-release.mjs v0.9.0 --controls-only --paused
// Работает, только если у релиза есть latest.json.sig и он проверяется вашим
// ключом; меняются только эти поля и seq, остальной манифест как был, поля
// без флага остаются прежними.
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
import { applyControls, hasControls, onlyControlsChanged, parseArgs, stripControls, withSeq } from "./lib/controls.mjs";
import { KEY_PATH, loadSigningKey, rawPublic } from "./lib/keys.mjs";
import { askHidden } from "./lib/prompt.mjs";
import { WORKFLOW_PATH, parseSums, pickRun, sameCommit, sumsArtifacts, verifySums } from "./lib/release-checks.mjs";
import { readZip } from "./lib/zip-lite.mjs";

const REPO = "iimperium-dev/ClodHelper";
const API = `https://api.github.com/repos/${REPO}`;
const MANIFEST = "latest.json";
const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");

const git = (...args) => execFileSync("git", args, { cwd: ROOT, encoding: "utf8", stdio: ["ignore", "pipe", "pipe"] }).trim();

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

/**
 * Коммит тега по API GitHub должен совпасть с локальным клоном, а workflow на
 * нём - с тем release.yml, который лежит у вас в HEAD. Сверка именно с HEAD,
 * а не с файлом на коммите тега: файл на коммите тега - это то, что запустил
 * сам GitHub, и подменить его вместе с тегом проще всего; HEAD - ваша
 * просмотренная копия. Не совпало - остановка (обновите клон: git fetch
 * --tags; поменяли workflow после выпуска - выпускайте с актуального HEAD).
 */
async function checkTagLocally(tok, tag) {
  const apiSha = await tagCommit(tok, tag);
  let local;
  try {
    local = git("rev-parse", `${tag}^{commit}`);
  } catch {
    die(`тега ${tag} нет в локальном клоне: выполните git fetch --tags`);
  }
  if (!sameCommit(apiSha, local)) {
    die(`тег ${tag}: на GitHub коммит ${apiSha.slice(0, 10)}, в локальном клоне ${local.slice(0, 10)}. Тег переставили? Разберитесь, прежде чем подписывать`);
  }
  let atTag, atHead;
  try {
    atTag = git("rev-parse", `${apiSha}:${WORKFLOW_PATH}`);
    atHead = git("rev-parse", `HEAD:${WORKFLOW_PATH}`);
  } catch (e) {
    die(`не удалось сверить ${WORKFLOW_PATH}: ${e.message.split("\n")[0]}`);
  }
  if (atTag !== atHead) {
    die(`${WORKFLOW_PATH} на коммите тега не совпадает с вашим HEAD: CI собрал по чужому workflow. Проверьте git diff ${tag} HEAD -- ${WORKFLOW_PATH}`);
  }
  console.log(`✓ коммит тега совпал с локальным клоном, workflow как в HEAD`);
  return apiSha;
}

/** Суммы, посчитанные самой сборкой CI (Map имя -> sha). */
async function builtSums(tok, tag, sha) {
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
  if (opts.controlsOnly && !hasControls(opts)) die("--controls-only нужны флаги: --rollout, --paused, --force или --min-version");
  if (!rel.draft && !opts.controlsOnly && !hasControls(opts)) {
    die(`${tag} уже опубликован; сменить управляющие поля: --controls-only --paused (и т.п.)`);
  }

  const body = await asset(tok, mAsset);
  const m = JSON.parse(body.toString("utf8"));
  if (m.tag !== tag || `v${m.version}` !== tag) die(`манифест про ${m.tag}/${m.version}, а релиз ${tag}`);

  const upload = (name, type, data) =>
    gh(tok, rel.upload_url.replace(/\{.*\}$/, "") + `?name=${name}`, { method: "POST", headers: { "Content-Type": type }, body: data });
  const replaceAsset = async (name, type, data, old) => {
    if (old) await gh(tok, `/releases/assets/${old.id}`, { method: "DELETE" });
    await upload(name, type, data);
  };

  if (opts.controlsOnly) {
    // Только управляющие поля уже подписанного вами манифеста: CI не трогаем.
    const sigAsset = byName(`${MANIFEST}.sig`);
    if (!sigAsset) die(`у ${tag} нет ${MANIFEST}.sig: переподписать только управляющие поля нельзя (полный путь без --controls-only)`);
    const oldSig = Buffer.from((await asset(tok, sigAsset)).toString("utf8").trim(), "base64");
    if (!verify(null, body, createPublicKey(key), oldSig)) die(`текущая подпись ${tag} не проверяется вашим ключом: переподписать только управляющие поля нельзя`);
    let next;
    try {
      next = withSeq(applyControls(m, opts));
    } catch (e) {
      die(e.message);
    }
    if (!onlyControlsChanged(m, next)) die("внутренняя ошибка: изменилось не только управляющее поле");
    const out = Buffer.from(JSON.stringify(next, null, 2) + "\n");
    const sig = sign(null, out, key);
    if (!verify(null, out, createPublicKey(key), sig)) die("подпись не проверилась");
    await replaceAsset(MANIFEST, "application/json", out, mAsset);
    await replaceAsset(`${MANIFEST}.sig`, "text/plain", sig.toString("base64"), sigAsset);
    console.log(`✓ ${tag}: управляющие поля обновлены, seq=${next.seq}: rollout=${next.rollout ?? 100} paused=${!!next.paused} force=${!!next.force} min_version=${next.min_version ?? "-"}`);
    return;
  }

  // Подписываем только то, что собрал CI: коммит тега, суммы сборки,
  // черновика и манифеста должны совпасть.
  const commit = await checkTagLocally(tok, tag);
  const built = await builtSums(tok, tag, commit);
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

  // Управляющие поля - только из флагов (из черновика не берутся), плюс seq.
  let outManifest;
  try {
    outManifest = withSeq(applyControls(stripControls(m), opts));
  } catch (e) {
    die(e.message);
  }
  const out = Buffer.from(JSON.stringify(outManifest, null, 2) + "\n");

  const sig = sign(null, out, key);
  if (!verify(null, out, createPublicKey(key), sig)) die("подпись не проверилась");
  const sigText = sig.toString("base64");

  await replaceAsset(MANIFEST, "application/json", out, mAsset);
  console.log(`✓ ${MANIFEST} обновлён: seq=${outManifest.seq} rollout=${outManifest.rollout ?? 100} paused=${!!outManifest.paused} force=${!!outManifest.force} min_version=${outManifest.min_version ?? "-"}`);
  await replaceAsset(`${MANIFEST}.sig`, "text/plain", sigText, byName(`${MANIFEST}.sig`));
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
