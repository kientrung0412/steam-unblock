//! Proxy HTTP/HTTPS cục bộ:
//! - phân giải tên miền bằng DoH (né chặn DNS),
//! - chia nhỏ TLS ClientHello (né lọc SNI kiểu DPI),
//! - phục vụ file PAC tại /proxy.pac để trình duyệt chỉ đẩy Steam qua proxy.

use crate::doh::Doh;
use anyhow::{anyhow, bail, Result};
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::time::timeout;

pub const STEAM_DOMAINS: &[&str] = &[
    "steampowered.com",
    "steamcommunity.com",
    "steamstatic.com",
    "steamcontent.com",
    "steamserver.net",
    "steamusercontent.com",
    "steam-chat.com",
    "steamgames.com",
    "steamdeck.com",
    "valvesoftware.com",
    "s.team",
    "steamcdn-a.akamaihd.net",
];

#[derive(Clone)]
pub struct Config {
    pub port: u16,
    pub fragment: bool,
    pub verbose: bool,
    /// Số mảnh cắt tên miền SNI
    pub pieces: usize,
}

pub async fn run(bind: SocketAddr, cfg: Config, doh: Arc<Doh>) -> Result<()> {
    let listener = TcpListener::bind(bind).await?;
    println!("Proxy đang chạy tại http://{bind}");
    println!("PAC (chỉ Steam đi qua proxy): http://127.0.0.1:{}/proxy.pac", cfg.port);
    println!("Chia nhỏ ClientHello: {}", if cfg.fragment { "bật" } else { "tắt" });
    serve(listener, cfg, doh).await
}

/// Phục vụ proxy trên một listener đã mở sẵn (dùng nội bộ cho chế độ `hosts`).
pub async fn serve(listener: TcpListener, cfg: Config, doh: Arc<Doh>) -> Result<()> {
    loop {
        let (sock, peer) = listener.accept().await?;
        let (cfg, doh) = (cfg.clone(), doh.clone());
        tokio::spawn(async move {
            if let Err(e) = handle(sock, &cfg, &doh).await {
                eprintln!("[proxy] {peer}: {e}");
            }
        });
    }
}

async fn handle(mut client: TcpStream, cfg: &Config, doh: &Arc<Doh>) -> Result<()> {
    client.set_nodelay(true)?;

    // Đọc phần header của request đầu tiên.
    let mut buf = Vec::with_capacity(4096);
    let head_end = loop {
        let mut tmp = [0u8; 4096];
        let n = timeout(Duration::from_secs(15), client.read(&mut tmp)).await??;
        if n == 0 {
            return Ok(());
        }
        buf.extend_from_slice(&tmp[..n]);
        if let Some(p) = find(&buf, b"\r\n\r\n") {
            break p + 4;
        }
        if buf.len() > 64 * 1024 {
            bail!("header quá dài");
        }
    };
    let head = std::str::from_utf8(&buf[..head_end])?.to_string();
    let rest = buf[head_end..].to_vec();
    let mut parts = head.lines().next().unwrap_or("").split_whitespace();
    let (method, target, version) = (
        parts.next().unwrap_or(""),
        parts.next().unwrap_or(""),
        parts.next().unwrap_or("HTTP/1.1"),
    );

    if method.eq_ignore_ascii_case("CONNECT") {
        let (host, port) = split_host_port(target, 443)?;
        let t0 = Instant::now();
        let ips = resolve(doh, &host).await?;
        let t_dns = t0.elapsed();
        let mut up = dial(&host, &ips, port).await?;
        let t_conn = t0.elapsed();
        client.write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n").await?;

        // Gói đầu tiên của client thường là TLS ClientHello chứa SNI.
        let first = if !rest.is_empty() {
            rest
        } else {
            let mut tmp = vec![0u8; 16 * 1024];
            let n = client.read(&mut tmp).await?;
            tmp.truncate(n);
            tmp
        };
        if cfg.fragment && first.first() == Some(&0x16) {
            send_fragmented(&mut up, &first, &host, cfg.pieces).await?;
        } else {
            up.write_all(&first).await?;
        }
        if cfg.verbose {
            println!(
                "[{host}] dns={}ms connect={}ms hello-sent={}ms",
                t_dns.as_millis(),
                (t_conn - t_dns).as_millis(),
                (t0.elapsed() - t_conn).as_millis()
            );
        }
        let r = tokio::io::copy_bidirectional(&mut client, &mut up).await;
        if cfg.verbose {
            let ip = up.peer_addr().map(|a| a.ip().to_string()).unwrap_or_default();
            match r {
                Ok((tx, rx)) => println!("[{host} {ip}] đóng: gửi={tx}B nhận={rx}B sau {}ms", t0.elapsed().as_millis()),
                Err(e) => println!("[{host} {ip}] đóng lỗi: {e} sau {}ms", t0.elapsed().as_millis()),
            }
        }
        return Ok(());
    }

    if target.starts_with('/') {
        return serve_local(&mut client, target, cfg).await;
    }

    // Proxy HTTP thường: "GET http://host/path HTTP/1.1"
    let url = target
        .strip_prefix("http://")
        .ok_or_else(|| anyhow!("không hỗ trợ: {target}"))?;
    let (authority, path) = match url.find('/') {
        Some(i) => (&url[..i], &url[i..]),
        None => (url, "/"),
    };
    let (host, port) = split_host_port(authority, 80)?;
    let mut up = connect(doh, &host, port).await?;

    let mut out = format!("{method} {path} {version}\r\n");
    for line in head.lines().skip(1).filter(|l| !l.is_empty()) {
        let name = line.split(':').next().unwrap_or("").trim().to_ascii_lowercase();
        if name == "host" && cfg.fragment {
            // DPI so khớp chính xác "Host:" → đổi hoa/thường (server vẫn chấp nhận).
            out.push_str("hOSt:");
            out.push_str(&line[5..]);
            out.push_str("\r\n");
        } else if name != "proxy-connection" && name != "connection" && name != "keep-alive" {
            out.push_str(line);
            out.push_str("\r\n");
        }
    }
    // Mỗi kết nối chỉ phục vụ 1 request để đơn giản & đúng host.
    out.push_str("Connection: close\r\n\r\n");
    let mut data = out.into_bytes();
    data.extend_from_slice(&rest);
    if cfg.fragment {
        // Header Host cũng bị DPI đọc → cắt gói ở giữa tên miền.
        let split = find(&data, host.as_bytes()).map(|i| i + host.len() / 2).unwrap_or(1);
        send_chunks(&mut up, &[&data[..split], &data[split..]]).await?;
    } else {
        up.write_all(&data).await?;
    }
    tokio::io::copy_bidirectional(&mut client, &mut up).await.ok();
    Ok(())
}

async fn connect(doh: &Arc<Doh>, host: &str, port: u16) -> Result<TcpStream> {
    let ips = resolve(doh, host).await?;
    dial(host, &ips, port).await
}

pub(crate) async fn resolve(doh: &Arc<Doh>, host: &str) -> Result<Vec<std::net::IpAddr>> {
    match doh.resolve(host).await {
        Ok(ips) => Ok(ips),
        Err(e) => {
            eprintln!("[proxy] DoH lỗi cho {host} ({e}), dùng DNS hệ thống");
            // Bỏ IP loopback: ở chế độ `hosts` DNS hệ thống trả 127.0.0.1 → sẽ tự gọi lại chính mình.
            let ips: Vec<_> = tokio::net::lookup_host((host, 0))
                .await?
                .map(|a| a.ip())
                .filter(|ip| !ip.is_loopback())
                .collect();
            if ips.is_empty() {
                bail!("không phân giải được {host}");
            }
            Ok(ips)
        }
    }
}

pub(crate) async fn dial(host: &str, ips: &[std::net::IpAddr], port: u16) -> Result<TcpStream> {
    // Kiểu "Happy Eyeballs": IP đầu chưa xong sau 250ms thì thử song song IP tiếp theo,
    // tránh kẹt ~1s khi gói SYN bị rớt.
    let mut set = tokio::task::JoinSet::new();
    let mut pending = ips.iter().copied();
    let mut last = anyhow!("không có IP cho {host}");
    loop {
        if let Some(ip) = pending.next() {
            set.spawn(async move {
                let r = timeout(Duration::from_secs(8), TcpStream::connect((ip, port))).await;
                (ip, r)
            });
        }
        if set.is_empty() {
            return Err(last);
        }
        let next = if pending.len() > 0 {
            match timeout(Duration::from_millis(250), set.join_next()).await {
                Ok(r) => r,
                Err(_) => continue, // hết 250ms → mở thêm IP
            }
        } else {
            set.join_next().await
        };
        match next {
            Some(Ok((_, Ok(Ok(s))))) => {
                s.set_nodelay(true)?;
                return Ok(s);
            }
            Some(Ok((ip, Ok(Err(e))))) => last = anyhow!("{host} ({ip}): {e}"),
            Some(Ok((ip, Err(_)))) => last = anyhow!("{host} ({ip}): hết thời gian chờ"),
            _ => {}
        }
    }
}

/// Tách ClientHello thành nhiều bản ghi TLS, mỗi bản ghi đi một gói TCP riêng.
/// Tên miền SNI bị cắt vụn qua nhiều bản ghi để DPI không ghép lại kịp.
pub(crate) async fn send_fragmented(up: &mut TcpStream, data: &[u8], host: &str, pieces: usize) -> Result<()> {
    let rec_len = data.get(3..5).map(|b| u16::from_be_bytes([b[0], b[1]]) as usize);
    let sni = find(data, host.as_bytes());

    let chunks: Vec<Vec<u8>> = match (rec_len, sni) {
        // Cả bản ghi nằm gọn trong `data` → tách ở tầng bản ghi TLS.
        (Some(len), Some(at)) if data.len() >= 5 + len && at > 5 && at + host.len() <= 5 + len => {
            let payload = &data[5..5 + len];
            let start = at - 5;
            // Điểm cắt: ngay trước tên miền, rồi chia đều tên miền thành `pieces` phần.
            let mut cuts = vec![start.saturating_sub(1).max(1)];
            for k in 1..pieces {
                cuts.push(start + host.len() * k / pieces);
            }
            cuts.dedup();
            cuts.retain(|&c| c > 0 && c < payload.len());

            let rec = |body: &[u8]| {
                let mut r = vec![0x16, data[1], data[2]];
                r.extend_from_slice(&(body.len() as u16).to_be_bytes());
                r.extend_from_slice(body);
                r
            };
            let mut out = Vec::new();
            let mut prev = 0;
            for c in cuts {
                out.push(rec(&payload[prev..c]));
                prev = c;
            }
            let mut last = rec(&payload[prev..]);
            last.extend_from_slice(&data[5 + len..]);
            out.push(last);
            out
        }
        // Không thì chỉ cắt ở tầng TCP.
        _ => {
            let split = sni.map(|i| i + host.len() / 2).unwrap_or(data.len().min(6));
            vec![data[..split].to_vec(), data[split..].to_vec()]
        }
    };

    send_chunks(up, &chunks.iter().map(Vec::as_slice).collect::<Vec<_>>()).await
}

/// Gửi từng phần thành gói TCP riêng (đã bật TCP_NODELAY).
pub(crate) async fn send_chunks(up: &mut TcpStream, chunks: &[&[u8]]) -> Result<()> {
    for (i, c) in chunks.iter().enumerate() {
        up.write_all(c).await?;
        up.flush().await?;
        if i + 1 < chunks.len() {
            // Nhường 1 nhịp để các phần chắc chắn đi thành gói TCP riêng.
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    }
    Ok(())
}

async fn serve_local(client: &mut TcpStream, path: &str, cfg: &Config) -> Result<()> {
    let (ctype, body) = if path.starts_with("/proxy.pac") {
        ("application/x-ns-proxy-autoconfig", pac(cfg.port))
    } else {
        (
            "text/plain; charset=utf-8",
            format!(
                "steam-unblock đang chạy.\nPAC: http://127.0.0.1:{}/proxy.pac\n",
                cfg.port
            ),
        )
    };
    let resp = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    client.write_all(resp.as_bytes()).await?;
    Ok(())
}

pub fn pac(port: u16) -> String {
    let list = STEAM_DOMAINS
        .iter()
        .map(|d| format!("\"{d}\""))
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "function FindProxyForURL(url, host) {{\n  var d = [{list}];\n  for (var i = 0; i < d.length; i++) {{\n    if (host == d[i] || dnsDomainIs(host, \".\" + d[i])) return \"PROXY 127.0.0.1:{port}\";\n  }}\n  return \"DIRECT\";\n}}\n"
    )
}

fn split_host_port(s: &str, default: u16) -> Result<(String, u16)> {
    if let Some(r) = s.strip_prefix('[') {
        // IPv6: [::1]:443
        let end = r.find(']').ok_or_else(|| anyhow!("địa chỉ sai: {s}"))?;
        let port = r[end + 1..].strip_prefix(':').map(str::parse).transpose()?;
        return Ok((r[..end].to_string(), port.unwrap_or(default)));
    }
    match s.rsplit_once(':') {
        Some((h, p)) => Ok((h.to_string(), p.parse()?)),
        None => Ok((s.to_string(), default)),
    }
}

pub(crate) fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() {
        return None;
    }
    hay.windows(needle.len()).position(|w| w == needle)
}
