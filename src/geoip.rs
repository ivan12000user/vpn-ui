use crate::vpn::common::InterfaceStatus;
use futures_util::{StreamExt, stream};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::HashMap,
    net::IpAddr,
    path::{Path, PathBuf},
    sync::Arc,
};
use tokio::{
    fs,
    sync::{Mutex, RwLock},
};
use tracing::warn;

#[derive(Clone, Debug)]
pub struct GeoInfo {
    pub provider: Option<String>,
    pub location: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct CacheEntry {
    updated_at: u64,
    success: bool,
    provider: Option<String>,
    location: Option<String>,
}

#[derive(Clone)]
pub struct GeoIpService {
    client: reqwest::Client,
    cache: Arc<RwLock<HashMap<String, CacheEntry>>>,
    persist_lock: Arc<Mutex<()>>,
    path: PathBuf,
    ttl_secs: u64,
}

impl GeoIpService {
    pub async fn new(path: impl Into<PathBuf>, ttl_secs: u64) -> Self {
        let path = path.into();

        let cache = match fs::read(&path).await {
            Ok(data) => {
                serde_json::from_slice::<HashMap<String, CacheEntry>>(&data).unwrap_or_default()
            }
            Err(_) => HashMap::new(),
        };

        let client = reqwest::Client::builder()
            .user_agent(concat!("vpn-ui/", env!("CARGO_PKG_VERSION")))
            .timeout(std::time::Duration::from_secs(3))
            .build()
            .expect("failed to build HTTP client");

        Self {
            client,
            cache: Arc::new(RwLock::new(cache)),
            persist_lock: Arc::new(Mutex::new(())),
            path,
            ttl_secs,
        }
    }

    pub async fn lookup(&self, ip: &str) -> Option<GeoInfo> {
        let now = crate::vpn::common::now_epoch();

        {
            let cache = self.cache.read().await;

            if let Some(entry) = cache.get(ip) {
                let ttl = if entry.success { self.ttl_secs } else { 3600 };

                if now.saturating_sub(entry.updated_at) < ttl {
                    return entry_to_info(entry);
                }
            }
        }

        let fetched = self.fetch(ip).await;

        let entry = match fetched {
            Ok(info) => CacheEntry {
                updated_at: now,
                success: true,
                provider: info.provider.clone(),
                location: info.location.clone(),
            },

            Err(err) => {
                warn!("GeoIP lookup failed for {}: {}", ip, err);

                CacheEntry {
                    updated_at: now,
                    success: false,
                    provider: None,
                    location: None,
                }
            }
        };

        {
            let mut cache = self.cache.write().await;
            cache.insert(ip.to_string(), entry.clone());
        }

        self.persist().await;

        entry_to_info(&entry)
    }

    async fn fetch(&self, ip: &str) -> Result<GeoInfo, String> {
        let parsed: IpAddr = ip.parse().map_err(|_| "invalid endpoint IP".to_string())?;

        if !is_public_ip(parsed) {
            return Err("endpoint is not public".to_string());
        }

        let url = format!("https://ipwho.is/{ip}");

        let response = self
            .client
            .get(url)
            .send()
            .await
            .map_err(|e| e.to_string())?;

        if !response.status().is_success() {
            return Err(format!("HTTP {}", response.status()));
        }

        let value: Value = response.json().await.map_err(|e| e.to_string())?;

        if value.get("success").and_then(Value::as_bool) == Some(false) {
            return Err(value
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("GeoIP service rejected lookup")
                .to_string());
        }

        let city = string_field(&value, "city");
        let country = string_field(&value, "country");

        let isp = value
            .pointer("/connection/isp")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(ToString::to_string);

        let asn = value
            .pointer("/connection/asn")
            .and_then(value_to_string)
            .map(|v| {
                if v.to_ascii_uppercase().starts_with("AS") {
                    v
                } else {
                    format!("AS{v}")
                }
            });

        let provider = join_nonempty(&[isp, asn], " · ");
        let location = join_nonempty(&[city, country], ", ");

        if provider.is_none() && location.is_none() {
            return Err("GeoIP returned no useful fields".to_string());
        }

        Ok(GeoInfo { provider, location })
    }

    async fn persist(&self) {
        let _guard = self.persist_lock.lock().await;

        let data = {
            let cache = self.cache.read().await;

            match serde_json::to_vec_pretty(&*cache) {
                Ok(data) => data,
                Err(err) => {
                    warn!("cannot serialize GeoIP cache: {}", err);
                    return;
                }
            }
        };

        if let Some(parent) = self.path.parent()
            && let Err(err) = fs::create_dir_all(parent).await
        {
            warn!("cannot create GeoIP cache directory: {}", err);
            return;
        }

        let tmp = temporary_path(&self.path);

        if let Err(err) = fs::write(&tmp, data).await {
            warn!("cannot write GeoIP cache: {}", err);
            return;
        }

        if let Err(err) = fs::rename(&tmp, &self.path).await {
            warn!("cannot install GeoIP cache: {}", err);
        }
    }
}

pub async fn enrich_status(service: &GeoIpService, status: &mut InterfaceStatus) {
    let jobs: Vec<(usize, String)> = status
        .peers
        .iter()
        .enumerate()
        .filter_map(|(index, peer)| {
            peer.endpoint
                .as_deref()
                .and_then(endpoint_ip)
                .map(|ip| (index, ip))
        })
        .collect();

    let service = service.clone();

    let results = stream::iter(jobs.into_iter().map(move |(index, ip)| {
        let service = service.clone();

        async move {
            let info = service.lookup(&ip).await;
            (index, info)
        }
    }))
    .buffer_unordered(4)
    .collect::<Vec<_>>()
    .await;

    for (index, info) in results {
        let Some(info) = info else {
            continue;
        };

        if let Some(peer) = status.peers.get_mut(index) {
            peer.geo_provider = info.provider;
            peer.geo_location = info.location;
        }
    }
}

fn endpoint_ip(endpoint: &str) -> Option<String> {
    let endpoint = endpoint.trim();

    let host = if endpoint.starts_with('[') {
        let end = endpoint.find(']')?;
        &endpoint[1..end]
    } else {
        endpoint
            .rsplit_once(':')
            .map(|(host, _)| host)
            .unwrap_or(endpoint)
    };

    host.parse::<IpAddr>().ok().map(|ip| ip.to_string())
}

fn is_public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            !ip.is_private()
                && !ip.is_loopback()
                && !ip.is_link_local()
                && !ip.is_multicast()
                && !ip.is_unspecified()
        }

        IpAddr::V6(ip) => !ip.is_loopback() && !ip.is_multicast() && !ip.is_unspecified(),
    }
}

fn string_field(value: &Value, name: &str) -> Option<String> {
    value
        .get(name)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToString::to_string)
}

fn value_to_string(value: &Value) -> Option<String> {
    if let Some(value) = value.as_str() {
        return Some(value.to_string());
    }

    if let Some(value) = value.as_u64() {
        return Some(value.to_string());
    }

    None
}

fn join_nonempty(values: &[Option<String>], separator: &str) -> Option<String> {
    let values: Vec<String> = values
        .iter()
        .filter_map(Clone::clone)
        .filter(|value| !value.trim().is_empty())
        .collect();

    if values.is_empty() {
        None
    } else {
        Some(values.join(separator))
    }
}

fn entry_to_info(entry: &CacheEntry) -> Option<GeoInfo> {
    if !entry.success {
        return None;
    }

    Some(GeoInfo {
        provider: entry.provider.clone(),
        location: entry.location.clone(),
    })
}

fn temporary_path(path: &Path) -> PathBuf {
    let mut value = path.as_os_str().to_os_string();
    value.push(".tmp");
    PathBuf::from(value)
}
