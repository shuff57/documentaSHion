#!/usr/bin/env bash
set -euo pipefail
binary="${1:-target/release/documentashion}"
if [[ ! -f "$binary" ]]; then
  echo "Build first: cargo build --release --locked" >&2
  exit 1
fi
install -d "$HOME/.local/bin" "$HOME/.local/share/applications" "$HOME/.local/share/icons/hicolor/scalable/apps"
install -m 755 "$binary" "$HOME/.local/bin/documentashion.new"
strip "$HOME/.local/bin/documentashion.new"
mv "$HOME/.local/bin/documentashion.new" "$HOME/.local/bin/documentashion"
install -m 644 assets/documentashion.svg "$HOME/.local/share/icons/hicolor/scalable/apps/documentashion.svg"
cat > "$HOME/.local/share/applications/documentashion.desktop" <<DESKTOP
[Desktop Entry]
Type=Application
Name=documentaSHion
Comment=Scan, transcribe, and review school documents
Exec="$HOME/.local/bin/documentashion"
Icon=documentashion
Terminal=false
Categories=Graphics;Scanning;
StartupNotify=true
DESKTOP
# verSHion's figure generators, which draw the graphs of rebuilt worksheets. Optional: without
# it a rebuild leaves graphs out.
figure="tools/vershion-figure/target/release/vershion-figure"
if [[ -f "$figure" ]]; then
  install -m 755 "$figure" "$HOME/.local/bin/vershion-figure"
else
  printf 'Graphs in rebuilt worksheets need: (cd tools/vershion-figure && cargo build --release), then ./install.sh again.\n'
fi
if command -v update-desktop-database >/dev/null; then
  update-desktop-database "$HOME/.local/share/applications"
fi
printf 'Installed documentaSHion. Launch it from your application menu.\n'
