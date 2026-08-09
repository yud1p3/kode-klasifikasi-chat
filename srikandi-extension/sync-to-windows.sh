#!/bin/bash
# Sync extension ke Windows untuk Chrome load unpacked
# (sumber diambil dari lokasi script ini — kode-klasifikasi-chat/srikandi-extension)
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
rsync -av --delete \
  "$SCRIPT_DIR/extension/" \
  /mnt/c/Users/yudi/srikandi-extension/

echo ""
echo "✅ Extension tersinkronisasi ke C:\\Users\\yudi\\srikandi-extension"
echo "   🔄 Reload extension di chrome://extensions (ikon reload)"
