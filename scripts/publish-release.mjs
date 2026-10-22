#!/usr/bin/env node
// Подпись и публикация релиза Clod Helper.
//
// CI собирает установщики и манифест обновления (latest.json) и создаёт
// релиз ЧЕРНОВИКОМ. Этот скрипт на машине разработчика:
//   1. скачивает манифест и установщики черновика;
//   2. сам пересчитывает их SHA-256 и сверяет с манифестом;
//   3. подписывает манифест закрытым ключом ed25519 (он есть только здесь,
//      на GitHub его нет) → latest.json.sig;
//   4. публикует релиз (черновик → последний).
// Программа ставит обновление, только если подпись сошлась с открытым
// ключом, зашитым в неё (src-tauri/src/updater.rs, UPDATE_KEYS). Подмена
// файлов релиза (утёкший токен, взлом аккаунта, зеркало) не пройдёт.
//
//   node scripts/publish-release.mjs v0.8.9      подписать и опубликовать
//   node scripts/publish-release.mjs --keygen    создать ключ (один раз)
//
// Ключ: %APPDATA%\ClodHelperRelease\update-signing-key.pem (или путь в
// CLOD_SIGNING_KEY). ПОТЕРЯ КЛЮЧА = программа не примет ни одно новое
// обновление. Храните резервную копию (менеджер паролей, флешка).

import { createHash, createPrivateKey, createPublicKey, generateKeyPairSync, sign, verify } from "node:crypto";
import { execFileSync } from "node:child_process";
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";

const REPO = "iimperium-dev/ClodHelper";
const API = `https://api.github.com/repos/${REPO}`;
const MANIFEST = "latest.json";
const KEY_PATH =
  process.env.CLOD_SIGNING_KEY ||
  join(process.env.APPDATA || join(process.env.HOME || ".", ".config"), "ClodHelperRelease", "update-signing-key.pem");

const die = (msg) => {
  console.error(`ОШИБКА: ${msg}`);
  process.exit(1);
};

/** Открытый ключ — base64 сырых 32 байт (последние 32 байта SPKI DER). */
const rawPublic = (key) => createPublicKey(key).export({ type: "spki", format: "der" }).subarray(-32).toString("base64");

function keygen() {
  if (existsSync(KEY_PATH)) die(`ключ уже есть: ${KEY_PATH} — не перезаписываю`);
  const { privateKey } = generateKeyPairSync("ed25519");
  mkdirSync(dirname(KEY_PATH), { recursive: true });
  writeFileSync(KEY_PATH, privateKey.export({ type: "pkcs8", format: "pem" }), { mode: 0o600 });
  console.log(`Закрытый ключ: ${KEY_PATH}`);
  console.log("СДЕЛАЙТЕ РЕЗЕРВНУЮ КОПИЮ этого файла — без него обновления не выпустить.");
  console.log(`Открытый ключ (в UPDATE_KEYS): ${rawPublic(privateKey)}`);
}

/** Токен GitHub — тот же, которым git делает push (Git Credential Manager). */
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

/** Файл релиза (в т. ч. черновика) — через API, с токеном. */
async function asset(tok, a) {
  const res = await gh(tok, `/releases/assets/${a.id}`, { headers: { Accept: "application/octet-stream" } });
  return Buffer.from(await res.arrayBuffer());
}

async function publish(tag) {
  if (!/^v\d+\.\d+\.\d+$/.test(tag)) die(`тег вида v1.2.3, а не «${tag}»`);
  if (!existsSync(KEY_PATH)) die(`нет ключа ${KEY_PATH} (создать: --keygen)`);
  const key = createPrivateKey(readFileSync(KEY_PATH));
  const tok = token();

  const releases = await (await gh(tok, "/releases?per_page=100")).json();
  const rel = releases.find((r) => r.tag_name === tag);
  if (!rel) die(`релиза ${tag} нет (CI ещё собирает?)`);
  const byName = (n) => rel.assets.find((a) => a.name === n);
  const mAsset = byName(MANIFEST);
  if (!mAsset) die(`в релизе нет ${MANIFEST}`);

  const body = await asset(tok, mAsset);
  const m = JSON.parse(body.toString("utf8"));
  if (m.tag !== tag || `v${m.version}` !== tag) die(`манифест про ${m.tag}/${m.version}, а релиз ${tag}`);
  const platforms = Object.entries(m.platforms || {});
  if (!platforms.length) die("в манифесте нет ни одной платформы");

  // Подписываем только то, что сами проверили: суммы считаем здесь.
  for (const [p, f] of platforms) {
    const a = byName(f.file);
    if (!a) die(`${p}: файла ${f.file} нет в релизе`);
    const sum = createHash("sha256").update(await asset(tok, a)).digest("hex");
    if (sum !== f.sha256.toLowerCase()) die(`${p}: SHA-256 ${f.file} не совпал с манифестом`);
    console.log(`✓ ${p}: ${f.file}`);
  }

  const sig = sign(null, body, key);
  if (!verify(null, body, createPublicKey(key), sig)) die("подпись не проверилась");
  const sigText = sig.toString("base64");

  const old = byName(`${MANIFEST}.sig`);
  if (old) await gh(tok, `/releases/assets/${old.id}`, { method: "DELETE" });
  const upload = rel.upload_url.replace(/\{.*\}$/, "") + `?name=${MANIFEST}.sig`;
  await gh(tok, upload, { method: "POST", headers: { "Content-Type": "text/plain" }, body: sigText });
  console.log(`✓ подпись ${MANIFEST}.sig (ключ ${rawPublic(key).slice(0, 12)}…)`);

  await gh(tok, `/releases/${rel.id}`, {
    method: "PATCH",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ draft: false, make_latest: "true" }),
  });
  console.log(`✓ ${tag} опубликован`);
}

const arg = process.argv[2];
if (arg === "--keygen") keygen();
else if (arg) await publish(arg);
else die("укажите тег (v1.2.3) или --keygen");
