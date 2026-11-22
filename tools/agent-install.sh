#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-or-later
# Installe l'assistant local sur la machine d'inference : modele Qwen3-8B + service systemd (utilisateur) pour llama-server.
# Usage : tools/agent-install.sh --llama-dir <dossier de llama-server> [--models-dir D] [--port 8081] [--host 0.0.0.0] [--dry-run]
# Rien n'est lance en root ; le service est cree dans ~/.config/systemd/user.
set -euo pipefail

MODEL_FILE="Qwen3-8B-Q4_K_M.gguf"
MODEL_URL="https://huggingface.co/Qwen/Qwen3-8B-GGUF/resolve/main/${MODEL_FILE}"
MODELS_DIR="$HOME/IA/models"
PORT=8081
HOST=127.0.0.1
LLAMA_DIR=""
DRY=0

while [ $# -gt 0 ]; do
  case "$1" in
    --llama-dir) LLAMA_DIR="$2"; shift 2 ;;
    --models-dir) MODELS_DIR="$2"; shift 2 ;;
    --port) PORT="$2"; shift 2 ;;
    --host) HOST="$2"; shift 2 ;;
    --dry-run) DRY=1; shift ;;
    *) echo "option inconnue : $1" >&2; exit 2 ;;
  esac
done

[ -n "$LLAMA_DIR" ] || { echo "--llama-dir <dossier de llama-server> requis (binaire llama.cpp avec CUDA)" >&2; exit 2; }
[ -x "$LLAMA_DIR/llama-server" ] || { echo "$LLAMA_DIR/llama-server introuvable ou non executable" >&2; exit 1; }

run() { if [ "$DRY" = 1 ]; then echo "[essai] $*"; else "$@"; fi; }

# 1. Modele (5 Go), repris si le telechargement a ete interrompu.
run mkdir -p "$MODELS_DIR"
if [ -f "$MODELS_DIR/$MODEL_FILE" ]; then
  echo "modele deja present : $MODELS_DIR/$MODEL_FILE"
else
  run curl -fL -C - -o "$MODELS_DIR/$MODEL_FILE" "$MODEL_URL"
fi

# 2. Service systemd utilisateur : contexte 16 k, appels d'outils (--jinja), une requete a la fois.
UNIT_DIR="$HOME/.config/systemd/user"
UNIT="$UNIT_DIR/dominet-llm.service"
CONTENT="[Unit]
Description=DomiNet assistant local (llama-server, Qwen3-8B)
After=network.target

[Service]
Environment=LD_LIBRARY_PATH=$LLAMA_DIR
ExecStart=$LLAMA_DIR/llama-server -m $MODELS_DIR/$MODEL_FILE --host $HOST --port $PORT -c 16384 --jinja -np 1
Restart=on-failure
RestartSec=5

[Install]
WantedBy=default.target
"
if [ "$DRY" = 1 ]; then
  echo "[essai] ecrirait $UNIT :"; echo "$CONTENT"
  echo "[essai] systemctl --user daemon-reload && systemctl --user enable --now dominet-llm.service"
  exit 0
fi
mkdir -p "$UNIT_DIR"
printf '%s' "$CONTENT" > "$UNIT"
systemctl --user daemon-reload
systemctl --user enable --now dominet-llm.service
echo "service demarre ; pour qu'il survive a la deconnexion : sudo loginctl enable-linger $USER"
echo "dans DomiNet (Reglages > Assistant) : AGENT_LLM_URL=http://<cette machine>:$PORT  AGENT_MODEL=${MODEL_FILE%.gguf}"
