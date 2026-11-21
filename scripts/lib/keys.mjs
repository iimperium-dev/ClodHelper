// Ключ подписи обновлений: пути, шифрование паролем, загрузка.
// Содержимое ключей нигде не печатается.
import { createCipheriv, createDecipheriv, createPrivateKey, createPublicKey, randomBytes, scryptSync } from "node:crypto";
import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";

const DIR = join(process.env.APPDATA || join(process.env.HOME || ".", ".config"), "ClodHelperRelease");
export const KEY_PATH = process.env.CLOD_SIGNING_KEY || join(DIR, "update-signing-key.pem");
/** Зашифрованный основной ключ лежит рядом с открытым PEM. */
export const ENC_PATH = KEY_PATH.replace(/\.pem$/i, "") + ".enc.json";

const SCRYPT = { N: 1 << 17, r: 8, p: 1, maxmem: 512 * 1024 * 1024 };

/** Открытый ключ - base64 сырых 32 байт (последние 32 байта SPKI DER). */
export const rawPublic = (key) => createPublicKey(key).export({ type: "spki", format: "der" }).subarray(-32).toString("base64");

/** PEM -> JSON-текст: scrypt (N=2^17) + AES-256-GCM. */
export function encryptPem(pem, password, params = SCRYPT) {
  const salt = randomBytes(16);
  const iv = randomBytes(12);
  const key = scryptSync(password, salt, 32, { ...SCRYPT, ...params });
  const c = createCipheriv("aes-256-gcm", key, iv);
  const data = Buffer.concat([c.update(pem, "utf8"), c.final()]);
  return JSON.stringify(
    {
      v: 1,
      kdf: "scrypt",
      N: params.N,
      r: params.r,
      p: params.p,
      salt: salt.toString("base64"),
      iv: iv.toString("base64"),
      tag: c.getAuthTag().toString("base64"),
      data: data.toString("base64"),
    },
    null,
    2,
  );
}

/** Обратное действие; неверный пароль - исключение. */
export function decryptPem(text, password) {
  const j = JSON.parse(text);
  if (j.v !== 1 || j.kdf !== "scrypt") throw new Error("формат зашифрованного ключа не поддержан");
  const key = scryptSync(password, Buffer.from(j.salt, "base64"), 32, { N: j.N, r: j.r, p: j.p, maxmem: SCRYPT.maxmem });
  try {
    const d = createDecipheriv("aes-256-gcm", key, Buffer.from(j.iv, "base64"));
    d.setAuthTag(Buffer.from(j.tag, "base64"));
    return Buffer.concat([d.update(Buffer.from(j.data, "base64")), d.final()]).toString("utf8");
  } catch {
    throw new Error("неверный пароль или файл повреждён");
  }
}

/**
 * Закрытый ключ для подписи. Зашифрованный файл важнее открытого PEM.
 * askPass() - скрытый ввод; пароль также берётся из CLOD_SIGN_PASS.
 */
export async function loadSigningKey(askPass) {
  const encrypted = existsSync(ENC_PATH);
  if (!encrypted && !existsSync(KEY_PATH)) throw new Error(`нет ключа ${KEY_PATH} (создать: --keygen)`);
  if (!encrypted) return createPrivateKey(readFileSync(KEY_PATH));
  const pass = process.env.CLOD_SIGN_PASS || (await askPass("Пароль ключа подписи: "));
  return createPrivateKey(decryptPem(readFileSync(ENC_PATH, "utf8"), pass));
}
