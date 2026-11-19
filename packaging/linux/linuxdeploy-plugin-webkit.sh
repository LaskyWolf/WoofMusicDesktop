#!/bin/sh
# linuxdeploy input plugin: WebKitGTK has the absolute paths of its helper
# processes compiled in (/usr/lib/.../webkit2gtk-4.1). Rewriting "/usr" to
# "././" (same length) makes them relative to the working directory, which
# AppRun sets to the AppImage's usr/.
case "$1" in
    --plugin-api-version) echo 0; exit 0 ;;
    --plugin-type) echo input; exit 0 ;;
    --appdir) find "$2/usr/lib" -name 'libwebkit*' -exec sed -i -e 's|/usr|././|g' {} + ;;
    *) echo "usage: $0 --appdir <AppDir>" >&2; exit 1 ;;
esac
