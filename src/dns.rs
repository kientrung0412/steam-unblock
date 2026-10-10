//! DNS server cục bộ (UDP) chuyển mọi truy vấn sang DNS-over-HTTPS.
//! Đặt DNS hệ thống thành 127.0.0.1 để cả Steam client cũng dùng được.

use crate::doh::Doh;
use anyhow::Result;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::UdpSocket;

pub async fn run(bind: SocketAddr, doh: Arc<Doh>) -> Result<()> {
    let sock = Arc::new(UdpSocket::bind(bind).await.map_err(|e| {
        anyhow::anyhow!("không mở được {bind}: {e} (cổng 53 cần quyền admin/sudo)")
    })?);
    println!("DNS-over-HTTPS server đang chạy tại {bind}");
    println!("→ Đặt DNS của máy thành {} để dùng.", bind.ip());

    let mut buf = vec![0u8; 4096];
    loop {
        let (n, peer) = sock.recv_from(&mut buf).await?;
        let query = buf[..n].to_vec();
        let (sock, doh) = (sock.clone(), doh.clone());
        tokio::spawn(async move {
            match doh.forward_wire(&query).await {
                Ok(resp) => {
                    let _ = sock.send_to(&resp, peer).await;
                }
                Err(e) => eprintln!("[dns] lỗi DoH: {e}"),
            }
        });
    }
}
