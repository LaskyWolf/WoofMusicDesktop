#!/usr/bin/env bash
# Builds WoofMusic-x86_64.AppImage from target/release/woofmusic. Runs on
# Ubuntu 24.04 in CI (the oldest system with libmpv 0.35+), with the build
# packages from .github/workflows/build.yml installed.
#
# linuxdeploy copies the binary and every library it needs (libmpv, ffmpeg,
# GTK, WebKitGTK). WebKitGTK also starts two helper processes from fixed
# /usr paths: they are copied in by hand and the paths made relative, the
# same way Tauri's bundler does it.
set -euo pipefail
cd "$(dirname "$0")/../.."
here=packaging/linux
tools=target/appimage-tools
appdir=target/AppDir

mkdir -p "$tools"
fetch() {
    [ -x "$tools/$1" ] || { curl -fsSL -o "$tools/$1" "$2" && chmod +x "$tools/$1"; }
}
fetch linuxdeploy https://github.com/linuxdeploy/linuxdeploy/releases/download/continuous/linuxdeploy-x86_64.AppImage
fetch linuxdeploy-plugin-gtk.sh https://raw.githubusercontent.com/linuxdeploy/linuxdeploy-plugin-gtk/master/linuxdeploy-plugin-gtk.sh
install -m 755 "$here/linuxdeploy-plugin-webkit.sh" "$tools/"
export PATH="$PWD/$tools:$PATH" APPIMAGE_EXTRACT_AND_RUN=1 ARCH=x86_64
export OUTPUT=WoofMusic-x86_64.AppImage

rm -rf "$appdir"
webkit=/usr/lib/x86_64-linux-gnu/webkit2gtk-4.1
for file in WebKitNetworkProcess WebKitWebProcess injected-bundle/libwebkit2gtkinjectedbundle.so; do
    install -D "$webkit/$file" "$appdir$webkit/$file"
done

linuxdeploy --appdir "$appdir" \
    --executable target/release/woofmusic \
    --desktop-file "$here/com.laskywolf.woofmusic.desktop" \
    --icon-file crates/app/assets/woofmusic.svg \
    --custom-apprun "$here/AppRun" \
    --plugin gtk --plugin webkit \
    --output appimage
