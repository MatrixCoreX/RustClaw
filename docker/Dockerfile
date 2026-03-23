ARG DEBIAN_MIRROR=http://mirrors.tuna.tsinghua.edu.cn/debian
ARG DEBIAN_SECURITY_MIRROR=http://mirrors.tuna.tsinghua.edu.cn/debian-security

FROM rust:1.90-bookworm AS builder
ARG DEBIAN_MIRROR
ARG DEBIAN_SECURITY_MIRROR

ENV DEBIAN_FRONTEND=noninteractive

RUN sed -i "s|http://deb.debian.org/debian|${DEBIAN_MIRROR}|g; s|http://deb.debian.org/debian-security|${DEBIAN_SECURITY_MIRROR}|g" /etc/apt/sources.list.d/debian.sources \
 && apt-get update \
 && apt-get install -y --no-install-recommends \
    bash \
    build-essential \
    clang \
    libclang-dev \
    pkg-config \
    protobuf-compiler \
    python3 \
 && rm -rf /var/lib/apt/lists/*

WORKDIR /build

COPY . /build

RUN cargo build --release \
    -p clawd \
    -p skill-runner \
    -p telegramd \
    -p wechatd \
    -p whatsappd \
    -p whatsapp_webd \
    -p archive-basic-skill \
    -p audio-synthesize-skill \
    -p audio-transcribe-skill \
    -p browser-web-skill \
    -p chat-skill \
    -p config-guard-skill \
    -p crypto-skill \
    -p db-basic-skill \
    -p docker-basic-skill \
    -p fs-search-skill \
    -p git-basic-skill \
    -p health-check-skill \
    -p http-basic-skill \
    -p image-edit-skill \
    -p image-generate-skill \
    -p image-vision-skill \
    -p install-module-skill \
    -p log-analyze-skill \
    -p package-manager-skill \
    -p process-basic-skill \
    -p rss-fetch-skill \
    -p service-control-skill \
    -p stock-skill \
    -p system-basic-skill \
    -p web-search-extract-skill \
    -p x-skill

FROM debian:bookworm-slim
ARG DEBIAN_MIRROR
ARG DEBIAN_SECURITY_MIRROR

ENV DEBIAN_FRONTEND=noninteractive

RUN sed -i "s|http://deb.debian.org/debian|${DEBIAN_MIRROR}|g; s|http://deb.debian.org/debian-security|${DEBIAN_SECURITY_MIRROR}|g" /etc/apt/sources.list.d/debian.sources \
 && apt-get update \
 && apt-get install -y --no-install-recommends \
    bash \
    ca-certificates \
    curl \
    jq \
    libsqlite3-0 \
    libssl3 \
    libstdc++6 \
    openssl \
    procps \
    python3 \
    python3-pip \
 && rm -rf /var/lib/apt/lists/*

WORKDIR /app

RUN mkdir -p /app/target/release /app/UI/dist /app/docker/config

COPY --from=builder /build/target/release/clawd /app/target/release/
COPY --from=builder /build/target/release/skill-runner /app/target/release/
COPY --from=builder /build/target/release/telegramd /app/target/release/
COPY --from=builder /build/target/release/wechatd /app/target/release/
COPY --from=builder /build/target/release/whatsappd /app/target/release/
COPY --from=builder /build/target/release/whatsapp_webd /app/target/release/
COPY --from=builder /build/target/release/*-skill /app/target/release/

COPY UI/dist/ /app/UI/dist/
COPY configs/ /app/configs/
COPY migrations/ /app/migrations/
COPY prompts/ /app/prompts/
COPY services/ /app/services/
COPY scripts/ /app/scripts/
COPY start-all-bin.sh /app/start-all-bin.sh
COPY start-wechatd.sh /app/start-wechatd.sh
COPY start-future-adapters.sh /app/start-future-adapters.sh
COPY stop-rustclaw.sh /app/stop-rustclaw.sh
COPY docker/docker-entrypoint.sh /app/docker/docker-entrypoint.sh

RUN chmod +x \
    /app/start-all-bin.sh \
    /app/start-wechatd.sh \
    /app/start-future-adapters.sh \
    /app/stop-rustclaw.sh \
    /app/docker/docker-entrypoint.sh \
    /app/scripts/auth-key.sh \
    /app/target/release/clawd \
    /app/target/release/skill-runner \
    /app/target/release/telegramd \
    /app/target/release/wechatd \
    /app/target/release/whatsappd \
    /app/target/release/whatsapp_webd \
    /app/target/release/*-skill \
 && mkdir -p /app/.pids /app/data /app/logs /app/image /app/audio /app/document

ENV RUST_LOG=warn

EXPOSE 8787

ENTRYPOINT ["/app/docker/docker-entrypoint.sh"]
