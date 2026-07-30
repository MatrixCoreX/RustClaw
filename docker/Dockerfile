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

# Registry is the single package-selection authority. This builds all runtime
# and core-tool binaries plus Linux-compatible fixed skills, while every
# install_mode=on_demand package remains absent until its own install action.
RUN set -eu; \
    BUILD_EXCLUDES="$(python3 scripts/skill_store_packages.py \
      --scope build-excludes --target linux --format cargo-excludes)"; \
    cargo build --release --workspace ${BUILD_EXCLUDES}; \
    mkdir -p /build/runtime-release; \
    find /build/target/release -maxdepth 1 -type f -perm /111 \
      -exec cp -p '{}' /build/runtime-release/ \;

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

RUN mkdir -p /app/target/release /app/UI/dist /app/docker/config /app/component_start

# The builder staging directory contains every selected executable: core tools,
# channel services, and fixed Linux runners. It cannot contain on-demand runners
# because their packages were excluded from the build above.
COPY --from=builder /build/runtime-release/ /app/target/release/

COPY UI/dist/ /app/UI/dist/
COPY configs/ /app/configs/
COPY migrations/ /app/migrations/
COPY prompts/ /app/prompts/
COPY services/ /app/services/
COPY scripts/ /app/scripts/
COPY component_start/ /app/component_start/
COPY start-all-bin.sh /app/start-all-bin.sh
COPY stop-agent.sh /app/stop-agent.sh
COPY docker/docker-entrypoint.sh /app/docker/docker-entrypoint.sh

RUN chmod +x \
    /app/start-all-bin.sh \
    /app/component_start/*.sh \
    /app/stop-agent.sh \
    /app/docker/docker-entrypoint.sh \
    /app/scripts/auth-key.sh \
 && find /app/target/release -maxdepth 1 -type f -exec chmod +x '{}' + \
 && mkdir -p /app/.pids /app/data /app/logs /app/image /app/audio /app/document

ENV RUST_LOG=warn

EXPOSE 8788

ENTRYPOINT ["/app/docker/docker-entrypoint.sh"]
