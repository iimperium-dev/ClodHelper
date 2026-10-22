#!/usr/bin/env node
// Манифест обновления latest.json для релиза (запускает CI, release.yml).
//
//   node scripts/make-manifest.mjs <папка-с-файлами> <тег> <whats-new.json>
//
// Программа (≥0.8.9) проверяет обновления по нему, а ставит — только если
// манифест подписан разработчиком (scripts/publish-release.mjs).

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

const platforms = {};
const win = entry("_x64-setup.exe");
const mac = entry("_universal.dmg");
if (win) platforms["windows-x86_64"] = win;
if (mac) platforms["darwin-universal"] = mac;
if (!Object.keys(platforms).length) {
  console.error("нет ни одного установщика");
  process.exit(1);
}

const notes = JSON.parse(readFileSync(whatsNew, "utf8"));
const manifest = {
  version,
  tag,
  pub_date: new Date().toISOString(),
  highlights: notes[version]?.items ?? [],
  platforms,
};
writeFileSync(join(dir, "latest.json"), JSON.stringify(manifest, null, 2) + "\n");
console.log(JSON.stringify(manifest, null, 2));
