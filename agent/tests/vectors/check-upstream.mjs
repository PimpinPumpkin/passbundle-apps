// Checks the native-messaging test vectors with upstream's own crypto (the
// SDK's PureCrypto, which the extension uses for these exact operations):
// every EncString decrypts to its plaintext, the wrapped session key unwraps,
// and what upstream encrypts decrypts with the vectors' plain-Node reference.
import crypto from "node:crypto";
import fs from "node:fs";
import { createRequire } from "node:module";
import path from "node:path";

const [vectorsPath, clientsRoot] = process.argv.slice(2);
// createRequire wants an absolute path.
const require = createRequire(path.resolve(clientsRoot, "package.json"));
const { PureCrypto } = require("@bitwarden/sdk-internal");

const v = JSON.parse(fs.readFileSync(vectorsPath, "utf8"));
const key = Buffer.from(v.sessionKey, "base64");
let failures = 0;
const check = (ok, what) => {
  console.log(`${ok ? "ok  " : "FAIL"} ${what}`);
  if (!ok) failures++;
};

for (const m of v.messages) {
  const plain = PureCrypto.symmetric_decrypt_string(m.encString, key);
  check(plain === m.plaintext, `upstream decrypts ${JSON.stringify(m.plaintext.slice(0, 30))}`);
}

// Tampering with any part is refused.
const [head, rest] = v.messages[0].encString.split(".");
const [iv, ct, mac] = rest.split("|");
const flip = (b64) => {
  const b = Buffer.from(b64, "base64");
  b[0] ^= 1;
  return b.toString("base64");
};
for (const [name, s] of [
  ["IV", `${head}.${flip(iv)}|${ct}|${mac}`],
  ["ciphertext", `${head}.${iv}|${flip(ct)}|${mac}`],
  ["MAC", `${head}.${iv}|${ct}|${flip(mac)}`],
]) {
  let refused = false;
  try {
    PureCrypto.symmetric_decrypt_string(s, key);
  } catch {
    refused = true;
  }
  check(refused, `upstream refuses a changed ${name}`);
}

const unwrapped = PureCrypto.rsa_decrypt_data(
  Buffer.from(v.rsa.wrappedSessionKey, "base64"),
  Buffer.from(v.rsa.privateKeyPkcs8, "base64"),
);
check(Buffer.from(unwrapped).equals(key), "upstream unwraps the RSA-OAEP session key");

// The other direction: upstream's encryption, read with plain Node.
const ours = PureCrypto.symmetric_encrypt_string("from upstream", key);
const [, body] = ours.split(".");
const [oiv, oct, omac] = body.split("|").map((p) => Buffer.from(p, "base64"));
const macOk = crypto
  .createHmac("sha256", key.subarray(32))
  .update(Buffer.concat([oiv, oct]))
  .digest()
  .equals(omac);
const decipher = crypto.createDecipheriv("aes-256-cbc", key.subarray(0, 32), oiv);
const text = Buffer.concat([decipher.update(oct), decipher.final()]).toString("utf8");
check(ours.startsWith("2.") && macOk && text === "from upstream", "upstream's EncStrings are type 2, MAC over IV and ciphertext");

process.exit(failures ? 1 : 0);
