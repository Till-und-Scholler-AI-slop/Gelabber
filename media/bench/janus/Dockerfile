FROM debian:trixie-slim@sha256:a99cfc517144bc59b1978475ec53b46ecabec7e43635402ee5b77cc54cd1b20a AS build
ENV JANUS_COMMIT=0a24110ae55a172c4293749b763dbb66a138f9ec
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates git build-essential autoconf automake libtool pkg-config \
    libglib2.0-dev libjansson-dev libconfig-dev libnice-dev libssl-dev \
    libsrtp2-dev libmicrohttpd-dev && rm -rf /var/lib/apt/lists/*
RUN git init /source && git -C /source fetch --depth 1 https://github.com/meetecho/janus-gateway.git "$JANUS_COMMIT" \
    && git -C /source checkout --detach FETCH_HEAD && test "$(git -C /source rev-parse HEAD)" = "$JANUS_COMMIT"
WORKDIR /source
RUN sh autogen.sh && ./configure --prefix=/opt/janus --disable-data-channels --disable-websockets \
    --disable-rabbitmq --disable-mqtt --disable-unix-sockets --disable-nanomsg \
    --disable-plugin-audiobridge --disable-plugin-echotest --disable-plugin-recordplay \
    --disable-plugin-sip --disable-plugin-nosip --disable-plugin-streaming \
    --disable-plugin-textroom --disable-plugin-videocall --disable-plugin-lua --disable-plugin-duktape \
    --disable-docs && make -j4 && make install
FROM debian:trixie-slim@sha256:a99cfc517144bc59b1978475ec53b46ecabec7e43635402ee5b77cc54cd1b20a
RUN apt-get update && apt-get install -y --no-install-recommends \
    libglib2.0-0t64 libjansson4 libconfig11 libnice10 libssl3t64 libsrtp2-1 libmicrohttpd12t64 \
    && rm -rf /var/lib/apt/lists/*
COPY --from=build /opt/janus /opt/janus
COPY janus.jcfg janus.transport.http.jcfg janus.plugin.videoroom.jcfg /opt/janus/etc/janus/
LABEL org.opencontainers.image.version="janus-v1.4.2" \
    org.opencontainers.image.revision="0a24110ae55a172c4293749b763dbb66a138f9ec"
USER nobody
EXPOSE 8088 10000-10199/udp
ENTRYPOINT ["/opt/janus/bin/janus", "-F", "/opt/janus/etc/janus"]
CMD ["--help"]
