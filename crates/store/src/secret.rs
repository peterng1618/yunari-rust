//! The OS protector: DPAPI on Windows.
//!
//! A copy of the session file taken to another account or machine no longer
//! yields the token. The one unsafe call in this crate is here, because Win32
//! has no safe binding; every other module stays safe.
//!
//! On a platform without DPAPI the bytes are kept as they are: this build ships
//! Windows first, and a fallback that pretends to encrypt would be worse than
//! one that says what it does.

#[cfg(windows)]
#[allow(unsafe_code)]
mod os {
    use std::ptr;

    use windows_sys::Win32::Foundation::{ERROR_INVALID_DATA, GetLastError, LocalFree};
    use windows_sys::Win32::Security::Cryptography::{
        CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN, CryptProtectData, CryptUnprotectData,
    };

    /// Protects `plain` for this Windows account.
    pub fn protect(plain: &[u8]) -> Result<Vec<u8>, String> {
        exchange(plain, true)
    }

    /// Recovers bytes protected for this Windows account.
    pub fn unprotect(sealed: &[u8]) -> Result<Vec<u8>, String> {
        exchange(sealed, false)
    }

    fn exchange(input: &[u8], sealing: bool) -> Result<Vec<u8>, String> {
        if input.is_empty() {
            return Err("There is nothing to protect.".to_owned());
        }
        let given = CRYPT_INTEGER_BLOB {
            cbData: input.len() as u32,
            pbData: input.as_ptr() as *mut u8,
        };
        let mut out = CRYPT_INTEGER_BLOB {
            cbData: 0,
            pbData: ptr::null_mut(),
        };
        // SAFETY: both blobs point at buffers this call owns for its duration;
        // the optional arguments are null, which the API accepts, and the
        // output is freed with LocalFree below.
        let ok = unsafe {
            if sealing {
                CryptProtectData(
                    &given,
                    ptr::null(),
                    ptr::null(),
                    ptr::null(),
                    ptr::null(),
                    CRYPTPROTECT_UI_FORBIDDEN,
                    &mut out,
                )
            } else {
                CryptUnprotectData(
                    &given,
                    ptr::null_mut(),
                    ptr::null(),
                    ptr::null(),
                    ptr::null(),
                    CRYPTPROTECT_UI_FORBIDDEN,
                    &mut out,
                )
            }
        };
        if ok == 0 {
            let code = unsafe { GetLastError() };
            let reason = if code == ERROR_INVALID_DATA {
                "The stored session cannot be read on this account.".to_owned()
            } else {
                format!("The OS protector refused the session (error {code}).")
            };
            return Err(reason);
        }
        // SAFETY: `out` was filled by the call above and is owned here.
        let bytes = unsafe {
            let slice = std::slice::from_raw_parts(out.pbData, out.cbData as usize);
            let copy = slice.to_vec();
            LocalFree(out.pbData as *mut std::ffi::c_void);
            copy
        };
        Ok(bytes)
    }
}

#[cfg(windows)]
pub use os::{protect, unprotect};

#[cfg(not(windows))]
mod other {
    pub fn protect(plain: &[u8]) -> Result<Vec<u8>, String> {
        Ok(plain.to_vec())
    }

    pub fn unprotect(sealed: &[u8]) -> Result<Vec<u8>, String> {
        Ok(sealed.to_vec())
    }
}

#[cfg(not(windows))]
pub use other::{protect, unprotect};
