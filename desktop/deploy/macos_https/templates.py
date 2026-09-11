"""Render an independent HTTPS listener without changing the HTTP server."""
import ipaddress
import re


def safe_path(value):
    text = str(value)
    if not text.startswith('/') or any(c in text for c in '\n\r\x00"$\\'):
        raise ValueError('configuration_path_invalid')
    return '"' + text + '"'


def validate_network(address, hostname, subnet):
    ip = ipaddress.IPv4Address(address)
    network = ipaddress.IPv4Network(subnet, strict=True)
    private = any(ip in ipaddress.IPv4Network(cidr) for cidr in
                  ['10.0.0.0/8', '172.16.0.0/12', '192.168.0.0/16'])
    if not private or ip not in network or network.prefixlen < 24:
        raise ValueError('current_private_lan_required')
    if not re.fullmatch(r'[A-Za-z0-9](?:[A-Za-z0-9-]{0,61}[A-Za-z0-9])?\.local', hostname):
        raise ValueError('local_hostname_required')


def nginx_config(state, ui_root, mime_types, address, hostname, subnet, port=8443):
    validate_network(address, hostname, subnet)
    if not isinstance(port, int) or not 1024 <= port <= 65535:
        raise ValueError("unprivileged_https_port_required")
    _, ui, mime = map(safe_path, [state, ui_root, mime_types])
    def file(name):
        return safe_path(state / name)
    return f'''worker_processes 1;
pid {file('nginx.pid')};
error_log {file('error.log')} warn;
events {{ worker_connections 256; }}
http {{
    include {mime};
    default_type application/octet-stream;
    access_log off;
    server_tokens off;
    sendfile on;
    client_body_temp_path {file('tmp/client')};
    proxy_temp_path {file('tmp/proxy')};
    fastcgi_temp_path {file('tmp/fastcgi')};
    uwsgi_temp_path {file('tmp/uwsgi')};
    scgi_temp_path {file('tmp/scgi')};
    map $host $desktop_host_allowed {{
        default 0;
        {address} 1;
        {hostname} 1;
        127.0.0.1 1;
        "[::1]" 1;
    }}
    server {{
        listen {address}:{port} ssl;
        listen 127.0.0.1:{port} ssl;
        listen [::1]:{port} ssl;
        server_name {hostname} {address};
        if ($desktop_host_allowed = 0) {{ return 400; }}
        allow {subnet};
        allow 127.0.0.1;
        allow ::1;
        deny all;
        ssl_certificate {file('server.crt')};
        ssl_certificate_key {file('server.key')};
        ssl_protocols TLSv1.2 TLSv1.3;
        ssl_ciphers ECDHE-RSA-AES128-GCM-SHA256:ECDHE-RSA-AES256-GCM-SHA384:ECDHE-RSA-CHACHA20-POLY1305;
        ssl_session_tickets off;
        root {ui};
        index index.html;
        location ~ ^/(v1|webd)/ {{
            proxy_pass http://127.0.0.1:8788;
            proxy_http_version 1.1;
            proxy_set_header Host $http_host;
            proxy_set_header X-Real-IP $remote_addr;
            proxy_set_header X-Forwarded-For $remote_addr;
            proxy_set_header X-Forwarded-Proto https;
            proxy_set_header Connection "";
            proxy_buffering off;
            proxy_read_timeout 3600s;
        }}
        location = /index.html {{
            add_header Cache-Control "no-store, no-cache, must-revalidate" always;
            add_header Pragma "no-cache" always;
            expires -1;
        }}
        location / {{ try_files $uri $uri/ /index.html; }}
    }}
}}
'''
