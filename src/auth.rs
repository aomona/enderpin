//! Microsoft device login. Refresh credentials stay in the OS credential store.
use anyhow::{Context, Result, bail, ensure};
use reqwest::blocking::{Client, Response};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::json;
use std::{
    io::Read,
    time::{Duration, Instant},
};

const DEVICE: &str = "https://login.microsoftonline.com/consumers/oauth2/v2.0/devicecode";
const TOKEN: &str = "https://login.microsoftonline.com/consumers/oauth2/v2.0/token";
const SCOPE: &str = "XboxLive.signin offline_access";
pub mod broker;

#[derive(Clone)]
pub struct Session {
    pub name: String,
    pub uuid: String,
    pub(crate) access_token: String,
    pub(crate) expires_at: Instant,
    pub(crate) generation: Option<String>,
}
impl Session {
    pub(crate) fn is_current(&self) -> bool {
        Instant::now() < self.expires_at
            && self.generation.as_ref().is_some_and(|generation| {
                auth_generation().is_ok_and(|current| &current == generation)
            })
    }
}

pub(crate) fn state_root() -> Result<std::path::PathBuf> {
    let dirs = directories::ProjectDirs::from("", "", "enderpin")
        .context("OS application directory unavailable")?;
    let root = dirs.data_local_dir();
    std::fs::create_dir_all(root)?;
    Ok(crate::storage::directory(&root.canonicalize()?, "auth")?.canonicalize()?)
}
fn auth_generation() -> Result<String> {
    crate::storage::read_optional(&state_root()?, "generation")?
        .context("authentication lease was revoked")
}
fn advance_generation() -> Result<String> {
    let root = state_root()?;
    let mut bytes = [0; 32];
    getrandom::fill(&mut bytes).map_err(|_| anyhow::anyhow!("secure randomness unavailable"))?;
    let generation: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
    use std::io::Write;
    let mut file = tempfile::NamedTempFile::new_in(&root)?;
    file.write_all(generation.as_bytes())?;
    file.as_file().sync_all()?;
    file.persist(crate::storage::safe_path(&root, "generation")?)
        .map_err(|error| error.error)?;
    Ok(generation)
}

/// Safe fields to display to the person signing in. Never includes device/refresh tokens.
pub struct LoginPrompt {
    pub verification_uri: String,
    pub user_code: String,
    pub expires_in: u64,
}

#[derive(Deserialize)]
struct Device {
    device_code: String,
    user_code: String,
    verification_uri: String,
    expires_in: u64,
    interval: u64,
}
#[derive(Serialize, Deserialize)]
struct Credential {
    client_id: String,
    refresh_token: String,
}
#[derive(Deserialize)]
struct Tokens {
    access_token: String,
    refresh_token: Option<String>,
}

fn client() -> Result<Client> {
    Ok(Client::builder()
        .https_only(true)
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(30))
        .user_agent(concat!("Enderpin/", env!("CARGO_PKG_VERSION")))
        .build()?)
}
fn body<T: DeserializeOwned>(response: Response) -> Result<T> {
    let mut bytes = vec![];
    response
        .take(262145)
        .read_to_end(&mut bytes)
        .context("authentication response could not be read")?;
    ensure!(
        bytes.len() <= 262144,
        "authentication response is too large"
    );
    // Do not include serde's offending value in errors: it may contain a secret.
    serde_json::from_slice(&bytes)
        .map_err(|_| anyhow::anyhow!("authentication returned an invalid response"))
}
fn success<T: DeserializeOwned>(response: Response, service: &str) -> Result<T> {
    ensure!(
        response.status().is_success(),
        "{service} returned HTTP {}",
        response.status()
    );
    body(response)
}
fn token(value: &str) -> Result<()> {
    ensure!(
        !value.is_empty() && value.len() <= 32768 && value.bytes().all(|b| b.is_ascii_graphic()),
        "invalid authentication token"
    );
    Ok(())
}
pub fn validate_client_id(id: &str) -> Result<()> {
    ensure!(
        id.len() == 36
            && id
                .bytes()
                .enumerate()
                .all(|(i, b)| if [8, 13, 18, 23].contains(&i) {
                    b == b'-'
                } else {
                    b.is_ascii_hexdigit()
                }),
        "Microsoft application client ID must be a UUID"
    );
    Ok(())
}
fn entry() -> Result<keyring::Entry> {
    Ok(keyring::Entry::new("enderpin.microsoft", "default")?)
}
fn save(credential: &Credential) -> Result<()> {
    entry()?
        .set_password(&serde_json::to_string(credential)?)
        .context("OS credential storage is unavailable; no plaintext fallback")
}

/// Ask for login only after the user starts this operation. The callback may cancel it.
pub fn login(
    client_id: &str,
    show: impl FnOnce(LoginPrompt) -> Result<()>,
    cancelled: impl Fn() -> bool,
) -> Result<Session> {
    validate_client_id(client_id)?;
    let client = client()?;
    let device: Device = success(
        client
            .post(DEVICE)
            .form(&[("client_id", client_id), ("scope", SCOPE)])
            .send()?,
        "Microsoft device login",
    )?;
    token(&device.device_code)?;
    ensure!(
        [
            "https://www.microsoft.com/link",
            "https://microsoft.com/devicelogin"
        ]
        .contains(&device.verification_uri.as_str()),
        "unexpected Microsoft verification URL"
    );
    ensure!(
        !device.user_code.is_empty()
            && device.user_code.len() <= 64
            && device
                .user_code
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-'),
        "invalid login code"
    );
    ensure!(
        (1..=3600).contains(&device.expires_in) && (1..=60).contains(&device.interval),
        "invalid login lifetime"
    );
    let started = Instant::now();
    show(LoginPrompt {
        verification_uri: device.verification_uri,
        user_code: device.user_code,
        expires_in: device.expires_in,
    })?;
    let mut interval = device.interval;
    loop {
        for _ in 0..interval * 10 {
            ensure!(!cancelled(), "login cancelled");
            ensure!(
                started.elapsed().as_secs() < device.expires_in,
                "login code expired; run login again"
            );
            std::thread::sleep(Duration::from_millis(100));
        }
        let response = client
            .post(TOKEN)
            .form(&[
                ("client_id", client_id),
                ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
                ("device_code", &device.device_code),
            ])
            .send()?;
        if response.status().is_success() {
            let tokens: Tokens = body(response)?;
            token(&tokens.access_token)?;
            let refresh_token = tokens
                .refresh_token
                .context("Microsoft did not issue a refresh credential")?;
            token(&refresh_token)?;
            let mut session = minecraft(&client, &tokens.access_token)?;
            let _guard = crate::storage::operation_lock(&state_root()?)?;
            session.generation = Some(advance_generation()?);
            save(&Credential {
                client_id: client_id.into(),
                refresh_token,
            })?;
            return Ok(session);
        }
        #[derive(Deserialize)]
        struct ErrorCode {
            error: String,
        }
        let error: ErrorCode = body(response)?;
        match error.error.as_str() {
            "authorization_pending" => (),
            "slow_down" => interval = (interval + 5).min(120),
            "authorization_declined" | "access_denied" => bail!("Microsoft login was declined"),
            "expired_token" => bail!("login code expired; run login again"),
            _ => {
                bail!("Microsoft device login failed; check the application registration and retry")
            }
        }
    }
}

pub fn session() -> Result<Session> {
    let _guard = crate::storage::operation_lock(&state_root()?)?;
    let generation = match crate::storage::read_optional(&state_root()?, "generation")? {
        Some(value) => value,
        None => advance_generation()?,
    };
    let text = entry()?
        .get_password()
        .context("no readable Enderpin login; run enderpin login first")?;
    let mut stored: Credential = serde_json::from_str(&text)
        .map_err(|_| anyhow::anyhow!("invalid stored login; sign in again"))?;
    validate_client_id(&stored.client_id)?;
    token(&stored.refresh_token)?;
    let client = client()?;
    let tokens: Tokens = success(
        client
            .post(TOKEN)
            .form(&[
                ("client_id", stored.client_id.as_str()),
                ("grant_type", "refresh_token"),
                ("refresh_token", &stored.refresh_token),
                ("scope", SCOPE),
            ])
            .send()?,
        "Microsoft token refresh (sign in again if expired)",
    )?;
    token(&tokens.access_token)?;
    if let Some(refresh) = tokens.refresh_token {
        token(&refresh)?;
        stored.refresh_token = refresh;
        // Save rotation before downstream requests, which may fail independently.
        save(&stored)?;
    }
    let mut session = minecraft(&client, &tokens.access_token)?;
    session.generation = Some(generation);
    Ok(session)
}
pub fn logout() -> Result<()> {
    let _guard = crate::storage::operation_lock(&state_root()?)?;
    advance_generation()?;
    entry()?
        .delete_credential()
        .context("could not delete the Enderpin credential")
}

fn minecraft(client: &Client, access_token: &str) -> Result<Session> {
    #[derive(Deserialize)]
    #[serde(rename_all = "PascalCase")]
    struct Xbox {
        token: String,
        display_claims: Claims,
    }
    #[derive(Deserialize)]
    struct Claims {
        xui: Vec<Claim>,
    }
    #[derive(Deserialize)]
    struct Claim {
        uhs: String,
    }
    let xbox: Xbox = success(client.post("https://user.auth.xboxlive.com/user/authenticate").json(&json!({"Properties":{"AuthMethod":"RPS","SiteName":"user.auth.xboxlive.com","RpsTicket":format!("d={access_token}")},"RelyingParty":"http://auth.xboxlive.com","TokenType":"JWT"})).send()?, "Xbox authentication")?;
    token(&xbox.token)?;
    let xsts: Xbox = success(client.post("https://xsts.auth.xboxlive.com/xsts/authorize").json(&json!({"Properties":{"SandboxId":"RETAIL","UserTokens":[xbox.token]},"RelyingParty":"rp://api.minecraftservices.com/","TokenType":"JWT"})).send()?, "Xbox authorization")?;
    token(&xsts.token)?;
    let hash = &xsts
        .display_claims
        .xui
        .first()
        .context("Xbox identity is missing")?
        .uhs;
    ensure!(
        !hash.is_empty() && hash.len() <= 256 && hash.bytes().all(|b| b.is_ascii_alphanumeric()),
        "invalid Xbox identity"
    );
    #[derive(Deserialize)]
    struct MinecraftToken {
        access_token: String,
        expires_in: u64,
    }
    let minecraft: MinecraftToken = success(
        client
            .post("https://api.minecraftservices.com/authentication/login_with_xbox")
            .json(&json!({"identityToken":format!("XBL3.0 x={hash};{}", xsts.token)}))
            .send()?,
        "Minecraft authentication",
    )?;
    token(&minecraft.access_token)?;
    ensure!(
        (60..=172800).contains(&minecraft.expires_in),
        "invalid Minecraft token lifetime"
    );
    let expires_at = Instant::now() + Duration::from_secs(minecraft.expires_in);
    #[derive(Deserialize)]
    struct Entitlements {
        items: Vec<serde_json::Value>,
    }
    let entitlements: Entitlements = success(
        client
            .get("https://api.minecraftservices.com/entitlements/mcstore")
            .bearer_auth(&minecraft.access_token)
            .send()?,
        "Minecraft ownership",
    )?;
    ensure!(
        !entitlements.items.is_empty(),
        "this account has no Minecraft Java Edition entitlement"
    );
    #[derive(Deserialize)]
    struct Profile {
        id: String,
        name: String,
    }
    let profile: Profile = success(
        client
            .get("https://api.minecraftservices.com/minecraft/profile")
            .bearer_auth(&minecraft.access_token)
            .send()?,
        "Minecraft profile",
    )?;
    ensure!(
        profile.id.len() == 32
            && profile.id.bytes().all(|b| b.is_ascii_hexdigit())
            && (1..=16).contains(&profile.name.len())
            && profile
                .name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_'),
        "invalid Minecraft profile"
    );
    Ok(Session {
        name: profile.name,
        uuid: profile.id,
        access_token: minecraft.access_token,
        expires_at,
        generation: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn auth_inputs_are_bounded_and_secrets_have_no_debug_or_serialization() {
        assert!(validate_client_id("00000000-0000-0000-0000-000000000000").is_ok());
        assert!(validate_client_id("bad\nclient").is_err());
        assert!(token("secret\r\nheader").is_err());
        assert!(token(&"x".repeat(32769)).is_err());
    }
}
