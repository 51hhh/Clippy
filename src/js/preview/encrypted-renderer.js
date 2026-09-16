/**
 * preview/encrypted-renderer.js — 加密内容展示与本地解密交互
 */

import { t } from "../../i18n/i18n.js";

export function createEncryptedRenderer({ contentEl, badgeEl }) {
  /** 加密内容渲染 + 密钥输入解密 */
  function renderEncrypted(text, encType) {
    badgeEl.textContent = `ENCRYPTED · ${encType}`;
    contentEl.classList.add("preview-content--encoded");

    // 加密标识
    const lockIcon = document.createElement("div");
    lockIcon.className = "encrypted-header";
    lockIcon.textContent = t("preview.encryptedHint") || "🔒 Encrypted content detected";
    contentEl.appendChild(lockIcon);

    // PGP / ENCRYPTED-KEY 不提供解密 UI
    if (encType === "PGP" || encType === "ENCRYPTED-KEY") {
      const box = document.createElement("pre");
      box.className = "encoded-box encoded-box--muted";
      box.textContent = text;
      contentEl.appendChild(box);
      const hint = document.createElement("div");
      hint.className = "encoded-hint";
      hint.textContent = t("preview.pgpHint") || "Use gpg or ssh-keygen to decrypt";
      contentEl.appendChild(hint);
      return;
    }

    // 解密表单
    const form = document.createElement("div");
    form.className = "decrypt-form";

    // 算法选择
    const algoRow = _formRow(t("preview.algorithm") || "Algorithm");
    const algoSelect = document.createElement("select");
    algoSelect.className = "decrypt-select";
    const algos = encType === "AES-OpenSSL"
      ? ["AES-256-CBC", "AES-128-CBC"]
      : ["AES-256-CBC", "AES-128-CBC", "AES-256-GCM", "AES-128-GCM"];
    for (const a of algos) {
      const opt = document.createElement("option");
      opt.value = a;
      opt.textContent = a;
      algoSelect.appendChild(opt);
    }
    algoRow.appendChild(algoSelect);
    form.appendChild(algoRow);

    // OpenSSL PBKDF2 模式提示
    if (encType === "AES-OpenSSL") {
      const note = document.createElement("div");
      note.className = "encoded-hint";
      note.textContent = t("preview.opensslNote") || "Only supports openssl enc -pbkdf2 format";
      form.appendChild(note);
    }

    // 密码/密钥
    const keyRow = _formRow(encType === "AES-OpenSSL"
      ? (t("preview.password") || "Password")
      : (t("preview.key") || "Key (hex)"));
    const keyInput = document.createElement("input");
    keyInput.type = encType === "AES-OpenSSL" ? "password" : "text";
    keyInput.className = "decrypt-input";
    keyInput.placeholder = encType === "AES-OpenSSL" ? "passphrase" : "hex key";
    keyRow.appendChild(keyInput);
    form.appendChild(keyRow);

    // IV 输入（非 OpenSSL 格式需要）
    let ivInput = null;
    if (encType !== "AES-OpenSSL") {
      const ivRow = _formRow("IV (hex)");
      ivInput = document.createElement("input");
      ivInput.type = "text";
      ivInput.className = "decrypt-input";
      ivInput.placeholder = "initialization vector (hex)";
      ivRow.appendChild(ivInput);
      form.appendChild(ivRow);
    }

    // 解密按钮
    const btn = document.createElement("button");
    btn.className = "decrypt-btn";
    btn.textContent = t("preview.decrypt") || "🔓 Decrypt";
    form.appendChild(btn);
    contentEl.appendChild(form);

    // 密文展示
    const cipherSection = document.createElement("details");
    cipherSection.className = "encoded-section";
    const cipherSummary = document.createElement("summary");
    cipherSummary.className = "encoded-label encoded-toggle";
    cipherSummary.textContent = t("preview.ciphertext") || "Ciphertext";
    const cipherBox = document.createElement("pre");
    cipherBox.className = "encoded-box encoded-box--muted";
    cipherBox.textContent = text;
    cipherSection.append(cipherSummary, cipherBox);
    contentEl.appendChild(cipherSection);

    // 解密结果区
    const resultArea = document.createElement("div");
    resultArea.className = "decrypt-result";
    resultArea.hidden = true;
    contentEl.appendChild(resultArea);

    btn.addEventListener("click", async () => {
      btn.disabled = true;
      btn.textContent = "⏳";
      resultArea.hidden = true;
      try {
        let decrypted;
        if (encType === "AES-OpenSSL") {
          decrypted = await decryptOpenSSL(text, keyInput.value, algoSelect.value);
        } else {
          decrypted = await decryptGeneric(text, keyInput.value, ivInput?.value || "", algoSelect.value);
        }
        resultArea.hidden = false;
        resultArea.innerHTML = "";
        const label = document.createElement("div");
        label.className = "encoded-label";
        label.textContent = t("preview.decrypted") || "Decrypted";
        const box = document.createElement("pre");
        box.className = "encoded-box";
        box.textContent = decrypted;
        resultArea.append(label, box);
      } catch (err) {
        resultArea.hidden = false;
        resultArea.innerHTML = "";
        const errEl = document.createElement("div");
        errEl.className = "decrypt-error";
        errEl.textContent = `❌ ${err.message || err}`;
        resultArea.appendChild(errEl);
      } finally {
        btn.disabled = false;
        btn.textContent = t("preview.decrypt") || "🔓 Decrypt";
      }
    });
  }

  function _formRow(label) {
    const row = document.createElement("div");
    row.className = "decrypt-row";
    const lbl = document.createElement("label");
    lbl.className = "decrypt-label";
    lbl.textContent = label;
    row.appendChild(lbl);
    return row;
  }

  /** OpenSSL 格式解密：Salted__ + PBKDF2 → AES (仅支持 openssl enc -pbkdf2) */
  async function decryptOpenSSL(b64Text, password, algo) {
    const raw = Uint8Array.from(atob(b64Text), c => c.charCodeAt(0));
    // 前 8 字节 = "Salted__"，接下来 8 字节 = salt
    if (raw.length < 16) throw new Error("Invalid OpenSSL format");
    const magic = new TextDecoder().decode(raw.slice(0, 8));
    if (magic !== "Salted__") throw new Error("Missing OpenSSL Salted__ header");
    const salt = raw.slice(8, 16);
    const ciphertext = raw.slice(16);

    const keyLen = algo.includes("256") ? 32 : 16;
    const enc = new TextEncoder();
    const keyMaterial = await crypto.subtle.importKey("raw", enc.encode(password), "PBKDF2", false, ["deriveBits"]);
    const bits = await crypto.subtle.deriveBits(
      { name: "PBKDF2", salt, iterations: 10000, hash: "SHA-256" },
      keyMaterial, (keyLen + 16) * 8
    );
    const derived = new Uint8Array(bits);
    const key = derived.slice(0, keyLen);
    const iv = derived.slice(keyLen, keyLen + 16);

    const cryptoKey = await crypto.subtle.importKey("raw", key, "AES-CBC", false, ["decrypt"]);
    const decrypted = await crypto.subtle.decrypt({ name: "AES-CBC", iv }, cryptoKey, ciphertext);
    return new TextDecoder().decode(decrypted);
  }

  /** 通用 AES 解密：用户提供 hex key + hex IV */
  async function decryptGeneric(b64Text, hexKey, hexIV, algo) {
    const ciphertext = Uint8Array.from(atob(b64Text), c => c.charCodeAt(0));
    const key = hexToBytes(hexKey);
    const iv = hexToBytes(hexIV);

    const isGCM = algo.includes("GCM");
    const algoName = isGCM ? "AES-GCM" : "AES-CBC";
    const cryptoKey = await crypto.subtle.importKey("raw", key, algoName, false, ["decrypt"]);
    const params = isGCM ? { name: "AES-GCM", iv } : { name: "AES-CBC", iv };
    const decrypted = await crypto.subtle.decrypt(params, cryptoKey, ciphertext);
    return new TextDecoder().decode(decrypted);
  }

  function hexToBytes(hex) {
    const clean = hex.replace(/\s/g, "");
    if (clean.length === 0 || clean.length % 2 !== 0) throw new Error("Invalid hex length");
    if (!/^[0-9a-f]+$/i.test(clean)) throw new Error("Invalid hex characters");
    const bytes = new Uint8Array(clean.length / 2);
    for (let i = 0; i < clean.length; i += 2) {
      bytes[i / 2] = parseInt(clean.slice(i, i + 2), 16);
    }
    return bytes;
  }

  return { renderEncrypted };
}
