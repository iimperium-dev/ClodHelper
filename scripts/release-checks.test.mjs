// node --test scripts/
import assert from "node:assert/strict";
import { createPrivateKey, generateKeyPairSync } from "node:crypto";
import { deflateRawSync } from "node:zlib";
import { test } from "node:test";
import { readFileSync } from "node:fs";
import { applyControls, hasControls, onlyControlsChanged, parseArgs, stripControls, withSeq } from "./lib/controls.mjs";
import { decryptPem, encryptPem, rawPublic } from "./lib/keys.mjs";
import { assetName, parseSums, pickRun, sameCommit, sumsArtifacts, verifySums } from "./lib/release-checks.mjs";
import { readZip } from "./lib/zip-lite.mjs";

const A = "a".repeat(64);
const B = "b".repeat(64);
const WIN = "Clod.Helper_0.9.0_x64-setup.exe";
const DMG = "Clod.Helper_0.9.0_universal.dmg";
const manifest = () => ({
  version: "0.9.0",
  tag: "v0.9.0",
  platforms: { "windows-x86_64": { file: WIN, sha256: A }, "darwin-universal": { file: DMG, sha256: B } },
});
const good = () => ({ built: new Map([[WIN, A], [DMG, B]]), assets: new Map([[WIN, A], [DMG, B]]), manifest: manifest() });

test("суммы сборки: пробелы в именах становятся точками", () => {
  const m = parseSums(`${A}  Clod Helper_0.9.0_x64-setup.exe\n${B} *Clod Helper_0.9.0_universal.dmg\n`);
  assert.equal(m.get(WIN), A);
  assert.equal(m.get(DMG), B);
  assert.equal(assetName("a b c"), "a.b.c");
  assert.throws(() => parseSums("мусор"));
});

test("сверка: всё совпало - отчёт, расхождение - остановка", () => {
  assert.equal(verifySums(good()).length, 2);
  // Подменили файл в черновике.
  const swapped = good();
  swapped.assets.set(WIN, "c".repeat(64));
  assert.throws(() => verifySums(swapped), /НЕ тот, что собрал CI/);
  // Подменили и файл, и манифест: сумма CI всё равно другая.
  const both = good();
  both.assets.set(WIN, "c".repeat(64));
  both.manifest.platforms["windows-x86_64"].sha256 = "c".repeat(64);
  assert.throws(() => verifySums(both), /НЕ тот, что собрал CI/);
  // Манифест врёт при верном файле.
  const lie = good();
  lie.manifest.platforms["windows-x86_64"].sha256 = "d".repeat(64);
  assert.throws(() => verifySums(lie), /в манифесте не совпал/);
  // Нет суммы сборки / нет файла в черновике / лишний установщик.
  const nobuilt = good();
  nobuilt.built.delete(DMG);
  assert.throws(() => verifySums(nobuilt), /нет среди собранных/);
  const noasset = good();
  noasset.assets.delete(WIN);
  assert.throws(() => verifySums(noasset), /нет в черновике/);
  const extra = good();
  extra.assets.set("Evil_setup.exe", A);
  assert.throws(() => verifySums(extra), /лишний установщик/);
  assert.throws(() => verifySums({ ...good(), manifest: { platforms: {} } }), /нет ни одной платформы/);
  // Регистр суммы в манифесте не важен.
  const upper = good();
  upper.manifest.platforms["windows-x86_64"].sha256 = A.toUpperCase();
  assert.equal(verifySums(upper).length, 2);
});

test("сверка SHA256SUMS.txt для старых версий", () => {
  const g = good();
  assert.equal(verifySums({ ...g, sumsTxt: `${A}  ${WIN}\n${B}  ${DMG}\n` }).length, 3);
  assert.throws(() => verifySums({ ...g, sumsTxt: `${B}  ${WIN}\n` }), /SHA256SUMS/);
});

test("запуск CI: коммит, путь, успех", () => {
  const run = (o = {}) => ({
    id: 1,
    head_sha: "abc",
    path: ".github/workflows/release.yml",
    event: "push",
    head_branch: "v0.9.0",
    status: "completed",
    conclusion: "success",
    created_at: "2026-11-21T10:00:00Z",
    head_repository: { full_name: "iimperium-dev/ClodHelper" },
    ...o,
  });
  const ctx = { tag: "v0.9.0", sha: "abc", repo: "iimperium-dev/ClodHelper" };
  assert.equal(pickRun([run()], ctx).id, 1);
  assert.equal(pickRun([run({ id: 2, created_at: "2026-11-21T11:00:00Z" }), run()], ctx).id, 2);
  assert.throws(() => pickRun([run({ conclusion: "failure" })], ctx), /не успешен/);
  assert.throws(() => pickRun([run({ status: "in_progress", conclusion: null })], ctx), /не успешен/);
  assert.throws(() => pickRun([run({ path: ".github/workflows/evil.yml" })], ctx), /нет запуска/);
  assert.throws(() => pickRun([run({ head_sha: "other" })], ctx), /нет запуска/);
  assert.throws(() => pickRun([run({ event: "pull_request" })], ctx), /нет запуска/);
  assert.throws(() => pickRun([run({ head_repository: { full_name: "x/y" } })], ctx), /нет запуска/);
  assert.throws(() => pickRun([], ctx), /нет запуска/);
  assert.equal(pickRun([run({ path: ".github/workflows/release.yml@refs/tags/v0.9.0" })], ctx).id, 1);
});

test("артефакты сумм: только нужные и неистёкшие", () => {
  const m = sumsArtifacts([
    { name: "build-sums-windows", expired: false },
    { name: "build-sums-macos", expired: true },
    { name: "clod-helper-windows" },
  ]);
  assert.deepEqual([...m.keys()], ["build-sums-windows"]);
});

test("zip: чтение deflate и stored по центральному каталогу", () => {
  const entry = (name, data, method) => {
    const raw = method === 8 ? deflateRawSync(data) : data;
    const nm = Buffer.from(name);
    const local = Buffer.alloc(30);
    local.writeUInt32LE(0x04034b50, 0);
    local.writeUInt16LE(method, 8);
    local.writeUInt32LE(raw.length, 18);
    local.writeUInt32LE(data.length, 22);
    local.writeUInt16LE(nm.length, 26);
    return { nm, raw, local, method, size: data.length };
  };
  const files = [entry("sums-windows.txt", Buffer.from(`${A}  ${WIN}\n`), 8), entry("b.txt", Buffer.from("hi"), 0)];
  let off = 0;
  const parts = [];
  const central = [];
  for (const f of files) {
    const c = Buffer.alloc(46);
    c.writeUInt32LE(0x02014b50, 0);
    c.writeUInt16LE(f.method, 10);
    c.writeUInt32LE(f.raw.length, 20);
    c.writeUInt32LE(f.size, 24);
    c.writeUInt16LE(f.nm.length, 28);
    c.writeUInt32LE(off, 42);
    central.push(c, f.nm);
    parts.push(f.local, f.nm, f.raw);
    off += 30 + f.nm.length + f.raw.length;
  }
  const cd = Buffer.concat(central);
  const end = Buffer.alloc(22);
  end.writeUInt32LE(0x06054b50, 0);
  end.writeUInt16LE(files.length, 10);
  end.writeUInt32LE(cd.length, 12);
  end.writeUInt32LE(off, 16);
  const zip = readZip(Buffer.concat([...parts, cd, end]));
  assert.equal(parseSums(zip.get("sums-windows.txt").toString()).get(WIN), A);
  assert.equal(zip.get("b.txt").toString(), "hi");
  assert.throws(() => readZip(Buffer.from("not a zip at all, definitely")));
});

test("управление выкладкой: проверка и значения по умолчанию", () => {
  const m = manifest();
  assert.deepEqual(applyControls(m, {}), m);
  assert.equal(applyControls(m, { rollout: "25" }).rollout, 25);
  assert.equal(applyControls(m, { rollout: "0" }).rollout, 0);
  assert.equal("rollout" in applyControls({ ...m, rollout: 10 }, { rollout: "100" }), false, "100 - значение по умолчанию");
  assert.equal(applyControls(m, { paused: true }).paused, true);
  assert.equal("paused" in applyControls({ ...m, paused: true }, { paused: false }), false);
  assert.equal(applyControls(m, { force: true }).force, true);
  assert.equal(applyControls(m, { minVersion: "0.8.0" }).min_version, "0.8.0");
  assert.equal("min_version" in applyControls({ ...m, min_version: "0.8.0" }, { minVersion: "none" }), false);
  for (const bad of ["101", "-1", "2.5", "x", ""]) assert.throws(() => applyControls(m, { rollout: bad }));
  assert.throws(() => applyControls(m, { minVersion: "v1" }));
  // Поля без флага не трогаются.
  assert.equal(applyControls({ ...m, paused: true }, { rollout: "5" }).paused, true);
  // Исходный объект не меняется.
  applyControls(m, { paused: true });
  assert.equal("paused" in m, false);
});

test("флаги командной строки", () => {
  const p = parseArgs(["v0.9.0", "--rollout", "30", "--paused", "--min-version", "0.8.5", "--force"]);
  assert.equal(p.tag, "v0.9.0");
  assert.deepEqual(p.opts, { rollout: "30", paused: true, minVersion: "0.8.5", force: true });
  assert.equal(parseArgs(["v1.0.0", "--no-paused"]).opts.paused, false);
  assert.throws(() => parseArgs(["v1.0.0", "--rollout"]), /нет значения/);
  assert.throws(() => parseArgs(["v1.0.0", "--rollout", "--paused"]), /нет значения/);
  assert.throws(() => parseArgs(["v1.0.0", "--bogus"]), /неизвестный/);
  assert.deepEqual(parseArgs(["a", "b", "c"]).positional, ["a", "b", "c"]);
});

test("шифрование ключа паролем", () => {
  const { privateKey } = generateKeyPairSync("ed25519");
  const pem = privateKey.export({ type: "pkcs8", format: "pem" });
  const fast = { N: 1024, r: 8, p: 1 };
  const text = encryptPem(pem, "correct horse battery", fast);
  assert.ok(!text.includes("PRIVATE KEY"), "в файле нет открытого PEM");
  assert.equal(rawPublic(createPrivateKey(decryptPem(text, "correct horse battery"))), rawPublic(privateKey));
  assert.throws(() => decryptPem(text, "wrong password"), /неверный пароль/);
  // Другая соль и вектор при каждом шифровании.
  assert.notEqual(text, encryptPem(pem, "correct horse battery", fast));
});

test("строка сумм из CI: имя с пробелом доезжает целиком", () => {
  // Ровно то, что печатает шаг macOS в release.yml:
  //   printf '%s  %s\n' "$h" "$(basename "$f")"
  const line = `${B}  Clod Helper_0.9.0_universal.dmg\n`;
  const built = parseSums(line);
  assert.equal(built.get(DMG), B, "имя не обрезано по пробелу");
  const g = good();
  g.built = new Map([...parseSums(`${A}  Clod Helper_0.9.0_x64-setup.exe\n`), ...built]);
  assert.equal(verifySums(g).length, 2);
  // Страж: в workflow суммы пишутся циклом с basename, а не через awk по пробелам.
  const yml = readFileSync(new URL("../.github/workflows/release.yml", import.meta.url), "utf8");
  assert.match(yml, /basename "\$f"/);
  assert.doesNotMatch(yml, /awk '\{ n=\$2/);
});

test("платформа и тип файла: windows - .exe, darwin - .dmg", () => {
  const swapped = good();
  swapped.manifest.platforms["windows-x86_64"].file = DMG;
  swapped.built.set(DMG, A);
  swapped.assets.set(DMG, A);
  assert.throws(() => verifySums(swapped), /не того типа/);
  const mac = good();
  mac.manifest.platforms["darwin-universal"].file = WIN;
  assert.throws(() => verifySums(mac), /не того типа/);
  const odd = good();
  odd.manifest.platforms["linux-x86_64"] = { file: "x.AppImage", sha256: A };
  assert.throws(() => verifySums(odd), /неизвестная платформа/);
});

test("управляющие поля: только из флагов, из черновика не берутся", () => {
  const draft = { ...manifest(), rollout: 5, paused: true, force: true, min_version: "0.1.0" };
  const out = applyControls(stripControls(draft), { rollout: "40" });
  assert.equal(out.rollout, 40);
  assert.equal(out.paused, undefined);
  assert.equal(out.force, undefined);
  assert.equal(out.min_version, undefined);
  // Без флагов - чистый манифест.
  assert.deepEqual(applyControls(stripControls(draft), {}), manifest());
});

test("seq: время подписи, всегда больше прежнего", () => {
  assert.equal(withSeq(manifest(), 1_790_000_000_500).seq, 1_790_000_000);
  assert.equal(withSeq({ ...manifest(), seq: 1_790_000_000 }, 1_790_000_000_500).seq, 1_790_000_001, "часы не ушли вперёд");
  assert.equal(withSeq({ ...manifest(), seq: 5 }, 1_790_000_000_000).seq, 1_790_000_000);
});

test("только управляющие поля: чистая проверка для --controls-only", () => {
  const m = { ...manifest(), seq: 1 };
  const next = withSeq(applyControls(m, { paused: true, rollout: "10", minVersion: "0.8.0", force: true }), 2_000_000);
  assert.ok(onlyControlsChanged(m, next));
  // Тронули сумму, платформу или версию - нельзя.
  const bad = structuredClone(next);
  bad.platforms["windows-x86_64"].sha256 = B;
  assert.ok(!onlyControlsChanged(m, bad));
  assert.ok(!onlyControlsChanged(m, { ...next, version: "0.9.1" }));
  assert.ok(!onlyControlsChanged(m, { ...next, evil: 1 }));
  // Порядок ключей не важен.
  assert.ok(onlyControlsChanged({ a: 1, b: 2 }, { b: 2, a: 1, paused: true }));
  // Флаг --controls-only сам по себе не управляющее поле.
  const p = parseArgs(["v0.9.0", "--controls-only", "--paused"]);
  assert.equal(p.opts.controlsOnly, true);
  assert.ok(hasControls(p.opts));
  assert.ok(!hasControls(parseArgs(["v0.9.0", "--controls-only"]).opts));
});

test("коммит тега: полный sha, без учёта регистра", () => {
  const a = "a".repeat(40);
  assert.ok(sameCommit(a, a.toUpperCase()));
  assert.ok(!sameCommit(a, "b".repeat(40)));
  assert.ok(!sameCommit("abc", "abc"), "короткие не сравниваем");
  assert.ok(!sameCommit(undefined, undefined));
});
