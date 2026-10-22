# The duel's match server. CI builds the binary on ubuntu-latest (see .github/workflows/duel-server.yml)
# and copies it in, so the runtime image matches the glibc it was linked against.
FROM ubuntu:24.04
RUN apt-get update \
    && apt-get install -y --no-install-recommends libasound2t64 ca-certificates \
    && rm -rf /var/lib/apt/lists/*
COPY duel /usr/local/bin/duel
CMD ["duel", "server"]
