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

# 整仓拷贝 + .dockerignore 瘦身：cargo 解析 workspace 需要每个成员的清单存在，
# 逐成员 COPY 的清单已经两次漏掉新成员（sebas-node-link、后来的 sebas-db 系），
# 每次漏都让镜像构建在解析阶段以 101 失败。整仓拷贝对新成员免疫；
# .dockerignore 排除 target/、node_modules、**/dist 等，context 保持在几 MB。
COPY . .

# 镜像带双二进制：主控 `sebas` 与执行节点 `sebas-node`
# （节点机 `docker run <image> sebas-node …` 覆盖 command 使用）。

RUN cargo build --release --locked --bin sebas
# sebas-node 是独立二进制的执行节点。必须带 -p：`--bin` 只在默认包里解析
# 目标，跨包要显式指定包（与 ci.yml 一致）。
RUN cargo build --release --locked -p sebas-node --bin sebas-node

FROM debian:stable-slim AS runtime

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        ca-certificates \
        tzdata \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/release/sebas /usr/local/bin/sebas
COPY --from=builder /app/target/release/sebas-node /usr/local/bin/sebas-node

# 状态存储默认落在 ~/.sebas（容器内即 /root/.sebas），预建避免每次启动报
# 一次"打开数据库失败→回退文件存储"的 ERROR。
RUN mkdir -p /root/.sebas

WORKDIR /app

# sebas core 在找不到 config.toml 时会回退到环境变量
# （SEBAS_FEISHU_APP_ID / SEBAS_FEISHU_APP_SECRET），
# 也可以用 -v ./config.toml:/app/config.toml 挂载配置。
ENTRYPOINT ["/usr/local/bin/sebas"]
CMD ["core", "--config", "/app/config.toml"]
