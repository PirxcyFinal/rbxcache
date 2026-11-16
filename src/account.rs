//! Roblox accounts whose session cookie has been pasted in, and the request
//! headers that go with them.
//!
//! Signing in is how sounds, and other assets Roblox will not hand to a guest,
//! can be fetched and matched. There is no Roblox login flow built into
//! rbxcache: the user copies their own `.ROBLOSECURITY` cookie from their own
//! already-signed-in browser and pastes it in. rbxcache never reads
//! `RobloxCookies.dat` or any browser's cookie storage by itself.
//!
//! **The cookie is the whole account to anyone who holds it. It can be a main
//! account; an alternate account is recommended. Never paste anyone else's.**
//! See [`WARNING`] for the text shown wherever a cookie is pasted. Nothing is
//! sent until that warning has been up for [`SUBMIT_HOLD`].
//!
//! Accounts are kept in `accounts.json` in the data folder, several at once,
//! each with its cookie encrypted for this Windows user (DPAPI), so the file
//! is no use on another user or computer. Removing an account deletes it from
//! this computer at once; it does not end the session on Roblox's side; the
//! account page says so and links to where that can be done.

use crate::protection::data_dir;
use crate::store::hex;
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::Duration;

/// Who a cookie belongs to, as Roblox reports it.
#[derive(Clone, Debug, PartialEq)]
pub struct Identity {
    pub user_id: u64,
    pub name: String,
    pub display: String,
}

/// The bold warning shown before a cookie is sent, and beside every place one is pasted.
pub const WARNING: &str = "This can be a main account. An alternate account is recommended. \
     Whoever holds this cookie can sign in as that account and do anything it can do, including spending its Robux. \
     Never paste anyone else's cookie.";

/// How long [`WARNING`] stays up before a cookie may be sent.
pub const SUBMIT_HOLD: Duration = Duration::from_secs(5);

/// One account that has been added, as kept on disk. The cookie is encrypted.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Account {
    pub user_id: u64,
    pub name: String,
    #[serde(default)]
    pub display: String,
    /// The session cookie, encrypted for this user; see [`encrypt`].
    cookie: String,
    /// When it was added, milliseconds since the Unix epoch.
    #[serde(default)]
    pub added: i64,
}

impl Account {
    /// The decrypted session cookie, if it can still be read on this machine.
    pub fn cookie(&self) -> Option<String> {
        decrypt(&self.cookie)
    }
}

/// The accounts that have been added, and which one is in use.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Accounts {
    #[serde(default)]
    pub accounts: Vec<Account>,
    /// The user ID of the account whose cookie is sent with requests.
    #[serde(default)]
    pub active: Option<u64>,
}

fn path() -> Result<PathBuf> {
    Ok(data_dir()?.join("accounts.json"))
}

impl Accounts {
    /// The accounts that have been added, or none when there are none or the file cannot be read.
    pub fn load() -> Self {
        let read = || -> Result<Self> { Ok(serde_json::from_slice(&std::fs::read(path()?)?)?) };
        read().unwrap_or_default()
    }

    pub fn save(&self) -> Result<()> {
        let path = path()?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(self)?).with_context(|| format!("writing {}", tmp.display()))?;
        std::fs::rename(&tmp, &path).with_context(|| format!("replacing {}", path.display()))
    }

    /// Adds an account from a cookie that has just been checked with [`whoami`],
    /// or replaces the one already added under that user ID, and makes it the
    /// active account.
    pub fn add(&mut self, who: Identity, cookie: &str) -> Result<()> {
        let cookie = encrypt(cookie).context("this cookie could not be stored securely on this computer")?;
        let added = self.accounts.iter().find(|a| a.user_id == who.user_id).map_or(now_ms(), |a| a.added);
        let account = Account { user_id: who.user_id, name: who.name, display: who.display, cookie, added };
        self.accounts.retain(|a| a.user_id != account.user_id);
        self.active = Some(account.user_id);
        self.accounts.push(account);
        self.save()
    }

    /// Removes an account from this computer at once. It does not end the
    /// session on Roblox's side: that cookie goes on working until it expires
    /// or the account is signed out of on roblox.com.
    pub fn remove(&mut self, user_id: u64) -> Result<()> {
        self.accounts.retain(|a| a.user_id != user_id);
        if self.active == Some(user_id) {
            self.active = self.accounts.first().map(|a| a.user_id);
        }
        self.save()
    }

    pub fn set_active(&mut self, user_id: u64) -> Result<()> {
        if self.accounts.iter().any(|a| a.user_id == user_id) {
            self.active = Some(user_id);
            self.save()?;
        }
        Ok(())
    }

    pub fn active(&self) -> Option<&Account> {
        let id = self.active?;
        self.accounts.iter().find(|a| a.user_id == id).or(self.accounts.first())
    }

    /// The cookie to send with asset requests: the active account's.
    pub fn active_cookie(&self) -> Option<String> {
        self.active().and_then(Account::cookie)
    }

    pub fn is_empty(&self) -> bool {
        self.accounts.is_empty()
    }
}

/// Points requests at the active account's cookie, or at none when nobody is added.
pub fn activate() {
    crate::resolve::set_auth(Accounts::load().active_cookie());
}

fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

/// Pulls just the `.ROBLOSECURITY` value out of whatever was pasted: the
/// cookie header's `name=value` form, a `Cookie:` line, or the bare value.
pub fn extract(pasted: &str) -> Option<String> {
    let pasted = pasted.trim();
    if pasted.is_empty() {
        return None;
    }
    for part in pasted.split(['\n', ';']) {
        let part = part.trim();
        if let Some(rest) = part.strip_prefix("Cookie:") {
            if let Some(value) = extract(rest) {
                return Some(value);
            }
        }
        if let Some(rest) = part.strip_prefix(".ROBLOSECURITY=") {
            let value = rest.trim().trim_matches('"');
            if !value.is_empty() {
                return Some(value.to_string());
            }
        }
    }
    // No `name=value` form found anywhere: take the whole paste as the value itself.
    (!pasted.contains('=') || pasted.starts_with("_|WARNING")).then(|| pasted.to_string())
}

/// Asks Roblox who a cookie belongs to. This both reads the account's name and
/// proves the cookie is a real, signed-in session: a guest or an expired
/// cookie is refused, and returns an error here.
pub fn whoami(cookie: &str) -> Result<Identity> {
    let response = ureq::get("https://users.roblox.com/v1/users/authenticated")
        .set("Cookie", &format!(".ROBLOSECURITY={cookie}"))
        .set("User-Agent", "rbxcache")
        .timeout(Duration::from_secs(15))
        .call();
    let body: serde_json::Value = match response {
        Ok(response) => serde_json::from_reader(response.into_reader()).context("Roblox sent an unreadable answer")?,
        Err(ureq::Error::Status(401, _)) => bail!("that is not a signed-in cookie (Roblox returned 401); it may be wrong or have expired"),
        Err(e) => return Err(e).context("could not reach Roblox to check the cookie"),
    };
    identity_from(&body)
}

/// The identity in Roblox's answer to `users/authenticated`.
fn identity_from(body: &serde_json::Value) -> Result<Identity> {
    let user_id = body["id"].as_u64().context("Roblox did not say who the cookie belongs to")?;
    let name = body["name"].as_str().unwrap_or("").to_string();
    let display = body["displayName"].as_str().unwrap_or(&name).to_string();
    Ok(Identity { user_id, name, display })
}

/// Checks with Roblox that the active account is still signed in, before
/// anything is fetched as it. `Ok(None)` when no account has been added.
/// Fetches are sent with this account's cookie, so a cookie Roblox has
/// rejected is an error here; otherwise every refusal after it would look
/// like the asset being unavailable.
pub fn check_active() -> Result<Option<Identity>> {
    let accounts = Accounts::load();
    let Some(account) = accounts.active() else { return Ok(None) };
    let who = format!("@{}", account.name);
    let cookie =
        account.cookie().with_context(|| format!("Signing in as {who}: the stored cookie cannot be read on this computer; add the account again"))?;
    whoami(&cookie).map(Some).with_context(|| format!("Signing in as {who}"))
}

/// The picture shown for an account: Roblox's avatar headshot, as image bytes.
pub fn avatar(user_id: u64) -> Option<Vec<u8>> {
    use std::io::Read;
    let listing = format!("https://thumbnails.roblox.com/v1/users/avatar-headshot?userIds={user_id}&size=150x150&format=Png&isCircular=false");
    let found: serde_json::Value =
        serde_json::from_reader(ureq::get(&listing).set("User-Agent", "rbxcache").timeout(Duration::from_secs(10)).call().ok()?.into_reader())
            .ok()?;
    let url = found["data"][0]["imageUrl"].as_str()?;
    let mut bytes = Vec::new();
    ureq::get(url).timeout(Duration::from_secs(10)).call().ok()?.into_reader().take(2 << 20).read_to_end(&mut bytes).ok()?;
    (!bytes.is_empty()).then_some(bytes)
}

/// Encrypts a cookie for storage. On Windows this is DPAPI, tied to the signed-in
/// Windows user, so the file is no use on another account or machine.
pub fn encrypt(cookie: &str) -> Option<String> {
    #[cfg(windows)]
    {
        dpapi::protect(cookie.as_bytes()).map(|bytes| format!("dpapi:{}", hex(&bytes)))
    }
    // There is no account storage on other platforms yet; the plain form only
    // keeps the module building and tested.
    #[cfg(not(windows))]
    {
        Some(format!("plain:{}", hex(cookie.as_bytes())))
    }
}

/// Reads a cookie encrypted by [`encrypt`]. `None` if it cannot be decrypted here.
pub fn decrypt(stored: &str) -> Option<String> {
    let unhex = |s: &str| -> Option<Vec<u8>> {
        if !s.len().is_multiple_of(2) {
            return None;
        }
        (0..s.len() / 2).map(|i| u8::from_str_radix(s.get(i * 2..i * 2 + 2)?, 16).ok()).collect()
    };
    if let Some(rest) = stored.strip_prefix("plain:") {
        return unhex(rest).and_then(|bytes| String::from_utf8(bytes).ok());
    }
    #[cfg(windows)]
    if let Some(rest) = stored.strip_prefix("dpapi:") {
        return dpapi::unprotect(&unhex(rest)?).and_then(|bytes| String::from_utf8(bytes).ok());
    }
    None
}

#[cfg(windows)]
mod dpapi {
    use std::ffi::c_void;

    #[repr(C)]
    struct Blob {
        cb: u32,
        data: *mut u8,
    }

    #[link(name = "crypt32")]
    extern "system" {
        fn CryptProtectData(
            input: *const Blob,
            name: *const u16,
            entropy: *const Blob,
            reserved: *mut c_void,
            prompt: *mut c_void,
            flags: u32,
            out: *mut Blob,
        ) -> i32;
        fn CryptUnprotectData(
            input: *const Blob,
            name: *mut *mut u16,
            entropy: *const Blob,
            reserved: *mut c_void,
            prompt: *mut c_void,
            flags: u32,
            out: *mut Blob,
        ) -> i32;
    }
    #[link(name = "kernel32")]
    extern "system" {
        fn LocalFree(mem: *mut c_void) -> *mut c_void;
    }

    /// Fail rather than pop up a prompt.
    const UI_FORBIDDEN: u32 = 0x1;

    /// Copies the output blob out and frees it the way DPAPI requires.
    unsafe fn take(ok: i32, out: Blob) -> Option<Vec<u8>> {
        if ok == 0 || out.data.is_null() {
            return None;
        }
        let bytes = std::slice::from_raw_parts(out.data, out.cb as usize).to_vec();
        LocalFree(out.data as *mut c_void);
        Some(bytes)
    }

    pub fn protect(plain: &[u8]) -> Option<Vec<u8>> {
        // SAFETY: the input blob points at `plain` for its length; the output blob is taken and freed by `take`.
        unsafe {
            let input = Blob { cb: plain.len() as u32, data: plain.as_ptr() as *mut u8 };
            let mut out = Blob { cb: 0, data: std::ptr::null_mut() };
            let ok = CryptProtectData(&input, std::ptr::null(), std::ptr::null(), std::ptr::null_mut(), std::ptr::null_mut(), UI_FORBIDDEN, &mut out);
            take(ok, out)
        }
    }

    pub fn unprotect(encrypted: &[u8]) -> Option<Vec<u8>> {
        // SAFETY: as in `protect`.
        unsafe {
            let input = Blob { cb: encrypted.len() as u32, data: encrypted.as_ptr() as *mut u8 };
            let mut out = Blob { cb: 0, data: std::ptr::null_mut() };
            let ok = CryptUnprotectData(
                &input,
                std::ptr::null_mut(),
                std::ptr::null(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                UI_FORBIDDEN,
                &mut out,
            );
            take(ok, out)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_cookie_survives_a_round_trip_through_storage() {
        let secret = "_|WARNING:-DO-NOT-SHARE-THIS.--Sharing-this-will-allow-someone-to-log-in-as-you.|_ABCDEF0123456789";
        let stored = encrypt(secret).expect("encrypt");
        assert_ne!(stored, secret);
        assert!(!stored.contains(secret), "the cookie must not be stored in the clear");
        assert_eq!(decrypt(&stored).as_deref(), Some(secret));
        assert_eq!((decrypt("plain:not-hex!"), decrypt("dpapi:zzzz")), (None, None));
    }

    #[test]
    fn the_cookie_value_is_pulled_out_of_whatever_form_was_pasted() {
        let value = "_|WARNING:-abc123";
        assert_eq!(extract(&format!(".ROBLOSECURITY={value}; Path=/; HttpOnly")), Some(value.to_string()));
        assert_eq!(extract(&format!("Cookie: .ROBLOSECURITY={value}")), Some(value.to_string()));
        assert_eq!(extract(&format!("  {value}  \n")), Some(value.to_string()));
        assert_eq!(extract(value), Some(value.to_string()));
        assert_eq!(extract(""), None);
        assert_eq!(extract("Path=/; HttpOnly"), None);
    }

    #[test]
    fn roblox_says_whose_cookie_it_is() {
        let who = identity_from(&serde_json::json!({"id": 42, "name": "alt_one", "displayName": "Alt One"})).unwrap();
        assert_eq!(who, Identity { user_id: 42, name: "alt_one".into(), display: "Alt One".into() });
        // A missing display name falls back to the account name.
        assert_eq!(identity_from(&serde_json::json!({"id": 7, "name": "x"})).unwrap().display, "x");
        assert!(identity_from(&serde_json::json!({"name": "no id"})).is_err());
    }

    #[test]
    fn accounts_are_added_removed_and_kept_distinct() {
        let mut accounts = Accounts::default();
        accounts.accounts.push(Account { user_id: 1, name: "alt_one".into(), display: "Alt One".into(), cookie: encrypt("c1").unwrap(), added: 1 });
        accounts.active = Some(1);
        assert_eq!(accounts.active().unwrap().name, "alt_one");
        assert_eq!(accounts.active_cookie().as_deref(), Some("c1"));

        // The same user adding a new cookie replaces the old entry rather than doubling it, and keeps `added`.
        let kept_added = accounts.accounts[0].added;
        let replace =
            Account { user_id: 1, name: "alt_one".into(), display: "Alt One".into(), cookie: encrypt("c1-new").unwrap(), added: kept_added };
        accounts.accounts.retain(|a| a.user_id != 1);
        accounts.accounts.push(replace);
        assert_eq!((accounts.accounts.len(), accounts.accounts[0].added), (1, 1));
        assert_eq!(accounts.active_cookie().as_deref(), Some("c1-new"));

        accounts.accounts.push(Account { user_id: 2, name: "alt_two".into(), display: "Alt Two".into(), cookie: encrypt("c2").unwrap(), added: 3 });
        // Removing the active account hands the active slot to another.
        accounts.accounts.retain(|a| a.user_id != 1);
        accounts.active = accounts.accounts.first().map(|a| a.user_id);
        assert_eq!(accounts.active, Some(2));
        assert_eq!(accounts.active().unwrap().user_id, 2);
        accounts.accounts.clear();
        assert!(accounts.active().is_none() && accounts.is_empty());
    }
}
