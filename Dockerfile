# syntax=docker/dockerfile:1

FROM rust:1-slim AS builder

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        build-essential \
        ca-certificates \
        clang \
        cmake \
        libprotobuf-dev \
        pkg-config \
        protobuf-compiler \
        curl \
    && rm -rf /var/lib/apt/lists/*

# Node 22 + corepack：sebas-webui/build.rs 会在 cargo build 时自动构建前端并
# 嵌入二进制；没有 Node 工具链时只会嵌入占位页（见 build.rs 的降级逻辑）。
RUN curl -fsSL https://deb.nodesource.com/setup_22.x | bash - \
    && apt-get install -y --no-install-recommends nodejs \
    && rm -rf /var/lib/apt/lists/* \
    && corepack enable

WORKDIR /app

COPY Cargo.toml Cargo.lock ./
COPY src ./src
COPY tests ./tests
COPY xtask ./xtask
COPY sebas-router ./sebas-router
COPY sebas-feishu ./sebas-feishu
COPY sebas-acp ./sebas-acp
COPY sebas-gateway ./sebas-gateway
COPY sebas-channels ./sebas-channels
COPY sebas-agent ./sebas-agent
COPY sebas-webui ./sebas-webui

RUN cargo build --release --locked --bin sebas

FROM debian:stable-slim AS runtime

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        ca-certificates \
        tzdata \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/release/sebas /usr/local/bin/sebas

WORKDIR /app

# sebas core 在找不到 config.toml 时会回退到环境变量
# （SEBAS_FEISHU_APP_ID / SEBAS_FEISHU_APP_SECRET），
# 也可以用 -v ./config.toml:/app/config.toml 挂载配置。
ENTRYPOINT ["/usr/local/bin/sebas"]
CMD ["core", "--config", "/app/config.toml"]
