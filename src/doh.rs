//! Phân giải tên miền qua DNS-over-HTTPS để né DNS bị nhà mạng "đầu độc".

use anyhow::{anyhow, Result};
use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Endpoint JSON (dùng IP trực tiếp nên không cần DNS hệ thống).
const JSON_ENDPOINTS: &[&str] = &["https://1.1.1.1/dns-query", "https://8.8.8.8/resolve"];
/// Endpoint RFC 8484 (dns-message) cho chế độ DNS server.
const WIRE_ENDPOINTS: &[&str] = &["https://1.1.1.1/dns-query", "https://8.8.8.8/dns-query"];

/// Tên miền Steam hay dùng, được tra sẵn lúc khởi động.
const PREFETCH: &[&str] = &[
    "store.steampowered.com",
    "steamcommunity.com",
    "api.steampowered.com",
    "help.steampowered.com",
    "login.steampowered.com",
    "checkout.steampowered.com",
    "shared.fastly.steamstatic.com",
    "cdn.fastly.steamstatic.com",
    "store.fastly.steamstatic.com",
    "community.fastly.steamstatic.com",
    "avatars.fastly.steamstatic.com",
    "video.fastly.steamstatic.com",
    "shared.akamai.steamstatic.com",
    "cdn.akamai.steamstatic.com",
    "store.akamai.steamstatic.com",
    "community.akamai.steamstatic.com",
    "avatars.akamai.steamstatic.com",
    "community.cloudflare.steamstatic.com",
    "steamcdn-a.akamaihd.net",
];

pub struct Doh {
    client: reqwest::Client,
    cache: Mutex<HashMap<String, (Vec<IpAddr>, Instant)>>,
    /// Khoá theo tên miền: nhiều kết nối cùng lúc chỉ gửi 1 truy vấn DoH.
    inflight: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
}

impl Doh {
    pub fn new() -> Result<Self> {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            .no_proxy()
            .pool_idle_timeout(Duration::from_secs(300))
            .http2_keep_alive_interval(Duration::from_secs(30))
            .build()?;
        Ok(Self {
            client,
            cache: Mutex::new(HashMap::new()),
            inflight: Mutex::new(HashMap::new()),
        })
    }

    /// Tra sẵn các tên miền Steam (chạy nền) để lần truy cập đầu không phải chờ.
    pub fn prefetch(self: &Arc<Self>) {
        for host in PREFETCH {
            let me = self.clone();
            tokio::spawn(async move {
                let _ = me.resolve(host).await;
            });
        }
    }

    /// Trả về danh sách IPv4 của `host`, có cache theo TTL.
    /// Bản ghi hết hạn vẫn được dùng ngay, đồng thời làm mới ở nền.
    pub async fn resolve(self: &Arc<Self>, host: &str) -> Result<Vec<IpAddr>> {
        if let Ok(ip) = host.trim_matches(['[', ']']).parse::<IpAddr>() {
            return Ok(vec![ip]);
        }
        let key = host.to_ascii_lowercase();
        let cached = self.cache.lock().unwrap().get(&key).cloned();
        match cached {
            Some((ips, exp)) if exp > Instant::now() => Ok(ips),
            Some((ips, _)) => {
                let me = self.clone();
                tokio::spawn(async move {
                    let _ = me.refresh(&key).await;
                });
                Ok(ips)
            }
            None => self.refresh(&key).await,
        }
    }

    async fn refresh(&self, key: &str) -> Result<Vec<IpAddr>> {
        let lock = self.inflight.lock().unwrap().entry(key.to_string()).or_default().clone();
        let _guard = lock.lock().await;
        // Có thể một kết nối khác vừa tra xong trong lúc ta chờ khoá.
        if let Some((ips, exp)) = self.cache.lock().unwrap().get(key) {
            if *exp > Instant::now() {
                return Ok(ips.clone());
            }
        }

        let mut last_err = anyhow!("không có endpoint DoH nào");
        for ep in JSON_ENDPOINTS {
            match self.query_json(ep, key).await {
                Ok((ips, ttl)) if !ips.is_empty() => {
                    // TTL của CDN rất ngắn (20s) nhưng IP hiếm khi đổi → giữ tối thiểu 5 phút.
                    let exp = Instant::now() + Duration::from_secs(ttl.clamp(300, 3600));
                    self.cache.lock().unwrap().insert(key.to_string(), (ips.clone(), exp));
                    return Ok(ips);
                }
                Ok(_) => last_err = anyhow!("{ep}: không có bản ghi A cho {key}"),
                Err(e) => last_err = e,
            }
        }
        Err(last_err)
    }

    async fn query_json(&self, endpoint: &str, name: &str) -> Result<(Vec<IpAddr>, u64)> {
        let v: serde_json::Value = self
            .client
            .get(endpoint)
            .query(&[("name", name), ("type", "A")])
            .header("accept", "application/dns-json")
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;

        let mut ips = Vec::new();
        let mut ttl = u64::MAX;
        for ans in v["Answer"].as_array().into_iter().flatten() {
            if ans["type"].as_u64() == Some(1) {
                if let Some(ip) = ans["data"].as_str().and_then(|s| s.parse::<Ipv4Addr>().ok()) {
                    ips.push(IpAddr::V4(ip));
                    ttl = ttl.min(ans["TTL"].as_u64().unwrap_or(300));
                }
            }
        }
        Ok((ips, if ttl == u64::MAX { 300 } else { ttl }))
    }

    /// Chuyển tiếp nguyên gói DNS (wire format) qua DoH, trả về gói trả lời.
    pub async fn forward_wire(&self, query: &[u8]) -> Result<Vec<u8>> {
        let mut last_err = anyhow!("không có endpoint DoH nào");
        for ep in WIRE_ENDPOINTS {
            let res = self
                .client
                .post(*ep)
                .header("content-type", "application/dns-message")
                .header("accept", "application/dns-message")
                .body(query.to_vec())
                .send()
                .await
                .and_then(|r| r.error_for_status());
            match res {
                Ok(r) => return Ok(r.bytes().await?.to_vec()),
                Err(e) => last_err = e.into(),
            }
        }
        Err(last_err)
    }
}
