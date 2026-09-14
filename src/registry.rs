use std::{
    collections::BTreeMap,
    io::{Read, Write},
    net::{IpAddr, SocketAddr, ToSocketAddrs},
    sync::{Mutex, OnceLock},
    time::Duration,
};

use anyhow::{Context, Result, bail, ensure};
use reqwest::{Url, blocking::Response, redirect::Policy};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use sha2::{Digest, Sha512};

use crate::model::{Kind, Side, https_url, identifier, validate_hash};

pub const MAX_FILE_SIZE: u64 = 1024 * 1024 * 1024;
const MAX_JSON_SIZE: u64 = 32 * 1024 * 1024;
const API: &str = "https://api.modrinth.com/v2";

type Origin = (String, u16, Vec<SocketAddr>);
static CONNECTIONS: OnceLock<Mutex<BTreeMap<Origin, reqwest::blocking::Client>>> = OnceLock::new();

fn pinned_client(
    host: &str,
    port: u16,
    addresses: &[SocketAddr],
) -> Result<reqwest::blocking::Client> {
    let key = (host.to_owned(), port, addresses.to_vec());
    let mut clients = CONNECTIONS
        .get_or_init(Mutex::default)
        .lock()
        .map_err(|_| anyhow::anyhow!("connection cache is unavailable"))?;
    if let Some(client) = clients.get(&key) {
        return Ok(client.clone());
    }
    let client = reqwest::blocking::Client::builder()
        .redirect(Policy::none())
        .no_proxy()
        .user_agent(concat!(
            "enderpin/",
            env!("CARGO_PKG_VERSION"),
            " (Minecraft workspace manager)"
        ))
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(180))
        .resolve_to_addrs(host, addresses)
        .build()?;
    // ponytail: bounded origin cache; clearing at 64 entries avoids an LRU dependency.
    if clients.len() >= 64 {
        clients.clear();
    }
    clients.insert(key, client.clone());
    Ok(client)
}

/// HTTPS transport validates and pins DNS results for every redirect hop.
/// It never forwards credentials or records temporary signed redirect URLs.
pub fn response(raw: &str) -> Result<Response> {
    let mut url = https_url(raw)?;
    for _ in 0..10 {
        let host = url
            .host_str()
            .context("URL has no host")?
            .trim_matches(['[', ']']);
        let port = url.port_or_known_default().context("URL has no port")?;
        let mut addresses: Vec<SocketAddr> = (host, port)
            .to_socket_addrs()
            .context("DNS lookup failed")?
            .collect();
        ensure!(
            !addresses.is_empty() && addresses.iter().all(|a| public_address(a.ip())),
            "download host must resolve only to public addresses"
        );
        addresses.sort_unstable();
        addresses.dedup();
        let client = pinned_client(host, port, &addresses)?;
        let reply = client
            .get(url.clone())
            .send()
            .map_err(|e| anyhow::anyhow!("HTTPS request failed: {}", e.without_url()))?;
        if reply.status().is_redirection() {
            let location = reply
                .headers()
                .get(reqwest::header::LOCATION)
                .context("redirect has no Location")?
                .to_str()?;
            url = https_url(url.join(location)?.as_str())?;
            continue;
        }
        ensure!(
            reply.status().is_success(),
            "download returned HTTP {}",
            reply.status()
        );
        return Ok(reply);
    }
    bail!("too many HTTPS redirects")
}

pub fn public_address(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            let [a, b, c, _] = ip.octets();
            !(ip.is_private()
                || ip.is_loopback()
                || ip.is_link_local()
                || ip.is_multicast()
                || ip.is_unspecified()
                || ip.is_broadcast()
                || ip.is_documentation()
                || a == 0
                || a >= 240
                || (a == 100 && (64..=127).contains(&b))
                || (a == 198 && (b == 18 || b == 19))
                || (a == 192 && b == 0 && c == 0))
        }
        IpAddr::V6(ip) => {
            if let Some(v4) = ip.to_ipv4_mapped() {
                return public_address(IpAddr::V4(v4));
            }
            let seg = ip.segments();
            // Only global unicast, excluding documentation and transition mechanisms.
            (seg[0] & 0xe000) == 0x2000
                && seg[0] != 0x2002
                && !(seg[0] == 0x2001 && (seg[1] < 0x0200 || seg[1] == 0x0db8))
        }
    }
}

pub fn json<T: DeserializeOwned>(url: &Url) -> Result<T> {
    let mut data = vec![];
    response(url.as_str())?
        .take(MAX_JSON_SIZE + 1)
        .read_to_end(&mut data)?;
    ensure!(
        data.len() as u64 <= MAX_JSON_SIZE,
        "API response is too large"
    );
    serde_json::from_slice(&data).context("invalid API JSON")
}

pub fn download(raw: &str, output: &mut impl Write) -> Result<(String, u64)> {
    let mut reply = response(raw)?;
    ensure!(
        reply.content_length().unwrap_or(0) <= MAX_FILE_SIZE,
        "download is too large"
    );
    let mut digest = Sha512::new();
    let mut size = 0;
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let n = reply.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        size += n as u64;
        ensure!(size <= MAX_FILE_SIZE, "download exceeds 1 GiB limit");
        digest.update(&buffer[..n]);
        output.write_all(&buffer[..n])?;
    }
    ensure!(size > 0, "download is empty");
    Ok((format!("{:x}", digest.finalize()), size))
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Project {
    pub id: String,
    pub slug: String,
    pub title: String,
    pub description: String,
    pub project_type: String,
    #[serde(default)]
    pub client_side: String,
    #[serde(default)]
    pub server_side: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Version {
    pub id: String,
    pub project_id: String,
    pub version_number: String,
    pub version_type: String,
    pub date_published: String,
    pub game_versions: Vec<String>,
    pub loaders: Vec<String>,
    #[serde(default)]
    pub environment: String,
    #[serde(default)]
    pub dependencies: Vec<ApiDependency>,
    pub files: Vec<ApiFile>,
}

impl Version {
    pub fn compatible(&self, minecraft: &str, loader: &str, kind: Kind) -> bool {
        self.game_versions.iter().any(|v| v == minecraft)
            && match kind {
                Kind::Mod => self.loaders.iter().any(|l| l == loader),
                Kind::Plugin => {
                    self.loaders
                        .iter()
                        .any(|l| ["paper", "spigot", "bukkit"].contains(&l.as_str()))
                        && loader == "paper"
                }
                Kind::Resourcepack => self.loaders.iter().any(|l| l == "minecraft"),
                Kind::Shader => self
                    .loaders
                    .iter()
                    .any(|l| ["iris", "optifine", "canvas", "vanilla"].contains(&l.as_str())),
            }
    }
    pub fn default_enabled(&self, project: &Project, side: Side, kind: Kind) -> bool {
        if kind != Kind::Mod {
            return match kind {
                Kind::Plugin => side == Side::Server,
                _ => side == Side::Client,
            };
        }
        match self.environment.as_str() {
            "client_only" | "client_only_server_optional" | "singleplayer_only" => {
                side == Side::Client
            }
            "server_only" | "server_only_client_optional" | "dedicated_server_only" => {
                side == Side::Server
            }
            "client_and_server" | "client_or_server" | "client_or_server_prefers_both" => true,
            _ => match side {
                Side::Client => project.client_side != "unsupported",
                Side::Server => project.server_side != "unsupported",
            },
        }
    }
    pub fn file(&self, kind: Kind, requested: Option<&str>) -> Result<&ApiFile> {
        let eligible: Vec<_> = self
            .files
            .iter()
            .filter(|f| {
                f.filename
                    .to_ascii_lowercase()
                    .ends_with(&format!(".{}", kind.extension()))
            })
            .collect();
        let selected = if let Some(name) = requested {
            eligible.iter().copied().find(|f| f.filename == name)
        } else {
            let primaries: Vec<_> = eligible.iter().copied().filter(|f| f.primary).collect();
            if primaries.len() == 1 {
                primaries.first().copied()
            } else if eligible.len() == 1 {
                eligible.first().copied()
            } else {
                None
            }
        }
        .context("no unambiguous artifact; specify filename in the package declaration")?;
        crate::model::filename(&selected.filename, kind)?;
        https_url(&selected.url)?;
        validate_hash(
            selected
                .hashes
                .get("sha512")
                .context("Modrinth artifact has no SHA-512")?,
        )?;
        ensure!(
            selected.size > 0 && selected.size <= MAX_FILE_SIZE,
            "invalid artifact size"
        );
        Ok(selected)
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ApiDependency {
    pub version_id: Option<String>,
    pub project_id: Option<String>,
    pub dependency_type: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ApiFile {
    pub hashes: BTreeMap<String, String>,
    pub url: String,
    pub filename: String,
    pub primary: bool,
    pub size: u64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SearchHit {
    pub project_id: String,
    pub slug: String,
    pub title: String,
    pub description: String,
    pub project_type: String,
    pub downloads: u64,
}
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SearchResults {
    pub hits: Vec<SearchHit>,
    pub total_hits: u64,
    pub offset: u64,
}

#[derive(Default)]
pub struct Modrinth {
    pub(crate) projects: BTreeMap<String, Project>,
    pub(crate) versions: BTreeMap<String, Version>,
    pub(crate) project_versions: BTreeMap<String, Vec<Version>>,
}

impl Modrinth {
    pub fn project(&mut self, id: &str) -> Result<Project> {
        identifier(id)?;
        if let Some(project) = self.projects.get(id) {
            return Ok(project.clone());
        }
        let project: Project = json(&Url::parse(&format!("{API}/project/{id}"))?)?;
        identifier(&project.id)?;
        identifier(&project.slug)?;
        ensure!(
            project.id == id || project.slug == id,
            "Modrinth project identity mismatch"
        );
        self.projects.insert(id.into(), project.clone());
        self.projects.insert(project.id.clone(), project.clone());
        Ok(project)
    }
    pub fn version(&mut self, id: &str) -> Result<Version> {
        identifier(id)?;
        if let Some(version) = self.versions.get(id) {
            return Ok(version.clone());
        }
        let version: Version = json(&Url::parse(&format!("{API}/version/{id}"))?)?;
        ensure!(version.id == id, "Modrinth version identity mismatch");
        self.versions.insert(id.into(), version.clone());
        Ok(version)
    }
    pub fn versions(&mut self, project: &str) -> Result<Vec<Version>> {
        identifier(project)?;
        if let Some(versions) = self.project_versions.get(project) {
            return Ok(versions.clone());
        }
        let mut versions: Vec<Version> =
            json(&Url::parse(&format!("{API}/project/{project}/version"))?)?;
        ensure!(versions.len() <= 10000, "too many project versions");
        ensure!(
            versions.iter().all(|v| v.project_id == project),
            "project version identity mismatch"
        );
        versions.sort_by(|a, b| {
            b.date_published
                .cmp(&a.date_published)
                .then(a.id.cmp(&b.id))
        });
        for v in &versions {
            self.versions.insert(v.id.clone(), v.clone());
        }
        self.project_versions
            .insert(project.into(), versions.clone());
        Ok(versions)
    }
    pub fn search(
        &self,
        query: &str,
        minecraft: &str,
        loader: &str,
        kind: Kind,
        offset: u64,
    ) -> Result<SearchResults> {
        ensure!(query.len() <= 400, "search query is too long");
        let mut facets = vec![
            vec![format!("project_type:{}", kind.api_type())],
            vec![format!("versions:{minecraft}")],
        ];
        if kind == Kind::Mod {
            facets.push(vec![format!("categories:{loader}")]);
        }
        if kind == Kind::Plugin {
            facets.push(vec![
                "categories:paper".into(),
                "categories:spigot".into(),
                "categories:bukkit".into(),
            ]);
        }
        let mut url = Url::parse(&format!("{API}/search"))?;
        url.query_pairs_mut()
            .append_pair("query", query)
            .append_pair("facets", &serde_json::to_string(&facets)?)
            .append_pair("limit", "20")
            .append_pair("offset", &offset.to_string());
        json(&url)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reject_local_and_transition_addresses() -> Result<()> {
        for ip in [
            "127.0.0.1",
            "10.2.3.4",
            "169.254.169.254",
            "100.64.0.1",
            "192.168.1.1",
            "::1",
            "::ffff:127.0.0.1",
            "fc00::1",
            "2001:db8::1",
            "2002:7f00:1::",
        ] {
            assert!(!public_address(ip.parse()?));
        }
        for ip in ["1.1.1.1", "8.8.8.8", "2606:4700:4700::1111"] {
            assert!(public_address(ip.parse()?));
        }
        Ok(())
    }
}
