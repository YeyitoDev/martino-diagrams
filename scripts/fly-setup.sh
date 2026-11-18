#!/usr/bin/env bash
# Prepara Martino en Fly una sola vez, desde una máquina con sesión en Fly:
#   curl -fsSL https://raw.githubusercontent.com/YeyitoDev/martino-diagrams/master/scripts/fly-setup.sh | bash
# Crea la app y el volumen, genera la llave de acceso, y deja el token de despliegue como secreto
# del repo (si `gh` tiene sesión) para que el CI despliegue en cada push a master.
set -euo pipefail
APP="${APP:-martino}"
REGION="${REGION:-gru}"
REPO="${REPO:-YeyitoDev/martino-diagrams}"
FLY="$(command -v fly || command -v flyctl || echo "$HOME/.fly/bin/fly")"

"$FLY" auth whoami >/dev/null || { echo "Inicia sesión primero: fly auth login" >&2; exit 1; }

if ! "$FLY" status -a "$APP" >/dev/null 2>&1; then
  "$FLY" apps create "$APP"
fi
if ! "$FLY" volumes list -a "$APP" 2>/dev/null | grep -q martino_data; then
  "$FLY" volumes create martino_data --region "$REGION" --size 1 -a "$APP" -y
fi

KEY=""
if ! "$FLY" secrets list -a "$APP" 2>/dev/null | grep -q MARTINO_PUBLIC_KEY; then
  KEY="$(openssl rand -hex 16)"
  "$FLY" secrets set "MARTINO_PUBLIC_KEY=$KEY" -a "$APP" --stage >/dev/null
fi

TOKEN="$("$FLY" tokens create deploy -a "$APP" -x 8760h)"
if command -v gh >/dev/null && gh auth status >/dev/null 2>&1; then
  printf '%s' "$TOKEN" | gh secret set FLY_API_TOKEN -R "$REPO"
  gh workflow run deploy -R "$REPO"
  echo "✓ Token guardado en $REPO y despliegue lanzado (mira: gh run watch -R $REPO)."
else
  echo "gh no tiene sesión aquí. Guarda este token como secreto FLY_API_TOKEN en"
  echo "https://github.com/$REPO/settings/secrets/actions y vuelve a correr el workflow «deploy»:"
  echo "$TOKEN"
fi

if [ -n "$KEY" ]; then
  echo
  echo "🔑 Tu enlace (guárdalo en tu gestor de contraseñas; no se vuelve a mostrar):"
  echo "   https://$APP.fly.dev/?key=$KEY"
else
  echo "La llave ya existía; para cambiarla: fly secrets set MARTINO_PUBLIC_KEY=\$(openssl rand -hex 16) -a $APP"
fi
