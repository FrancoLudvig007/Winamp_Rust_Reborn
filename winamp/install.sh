#!/usr/bin/env bash
# Instala o Winamp RS para o usuário atual (sem sudo) e o registra em "Abrir com" para .mp3/.wav.
# Uso: cargo build --release && ./install.sh
set -euo pipefail
cd "$(dirname "$0")"

BIN="target/release/winamp-rs"
[ -x "$BIN" ] || { echo "Compile antes: cargo build --release" >&2; exit 1; }

APPS="$HOME/.local/share/applications"
ICONS="$HOME/.local/share/icons/hicolor/256x256/apps"
mkdir -p "$HOME/.local/bin" "$APPS" "$ICONS"

install -m 755 "$BIN" "$HOME/.local/bin/winamp-rs"
# O ícone vem embutido no executável; aqui ele é exportado como PNG para o menu/Abrir com.
"$HOME/.local/bin/winamp-rs" --export-icon "$ICONS/winamp-rs.png" || echo "Aviso: ícone não exportado (winamp.ico ausente no build)."

cat > "$APPS/winamp-rs.desktop" <<EOF
[Desktop Entry]
Type=Application
Name=Winamp RS
GenericName=Music Player
Comment=Player de áudio estilo Winamp (mp3, wav)
Exec=$HOME/.local/bin/winamp-rs %F
Icon=winamp-rs
Terminal=false
Categories=AudioVideo;Audio;Player;
MimeType=audio/mpeg;audio/mp3;audio/x-mp3;audio/x-mpeg;audio/wav;audio/x-wav;audio/vnd.wave;
StartupWMClass=winamp-rs
EOF

update-desktop-database "$APPS" 2>/dev/null || true
gtk-update-icon-cache -q -t "$HOME/.local/share/icons/hicolor" 2>/dev/null || true

echo "Instalado. Clique direito em um .mp3/.wav > Abrir com > Winamp RS."
echo "Para torná-lo padrão: xdg-mime default winamp-rs.desktop audio/mpeg audio/x-wav"