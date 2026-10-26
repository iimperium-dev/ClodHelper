// Описание релиза на GitHub: шапка с кнопками скачивания, «Что нового»,
// инструкции первой установки под спойлерами.
// Запуск: node scripts/release-notes.mjs vX.Y.Z src/whats-new.json > release-notes.md
//
// ВАЖНО: старые версии программы (≤0.8.8) берут пункты из этого описания —
// всё, что начинается с «- » после заголовка «Что нового» (updater::highlights,
// ** и ` они вырезают). Поэтому: заголовок «## Что нового» и пункты «- »
// оставлять как есть; ниже секции — никаких строк, начинающихся с «- »;
// в пунктах — обычный текст без ссылок и HTML.
import { readFileSync } from "node:fs";

const [tag, whatsNew] = process.argv.slice(2);
if (!/^v\d+\.\d+\.\d+$/.test(tag || "") || !whatsNew) {
  console.error("usage: release-notes.mjs vX.Y.Z <whats-new.json>");
  process.exit(1);
}
const version = tag.slice(1);
const REPO = "iimperium-dev/ClodHelper";
const raw = `https://raw.githubusercontent.com/${REPO}/main/.github/assets`;
const dl = (file) => `https://github.com/${REPO}/releases/download/${tag}/${file}`;
const exe = dl(`Clod.Helper_${version}_x64-setup.exe`);
const dmg = dl(`Clod.Helper_${version}_universal.dmg`);
const badge = (os, logo) =>
  `https://img.shields.io/badge/${encodeURIComponent(os)}-${encodeURIComponent("скачать")}-34d399?style=for-the-badge&logo=${logo}&logoColor=white&labelColor=0b0f0d`;

const notes = JSON.parse(readFileSync(whatsNew, "utf8"));
const items = notes[version]?.items ?? [];
// Пункт только для одной системы — значок впереди (в окне старой версии он тоже виден).
const mark = (p) => (p.length === 1 ? (p[0] === "mac" ? "🍏 " : "🪟 ") : "");
const lines = items.map((i) => (typeof i === "string" ? `- ${i}` : `- ${mark(i.platforms ?? [])}${i.text}`));
const onlyWin = items.some((i) => typeof i === "object" && (i.platforms ?? []).length === 1 && i.platforms[0] === "windows");
const onlyMac = items.some((i) => typeof i === "object" && (i.platforms ?? []).length === 1 && i.platforms[0] === "mac");

const out = [];
out.push(`<div align="center">`);
out.push(``);
out.push(`<img src="${raw}/logo.png" width="72" alt="Clod Helper" />`);
out.push(``);
out.push(`### Clod Helper ${version}`);
out.push(``);
out.push(`<a href="${exe}"><img src="${badge("Windows", "windows")}" alt="Скачать для Windows" /></a>`);
out.push(`<a href="${dmg}"><img src="${badge("macOS", "apple")}" alt="Скачать для macOS" /></a>`);
out.push(``);
out.push(`<sub>Clod Helper уже установлен? Обновление придёт само — нажмите «Обновить» в уведомлении программы.</sub>`);
out.push(``);
out.push(`</div>`);
out.push(``);
if (lines.length) {
  out.push(`## Что нового`);
  out.push(``);
  out.push(...lines);
  out.push(``);
  if (onlyWin || onlyMac) {
    const legend = [onlyWin && "🪟 — только Windows", onlyMac && "🍏 — только macOS"].filter(Boolean).join(" · ");
    out.push(`<sub>${legend}</sub>`);
    out.push(``);
  }
}
out.push(`<details>`);
out.push(`<summary><b>Первая установка на Windows 10 / 11</b></summary>`);
out.push(``);
out.push(`1. Скачайте установщик кнопкой выше и запустите его.`);
out.push(`2. Если появится синее окно «Система Windows защитила ваш компьютер» — нажмите **Подробнее → Выполнить в любом случае**.`);
out.push(`3. Если файл не запускается — ПКМ по нему → **Свойства** → галочка **«Разблокировать»** → **ОК**.`);
out.push(``);
out.push(`Программа не подписана платным сертификатом Microsoft, поэтому Windows осторожничает. Исходный код открыт в этом репозитории.`);
out.push(``);
out.push(`</details>`);
out.push(``);
out.push(`<details>`);
out.push(`<summary><b>Первая установка на macOS 11+ (Apple Silicon и Intel)</b></summary>`);
out.push(``);
out.push(`1. Скачайте образ кнопкой выше, откройте его и перетащите Clod Helper в «Программы».`);
out.push(`2. Первый запуск: ПКМ по приложению → **Открыть** → **Открыть**.`);
out.push(`3. На macOS 15 и новее, если кнопки «Открыть» нет: **Системные настройки → Конфиденциальность и безопасность → «Всё равно открыть»**.`);
out.push(``);
out.push(`Приложение не заверено Apple, поэтому macOS предупреждает. Исходный код открыт в этом репозитории.`);
out.push(``);
out.push(`</details>`);
console.log(out.join("\n"));
