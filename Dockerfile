# syntax=docker/dockerfile:1

FROM rust:1-slim AS builder

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        build-essential \
        ca-certificates \
        clang \
        cmake \
        pkg-config \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

COPY Cargo.toml Cargo.lock ./
COPY src ./src
COPY tests ./tests
COPY router ./router
COPY feishu ./feishu
COPY acp-claude ./acp-claude
COPY gateway ./gateway

RUN cargo build --release --locked --bin sebas

FROM debian:stable-slim AS runtime

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        ca-certificates \
        tzdata \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/release/sebas /usr/local/bin/sebas

WORKDIR /app

# sebas run 在找不到 config.toml 时会回退到环境变量
# （SEBAS_FEISHU_APP_ID / SEBAS_FEISHU_APP_SECRET），
# 也可以用 -v ./config.toml:/app/config.toml 挂载配置。
ENTRYPOINT ["/usr/local/bin/sebas"]
CMD ["run", "--config", "/app/config.toml"]
