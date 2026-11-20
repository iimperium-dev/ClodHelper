#!/usr/bin/env node
// Зашифровать основной ключ подписи обновлений паролем.
//
//   node scripts/encrypt-signing-key.mjs
//
// Пароль вводит владелец сам (скрытый ввод, дважды). Рядом с ключом
// появляется зашифрованный файл update-signing-key.enc.json (scrypt +
// AES-256-GCM). Исходный update-signing-key.pem НЕ удаляется: сначала
// проверьте, что `node scripts/publish-release.mjs` работает с паролем
// (пароль можно передать и переменной CLOD_SIGN_PASS), и только потом
// удалите открытый PEM вручную. Пароль не восстановить: потеряете его и
// копию PEM - обновления не выпустить (для этого есть резервный ключ).
//
// publish-release.mjs, если рядом есть .enc.json, берёт его, а не PEM.

import { createPrivateKey } from "node:crypto";
import { existsSync, readFileSync, writeFileSync } from "node:fs";
import { ENC_PATH, KEY_PATH, decryptPem, encryptPem, rawPublic } from "./lib/keys.mjs";
import { askHidden } from "./lib/prompt.mjs";

const die = (msg) => {
  console.error(`ОШИБКА: ${msg}`);
  process.exit(1);
};

if (!existsSync(KEY_PATH)) die(`нет ключа ${KEY_PATH}`);
if (existsSync(ENC_PATH)) die(`зашифрованный файл уже есть: ${ENC_PATH} - не перезаписываю`);
const pem = readFileSync(KEY_PATH, "utf8");
if (/ENCRYPTED/.test(pem)) die("ключ уже зашифрован");
const key = createPrivateKey(pem);

const p1 = await askHidden("Новый пароль (не короче 10 символов): ").catch((e) => die(e.message));
if (p1.length < 10) die("пароль короче 10 символов");
const p2 = await askHidden("Повторите пароль: ").catch((e) => die(e.message));
if (p1 !== p2) die("пароли не совпали");

const text = encryptPem(pem, p1);
// Проверяем сразу: расшифровывается и даёт тот же ключ.
if (rawPublic(createPrivateKey(decryptPem(text, p1))) !== rawPublic(key)) die("проверка расшифровки не прошла, файл не записан");
writeFileSync(ENC_PATH, text, { mode: 0o600, flag: "wx" });

console.log(`Записано: ${ENC_PATH}`);
console.log("Проверка расшифровки прошла, открытый ключ совпал.");
console.log(`ИСХОДНЫЙ ФАЙЛ НЕ УДАЛЁН: ${KEY_PATH}`);
console.log("Проверьте публикацию с паролем, затем удалите исходный PEM вручную.");
