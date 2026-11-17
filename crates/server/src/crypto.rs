//! Paso de Cifrado: JWE, JWS, AES-256 (GCM/CBC), RSA-OAEP, HMAC, SHA y Base64, todo en Rust puro.
use aes::cipher::{block_padding::Pkcs7, BlockDecryptMut, BlockEncryptMut, KeyIvInit};
use aes_gcm::aead::{Aead, KeyInit, Payload};
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use base64::Engine;
use hmac::{Hmac, Mac};
use martino_core::crypto::CryptoSpec;
use rand::RngCore;
use rsa::pkcs1::{DecodeRsaPrivateKey, DecodeRsaPublicKey};
use rsa::pkcs8::{DecodePrivateKey, DecodePublicKey, EncodePrivateKey, EncodePublicKey, LineEnding};
use rsa::{Oaep, Pkcs1v15Sign, Pss, RsaPrivateKey, RsaPublicKey};
use serde_json::{json, Value};
use sha2::{Digest, Sha256, Sha512};

type R<T> = Result<T, String>;

fn b64u(b: &[u8]) -> String {
    URL_SAFE_NO_PAD.encode(b)
}

/// Decodifica base64 estándar o URL, con o sin `=`.
fn unb64(s: &str) -> R<Vec<u8>> {
    let t: String = s.chars().filter(|c| !c.is_whitespace()).collect();
    let t = t.trim_end_matches('=');
    URL_SAFE_NO_PAD.decode(t.replace('+', "-").replace('/', "_")).map_err(|_| "No es base64 válido".into())
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn unhex(s: &str) -> Option<Vec<u8>> {
    let s = s.trim();
    (s.len() % 2 == 0 && s.chars().all(|c| c.is_ascii_hexdigit())).then(|| (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect())
}

fn encode(b: &[u8], out: &str) -> String {
    match out {
        "hex" => hex(b),
        "base64url" => b64u(b),
        _ => STANDARD.encode(b),
    }
}

fn decode(s: &str, out: &str) -> R<Vec<u8>> {
    if out == "hex" {
        unhex(s).ok_or_else(|| "No es hex válido".into())
    } else {
        unb64(s)
    }
}

fn random(n: usize) -> Vec<u8> {
    let mut v = vec![0u8; n];
    rand::thread_rng().fill_bytes(&mut v);
    v
}

/// Llave simétrica de `len` bytes: en hex, base64 o texto tal cual.
fn sym_key(s: &str, len: usize) -> R<Vec<u8>> {
    let s = s.trim();
    if s.is_empty() {
        return Err("Falta la llave".into());
    }
    if let Some(b) = unhex(s).filter(|b| b.len() == len) {
        return Ok(b);
    }
    if let Some(b) = unb64(s).ok().filter(|b| b.len() == len) {
        return Ok(b);
    }
    if s.len() == len {
        return Ok(s.as_bytes().to_vec());
    }
    Err(format!("La llave debe tener {len} bytes (en hex, base64 o texto); llegó una de {} caracteres", s.len()))
}

/// Los entornos guardan el PEM en una línea: `\n` literal → salto de línea.
fn pem(s: &str) -> String {
    s.trim().replace("\\n", "\n")
}

fn rsa_private(s: &str) -> R<RsaPrivateKey> {
    let p = pem(s);
    RsaPrivateKey::from_pkcs8_pem(&p).or_else(|_| RsaPrivateKey::from_pkcs1_pem(&p)).map_err(|_| "La llave no es una privada RSA en PEM (PKCS#8 o PKCS#1)".into())
}

/// Pública en PEM; si llega la privada, se usa su parte pública.
fn rsa_public(s: &str) -> R<RsaPublicKey> {
    let p = pem(s);
    RsaPublicKey::from_public_key_pem(&p)
        .or_else(|_| RsaPublicKey::from_pkcs1_pem(&p))
        .or_else(|_| rsa_private(&p).map(|k| RsaPublicKey::from(&k)).map_err(|_| ()))
        .map_err(|_| "La llave no es una pública RSA en PEM".into())
}

fn oaep(hash: &str) -> Oaep {
    if hash == "SHA-1" || hash == "RSA-OAEP" {
        Oaep::new::<sha1::Sha1>()
    } else {
        Oaep::new::<Sha256>()
    }
}

fn hmac_sha(alg: &str, key: &[u8], parts: &[&[u8]]) -> Vec<u8> {
    macro_rules! go {
        ($h:ty) => {{
            let mut m = <Hmac<$h> as Mac>::new_from_slice(key).expect("HMAC acepta cualquier largo");
            parts.iter().for_each(|p| m.update(p));
            m.finalize().into_bytes().to_vec()
        }};
    }
    if alg.contains("512") {
        go!(Sha512)
    } else {
        go!(Sha256)
    }
}

/// Comparación en tiempo constante.
fn same(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

fn gcm_encrypt(key: &[u8], iv: &[u8], msg: &[u8], aad: &[u8]) -> R<Vec<u8>> {
    aes_gcm::Aes256Gcm::new_from_slice(key).map_err(|e| e.to_string())?.encrypt(aes_gcm::Nonce::from_slice(iv), Payload { msg, aad }).map_err(|_| "No se pudo cifrar".into())
}

fn gcm_decrypt(key: &[u8], iv: &[u8], ct_tag: &[u8], aad: &[u8]) -> R<Vec<u8>> {
    if iv.len() != 12 {
        return Err("El IV de AES-GCM debe tener 12 bytes".into());
    }
    aes_gcm::Aes256Gcm::new_from_slice(key)
        .map_err(|e| e.to_string())?
        .decrypt(aes_gcm::Nonce::from_slice(iv), Payload { msg: ct_tag, aad })
        .map_err(|_| "No se pudo descifrar: la llave no corresponde o el contenido fue alterado".into())
}

fn cbc_encrypt(key: &[u8], iv: &[u8], msg: &[u8]) -> R<Vec<u8>> {
    Ok(match key.len() {
        16 => cbc::Encryptor::<aes::Aes128>::new_from_slices(key, iv).map_err(|e| e.to_string())?.encrypt_padded_vec_mut::<Pkcs7>(msg),
        _ => cbc::Encryptor::<aes::Aes256>::new_from_slices(key, iv).map_err(|e| e.to_string())?.encrypt_padded_vec_mut::<Pkcs7>(msg),
    })
}

fn cbc_decrypt(key: &[u8], iv: &[u8], ct: &[u8]) -> R<Vec<u8>> {
    let bad = |_| "No se pudo descifrar: la llave no corresponde o el contenido fue alterado".to_string();
    match key.len() {
        16 => cbc::Decryptor::<aes::Aes128>::new_from_slices(key, iv).map_err(|e| e.to_string())?.decrypt_padded_vec_mut::<Pkcs7>(ct).map_err(bad),
        _ => cbc::Decryptor::<aes::Aes256>::new_from_slices(key, iv).map_err(|e| e.to_string())?.decrypt_padded_vec_mut::<Pkcs7>(ct).map_err(bad),
    }
}

// ---------- JWE (RFC 7516, compacto) ----------

fn jwe_encrypt(s: &CryptoSpec, msg: &[u8]) -> R<String> {
    let mut h = json!({"alg": s.alg, "enc": s.enc});
    if !s.kid.is_empty() {
        h["kid"] = json!(s.kid);
    }
    let protected = b64u(h.to_string().as_bytes());
    let cek = if s.alg == "dir" { sym_key(&s.key, 32)? } else { random(32) };
    let ek = match s.alg.as_str() {
        "RSA-OAEP-256" | "RSA-OAEP" => rsa_public(&s.key)?.encrypt(&mut rand::thread_rng(), oaep(&s.alg), &cek).map_err(|e| e.to_string())?,
        "A256KW" => aes_kw::KekAes256::try_from(sym_key(&s.key, 32)?.as_slice()).map_err(|e| e.to_string())?.wrap_vec(&cek).map_err(|e| e.to_string())?,
        "dir" => vec![],
        a => return Err(format!("JWE: alg «{a}» no soportado")),
    };
    let aad = protected.as_bytes();
    let (iv, ct, tag) = match s.enc.as_str() {
        "A256GCM" => {
            let iv = random(12);
            let mut ct = gcm_encrypt(&cek, &iv, msg, aad)?;
            let tag = ct.split_off(ct.len() - 16);
            (iv, ct, tag)
        }
        "A128CBC-HS256" => {
            let iv = random(16);
            let ct = cbc_encrypt(&cek[16..], &iv, msg)?;
            let tag = hmac_sha("256", &cek[..16], &[aad, &iv, &ct, &((aad.len() as u64) * 8).to_be_bytes()])[..16].to_vec();
            (iv, ct, tag)
        }
        e => return Err(format!("JWE: enc «{e}» no soportado")),
    };
    Ok([protected, b64u(&ek), b64u(&iv), b64u(&ct), b64u(&tag)].join("."))
}

fn jwe_decrypt(s: &CryptoSpec, token: &str) -> R<Vec<u8>> {
    let p: Vec<&str> = token.trim().split('.').collect();
    if p.len() != 5 {
        return Err(format!("JWE: el token debe tener 5 partes separadas por punto; tiene {}", p.len()));
    }
    let h: Value = serde_json::from_slice(&unb64(p[0])?).map_err(|_| "JWE: la cabecera no es JSON")?;
    let (alg, enc) = (h["alg"].as_str().unwrap_or(""), h["enc"].as_str().unwrap_or(""));
    let ek = unb64(p[1])?;
    let cek = match alg {
        "RSA-OAEP-256" | "RSA-OAEP" => rsa_private(&s.key)?.decrypt(oaep(alg), &ek).map_err(|_| "JWE: la llave privada no corresponde")?,
        "A256KW" => aes_kw::KekAes256::try_from(sym_key(&s.key, 32)?.as_slice()).map_err(|e| e.to_string())?.unwrap_vec(&ek).map_err(|_| "JWE: la llave no corresponde")?,
        "dir" => sym_key(&s.key, 32)?,
        a => return Err(format!("JWE: alg «{a}» no soportado")),
    };
    if cek.len() != 32 {
        return Err("JWE: la llave de contenido no mide 32 bytes".into());
    }
    let (aad, iv, ct, tag) = (p[0].as_bytes(), unb64(p[2])?, unb64(p[3])?, unb64(p[4])?);
    match enc {
        "A256GCM" => gcm_decrypt(&cek, &iv, &[ct, tag].concat(), aad),
        "A128CBC-HS256" => {
            let want = hmac_sha("256", &cek[..16], &[aad, &iv, &ct, &((aad.len() as u64) * 8).to_be_bytes()]);
            if !same(&want[..16], &tag) {
                return Err("JWE: la etiqueta no coincide (llave equivocada o token alterado)".into());
            }
            cbc_decrypt(&cek[16..], &iv, &ct)
        }
        e => Err(format!("JWE: enc «{e}» no soportado")),
    }
}

// ---------- JWS (RFC 7515, compacto) ----------

fn jws_sig(alg: &str, key: &str, input: &[u8]) -> R<Vec<u8>> {
    let hashed = Sha256::digest(input);
    match alg {
        "HS256" => Ok(hmac_sha("256", key.as_bytes(), &[input])),
        "RS256" => rsa_private(key)?.sign(Pkcs1v15Sign::new::<Sha256>(), &hashed).map_err(|e| e.to_string()),
        "PS256" => rsa_private(key)?.sign_with_rng(&mut rand::thread_rng(), Pss::new::<Sha256>(), &hashed).map_err(|e| e.to_string()),
        a => Err(format!("JWS: alg «{a}» no soportado")),
    }
}

fn jws_verify(key: &str, token: &str) -> R<Vec<u8>> {
    let p: Vec<&str> = token.trim().trim_start_matches("Bearer ").split('.').collect();
    if p.len() != 3 {
        return Err(format!("JWS: el token debe tener 3 partes; tiene {}", p.len()));
    }
    let h: Value = serde_json::from_slice(&unb64(p[0])?).map_err(|_| "JWS: la cabecera no es JSON")?;
    let input = format!("{}.{}", p[0], p[1]);
    let sig = unb64(p[2])?;
    let hashed = Sha256::digest(input.as_bytes());
    let ok = match h["alg"].as_str().unwrap_or("") {
        "HS256" => same(&hmac_sha("256", key.as_bytes(), &[input.as_bytes()]), &sig),
        "RS256" => rsa_public(key)?.verify(Pkcs1v15Sign::new::<Sha256>(), &hashed, &sig).is_ok(),
        "PS256" => rsa_public(key)?.verify(Pss::new::<Sha256>(), &hashed, &sig).is_ok(),
        a => return Err(format!("JWS: alg «{a}» no soportado")),
    };
    if !ok {
        return Err("JWS: la firma no es válida".into());
    }
    unb64(p[1])
}

/// Bytes descifrados → JSON si lo es; si no, texto.
fn plain(b: Vec<u8>) -> R<Value> {
    let t = String::from_utf8(b).map_err(|_| "El resultado no es texto UTF-8")?;
    Ok(serde_json::from_str(&t).unwrap_or(Value::String(t)))
}

/// Ejecuta el paso. `input` y la llave ya vienen con las plantillas resueltas.
pub fn run(s: &CryptoSpec, input: &str) -> R<Value> {
    let msg = input.as_bytes();
    let txt = |x: String| Ok(Value::String(x));
    match (s.format.as_str(), s.op.as_str()) {
        ("jwe", "encrypt") => txt(jwe_encrypt(s, msg)?),
        ("jwe", "decrypt") => plain(jwe_decrypt(s, input)?),
        ("jws", "sign") => {
            let mut h = json!({"alg": s.alg, "typ": "JWT"});
            if !s.kid.is_empty() {
                h["kid"] = json!(s.kid);
            }
            let si = format!("{}.{}", b64u(h.to_string().as_bytes()), b64u(msg));
            let sig = jws_sig(&s.alg, &pem(&s.key), si.as_bytes())?;
            txt(format!("{si}.{}", b64u(&sig)))
        }
        ("jws", "verify") => plain(jws_verify(&pem(&s.key), input)?),
        ("aes-gcm", "encrypt") => {
            let iv = random(12);
            txt(encode(&[iv.clone(), gcm_encrypt(&sym_key(&s.key, 32)?, &iv, msg, b"")?].concat(), &s.output))
        }
        ("aes-gcm", "decrypt") => {
            let b = decode(input, &s.output)?;
            if b.len() < 28 {
                return Err("AES-GCM: el texto es muy corto (se espera IV de 12 + cifrado + etiqueta de 16)".into());
            }
            plain(gcm_decrypt(&sym_key(&s.key, 32)?, &b[..12], &b[12..], b"")?)
        }
        ("aes-cbc", "encrypt") => {
            let iv = random(16);
            txt(encode(&[iv.clone(), cbc_encrypt(&sym_key(&s.key, 32)?, &iv, msg)?].concat(), &s.output))
        }
        ("aes-cbc", "decrypt") => {
            let b = decode(input, &s.output)?;
            if b.len() < 32 {
                return Err("AES-CBC: el texto es muy corto (se espera IV de 16 + cifrado)".into());
            }
            plain(cbc_decrypt(&sym_key(&s.key, 32)?, &b[..16], &b[16..])?)
        }
        ("rsa-oaep", "encrypt") => txt(encode(
            &rsa_public(&s.key)?.encrypt(&mut rand::thread_rng(), oaep(&s.alg), msg).map_err(|e| format!("RSA: {e} (¿el texto es muy largo para la llave?)"))?,
            &s.output,
        )),
        ("rsa-oaep", "decrypt") => plain(rsa_private(&s.key)?.decrypt(oaep(&s.alg), &decode(input, &s.output)?).map_err(|_| "RSA: la llave privada no corresponde")?),
        ("hmac", "sign") => txt(encode(&hmac_sha(&s.alg, s.key.as_bytes(), &[msg]), &s.output)),
        ("hmac", "verify") => {
            let want = hmac_sha(&s.alg, s.key.as_bytes(), &[msg]);
            let got = decode(&s.signature, &s.output)?;
            if same(&want, &got) {
                Ok(json!(true))
            } else {
                Err("HMAC: la firma no coincide".into())
            }
        }
        ("hash", _) => txt(encode(&if s.alg.contains("512") { Sha512::digest(msg).to_vec() } else { Sha256::digest(msg).to_vec() }, &s.output)),
        ("base64", "encrypt") => txt(encode(msg, if s.output == "base64url" { "base64url" } else { "base64" })),
        ("base64", "decrypt") => plain(unb64(input)?),
        (f, o) => Err(format!("Cifrado: «{o}» no aplica a {f}")),
    }
}

/// Par o llave de prueba: `rsa` → PEM pública + privada · `aes` → 32 bytes en base64.
pub fn keygen(kind: &str, bits: usize) -> R<Value> {
    if kind == "aes" {
        return Ok(json!({"key": STANDARD.encode(random(32))}));
    }
    let k = RsaPrivateKey::new(&mut rand::thread_rng(), bits).map_err(|e| e.to_string())?;
    let private = k.to_pkcs8_pem(LineEnding::LF).map_err(|e| e.to_string())?.to_string();
    let public = RsaPublicKey::from(&k).to_public_key_pem(LineEnding::LF).map_err(|e| e.to_string())?;
    Ok(json!({"public": public, "private": private}))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(format: &str, op: &str, alg: &str, enc: &str, key: &str) -> CryptoSpec {
        CryptoSpec { format: format.into(), op: op.into(), alg: alg.into(), enc: enc.into(), key: key.into(), output: "base64".into(), ..Default::default() }
    }

    #[test]
    fn round_trips() {
        let k = keygen("rsa", 1024).unwrap();
        let (public, private) = (k["public"].as_str().unwrap(), k["private"].as_str().unwrap());
        let aes = keygen("aes", 0).unwrap()["key"].as_str().unwrap().to_string();
        let body = r#"{"monto":150,"moneda":"PEN"}"#;
        for (alg, ek, dk) in [("RSA-OAEP-256", public, private), ("RSA-OAEP", public, private), ("A256KW", aes.as_str(), aes.as_str()), ("dir", aes.as_str(), aes.as_str())] {
            for enc in ["A256GCM", "A128CBC-HS256"] {
                let t = run(&spec("jwe", "encrypt", alg, enc, ek), body).unwrap();
                assert_eq!(t.as_str().unwrap().split('.').count(), 5);
                let back = run(&spec("jwe", "decrypt", "", "", dk), t.as_str().unwrap()).unwrap();
                assert_eq!(back, json!({"monto": 150, "moneda": "PEN"}), "{alg} {enc}");
            }
        }
        let t = run(&spec("jwe", "encrypt", "A256KW", "A256GCM", &aes), body).unwrap();
        let mut bad = t.as_str().unwrap().to_string();
        bad.pop();
        bad.push(if t.as_str().unwrap().ends_with('A') { 'B' } else { 'A' });
        assert!(run(&spec("jwe", "decrypt", "", "", &aes), &bad).is_err(), "token alterado");
        for (alg, sk, vk) in [("RS256", private, public), ("PS256", private, public), ("HS256", "secreto", "secreto")] {
            let t = run(&spec("jws", "sign", alg, "", sk), body).unwrap();
            assert_eq!(run(&spec("jws", "verify", "", "", vk), t.as_str().unwrap()).unwrap()["monto"], json!(150), "{alg}");
        }
        assert!(run(&spec("jws", "verify", "", "", "otro"), run(&spec("jws", "sign", "HS256", "", "secreto"), body).unwrap().as_str().unwrap()).is_err());
        for f in ["aes-gcm", "aes-cbc"] {
            let c = run(&spec(f, "encrypt", "", "", &aes), "hola").unwrap();
            assert_eq!(run(&spec(f, "decrypt", "", "", &aes), c.as_str().unwrap()).unwrap(), json!("hola"), "{f}");
        }
        let c = run(&spec("rsa-oaep", "encrypt", "SHA-256", "", public), "hola").unwrap();
        assert_eq!(run(&spec("rsa-oaep", "decrypt", "SHA-256", "", private), c.as_str().unwrap()).unwrap(), json!("hola"));
        let mut h = spec("hash", "sign", "SHA-256", "", "");
        h.output = "hex".into();
        assert_eq!(run(&h, "abc").unwrap(), json!("ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"));
        let mut m = spec("hmac", "sign", "SHA-256", "", "key");
        m.output = "hex".into();
        let sig = run(&m, "The quick brown fox jumps over the lazy dog").unwrap();
        assert_eq!(sig, json!("f7bc83f430538424b13298e6aa6fb143ef4d59a14946175997479dbc2d1a3cd8"));
        m.op = "verify".into();
        m.signature = sig.as_str().unwrap().into();
        assert_eq!(run(&m, "The quick brown fox jumps over the lazy dog").unwrap(), json!(true));
        assert!(run(&m, "otro texto").is_err());
        assert_eq!(run(&spec("base64", "decrypt", "", "", ""), "eyJhIjoxfQ").unwrap(), json!({"a": 1}));
        assert!(sym_key("corta", 32).unwrap_err().contains("32 bytes"));
        assert_eq!(unb64("+w==").unwrap(), unb64("-w").unwrap());
    }
}
