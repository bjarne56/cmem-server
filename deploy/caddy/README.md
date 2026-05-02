# Caddy 反代

`Caddyfile.example` 是把 cmem-server 暴露到公网的最简模板。Caddy 会自动:

- 申请并续签 Let's Encrypt TLS 证书
- 强制 HTTPS、启用 HTTP/2 与 HTTP/3
- 转发真实 IP 给 cmem-server(`X-Forwarded-For`)
- 给 `/admin/*` 加 HSTS / X-Frame-Options 等安全头

## 部署步骤

```bash
# 1. 装 Caddy(Debian/Ubuntu)
sudo apt install -y debian-keyring debian-archive-keyring apt-transport-https
curl -1sLf 'https://dl.cloudsmith.io/public/caddy/stable/gpg.key' \
    | sudo gpg --dearmor -o /usr/share/keyrings/caddy-stable-archive-keyring.gpg
curl -1sLf 'https://dl.cloudsmith.io/public/caddy/stable/debian.deb.txt' \
    | sudo tee /etc/apt/sources.list.d/caddy-stable.list
sudo apt update && sudo apt install -y caddy

# 2. 把模板放到 /etc/caddy/Caddyfile.d/cmem.conf
sudo mkdir -p /etc/caddy/Caddyfile.d
sudo install -m 0644 Caddyfile.example /etc/caddy/Caddyfile.d/cmem.conf
sudo sed -i 's/{$DOMAIN}/cmem.example.com/' /etc/caddy/Caddyfile.d/cmem.conf

# 3. 在主 Caddyfile 末尾追加 import(若未配置)
echo 'import /etc/caddy/Caddyfile.d/*.conf' | sudo tee -a /etc/caddy/Caddyfile

# 4. reload
sudo systemctl reload caddy
```

`install-server.sh --domain cmem.example.com` 会自动跑完上面所有步骤。
