# steam-unblock

Công cụ dòng lệnh viết bằng Rust giúp truy cập Steam khi bị nhà mạng chặn. Chạy trên macOS, Linux và Windows.

## Cách hoạt động

| Kiểu chặn | Cách vượt |
|---|---|
| DNS bị "đầu độc" | Phân giải tên miền qua DNS-over-HTTPS (1.1.1.1, dự phòng 8.8.8.8) |
| Lọc SNI trên HTTPS (DPI) | Chia TLS ClientHello thành 2 bản ghi, gửi thành 2 gói TCP, điểm cắt nằm giữa tên miền |
| Lọc header `Host` trên HTTP | Đổi thành `hOSt:` và cắt gói ở giữa tên miền |

Không cần máy chủ trung gian hay VPN: dữ liệu vẫn đi thẳng tới Steam và vẫn được mã hoá TLS đầu-cuối.

## Build

```bash
cargo build --release
# file chạy: target/release/steam-unblock  (Windows: steam-unblock.exe)
```

Push lên GitHub thì workflow `.github/workflows/release.yml` sẽ build sẵn cho cả 3 hệ điều hành.

## Sử dụng

```bash
steam-unblock check          # kiểm tra Steam đang bị chặn kiểu gì
steam-unblock                # chạy proxy tại 127.0.0.1:8899
steam-unblock proxy --system # chạy proxy và tự cấu hình PAC cho hệ thống (Ctrl+C sẽ gỡ cấu hình)
```

### Trình duyệt / web Steam
Cách 1: dùng `steam-unblock proxy --system`.
Cách 2: tự đặt "Automatic proxy configuration URL" là `http://127.0.0.1:8899/proxy.pac`.
Khi dùng PAC thì chỉ các tên miền Steam đi qua proxy, các trang khác vẫn đi thẳng.

### Steam client (ứng dụng)
- Windows: Steam client đọc cấu hình proxy của hệ thống, nên `steam-unblock proxy --system` thường là đủ.
- Nếu nhà mạng chỉ chặn DNS: chạy `sudo steam-unblock dns` (Windows thì mở terminal bằng Administrator),
  rồi đặt DNS của máy thành `127.0.0.1`. Cách này áp dụng cho mọi ứng dụng.
  Lưu ý: chế độ `dns` **không** vượt được lọc SNI. Gặp trường hợp đó thì dùng `proxy`.

### Tuỳ chọn khác
```
proxy -p 9000           đổi cổng
proxy --host 0.0.0.0    chia sẻ proxy cho các máy khác trong mạng LAN
proxy --no-fragment     chỉ dùng DoH, không chia nhỏ gói
proxy -v                in thời gian dns/connect của từng kết nối (tìm chỗ chậm)
pac -p 8899             in nội dung file PAC
```
