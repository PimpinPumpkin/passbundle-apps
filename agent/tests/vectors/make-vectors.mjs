// Makes the native-messaging test vectors: fixed inputs and the bytes they
// must produce, so upstream's TypeScript and the agent's Rust are checked
// against the same values. Run once; the output is committed.
import crypto from "node:crypto";

const out = process.argv[2];
const b64 = (b) => Buffer.from(b).toString("base64");
const bytes = (n, start) => Buffer.from(Array.from({ length: n }, (_, i) => (start + i) & 0xff));

// The 64-byte session key: AES-256 key, then HMAC-SHA256 key.
const sessionKey = bytes(64, 0x00);
const encKey = sessionKey.subarray(0, 32);
const macKey = sessionKey.subarray(32);

function encString(plaintext, iv) {
  const cipher = crypto.createCipheriv("aes-256-cbc", encKey, iv);
  const ct = Buffer.concat([cipher.update(Buffer.from(plaintext, "utf8")), cipher.final()]);
  const mac = crypto.createHmac("sha256", macKey).update(Buffer.concat([iv, ct])).digest();
  return `2.${b64(iv)}|${b64(ct)}|${b64(mac)}`;
}

const messages = [
  {
    // A request, as the extension sends it.
    iv: bytes(16, 0xa0),
    plaintext: JSON.stringify({
      command: "unlockWithBiometricsForUser",
      messageId: 7,
      userId: "3f1c2a64-8d0e-4b7f-9a55-6c2e1d0b9f10",
      timestamp: 1760000000000,
    }),
  },
  {
    // A reply with a user key, as the agent sends it.
    iv: bytes(16, 0xb0),
    plaintext: JSON.stringify({
      command: "unlockWithBiometricsForUser",
      messageId: 7,
      response: true,
      timestamp: 1760000000500,
      userKeyB64: b64(bytes(64, 0x40)),
    }),
  },
  {
    // Exactly two AES blocks, so padding adds a whole block.
    iv: bytes(16, 0xc0),
    plaintext: "0123456789abcdef0123456789abcdef",
  },
  {
    // Non-ASCII text: UTF-8 on both sides.
    iv: bytes(16, 0xd0),
    plaintext: "Ünïcödé ✓ 鍵",
  },
].map((m) => ({ iv: b64(m.iv), plaintext: m.plaintext, encString: encString(m.plaintext, m.iv) }));

// A fixed RSA-2048 key pair standing in for the extension's, and the session
// key wrapped to it with RSA-OAEP (SHA-1), as upstream's desktop app does.
const { publicKey, privateKey } = crypto.generateKeyPairSync("rsa", { modulusLength: 2048 });
const spki = publicKey.export({ type: "spki", format: "der" });
const pkcs8 = privateKey.export({ type: "pkcs8", format: "der" });
const wrapped = crypto.publicEncrypt(
  { key: publicKey, padding: crypto.constants.RSA_PKCS1_OAEP_PADDING, oaepHash: "sha1" },
  sessionKey,
);

const vectors = {
  about:
    "Test vectors for the native-messaging channel between the extension and the desktop agent. TEST KEYS ONLY. See DESIGN.md, \"Biometric unlock\".",
  sessionKey: b64(sessionKey),
  messages,
  rsa: { publicKeySpki: b64(spki), privateKeyPkcs8: b64(pkcs8), wrappedSessionKey: b64(wrapped) },
};
const fs = await import("node:fs");
fs.writeFileSync(out, JSON.stringify(vectors, null, 2) + "\n");
console.log(`wrote ${out}`);
