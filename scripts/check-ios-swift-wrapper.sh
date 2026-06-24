#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."

./scripts/build-ios-xcframework.sh

headers="target/ios/ReaderCore.xcframework/ios-arm64-simulator/Headers"
test -f "$headers/reader_core.h"
test -f "$headers/module.modulemap"

xcrun --sdk iphonesimulator swiftc \
  -target arm64-apple-ios13.0-simulator \
  -I "$headers" \
  -typecheck bindings/ios/Sources/ReaderCoreClient/ReaderCoreClient.swift

echo "typechecked bindings/ios/Sources/ReaderCoreClient/ReaderCoreClient.swift"
