#!/bin/sh
# Arranque en Fly: siembra el volumen la primera vez y levanta Martino en modo público con llave.
set -e
if [ -z "$MARTINO_PUBLIC_KEY" ] || [ "${#MARTINO_PUBLIC_KEY}" -lt 24 ]; then
  echo "Falta MARTINO_PUBLIC_KEY (24+ caracteres): fly secrets set MARTINO_PUBLIC_KEY=\$(openssl rand -hex 16)" >&2
  exit 1
fi
# Agrega los ejemplos nuevos (p. ej. tutoriales) sin pisar nada de lo que ya está en el volumen.
cp -Rn /seed/collections /seed/flows /data/
exec martino --root /data serve --public --port 4400
