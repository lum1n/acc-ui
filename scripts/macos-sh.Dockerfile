# A stand-in for macOS's sh and awk, for scripts/test-shells.sh:
# bash 3.2 as /bin/sh (which, run as sh, starts in POSIX mode like macOS's)
# and the one true awk (BWK awk), which macOS ships as awk.
#
#   docker build -t acc-ui-macos-sh -f scripts/macos-sh.Dockerfile scripts
FROM bash:3.2
# The one true awk as Debian packages it (original-awk), from the Debian
# archive.
ARG AWK_VERSION=2025-12-25
RUN apk add --no-cache --virtual .build build-base bison \
 && wget -qO /tmp/awk.tar.gz "https://deb.debian.org/debian/pool/main/o/original-awk/original-awk_${AWK_VERSION}.orig.tar.gz" \
 && mkdir /tmp/awk && tar xzf /tmp/awk.tar.gz -C /tmp/awk --strip-components=1 \
 && make -C /tmp/awk \
 && install -m 755 /tmp/awk/a.out /usr/local/bin/awk \
 && ln -sf /usr/local/bin/awk /usr/bin/awk \
 && ln -sf /usr/local/bin/bash /bin/sh \
 && apk del .build && rm -rf /tmp/awk /tmp/awk.tar.gz
