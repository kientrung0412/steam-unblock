//! Chẩn đoán: so sánh DNS hệ thống với DoH và thử tải trang Steam.

use crate::doh::Doh;
use std::net::SocketAddr;
use std::time::Duration;

const HOSTS: &[&str] = &["store.steampowered.com", "steamcommunity.com", "api.steampowered.com"];

pub async fn run(doh: &std::sync::Arc<Doh>) {
    for host in HOSTS {
        println!("== {host}");
        let sys: Vec<_> = match tokio::net::lookup_host((*host, 443)).await {
            Ok(a) => a.map(|a| a.ip().to_string()).collect(),
            Err(e) => vec![format!("lỗi: {e}")],
        };
        println!("  DNS hệ thống : {}", sys.join(", "));

        let ips = match doh.resolve(host).await {
            Ok(ips) => ips,
            Err(e) => {
                println!("  DoH          : lỗi: {e}");
                continue;
            }
        };
        println!("  DoH          : {}", ips.iter().map(|i| i.to_string()).collect::<Vec<_>>().join(", "));

        // Tải trực tiếp bằng IP từ DoH (không chia nhỏ ClientHello).
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(8))
            .no_proxy()
            .resolve(host, SocketAddr::new(ips[0], 443))
            .build()
            .unwrap();
        match client.get(format!("https://{host}/")).send().await {
            Ok(r) => println!("  HTTPS (DoH)  : OK {} → chặn DNS, dùng `dns` hoặc `proxy` đều được", r.status()),
            Err(e) => println!("  HTTPS (DoH)  : thất bại ({e}) → có thể bị lọc SNI, dùng `proxy` (chia nhỏ ClientHello)"),
        }
    }
}
