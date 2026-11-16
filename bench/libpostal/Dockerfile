FROM debian:trixie-slim
RUN apt-get update && apt-get install -y --no-install-recommends build-essential autoconf automake libtool pkg-config curl git ca-certificates python3 python3-dev python3-pip time procps && rm -rf /var/lib/apt/lists/*
RUN git clone https://github.com/openvenues/libpostal /src/libpostal && cd /src/libpostal && git checkout v1.1.4
WORKDIR /src/libpostal
RUN ./bootstrap.sh && ./configure --datadir=/data && make -j8 && make install && ldconfig
RUN pip3 install --break-system-packages postal
