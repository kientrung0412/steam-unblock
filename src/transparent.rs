//! Chế độ `hosts`: proxy "trong suốt" cho các ứng dụng không dùng proxy hệ thống (Steam client).
//!
//! File hosts trỏ tên miền Steam về 127.0.0.1. Ứng dụng kết nối thẳng tới 127.0.0.1:443 (hoặc :80…),
//! ta đọc tên miền từ SNI (HTTPS) hoặc header Host (HTTP), phân giải bằng DoH (không qua file hosts),
//! rồi chuyển tiếp tới máy chủ thật với ClientHello đã chia nhỏ. TLS vẫn đầu-cuối, không cần chứng chỉ.

use crate::doh::Doh;
use crate::proxy::{self, Config};
use anyhow::{anyhow, bail, Result};
use std::collections::BTreeSet;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::time::timeout;

/// Tên miền cố định cần trỏ về proxy (file hosts không hỗ trợ wildcard).
pub const HOSTS: &[&str] = &[
    // Bộ cập nhật của Steam client ("Steam needs to be online to update")
    "client-update.steamstatic.com",
    "client-update.akamai.steamstatic.com",
    "client-update.fastly.steamstatic.com",
    "client-update.cloudflare.steamstatic.com",
    "media.steampowered.com",
    "test.steampowered.com",
    // API, đăng nhập, cửa hàng, cộng đồng
    "api.steampowered.com",
    "login.steampowered.com",
    "store.steampowered.com",
    "help.steampowered.com",
    "checkout.steampowered.com",
    "partner.steamgames.com",
    "steamcommunity.com",
    "www.steamcommunity.com",
    "steam-chat.com",
    "s.team",
    // Ảnh, CSS, JS
    "cdn.steamstatic.com",
    "community.steamstatic.com",
    "store.steamstatic.com",
    "shared.steamstatic.com",
    "avatars.steamstatic.com",
    "shared.fastly.steamstatic.com",
    "cdn.fastly.steamstatic.com",
    "store.fastly.steamstatic.com",
    "community.fastly.steamstatic.com",
    "avatars.fastly.steamstatic.com",
    "clan.fastly.steamstatic.com",
    "video.fastly.steamstatic.com",
    "shared.akamai.steamstatic.com",
    "cdn.akamai.steamstatic.com",
    "store.akamai.steamstatic.com",
    "community.akamai.steamstatic.com",
    "avatars.akamai.steamstatic.com",
    "clan.akamai.steamstatic.com",
    "video.akamai.steamstatic.com",
    "shared.cloudflare.steamstatic.com",
    "cdn.cloudflare.steamstatic.com",
    "store.cloudflare.steamstatic.com",
    "community.cloudflare.steamstatic.com",
    "avatars.cloudflare.steamstatic.com",
    "clan.cloudflare.steamstatic.com",
    "steamcdn-a.akamaihd.net",
    "steamuserimages-a.akamaihd.net",
    "steamcommunity-a.akamaihd.net",
    "steamstore-a.akamaihd.net",
];

pub struct Options {
    pub ip: IpAddr,
    pub cfg: Config,
    /// Tải thêm danh sách CM server / content server từ Steam API
    pub dynamic: bool,
}

pub async fn run(opts: Options, doh: Arc<Doh>) -> Result<()> {
    doh.prefetch();

    // Proxy CONNECT nội bộ (cổng ngẫu nhiên) để tự gọi Steam API qua đường đã vượt chặn.
    let inner = TcpListener::bind("127.0.0.1:0").await?;
    let inner_addr = inner.local_addr()?;
    tokio::spawn(proxy::serve(inner, opts.cfg.clone(), doh.clone()));

    let mut hosts: BTreeSet<String> = HOSTS.iter().map(|s| s.to_string()).collect();
    let mut ports: BTreeSet<u16> = [80, 443].into();
    if opts.dynamic {
        match fetch_dynamic(inner_addr).await {
            Ok((h, p)) => {
                println!("Lấy được {} tên miền CM/content server từ Steam API.", h.len());
                hosts.extend(h);
                ports.extend(p);
            }
            Err(e) => eprintln!("Không lấy được danh sách CM server ({e}), chỉ dùng danh sách cố định."),
        }
    }

    // Mở cổng trước, rồi mới sửa hosts: lỗi thì không để lại file hosts hỏng.
    let mut listeners = Vec::new();
    for port in &ports {
        let addr = SocketAddr::new(opts.ip, *port);
        match TcpListener::bind(addr).await {
            Ok(l) => listeners.push((l, *port)),
            Err(e) if *port == 80 || *port == 443 => {
                bail!("không mở được {addr}: {e}\n→ Cần chạy bằng Administrator/sudo, và không có chương trình khác (IIS, Skype, XAMPP…) chiếm cổng này. Có thể thử --ip 127.0.0.2")
            }
            Err(e) => eprintln!("Bỏ qua cổng {port}: {e}"),
        }
    }

    let hosts: Vec<String> = hosts.into_iter().collect();
    crate::hosts::install(opts.ip, &hosts)?;
    println!("Đã thêm {} tên miền Steam vào {}", hosts.len(), crate::hosts::path().display());
    println!(
        "Proxy trong suốt đang chạy tại {} cổng {}",
        opts.ip,
        ports.iter().map(u16::to_string).collect::<Vec<_>>().join(", ")
    );
    println!("→ Thoát Steam hoàn toàn (kể cả ở khay hệ thống) rồi mở lại. Nhấn Ctrl+C để dừng và gỡ file hosts.");

    let cfg = Arc::new(opts.cfg);
    for (l, port) in listeners {
        let (cfg, doh) = (cfg.clone(), doh.clone());
        tokio::spawn(async move {
            loop {
                let Ok((sock, peer)) = l.accept().await else { continue };
                let (cfg, doh) = (cfg.clone(), doh.clone());
                tokio::spawn(async move {
                    if let Err(e) = handle(sock, port, &cfg, &doh).await {
                        eprintln!("[hosts:{port}] {peer}: {e}");
                    }
                });
            }
        });
    }

    crate::shutdown().await;
    println!("\nĐang thoát...");
    crate::hosts::uninstall()?;
    println!("Đã gỡ tên miền Steam khỏi file hosts.");
    Ok(())
}

async fn handle(mut client: TcpStream, port: u16, cfg: &Config, doh: &Arc<Doh>) -> Result<()> {
    client.set_nodelay(true)?;
    let t0 = Instant::now();

    // Đọc đủ gói đầu: cả bản ghi TLS (ClientHello) hoặc cả phần header HTTP.
    let mut buf = Vec::with_capacity(4096);
    loop {
        let mut tmp = [0u8; 8192];
        let n = timeout(Duration::from_secs(15), client.read(&mut tmp)).await??;
        if n == 0 {
            return Ok(());
        }
        buf.extend_from_slice(&tmp[..n]);
        if buf[0] == 0x16 {
            if buf.len() >= 5 && buf.len() >= 5 + u16::from_be_bytes([buf[3], buf[4]]) as usize {
                break;
            }
        } else if proxy::find(&buf, b"\r\n\r\n").is_some() {
            break;
        }
        if buf.len() > 64 * 1024 {
            bail!("gói đầu quá dài");
        }
    }

    let tls = buf[0] == 0x16;
    let host = if tls { parse_sni(&buf) } else { parse_http_host(&buf) }
        .ok_or_else(|| anyhow!("không đọc được tên miền ({})", if tls { "SNI" } else { "Host" }))?;

    let ips = doh.resolve(&host).await?;
    let ips: Vec<_> = ips.into_iter().filter(|ip| !ip.is_loopback()).collect();
    let mut up = proxy::dial(&host, &ips, port).await?;

    if !cfg.fragment {
        up.write_all(&buf).await?;
    } else if tls {
        proxy::send_fragmented(&mut up, &buf, &host, cfg.pieces).await?;
    } else {
        // HTTP: đổi "Host:" → "hOSt:" và cắt gói giữa tên miền.
        if let Some(i) = find_ci(&buf, b"\r\nhost:") {
            buf[i + 2..i + 7].copy_from_slice(b"hOSt:");
        }
        let split = proxy::find(&buf, host.as_bytes()).map(|i| i + host.len() / 2).unwrap_or(1);
        proxy::send_chunks(&mut up, &[&buf[..split], &buf[split..]]).await?;
    }
    if cfg.verbose {
        println!("[{host}:{port}] kết nối sau {}ms", t0.elapsed().as_millis());
    }
    tokio::io::copy_bidirectional(&mut client, &mut up).await.ok();
    Ok(())
}

/// Lấy tên miền SNI trong TLS ClientHello.
pub fn parse_sni(d: &[u8]) -> Option<String> {
    let rd = |i: usize| d.get(i).copied().map(usize::from);
    let rd16 = |i: usize| Some(rd(i)? << 8 | rd(i + 1)?);
    if rd(0)? != 0x16 || rd(5)? != 0x01 {
        return None;
    }
    // 5 (record) + 4 (handshake) + 2 (version) + 32 (random)
    let mut p = 43;
    p += 1 + rd(p)?; // session id
    p += 2 + rd16(p)?; // cipher suites
    p += 1 + rd(p)?; // compression
    let ext_end = (p + 2 + rd16(p)?).min(d.len());
    p += 2;
    while p + 4 <= ext_end {
        let (ty, len) = (rd16(p)?, rd16(p + 2)?);
        p += 4;
        if ty == 0 {
            // server_name_list: len(2) type(1) name_len(2) name
            let name_len = rd16(p + 3)?;
            let name = d.get(p + 5..p + 5 + name_len)?;
            return std::str::from_utf8(name).ok().map(|s| s.to_ascii_lowercase());
        }
        p += len;
    }
    None
}

/// Lấy giá trị header Host (bỏ cổng).
pub fn parse_http_host(d: &[u8]) -> Option<String> {
    let head = std::str::from_utf8(&d[..proxy::find(d, b"\r\n\r\n")?]).ok()?;
    head.lines().skip(1).find_map(|l| {
        let (k, v) = l.split_once(':')?;
        if !k.trim().eq_ignore_ascii_case("host") {
            return None;
        }
        let v = v.trim();
        let h = if v.starts_with('[') { v } else { v.split(':').next()? };
        Some(h.to_ascii_lowercase())
    })
}

fn find_ci(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w.eq_ignore_ascii_case(needle))
}

/// Gọi Steam API (qua proxy nội bộ) lấy tên miền + cổng của CM server và content server.
async fn fetch_dynamic(inner: SocketAddr) -> Result<(Vec<String>, Vec<u16>)> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .proxy(reqwest::Proxy::all(format!("http://{inner}"))?)
        .build()?;
    let mut hosts = Vec::new();
    let mut ports = Vec::new();

    let mut add = |ep: &str| {
        let (h, p) = match ep.rsplit_once(':') {
            Some((h, p)) => (h, p.parse::<u16>().ok()),
            None => (ep, None),
        };
        if h.parse::<IpAddr>().is_err() && h.contains('.') {
            hosts.push(h.to_ascii_lowercase());
            if let Some(p) = p {
                ports.push(p);
            }
        }
    };

    let v: serde_json::Value = client
        .get("https://api.steampowered.com/ISteamDirectory/GetCMListForConnect/v1/?cellid=0&maxcount=100")
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    for s in v["response"]["serverlist"].as_array().into_iter().flatten() {
        if let Some(ep) = s["endpoint"].as_str() {
            add(ep);
        }
    }

    // Content server (tải game/cập nhật) — không bắt buộc.
    if let Ok(r) = client
        .get("https://api.steampowered.com/IContentServerDirectoryService/GetServersForSteamPipe/v1/?cell_id=0")
        .send()
        .await
    {
        if let Ok(v) = r.json::<serde_json::Value>().await {
            for s in v["response"]["servers"].as_array().into_iter().flatten() {
                for k in ["host", "vhost"] {
                    if let Some(h) = s[k].as_str() {
                        add(h);
                    }
                }
            }
        }
    }
    hosts.sort();
    hosts.dedup();
    if hosts.is_empty() {
        bail!("danh sách rỗng");
    }
    Ok((hosts, ports))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn http_host() {
        let req = b"GET /x HTTP/1.1\r\nHost: Media.SteamPowered.com:80\r\nA: b\r\n\r\n";
        assert_eq!(parse_http_host(req).as_deref(), Some("media.steampowered.com"));
    }

    #[test]
    fn sni() {
        // ClientHello thật (Python ssl) gửi tới client-update.akamai.steamstatic.com
        let d = include_bytes!("testdata/clienthello.bin");
        assert_eq!(parse_sni(d).as_deref(), Some("client-update.akamai.steamstatic.com"));
        assert_eq!(parse_sni(&d[..20]), None);
    }
}
