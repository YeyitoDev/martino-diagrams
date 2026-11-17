#!/bin/bash
# Instala Martino como servicio en una Mac y lo publica sólo dentro de tu red de Tailscale.
#   curl -fsSL https://raw.githubusercontent.com/YeyitoDev/martino-diagrams/master/scripts/instalar-mac.sh | bash
# Volver a correrlo actualiza a la última versión y conserva la llave y tus datos.
set -euo pipefail

REPO=YeyitoDev/martino-diagrams
DIR="$HOME/Martino"
BIN="$DIR/bin/martino"
PORT="${PORT:-4400}"
LABEL=dev.martino
PLIST="$HOME/Library/LaunchAgents/$LABEL.plist"

TS=$(command -v tailscale || true)
[ -z "$TS" ] && [ -x /Applications/Tailscale.app/Contents/MacOS/Tailscale ] && TS=/Applications/Tailscale.app/Contents/MacOS/Tailscale
[ -z "$TS" ] && { echo "✗ No encuentro Tailscale. Instálalo y entra a tu red primero."; exit 1; }

mkdir -p "$DIR/bin" "$HOME/Library/LaunchAgents"
echo "→ Descargando la última versión…"
curl -fsSL "https://github.com/$REPO/releases/latest/download/martino-mac" -o "$BIN.nuevo"
chmod +x "$BIN.nuevo"
xattr -d com.apple.quarantine "$BIN.nuevo" 2>/dev/null || true
codesign -s - --force "$BIN.nuevo" 2>/dev/null || true
mv "$BIN.nuevo" "$BIN"

# La llave se genera una sola vez: así el enlace sigue sirviendo después de actualizar o reiniciar.
[ -f "$DIR/.llave" ] || { openssl rand -hex 16 > "$DIR/.llave"; chmod 600 "$DIR/.llave"; }
KEY=$(cat "$DIR/.llave")

cat > "$PLIST" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>Label</key><string>$LABEL</string>
  <key>ProgramArguments</key><array>
    <string>$BIN</string><string>serve</string><string>--public</string>
    <string>--port</string><string>$PORT</string><string>--root</string><string>$DIR</string>
  </array>
  <key>EnvironmentVariables</key><dict><key>MARTINO_PUBLIC_KEY</key><string>$KEY</string></dict>
  <key>RunAtLoad</key><true/>
  <key>KeepAlive</key><true/>
  <key>StandardOutPath</key><string>$DIR/martino.log</string>
  <key>StandardErrorPath</key><string>$DIR/martino.log</string>
</dict></plist>
EOF
chmod 600 "$PLIST"

launchctl bootout "gui/$(id -u)/$LABEL" 2>/dev/null && sleep 2 || true
launchctl bootstrap "gui/$(id -u)" "$PLIST"

# serve = sólo tu tailnet. Nunca funnel: Martino lee archivos y llama a cualquier URL.
"$TS" serve --bg --https="$PORT" "http://127.0.0.1:$PORT" >/dev/null

for _ in 1 2 3 4 5 6 7 8 9 10; do curl -fsS "http://127.0.0.1:$PORT/" -o /dev/null 2>/dev/null && break; sleep 1; done
HOST=$("$TS" status --json | grep -m1 '"DNSName"' | sed -E 's/.*"DNSName": *"([^"]*)\.".*/\1/')
echo "✓ Martino corriendo (se reinicia solo al prender la Mac)."
echo "  Abre desde cualquier equipo de tu tailnet:"
echo "  https://$HOST:$PORT/?key=$KEY"
echo "  Datos: $DIR · Registro: $DIR/martino.log"
