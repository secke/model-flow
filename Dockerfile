# syntax=docker/dockerfile:1
# ModelFlow image: `mf` and `mf-server` binaries, Python and QuantLib for external models.

FROM rust:1.88-bookworm AS build
WORKDIR /src
COPY . .
RUN cargo build --release --locked -p mf-cli -p mf-server \
 && strip target/release/mf target/release/mf-server

FROM python:3.12-slim-bookworm
ARG QUANTLIB_VERSION=1.44
RUN pip install --no-cache-dir "QuantLib==${QUANTLIB_VERSION}" \
 && useradd --uid 10001 --user-group --no-create-home --home-dir /workspace modelflow \
 && mkdir -p /workspace /etc/modelflow \
 && chown modelflow:modelflow /workspace
COPY --from=build /src/target/release/mf /src/target/release/mf-server /usr/local/bin/
COPY --chown=root:root examples /opt/modelflow/examples
WORKDIR /workspace
USER 10001:10001
EXPOSE 8080
HEALTHCHECK --interval=30s --timeout=3s CMD python3 -c "import urllib.request; urllib.request.urlopen('http://127.0.0.1:8080/api/health', timeout=2)" || exit 1
ENTRYPOINT ["mf-server"]
CMD ["serve", "--config", "/etc/modelflow/server.toml"]
