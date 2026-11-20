//! 敏感令牌（GitHub OAuth token 等）落盘保护。
//!
//! Windows 上使用 DPAPI（`CryptProtectData`）：密钥由系统按当前用户派生，
//! 文件被拷到其他机器或其他用户下无法解密，无需应用侧管理密钥。
//! macOS 与 Linux 使用平台凭据服务保存 AES-256-GCM 密钥。

const DPAPI_PREFIX: &str = "dpapi-v1:";
const VAULT_PREFIX: &str = "vault-v1:";

pub fn needs_upgrade(encoded: &str) -> bool {
    !encoded.is_empty() && !encoded.starts_with(DPAPI_PREFIX) && !encoded.starts_with(VAULT_PREFIX)
}

pub fn migrate_saved_sessions(root: &std::path::Path) {
    for (name, fields) in [
        ("qookix_auth.json", &["access_token", "refresh_token"][..]),
        ("shimu_auth.json", &["access_token"][..]),
        ("frp_auth.json", &["token"][..]),
        (
            "cloud_sync/state.json",
            &["access_token", "refresh_token"][..],
        ),
    ] {
        let path = root.join(name);
        let result = (|| -> Result<(), String> {
            let bytes = match std::fs::read(&path) {
                Ok(bytes) => bytes,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
                Err(error) => return Err(error.to_string()),
            };
            let mut data: serde_json::Value =
                serde_json::from_slice(&bytes).map_err(|err| err.to_string())?;
            let mut changed = false;
            for field in fields {
                let Some(encoded) = data
                    .get(field)
                    .and_then(|v| v.as_str())
                    .filter(|value| needs_upgrade(value))
                else {
                    continue;
                };
                let plain = if name == "cloud_sync/state.json" {
                    crate::cloud_sync::store::decrypt_secret(encoded)
                } else {
                    unprotect(encoded)?
                };
                if plain.is_empty() {
                    return Err("旧凭据无法读取，保留原文件供恢复".into());
                }
                data[*field] = serde_json::Value::String(protect(&plain)?);
                changed = true;
            }
            if changed {
                let bytes = serde_json::to_vec_pretty(&data).map_err(|err| err.to_string())?;
                crate::fsutil::atomic_write(&path, &bytes).map_err(|err| err.to_string())?;
            }
            Ok(())
        })();
        if let Err(error) = result {
            eprintln!("[credentials] {name} 迁移失败: {error}");
        }
    }
}

/// 加密明文，返回可直接写入 JSON 的字符串（Windows: base64(DPAPI blob)）。
pub fn protect(plain: &str) -> Result<String, String> {
    if plain.is_empty() {
        return Ok(String::new());
    }
    #[cfg(windows)]
    {
        use base64::Engine as _;
        use windows_sys::Win32::Foundation::LocalFree;
        use windows_sys::Win32::Security::Cryptography::CryptProtectData;
        use windows_sys::Win32::Security::Cryptography::CRYPT_INTEGER_BLOB;

        let data = plain.as_bytes();
        let mut input = CRYPT_INTEGER_BLOB {
            cbData: data.len() as u32,
            pbData: data.as_ptr() as *mut u8,
        };
        let mut output = CRYPT_INTEGER_BLOB {
            cbData: 0,
            pbData: std::ptr::null_mut(),
        };
        // CRYPTPROTECT_UI_FORBIDDEN：后台调用，不允许弹窗
        let ok = unsafe {
            CryptProtectData(
                &mut input,
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                1, // CRYPTPROTECT_UI_FORBIDDEN
                &mut output,
            )
        };
        if ok == 0 {
            return Err("DPAPI 加密失败".into());
        }
        let slice = unsafe { std::slice::from_raw_parts(output.pbData, output.cbData as usize) };
        let encoded = format!(
            "{DPAPI_PREFIX}{}",
            base64::engine::general_purpose::STANDARD.encode(slice)
        );
        unsafe {
            LocalFree(output.pbData as _);
        }
        Ok(encoded)
    }
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        vault::seal(plain)
    }
    #[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
    {
        Err("当前平台没有可用的系统凭据库".into())
    }
}

/// 解密 protect() 产生的内容。
pub fn unprotect(encoded: &str) -> Result<String, String> {
    if encoded.is_empty() {
        return Ok(String::new());
    }
    #[cfg(windows)]
    {
        use base64::Engine as _;
        use windows_sys::Win32::Foundation::LocalFree;
        use windows_sys::Win32::Security::Cryptography::CryptUnprotectData;
        use windows_sys::Win32::Security::Cryptography::CRYPT_INTEGER_BLOB;

        let data = base64::engine::general_purpose::STANDARD
            .decode(encoded.strip_prefix(DPAPI_PREFIX).unwrap_or(encoded))
            .map_err(|e| format!("密文 base64 解码失败: {e}"))?;
        let mut input = CRYPT_INTEGER_BLOB {
            cbData: data.len() as u32,
            pbData: data.as_ptr() as *mut u8,
        };
        let mut output = CRYPT_INTEGER_BLOB {
            cbData: 0,
            pbData: std::ptr::null_mut(),
        };
        let ok = unsafe {
            CryptUnprotectData(
                &mut input,
                std::ptr::null_mut(),
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                1, // CRYPTPROTECT_UI_FORBIDDEN
                &mut output,
            )
        };
        if ok == 0 {
            return Err("DPAPI 解密失败（可能来自其他用户/机器）".into());
        }
        let slice = unsafe { std::slice::from_raw_parts(output.pbData, output.cbData as usize) };
        let plain = String::from_utf8_lossy(slice).to_string();
        unsafe {
            LocalFree(output.pbData as _);
        }
        Ok(plain)
    }
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        if let Some(ciphertext) = encoded.strip_prefix(VAULT_PREFIX) {
            return vault::open(ciphertext);
        }
        if needs_upgrade(encoded) {
            let legacy = crate::cloud_sync::store::deobfuscate(encoded);
            if !legacy.is_empty() {
                return Ok(legacy);
            }
        }
        Err("无法解密凭据，请检查系统凭据库或重新登录".into())
    }
    #[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
    {
        Err("当前平台没有可用的系统凭据库".into())
    }
}

#[cfg(any(test, target_os = "linux", target_os = "macos"))]
mod vault {
    use super::VAULT_PREFIX;
    use base64::{engine::general_purpose::STANDARD, Engine};
    use ring::{
        aead,
        rand::{SecureRandom, SystemRandom},
    };
    #[cfg(not(test))]
    use std::sync::Mutex;

    const SERVICE: &str = "cn.qookix.launcher.credentials";
    const NONCE_LENGTH: usize = 12;

    fn key(create: bool) -> Result<aead::LessSafeKey, String> {
        let bytes = key_bytes(create)?;
        aead::UnboundKey::new(&aead::AES_256_GCM, &bytes)
            .map(aead::LessSafeKey::new)
            .map_err(|_| "无法初始化凭据保护".into())
    }

    #[cfg(not(test))]
    fn key_bytes(create: bool) -> Result<[u8; 32], String> {
        const ENTRY: &str = "local-encryption-v1";
        static KEY: Mutex<Option<[u8; 32]>> = Mutex::new(None);
        let mut cached = KEY.lock().map_err(|_| "无法锁定系统凭据")?;
        let bytes = match *cached {
            Some(bytes) => bytes,
            None => {
                let entry = keyring::Entry::new(SERVICE, ENTRY).map_err(vault_error)?;
                let bytes = match entry.get_secret() {
                    Ok(bytes) => bytes.try_into().map_err(|_| "系统凭据库中的密钥格式无效")?,
                    Err(keyring::Error::NoEntry) if create => {
                        let mut bytes = [0; 32];
                        SystemRandom::new()
                            .fill(&mut bytes)
                            .map_err(|_| "无法生成系统凭据密钥")?;
                        entry.set_secret(&bytes).map_err(vault_error)?;
                        bytes
                    }
                    Err(err) => return Err(vault_error(err)),
                };
                *cached = Some(bytes);
                bytes
            }
        };
        Ok(bytes)
    }

    #[cfg(test)]
    fn key_bytes(_create: bool) -> Result<[u8; 32], String> {
        // Tests must never create or overwrite a user's platform keyring entry.
        Ok([0x4b; 32])
    }

    #[cfg(not(test))]
    fn vault_error(error: keyring::Error) -> String {
        format!("系统凭据库不可用，请解锁钥匙串或启动 Secret Service 后重试: {error}")
    }

    pub fn seal(plain: &str) -> Result<String, String> {
        let key = key(true)?;
        let mut nonce = [0; NONCE_LENGTH];
        SystemRandom::new()
            .fill(&mut nonce)
            .map_err(|_| "无法生成凭据随机数")?;
        let mut data = plain.as_bytes().to_vec();
        key.seal_in_place_append_tag(
            aead::Nonce::assume_unique_for_key(nonce),
            aead::Aad::from(SERVICE),
            &mut data,
        )
        .map_err(|_| "无法加密凭据")?;
        let mut envelope = nonce.to_vec();
        envelope.extend_from_slice(&data);
        Ok(format!("{VAULT_PREFIX}{}", STANDARD.encode(envelope)))
    }

    pub fn open(ciphertext: &str) -> Result<String, String> {
        let mut envelope = STANDARD.decode(ciphertext).map_err(|_| "凭据格式无效")?;
        let (nonce, data) = envelope
            .split_at_mut_checked(NONCE_LENGTH)
            .ok_or("凭据长度无效")?;
        let nonce = aead::Nonce::try_assume_unique_for_key(nonce).map_err(|_| "凭据随机数无效")?;
        let plain = key(false)?
            .open_in_place(nonce, aead::Aad::from(SERVICE), data)
            .map_err(|_| "凭据校验失败，请重新登录")?;
        String::from_utf8(plain.to_vec()).map_err(|_| "凭据文本格式无效".into())
    }

    #[cfg(test)]
    #[test]
    fn stored_envelope_authenticates_the_token_and_uses_a_new_nonce_each_time() {
        let first = seal("fixture-token").unwrap();
        assert_ne!(first, seal("fixture-token").unwrap());
        let encoded = first.strip_prefix(VAULT_PREFIX).unwrap();
        assert_eq!(open(encoded).unwrap(), "fixture-token");
        let mut tampered = STANDARD.decode(encoded).unwrap();
        let last = tampered.last_mut().unwrap();
        *last ^= 1;
        assert!(open(&STANDARD.encode(tampered)).is_err());
    }
}
