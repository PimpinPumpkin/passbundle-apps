// Makes the Windows Hello sealing test vector: a fixed challenge, a fixed
// stand-in for the Windows Hello signature, a fixed user key, and a fixed
// nonce, and the sealed bytes they must give. Plain Node crypto, as an
// independent check on the agent's Rust (src/sealed.rs). Run once; the output
// is committed. TEST VALUES ONLY.
import crypto from "node:crypto";
import fs from "node:fs";

const b64 = (b) => Buffer.from(b).toString("base64");
const bytes = (n, start) => Buffer.from(Array.from({ length: n }, (_, i) => (start + i * 7) & 0xff));

const INFO = "passbundle-agent windows-hello seal v1";
const userId = "3f1c2a64-8d0e-4b7f-9a55-6c2e1d0b9f10";
const challenge = bytes(32, 0x11);
// An RSA-2048 PKCS#1 v1.5 signature is 256 bytes.
const signature = bytes(256, 0x22);
const userKey = bytes(64, 0x33);
const nonce = bytes(12, 0x44);

// HKDF-SHA256: the challenge as salt, the signature as input key material.
const key = Buffer.from(crypto.hkdfSync("sha256", signature, challenge, INFO, 32));
const cipher = crypto.createCipheriv("aes-256-gcm", key, nonce);
cipher.setAAD(Buffer.from(userId, "utf8"));
const ciphertext = Buffer.concat([cipher.update(userKey), cipher.final(), cipher.getAuthTag()]);

fs.writeFileSync(
  process.argv[2],
  JSON.stringify(
    {
      about: "Test vector for sealing a user key with a key derived from a Windows Hello signature. TEST VALUES ONLY. See DESIGN.md, \"Biometric unlock\".",
      info: INFO,
      userId,
      challenge: b64(challenge),
      signature: b64(signature),
      userKey: b64(userKey),
      nonce: b64(nonce),
      ciphertext: b64(ciphertext),
    },
    null,
    2,
  ) + "\n",
);
