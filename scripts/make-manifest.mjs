#!/usr/bin/env node
// Манифест обновления latest.json для релиза (запускает CI, release.yml).
//
//   node scripts/make-manifest.mjs <папка-с-файлами> <тег> <whats-new.json>
//
// Программа (≥0.8.9) проверяет обновления по нему, а ставит — только если
// манифест подписан разработчиком (scripts/publish-release.mjs).
//
// Пункты «Что нового» (src/whats-new.json) — строка (для всех) или
// { "text": "…", "platforms": ["mac"] | ["windows"] }. У каждой ОС в
// манифесте свой список: пустой — в релизе для неё ничего нового, и
// программа на этой ОС (≥0.8.11) обновление не предлагает.

import { createHash } from "node:crypto";
import { readdirSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";

const [dir, tag, whatsNew] = process.argv.slice(2);
if (!dir || !/^v\d+\.\d+\.\d+$/.test(tag ?? "") || !whatsNew) {
  console.error("usage: make-manifest.mjs <dir> <vX.Y.Z> <whats-new.json>");
  process.exit(1);
}
const version = tag.slice(1);
const files = readdirSync(dir);

/** Установщик платформы: имя — как его назовёт GitHub (пробелы → точки). */
function entry(suffix) {
  const name = files.find((f) => f.endsWith(suffix));
  if (!name) return null;
  const sha256 = createHash("sha256").update(readFileSync(join(dir, name))).digest("hex");
  return { file: name.replaceAll(" ", "."), sha256 };
}

const notes = JSON.parse(readFileSync(whatsNew, "utf8"));
const items = notes[version]?.items ?? [];
/** Пункты для ОС ("windows" | "mac"): строки — всем, объекты — своим. */
const itemsFor = (os) =>
  items
    .filter((i) => typeof i === "string" || (i.platforms ?? []).includes(os))
    .map((i) => (typeof i === "string" ? i : i.text));

const platforms = {};
const win = entry("_x64-setup.exe");
const mac = entry("_universal.dmg");
if (win) platforms["windows-x86_64"] = { ...win, highlights: itemsFor("windows") };
if (mac) platforms["darwin-universal"] = { ...mac, highlights: itemsFor("mac") };
if (!Object.keys(platforms).length) {
  console.error("нет ни одного установщика");
  process.exit(1);
}

// Общий список — для версий до 0.8.11, которые не знают пунктов по ОС.
const manifest = {
  version,
  tag,
  pub_date: new Date().toISOString(),
  highlights: items.map((i) => (typeof i === "string" ? i : i.text)),
  platforms,
};
writeFileSync(join(dir, "latest.json"), JSON.stringify(manifest, null, 2) + "\n");
console.log(JSON.stringify(manifest, null, 2));
