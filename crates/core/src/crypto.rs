//! Qué puede hacer un paso de Cifrado (lo comparten el servidor, que lo ejecuta, y la UI, que lo edita).
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct CryptoSpec {
    /// encrypt | decrypt | sign | verify
    pub op: String,
    /// Uno de `FORMATS`.
    pub format: String,
    /// Algoritmo según el formato (`RSA-OAEP-256`, `RS256`, `SHA-256`…).
    pub alg: String,
    /// JWE: cifrado del contenido.
    pub enc: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub kid: String,
    /// Plantilla con la llave (normalmente un secreto del entorno: `{{publica_banco}}`).
    pub key: String,
    /// Plantilla con lo que se procesa.
    pub input: String,
    /// verify de HMAC: la firma recibida.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub signature: String,
    /// base64 | base64url | hex
    pub output: String,
}

impl Default for CryptoSpec {
    fn default() -> Self {
        CryptoSpec {
            op: "encrypt".into(),
            format: "jwe".into(),
            alg: "RSA-OAEP-256".into(),
            enc: "A256GCM".into(),
            kid: String::new(),
            key: String::new(),
            input: String::new(),
            signature: String::new(),
            output: "base64".into(),
        }
    }
}

pub struct Format {
    pub id: &'static str,
    pub name: &'static str,
    pub hint: &'static str,
    /// Operaciones que admite, en el orden en que se muestran.
    pub ops: &'static [&'static str],
    pub algs: &'static [&'static str],
    pub encs: &'static [&'static str],
    pub outputs: &'static [&'static str],
    /// Qué llave pide (texto de ayuda); vacío = no usa llave.
    pub key: &'static str,
}

const ENC_DEC: &[&str] = &["encrypt", "decrypt"];
const SIGN_VER: &[&str] = &["sign", "verify"];

pub const FORMATS: [Format; 8] = [
    Format {
        id: "jwe",
        name: "JWE",
        hint: "token cifrado (compacto)",
        ops: ENC_DEC,
        algs: &["RSA-OAEP-256", "RSA-OAEP", "A256KW", "dir"],
        encs: &["A256GCM", "A128CBC-HS256"],
        outputs: &[],
        key: "RSA: PEM pública para cifrar, privada para descifrar · A256KW/dir: 32 bytes",
    },
    Format {
        id: "jws",
        name: "JWS / JWT",
        hint: "firma, no oculta",
        ops: SIGN_VER,
        algs: &["RS256", "PS256", "HS256"],
        encs: &[],
        outputs: &[],
        key: "RS/PS: PEM privada para firmar, pública para verificar · HS: secreto",
    },
    Format {
        id: "aes-gcm",
        name: "AES-256-GCM",
        hint: "simétrico, con integridad",
        ops: ENC_DEC,
        algs: &[],
        encs: &[],
        outputs: &["base64", "hex"],
        key: "32 bytes en base64, hex o texto",
    },
    Format {
        id: "aes-cbc",
        name: "AES-256-CBC",
        hint: "simétrico, sistemas antiguos",
        ops: ENC_DEC,
        algs: &[],
        encs: &[],
        outputs: &["base64", "hex"],
        key: "32 bytes en base64, hex o texto",
    },
    Format {
        id: "rsa-oaep",
        name: "RSA-OAEP",
        hint: "llave pública / privada",
        ops: ENC_DEC,
        algs: &["SHA-256", "SHA-1"],
        encs: &[],
        outputs: &["base64", "hex"],
        key: "PEM pública para cifrar, privada para descifrar",
    },
    Format {
        id: "hmac",
        name: "HMAC",
        hint: "firma de webhooks",
        ops: SIGN_VER,
        algs: &["SHA-256", "SHA-512"],
        encs: &[],
        outputs: &["hex", "base64"],
        key: "secreto compartido (texto)",
    },
    Format { id: "hash", name: "SHA", hint: "huella, sin llave", ops: &["sign"], algs: &["SHA-256", "SHA-512"], encs: &[], outputs: &["hex", "base64"], key: "" },
    Format { id: "base64", name: "Base64", hint: "sólo codifica", ops: ENC_DEC, algs: &[], encs: &[], outputs: &["base64", "base64url"], key: "" },
];

pub fn format(id: &str) -> Option<&'static Format> {
    FORMATS.iter().find(|f| f.id == id)
}

pub fn op_label(op: &str, format: &str) -> &'static str {
    match (op, format) {
        ("encrypt", "base64") => "Codificar",
        ("decrypt", "base64") => "Decodificar",
        ("sign", "hash") => "Calcular huella",
        ("encrypt", _) => "Cifrar",
        ("decrypt", _) => "Descifrar",
        ("sign", _) => "Firmar",
        ("verify", _) => "Verificar",
        _ => "?",
    }
}

impl CryptoSpec {
    /// Pasa a otro formato dejando valores válidos para él (op, alg, enc y salida).
    pub fn switch_format(&mut self, id: &str) {
        let Some(f) = format(id) else { return };
        self.format = id.into();
        if !f.ops.contains(&self.op.as_str()) {
            self.op = f.ops[0].into();
        }
        self.alg = f.algs.first().map(|s| s.to_string()).unwrap_or_default();
        self.enc = f.encs.first().map(|s| s.to_string()).unwrap_or_default();
        self.output = f.outputs.first().map(|s| s.to_string()).unwrap_or_default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn switch_keeps_spec_valid() {
        let mut s = CryptoSpec { op: "decrypt".into(), ..Default::default() };
        s.switch_format("jws");
        assert_eq!((s.op.as_str(), s.alg.as_str(), s.enc.as_str()), ("sign", "RS256", ""));
        s.switch_format("aes-gcm");
        assert_eq!((s.op.as_str(), s.output.as_str()), ("encrypt", "base64"));
        assert!(FORMATS.iter().all(|f| !f.ops.is_empty()));
    }
}
