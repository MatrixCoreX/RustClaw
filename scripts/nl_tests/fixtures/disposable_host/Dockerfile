FROM ubuntu:26.04
ENV DEBIAN_FRONTEND=noninteractive
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates curl jq git python3 python3-pip python3-venv python3-yaml nodejs npm \
    procps iproute2 util-linux sqlite3 sudo bubblewrap \
    && rm -rf /var/cache/apt/archives/*.deb
WORKDIR /workspace
