# Imagen de Martino para Fly: sólo el binario ya compilado (el CI lo arma con la UI embebida).
# Mismo Ubuntu que el runner del CI para que la glibc coincida.
FROM ubuntu:24.04
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates && rm -rf /var/lib/apt/lists/*
COPY target/release/martino /usr/local/bin/martino
# Datos de ejemplo: se copian al volumen /data sólo la primera vez.
COPY collections /seed/collections
COPY flows /seed/flows
COPY deploy/start.sh /usr/local/bin/start.sh
EXPOSE 4400
CMD ["/usr/local/bin/start.sh"]
