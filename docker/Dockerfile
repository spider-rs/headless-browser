FROM rust:alpine3.21 AS rustbuilder

WORKDIR /app

RUN apk upgrade --update-cache --available && \
    apk add --no-cache gcc make g++ cmake musl-dev perl libressl-dev

COPY ../headless_browser/ ./headless_browser
COPY ../headless_browser_lib/ ./headless_browser_lib
COPY ../benches/ ./benches
COPY ../Cargo.* .

RUN rustup update stable
RUN RUST_LOG=error cargo install --no-default-features --path headless_browser

FROM alpine:3.21

# Installs latest Chromium package.
RUN apk upgrade --no-cache --available \
    && apk add --no-cache \
      chromium-swiftshader \
      ttf-freefont \
      font-noto-emoji \
    && apk add --no-cache \
      --repository=https://dl-cdn.alpinelinux.org/alpine/edge/community \
      font-wqy-zenhei

COPY ../local.conf /etc/fonts/local.conf

# Add Chrome as a user
RUN mkdir -p /usr/src/app \
    && adduser -D chrome \
    && chown -R chrome:chrome /usr/src/app
# Run Chrome as non-privileged
USER chrome
WORKDIR /usr/src/app

ENV CHROME_BIN=/usr/bin/chromium-browser \
    CHROME_PATH=/usr/lib/chromium/

EXPOSE 9222 6000

USER root

COPY --from=rustbuilder /usr/local/cargo/bin/headless_browser /usr/local/bin/headless_browser
COPY ../scripts/docker-entrypoint.sh /

RUN apk add --no-cache tini curl sudo nss dbus freetype harfbuzz ca-certificates libxcomposite libxrandr \
    libxdamage libxext libxshmfence mesa-gl udev

RUN chmod +x /docker-entrypoint.sh

USER chrome

ENV REMOTE_ADDRESS=0.0.0.0
ENV LAUNCH=init
ENV DEFAULT_PORT=9223
ENV DEFAULT_PORT_SERVER=6000
# ENV HOSTNAME_OVERRIDE=127.0.0.1

ENTRYPOINT ["tini", "--", "/docker-entrypoint.sh"]
