//! Stored connections never return keys to the webview. Windows protects them with user DPAPI.
pub fn seal(text: &str) -> Result<String, String> {
    #[cfg(windows)]
    {
        use base64::Engine;
        use windows_sys::Win32::{Foundation::LocalFree, Security::Cryptography::*};
        let input = CRYPT_INTEGER_BLOB {
            cbData: text.len() as u32,
            pbData: text.as_ptr() as *mut u8,
        };
        let mut output = CRYPT_INTEGER_BLOB {
            cbData: 0,
            pbData: std::ptr::null_mut(),
        };
        let ok = unsafe {
            CryptProtectData(
                &input,
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        };
        if ok == 0 {
            return Err("无法使用 Windows 加密保存连接配置".into());
        }
        let encoded = unsafe {
            base64::engine::general_purpose::STANDARD.encode(std::slice::from_raw_parts(
                output.pbData,
                output.cbData as usize,
            ))
        };
        unsafe {
            LocalFree(output.pbData as *mut _);
        }
        Ok(format!("dpapi:{encoded}"))
    }
    #[cfg(not(windows))]
    {
        Ok(text.to_owned())
    }
}
pub fn open(text: &str) -> Result<String, String> {
    let Some(encoded) = text.strip_prefix("dpapi:") else {
        return Ok(text.to_owned());
    };
    #[cfg(windows)]
    {
        use base64::Engine;
        use windows_sys::Win32::{Foundation::LocalFree, Security::Cryptography::*};
        let mut bytes = base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .map_err(|_| "加密连接数据损坏")?;
        let input = CRYPT_INTEGER_BLOB {
            cbData: bytes.len() as u32,
            pbData: bytes.as_mut_ptr(),
        };
        let mut output = CRYPT_INTEGER_BLOB {
            cbData: 0,
            pbData: std::ptr::null_mut(),
        };
        let ok = unsafe {
            CryptUnprotectData(
                &input,
                std::ptr::null_mut(),
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        };
        if ok == 0 {
            return Err("此连接配置无法由当前 Windows 用户解密，请重新填写 API Key".into());
        }
        let result = unsafe {
            String::from_utf8(
                std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec(),
            )
        }
        .map_err(|_| "连接数据不是 UTF-8");
        unsafe {
            LocalFree(output.pbData as *mut _);
        }
        result.map_err(str::to_owned)
    }
    #[cfg(not(windows))]
    {
        let _ = encoded;
        Err("Windows 加密配置需要原 Windows 用户重新导入".into())
    }
}
