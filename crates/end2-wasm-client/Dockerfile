FROM rust:1.99-alpine3.22 AS wasm-builder
WORKDIR /build
RUN cargo install wasm-pack
COPY Cargo.toml .
COPY ./src ./src
RUN wasm-pack build --target bundler

FROM node:24-alpine AS frontend-builder
WORKDIR /app/frontend
COPY ./frontend /app/frontend
COPY --from=wasm-builder /build/pkg /app/pkg
RUN npm install
# Vite pins these into the bundle, so they come in as build args rather than runtime env.
ARG VITE_VRF_PUBLIC_KEY
ARG VITE_COMET_RPC_URLS
RUN node --run build

FROM frontend-builder AS frontend
WORKDIR /app/frontend
RUN npm install -g serve
CMD ["serve", "-s", "dist", "-l", "8081"]